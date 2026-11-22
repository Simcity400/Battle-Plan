use super::*;
use dashboard_follow::panel::ProductReader;
use dashboard_query::{identity::Holders, timeline::Timeline};
use replay::{campaign::Replay, cells::TableReader, columnar_frames::Recording};
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

type Proven = (BTreeMap<u32, i32>, BTreeMap<u32, i32>);

struct Current {
    piece: Piece,
    owners: BTreeMap<u32, Vec<(i32, i32)>>,
    edges: Vec<i32>,
    linked: BTreeSet<u32>,
    recheck: i32,
    known: BTreeSet<u32>,
    programs: BTreeSet<u128>,
    projects: Track,
    history: Track,
    reads: Track,
    scientists: Option<Track>,
    proven: Proven,
}

fn read_of(known: &BTreeSet<u32>, key: u128) -> bool {
    known.contains(&(key as u32)) && [TableId::Projects, TableId::ProjectHistory].iter().any(|table| key >> 32 == table.index() as u128)
}

impl Current {
    fn supplies(&self, table: TableId, raw: u32, hour: i32) -> Option<i32> {
        supplied(&self.owners, raw, sample(self.reads.rows.get(&read_key(table, raw)).map(AsRef::as_ref)), hour)
    }

    fn prove(&mut self, hour: i32) -> bool {
        let reads = |table| self.linked.iter().filter_map(|&raw| Some((raw, self.supplies(table, raw, hour)?))).collect::<BTreeMap<_, _>>();
        let projects = if self.projects.reader.status() == Status::Complete { reads(TableId::Projects) } else { BTreeMap::new() };
        let histories = if self.history.reader.status() == Status::Complete { reads(TableId::ProjectHistory) } else { BTreeMap::new() };
        self.recheck = projects.values().chain(histories.values()).min().map_or(i32::MAX, |read| read.saturating_add(STAGGER_PERIOD));
        let proven = (projects, histories);
        let changed = proven != self.proven;
        self.proven = proven;
        changed
    }
}

fn edges(owners: &BTreeMap<u32, Vec<(i32, i32)>>) -> Vec<i32> {
    let mut edges: Vec<i32> = owners.values().flatten().flat_map(|&(first, last)| [Some(first), last.checked_add(1)]).flatten().collect();
    edges.sort_unstable();
    edges.dedup();
    edges
}

struct Episodes {
    fold: Fold,
    observed: Vec<(i32, i32)>,
    partial: Vec<(i32, i32)>,
    current: Option<Current>,
    spelling: Option<Spelling>,
    through: Option<i32>,
    data: Arc<dashboard_follow::panel::PanelData>,
    links: HashMap<u32, RunLinks>,
    holding: Holding,
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
        Self { fold: Fold::new(first), observed: Vec::new(), partial: Vec::new(), current: None, spelling: None, through: None, data: Arc::new(Err(String::new())), links: HashMap::new(), holding: Holding::default() }
    }

    fn enter(&mut self, q: &Query<'_>, piece: Piece, last: bool, cancelled: &AtomicBool, count: &mut u64) -> io::Result<()> {
        let links = links_of(q, &mut self.links, piece.run)?;
        let read = read_piece(q, links, &piece, None)?;
        fold_piece(&mut self.fold, &piece, &read, &mut self.observed, &mut self.partial, cancelled)?;
        self.current = if last {
            let known: BTreeSet<_> = read.owners.keys().copied().collect();
            let programs: BTreeSet<_> = read.projects.iter().flat_map(|h| &h.keys).flat_map(|(_, s)| s).filter_map(|s| program(s.cells.as_deref()?)).collect();
            let r = q.replay.recording();
            let owners = owner_ids(links, piece.slot, piece.reach(), q.timeline.at(piece.last).unwrap().last);
            let linked: BTreeSet<u32> = owners.iter().filter(|(_, runs)| within(runs, piece.last)).map(|(&raw, _)| raw).collect();
            let mut current = Current {
                projects: Track::new(r, &piece, TableId::Projects, &PROJECT_COLUMNS, |k| known.contains(&((k >> 32) as u32)), count)?,
                history: Track::new(r, &piece, TableId::ProjectHistory, &HISTORY_COLUMNS, |k| known.contains(&((k >> 64) as u32)), count)?,
                reads: Track::new(r, &piece, TableId::SliceReads, &read_columns()?, |k| read_of(&known, k), count)?,
                scientists: open_programs(&programs, || Track::new(r, &piece, TableId::ProjectPrograms, &SCIENTIST_COLUMNS, |k| programs.contains(&k), count))?,
                proven: Default::default(),
                edges: edges(&owners),
                recheck: i32::MAX,
                piece, owners, linked, known, programs,
            };
            current.prove(current.piece.last);
            Some(current)
        } else { None };
        Ok(())
    }

    fn advance(&mut self, q: &Query<'_>, piece: &Piece, cancelled: &AtomicBool, count: &mut u64) -> io::Result<bool> {
        let c = self.current.as_mut().expect("continuing the same piece");
        let hour = piece.last;
        let mut observed_change = false;
        let r = q.replay.recording();
        while c.piece.last < hour {
            crate::metadata::check_cancel(cancelled)?;
            let before = c.piece.last;
            let next = before + 1;
            let mut boundary = false;
            if c.edges.binary_search(&next).is_ok() {
                let linked: BTreeSet<u32> = c.owners.iter().filter(|(_, runs)| within(runs, next)).map(|(&raw, _)| raw).collect();
                for &raw in &linked {
                    if c.known.insert(raw) {
                        c.projects.include(|k| (k >> 32) as u32 == raw, count);
                        c.history.include(|k| (k >> 64) as u32 == raw, count);
                        c.reads.include(|k| k as u32 == raw && read_of(&c.known, k), count);
                    }
                }
                boundary |= linked != c.linked;
                c.linked = linked;
            }
            boundary |= c.projects.step(r, |k| c.known.contains(&((k >> 32) as u32)), count)?;
            boundary |= c.history.step(r, |k| c.known.contains(&((k >> 64) as u32)), count)?;
            boundary |= c.reads.step(r, |k| read_of(&c.known, k), count)?;
            if let Some(track) = &mut c.scientists { boundary |= track.step(r, |k| c.programs.contains(&k), count)?; }
            if matches!(c.projects.reader.status(), Status::Truncated { .. }) {
                extend(&mut self.partial, next, next);
            }
            if boundary || next >= c.recheck {
                boundary |= c.prove(next);
            }
            let observable = c.projects.reader.status() == Status::Complete && c.proven.0.len() == c.linked.len();
            observed_change |= boundary;
            if boundary && c.projects.reader.status() == Status::Complete {
                    let (reads, histories) = c.proven.clone();
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
                        if reads.contains_key(&((key >> 32) as u32)) && let Some(row) = project_row((key >> 32) as u32, key as u32, cells) {
                            rows.push((key, row));
                        }
                    }
                    let shown: Vec<_> = rows.iter().map(|(key, row)| Observed {
                        row, read: reads[&row.owner], history: histories.get(&row.owner).map(|&read| (read, history.get(key).map_or(&[][..], Vec::as_slice))),
                        person: person(row.program, (row.program as u32 == PROGRAM_REFERENCE).then(|| c.scientists.as_ref().and_then(|t| t.rows.get(&u128::from(row.program >> 32))).map(AsRef::as_ref))),
                    }).collect();
                    if observable {
                        self.fold.observe(piece.possession(), next, next, &shown);
                    } else {
                        self.fold.absent(piece.possession(), next, |raw| reads.contains_key(&raw), &shown);
                    }
            } else if observable { self.fold.extend(before, next); }
            if observable {
                extend(&mut self.observed, next, next);
            }
            c.piece.last = next;
        }
        Ok(observed_change)
    }

    fn follow(&mut self, query: &Query<'_>, country: &Country, hour: i32, cancelled: &AtomicBool, count: &mut u64) -> io::Result<(bool, Holding)> {
        let (pieces, holding) = pieces(query, country, query.timeline.first, hour);
        let mut changed = false;
        for piece in pieces.into_iter().filter(|p| self.through.is_none_or(|h| p.last > h)).collect::<Vec<_>>() {
            if self.current.as_ref().is_some_and(|c| (c.piece.first, c.piece.run) == (piece.first, piece.run)) {
                changed |= self.advance(query, &piece, cancelled, count)?;
            } else {
                let last = piece.last == hour;
                self.enter(query, piece, last, cancelled, count)?;
                changed = true;
            }
        }
        Ok((changed, holding))
    }

    fn extend_reply(&mut self, before: i32, hour: i32) -> io::Result<()> {
        let value = Arc::make_mut(&mut self.data).as_mut().expect("presented");
        for group in ["available", "partial", "identities"] {
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
            let tag = query.holders.name(&params.country, hour);
            let mut changed = s.through.is_none() || s.data.as_ref().as_ref().is_ok_and(|reply| reply["tag"].as_str() != Some(tag.as_str()));
            let (stepped, holding) = s.follow(&query, &params.country, hour, cancelled, &mut count)?;
            changed |= stepped || holding.unresolved != s.holding.unresolved;
            s.holding = holding;
            let previous = s.through.replace(hour);
            if !changed && let Some(before) = previous {
                if s.current.as_ref().is_some_and(|c| c.piece.last == hour) {
                    s.extend_reply(before, hour)?;
                } else if s.current.as_ref().is_some_and(|c| c.piece.last > before) {
                    changed = true;
                }
            }
            if changed {
                let reply = present(&query, &params, &content, &s.fold, Observation { observed: s.observed.clone(), partial: s.partial.clone() }, s.holding.clone(), &tag, cancelled, &mut s.spelling)?;
                s.data = Arc::new(Ok(serde_json::to_value(reply)?));
            }
            Ok(s.data.clone())
        });
        let value = answer.unwrap_or_else(|e| { *held = None; Arc::new(Err(e.to_string())) });
        dashboard_follow::panel::ProductRead { value, rows: count, fresh: true, pending: false, live: None, lanes: Vec::new() }
    })
}

#[cfg(test)]
pub(super) fn followed(query: &Query<'_>, country: &Country, first: i32, through: i32) -> io::Result<(Fold, Vec<(i32, i32)>, Vec<(i32, i32)>)> {
    let mut episodes = Episodes::new(first);
    for hour in first..=through {
        episodes.follow(query, country, hour, &AtomicBool::new(false), &mut 0)?;
        episodes.through = Some(hour);
    }
    Ok((episodes.fold, episodes.observed, episodes.partial))
}

#[cfg(test)]
#[test]
fn an_unreferenced_program_table_can_be_absent() {
    let missing = || Err::<Track, _>(io::Error::other("ProjectPrograms missing"));
    assert!(open_programs(&BTreeSet::new(), missing).unwrap().is_none());
    assert!(open_programs(&BTreeSet::from([7]), missing).is_err());
}
