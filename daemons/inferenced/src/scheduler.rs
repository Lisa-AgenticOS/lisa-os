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
use tokio::sync::{Mutex, Semaphore};

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

pub struct Scheduler {
    slots: Arc<Semaphore>,
    /// Registered background work, keyed so completed entries prune
    /// themselves (#40 — a long-lived daemon must not accumulate stale
    /// handles). Drained wholesale on interactive preemption.
    background: Arc<Mutex<Vec<(u64, AbortHandle)>>>,
    next_id: std::sync::atomic::AtomicU64,
}

impl Scheduler {
    pub fn new(slots: usize) -> Self {
        Self {
            slots: Arc::new(Semaphore::new(slots)),
            background: Arc::new(Mutex::new(Vec::new())),
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
                for (_, handle) in self.background.lock().await.drain(..) {
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
    async fn register_background(&self) -> (u64, futures::stream::AbortRegistration) {
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (handle, registration) = AbortHandle::new_pair();
        self.background.lock().await.push((id, handle));
        (id, registration)
    }

    async fn prune_background(&self, id: u64) {
        self.background.lock().await.retain(|(i, _)| *i != id);
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
                let (id, registration) = self.register_background().await;
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
                self.prune_background(id).await;
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
                let (id, registration) = self.register_background().await;
                let slots = Arc::clone(&self.slots);
                let background = Arc::clone(&self.background);
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
                    background.lock().await.retain(|(i, _)| *i != id);
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
