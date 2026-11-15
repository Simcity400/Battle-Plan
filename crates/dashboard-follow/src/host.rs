//! Follow hosted on a playhead: its reader declared and bound, and a driver that advances the
//! playhead to its clock's head sixty times a second and takes the steps for the reader, so the
//! stepper runs whether or not a map is drawn; the follow's messages go as it takes their steps
//! ([`Due`]), and the playhead is kept exact while paused, so a followed value is the clock's hour's.
//! The hook itself is the owner's to place (the viewer's host slot, or `Playhead::new` in a bench),
//! with [`Due::sink`]. Panels shown on the playhead are evaluated on a thread of their own from rows
//! at the head the owner provides ([`PanelRows`]).

use crate::{
    Follow,
    due::Due,
    live::{Group, GroupState, Live, cells_id},
    panel::{self, Panel, PanelData, PanelReader, PanelRows},
    push::{PANEL, Push},
    registry::{Anchor, Hosted, lacking},
};
use serde_json::Value;
use replay_frame::frame::DecodedFrame;
use std::collections::BTreeMap;
use playhead::{
    Playhead, Reader,
    pages::{Pages, Run},
};
use replay_columnar::{
    manifest::Status,
    unit::{BLOCK, reader},
};
use replay_data::Hour;
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const FRAME: Duration = Duration::from_micros(16_667);

/// Which tables `pages` holds: those with a restore in every run (its first, or one at a block's
/// end where a run is read from a later block) and some hour recorded. A table every hour of which
/// is "not recorded" (a derived table the files only name) is not.
pub fn held_tables(pages: &dyn Pages, tables: usize) -> Vec<bool> {
    (0..tables)
        .map(|table| {
            let restored = pages.runs().iter().all(|run| run.pages.iter().filter_map(|page| page.tables.get(table)).any(|entry| entry.first.is_some() || entry.restore.is_some()));
            restored && pages.runs().iter().any(|run| recorded(run, table))
        })
        .collect()
}

/// Whether any hour of `run` records `table`.
fn recorded(run: &Run, table: usize) -> bool {
    run.pages.iter().enumerate().any(|(block, page)| {
        let Some(entry) = page.tables.get(table) else { return false };
        let first = run.first_hour + (block as u32 * BLOCK) as Hour;
        let mut hour = 0;
        loop {
            match reader::status(entry, hour, first) {
                Ok((status, ..)) if status != Status::NotRecorded => return true,
                Ok((_, _, _, Some(next))) if next > hour => hour = next,
                _ => return false,
            }
        }
    })
}

/// One panel shown: its rows reader, the frame its data was made from, the data and live cells
/// pushed last and the hour they were pushed at, and each of those cells' groups followed, with
/// its follow id.
struct Shown {
    panel: Box<dyn Panel>,
    reader: Box<dyn PanelReader>,
    frame: Option<Arc<DecodedFrame>>,
    data: Option<PanelData>,
    live: Option<Live>,
    pushed: Option<Hour>,
    cells: Vec<(Group, u32)>,
}

impl Shown {
    fn new(panel: Box<dyn Panel>, reader: Box<dyn PanelReader>) -> Self {
        Self { panel, reader, frame: None, data: None, live: None, pushed: None, cells: Vec::new() }
    }
}

/// The groups followed under `ids` let go.
fn release(ids: &mut Vec<(Group, u32)>, cells: Option<&HostCells>) {
    if let Some(cells) = cells {
        for (_, id) in ids.drain(..) {
            let _ = cells.follow.unfollow(id);
        }
    }
    ids.clear();
}

/// Where panels' live cells are followed: the playhead's follow, and which tables its files hold.
struct HostCells {
    follow: Follow,
    held: Vec<bool>,
}

/// `data` as pushed: with `live` its cells, each group followed under the id it was followed
/// under before when it is the same group (same table, columns and keys: the reader keeps its
/// values, never blank while a new id waits for its first answer), else under a new id, or why it
/// is not followed. The groups followed before for it and named no more are let go.
fn with_cells(data: &PanelData, live: Option<&Live>, ids: &mut Vec<(Group, u32)>, cells: Option<&HostCells>) -> PanelData {
    let (Ok(reply @ Value::Object(_)), Some(live)) = (data, live.filter(|live| !live.is_empty())) else {
        release(ids, cells);
        return data.clone();
    };
    let mut kept = Vec::with_capacity(live.groups.len());
    let states: Vec<GroupState> = live
        .groups
        .iter()
        .map(|group| {
            let Some(cells) = cells else { return GroupState::Unavailable("This playhead follows no live cells".into()) };
            if let Some(reason) = lacking(group.table.index(), cells.held.get(group.table.index()).copied().unwrap_or(false)) {
                return GroupState::Unavailable(reason);
            }
            if let Some(at) = ids.iter().position(|(held, _)| held == group) {
                let (held, id) = ids.swap_remove(at);
                kept.push((held, id));
                return GroupState::Follow(id);
            }
            let id = cells_id();
            match cells.follow.follow(id, group.query()) {
                Ok(()) => {
                    kept.push((group.clone(), id));
                    GroupState::Follow(id)
                }
                Err(error) => GroupState::Unavailable(error.to_string()),
            }
        })
        .collect();
    release(ids, cells);
    *ids = kept;
    let mut reply = reply.clone();
    if let Some(fields) = reply.as_object_mut() {
        fields.insert("live".into(), live.json(&states));
    }
    Ok(reply)
}

/// What the panels' thread measured: each evaluation's nanoseconds, and the messages and bytes pushed.
#[derive(Clone, Debug, Default)]
pub struct PanelStats {
    pub evaluations: Vec<u64>,
    pub pushed: u64,
    pub bytes: u64,
}

#[derive(Default)]
struct Panels {
    rows: Option<Arc<dyn PanelRows>>,
    push: Option<Arc<Push>>,
    shown: BTreeMap<u32, Shown>,
    thread: Option<JoinHandle<()>>,
    stats: PanelStats,
    cells: Option<HostCells>,
}

impl Panels {
    /// Panel `id` goes, its live cells with it.
    fn remove(&mut self, id: u32) {
        if let Some(mut shown) = self.shown.remove(&id) {
            release(&mut shown.cells, self.cells.as_ref());
        }
    }
}

/// The message a panic carried.
fn panicked(payload: &(dyn std::any::Any + Send)) -> String {
    payload.downcast_ref::<&str>().map(|text| (*text).to_owned()).or_else(|| payload.downcast_ref::<String>().cloned()).unwrap_or_else(|| "a panic with no message".to_owned())
}

/// Each panel whose rows moved: its data made again, and pushed, in one message, when it or its
/// live cells differ, or when the head went back before the hour it was last pushed at (a seek:
/// the reader would otherwise hold a reply dated after the clock); the cells followed are replaced
/// with them. A panel whose rows stopped, or whose data panicked, is refused by name; the others go on.
fn evaluate(panels: &mut Panels) {
    let mut changed = Vec::new();
    let cells = panels.cells.as_ref();
    for (&id, shown) in &mut panels.shown {
        if let Some((hour, reason)) = shown.reader.stopped() {
            let data: PanelData = Err(reason);
            if shown.data.as_ref() != Some(&data) {
                changed.push((id, hour, with_cells(&data, None, &mut shown.cells, cells)));
                (shown.frame, shown.live) = (None, None);
                shown.data = Some(data);
            }
            continue;
        }
        let Some(frame) = shown.reader.frame() else { continue };
        if shown.frame.as_ref().is_some_and(|last| Arc::ptr_eq(last, &frame)) {
            continue;
        }
        let started = Instant::now();
        let data = catch_unwind(AssertUnwindSafe(|| shown.panel.data(&frame))).unwrap_or_else(|payload| Err(format!("This panel could not be made at hour {} ({})", frame.hour, panicked(&*payload))));
        panels.stats.evaluations.push(started.elapsed().as_nanos() as u64);
        shown.frame = Some(Arc::clone(&frame));
        let live = shown.panel.live().filter(|_| data.is_ok());
        let back = shown.pushed.is_some_and(|pushed| frame.hour < pushed);
        if back || shown.data.as_ref() != Some(&data) || shown.live.as_ref() != live {
            changed.push((id, frame.hour, with_cells(&data, live, &mut shown.cells, cells)));
            shown.live = live.cloned();
            shown.data = Some(data);
            shown.pushed = Some(frame.hour);
        }
    }
    if let (false, Some(push)) = (changed.is_empty(), &panels.push) {
        let message = panel::message(&changed.iter().map(|(id, hour, data)| (*id, *hour, data)).collect::<Vec<_>>());
        panels.stats.pushed += 1;
        panels.stats.bytes += message.len() as u64;
        push.offer_kind(PANEL, message);
    }
}

pub struct PlayheadHost<E: Send + Sync + 'static> {
    owner: Arc<E>,
    panels: Arc<Mutex<Panels>>,
    get: fn(&E) -> &Playhead,
    /// The owner's one anchor for its (recording, timeline), which drops repeats.
    anchored: fn(&E, Anchor),
    follow: Follow,
    reader: Reader,
    held: Vec<bool>,
    last: Mutex<Option<Anchor>>,
    stop: Arc<AtomicBool>,
    driver: Mutex<Option<JoinHandle<()>>>,
}

impl<E: Send + Sync + 'static> PlayheadHost<E> {
    /// `follow` on the playhead `get` finds in `owner`, whose files hold the tables `held` marks,
    /// its hook's messages held in `due`; anchors go through `anchored`, and while `drawn` holds a
    /// map advances the playhead and the driver only takes its steps.
    pub fn new(owner: Arc<E>, get: fn(&E) -> &Playhead, anchored: fn(&E, Anchor), drawn: fn(&E) -> bool, follow: Follow, due: Arc<Due>, held: Vec<bool>) -> std::io::Result<Arc<Self>> {
        let playhead = get(&owner);
        playhead.exact_when_paused();
        let reader = playhead.declare(Vec::new(), None)?;
        let mut consumer = playhead.consumer(reader);
        let bound = Arc::downgrade(&owner);
        follow.bind(move |tables| bound.upgrade().map_or(Ok(()), |owner| get(&owner).redeclare(reader, tables)), playhead.log_reads())?;
        let stop = Arc::new(AtomicBool::new(false));
        let (driven, stopping) = (Arc::clone(&owner), Arc::clone(&stop));
        let driver = std::thread::Builder::new().name("follow driver".into()).spawn(move || {
            let mut taken = Vec::new();
            let mut next = Instant::now();
            while !stopping.load(Ordering::Acquire) {
                let playhead = get(&driven);
                let head = playhead.head(Instant::now());
                if !drawn(&driven) {
                    playhead.advance(head);
                }
                consumer.take(head, &mut taken);
                if let Some(step) = taken.last() {
                    due.reached(step.epoch(), step.last());
                }
                taken.clear();
                next += FRAME;
                let now = Instant::now();
                if next > now {
                    std::thread::sleep(next - now);
                } else {
                    next = now;
                }
            }
        })?;
        let panels = Arc::new(Mutex::new(Panels { cells: Some(HostCells { follow: follow.clone(), held: held.clone() }), ..Panels::default() }));
        Ok(Arc::new(Self { owner, panels, get, anchored, follow, reader, held, last: Mutex::new(None), stop, driver: Mutex::new(Some(driver)) }))
    }

    pub fn playhead(&self) -> &Playhead {
        (self.get)(&self.owner)
    }

    /// Panels from now on: their rows from `rows`, their data pushed to `push`.
    pub fn show_panels(&self, rows: Arc<dyn PanelRows>, push: Arc<Push>) {
        let mut panels = self.panels.lock().unwrap_or_else(PoisonError::into_inner);
        panels.rows = Some(rows);
        panels.push = Some(push);
    }

    pub fn panel_stats(&self) -> PanelStats {
        self.panels.lock().unwrap_or_else(PoisonError::into_inner).stats.clone()
    }
}

impl<E: Send + Sync + 'static> Hosted for PlayheadHost<E> {
    fn follow(&self) -> &Follow {
        &self.follow
    }

    fn lacks(&self, table: usize) -> Option<String> {
        lacking(table, self.held.get(table).copied().unwrap_or(false))
    }

    fn panel(&self, id: u32, panel: Box<dyn Panel>) -> Result<(), String> {
        let mut panels = self.panels.lock().unwrap_or_else(PoisonError::into_inner);
        let rows = panels.rows.clone().ok_or("This playhead's host gives panels no rows at the head")?;
        let reader = rows.reader(panel.tables()).map_err(|error| format!("Rows at the head are refused: {error}"))?;
        panels.remove(id);
        panels.shown.insert(id, Shown::new(panel, reader));
        if panels.thread.is_none() {
            let (held, stopping) = (Arc::downgrade(&self.panels), Arc::clone(&self.stop));
            let thread = std::thread::Builder::new()
                .name("follow panels".into())
                .spawn(move || {
                    let mut next = Instant::now();
                    while !stopping.load(Ordering::Acquire) {
                        let Some(panels) = held.upgrade() else { return };
                        evaluate(&mut panels.lock().unwrap_or_else(PoisonError::into_inner));
                        drop(panels);
                        next += FRAME;
                        let now = Instant::now();
                        if next > now {
                            std::thread::sleep(next - now);
                        } else {
                            next = now;
                        }
                    }
                })
                .map_err(|error| error.to_string())?;
            panels.thread = Some(thread);
        }
        Ok(())
    }

    fn unpanel(&self, id: u32) {
        self.panels.lock().unwrap_or_else(PoisonError::into_inner).remove(id);
    }

    fn anchor(&self, anchor: Anchor) {
        let mut last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
        if *last == Some(anchor) {
            return;
        }
        *last = Some(anchor);
        drop(last);
        (self.anchored)(&self.owner, anchor);
    }
}

impl<E: Send + Sync + 'static> Drop for PlayheadHost<E> {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(driver) = self.driver.lock().unwrap_or_else(PoisonError::into_inner).take() {
            let _ = driver.join();
        }
        let thread = self.panels.lock().unwrap_or_else(PoisonError::into_inner).thread.take();
        if let Some(thread) = thread {
            let _ = thread.join();
        }
        self.playhead().retire(self.reader);
    }
}

#[cfg(test)]
mod tests {
    use super::{HostCells, Panels, Shown, evaluate};
    use crate::{
        Follow,
        live::{CELLS, Cells, Live, Source},
        panel::{Panel, PanelData, PanelReader},
        push::{PANEL, Push},
    };
    use replay_data::{Hour, rows::TableId};
    use replay_frame::frame::{DecodedFrame, not_selected};
    use std::sync::{Arc, Mutex};

    /// Its data is the hour's tens; refused at hours past 200.
    struct Tens;

    impl Panel for Tens {
        fn tables(&self) -> &[TableId] {
            &[TableId::Countries]
        }

        fn data(&mut self, frame: &DecodedFrame) -> PanelData {
            if frame.hour > 200 {
                return Err("past the fixture".into());
            }
            Ok(serde_json::json!({ "tens": frame.hour / 10 }))
        }
    }

    struct Given(Arc<Mutex<Option<Arc<DecodedFrame>>>>);

    impl PanelReader for Given {
        fn frame(&self) -> Option<Arc<DecodedFrame>> {
            self.0.lock().unwrap().clone()
        }
    }

    fn frame(hour: Hour) -> Arc<DecodedFrame> {
        Arc::new(DecodedFrame { branch: 1, hour, tables: std::array::from_fn(|_| not_selected()), metrics: Default::default() })
    }

    fn pushed(push: &Push) -> serde_json::Value {
        let (kind, bytes, ..) = push.take_kind().unwrap();
        assert_eq!(kind, PANEL);
        serde_json::from_slice(&bytes[4..]).unwrap()
    }

    #[test]
    fn a_panel_is_made_again_only_when_its_rows_move_and_pushed_only_when_its_data_differs() {
        let given = Arc::new(Mutex::new(None));
        let push = Arc::new(Push::default());
        let mut panels = Panels { push: Some(Arc::clone(&push)), ..Default::default() };
        panels.shown.insert(7, Shown::new(Box::new(Tens), Box::new(Given(Arc::clone(&given)))));
        evaluate(&mut panels);
        assert_eq!((panels.stats.evaluations.len(), push.counters().offered), (0, 0), "nothing while its rows load");
        let first = frame(100);
        *given.lock().unwrap() = Some(Arc::clone(&first));
        evaluate(&mut panels);
        evaluate(&mut panels);
        assert_eq!((panels.stats.evaluations.len(), push.counters().offered), (1, 1), "the same rows are not made again");
        *given.lock().unwrap() = Some(frame(105));
        evaluate(&mut panels);
        assert_eq!((panels.stats.evaluations.len(), push.counters().offered), (2, 1), "new rows, the same data: nothing pushed");
        // A frame the reader hands over (a seek or a pause hands one at once) is evaluated and pushed
        // at the next evaluation: the host holds no cadence of its own.
        *given.lock().unwrap() = Some(frame(112));
        evaluate(&mut panels);
        assert_eq!(push.counters().offered, 2);
        assert_eq!(pushed(&push)["panels"][0]["data"]["tens"], 10);
        let body = pushed(&push);
        assert_eq!(body["panels"][0]["id"], 7);
        assert_eq!(body["panels"][0]["hour"], 112);
        assert_eq!(body["panels"][0]["data"]["tens"], 11);
        assert!(body["panels"][0]["refused"].is_null());
        *given.lock().unwrap() = Some(frame(201));
        evaluate(&mut panels);
        *given.lock().unwrap() = Some(frame(202));
        evaluate(&mut panels);
        assert_eq!(push.counters().offered, 3, "the same refusal is pushed once");
        let body = pushed(&push);
        assert_eq!(body["panels"][0]["refused"], "past the fixture");
        assert!(body["panels"][0]["data"].is_null());
    }

    #[test]
    fn a_seek_back_before_a_panels_last_push_pushes_it_again_at_the_hour_sought_though_its_data_is_the_same() {
        let given = Arc::new(Mutex::new(Some(frame(128))));
        let push = Arc::new(Push::default());
        let mut panels = Panels { push: Some(Arc::clone(&push)), ..Default::default() };
        panels.shown.insert(7, Shown::new(Box::new(Tens), Box::new(Given(Arc::clone(&given)))));
        evaluate(&mut panels);
        assert_eq!(pushed(&push)["panels"][0]["hour"], 128);
        *given.lock().unwrap() = Some(frame(129));
        evaluate(&mut panels);
        assert_eq!(push.counters().offered, 1, "forward with the same data: nothing pushed");
        *given.lock().unwrap() = Some(frame(121));
        evaluate(&mut panels);
        assert_eq!(push.counters().offered, 2, "back before the last push: pushed though the data is the same");
        let body = pushed(&push);
        assert_eq!((&body["panels"][0]["hour"], &body["panels"][0]["data"]["tens"]), (&serde_json::json!(121), &serde_json::json!(12)), "dated the hour sought, never after the clock");
    }

    /// Panics at every frame.
    struct Broken;

    impl Panel for Broken {
        fn tables(&self) -> &[TableId] {
            &[TableId::Countries]
        }

        fn data(&mut self, _frame: &DecodedFrame) -> PanelData {
            panic!("a broken product")
        }
    }

    /// Rows that stopped for good.
    struct Stopped;

    impl PanelReader for Stopped {
        fn frame(&self) -> Option<Arc<DecodedFrame>> {
            None
        }

        fn stopped(&self) -> Option<(Hour, String)> {
            Some((90, "Rows at the head stopped: a table failed".into()))
        }
    }

    #[test]
    fn a_panel_that_panics_or_whose_rows_stopped_is_refused_by_name_and_the_others_go_on() {
        let given = Arc::new(Mutex::new(Some(frame(100))));
        let push = Arc::new(Push::default());
        let mut panels = Panels { push: Some(Arc::clone(&push)), ..Default::default() };
        panels.shown.insert(1, Shown::new(Box::new(Broken), Box::new(Given(Arc::clone(&given)))));
        panels.shown.insert(2, Shown::new(Box::new(Tens), Box::new(Stopped)));
        panels.shown.insert(3, Shown::new(Box::new(Tens), Box::new(Given(Arc::clone(&given)))));
        evaluate(&mut panels);
        let body = pushed(&push);
        let panel = |id: u64| body["panels"].as_array().unwrap().iter().find(|panel| panel["id"] == id).cloned().unwrap_or_else(|| panic!("panel {id} is pushed: {body}"));
        let broken = panel(1)["refused"].as_str().map(str::to_owned).unwrap_or_default();
        assert!(broken.contains("a broken product") && broken.contains("100"), "a panic is the panel's refusal by name, at its hour: {broken}");
        assert_eq!((&panel(2)["hour"], &panel(2)["refused"]), (&serde_json::json!(90), &serde_json::json!("Rows at the head stopped: a table failed")), "never left loading");
        assert_eq!(panel(3)["data"]["tens"], 10, "the others are made as before");
        evaluate(&mut panels);
        assert_eq!(push.counters().offered, 1, "each refusal is pushed once");
    }

    /// Its structure is which countries it shows: 100 and 101 before hour 150, then 102 too; each
    /// one's political power (column 2) is live, its command power (column 3) from hour 150, and
    /// its stability from a table the playhead's files do not hold.
    struct Powers {
        live: Option<Live>,
    }

    impl Panel for Powers {
        fn tables(&self) -> &[TableId] {
            &[TableId::Countries]
        }

        fn data(&mut self, frame: &DecodedFrame) -> PanelData {
            let keys: &[u128] = if frame.hour < 150 { &[100, 101] } else { &[100, 101, 102] };
            let cells = Cells::default();
            for &key in keys {
                let row = key.to_string();
                cells.value(&row, "power", Source { table: TableId::Countries, column: 2, key }, 0.001);
                if frame.hour >= 150 {
                    cells.ratio(&row, "share", Source { table: TableId::Countries, column: 3, key }, Source { table: TableId::Countries, column: 3, key: 100 }, 1.0);
                }
                cells.value(&row, "stability", Source { table: TableId::CountryStabilityInputs, column: 0, key }, 1.0);
            }
            self.live = Some(cells.finish());
            Ok(serde_json::json!({ "rows": keys.iter().map(u128::to_string).collect::<Vec<_>>() }))
        }

        fn live(&self) -> Option<&Live> {
            self.live.as_ref()
        }
    }

    fn declared(follow: &Follow) -> Vec<(usize, Vec<usize>)> {
        follow.declared().unwrap().into_iter().map(|declared| (declared.table, declared.columns)).collect()
    }

    #[test]
    fn a_panels_live_cells_are_followed_by_key_and_replaced_when_its_structure_changes() {
        let given = Arc::new(Mutex::new(None));
        let push = Arc::new(Push::default());
        let follow = Follow::new(1 << 16);
        let mut held = vec![false; TableId::ALL.len()];
        held[TableId::Countries.index()] = true;
        let mut panels = Panels { push: Some(Arc::clone(&push)), cells: Some(HostCells { follow: follow.clone(), held }), ..Default::default() };
        panels.shown.insert(7, Shown::new(Box::new(Powers { live: None }), Box::new(Given(Arc::clone(&given)))));
        *given.lock().unwrap() = Some(frame(100));
        evaluate(&mut panels);
        let body = pushed(&push);
        let live = &body["panels"][0]["data"]["live"];
        assert_eq!(body["panels"][0]["data"]["rows"], serde_json::json!(["100", "101"]), "the structure as the product made it");
        let groups = live["groups"].as_array().unwrap();
        assert_eq!(groups.len(), 2, "one followed query a table and column set: {live}");
        let countries = groups.iter().find(|group| group["table"] == TableId::Countries.index()).unwrap();
        assert_eq!((&countries["columns"], &countries["keys"]), (&serde_json::json!([2]), &serde_json::json!(["100", "101"])));
        let first = countries["follow"].as_u64().unwrap() as u32;
        assert_ne!(first & CELLS, 0, "a host's id, never one the UI takes");
        let lacking = groups.iter().find(|group| group["table"] == TableId::CountryStabilityInputs.index()).unwrap();
        assert!(lacking["follow"].is_null());
        assert!(lacking["unavailable"].as_str().unwrap().contains("not in this playhead's files"), "a table the playhead lacks is missing by name: {lacking}");
        assert_eq!(live["rows"], serde_json::json!(["100", "101"]));
        assert_eq!(live["kinds"][0], serde_json::json!({ "id": "power", "op": "value", "scale": 0.001 }));
        assert_eq!(declared(&follow), [(TableId::Countries.index(), vec![2])], "only the held table is followed");

        *given.lock().unwrap() = Some(frame(120));
        evaluate(&mut panels);
        assert_eq!(push.counters().offered, 1, "the same structure and cells: nothing pushed, the cells followed as they were");

        *given.lock().unwrap() = Some(frame(150));
        evaluate(&mut panels);
        let body = pushed(&push);
        let groups = body["panels"][0]["data"]["live"]["groups"].as_array().unwrap().clone();
        let countries: Vec<&serde_json::Value> = groups.iter().filter(|group| group["table"] == TableId::Countries.index()).collect();
        assert_eq!(countries.len(), 1, "100 and the rest are read in the same columns: {groups:?}");
        assert_eq!((&countries[0]["columns"], &countries[0]["keys"]), (&serde_json::json!([2, 3]), &serde_json::json!(["100", "101", "102"])));
        let second = countries[0]["follow"].as_u64().unwrap() as u32;
        assert!(second > first, "a new structure's cells under new ids: a late answer of the old ones is told apart");
        assert_eq!(declared(&follow), [(TableId::Countries.index(), vec![2, 3])], "the old cells let go, the new followed");

        panels.remove(7);
        assert!(declared(&follow).is_empty(), "a panel that goes takes its cells");
    }

    /// Its data names the hour it was made at, so every frame pushes it; its one cell never changes.
    struct Dated {
        live: Option<Live>,
    }

    impl Panel for Dated {
        fn tables(&self) -> &[TableId] {
            &[TableId::Countries]
        }

        fn data(&mut self, frame: &DecodedFrame) -> PanelData {
            let cells = Cells::default();
            cells.value("100", "power", Source { table: TableId::Countries, column: 2, key: 100 }, 0.001);
            self.live = Some(cells.finish());
            Ok(serde_json::json!({ "made": frame.hour }))
        }

        fn live(&self) -> Option<&Live> {
            self.live.as_ref()
        }
    }

    #[test]
    fn a_panel_pushed_again_keeps_the_follow_of_a_group_it_names_unchanged_so_its_values_never_blank() {
        let given = Arc::new(Mutex::new(None));
        let push = Arc::new(Push::default());
        let follow = Follow::new(1 << 16);
        let mut held = vec![false; TableId::ALL.len()];
        held[TableId::Countries.index()] = true;
        let mut panels = Panels { push: Some(Arc::clone(&push)), cells: Some(HostCells { follow: follow.clone(), held }), ..Default::default() };
        panels.shown.insert(7, Shown::new(Box::new(Dated { live: None }), Box::new(Given(Arc::clone(&given)))));
        let id = |body: &serde_json::Value| body["panels"][0]["data"]["live"]["groups"][0]["follow"].as_u64().unwrap();
        *given.lock().unwrap() = Some(frame(100));
        evaluate(&mut panels);
        let first = id(&pushed(&push));
        *given.lock().unwrap() = Some(frame(120));
        evaluate(&mut panels);
        let body = pushed(&push);
        assert_eq!(body["panels"][0]["data"]["made"], 120, "the new data is pushed");
        assert_eq!(id(&body), first, "the same group under the same id: the reader keeps its values");
        assert_eq!(declared(&follow), [(TableId::Countries.index(), vec![2])], "followed once, never let go and taken again");
    }
}
