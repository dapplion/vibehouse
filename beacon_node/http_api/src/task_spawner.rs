use crate::api_error::ApiError;
use beacon_processor::{BeaconProcessorSend, BlockingOrAsync, Work, WorkEvent};
use serde::Serialize;
use std::future::Future;
use tokio::sync::{mpsc::error::TrySendError, oneshot};
use types::EthSpec;
use warp::reply::{Reply, Response};

/// Maps a request to a queue in the `BeaconProcessor`.
#[derive(Clone, Copy)]
pub enum Priority {
    /// The highest priority.
    P0,
    /// The lowest priority.
    P1,
}

impl Priority {
    /// Wrap `self` in a `WorkEvent` with an appropriate priority.
    fn work_event<E: EthSpec>(&self, process_fn: BlockingOrAsync) -> WorkEvent<E> {
        let work = match self {
            Priority::P0 => Work::ApiRequestP0(process_fn),
            Priority::P1 => Work::ApiRequestP1(process_fn),
        };
        WorkEvent {
            drop_during_sync: false,
            work,
        }
    }
}

/// Spawns tasks on the `BeaconProcessor` or directly on the tokio executor.
#[derive(Clone)]
pub struct TaskSpawner<E: EthSpec> {
    /// Used to send tasks to the `BeaconProcessor`. The tokio executor will be
    /// used if this is `None`.
    beacon_processor_send: Option<BeaconProcessorSend<E>>,
}

impl<E: EthSpec> TaskSpawner<E> {
    pub fn new(beacon_processor_send: Option<BeaconProcessorSend<E>>) -> Self {
        Self {
            beacon_processor_send,
        }
    }

    /// Executes a "blocking" (non-async) task which returns an arbitrary value.
    pub async fn blocking_task<F, T>(self, priority: Priority, func: F) -> Result<T, ApiError>
    where
        F: FnOnce() -> Result<T, ApiError> + Send + Sync + 'static,
        T: Send + 'static,
    {
        if let Some(beacon_processor_send) = &self.beacon_processor_send {
            let (tx, rx) = oneshot::channel();
            let process_fn = move || {
                let func_result = func();
                let _ = tx.send(func_result);
            };

            send_to_beacon_processor(
                beacon_processor_send,
                priority,
                BlockingOrAsync::Blocking(Box::new(process_fn)),
                rx,
            )
            .await
            .and_then(|x| x)
        } else {
            tokio::task::spawn_blocking(func)
                .await
                .map_err(|_| ApiError::server_error("Tokio failed to spawn blocking task"))?
        }
    }

    /// Executes a "blocking" (non-async) task which returns a `Response`.
    pub async fn blocking_response_task<F, T>(self, priority: Priority, func: F) -> Response
    where
        F: FnOnce() -> Result<T, ApiError> + Send + Sync + 'static,
        T: Reply + Send + 'static,
    {
        let result = self.blocking_task(priority, func).await;
        crate::api_error::convert_api_error(result)
    }

    /// Executes a "blocking" (non-async) task which returns a JSON-serializable
    /// object.
    pub async fn blocking_json_task<F, T>(self, priority: Priority, func: F) -> Response
    where
        F: FnOnce() -> Result<T, ApiError> + Send + Sync + 'static,
        T: Serialize + Send + 'static,
    {
        let func = || func().map(|t| warp::reply::json(&t).into_response());
        self.blocking_response_task(priority, func).await
    }

    /// Executes an async task which may return an `ApiError`, which will be converted to a response.
    pub async fn spawn_async_with_rejection(
        self,
        priority: Priority,
        func: impl Future<Output = Result<Response, ApiError>> + Send + Sync + 'static,
    ) -> Response {
        let result = self
            .spawn_async_with_rejection_no_conversion(priority, func)
            .await;
        crate::api_error::convert_api_error(result)
    }

    /// Same as `spawn_async_with_rejection` but returning a result with the unhandled error.
    pub async fn spawn_async_with_rejection_no_conversion(
        self,
        priority: Priority,
        func: impl Future<Output = Result<Response, ApiError>> + Send + Sync + 'static,
    ) -> Result<Response, ApiError> {
        if let Some(beacon_processor_send) = &self.beacon_processor_send {
            let (tx, rx) = oneshot::channel();
            let process_fn = async move {
                let func_result = func.await;
                let _ = tx.send(func_result);
            };

            send_to_beacon_processor(
                beacon_processor_send,
                priority,
                BlockingOrAsync::Async(Box::pin(process_fn)),
                rx,
            )
            .await
            .and_then(|x| x)
        } else {
            tokio::task::spawn(func)
                .await
                .map_err(|_| ApiError::server_error("Tokio failed to spawn task"))?
        }
    }

    pub fn try_send(&self, work_event: WorkEvent<E>) -> Result<(), ApiError> {
        if let Some(beacon_processor_send) = &self.beacon_processor_send {
            let error_message = match beacon_processor_send.try_send(work_event) {
                Ok(()) => None,
                Err(TrySendError::Full(_)) => {
                    Some("The task was dropped. The server is overloaded.")
                }
                Err(TrySendError::Closed(_)) => {
                    Some("The task was dropped. The server is shutting down.")
                }
            };

            if let Some(error_message) = error_message {
                return Err(ApiError::server_error(error_message));
            };

            Ok(())
        } else {
            Err(ApiError::server_error(
                "The beacon processor is unavailable",
            ))
        }
    }
}

/// Send a task to the beacon processor and await execution.
async fn send_to_beacon_processor<E: EthSpec, T>(
    beacon_processor_send: &BeaconProcessorSend<E>,
    priority: Priority,
    process_fn: BlockingOrAsync,
    rx: oneshot::Receiver<T>,
) -> Result<T, ApiError> {
    let error_message = match beacon_processor_send.try_send(priority.work_event(process_fn)) {
        Ok(()) => match rx.await {
            Ok(func_result) => return Ok(func_result),
            Err(_) => "The task did not execute. The server is overloaded or shutting down.",
        },
        Err(TrySendError::Full(_)) => "The task was dropped. The server is overloaded.",
        Err(TrySendError::Closed(_)) => "The task was dropped. The server is shutting down.",
    };

    Err(ApiError::server_error(error_message))
}
