use super::*;
use dashboard_follow::panel::ProductReader;
use dashboard_query::{identity::Holders, timeline::Timeline};
use replay::campaign::Replay;
use replay_recording::{cells::TableReader, columnar_frames::Recording};
use replay_columnar::store::Value;

struct Track {
    reader: TableReader,
    rows: BTreeMap<u128, Box<[Cell]>>,
}

impl Track {
    fn new(recording: &Recording, piece: &Piece, table: TableId, columns: &[usize], keep: impl Fn(u128) -> bool, count: &mut u64) -> io::Result<Self> {
        let mut reader = TableReader::new(recording, piece.run, table, Some(columns))?;
        reader.index_keys();
        reader.seek(recording, recording.run(piece.run)?.ordinal(piece.last).unwrap())?;
        let mut track = Self { reader, rows: BTreeMap::new() };
        track.refresh(&keep, count);
        Ok(track)
    }

    fn update(&mut self, key: u128, count: &mut u64) -> bool {
        let r = &self.reader;
        let slot = r.slot(key).filter(|slot| r.available() && r.read_rows().is_none_or(|rows| rows.binary_search(slot).is_ok()));
        let Some(slot) = slot else { return self.rows.remove(&key).is_some() };
        *count += 1;
        let cells = r.columns().iter().map(|&col| match r.cells().value(col, slot) {
            Value::Number(n) => Cell::Number(n as i64),
            Value::Missing => Cell::Missing,
            _ => Cell::Unrecorded,
        }).collect::<Box<[_]>>();
        if self.rows.get(&key) == Some(&cells) { false } else { self.rows.insert(key, cells); true }
    }

    fn include(&mut self, keep: impl Fn(u128) -> bool, count: &mut u64) {
        let keys: Vec<_> = self.reader.cells().rows().filter_map(|s| self.reader.cells().key(s)).filter(|&k| keep(k)).collect();
        for key in keys { self.update(key, count); }
    }

    fn refresh(&mut self, keep: &impl Fn(u128) -> bool, count: &mut u64) -> bool {
        let keys: BTreeSet<_> = self.rows.keys().copied().chain(self.reader.cells().rows().filter_map(|s| self.reader.cells().key(s)).filter(|&k| keep(k))).collect();
        let mut changed = false;
        for key in keys { changed |= self.update(key, count); }
        changed
    }

    fn step(&mut self, recording: &Recording, keep: impl Fn(u128) -> bool, count: &mut u64) -> io::Result<bool> {
        let before = self.reader.status();
        self.reader.step(recording)?;
        let r = &self.reader;
        if before != r.status() || r.read_rows().is_some() {
            let changed = self.refresh(&keep, count);
            return Ok(changed || before != self.reader.status());
        }
        let changes = r.changes();
        let keys: BTreeSet<_> = changes.inserted.iter().chain(changes.written.iter().flatten()).filter_map(|&s| r.cells().key(s))
            .chain(changes.deleted.iter().map(|&(_, key)| key)).filter(|&k| keep(k)).collect();
        let mut changed = false;
        for key in keys { changed |= self.update(key, count); }
        Ok(changed)
    }
}

struct Current {
    piece: Piece,
    owners: BTreeMap<u32, Vec<(i32, i32)>>,
    known: BTreeSet<u32>,
    programs: BTreeSet<u128>,
    projects: Track,
    history: Track,
    scientists: Option<Track>,
}

struct Episodes {
    fold: Fold,
    observed: Vec<(i32, i32)>,
    current: Option<Current>,
    spelling: Option<Spelling>,
    through: Option<i32>,
    data: Arc<dashboard_follow::panel::PanelData>,
    links: HashMap<u32, RunLinks>,
}

fn program(cells: &[Cell]) -> Option<u128> {
    let raw = number(*cells.get(1)?)? as u64;
    (raw as u32 == PROGRAM_REFERENCE).then_some(u128::from(raw >> 32))
}

fn open_programs(keys: &BTreeSet<u128>, open: impl FnOnce() -> io::Result<Track>) -> io::Result<Option<Track>> {
    (!keys.is_empty()).then(open).transpose()
}

impl Episodes {
    fn new(first: i32) -> Self {
        Self { fold: Fold::new(first), observed: Vec::new(), current: None, spelling: None, through: None, data: Arc::new(Err(String::new())), links: HashMap::new() }
    }

    fn enter(&mut self, q: &Query<'_>, piece: Piece, last: bool, cancelled: &AtomicBool, count: &mut u64) -> io::Result<()> {
        let links = links_of(q, &mut self.links, piece.run)?;
        let read = read_piece(q, links, &piece)?;
        fold_piece(&mut self.fold, &piece, &read, &mut self.observed, cancelled)?;
        self.current = if last {
            let known: BTreeSet<_> = read.owners.keys().copied().collect();
            let programs: BTreeSet<_> = read.projects.iter().flat_map(|h| &h.keys).flat_map(|(_, s)| s).filter_map(|s| program(s.cells.as_deref()?)).collect();
            let r = q.replay.recording();
            let owners = owner_ids(links, piece.slot, piece.first, q.timeline.at(piece.last).unwrap().last);
            Some(Current {
                projects: Track::new(r, &piece, TableId::Projects, &PROJECT_COLUMNS, |k| known.contains(&((k >> 32) as u32)), count)?,
                history: Track::new(r, &piece, TableId::ProjectHistory, &HISTORY_COLUMNS, |k| known.contains(&((k >> 64) as u32)), count)?,
                scientists: open_programs(&programs, || Track::new(r, &piece, TableId::ProjectPrograms, &SCIENTIST_COLUMNS, |k| programs.contains(&k), count))?,
                piece, owners, known, programs,
            })
        } else { None };
        Ok(())
    }

    fn advance(&mut self, q: &Query<'_>, piece: &Piece, cancelled: &AtomicBool, count: &mut u64) -> io::Result<bool> {
        let c = self.current.as_mut().expect("continuing the same piece");
        let hour = piece.last;
        let mut observed_change = false;
        let r = q.replay.recording();
        while c.piece.last < hour {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let before = c.piece.last;
            let next = before + 1;
            let mut boundary = false;
            for (raw, runs) in &c.owners {
                let linked = within(runs, next);
                if linked && c.known.insert(*raw) {
                    c.projects.include(|k| (k >> 32) as u32 == *raw, count);
                    c.history.include(|k| (k >> 64) as u32 == *raw, count);
                }
                boundary |= linked != within(runs, before);
            }
            boundary |= c.projects.step(r, |k| c.known.contains(&((k >> 32) as u32)), count)?;
            boundary |= c.history.step(r, |k| c.known.contains(&((k >> 64) as u32)), count)?;
            if let Some(track) = &mut c.scientists { boundary |= track.step(r, |k| c.programs.contains(&k), count)?; }
            observed_change |= boundary;
            if c.projects.reader.available() {
                if boundary {
                    let mut history: BTreeMap<u128, Vec<Entry>> = BTreeMap::new();
                    for (&key, cells) in &c.history.rows {
                        if let Some(entry) = history_entry(key as u32, cells) { history.entry(key >> 32).or_default().push(entry); }
                    }
                    let mut rows = Vec::new();
                    for (&key, cells) in &c.projects.rows {
                        if let Some(id) = program(cells) && c.programs.insert(id) {
                            if let Some(track) = &mut c.scientists { track.update(id, count); }
                            else {
                                let at = Piece { last: next, ..c.piece };
                                c.scientists = Some(Track::new(r, &at, TableId::ProjectPrograms, &SCIENTIST_COLUMNS, |k| c.programs.contains(&k), count)?);
                            }
                        }
                        if within(&c.owners[&((key >> 32) as u32)], next) && let Some(row) = project_row(key as u32, cells) {
                            rows.push((key, row));
                        }
                    }
                    let shown: Vec<_> = rows.iter().map(|(key, row)| Observed {
                        row, history: c.history.reader.available().then(|| history.get(key).map_or(&[][..], Vec::as_slice)),
                        person: person(row.program, (row.program as u32 == PROGRAM_REFERENCE).then(|| c.scientists.as_ref().and_then(|t| t.rows.get(&u128::from(row.program >> 32))).map(AsRef::as_ref))),
                    }).collect();
                    self.fold.observe((piece.slot, piece.id), next, next, &shown);
                } else { self.fold.extend(before, next); }
                if let Some(span) = self.observed.last_mut().filter(|s| s.1 == before) { span.1 = next; }
                else { self.observed.push((next, next)); }
            }
            c.piece.last = next;
        }
        Ok(observed_change)
    }

    fn extend_reply(&mut self, before: i32, hour: i32) -> io::Result<()> {
        let value = Arc::make_mut(&mut self.data).as_mut().expect("presented");
        for group in ["available", "identities"] {
            if let Some(span) = value[group].as_array_mut().and_then(|v| v.last_mut())
                && span["lastHour"] == before {
                span["lastHour"] = hour.into();
            }
        }
        for index in self.fold.open_episodes() {
            if let Some(last) = value["projects"][index]["runs"].as_array_mut().and_then(|v| v.last_mut())
                && last["lastHour"] == before {
                last["lastHour"] = hour.into();
            }
        }
        if hour.to_string().len() > before.to_string().len() { dashboard_base::products::check_reply_budget(value)?; }
        Ok(())
    }
}

pub(super) fn reader(replay: Arc<Replay>, timeline: Arc<Timeline>, holders: Arc<Holders>, params: Arc<SpecialProjectsParams>, content: Arc<ProjectContent>, executable: Option<engine_executable::executable_tables::Tables>) -> ProductReader {
    let state = Mutex::new(None::<Episodes>);
    Arc::new(move |hour, reset, cancelled, _step| {
        let query = Query { replay: &replay, holders: holders.clone(), timeline: timeline.clone() };
        let mut held = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut count = 0;
        let answer = engine_executable::executable_tables::scope(executable, || -> io::Result<_> {
            if let Some(s) = held.as_ref().filter(|s| s.through == Some(hour)) { return Ok(s.data.clone()); }
            if timeline.at(hour).is_none() { return Err(dashboard_core::refused("Special projects hour is not recorded")); }
            if reset || held.as_ref().is_none_or(|s| s.through.is_some_and(|h| h > hour)) {
                *held = Some(Episodes::new(timeline.first));
            }
            let s = held.as_mut().unwrap();
            let (pieces, identities) = pieces(&query, &params.country, hour);
            let tag = query.holders.name(&params.country, hour);
            let mut changed = s.through.is_none() || s.data.as_ref().as_ref().is_ok_and(|reply| reply["tag"].as_str() != Some(tag.as_str()));
            for piece in pieces.into_iter().filter(|p| s.through.is_none_or(|h| p.last > h)).collect::<Vec<_>>() {
                if s.current.as_ref().is_some_and(|c| (c.piece.first, c.piece.run) == (piece.first, piece.run)) {
                    changed |= s.advance(&query, &piece, cancelled, &mut count)?;
                } else {
                    let last = piece.last == hour;
                    s.enter(&query, piece, last, cancelled, &mut count)?;
                    changed = true;
                }
            }
            let previous = s.through.replace(hour);
            if !changed && let Some(before) = previous {
                if s.current.as_ref().is_some_and(|c| c.piece.last == hour) {
                    s.extend_reply(before, hour)?;
                } else if s.current.as_ref().is_some_and(|c| c.piece.last > before) {
                    changed = true;
                }
            }
            if changed {
                let reply = present(&query, &params, &content, &s.fold, s.observed.clone(), identities, &tag, cancelled, &mut s.spelling)?;
                s.data = Arc::new(Ok(serde_json::to_value(reply)?));
            }
            Ok(s.data.clone())
        });
        let value = answer.unwrap_or_else(|e| { *held = None; Arc::new(Err(e.to_string())) });
        dashboard_follow::panel::ProductRead { value, rows: count, fresh: true, pending: false, live: None, lanes: Vec::new() }
    })
}

#[cfg(test)]
#[test]
fn an_unreferenced_program_table_can_be_absent() {
    let missing = || Err::<Track, _>(io::Error::other("ProjectPrograms missing"));
    assert!(open_programs(&BTreeSet::new(), missing).unwrap().is_none());
    assert!(open_programs(&BTreeSet::from([7]), missing).is_err());
}
