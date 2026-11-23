use super::{Inner, MAX_SERIALIZED_BYTES, Request, ThreadExit};
use dashboard_core::transport::{DataEvent, MAX_REPLY_BYTES, ReplyChunks};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

#[derive(Default)]
pub(super) struct Signal {
    pub(super) version: AtomicU64,
    pub(super) mutex: Mutex<()>,
    pub(super) changed: Condvar,
}

impl Signal {
    pub(super) fn notify(&self) {
        let _guard = self.mutex.lock().unwrap_or_else(|error| error.into_inner());
        self.version.fetch_add(1, Ordering::Release);
        self.changed.notify_all();
    }
}

pub(super) fn run_pump(inner: Arc<Inner>, emit: Arc<dyn Fn(DataEvent) -> bool + Send + Sync>) {
    let _exit = ThreadExit(Arc::clone(&inner));
    let mut streams = VecDeque::<(Arc<Request>, ReplyChunks)>::new();
    while !inner.closed.load(Ordering::Acquire) {
        let observed = inner.signal.version.load(Ordering::Acquire);
        let mut pending = inner
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        streams.retain(|(request, _)| !request.cancelled());
        let error = pending.errors.pop_front();
        let serialized_bytes: usize = streams
            .iter()
            .map(|(_, chunks)| chunks.retained_bytes())
            .sum();
        let ready = if serialized_bytes <= MAX_SERIALIZED_BYTES - MAX_REPLY_BYTES - 128 {
            pending.ready.pop_front()
        } else {
            None
        };
        drop(pending);
        let mut progressed = false;
        if let Some((request, message)) = error {
            if !emit_error(&inner, &request, message.into(), &*emit) {
                return;
            }
            progressed = true;
        }
        if let Some((request, json)) = ready {
            if request.cancelled() || inner.closed.load(Ordering::Acquire) {
                inner.finish(&request, false);
            } else {
                if let Some(timing) = &request.timing {
                    timing.bytes(json.len());
                }
                match ReplyChunks::new(request.id.clone(), json) {
                    Ok(chunks)
                        if serialized_bytes.saturating_add(chunks.retained_bytes())
                            <= MAX_SERIALIZED_BYTES =>
                    {
                        streams.push_back((request, chunks));
                    }
                    Ok(_) => {
                        if !emit_error(
                            &inner,
                            &request,
                            "Projected replies exceed the transport byte budget".into(),
                            &*emit,
                        ) {
                            return;
                        }
                    }
                    Err(message) => {
                        if !emit_error(&inner, &request, message.into(), &*emit) {
                            return;
                        }
                    }
                }
            }
            progressed = true;
        }
        if let Some((request, mut chunks)) = streams.pop_front() {
            if request.cancelled() || inner.closed.load(Ordering::Acquire) {
                inner.finish(&request, false);
            } else if let Some(event) = chunks.next() {
                let last = matches!(&event, DataEvent::Chunk { last: true, .. });
                match deliver(&request, &inner.closed, event, &*emit) {
                    Delivery::Closed => return,
                    Delivery::Cancelled => inner.finish(&request, false),
                    Delivery::Sent if last => inner.finish(&request, true),
                    Delivery::Sent => streams.push_back((request, chunks)),
                }
            } else {
                inner.finish(&request, false);
            }
            progressed = true;
        }
        if !progressed {
            let guard = inner
                .signal
                .mutex
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if !inner.closed.load(Ordering::Acquire)
                && inner.signal.version.load(Ordering::Acquire) == observed
            {
                drop(
                    inner
                        .signal
                        .changed
                        .wait(guard)
                        .unwrap_or_else(|error| error.into_inner()),
                );
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Delivery {
    Sent,
    Cancelled,
    Closed,
}

pub(super) fn deliver(
    request: &Request,
    closed: &AtomicBool,
    event: DataEvent,
    emit: &(dyn Fn(DataEvent) -> bool + Send + Sync),
) -> Delivery {
    if request.cancelled() || closed.load(Ordering::Acquire) {
        return Delivery::Cancelled;
    }
    if !emit(event) {
        Delivery::Closed
    } else if request.cancelled() || closed.load(Ordering::Acquire) {
        Delivery::Cancelled
    } else {
        Delivery::Sent
    }
}

pub(super) fn emit_error(
    inner: &Inner,
    request: &Arc<Request>,
    error: dashboard_core::QueryError,
    emit: &(dyn Fn(DataEvent) -> bool + Send + Sync),
) -> bool {
    let lease = inner
        .pending
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .rollback(request);
    drop(lease);
    let delivery = deliver(
        request,
        &inner.closed,
        DataEvent::Error {
            id: request.id.clone(),
            error,
        },
        emit,
    );
    inner.finish(request, false);
    delivery != Delivery::Closed
}
