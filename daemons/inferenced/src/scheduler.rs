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
    background: Arc<Mutex<Vec<AbortHandle>>>,
}

impl Scheduler {
    pub fn new(slots: usize) -> Self {
        Self {
            slots: Arc::new(Semaphore::new(slots)),
            background: Arc::new(Mutex::new(Vec::new())),
        }
    }

    async fn acquire(&self, priority: Priority) -> tokio::sync::OwnedSemaphorePermit {
        match priority {
            Priority::Interactive => match Arc::clone(&self.slots).try_acquire_owned() {
                Ok(p) => p,
                Err(_) => {
                    // Preempt: abort every running background stream, then
                    // wait for a slot (freed when the aborted stream drops).
                    for handle in self.background.lock().await.drain(..) {
                        handle.abort();
                    }
                    Arc::clone(&self.slots)
                        .acquire_owned()
                        .await
                        .expect("scheduler semaphore never closes")
                }
            },
            Priority::Background => Arc::clone(&self.slots)
                .acquire_owned()
                .await
                .expect("scheduler semaphore never closes"),
        }
    }

    /// Admit one non-streaming unit of engine work under `priority` —
    /// the tools passthrough lane (issue #34). Same slot pool and
    /// preemption contract as `admit`: interactive arrivals abort
    /// running background work, including background tool turns.
    pub async fn admit_future<T: Send + 'static>(
        &self,
        priority: Priority,
        fut: futures::future::BoxFuture<'static, Result<T, EngineError>>,
    ) -> Result<T, EngineError> {
        let permit = self.acquire(priority).await;
        match priority {
            Priority::Interactive => {
                let _permit = permit;
                fut.await
            }
            Priority::Background => {
                let (handle, registration) = AbortHandle::new_pair();
                self.background.lock().await.push(handle);
                let abortable = Abortable::new(fut, registration);
                let _permit = permit;
                match abortable.await {
                    Ok(result) => result,
                    Err(futures::stream::Aborted) => Err(EngineError::Preempted),
                }
            }
        }
    }

    /// Admit `stream` under `priority`. The returned stream holds its
    /// slot until it completes; background streams can be aborted
    /// mid-flight when an interactive request needs the slot.
    pub async fn admit(&self, priority: Priority, stream: TokenStream) -> TokenStream {
        let permit = self.acquire(priority).await;

        match priority {
            Priority::Interactive => Box::pin(async_stream::stream! {
                let _permit = permit;
                let mut stream = stream;
                while let Some(item) = stream.next().await {
                    yield item;
                }
            }),
            Priority::Background => {
                let (handle, registration) = AbortHandle::new_pair();
                self.background.lock().await.push(handle);
                let mut abortable = Abortable::new(stream, registration);
                Box::pin(async_stream::stream! {
                    let _permit = permit;
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
