use crate::{Answer, Follow, Identity, Message, Query, Rows, Sink, State, StepStats, answer_at, decode};
use dashboard_declare::{Carry, Column, Key, Measure, Parts, Series};
use dashboard_query::{
    identity::{Holder, HolderRun, Holders},
    timeline::{Join, Stretch, Timeline as Stretches},
};
use playhead::{
    Playhead, Settings, Timeline, Wanted,
    lane::Lane,
    pages::{Pages, Run},
};
use replay_columnar::{
    manifest::Status,
    state::{Cell, Kind},
    store::{Store, TableChanges},
    unit::{BLOCK, BlockPage, Encoder, PartSink, Settings as Writing, reader::Extents},
};
use replay_data::{Hour, rows::TableId};
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

/// Countries, its first five columns as the schema has them: 1 is a text (the tag), 2 an optional
/// number (political power).
const TABLE: usize = 2;
const KINDS: [Kind; 5] = [Kind::I32, Kind::Bytes, Kind::OptionalI64, Kind::I64, Kind::I64];
const HOURS: u32 = 2 * BLOCK;
/// Steps the harness's playhead builds past its head, at most.
const RING: usize = 3;

#[derive(Default)]
struct Memory {
    bytes: Vec<u8>,
    pages: Vec<BlockPage>,
}

impl PartSink for Memory {
    fn frame(&mut self, stored: &[u8]) -> io::Result<u64> {
        let offset = self.bytes.len() as u64;
        self.bytes.extend_from_slice(stored);
        Ok(offset)
    }

    fn block(&mut self, page: BlockPage) -> io::Result<()> {
        self.pages.push(page);
        Ok(())
    }

    fn read_back(&mut self, offset: u64, stored: u32, out: &mut Vec<u8>) -> io::Result<()> {
        out.clear();
        out.extend_from_slice(&self.bytes[offset as usize..offset as usize + stored as usize]);
        Ok(())
    }
}

struct Recording {
    bytes: Vec<u8>,
    runs: Vec<Run>,
    kinds: Vec<Vec<Kind>>,
}

impl Pages for Recording {
    fn extents(&self, _run: usize) -> &dyn Extents {
        &self.bytes
    }

    fn runs(&self) -> &[Run] {
        &self.runs
    }

    fn kinds(&self, table: usize) -> Option<&[Kind]> {
        self.kinds.get(table).map(Vec::as_slice)
    }
}

/// A run of [`HOURS`] hours from hour 0 in which `hour` makes Countries' changes; the other
/// tables hold nothing. Countries is not recorded at hours 300 to 329.
fn record(hour: impl FnMut(u32, &mut TableChanges<'_>)) -> Arc<Recording> {
    record_with(|_| Status::NotRecorded, hour)
}

/// As [`record`], Countries of the status `gap` gives at hours 300 to 329.
fn record_with(mut gap: impl FnMut(&mut Store) -> Status, mut hour: impl FnMut(u32, &mut TableChanges<'_>)) -> Arc<Recording> {
    let kinds = vec![KINDS.to_vec(); TABLE + 1];
    let mut store = Store::new(kinds.clone(), vec![false; kinds.len()]);
    store.attach(Encoder::new(&kinds, Writing { cap: 256, ..Writing::at(3) }).unwrap());
    let mut memory = Memory::default();
    store.begin_run(0);
    for at in 0..HOURS {
        store.begin_hour(at as Hour, &[]).unwrap();
        for table in 0..TABLE {
            store.status(table, Status::Complete, true, Some(at as Hour), true).unwrap();
        }
        let read = !unread(at);
        let status = if read { Status::Complete } else { gap(&mut store) };
        store.status(TABLE, status, read, read.then_some(at as Hour), true).unwrap();
        if read {
            hour(at, &mut store.table(TABLE).unwrap());
        }
        store.end_hour(&mut memory).unwrap();
        if at % BLOCK == BLOCK - 1 {
            store.end_block();
        }
    }
    Arc::new(Recording { bytes: memory.bytes, runs: vec![Run::new(0, memory.pages).unwrap()], kinds })
}

fn unread(at: u32) -> bool {
    (300..330).contains(&at)
}

/// Eight rows, keys 100 to 107 in slots 0 to 7. Column 0 of slots 0 to 4 moves every hour; column
/// 3 of slot 5 is the hour over 100 (it moves at hours 100, 200 and 400 recorded); column 2 of slot 6 is written every hour and ends every
/// step of whole days at the same value; column 2 of slots 0 and 1 moves every tenth hour; column 4
/// of slot 7 moves once a day.
fn quiet(at: u32, changes: &mut TableChanges<'_>) {
    if at == 0 {
        for slot in 0..8 {
            changes.insert_at(slot, 100 + u128::from(slot)).unwrap();
            changes.write(slot, 0, Cell::I32(0)).unwrap();
            changes.write(slot, 2, Cell::OptionalI64(Some(0))).unwrap();
            for column in [3, 4] {
                changes.write(slot, column, Cell::I64(0)).unwrap();
            }
        }
    }
    for slot in 0..5 {
        changes.write(slot, 0, Cell::I32((at * 10 + slot) as i32)).unwrap();
    }
    changes.offer(5, 3, Cell::I64(i64::from(at / 100))).unwrap();
    changes.write(6, 2, Cell::OptionalI64(Some(i64::from(at % 2)))).unwrap();
    for slot in 0..2 {
        changes.offer(slot, 2, Cell::OptionalI64(Some(i64::from(at / 10 + slot * 1000)))).unwrap();
    }
    changes.offer(7, 4, Cell::I64(i64::from(at / 24))).unwrap();
}

/// [`quiet`], and a row inserted every tenth hour recorded (key 1000 + the hour) that lives 25 hours.
fn churn(at: u32, changes: &mut TableChanges<'_>) {
    quiet(at, changes);
    if at >= 25
        && let Some(slot) = changes.slot(1000 + u128::from(at - 25))
    {
        changes.delete(slot).unwrap();
    }
    if born(at) {
        let slot = changes.insert(1000 + u128::from(at)).unwrap();
        changes.write(slot, 0, Cell::I32(at as i32)).unwrap();
        changes.write(slot, 3, Cell::I64(-i64::from(at))).unwrap();
    }
}

/// [`quiet`], and a row inserted every tenth hour recorded (key 1000 + the hour) that stays.
fn appends(at: u32, changes: &mut TableChanges<'_>) {
    quiet(at, changes);
    if born(at) {
        let slot = changes.insert(1000 + u128::from(at)).unwrap();
        changes.write(slot, 0, Cell::I32(at as i32)).unwrap();
        changes.write(slot, 3, Cell::I64(-i64::from(at))).unwrap();
    }
}

fn born(at: u32) -> bool {
    at.is_multiple_of(10) && at + 25 < HOURS && !unread(at)
}

/// A step and what it did. A part not read in time stalls Countries (the playhead counts it and
/// the table catches up in a later step), so tests judge by the table's own hours.
struct Played {
    stats: StepStats,
    messages: Vec<Message>,
    /// The hours of Countries the step applied: its head before (none at a reset) and after.
    from: Hour,
    head: Hour,
}

impl Played {
    /// The step applied an hour of Countries.
    fn moved(&self) -> bool {
        self.from <= self.head
    }

    /// Each answer of its messages, in order: id, hour and body.
    fn answers(&self) -> Vec<(u32, Hour, Answer)> {
        self.messages.iter().flat_map(|message| decode(&message.bytes).unwrap().answers.into_iter().map(|(id, hour, body)| (id, hour, Answer::parse(body).unwrap())).collect::<Vec<_>>()).collect()
    }
}

struct Harness {
    pages: Arc<Recording>,
    playhead: Option<Arc<Playhead>>,
    follow: Follow,
    consumer: playhead::Consumer,
    played: mpsc::Receiver<Played>,
    head: Hour,
    /// The last hour of the newest step received.
    seen: Hour,
    /// Hours a step.
    every: Hour,
    /// Countries' head at the newest step received.
    reached: Hour,
    /// A seek's reset has not come yet.
    awaiting: bool,
}

/// A step a day; the follow hook's sink is a channel.
fn harness(pages: &Arc<Recording>, budget: usize) -> Harness {
    stepping(pages, budget, 24)
}

/// Steps of `every` hours; the follow hook's sink is a channel.
fn stepping(pages: &Arc<Recording>, budget: usize, every: Hour) -> Harness {
    let follow = Follow::new(budget);
    let (send, played) = mpsc::channel();
    let heads = Arc::new(Mutex::new((0, 0)));
    let told = Arc::clone(&heads);
    let sink: Sink = Box::new(move |stats: &StepStats, messages: Vec<Message>| {
        let (from, head) = *told.lock().unwrap();
        let _ = send.send(Played { stats: *stats, messages, from, head });
    });
    let mut inner = follow.hook(sink);
    let mut before: Option<Hour> = None;
    let hook: playhead::Hook = Box::new(move |step, lanes| {
        let head = lanes.get(TABLE).and_then(Lane::head).unwrap_or(step.last());
        let from = if step.reset() { head } else { before.map_or(step.first(), |before| before + 1) };
        before = Some(head);
        *heads.lock().unwrap() = (from, head);
        inner(step, lanes);
    });
    // A seek reads every part its read-ahead covers before its reset: the whole run, so no step
    // waits on a read and every step's hours are the table's.
    let settings = Settings { step_threads: 1, fetch_threads: 1, ahead: 100, ring: RING, speed: f64::from(every), rate: 1.0 };
    let playhead = Arc::new(Playhead::new(Arc::clone(pages) as Arc<dyn Pages>, Timeline::whole(&**pages).unwrap(), settings, Some(hook)).unwrap());
    let reader = playhead.declare(Vec::new(), None).unwrap();
    let consumer = playhead.consumer(reader);
    let weak = Arc::downgrade(&playhead);
    follow.bind(move |tables| weak.upgrade().map_or(Ok(()), |playhead| playhead.redeclare(reader, tables)), playhead.log_reads()).unwrap();
    Harness { pages: Arc::clone(pages), playhead: Some(playhead), follow, consumer, played, head: 0, seen: Hour::MIN, every, reached: Hour::MIN, awaiting: false }
}

impl Harness {
    fn playhead(&self) -> &Playhead {
        self.playhead.as_ref().expect("playing")
    }

    fn seek(&mut self, hour: Hour) {
        self.playhead().seek(hour);
        self.head = hour;
        self.seen = Hour::MIN;
        self.reached = Hour::MIN;
        self.awaiting = true;
    }

    /// Every step until Countries reaches `to`, the head moving a step a frame (past `to` while the
    /// table catches up from a stall). After a seek, steps built before it (still on their way)
    /// are kept in the output but count for nothing until the seek's reset comes.
    fn play(&mut self, to: Hour) -> Vec<Played> {
        let mut out = Vec::new();
        loop {
            // The head moves once a step reached it, so the ring always has room for the next; steps
            // built ahead of the head may have reached `to` already.
            if !self.awaiting && self.seen >= self.head {
                if self.head < to || self.reached < to {
                    self.head = (self.head + self.every).min(if self.head < to { to } else { HOURS as Hour - 1 });
                }
                if self.seen >= self.head && self.reached >= to {
                    return out;
                }
            }
            self.playhead().advance(self.head);
            let played = self.played.recv_timeout(Duration::from_secs(120)).unwrap_or_else(|_| panic!("no step at head {} (the last seen ends at {}, Countries at {}, the newest built at {:?}, playing to {to}); the playhead says {:?}", self.head, self.seen, self.reached, self.playhead().newest(), self.playhead().error()));
            self.awaiting &= !played.stats.reset;
            if !self.awaiting {
                (self.seen, self.reached) = (played.stats.last, played.head);
            }
            out.push(played);
            self.consumer.take(self.head, &mut Vec::new());
        }
    }

    /// `query`'s answer from a lane of its own sought to `hour`.
    fn fresh(&self, query: &Query, hour: Hour) -> Answer {
        let mut lane = Lane::new(Arc::clone(&self.pages) as Arc<dyn Pages>, 0, TABLE, &(0..KINDS.len()).filter(|&column| column != 1).collect::<Vec<_>>(), false).unwrap();
        lane.seek(hour).unwrap();
        Answer::parse(&answer_at(query, &lane).unwrap()).unwrap()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        if let Some(playhead) = self.playhead.take()
            && let Ok(playhead) = Arc::try_unwrap(playhead)
        {
            playhead.finish();
        }
    }
}

fn keys(keys: &[u128], columns: &[usize]) -> Query {
    Query::Cells { table: TABLE, columns: columns.to_vec(), rows: Rows::Keys(keys.to_vec()) }
}

/// The step applied Countries' hour `hour`.
fn covers(played: &Played, hour: Hour) -> bool {
    played.from <= hour && hour <= played.head
}

/// The step applied an hour recorded at which slot 5's column 3 moves (each hundredth).
fn moves_slot_5(played: &Played) -> bool {
    (100..=played.head).step_by(100).any(|hour| !unread(hour as u32) && covers(played, hour))
}

#[test]
fn a_query_is_evaluated_only_when_a_step_touched_its_columns_and_keys() {
    let pages = record(quiet);
    let mut harness = harness(&pages, 64 << 10);
    // Slot 5's column 3 moves at hours 100 and 200; slot 0's column 3 never does, though its
    // column 0 moves every hour.
    harness.follow.follow(1, keys(&[105], &[3])).unwrap();
    harness.follow.follow(2, keys(&[100], &[3])).unwrap();
    harness.seek(0);
    let played = harness.play(280);
    let first = played.iter().position(|played| played.answers().len() == 2).expect("both answered once their lane held them");
    for played in played[first + 1..].iter().filter(|played| played.head < 300) {
        let moved = covers(played, 100) || covers(played, 200);
        assert_eq!(played.stats.touched, u32::from(moved), "steps {}..{}", played.stats.first, played.stats.last);
        assert_eq!(played.stats.evaluated, played.stats.touched);
        let ids: Vec<u32> = played.answers().iter().map(|(id, ..)| *id).collect();
        assert_eq!(ids, if moved { vec![1] } else { vec![] });
    }
    for (id, hour, answer) in played.iter().flat_map(Played::answers) {
        let query = if id == 1 { keys(&[105], &[3]) } else { keys(&[100], &[3]) };
        assert_eq!(answer, harness.fresh(&query, hour), "query {id} at hour {hour}");
    }
}

#[test]
fn an_answer_equal_to_the_last_is_not_sent() {
    let pages = record(quiet);
    let mut harness = harness(&pages, 64 << 10);
    // Written every hour, the same at every step's end.
    harness.follow.follow(7, keys(&[106], &[2])).unwrap();
    harness.seek(0);
    let played = harness.play(280);
    let first = played.iter().position(|played| !played.answers().is_empty()).expect("a first answer");
    let mut sent = played[first].head % 2;
    let mut unchanged = 0;
    for played in played[first + 1..].iter().filter(|played| played.moved() && played.head < 300) {
        assert_eq!(played.stats.evaluated, 1, "written at hours {}..{}", played.from, played.head);
        let differs = played.head % 2 != sent;
        assert_eq!(played.stats.changed, u32::from(differs), "hours {}..{}", played.from, played.head);
        assert_eq!(!played.messages.is_empty(), differs);
        unchanged += u32::from(!differs);
        sent = played.head % 2;
    }
    assert!(unchanged > 0, "an equal answer evaluated and not sent");
}

#[test]
fn a_reset_evaluates_every_query_once() {
    let pages = record(churn);
    let mut harness = harness(&pages, 64 << 10);
    let queries = [
        keys(&[100, 105], &[0, 3]),
        Query::Cells { table: TABLE, columns: vec![0], rows: Rows::All },
        Query::Cells { table: TABLE, columns: vec![3], rows: Rows::Top { column: 0, n: 3 } },
        keys(&[107], &[4]),
    ];
    for (id, query) in queries.iter().enumerate() {
        harness.follow.follow(id as u32, query.clone()).unwrap();
    }
    harness.seek(0);
    harness.play(200);
    harness.seek(900);
    let played = harness.play(948);
    let reset = played.iter().find(|played| played.stats.reset).expect("the seek's reset step");
    assert_eq!((reset.stats.touched, reset.stats.evaluated), (4, 4));
    let message = reset.messages.first().expect("a settled message");
    assert!(message.reset && decode(&message.bytes).unwrap().reset);
    for (id, hour, answer) in reset.answers() {
        assert_eq!(answer, harness.fresh(&queries[id as usize], hour), "query {id}");
    }
}

#[test]
fn a_table_not_recorded_answers_not_recorded() {
    let pages = record(quiet);
    let mut harness = harness(&pages, 64 << 10);
    harness.follow.follow(3, keys(&[100], &[0])).unwrap();
    harness.seek(0);
    let played = harness.play(400);
    let answers: Vec<(Hour, Answer)> = played.iter().flat_map(Played::answers).map(|(_, hour, answer)| (hour, answer)).collect();
    let gap: Vec<&(Hour, Answer)> = answers.iter().filter(|(hour, _)| (300..330).contains(hour)).collect();
    let stopped = played.iter().filter(|played| (300..330).contains(&played.head)).count();
    assert!(stopped > 0, "a step ends while it is not recorded");
    assert_eq!(gap.len(), 1, "one answer while it is not recorded, sent once");
    let reason = gap[0].1.not_recorded.as_deref().expect("not recorded");
    assert_eq!(reason, "Countries not recorded at this hour", "no verb to agree with a plural table");
    for (hour, answer) in &answers {
        if !(300..330).contains(hour) {
            assert_eq!(answer.not_recorded, None, "hour {hour}");
            assert_eq!(answer.values, vec![i64::from(*hour) * 10], "hour {hour}: the value, never a held one or zero");
        }
    }
}

/// Every message of `played`, in the order sent, gives each id's answers oldest first: order across
/// ids is the queue's, but no id's answer follows a newer one of its own.
#[test]
fn a_table_not_available_answers_its_recorded_reason_in_words_never_its_id() {
    const WHY: &str = "the game's navies could not be followed";
    let pages = record_with(|store| Status::Unavailable { reason: store.intern(WHY.as_bytes()).unwrap() }, quiet);
    let said = format!("Countries not available at this hour: {WHY}");
    let mut harness = harness(&pages, 64 << 10);
    let query = keys(&[100], &[0]);
    harness.follow.follow(3, query.clone()).unwrap();
    harness.seek(0);
    let mut played = harness.play(400);
    harness.seek(310);
    played.extend(harness.play(340));
    let reasons: Vec<String> = played.iter().flat_map(Played::answers).filter(|(_, hour, _)| (300..330).contains(hour)).filter_map(|(_, _, answer)| answer.not_recorded).collect();
    assert!(reasons.len() >= 2, "played into the stretch and sought inside it: {reasons:?}");
    for reason in &reasons {
        assert_eq!(reason, &said, "the recorded reason's text, never its id");
    }
    assert_eq!(harness.fresh(&query, 310).not_recorded.as_deref(), Some(said.as_str()), "a lane sought into the stretch says the same");
}

fn each_id_in_order(played: &[Played]) {
    let mut last: BTreeMap<u32, Hour> = BTreeMap::new();
    for (step, played) in played.iter().enumerate() {
        for (id, hour, _) in played.answers() {
            if let Some(&before) = last.get(&id) {
                assert!(hour >= before, "id {id}: an answer at hour {hour} after one at {before}, step {step} (hours {}..{})", played.stats.first, played.stats.last);
            }
            last.insert(id, hour);
        }
    }
}

#[test]
fn the_budget_splits_a_step_into_messages_within_it_and_holds_no_answer_back() {
    let pages = record(quiet);
    // Room for one answer of one row a message.
    let budget = 16 + 16 + 72;
    let mut harness = harness(&pages, budget);
    for slot in 0..3u128 {
        harness.follow.follow(slot as u32, keys(&[100 + slot], &[0])).unwrap();
    }
    harness.seek(0);
    let played = harness.play(280);
    let mut steps = 0;
    for played in played.iter().filter(|played| played.moved() && !played.stats.reset && played.head < 300) {
        let answers = played.answers();
        let mut ids: Vec<u32> = answers.iter().map(|(id, ..)| *id).collect();
        ids.sort_unstable();
        assert_eq!(ids, vec![0, 1, 2], "every answer of the step, in it, steps {}..{}", played.stats.first, played.stats.last);
        assert_eq!(played.messages.len(), 3, "one answer fits a message");
        assert!(played.messages.iter().all(|message| message.bytes.len() <= budget && !message.reset));
        for (id, hour, answer) in &answers {
            assert_eq!(*hour, played.head, "the newest answer of its id, made in this very step");
            assert_eq!(answer, &harness.fresh(&keys(&[100 + u128::from(*id)], &[0]), *hour));
        }
        steps += 1;
    }
    assert!(steps > 5, "{steps} steps judged");
    each_id_in_order(&played);
}

#[test]
fn a_paused_seek_whose_answers_pass_one_message_sends_them_all_in_its_own_step() {
    let pages = record(quiet);
    let budget = 16 + 16 + 72;
    let mut harness = harness(&pages, budget);
    for slot in 0..3u128 {
        harness.follow.follow(slot as u32, keys(&[100 + slot], &[0])).unwrap();
    }
    harness.seek(0);
    harness.play(48);
    harness.seek(600);
    let played = harness.play(600);
    let reset = played.iter().find(|played| played.stats.reset).expect("the seek's reset step");
    let answers = reset.answers();
    let mut ids: Vec<u32> = answers.iter().map(|(id, ..)| *id).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![0, 1, 2], "no answer waits for a step that may not come");
    for (id, hour, answer) in &answers {
        assert_eq!(answer, &harness.fresh(&keys(&[100 + u128::from(*id)], &[0]), *hour));
    }
    assert!(reset.messages.iter().all(|message| message.bytes.len() <= budget));
    assert!(reset.messages[0].reset && reset.messages[1..].iter().all(|message| !message.reset), "the first message settles, the rest add to it");
    assert!(answers.iter().all(|&(_, hour, _)| hour == reset.head), "each id's answer at the reset's hour");
    each_id_in_order(&played);
}

#[test]
fn seventy_thousand_texts_are_each_read_back_by_their_own_row() {
    let mut table = crate::answer::Table::default();
    table.clear(1, false);
    for row in 0..70_000u32 {
        table.row(u128::from(row));
        table.cell(0, State::Missing);
        if row % 2 == 0 { table.explain(&format!("reason {row}")) } else { table.name(&format!("name {row}")) }
    }
    let mut body = Vec::new();
    table.encode(&mut body);
    let answer = Answer::parse(&body).unwrap();
    assert_eq!(answer.texts.len(), 70_000);
    for row in 0..70_000usize {
        let (reason, name) = if row % 2 == 0 { (Some(format!("reason {row}")), None) } else { (None, Some(format!("name {row}"))) };
        assert_eq!((answer.reason(row), answer.name(row)), (reason.as_deref(), name.as_deref()), "row {row}");
    }
}

#[test]
fn unfollow_at_a_step_boundary_sends_nothing_more() {
    let pages = record(quiet);
    let mut harness = harness(&pages, 64 << 10);
    harness.follow.follow(1, keys(&[100], &[0])).unwrap();
    harness.follow.follow(2, keys(&[101], &[0])).unwrap();
    harness.seek(0);
    let before = harness.play(200);
    assert!(before.iter().flat_map(Played::answers).any(|(id, ..)| id == 1));
    harness.follow.unfollow(1).unwrap();
    let after = harness.play(400);
    let ids = |played: &Played| played.answers().iter().map(|(id, ..)| *id).collect::<Vec<u32>>();
    let steps = || after.iter().map(|played| (played.from, played.head, ids(played))).collect::<Vec<_>>();
    // `play` returns with every step to its head received, so the steps built before the unfollow
    // reached the hook are those the ring held past the head: at most `RING`.
    let named: Vec<usize> = after.iter().enumerate().filter(|&(_, played)| ids(played).contains(&1)).map(|(at, _)| at).collect();
    assert!(named.iter().all(|&at| at < RING), "id 1 answered after the {RING} steps built before the unfollow: steps {named:?} of (from, head, ids) {:?}", steps());
    // Id 2's answer moves with every hour recorded; "not recorded" is sent once, by the first step
    // ending in hours 300 to 329 (a head past the newest step makes a short step, so two can end there).
    let mut unrecorded = before.iter().rev().find(|played| played.moved()).is_some_and(|played| unread(played.head as u32));
    for played in after.iter().filter(|played| played.moved()) {
        let now = unread(played.head as u32);
        let changed = !(now && unrecorded);
        assert_eq!(ids(played).contains(&2), changed, "id 2 at the step of hours {}..{}: (from, head, ids) {:?}", played.from, played.head, steps());
        unrecorded = now;
    }
    assert_eq!(harness.follow.held().queries, 1);
}

#[test]
fn a_rank_reorders_only_when_its_column_was_written() {
    let pages = record(quiet);
    let mut harness = harness(&pages, 64 << 10);
    let rank = Query::Cells { table: TABLE, columns: vec![3], rows: Rows::Top { column: 3, n: 3 } };
    harness.follow.follow(9, rank.clone()).unwrap();
    harness.seek(0);
    let played = harness.play(280);
    let first = played.iter().position(|played| !played.answers().is_empty()).expect("a first answer");
    for played in played[first + 1..].iter().filter(|played| played.head < 300) {
        let written = covers(played, 100) || covers(played, 200);
        assert_eq!(played.stats.evaluated, u32::from(written), "steps {}..{}", played.stats.first, played.stats.last);
    }
    let answers: Vec<(Hour, Answer)> = played.iter().flat_map(Played::answers).map(|(_, hour, answer)| (hour, answer)).filter(|(hour, _)| *hour < 300).collect();
    let (hour, top) = answers.last().unwrap();
    assert_eq!(top, &harness.fresh(&rank, *hour));
    assert_eq!(top.keys, vec![105, 100, 101], "slot 5 first, then ties by key");
    assert_eq!(top.values, vec![2, 0, 0]);
}

#[test]
fn a_log_pushes_exactly_the_rows_inserted_since_its_last_push() {
    // A row inserted and deleted inside one step is in no change set (the playhead nets them), so
    // these rows stay.
    let pages = record(appends);
    let mut harness = harness(&pages, 64 << 10);
    harness.follow.follow(4, Query::Cells { table: TABLE, columns: vec![0, 3], rows: Rows::Appended }).unwrap();
    harness.seek(0);
    let played = harness.play(1000);
    let mut rows: Vec<(u128, Hour, i64, i64)> = Vec::new();
    for (id, hour, answer) in played.iter().flat_map(Played::answers) {
        assert_eq!(id, 4);
        let hours = answer.hours.clone().expect("rows appended");
        for (row, &born) in hours.iter().enumerate() {
            assert!(born <= hour);
            assert_eq!(answer.states[row * 2..row * 2 + 2], [State::Value, State::Value]);
            rows.push((answer.keys[row], born, answer.values[row * 2], answer.values[row * 2 + 1]));
        }
    }
    let first = rows.first().expect("rows").1;
    let last = played.last().expect("steps").head;
    let wanted: Vec<(u128, Hour, i64, i64)> = (first..=last).filter(|&at| born(at as u32)).map(|at| (1000 + at as u128, at, i64::from(at), -i64::from(at))).collect();
    assert_eq!(rows, wanted);
}

static POWER: Series = Series {
    kind: "test-power",
    table: TableId::Countries,
    key: Key::Country,
    parts: None,
    measures: &[Measure { name: "power", column: Column::Cell("political_power_raw"), decimals: 5, aggregates: 0, carry: Carry::None, rank: 0 }],
    drops: false,
};

#[test]
fn a_series_reads_its_tags_holder_at_the_head() {
    let pages = record(quiet);
    let mut harness = harness(&pages, 64 << 10);
    let run = |first, last, slot, id| HolderRun { first, last, holder: Holder::Found { slot, id }, starts_segment: true };
    let mut holders = Holders::default();
    holders.tags.insert("AAA".into(), vec![run(0, 149, 0, 100), run(150, HOURS as Hour - 1, 1, 101)]);
    holders.tags.insert("BBB".into(), vec![HolderRun { first: 0, last: HOURS as Hour - 1, holder: Holder::Unresolved, starts_segment: true }]);
    let timeline = Stretches { id: 1, first: 0, last: HOURS as Hour - 1, stretches: vec![Stretch { run: 0, first: 0, last: HOURS as Hour - 1, join: Join::Start }], unheld: Vec::new() };
    let series = Query::Series {
        series: &POWER,
        measure: &POWER.measures[0],
        tags: vec!["AAA".into(), "BBB".into(), "CCC".into()],
        top: None,
        least: false,
        parts: false,
        identity: Identity { holders: Arc::new(holders), timeline: Arc::new(timeline) },
    };
    harness.follow.follow(5, series.clone()).unwrap();
    harness.seek(0);
    let played = harness.play(250);
    let answers: Vec<(Hour, Answer)> = played.iter().flat_map(Played::answers).map(|(_, hour, answer)| (hour, answer)).collect();
    for (hour, answer) in &answers {
        assert_eq!(answer, &harness.fresh(&series, *hour), "hour {hour}");
        let holder = if *hour < 150 { (100, i64::from(*hour) / 10) } else { (101, i64::from(*hour) / 10 + 1000) };
        assert_eq!((answer.keys[0], answer.values[0], answer.states[0]), (holder.0, holder.1, State::Value), "hour {hour}");
        assert_eq!(answer.reason(1), Some("No link resolves the country holding BBB"));
        assert_eq!(answer.reason(2), Some("CCC is held by no country"));
        assert_eq!(answer.states[1..], [State::NoRow, State::NoRow]);
    }
    let moved = played.iter().find(|played| covers(played, 150)).expect("the step the holder changes in");
    assert_eq!(moved.answers().len(), 1, "a new holder is answered in the step it takes the tag");
    // Back before the holder changed: the tag is its first holder's again, watched as such, for a
    // plain series and a ranked one alike.
    let top = Query::series_top(&POWER, &POWER.measures[0], 1, series_identity(&series));
    harness.follow.follow(6, top.clone()).unwrap();
    harness.play(260);
    harness.seek(0);
    let played: Vec<Played> = harness.play(120).into_iter().skip_while(|played| !played.stats.reset).collect();
    let answers: Vec<(u32, Hour, Answer)> = played.iter().flat_map(Played::answers).collect();
    assert!(answers.iter().any(|(id, ..)| *id == 5) && answers.iter().any(|(id, ..)| *id == 6), "both answered after the seek");
    for (id, hour, answer) in &answers {
        assert_eq!(answer, &harness.fresh(if *id == 5 { &series } else { &top }, *hour), "query {id} at hour {hour} after the seek back");
        assert_eq!((answer.keys[0], answer.values[0]), (100, i64::from(*hour) / 10), "query {id} at hour {hour}: AAA's first holder");
    }
}

fn series_identity(query: &Query) -> Identity {
    let Query::Series { identity, .. } = query else { panic!("a series") };
    identity.clone()
}

#[test]
fn a_top_breaks_a_tie_at_its_last_place_by_tag_as_the_race_does() {
    let pages = record(quiet);
    let mut harness = harness(&pages, 64 << 10);
    let held = |slot, id| vec![HolderRun { first: 0, last: HOURS as Hour - 1, holder: Holder::Found { slot, id }, starts_segment: true }];
    // ZZZ's key (102) sorts before AAA's (103); their power is 0 alike, and one place is left after MMM's.
    let mut holders = Holders::default();
    holders.tags.insert("MMM".into(), held(1, 101));
    holders.tags.insert("ZZZ".into(), held(2, 102));
    holders.tags.insert("AAA".into(), held(3, 103));
    let timeline = Stretches { id: 1, first: 0, last: HOURS as Hour - 1, stretches: vec![Stretch { run: 0, first: 0, last: HOURS as Hour - 1, join: Join::Start }], unheld: Vec::new() };
    let identity = Identity { holders: Arc::new(holders), timeline: Arc::new(timeline) };
    let top = Query::series_top(&POWER, &POWER.measures[0], 2, identity.clone());
    let least = Query::series_top(&POWER, &POWER.measures[0], 2, identity).least();
    harness.follow.follow(7, top.clone()).unwrap();
    harness.follow.follow(8, least.clone()).unwrap();
    harness.seek(0);
    let played = harness.play(50);
    let last = |id: u32| played.iter().flat_map(Played::answers).filter(|(at, _, _)| *at == id).last().expect("an answer");
    let (_, hour, answer) = last(7);
    assert_eq!(answer, harness.fresh(&top, hour));
    let names: Vec<&str> = (0..answer.keys.len()).filter_map(|row| answer.name(row)).collect();
    assert_eq!(names, ["MMM", "AAA"], "value first, then tag in byte order, never key");
    let (_, hour, answer) = last(8);
    assert_eq!(answer, harness.fresh(&least, hour));
    let names: Vec<&str> = (0..answer.keys.len()).filter_map(|row| answer.name(row)).collect();
    assert_eq!(names, ["AAA", "ZZZ"], "the least first, a tie by tag in byte order");
}

static PARTS: Series = Series {
    kind: "test-parts",
    table: TableId::Countries,
    key: Key::Country,
    parts: Some(Parts { bits: 2, id: None }),
    measures: &[Measure { name: "slot", column: Column::Cell("canonical_slot"), decimals: 0, aggregates: 0, carry: Carry::None, rank: 0 }],
    drops: false,
};

/// Keys 100 to 103 are owner 25's parts, 104 to 107 owner 26's: a followed owner's parts are its
/// rows at the head in key order, or its top parts greatest first (ties by key), each named by its
/// tag, as a lane sought to the hour answers them.
#[test]
fn a_followed_owners_parts_are_its_rows_in_key_order_or_its_top_parts() {
    let pages = record(quiet);
    let mut harness = harness(&pages, 64 << 10);
    let held = |id| vec![HolderRun { first: 0, last: HOURS as Hour - 1, holder: Holder::Found { slot: 0, id }, starts_segment: true }];
    let mut holders = Holders::default();
    holders.tags.insert("AAA".into(), held(25));
    holders.tags.insert("BBB".into(), held(26));
    let timeline = Stretches { id: 1, first: 0, last: HOURS as Hour - 1, stretches: vec![Stretch { run: 0, first: 0, last: HOURS as Hour - 1, join: Join::Start }], unheld: Vec::new() };
    let identity = Identity { holders: Arc::new(holders), timeline: Arc::new(timeline) };
    let top = Query::Series { series: &PARTS, measure: &PARTS.measures[0], tags: vec!["AAA".into(), "BBB".into()], top: Some(2), least: false, parts: true, identity: identity.clone() };
    let all = Query::Series { series: &PARTS, measure: &PARTS.measures[0], tags: vec!["BBB".into()], top: None, least: false, parts: true, identity };
    harness.follow.follow(1, top.clone()).unwrap();
    harness.follow.follow(2, all.clone()).unwrap();
    harness.seek(0);
    let played = harness.play(400);
    let mut answered = 0;
    for (id, hour, answer) in played.iter().flat_map(Played::answers) {
        let query = if id == 1 { &top } else { &all };
        assert_eq!(answer, harness.fresh(query, hour), "query {id} at hour {hour}");
        if unread(hour as u32) {
            assert!(answer.not_recorded.is_some());
            continue;
        }
        let at = i64::from(hour) * 10;
        let wanted: Vec<(u128, i64, &str)> = match id {
            1 => vec![(103, at + 3, "AAA"), (102, at + 2, "AAA"), (104, at + 4, "BBB"), (105, 0, "BBB")],
            _ => vec![(104, at + 4, "BBB"), (105, 0, "BBB"), (106, 0, "BBB"), (107, 0, "BBB")],
        };
        let got: Vec<(u128, i64, &str)> = (0..answer.keys.len()).map(|row| (answer.keys[row], answer.values[row], answer.name(row).unwrap())).collect();
        assert_eq!(got, wanted, "query {id} at hour {hour}");
        answered += 1;
    }
    assert!(answered > 10, "{answered} answers compared");
    let one = Query::Series { series: &PARTS, measure: &PARTS.measures[0], tags: vec!["AAA".into()], top: None, least: false, parts: false, identity: nobody() };
    assert!(harness.follow.follow(3, one).is_err(), "a sub-keyed series is followed by its owners' parts");
}

#[test]
fn a_query_of_a_text_column_is_refused_by_name() {
    let pages = record(quiet);
    let mut harness = harness(&pages, 64 << 10);
    harness.follow.follow(6, keys(&[100], &[1])).unwrap();
    harness.seek(0);
    let played = harness.play(48);
    let answers: Vec<Answer> = played.iter().flat_map(Played::answers).map(|(.., answer)| answer).collect();
    assert_eq!(answers.len(), 1, "refused once");
    assert_eq!(answers[0].not_recorded.as_deref(), Some("Countries's tag is a text column, which a followed query does not read"));
}


/// A host with no playhead: its files hold the tables `held` marks; the anchors it was given.
struct Paper {
    follow: Follow,
    held: Vec<bool>,
    anchors: Mutex<Vec<crate::registry::Anchor>>,
}

impl crate::registry::Hosted for Paper {
    fn follow(&self) -> &Follow {
        &self.follow
    }

    fn lacks(&self, table: usize) -> Option<String> {
        crate::registry::lacking(table, self.held.get(table).copied().unwrap_or(false))
    }

    fn anchor(&self, anchor: crate::registry::Anchor) {
        self.anchors.lock().unwrap().push(anchor);
    }
}

/// Hosts by path, or by `path@clock` for a clock mapping other than 0.
struct Papers(BTreeMap<String, Arc<Paper>>);

impl crate::registry::Playheads for Papers {
    fn open(&self, path: &std::path::Path, _branch: u64, clock: i32) -> Result<Arc<dyn crate::registry::Hosted>, String> {
        let path = path.to_string_lossy();
        self.0.get(&if clock == 0 { path.into_owned() } else { format!("{path}@{clock}") }).map(|paper| Arc::clone(paper) as Arc<dyn crate::registry::Hosted>).ok_or_else(|| "No playhead for this recording".to_owned())
    }
}

fn paper() -> Arc<Paper> {
    Arc::new(Paper { follow: Follow::new(64 << 10), held: vec![true; TABLE + 1], anchors: Mutex::default() })
}

fn nobody() -> Identity {
    Identity { holders: Arc::new(Holders::default()), timeline: Arc::new(Stretches { id: 1, first: 0, last: 0, stretches: Vec::new(), unheld: Vec::new() }) }
}

fn follow_request(id: u32, kind: &str, measure: &str) -> serde_json::Value {
    serde_json::json!({ "id": id, "branchId": "1", "query": { "product": "series-state", "params": {
        "branchId": "1", "family": { "kind": kind, "measure": measure }, "reads": [{ "tag": "GER", "hour": 5 }, { "tag": "SOV", "hour": 5 }, { "tag": "GER", "hour": 9 }] } } })
}

#[test]
fn a_follow_for_a_table_the_playhead_lacks_answers_not_available_and_makes_no_reader() {
    let held = paper();
    let registry = crate::registry::Registry::new(Some(Arc::new(Papers(BTreeMap::from([("a".to_owned(), Arc::clone(&held))])))));
    let path = std::path::Path::new("a");
    let reply = registry.answer(path, "follow", &follow_request(1, "country-stability", "stability"), |_| Ok(nobody()), Box::new(|_| None)).unwrap();
    assert_eq!(reply["available"], false);
    assert_eq!(reply["reason"], "CountryStability is not in this playhead's files (a derived table, until format 12)");
    assert!(held.follow.declared().unwrap().is_empty(), "no reader made");
    let none = registry.answer(std::path::Path::new("b"), "follow", &follow_request(2, "country-measure", "politicalPower"), |_| Ok(nobody()), Box::new(|_| None)).unwrap();
    assert_eq!((none["available"].clone(), none["reason"].clone()), (serde_json::json!(false), serde_json::json!("No playhead for this recording")));
    let alone = crate::registry::Registry::new(None).answer(path, "follow", &follow_request(3, "country-measure", "politicalPower"), |_| Ok(nobody()), Box::new(|_| None)).unwrap();
    assert_eq!(alone["available"], false, "a process with no playheads follows nothing");
    let followed = registry.answer(path, "follow", &follow_request(4, "country-measure", "politicalPower"), |_| Ok(nobody()), Box::new(|_| None)).unwrap();
    assert_eq!(followed["available"], true);
    assert_eq!(held.follow.declared().unwrap(), vec![playhead::Declared { table: TABLE, columns: vec![2], keyed: false, passing: false }]);
    assert_eq!(registry.followed(), 1);
}

/// Two tracks show one recording's timeline, one aligned at the calendar and one at an anchor a
/// thousand hours on: each follows on a playhead of its own mapping, and each clock anchor moves
/// only its own, to the game hour that track shows.
#[test]
fn two_tracks_of_one_timeline_at_different_anchors_each_follow_their_own_hours() {
    let (calendar, anchored) = (paper(), paper());
    let registry = crate::registry::Registry::new(Some(Arc::new(Papers(BTreeMap::from([("a".to_owned(), Arc::clone(&calendar)), ("a@1000".to_owned(), Arc::clone(&anchored))])))));
    let path = std::path::Path::new("a");
    let mut asked = follow_request(1, "country-measure", "politicalPower");
    registry.answer(path, "follow", &asked, |_| Ok(nobody()), Box::new(|_| None)).unwrap();
    asked["id"] = 2.into();
    asked["clock"] = 1000.into();
    registry.answer(path, "follow", &asked, |_| Ok(nobody()), Box::new(|_| None)).unwrap();
    assert_eq!((calendar.follow.declared().unwrap().len(), anchored.follow.declared().unwrap().len()), (1, 1), "each mapping's playhead reads its own follow");
    let anchor = |clock: i32, hour: i32| serde_json::json!({ "branchId": "1", "clock": clock, "hour": hour, "playing": false, "speed": 24.0, "sought": true });
    registry.answer(path, "follow-clock", &anchor(0, 10), |_| Ok(nobody()), Box::new(|_| None)).unwrap();
    registry.answer(path, "follow-clock", &anchor(1000, 1010), |_| Ok(nobody()), Box::new(|_| None)).unwrap();
    let hours = |paper: &Paper| paper.anchors.lock().unwrap().iter().map(|anchor| anchor.hour).collect::<Vec<_>>();
    assert_eq!((hours(&calendar), hours(&anchored)), (vec![10], vec![1010]), "each at the hour its track shows");
}

#[test]
fn the_requests_a_widget_sends_parse_into_its_query() {
    match crate::parse(&follow_request(0, "country-measure", "commandPower")["query"], &nobody()).unwrap() {
        Query::Series { series, measure, tags, top, .. } => assert_eq!((series.kind, measure.name, tags, top), ("country-measure", "commandPower", vec!["GER".to_owned(), "SOV".to_owned()], None)),
        _ => panic!("a series"),
    }
    let army = serde_json::json!({ "product": "army-size", "params": { "branchId": "1", "tags": [], "top": 10, "measure": "divisions", "countFor": "owner", "byType": false } });
    match crate::parse(&army, &nobody()).unwrap() {
        Query::Series { series, measure, top, .. } => assert_eq!((series.kind, measure.name, top), ("army-totals", "ownerDivisions", Some(10))),
        _ => panic!("a series"),
    }
    let typed = serde_json::json!({ "product": "army-size", "params": { "branchId": "1", "tags": ["GER"], "measure": "battalions", "countFor": "owner", "byType": true } });
    assert_eq!(crate::parse(&typed, &nobody()).err().unwrap().to_string(), "Army sizes by unit type are not followed at the head");
    let air = serde_json::json!({ "product": "air-force-size", "params": { "branchId": "1", "tags": [], "top": 10, "measure": "wings", "ownership": "effective" } });
    match crate::parse(&air, &nobody()).unwrap() {
        Query::Series { series, measure, top, .. } => assert_eq!((series.kind, measure.name, top), ("air-force-totals", "effectiveWings", Some(10))),
        _ => panic!("a series"),
    }
    let navy = serde_json::json!({ "product": "navy-size", "params": { "branchId": "1", "tags": ["GER"], "top": null, "measure": "strength", "ownership": "logical", "grouping": "type" } });
    match crate::parse(&navy, &nobody()).unwrap() {
        Query::Series { series, measure, tags, top, .. } => assert_eq!((series.kind, measure.name, tags, top), ("navy-totals", "logicalStrength", vec!["GER".to_owned()], None)),
        _ => panic!("a series"),
    }
    let unknown = serde_json::json!({ "product": "navy-size", "params": { "branchId": "1", "tags": ["GER"], "measure": "planes", "ownership": "roster" } });
    assert_eq!(crate::parse(&unknown, &nobody()).err().unwrap().to_string(), "No navy-size measure planes counted for roster");
    let other = serde_json::json!({ "product": "series-summary", "params": {} });
    assert_eq!(crate::parse(&other, &nobody()).err().unwrap().to_string(), "series-summary is not followed at the head");
    let parts = serde_json::json!({ "product": "series-state", "params": { "branchId": "1", "family": { "kind": "resource-balance", "measure": "supply" }, "reads": [{ "tag": "GER", "hour": 5 }], "top": 3 } });
    match crate::parse(&parts, &nobody()).unwrap() {
        Query::Series { series, measure, tags, top, parts, .. } => assert_eq!((series.kind, measure.name, tags, top, parts), ("resource-balance", "supply", vec!["GER".to_owned()], Some(3), true)),
        _ => panic!("a series"),
    }
    let one = serde_json::json!({ "product": "series-state", "params": { "branchId": "1", "family": { "kind": "country-resources", "measure": "used" }, "reads": [{ "tag": "GER", "hour": 5, "part": "2" }] } });
    assert!(crate::parse(&one, &nobody()).is_err(), "one part is not followed");
    let territory = serde_json::json!({ "product": "series-state", "params": { "branchId": "1", "family": { "kind": "territory", "measure": "controlledStates" }, "top": 10, "hour": 5 } });
    match crate::parse(&territory, &nobody()).unwrap() {
        Query::Series { series, measure, top, parts, .. } => assert_eq!((series.kind, measure.name, top, parts), ("territory", "controlledStates", Some(10), false)),
        _ => panic!("a ranked series"),
    }
    let unranked = serde_json::json!({ "product": "series-state", "params": { "branchId": "1", "family": { "kind": "territory", "measure": "ownedArea" }, "hour": 5 } });
    assert!(crate::parse(&unranked, &nobody()).is_err(), "a series asked for no tags ranks a top");
    let ranked = serde_json::json!({ "product": "series-state", "params": { "branchId": "1", "family": { "kind": "country-game-scores", "measure": "total" }, "top": 10 } });
    match crate::parse(&ranked, &nobody()).unwrap() {
        Query::Series { series, measure, top, parts, .. } => assert_eq!((series.kind, measure.name, top, parts), ("country-game-scores", "total", Some(10), false)),
        _ => panic!("a ranked series"),
    }
    let chosen_top = serde_json::json!({ "product": "series-state", "params": { "branchId": "1", "family": { "kind": "country-game-scores", "measure": "total" }, "reads": [{ "tag": "GER", "hour": 0 }], "top": 1 } });
    assert!(crate::parse(&chosen_top, &nobody()).is_err(), "a top of countries names no tag");
    let world = serde_json::json!({ "product": "series-state", "params": { "branchId": "1", "family": { "kind": "world-tension", "measure": "tension" }, "top": 3 } });
    assert!(crate::parse(&world, &nobody()).is_err(), "a singleton is never ranked");
    let tension = serde_json::json!({ "product": "series-state", "params": { "branchId": "1", "family": { "kind": "world-tension", "measure": "tension" }, "reads": [{ "tag": "World", "hour": 0 }] } });
    assert!(matches!(crate::parse(&tension, &nobody()).unwrap(), Query::Series { top: None, parts: false, .. }));
    // A faction of 1,500 members is followed whole: no tag count we chose refuses it.
    let members: Vec<serde_json::Value> = (0..1_500).map(|at| serde_json::json!({ "tag": format!("T{at:04}"), "hour": 0 })).collect();
    let many = serde_json::json!({ "product": "series-state", "params": { "branchId": "1", "family": { "kind": "territory", "measure": "controlledStates" }, "reads": members } });
    match crate::parse(&many, &nobody()).unwrap() {
        Query::Series { tags, top: None, .. } => assert_eq!((tags.len(), tags[1_499].as_str()), (1_500, "T1499")),
        _ => panic!("a series of every member"),
    }
    let wide = serde_json::json!({ "product": "series-state", "params": { "branchId": "1", "family": { "kind": "territory", "measure": "controlledStates" }, "top": 1_000 } });
    assert!(matches!(crate::parse(&wide, &nobody()).unwrap(), Query::Series { top: Some(1_000), .. }));
    let log =serde_json::json!({ "product": "log-page", "params": { "branchId": "1", "log": "market-events", "to": 9, "pageSize": 50, "columns": ["kind", "country"] } });
    let columns = vec![dashboard_declare::field(TableId::MarketEvents, "kind").unwrap(), dashboard_declare::field(TableId::MarketEvents, "country").unwrap()];
    match crate::parse(&log, &nobody()).unwrap() {
        Query::Cells { table, columns: asked, rows } => assert_eq!((table, asked, rows), (TableId::MarketEvents.index(), columns, Rows::Appended)),
        _ => panic!("a log's appended rows"),
    }
}

#[test]
fn closing_a_recording_drops_its_follows() {
    let (a, b) = (paper(), paper());
    let registry = crate::registry::Registry::new(Some(Arc::new(Papers(BTreeMap::from([("a".to_owned(), Arc::clone(&a)), ("b".to_owned(), Arc::clone(&b))])))));
    for (id, path) in [(1, "a"), (2, "a"), (3, "b")] {
        assert_eq!(registry.answer(std::path::Path::new(path), "follow", &follow_request(id, "country-measure", "politicalPower"), |_| Ok(nobody()), Box::new(|_| None)).unwrap()["available"], true);
    }
    registry.close(std::path::Path::new("a"));
    assert_eq!(registry.followed(), 1);
    assert!(a.follow.declared().unwrap().is_empty(), "the closed recording's follows went");
    assert_eq!(b.follow.declared().unwrap().len(), 1, "the other recording's stayed");
}

/// A message of one answer: id `id`'s one value at `hour`.
fn one(id: u32, hour: Hour, value: i64) -> Vec<u8> {
    let mut message = Vec::new();
    crate::answer::begin(&mut message, 0, hour, false);
    crate::answer::push(&mut message, id, hour, &body(id, value));
    message
}

fn body(id: u32, value: i64) -> Vec<u8> {
    let mut out = Vec::new();
    let mut table = crate::answer::Table::default();
    table.clear(1, false);
    table.row(u128::from(id));
    table.cell(value, State::Value);
    table.encode(&mut out);
    out
}

#[test]
fn a_frame_round_trips_the_pipe_and_a_slow_reader_makes_the_queue_supersede_never_grow() {
    use crate::push::{FOLLOW, Push, QUEUED, read_frame, write_frame};
    let (mut reader, mut writer) = std::io::pipe().unwrap();
    let sent = one(7, 24, 123);
    write_frame(&mut writer, FOLLOW, 41, &sent).unwrap();
    let mut payload = Vec::new();
    assert_eq!(read_frame(&mut reader, &mut payload).unwrap(), Some((FOLLOW, 41)));
    assert_eq!(payload, sent, "byte for byte");

    // Nobody reads: forty steps of ids 2 and 3, and id 1 in every tenth.
    let push = Arc::new(Push::default());
    for step in 0..40u32 {
        let hour = step as Hour * 24;
        let mut message = Vec::new();
        crate::answer::begin(&mut message, 0, hour, false);
        for id in [2u32, 3] {
            crate::answer::push(&mut message, id, hour, &body(id, i64::from(step)));
        }
        if step % 10 == 0 {
            crate::answer::push(&mut message, 1, hour, &body(1, i64::from(step)));
        }
        push.offer(message);
        assert!(push.counters().most_queued <= QUEUED as u64, "never over its bound");
    }
    let counters = push.counters();
    assert_eq!(counters.offered, 40);
    assert!(counters.superseded > 0, "older answers were superseded");
    assert_eq!(counters.unmerged, 0);
    push.stop();
    let mut drained = Vec::new();
    push.drain(&mut drained).unwrap();
    std::io::Write::write_all(&mut writer, &drained).unwrap();
    drop(writer);
    let mut newest: BTreeMap<u32, (Hour, i64)> = BTreeMap::new();
    let mut frames = 0;
    while let Some((kind, _)) = read_frame(&mut reader, &mut payload).unwrap() {
        assert_eq!(kind, FOLLOW);
        frames += 1;
        for (id, hour, body) in decode(&payload).unwrap().answers {
            newest.insert(id, (hour, Answer::parse(body).unwrap().values[0]));
        }
    }
    assert!(frames <= QUEUED);
    assert_eq!(newest, BTreeMap::from([(1, (30 * 24, 30)), (2, (39 * 24, 39)), (3, (39 * 24, 39))]), "the newest answer of every id reached the reader");
}

#[test]
fn a_message_past_a_frame_part_reaches_the_reader_whole() {
    use crate::push::{FOLLOW, Push, read_frame};
    let push = Push::default();
    let large: Vec<u8> = (0..(9u32 << 20) + 5).map(|at| (at % 251) as u8).collect();
    push.offer(large.clone());
    push.offer(one(7, 24, 123));
    push.stop();
    let mut pipe = Vec::new();
    push.drain(&mut pipe).expect("a message of any size is written");
    let mut input = pipe.as_slice();
    let mut payload = Vec::new();
    assert_eq!(read_frame(&mut input, &mut payload).unwrap(), Some((FOLLOW, 0)));
    assert!(payload == large, "the large message, joined from its parts: {} bytes of {}", payload.len(), large.len());
    assert_eq!(read_frame(&mut input, &mut payload).unwrap(), Some((FOLLOW, 1)));
    assert_eq!(payload, one(7, 24, 123), "and the message after it");
    assert_eq!(read_frame(&mut input, &mut payload).unwrap(), None);
}

#[test]
fn a_failed_write_stops_the_pushed_answers_by_name() {
    use crate::push::{FAILURE, Push, failure};
    struct Gone;
    impl std::io::Write for Gone {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "the reader is gone"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let push = Push::default();
    push.offer(one(7, 24, 1));
    push.offer(one(7, 48, 2));
    assert!(push.drain(&mut Gone).is_err());
    let failed = push.failed().expect("the failure, by name");
    assert!(failed.contains("the reader is gone"), "{failed}");
    assert_eq!((push.counters().written, push.counters().unwritten), (0, 2));
    push.offer(one(7, 72, 3));
    assert_eq!(push.counters().offered, 2, "nothing is queued once stopped");
    let notice = failure(&failed);
    assert_eq!(u32::from_le_bytes(notice[..4].try_into().unwrap()), FAILURE);
    assert_eq!(std::str::from_utf8(&notice[4..]).unwrap(), failed);
}

/// Two recordings' follows share the pipe: a slow reader makes each one's messages merge with its
/// own alone. Each message reaches the reader with its route, every answer in it of that route,
/// at its own playhead's hour, and a reset of one never with the other's answers.
#[test]
fn the_queue_merges_a_follows_messages_with_its_own_and_never_with_another_follows() {
    use crate::push::{Push, QUEUED, read_frame};
    let push = Push::default();
    for step in 0..20u32 {
        for (route, id, first) in [(3u32, 1u32, 0), (8, 2, 5000)] {
            let hour = first + step as Hour;
            let mut message = Vec::new();
            crate::answer::begin(&mut message, route, hour, route == 8 && step == 10);
            crate::answer::push(&mut message, id, hour, &body(id, i64::from(step)));
            push.offer(message);
        }
        assert!(push.counters().most_queued <= QUEUED as u64);
    }
    assert!(push.counters().merges > 0, "a slow reader merges");
    push.stop();
    let mut pipe = Vec::new();
    push.drain(&mut pipe).unwrap();
    let (mut input, mut payload, mut last) = (pipe.as_slice(), Vec::new(), std::collections::BTreeMap::new());
    while read_frame(&mut input, &mut payload).unwrap().is_some() {
        let message = decode(&payload).unwrap();
        assert_eq!(crate::answer::route(&payload), Some(message.route));
        let (id, first) = if message.route == 3 { (1, 0) } else { (2, 5000) };
        assert!(message.answers.iter().all(|&(answer, at, _)| answer == id && at == message.hour), "route {} holds only its own answers at its hour", message.route);
        assert!((first..first + 20).contains(&message.hour));
        assert!(!message.reset || message.route == 8, "only the follow that sought is a reset");
        last.insert(message.route, message.hour);
    }
    assert_eq!(last, [(3, 19), (8, 5019)].into(), "each follow's newest message reaches the reader");
}

#[test]
fn a_merge_that_fails_sends_both_messages_as_they_are_in_order() {
    use crate::push::{Push, read_frame};
    let push = Push::default();
    push.offer(rows_message(10, false, &[(1, 10)]));
    // Rows changed naming a key the rows before do not hold: the two do not merge.
    push.offer(rows_message(11, true, &[(9, 1)]));
    for step in 0..3 {
        push.offer(one(7, 12 + step, i64::from(step)));
    }
    assert_eq!(push.counters().unmerged, 1);
    push.stop();
    let mut pipe = Vec::new();
    push.drain(&mut pipe).unwrap();
    let (mut input, mut payload, mut fours) = (pipe.as_slice(), Vec::new(), Vec::new());
    while read_frame(&mut input, &mut payload).unwrap().is_some() {
        fours.extend(decode(&payload).unwrap().answers.into_iter().filter(|&(id, ..)| id == 4).map(|(.., body)| Answer::parse(body).unwrap()));
    }
    assert_eq!(fours.iter().map(|answer| (answer.changed, answer.keys.clone())).collect::<Vec<_>>(), vec![(false, vec![1]), (true, vec![9])], "both reached the reader, in order");
}

#[test]
fn a_seek_sends_one_reset_message_and_follows_survive_it() {
    let pages = record(quiet);
    let mut harness = harness(&pages, 64 << 10);
    harness.follow.follow(1, keys(&[100], &[0])).unwrap();
    harness.seek(0);
    harness.play(200);
    harness.seek(600);
    let played = harness.play(700);
    let resets: Vec<&Played> = played.iter().filter(|played| played.stats.reset).collect();
    assert_eq!(resets.len(), 1, "one reset message");
    assert!(resets[0].messages.first().is_some_and(|message| message.reset));
    let after: Vec<(Hour, Answer)> = played.iter().filter(|played| played.head >= 600).flat_map(Played::answers).map(|(_, hour, answer)| (hour, answer)).collect();
    assert!(after.len() > 3, "still followed after the seek");
    for (hour, answer) in after {
        assert_eq!(answer.values, vec![i64::from(hour) * 10], "hour {hour}");
    }
    assert_eq!(harness.follow.held().queries, 1);
}

/// Messages wait for the head to reach their step, so a seek may abandon some never pushed: its
/// reset sends every answer again, whole, though equal to the last sent.
#[test]
fn a_reset_sends_every_answer_again_whole_though_equal_to_the_last_sent() {
    let pages = record(quiet);
    let mut harness = harness(&pages, 64 << 10);
    // Key 105's column 3 is the hour over 100: zero through hour 99.
    harness.follow.follow(1, keys(&[105], &[3])).unwrap();
    harness.follow.follow(2, Query::Cells { table: TABLE, columns: vec![3], rows: Rows::Live(vec![105]) }).unwrap();
    harness.seek(0);
    harness.play(48);
    harness.seek(24);
    let played = harness.play(24);
    let reset = played.iter().find(|played| played.stats.reset).expect("the seek's reset step");
    let answers = reset.answers();
    let mut ids: Vec<u32> = answers.iter().map(|(id, ..)| *id).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![1, 2], "each answer again in the reset's step");
    for (id, hour, answer) in &answers {
        assert!(!answer.changed, "query {id}: whole, never rows changed over rows the reader may not hold");
        assert_eq!(held_rows(answer), held_rows(&harness.fresh(&keys(&[105], &[3]), *hour)), "query {id} at hour {hour}");
    }
}

/// One message of each kind of answer, as the UI's parser reads it (`ui/src/dashboard/follow/store.test.ts`).
fn golden() -> Vec<u8> {
    use crate::answer::{Table, begin, not_recorded, push};
    let mut message = Vec::new();
    begin(&mut message, 0, 1234, true);
    let mut table = Table::default();
    table.clear(1, false);
    table.row(100);
    table.name("GER");
    table.cell(12_345_678, State::Value);
    table.row(101);
    table.name("SOV");
    table.cell(0, State::Missing);
    table.explain("SOV has no value");
    table.row(0);
    table.name("ITA");
    table.absent("ITA is held by no country");
    let mut body = Vec::new();
    table.encode(&mut body);
    push(&mut message, 7, 1200, &body);
    body.clear();
    not_recorded("Countries was not recorded at this hour", &mut body);
    push(&mut message, 8, 1234, &body);
    table.clear(2, true);
    table.row((1u128 << 70) + 5);
    table.hours.push(1100);
    table.cell(-2, State::Value);
    table.cell(9, State::Value);
    body.clear();
    table.encode(&mut body);
    push(&mut message, 9, 1234, &body);
    message
}

#[test]
fn the_message_bytes_are_the_ones_the_ui_reads() {
    let hex: String = golden().iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(hex, GOLDEN);
}

const GOLDEN: &str = "464f4c3101000000d20400000300000007000000b0040000c800000000000000010000000300000001000000050000006400000000000000650000000000000000000000000000000000000000000000000000000000000000000000000000004e61bc0000000000000000000000000000000000000000000001030000000000ffffffff020000000400000000000000000000000100000003000000000000000300000047455203000000534f5610000000534f5620686173206e6f2076616c756503000000495441190000004954412069732068656c64206279206e6f20636f756e747279000008000000d204000040000000000000000000000000000000000000000100000027000000436f756e747269657320776173206e6f74207265636f72646564206174207468697320686f7572000000000009000000d204000050000000000000000200000001000000020000000000000005000000000000004000000000000000feffffffffffffff09000000000000004c040000000000000000000000000000ffffffff00000000ffffffff00000000";

#[test]
fn a_table_never_recorded_in_the_files_is_not_held() {
    let kinds = vec![KINDS.to_vec(); TABLE + 1];
    let mut store = Store::new(kinds.clone(), vec![false; kinds.len()]);
    store.attach(Encoder::new(&kinds, Writing { cap: 256, ..Writing::at(3) }).unwrap());
    let mut memory = Memory::default();
    store.begin_run(0);
    for at in 0..BLOCK + 10 {
        store.begin_hour(at as Hour, &[]).unwrap();
        for table in 0..TABLE {
            store.status(table, Status::Complete, true, Some(at as Hour), true).unwrap();
        }
        store.status(TABLE, Status::NotRecorded, false, None, true).unwrap();
        store.end_hour(&mut memory).unwrap();
        if at % BLOCK == BLOCK - 1 {
            store.end_block();
        }
    }
    store.end_run(&mut memory).unwrap();
    let pages = Recording { bytes: memory.bytes, runs: vec![Run::new(0, memory.pages).unwrap()], kinds };
    assert_eq!(crate::host::held_tables(&pages, TABLE + 1), vec![true, true, false], "a table whose every hour is not recorded is not the playhead's to follow");
    let recorded = record(quiet);
    assert!(crate::host::held_tables(&*recorded, TABLE + 1)[TABLE], "one not recorded for some hours is");
}

/// The quiet rows, and every tenth hour recorded a row (key 1000 + the hour) that lives one hour.
fn brief(at: u32, changes: &mut TableChanges<'_>) {
    quiet(at, changes);
    if at >= 1
        && born(at - 1)
        && let Some(slot) = changes.slot(1000 + u128::from(at - 1))
    {
        changes.delete(slot).unwrap();
    }
    if born(at) {
        let slot = changes.insert(1000 + u128::from(at)).unwrap();
        changes.write(slot, 0, Cell::I32(at as i32)).unwrap();
        changes.write(slot, 3, Cell::I64(-i64::from(at))).unwrap();
    }
}

/// A row that lives one hour is born and gone inside a step of a day: the log pushes it, once,
/// with its hour and its cells, in hour order.
#[test]
fn a_log_pushes_a_row_born_and_gone_inside_a_step_once() {
    let pages = record(brief);
    let mut harness = harness(&pages, 64 << 10);
    harness.follow.follow(4, Query::Cells { table: TABLE, columns: vec![0, 3], rows: Rows::Appended }).unwrap();
    harness.seek(0);
    let played = harness.play(1000);
    let mut rows: Vec<(u128, Hour, i64, i64)> = Vec::new();
    for (_, hour, answer) in played.iter().flat_map(Played::answers) {
        let hours = answer.hours.clone().expect("rows appended");
        for (row, &born) in hours.iter().enumerate() {
            assert!(born <= hour);
            rows.push((answer.keys[row], born, answer.values[row * 2], answer.values[row * 2 + 1]));
        }
    }
    let last = played.last().expect("steps").head;
    let wanted: Vec<(u128, Hour, i64, i64)> = (1..=last).filter(|&at| born(at as u32)).map(|at| (1000 + at as u128, at, i64::from(at), -i64::from(at))).collect();
    assert!(wanted.len() > 50);
    assert_eq!(rows, wanted);
}

#[test]
fn panel_messages_merge_by_id_and_the_queue_merges_each_kind_with_its_own() {
    use crate::panel::{PanelData, merge, message};
    use crate::push::{FOLLOW, PANEL, Push};
    let data = |line: &str| -> PanelData { Ok(serde_json::json!({ "line": line })) };
    let (a, b, c) = (data("a"), data("b"), data("c"));
    let (merged, superseded) = merge(&message(&[(1, 10, &a), (2, 10, &b)]), &message(&[(1, 11, &c)])).unwrap();
    assert_eq!(superseded, 1);
    let body: serde_json::Value = serde_json::from_slice(&merged[4..]).unwrap();
    assert_eq!(body["panels"].as_array().unwrap().len(), 2);
    assert_eq!((body["panels"][0]["id"].as_u64(), body["panels"][0]["data"]["line"].as_str()), (Some(1), Some("c")), "the newer data of a panel wins");
    let push = Push::default();
    push.offer_kind(PANEL, message(&[(1, 1, &a)]));
    for at in 0..8 {
        push.offer(one(9, at, i64::from(at)));
    }
    push.offer_kind(PANEL, message(&[(1, 9, &c)]));
    assert!(push.counters().most_queued <= crate::push::QUEUED as u64);
    assert_eq!(push.counters().unmerged, 0, "every merge met its own kind");
    let mut kinds = Vec::new();
    while push.counters().written < push.counters().offered - push.counters().merges {
        let (kind, bytes, ..) = push.take_kind().unwrap();
        if kind == PANEL {
            let body: serde_json::Value = serde_json::from_slice(&bytes[4..]).unwrap();
            assert_eq!(body["panels"][0]["data"]["line"], "c", "the panel's newest data reaches the reader");
        }
        kinds.push(kind);
    }
    assert!(kinds.contains(&PANEL) && kinds.contains(&FOLLOW));
}

/// A log's columns in Countries' place: numbers, an optional number among them.
const LOG: [Kind; 4] = [Kind::I64, Kind::OptionalU32, Kind::I32, Kind::U16];

/// The log's hour `at` in the game's order: every 7th hour its oldest listed row forgotten first;
/// `at % 4` rows appended (an optional cell missing or not given in some), one more appended and
/// forgotten again every 13th hour; every 50th hour every other row of the oldest ten forgotten;
/// then the oldest while more than 30 are listed, consecutive ones as one run.
fn log_hour(at: u32, changes: &mut TableChanges<'_>) {
    let listed = |changes: &TableChanges<'_>| changes.held().listed().unwrap().ones().collect::<Vec<u64>>();
    if at % 7 == 0
        && let Some(&oldest) = listed(changes).first()
    {
        changes.forget(oldest).unwrap();
    }
    let count = at % 4 + u32::from(at % 13 == 0);
    for index in 0..count {
        let seed = u64::from(at) * 7 + u64::from(index);
        let cells = [
            Some(Cell::I64(seed as i64 * -1_000_003)),
            (seed % 4 != 0).then(|| Cell::OptionalU32((seed % 3 != 0).then_some(seed as u32 * 11))),
            Some(Cell::I32(at as i32)),
            Some(Cell::U16(index as u16)),
        ];
        let ordinal = changes.append(u128::from(at) << 64 | u128::from(index), &cells).unwrap();
        if at % 13 == 0 && index + 1 == count {
            changes.forget(ordinal).unwrap();
        }
    }
    if at % 50 == 0 {
        for &ordinal in listed(changes).iter().take(10).step_by(2) {
            changes.forget(ordinal).unwrap();
        }
    }
    let held = listed(changes);
    let excess = &held[..held.len().saturating_sub(30)];
    let mut start = 0;
    while start < excess.len() {
        let run = excess[start..].iter().enumerate().take_while(|&(offset, &ordinal)| ordinal == excess[start] + offset as u64).count();
        changes.forget_run(excess[start], run as u64).unwrap();
        start += run;
    }
}

/// A run of [`HOURS`] hours in which Countries is a log of [`LOG`] by [`log_hour`].
fn record_log() -> Arc<Recording> {
    let kinds = vec![LOG.to_vec(); TABLE + 1];
    let mut store = Store::new(kinds.clone(), vec![false; kinds.len()]);
    let mut logs = vec![false; kinds.len()];
    logs[TABLE] = true;
    store.declare_logs(&logs).unwrap();
    store.attach(Encoder::new(&kinds, Writing { cap: 256, ..Writing::at(3) }).unwrap());
    let mut memory = Memory::default();
    store.begin_run(0);
    for at in 0..HOURS {
        store.begin_hour(at as Hour, &[]).unwrap();
        for table in 0..=TABLE {
            store.status(table, Status::Complete, true, Some(at as Hour), true).unwrap();
        }
        log_hour(at, &mut store.table(TABLE).unwrap());
        store.end_hour(&mut memory).unwrap();
        if at % BLOCK == BLOCK - 1 {
            store.end_block();
        }
    }
    Arc::new(Recording { bytes: memory.bytes, runs: vec![Run::new(0, memory.pages).unwrap()], kinds })
}

/// A log's row as an answer shows it: key, hour and each cell of `columns`.
type LogRow = (u128, Hour, Vec<(i64, State)>);

/// Every row the log appended by game hour, and every ordinal it forgot, read directly by span.
fn log_read_directly(pages: &Recording, columns: &[usize]) -> (Vec<(u64, LogRow)>, Vec<(Hour, u64)>) {
    let run = &pages.runs[0];
    let (mut appended, mut forgotten) = (Vec::new(), Vec::new());
    replay_columnar::unit::reader::log_span(&pages.bytes, &run.pages, &LOG, TABLE, columns, run.first_hour, Hour::MIN..=Hour::MAX, &mut |hour, event| match event {
        replay_columnar::unit::reader::LogEvent::Appended { ordinal, key, rows, at, .. } => {
            appended.push((ordinal, (key, hour, columns.iter().map(|&column| crate::evaluate::number(rows.value(column, at))).collect())));
        }
        replay_columnar::unit::reader::LogEvent::Forgotten { from, to, .. } => forgotten.extend((from..to).map(|ordinal| (hour, ordinal))),
        replay_columnar::unit::reader::LogEvent::Named { .. } => {}
    })
    .unwrap();
    (appended, forgotten)
}

/// The rows a log answer pushed: each one's key, insert hour and cells.
fn pushed(answer: &Answer) -> Vec<LogRow> {
    let hours = answer.hours.clone().expect("rows appended");
    (0..answer.keys.len()).map(|row| (answer.keys[row], hours[row], (0..answer.columns).map(|column| (answer.values[row * answer.columns + column], answer.states[row * answer.columns + column])).collect())).collect()
}

/// The steps from a seek's reset on: those before it were built before the seek.
fn since_the_reset(mut played: Vec<Played>) -> Vec<Played> {
    let reset = played.iter().position(|played| played.stats.reset).expect("the seek's reset");
    played.split_off(reset)
}

/// A followed feed of a log table reads the log's lane: the table is declared keyed and not
/// passing (the playhead says it is a log), and at 1, 24 and 240 hours a step, across seeks forward
/// and back, every stretch between seeks pushes exactly the rows the log's span gives for its
/// hours, in order, with their hours and cells: rows forgotten in the hour of their append, before
/// and after an append, and in runs included.
#[test]
fn a_followed_feed_of_a_log_pushes_what_its_span_gives() {
    let pages = record_log();
    let columns = vec![0, 1, 2];
    let (appended, _) = log_read_directly(&pages, &columns);
    for every in [1, 24, 240] {
        let mut harness = stepping(&pages, 64 << 10, every);
        harness.follow.follow(1, Query::Cells { table: TABLE, columns: columns.clone(), rows: Rows::Appended }).unwrap();
        assert_eq!(harness.follow.declared().unwrap(), vec![playhead::Declared { table: TABLE, columns: columns.clone(), keyed: true, passing: false }]);
        let mut stretches: Vec<(Hour, Hour, Vec<LogRow>)> = Vec::new();
        for (seek, to) in [(0, 400), (1000, 1250), (600, 900), (1300, HOURS as Hour - 1)] {
            harness.seek(seek);
            let mut stretch = (seek, seek, Vec::new());
            for played in since_the_reset(harness.play(to)) {
                stretch.1 = stretch.1.max(played.head);
                stretch.2.extend(played.answers().iter().flat_map(|(_, _, answer)| pushed(answer)));
            }
            stretches.push(stretch);
        }
        for (from, to, rows) in stretches {
            let wanted: Vec<LogRow> = appended.iter().filter(|(_, row)| from < row.1 && row.1 <= to).map(|(_, row)| row.clone()).collect();
            assert!(!wanted.is_empty(), "rows in {from}..{to}");
            assert_eq!(rows, wanted, "{every} hours a step: the stretch {from}..={to}");
        }
    }
}

/// A followed list of what the game lists now holds, in every answer, the log's listed rows at
/// the answer's hour (by key, in ordinal order, with their cells), at 20 seeks and the steps after
/// them: the rows listed at a seek are asked by ordinal and the steps' appends and forgets kept.
/// Their cells are read on a fetcher and taken in a later step's hook: after each seek's reset the
/// test asks the rows itself and waits for them (one fetcher serves requests in order, so the
/// hook's, made at the reset, is read by then), and the first step built after takes it: past the
/// ring's steps, built before.
#[test]
fn a_followed_list_of_a_log_holds_its_listed_rows_at_twenty_seeks() {
    let pages = record_log();
    let columns = vec![0, 1, 2];
    let (appended, forgotten) = log_read_directly(&pages, &columns);
    let rows: BTreeMap<u64, LogRow> = appended.iter().cloned().collect();
    let listed = |hour: Hour| -> BTreeMap<u64, &LogRow> {
        let mut listed: BTreeMap<u64, &LogRow> = rows.iter().filter(|(_, row)| row.1 <= hour).map(|(&ordinal, row)| (ordinal, row)).collect();
        for &(_, ordinal) in forgotten.iter().filter(|(at, _)| *at <= hour) {
            listed.remove(&ordinal);
        }
        listed
    };
    let listed_at = |hour: Hour| -> Vec<(u128, Vec<(i64, State)>)> { listed(hour).values().map(|row| (row.0, row.2.clone())).collect() };
    let mut harness = harness(&pages, 64 << 10);
    harness.follow.follow(2, Query::Cells { table: TABLE, columns: columns.clone(), rows: Rows::Listed }).unwrap();
    let mut answered = 0;
    for seek in [0, 1, 37, 719, 720, 721, 1000, 1150, 100, 500, 1190, 13, 50, 650, 900, 1100, 1200, 300, 800, 1180] {
        harness.seek(seek);
        let mut played = since_the_reset(harness.play(seek + 24));
        let ordinals: Vec<u64> = listed(seek).keys().copied().collect();
        let read = harness.playhead().rows(Wanted { run: 0, table: TABLE, ordinals: ordinals.clone(), columns: columns.clone() }).wait();
        assert_eq!(read.map(|rows| rows.ordinals().to_vec()).map_err(|error| error.to_string()), Ok(ordinals), "the rows listed at {seek}, read by ordinal");
        played.extend(harness.play(seek + 24 + 6 * 24));
        let answers: Vec<(Hour, Answer)> = played.iter().flat_map(Played::answers).map(|(_, hour, answer)| (hour, answer)).collect();
        let steps: Vec<_> = played.iter().map(|played| (played.stats.first, played.stats.last, played.stats.reset, played.head, played.stats.touched, played.stats.evaluated, played.stats.changed, played.stats.sent)).collect();
        assert!(answers.iter().any(|(hour, _)| *hour >= seek), "an answer after the seek to {seek}: the steps (first, last, reset, head, touched, evaluated, changed, sent) {steps:?}; {:?}", harness.follow.counters());
        for (hour, answer) in answers {
            assert!(answer.not_recorded.is_none() && answer.hours.is_none(), "rows at {hour}");
            let held: Vec<(u128, Vec<(i64, State)>)> = (0..answer.keys.len()).map(|row| (answer.keys[row], (0..answer.columns).map(|column| (answer.values[row * answer.columns + column], answer.states[row * answer.columns + column])).collect())).collect();
            assert_eq!(held, listed_at(hour), "the rows listed at {hour}, after the seek to {seek}");
            answered += 1;
        }
    }
    assert!(answered >= 20);
}

/// What the game lists now is a log's: followed on a table holding state, it answers why.
#[test]
fn a_followed_list_of_a_table_holding_state_is_refused_by_name() {
    let pages = record(quiet);
    let mut harness = harness(&pages, 64 << 10);
    harness.follow.follow(3, Query::Cells { table: TABLE, columns: vec![0], rows: Rows::Listed }).unwrap();
    harness.seek(0);
    let refused = harness.play(48).iter().flat_map(Played::answers).find_map(|(_, _, answer)| answer.not_recorded).expect("an answer");
    assert!(refused.contains("Countries holds state"), "{refused}");
}

/// An answer's rows as its reader holds them: each key's cells and reason.
fn held_rows(answer: &Answer) -> Vec<(u128, Vec<(i64, State)>, Option<String>)> {
    (0..answer.rows())
        .map(|row| {
            let cells = (0..answer.columns).map(|column| (answer.values[row * answer.columns + column], answer.states[row * answer.columns + column])).collect();
            (answer.keys[row], cells, answer.reason(row).map(str::to_owned))
        })
        .collect()
}

/// `changed`'s rows laid over `held` by key, as the UI's reader lays them.
fn lay(held: &mut [(u128, Vec<(i64, State)>, Option<String>)], changed: &Answer) {
    for row in held_rows(changed) {
        let at = held.iter().position(|(key, ..)| *key == row.0).expect("rows changed name only keys held");
        held[at] = row;
    }
}

#[test]
fn a_live_query_pushes_only_the_rows_that_changed_and_its_reader_holds_the_answer_at_every_step() {
    let pages = record(quiet);
    let mut harness = harness(&pages, 64 << 10);
    // Key 100's column 0 moves every hour, key 105's column 3 at hours 100, 200 and 400, key 107 only in
    // a column not asked, and no row has key 999.
    let asked = [100, 105, 107, 999];
    harness.follow.follow(1, Query::Cells { table: TABLE, columns: vec![0, 3], rows: Rows::Live(asked.to_vec()) }).unwrap();
    assert!(harness.follow.follow(2, Query::Cells { table: TABLE, columns: vec![0], rows: Rows::Live(vec![105, 100]) }).is_err(), "live keys are named ascending");
    harness.seek(0);
    let played = harness.play(420);
    let mut held: Option<Vec<(u128, Vec<(i64, State)>, Option<String>)>> = None;
    let (mut whole, mut changes, mut unrecorded) = (0, 0, 0);
    for played in &played {
        for (_, hour, answer) in played.answers() {
            if let Some(reason) = &answer.not_recorded {
                assert_eq!(reason, "Countries not recorded at this hour", "no verb to agree with a plural table");
                held = None;
                unrecorded += 1;
                continue;
            }
            if answer.changed {
                let moved = moves_slot_5(played);
                assert_eq!(answer.keys, if moved { vec![100, 105] } else { vec![100] }, "only the rows that changed, hours {}..{}", played.from, played.head);
                lay(held.as_mut().expect("rows changed come only over rows held"), &answer);
                changes += 1;
            } else {
                assert_eq!(answer.keys, asked, "a whole answer holds every key asked");
                held = Some(held_rows(&answer));
                whole += 1;
            }
            let fresh = harness.fresh(&keys(&asked, &[0, 3]), hour);
            assert_eq!(held.as_deref(), Some(held_rows(&fresh).as_slice()), "the reader holds the answer at hour {hour}");
            assert_eq!(held.as_ref().unwrap()[3].2.as_deref(), Some(crate::live::NO_ROW), "a key with no row is missing by name, never zero");
        }
    }
    assert_eq!((whole, unrecorded), (2, 1), "whole at the first answer and again after the hours not recorded, which are said once");
    assert!(changes > 10, "{changes} answers of rows changed");
}

/// A message of one answer of rows (`changed`: rows changed) of id 4, a column each.
fn rows_message(hour: Hour, changed: bool, rows: &[(u128, i64)]) -> Vec<u8> {
    let mut table = crate::answer::Table::default();
    table.clear(1, false);
    for &(key, value) in rows {
        table.row(key);
        table.cell(value, State::Value);
    }
    let mut body = Vec::new();
    if changed { table.encode_changed(&mut body) } else { table.encode(&mut body) }
    let mut message = Vec::new();
    crate::answer::begin(&mut message, 0, hour, false);
    crate::answer::push(&mut message, 4, hour, &body);
    message
}

#[test]
fn rows_changed_queued_behind_the_rows_they_change_are_laid_over_them_never_dropped() {
    let whole = rows_message(10, false, &[(1, 10), (2, 20), (3, 30)]);
    let first = rows_message(11, true, &[(2, 21)]);
    let second = rows_message(12, true, &[(3, 32), (2, 22)]);
    let (merged, superseded) = crate::answer::merge(&first, &second).unwrap();
    assert_eq!(superseded, 0, "rows changed are laid over, not superseded");
    let (_, _, body) = decode(&merged).unwrap().answers[0];
    let changed = Answer::parse(body).unwrap();
    assert!(changed.changed);
    assert_eq!((changed.keys.clone(), changed.values.clone()), (vec![2, 3], vec![22, 32]), "two changes are the newer of each row");
    let (merged, _) = crate::answer::merge(&whole, &merged).unwrap();
    let decoded = decode(&merged).unwrap();
    assert_eq!(decoded.hour, 12);
    let rows = Answer::parse(decoded.answers[0].2).unwrap();
    assert!(!rows.changed, "changes laid over whole rows are whole rows");
    assert_eq!((rows.keys, rows.values), (vec![1, 2, 3], vec![10, 22, 32]));
    assert!(crate::answer::merge(&rows_message(10, false, &[(1, 10)]), &rows_message(11, true, &[(9, 1)])).is_err(), "a change of a row not held is refused by name");
}

#[test]
fn a_live_step_reads_the_cells_of_the_rows_it_touched_only() {
    let pages = record(quiet);
    let mut harness = harness(&pages, 64 << 10);
    // Of four rows, key 100 moves every hour, key 106's column 2 is written every hour with
    // the hour's parity, key 105 moves at hours 100 and 200, and key 107 only in a column not asked:
    // a step touches two rows, three at hours 100 and 200, never all four. A step ending in the hours
    // Countries was not recorded (300 to 329, built ahead of the head) reads no cell and says why;
    // the step after it reads every row again.
    harness.follow.follow(1, Query::Cells { table: TABLE, columns: vec![0, 2, 3], rows: Rows::Live(vec![100, 105, 106, 107]) }).unwrap();
    harness.seek(0);
    let played = harness.play(290);
    let first = played.iter().position(|played| !played.answers().is_empty()).expect("a first answer");
    assert_eq!(played[first].stats.cells, 4 * 3, "the first step reads every row once");
    let (mut checked, mut after_gap) = (0, false);
    for played in played[first + 1..].iter().filter(|played| played.moved()) {
        if unread(played.head as u32) {
            assert_eq!(played.stats.cells, 0, "hours {}..{}: Countries not recorded at the step's end", played.from, played.head);
            assert!(played.answers().iter().all(|(_, _, answer)| answer.not_recorded.is_some()), "hours {}..{}: only why", played.from, played.head);
            after_gap = true;
            continue;
        }
        if std::mem::take(&mut after_gap) {
            assert_eq!(played.stats.cells, 4 * 3, "hours {}..{}: every row read again after not recorded", played.from, played.head);
            continue;
        }
        let touched = 2 + u32::from(moves_slot_5(played));
        assert_eq!(played.stats.cells, 3 * touched, "hours {}..{}: the touched rows' cells, not the followed rows'", played.from, played.head);
        let changed: Vec<u128> = played.answers().iter().flat_map(|(_, _, answer)| answer.keys.clone()).collect();
        assert!(!changed.contains(&107) && (touched == 3 || !changed.contains(&105)), "only rows the step touched are sent: {changed:?}");
        checked += 1;
    }
    assert!(checked > 5, "{checked} steps checked");
}
