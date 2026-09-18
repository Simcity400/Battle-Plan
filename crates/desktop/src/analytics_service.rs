//! A single analytics worker with one replaceable pending request.
//!
//! Sliders and country changes cannot create an unbounded scan queue. Old jobs
//! cooperatively cancel, and cancellation from an old component cannot cancel
//! a newer request because each frontend request has its own identifier.

use analytics::{Analytics, CountrySeriesRequest};
use desktop_protocol::replay_revision;
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tauri::State;
use tokio::sync::oneshot;

const SUPERSEDED: &str = "Analytics request superseded";
type Reply = oneshot::Sender<Result<Value, String>>;

struct Job {
    id: String,
    path: String,
    series: Option<CountrySeriesRequest>,
    cancelled: Arc<AtomicBool>,
    reply: Reply,
}

#[derive(Default)]
struct Queue {
    next: Option<Job>,
    active: Option<(String, Arc<AtomicBool>)>,
    shutdown: bool,
}

pub struct AnalyticsService {
    shared: Arc<(Mutex<Queue>, Condvar)>,
}

impl Default for AnalyticsService {
    fn default() -> Self {
        let shared = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
        let worker = Arc::clone(&shared);
        std::thread::spawn(move || run_worker(worker));
        Self { shared }
    }
}

impl AnalyticsService {
    fn submit(
        &self,
        path: String,
        series: Option<CountrySeriesRequest>,
        id: String,
    ) -> Result<oneshot::Receiver<Result<Value, String>>, String> {
        if id.is_empty() || id.len() > 128 {
            return Err("Invalid analytics request identifier".into());
        }
        let (reply, receive) = oneshot::channel();
        let mut queue = self.shared.0.lock().map_err(|e| e.to_string())?;
        if queue.shutdown {
            return Err("The application is closing".into());
        }
        if let Some((_, cancel)) = &queue.active {
            cancel.store(true, Ordering::Relaxed);
        }
        let job = Job {
            id,
            path,
            series,
            cancelled: Arc::new(AtomicBool::new(false)),
            reply,
        };
        if let Some(old) = queue.next.replace(job) {
            old.cancelled.store(true, Ordering::Relaxed);
            let _ = old.reply.send(Err(SUPERSEDED.into()));
        }
        drop(queue);
        self.shared.1.notify_one();
        Ok(receive)
    }

    fn cancel(&self, id: &str) {
        let mut queue = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((active_id, cancel)) = &queue.active
            && active_id == id
        {
            cancel.store(true, Ordering::Relaxed);
        }
        if queue.next.as_ref().is_some_and(|job| job.id == id)
            && let Some(job) = queue.next.take()
        {
            job.cancelled.store(true, Ordering::Relaxed);
            let _ = job.reply.send(Err(SUPERSEDED.into()));
        }
    }

    pub fn shutdown(&self) {
        let mut queue = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
        queue.shutdown = true;
        if let Some((_, cancel)) = &queue.active {
            cancel.store(true, Ordering::Relaxed);
        }
        if let Some(job) = queue.next.take() {
            let _ = job.reply.send(Err("The application is closing".into()));
        }
        drop(queue);
        self.shared.1.notify_one();
    }
}

#[derive(PartialEq, Eq)]
struct FileKey {
    path: PathBuf,
    revision: String,
}

fn file_key(path: &str) -> Result<FileKey, String> {
    let path = crate::replay_path(path)?;
    let revision = replay_revision(&path).map_err(|e| e.to_string())?;
    Ok(FileKey { path, revision })
}

fn perform(job: &Job, cached: &mut Option<(FileKey, Analytics)>) -> Result<Value, String> {
    if job.cancelled.load(Ordering::Relaxed) {
        return Err(SUPERSEDED.into());
    }
    let key = file_key(&job.path)?;
    if cached.as_ref().is_none_or(|(existing, _)| *existing != key) {
        // A replay replacement releases the old compressed file before reading
        // the new one. Only one replay and one completed response stay cached.
        *cached = None;
        let analytics =
            Analytics::open_cancellable(&key.path, &job.cancelled).map_err(|e| e.to_string())?;
        *cached = Some((key, analytics));
    }
    let (loaded_key, analytics) = cached.as_mut().ok_or("Recording was not loaded")?;
    let mut result = match &job.series {
        Some(request) => serde_json::to_value(
            analytics
                .country_series(request, &job.cancelled)
                .map_err(|e| e.to_string())?,
        ),
        None => serde_json::to_value(analytics.metadata()),
    }
    .map_err(|e| e.to_string())?;
    if job.cancelled.load(Ordering::Relaxed) {
        return Err(SUPERSEDED.into());
    }
    result
        .as_object_mut()
        .ok_or("Analytics response is not an object")?
        .insert(
            "revision".into(),
            Value::String(loaded_key.revision.clone()),
        );
    Ok(result)
}

fn run_worker(shared: Arc<(Mutex<Queue>, Condvar)>) {
    let mut cached = None;
    loop {
        let job = {
            let mut queue = shared.0.lock().unwrap_or_else(|e| e.into_inner());
            while queue.next.is_none() && !queue.shutdown {
                queue = shared.1.wait(queue).unwrap_or_else(|e| e.into_inner());
            }
            if queue.shutdown {
                return;
            }
            let Some(job) = queue.next.take() else {
                continue;
            };
            queue.active = Some((job.id.clone(), Arc::clone(&job.cancelled)));
            job
        };
        let result = perform(&job, &mut cached);
        let result = if job.cancelled.load(Ordering::Relaxed) {
            Err(SUPERSEDED.into())
        } else {
            result
        };
        let _ = job.reply.send(result);
        let mut queue = shared.0.lock().unwrap_or_else(|e| e.into_inner());
        queue.active = None;
    }
}

#[tauri::command]
pub async fn analytics_metadata(
    service: State<'_, AnalyticsService>,
    path: String,
    request_id: String,
) -> Result<Value, String> {
    service
        .submit(path, None, request_id)?
        .await
        .map_err(|_| "Analytics worker stopped".to_string())?
}

#[tauri::command]
pub async fn analytics_series(
    service: State<'_, AnalyticsService>,
    path: String,
    request: CountrySeriesRequest,
    request_id: String,
) -> Result<Value, String> {
    service
        .submit(path, Some(request), request_id)?
        .await
        .map_err(|_| "Analytics worker stopped".to_string())?
}

#[tauri::command]
pub fn cancel_analytics(service: State<'_, AnalyticsService>, request_id: String) {
    service.cancel(&request_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obsolete_cancellation_does_not_cancel_new_work() {
        let current = Arc::new(AtomicBool::new(false));
        let queue = Queue {
            active: Some(("new".into(), Arc::clone(&current))),
            ..Default::default()
        };
        let service = AnalyticsService {
            shared: Arc::new((Mutex::new(queue), Condvar::new())),
        };
        service.cancel("old");
        assert!(!current.load(Ordering::Relaxed));
        service.cancel("new");
        assert!(current.load(Ordering::Relaxed));
    }
}
