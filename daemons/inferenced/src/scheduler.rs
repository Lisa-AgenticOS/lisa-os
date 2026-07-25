//! QoS scheduler (`docs/PLAN.md` §5.1): priority classes with preemption
//! by cancellation. M1 scope: two classes over one generation slot —
//! `interactive` (assistant, foreground) preempts `background` (indexing,
//! batch) by aborting its stream; the §5.1 budget is preemption within
//! 250 ms. Later: `ui` class, per-model slots, PSI awareness, power
//! signals.

use crate::engine::{EngineError, TokenStream};
use futures::StreamExt;
use futures::stream::{AbortHandle, Abortable};
use std::sync::Arc;
use tokio::sync::Semaphore;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    Interactive,
    Background,
}

impl Priority {
    pub fn parse(s: Option<&str>) -> Self {
        match s {
            Some("background") => Priority::Background,
            _ => Priority::Interactive,
        }
    }
}

/// The registry is a std (not tokio) mutex: every critical section is a
/// short push/drain/retain with no await inside, and pruning must be able
/// to run in `Drop` (#41 — a consumer that stops polling, or a stream
/// dropped on client disconnect, still has to deregister).
type BackgroundRegistry = Arc<std::sync::Mutex<Vec<(u64, AbortHandle)>>>;

/// Deregisters one background unit when dropped — however its work ended:
/// completion, error, preemption, or the consumer dropping the stream.
struct PruneGuard {
    id: u64,
    registry: BackgroundRegistry,
}

impl Drop for PruneGuard {
    fn drop(&mut self) {
        if let Ok(mut reg) = self.registry.lock() {
            reg.retain(|(i, _)| *i != self.id);
        }
    }
}

pub struct Scheduler {
    slots: Arc<Semaphore>,
    /// Registered background work, keyed so entries prune themselves via
    /// PruneGuard (#40/#41). Drained wholesale on interactive preemption.
    background: BackgroundRegistry,
    next_id: std::sync::atomic::AtomicU64,
}

impl Scheduler {
    pub fn new(slots: usize) -> Self {
        Self {
            slots: Arc::new(Semaphore::new(slots)),
            background: Arc::new(std::sync::Mutex::new(Vec::new())),
            next_id: std::sync::atomic::AtomicU64::new(0),
        }
    }

    async fn acquire_interactive(&self) -> tokio::sync::OwnedSemaphorePermit {
        match Arc::clone(&self.slots).try_acquire_owned() {
            Ok(p) => p,
            Err(_) => {
                // Preempt: abort every registered background unit — running
                // OR still waiting for the slot — then wait (freed when the
                // aborted work drops).
                let handles: Vec<(u64, AbortHandle)> = {
                    let mut reg = self.background.lock().expect("registry poisoned");
                    reg.drain(..).collect()
                };
                for (_, handle) in handles {
                    handle.abort();
                }
                Arc::clone(&self.slots)
                    .acquire_owned()
                    .await
                    .expect("scheduler semaphore never closes")
            }
        }
    }

    /// Register background work for preemption BEFORE it waits on the
    /// slot (#39): registering after acquisition left a window where an
    /// interactive sweep missed the handle and then blocked behind the
    /// whole background unit. The permit is acquired inside the abortable
    /// scope, so preemption also cancels background work still queueing.
    fn register_background(&self) -> (PruneGuard, futures::stream::AbortRegistration) {
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (handle, registration) = AbortHandle::new_pair();
        self.background
            .lock()
            .expect("registry poisoned")
            .push((id, handle));
        (
            PruneGuard {
                id,
                registry: Arc::clone(&self.background),
            },
            registration,
        )
    }

    /// Admit one non-streaming unit of engine work under `priority` —
    /// the tools passthrough lane (issue #34). Same slot pool and
    /// preemption contract as `admit`: interactive arrivals abort
    /// background work, including background tool turns.
    pub async fn admit_future<T: Send + 'static>(
        &self,
        priority: Priority,
        fut: futures::future::BoxFuture<'static, Result<T, EngineError>>,
    ) -> Result<T, EngineError> {
        match priority {
            Priority::Interactive => {
                let _permit = self.acquire_interactive().await;
                fut.await
            }
            Priority::Background => {
                let (guard, registration) = self.register_background();
                let slots = Arc::clone(&self.slots);
                let work = async move {
                    let _permit = slots
                        .acquire_owned()
                        .await
                        .expect("scheduler semaphore never closes");
                    fut.await
                };
                let outcome = match Abortable::new(work, registration).await {
                    Ok(result) => result,
                    Err(futures::stream::Aborted) => Err(EngineError::Preempted),
                };
                drop(guard);
                outcome
            }
        }
    }

    /// Admit `stream` under `priority`. The returned stream holds its
    /// slot until it completes; background streams can be aborted
    /// mid-flight when an interactive request needs the slot.
    pub async fn admit(&self, priority: Priority, stream: TokenStream) -> TokenStream {
        match priority {
            Priority::Interactive => {
                let permit = self.acquire_interactive().await;
                Box::pin(async_stream::stream! {
                    let _permit = permit;
                    let mut stream = stream;
                    while let Some(item) = stream.next().await {
                        yield item;
                    }
                })
            }
            Priority::Background => {
                let (guard, registration) = self.register_background();
                let slots = Arc::clone(&self.slots);
                // Permit acquisition happens inside the abortable stream,
                // so a preemption sweep also cancels queued (not-yet-
                // running) background streams (#39).
                let gated: TokenStream = Box::pin(async_stream::stream! {
                    let _permit = slots
                        .acquire_owned()
                        .await
                        .expect("scheduler semaphore never closes");
                    let mut stream = stream;
                    while let Some(item) = stream.next().await {
                        yield item;
                    }
                });
                let mut abortable = Abortable::new(gated, registration);
                Box::pin(async_stream::stream! {
                    // Moved into the generator: dropping the returned
                    // stream (client disconnect, error break) still runs
                    // the guard's deregistration (#41).
                    let _guard = guard;
                    loop {
                        match abortable.next().await {
                            Some(item) => yield item,
                            None => {
                                if abortable.is_aborted() {
                                    yield Err(EngineError::Preempted);
                                }
                                break;
                            }
                        }
                    }
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn slow_stream(tokens: usize, delay_ms: u64) -> TokenStream {
        Box::pin(async_stream::stream! {
            for i in 0..tokens {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                yield Ok(format!("t{i} "));
            }
        })
    }

    #[tokio::test]
    async fn interactive_preempts_background_within_budget() {
        let sched = Scheduler::new(1);

        // Background occupies the slot and streams slowly.
        let mut bg = sched
            .admit(Priority::Background, slow_stream(1000, 20))
            .await;
        let bg_task = tokio::spawn(async move {
            let mut items = Vec::new();
            while let Some(item) = bg.next().await {
                items.push(item);
            }
            items
        });
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Interactive arrives: must get its first token fast.
        let start = std::time::Instant::now();
        let mut ia = sched.admit(Priority::Interactive, slow_stream(3, 1)).await;
        let first = ia.next().await;
        assert!(first.is_some_and(|t| t.is_ok()));
        assert!(
            start.elapsed() < Duration::from_millis(250),
            "preemption took {:?} (budget 250 ms)",
            start.elapsed()
        );

        // The background stream ends with a Preempted error.
        let bg_items = bg_task.await.unwrap();
        assert!(
            matches!(bg_items.last(), Some(Err(EngineError::Preempted))),
            "background did not observe preemption: {:?}",
            bg_items.last()
        );
    }

    #[tokio::test]
    async fn background_runs_to_completion_when_uncontended() {
        let sched = Scheduler::new(1);
        let mut bg = sched.admit(Priority::Background, slow_stream(5, 1)).await;
        let mut count = 0;
        while let Some(item) = bg.next().await {
            assert!(item.is_ok());
            count += 1;
        }
        assert_eq!(count, 5);
    }
}
