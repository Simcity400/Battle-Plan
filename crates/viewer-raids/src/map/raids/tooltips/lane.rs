use super::{Content, Key, Tooltip};
use eframe::egui;
use replay_data::rows::TableId;
use replay_frame::frame::DecodedFrame;
use std::sync::{Arc, Condvar, Mutex};

#[derive(Clone)]
struct Request<C = Arc<Content>> {
    content: C,
    frame: Arc<DecodedFrame>,
    ours: u32,
    nato: bool,
    key: Key,
}
impl<C> Request<Arc<C>> {
    fn same(&self, other: &Self) -> bool {
        self.same_icon(other) && Arc::ptr_eq(&self.frame, &other.frame)
    }
    fn same_icon(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.content, &other.content)
            && self.ours == other.ours
            && self.nato == other.nato
            && self.key == other.key
    }
}
#[derive(Default)]
struct State {
    wanted: Option<Request>,
    pending: bool,
    result: Option<Arc<Tooltip>>,
    missed: Vec<TableId>,
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
        nato: bool,
        key: Key,
        ctx: &egui::Context,
    ) -> Option<Arc<Tooltip>> {
        let shared = self
            .shared
            .get_or_insert_with(|| Arc::new((Mutex::new(State::default()), Condvar::new())));
        if self.worker.is_none() {
            let shared = Arc::clone(shared);
            let ctx = ctx.clone();
            self.worker = Some(job_priority::spawn(move || run(shared, ctx)));
        }
        let request = Request {
            content,
            frame,
            ours,
            nato,
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
        if !state.wanted.as_ref().is_some_and(|old| old.same(&request)) && !state.pending {
            state.wanted = Some(request);
            state.pending = true;
            shared.1.notify_one();
        }
        state.result.clone()
    }

    pub fn take_missed(&mut self) -> Vec<TableId> {
        self.shared.as_ref().map_or_else(Vec::new, |shared| {
            std::mem::take(&mut shared.0.lock().unwrap_or_else(|e| e.into_inner()).missed)
        })
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
        let built = replay_frame::frame::discovering(|| {
            super::build(
                &request.content,
                &request.frame,
                request.ours,
                request.nato,
                &request.key,
            )
        });
        let mut state = shared.0.lock().unwrap_or_else(|e| e.into_inner());
        if state
            .wanted
            .as_ref()
            .is_some_and(|wanted| wanted.same(request))
        {
            publish(&mut state, built);
            viewer_runtime::worker_priority::at_least(Some(0), || ctx.request_repaint());
        }
    }
}

fn publish(state: &mut State, (text, missed): (Tooltip, Vec<TableId>)) {
    if missed.is_empty() {
        state.result = Some(Arc::new(text));
        return;
    }
    for table in missed {
        if !state.missed.contains(&table) {
            state.missed.push(table);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raid_request_keeps_old_hour_text_only_under_the_same_viewer_nato_choice() {
        let frame = Arc::new(DecodedFrame {
            branch: 0,
            hour: 10,
            tables: std::array::from_fn(|_| {
                replay_frame::frame::TableView::Unavailable {
                    reason: "not sampled".into(),
                }
                .into()
            }),
            metrics: Default::default(),
        });
        let old = Request {
            content: Arc::new(()),
            frame,
            ours: 7,
            nato: false,
            key: Key::Raid {
                id: 1,
                reference_type: 4,
                incoming: false,
            },
        };
        assert!(old.same(&old.clone()));
        let mut next = old.clone();
        next.frame = Arc::new((*old.frame).clone());
        Arc::make_mut(&mut next.frame).hour = 11;
        assert!(
            old.same_icon(&next),
            "playback may retain the old hour's text"
        );
        assert!(!old.same(&next), "the new hour must still build");
        next.nato = true;
        assert!(!old.same_icon(&next), "a toggle must clear old icon frames");
        assert!(!old.same(&next), "an in-flight old result must not publish");
        next.frame = Arc::clone(&old.frame);
        assert!(
            !old.same(&next),
            "the toggle rebuilds even at the same hour"
        );
        next.nato = false;
        next.ours = 8;
        assert!(!old.same_icon(&next));
        next.ours = old.ours;
        next.content = Arc::new(());
        assert!(
            !old.same_icon(&next),
            "another content object never shares text"
        );
    }

    #[test]
    fn a_build_that_read_an_undeclared_table_keeps_no_text_and_names_the_table() {
        let undeclared = replay_frame::frame::not_declared(TableId::Countries);
        let mut state = State::default();
        publish(
            &mut state,
            replay_frame::frame::discovering(|| {
                let _ = &*undeclared;
                Tooltip::default()
            }),
        );
        assert!(state.result.is_none(), "a build over an undeclared table is void");
        publish(&mut state, (Tooltip::default(), Vec::new()));
        assert!(state.result.is_some());
        let mut lane = Lane {
            shared: Some(Arc::new((Mutex::new(state), Condvar::new()))),
            worker: None,
        };
        assert_eq!(lane.take_missed(), [TableId::Countries]);
        assert!(lane.take_missed().is_empty(), "taken once");
    }

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
