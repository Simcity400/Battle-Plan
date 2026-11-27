use super::{Inner, MAX_CANCELLED, MAX_PENDING};
use dashboard_core::{QueryError, transport::bounded_text};
use dashboard_base::Source;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub(super) struct Request {
    pub(super) id: String,
    pub(super) cancelled: AtomicBool,
    pub(super) timing: Option<super::perf::RequestTiming>,
}

impl dashboard_base::presentation::Cancel for Request {
    fn flag(&self) -> &AtomicBool {
        &self.cancelled
    }
}

impl Request {
    pub(super) fn new(id: String) -> Arc<Self> {
        Arc::new(Self {
            id,
            cancelled: AtomicBool::new(false),
            timing: super::perf::RequestTiming::new(),
        })
    }

    pub(super) fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub(super) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

pub(super) struct Lease {
    pub(super) request: Arc<Request>,
    pub(super) source: Arc<Source>,
    pub(super) title_views: dashboard_politics::products::feed::index::TitleViews,
}

#[derive(Default)]
pub(super) struct Pending {
    pub(super) requests: BTreeMap<String, Arc<Request>>,
    pub(super) leases: BTreeMap<String, Lease>,
    pub(super) source_requests: BTreeMap<String, String>,
    pub(super) ready: VecDeque<(Arc<Request>, String)>,
    pub(super) errors: VecDeque<(Arc<Request>, QueryError)>,
    pub(super) cancelled: VecDeque<String>,
    pub(super) next_lease: u64,
}

impl Pending {
    pub(super) fn begin(&mut self, id: String) -> Option<Arc<Request>> {
        if self.cancelled.contains(&id)
            || self.requests.contains_key(&id)
            || self.source_requests.contains_key(&id)
            || self.requests.len() >= MAX_PENDING * 2
        {
            return None;
        }
        let at_capacity = self.requests.len() >= MAX_PENDING;
        let request = Request::new(id);
        self.requests
            .insert(request.id.clone(), Arc::clone(&request));
        if at_capacity {
            self.errors
                .push_back((request, dashboard_core::transport::CAPACITY_REACHED.into()));
            None
        } else {
            Some(request)
        }
    }

    pub(super) fn current(&self, request: &Arc<Request>) -> bool {
        !request.cancelled()
            && self
                .requests
                .get(&request.id)
                .is_some_and(|current| Arc::ptr_eq(current, request))
    }

    pub(super) fn rollback(&mut self, request: &Arc<Request>) -> Option<Lease> {
        let lease_id = self.source_requests.get(&request.id)?;
        if !self
            .leases
            .get(lease_id)
            .is_some_and(|lease| Arc::ptr_eq(&lease.request, request))
        {
            return None;
        }
        let lease_id = self.source_requests.remove(&request.id)?;
        self.leases.remove(&lease_id)
    }

    pub(super) fn finish(&mut self, request: &Arc<Request>, delivered: bool) -> Option<Lease> {
        if self
            .requests
            .get(&request.id)
            .is_some_and(|current| Arc::ptr_eq(current, request))
        {
            self.requests.remove(&request.id);
        }
        if delivered && !request.cancelled() {
            None
        } else {
            self.rollback(request)
        }
    }

    pub(super) fn cancel(&mut self, id: &str) -> Option<Lease> {
        if !self.cancelled.iter().any(|cancelled| cancelled == id) {
            if self.cancelled.len() == MAX_CANCELLED {
                self.cancelled.pop_front();
            }
            self.cancelled.push_back(id.to_owned());
        }
        if let Some(request) = self.requests.remove(id) {
            request.cancel();
        }
        self.ready.retain(|(request, _)| request.id != id);
        self.errors.retain(|(request, _)| request.id != id);
        let lease = self
            .source_requests
            .remove(id)
            .and_then(|lease_id| self.leases.remove(&lease_id));
        if let Some(lease) = &lease {
            lease.request.cancel();
        }
        lease
    }

    pub(super) fn stop(&mut self) -> Self {
        for request in self.requests.values() {
            request.cancel();
        }
        for lease in self.leases.values() {
            lease.request.cancel();
        }
        std::mem::take(self)
    }

    pub(super) fn complete(&mut self, request: Arc<Request>, result: Result<String, QueryError>) {
        if !self.current(&request) {
            return;
        }
        match result {
            Ok(json) => self.ready.push_back((request, json)),
            Err(error) => {
                let error = match error {
                    QueryError::Failed { message } => QueryError::Failed {
                        message: bounded_text(&message, 2048),
                    },
                    refused => refused,
                };
                self.errors.push_back((request, error));
            }
        }
    }
}

impl Inner {
    pub(super) fn lease_id(&self, sequence: u64) -> String {
        format!("lease-{}-{sequence}", self.lease_namespace)
    }

    pub(super) fn begin(&self, id: String) -> Option<Arc<Request>> {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.closed.load(Ordering::Acquire) {
            return None;
        }
        let request = pending.begin(id);
        drop(pending);
        self.signal.notify();
        request
    }

    pub(super) fn error(&self, request: Arc<Request>, message: QueryError) {
        self.complete(request, Err(message));
    }

    pub(super) fn complete(&self, request: Arc<Request>, result: Result<String, QueryError>) {
        if let Some(timing) = &request.timing {
            timing.completed();
        }
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !self.closed.load(Ordering::Acquire) {
            pending.complete(request, result);
        }
        drop(pending);
        self.signal.notify();
    }

    pub(super) fn finish(&self, request: &Arc<Request>, delivered: bool) {
        if let Some(timing) = &request.timing {
            let outcome = match (delivered, request.cancelled()) {
                (_, true) => "cancelled",
                (true, false) => "ok",
                (false, false) => "error",
            };
            timing.report(&request.id, outcome);
        }
        let lease = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .finish(request, delivered);
        drop(lease);
    }

    pub(super) fn stop(&self) {
        let pending = {
            let mut pending = self
                .pending
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            self.closed.store(true, Ordering::Release);
            pending.stop()
        };
        self.loader
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        drop(pending);
        self.content.close();
        for source in self.bound_sources() {
            source.bind(None, Default::default());
        }
        self.signal.notify();
    }
}
