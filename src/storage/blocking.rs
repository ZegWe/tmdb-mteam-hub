use std::fmt;
use std::sync::Arc;

use tokio::sync::Semaphore;

#[derive(Debug, Clone)]
pub(crate) struct BoundedBlockingExecutor {
    name: &'static str,
    permits: Arc<Semaphore>,
}

impl BoundedBlockingExecutor {
    pub(crate) fn try_new(
        name: &'static str,
        max_concurrency: usize,
    ) -> Result<Self, BlockingExecutorConfigError> {
        if name.trim().is_empty() {
            return Err(BlockingExecutorConfigError::EmptyName);
        }
        if max_concurrency == 0 {
            return Err(BlockingExecutorConfigError::ZeroConcurrency { name });
        }
        Ok(Self {
            name,
            permits: Arc::new(Semaphore::new(max_concurrency)),
        })
    }

    pub(crate) async fn run<T, F>(&self, operation: F) -> Result<T, BlockingTaskError>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let permit = Arc::clone(&self.permits)
            .acquire_owned()
            .await
            .map_err(|_| BlockingTaskError::Closed { name: self.name })?;
        let name = self.name;
        tokio::task::spawn_blocking(move || {
            // The permit belongs to the blocking closure rather than the awaiting future. If the
            // caller is cancelled, detached work still counts against the configured concurrency
            // limit until the closure actually returns.
            let _permit = permit;
            operation()
        })
        .await
        .map_err(|source| BlockingTaskError::Join { name, source })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockingExecutorConfigError {
    EmptyName,
    ZeroConcurrency { name: &'static str },
}

impl fmt::Display for BlockingExecutorConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName => formatter.write_str("blocking executor name must not be empty"),
            Self::ZeroConcurrency { name } => {
                write!(
                    formatter,
                    "{name} blocking concurrency must be greater than zero"
                )
            }
        }
    }
}

impl std::error::Error for BlockingExecutorConfigError {}

#[derive(Debug)]
pub(crate) enum BlockingTaskError {
    Closed {
        name: &'static str,
    },
    Join {
        name: &'static str,
        source: tokio::task::JoinError,
    },
}

impl BlockingTaskError {
    pub(crate) const fn is_closed(&self) -> bool {
        matches!(self, Self::Closed { .. })
    }
}

impl fmt::Display for BlockingTaskError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed { name } => {
                write!(formatter, "{name} blocking concurrency gate is closed")
            }
            Self::Join { name, source } => {
                write!(formatter, "{name} blocking task failed: {source}")
            }
        }
    }
}

impl std::error::Error for BlockingTaskError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Closed { .. } => None,
            Self::Join { source, .. } => Some(source),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use tokio::sync::oneshot;

    use super::BoundedBlockingExecutor;

    #[tokio::test(flavor = "current_thread")]
    async fn blocking_work_does_not_stall_a_single_thread_runtime() {
        let executor = BoundedBlockingExecutor::try_new("sqlite", 1).unwrap();
        let (started_tx, started_rx) = oneshot::channel();
        let (release_tx, release_rx) = mpsc::channel();

        let work = tokio::spawn(async move {
            executor
                .run(move || {
                    let _ = started_tx.send(());
                    release_rx.recv().expect("release blocking fixture");
                    7_u8
                })
                .await
                .unwrap()
        });
        started_rx.await.expect("blocking fixture started");

        tokio::time::timeout(Duration::from_millis(100), async {
            tokio::time::sleep(Duration::from_millis(10)).await;
        })
        .await
        .expect("Tokio timer must progress while blocking closure is waiting");

        release_tx.send(()).expect("release blocking fixture");
        assert_eq!(work.await.unwrap(), 7);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn caller_cancellation_does_not_release_a_live_blocking_permit() {
        let executor = BoundedBlockingExecutor::try_new("sqlite", 1).unwrap();
        let (started_tx, started_rx) = oneshot::channel();
        let (release_tx, release_rx) = mpsc::channel();

        let first_executor = executor.clone();
        let first = tokio::spawn(async move {
            first_executor
                .run(move || {
                    let _ = started_tx.send(());
                    release_rx.recv().expect("release cancelled fixture");
                })
                .await
        });
        started_rx.await.expect("cancelled fixture started");
        first.abort();

        let second_executor = executor.clone();
        let mut second = tokio::spawn(async move { second_executor.run(|| 9_u8).await.unwrap() });
        assert!(
            tokio::time::timeout(Duration::from_millis(30), &mut second)
                .await
                .is_err(),
            "a cancelled waiter must not free the permit while detached work is live"
        );

        release_tx.send(()).expect("release cancelled fixture");
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), second)
                .await
                .expect("second task should start after detached work returns")
                .unwrap(),
            9
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn independent_executors_do_not_starve_each_other() {
        let sqlite = BoundedBlockingExecutor::try_new("sqlite", 1).unwrap();
        let filesystem = BoundedBlockingExecutor::try_new("filesystem", 1).unwrap();
        let (started_tx, started_rx) = oneshot::channel();
        let (release_tx, release_rx) = mpsc::channel();

        let sqlite_task = tokio::spawn(async move {
            sqlite
                .run(move || {
                    let _ = started_tx.send(());
                    release_rx.recv().expect("release sqlite fixture");
                })
                .await
                .unwrap();
        });
        started_rx.await.expect("sqlite fixture started");

        let filesystem_result = tokio::time::timeout(
            Duration::from_millis(100),
            filesystem.run(|| "filesystem-ready"),
        )
        .await
        .expect("filesystem pool must remain available")
        .unwrap();
        assert_eq!(filesystem_result, "filesystem-ready");

        release_tx.send(()).expect("release sqlite fixture");
        sqlite_task.await.unwrap();
    }
}
