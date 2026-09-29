//! One coalescing hover worker. It retains only the latest request/result;
//! content and frame Arc identity bind results to their complete inputs.
use super::{Key, view::Prepared};
use crate::document::Content;
use eframe::egui;
use replay::frame::DecodedFrame;
use std::sync::{Arc, Condvar, Mutex};

#[derive(Clone)]
struct Request {
    content: Arc<Content>,
    frame: Arc<DecodedFrame>,
    ours: u32,
    key: Key,
}
impl Request {
    fn same(&self, other: &Self) -> bool {
        self.same_icon(other) && Arc::ptr_eq(&self.frame, &other.frame)
    }
    /// The same icon in another hour keeps its last text until the new one
    /// is built, so playback does not blank the popup every hour.
    fn same_icon(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.content, &other.content)
            && self.ours == other.ours
            && self.key == other.key
    }
}
#[derive(Default)]
struct State {
    wanted: Option<Request>,
    pending: bool,
    result: Option<Arc<Prepared>>,
    stop: bool,
}
type Shared = Arc<(Mutex<State>, Condvar)>;

#[derive(Default)]
pub struct Lane {
    shared: Option<Shared>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Lane {
    pub fn request(
        &mut self,
        content: Arc<Content>,
        frame: Arc<DecodedFrame>,
        ours: u32,
        key: Key,
        ctx: &egui::Context,
    ) -> Option<Arc<Prepared>> {
        let shared = self
            .shared
            .get_or_insert_with(|| Arc::new((Mutex::new(State::default()), Condvar::new())));
        if self.worker.is_none() {
            let shared = Arc::clone(shared);
            let ctx = ctx.clone();
            self.worker = Some(std::thread::spawn(move || run(shared, ctx)));
        }
        let request = Request {
            content,
            frame,
            ours,
            key,
        };
        let mut state = shared.0.lock().unwrap_or_else(|e| e.into_inner());
        let same_icon = state
            .wanted
            .as_ref()
            .is_some_and(|old| old.same_icon(&request));
        if !same_icon {
            state.result = None;
        }
        // One active and one queued request. The worker retains the active
        // request while waiting, so the UI never destroys the last frame Arc.
        if !state.wanted.as_ref().is_some_and(|old| old.same(&request)) && !state.pending {
            state.wanted = Some(request);
            state.pending = true;
            shared.1.notify_one();
        }
        state.result.clone()
    }
}
impl Drop for Lane {
    fn drop(&mut self) {
        if let Some(shared) = &self.shared {
            shared.0.lock().unwrap_or_else(|e| e.into_inner()).stop = true;
            shared.1.notify_one();
        }
    }
}
fn run(shared: Shared, ctx: egui::Context) {
    let mut held = None;
    loop {
        let next = {
            let mut state = shared.0.lock().unwrap_or_else(|e| e.into_inner());
            while !state.stop && !state.pending {
                state = shared.1.wait(state).unwrap_or_else(|e| e.into_inner());
            }
            if state.stop {
                return;
            }
            state.pending = false;
            state.wanted.clone().expect("pending request")
        };
        let request = held.insert(next);
        let result = super::build(&request.content, &request.frame, request.ours, &request.key);
        let result = super::view::prepare(result, &request.content.icons);
        let mut state = shared.0.lock().unwrap_or_else(|e| e.into_inner());
        if state
            .wanted
            .as_ref()
            .is_some_and(|wanted| wanted.same(request))
        {
            state.result = Some(Arc::new(result));
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dropping_an_idle_lane_wakes_and_stops_its_worker() {
        let shared = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let worker_shared = Arc::clone(&shared);
        let worker = std::thread::spawn(move || run(worker_shared, egui::Context::default()));
        let mut lane = Lane {
            shared: Some(Arc::clone(&shared)),
            worker: Some(worker),
        };
        let worker = lane.worker.take().unwrap();
        drop(lane);
        worker.join().unwrap();
        assert!(shared.0.lock().unwrap().stop);
    }
}
