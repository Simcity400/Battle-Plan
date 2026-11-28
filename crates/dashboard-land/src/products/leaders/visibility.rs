use super::{Evaluator, LeaderVisibility, LeadersContext, UnitLeaders, UnitLeadersParams, catalogue, core_gap, handle, needs_world, visible, world_of};
use dashboard_base::products::at_hour::{Read, ReadsAt};
use dashboard_text::triggers::{RECORDED_TABLES, inputs::{self, TableRead}};
use replay::campaign::{HourCursor, SeekResult};
use replay_data::rows::{TableId, tables};
use replay_frame::frame::{DecodedFrame, Keys, TableView, ViewIdentity, discovering, not_declared};
use std::{collections::{BTreeMap, BTreeSet, HashMap}, io};

#[derive(Default)]
pub(super) struct Visibility {
    pub(super) held: Option<Dated>,
    dates: Dates,
    cursor: Option<(Vec<TableId>, HourCursor)>,
    proofs: HashMap<u64, Proof>,
    role_inputs: Option<([ViewIdentity; 5], i32)>,
    pub(super) revision: u64,
    pub(super) changed: BTreeSet<u64>,
}

pub(super) struct Dated {
    run: u32,
    first: i32,
    pub(super) through: i32,
    tables: BTreeSet<TableId>,
    roles: Vec<(u64, u32, usize)>,
    pub(super) decisions: HashMap<u64, Result<bool, String>>,
}

impl Dated {
    pub(super) fn covers(&self, run: u32, hour: i32) -> bool {
        self.run == run && (self.first..=self.through).contains(&hour)
    }
}

const CORE: [TableId; 4] = [TableId::CountryLinks, TableId::Countries, TableId::Provinces, TableId::States];

pub(super) fn exact_reads(params: &UnitLeadersParams, frame: &DecodedFrame, at: &ReadsAt<'_, LeadersContext>, step: usize) -> Vec<Read> {
    if step == 2 {
        if !UnitLeaders::visibility_needed(params, frame, at).0 {
            return Vec::new();
        }
        return [TableId::Provinces, TableId::States, TableId::Characters, TableId::UnitLeaderTraits].into_iter().map(Read::all).collect();
    }
    let mut trial = frame.clone();
    for &table in RECORDED_TABLES {
        if trial.tables[table.index()].held().is_not_selected() {
            trial.tables[table.index()] = not_declared(table);
        }
    }
    let rows = UnitLeaders::kept(params, frame, at);
    let decided = at.context.verdicts_at(at.run, at.hour);
    let (((), missing), _) = catalogue::capture_isolated(|| discovering(|| {
        let mut world = None;
        for row in rows.into_iter().filter(|row| needs_world(row) && !decided.contains_key(&handle(row))) {
            let _ = visible(row, &trial, &at.context.content, &mut world);
        }
    }));
    missing.into_iter().map(Read::all).collect()
}

struct Proof {
    run: u32,
    first: i32,
    owner: u32,
    role: usize,
    through: i32,
    decision: Result<bool, String>,
}

struct Changes {
    status: Vec<i32>,
    rows: Vec<(u128, Vec<i32>)>,
}

#[derive(Default)]
struct Dates {
    span: Option<(u32, i32, i32)>,
    tables: HashMap<(TableId, u64, bool), Option<Changes>>,
}

impl Changes {
    fn read(replay: &replay::campaign::Replay, run: u32, read: &TableRead, first: i32, last: i32) -> io::Result<Self> {
        use replay_lanes::lanes::{self, Keys};
        if replay.recording().predates(read.table.index()).is_some() {
            return Ok(Self { status: Vec::new(), rows: Vec::new() });
        }
        let cols: Vec<_> = (0..read.table.schema().len().min(64)).filter(|i| read.fields & (1 << i) != 0).collect();
        if !read.ends && let Ok(Some(changes)) = Self::complete(replay, run, read.table, &cols, first, last) {
            return Ok(changes);
        }
        let history = lanes::history(replay.recording(), run, read.table, &cols, Keys::All, first, last)?;
        let status = history.statuses.iter().map(|s| s.first).collect();
        let end = read.ends.then(|| history.columns.iter().position(|&c| read.table.field_name(c) == Some("end_hour"))).flatten();
        let rows = history.keys.into_iter().filter_map(|(key, steps)| {
            let changes: Vec<_> = dashboard_base::products::at_hour::row_change_hours(&steps, end, last).filter(|&hour| hour > first).collect();
            (!changes.is_empty()).then_some((key, changes))
        }).collect();
        Ok(Self { status, rows })
    }

    fn complete(replay: &replay::campaign::Replay, run: u32, table: TableId, columns: &[usize], first: i32, last: i32) -> io::Result<Option<Self>> {
        use replay_columnar::manifest::Status;
        use replay_recording::cells::TableReader;
        let recording = replay.recording();
        let Some(start) = recording.run(run)?.ordinal(first) else { return Ok(None) };
        let mut reader = TableReader::new(recording, run, table, Some(columns))?;
        reader.seek(recording, start)?;
        if reader.status() != Status::Complete { return Ok(None); }
        let mut dates = BTreeMap::<u128, Vec<i32>>::new();
        for previous in first..last {
            let hour = previous + 1;
            if !reader.step(recording)? || reader.status() != Status::Complete || reader.changes().anew {
                return Ok(None);
            }
            let changes = reader.changes();
            let keys = changes.inserted.iter().chain(changes.written.iter().flatten())
                .filter_map(|&slot| reader.cells().key(slot));
            for key in keys.chain(changes.deleted.iter().map(|&(_, key)| key)) {
                let dates = dates.entry(key).or_default();
                if dates.last() != Some(&hour) { dates.push(hour); }
            }
        }
        Ok(Some(Self { status: Vec::new(), rows: dates.into_iter().collect() }))
    }

    fn through(&self, read: &TableRead, frame: &DecodedFrame, last: i32) -> i32 {
        let hour = frame.hour;
        let mut next = self.status.get(self.status.partition_point(|&h| h <= hour)).copied();
        let (lo, hi) = read.rows.unwrap_or((0, u128::MAX));
        let start = self.rows.partition_point(|r| r.0 < lo);
        let slots = read.owner.and_then(|o| o.by.slots(frame));
        for (key, dates) in self.rows[start..].iter().take_while(|r| r.0 <= hi) {
            if let Some(owner) = read.owner && let Some(slots) = slots
                && slots.of(key.checked_shr(owner.shift).unwrap_or(0) as u32).is_some_and(|s| s != owner.slot) { continue; }
            if let Some(&at) = dates.get(dates.partition_point(|&h| h <= hour)) { next = Some(next.map_or(at, |n| n.min(at))); }
        }
        next.map_or(last, |n| n - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_base::test_fixtures::{CAMPAIGN, snapshot_records, source};
    use replay::campaign::Replay;
    use replay_data::rows::{ScriptFlagRow, TableRows};
    use replay_lanes::lanes::{self, Keys};

    #[test]
    fn complete_proofs_index_membership_and_selected_column_edits() {
        let table = TableId::ScriptFlags;
        let frames = [Some(1), Some(0), Some(0), None, Some(1)]
            .into_iter()
            .zip(100..)
            .map(|(value, hour)| {
                let mut frame = source(1, hour, None);
                let rows = value.into_iter().map(|value| ScriptFlagRow {
                    scope_kind: 3, scope_id: 9, token: 900, value, ..Default::default()
                }).collect();
                frame.tables[table.index()] = TableView::Complete(TableRows::ScriptFlags(rows)).into();
                frame
            }).collect();
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[snapshot_records(1, frames)])
            .unwrap().into_timelines(&Default::default()).unwrap();
        let run = replay.block_at(1, 100).unwrap().run;
        let field = dashboard_declare::field(table, "value").unwrap();
        for columns in [&[][..], &[field][..]] {
            let proof = Changes::complete(&replay, run, table, columns, 100, 104).unwrap().unwrap();
            let history = lanes::history(replay.recording(), run, table, columns, Keys::All, 100, 104).unwrap();
            let expected: Vec<_> = history.keys.into_iter()
                .map(|(key, rows)| (key, rows.into_iter().filter_map(|row| (row.hour > 100).then_some(row.hour)).collect()))
                .collect();
            assert_eq!(proof.rows, expected);
        }
    }
}

impl Dates {
    fn through(&mut self, at: &ReadsAt<'_, LeadersContext>, frame: &DecodedFrame, read: TableRead, first: i32, last: i32) -> i32 {
        if self.span != Some((at.run, first, last)) {
            self.span = Some((at.run, first, last));
            self.tables.clear();
        }
        let changes = self.tables.entry((read.table, read.fields, read.ends))
            .or_insert_with(|| Changes::read(&at.context.replay, at.run, &read, first, last).ok());
        changes.as_ref().map_or(at.hour, |c| c.through(&read, frame, last))
    }
}

impl Visibility {
    #[cfg(test)]
    pub(super) fn proved_at(&self, handle: u64) -> Option<i32> {
        self.proofs.get(&handle).map(|proof| proof.first)
    }

    fn dated(&mut self, roles: Vec<(u64, u32, usize)>, frame: &DecodedFrame, at: &ReadsAt<'_, LeadersContext>) -> io::Result<Dated> {
        let replay = &at.context.replay;
        let block = replay.block_at(frame.branch, at.hour).ok_or_else(|| io::Error::other("Leader visibility hour is not recorded"))?;
        let (first, last) = (block.first_hour, block.last_hour);
        let mut tables = self.held.as_ref().map(|held| held.tables.clone()).unwrap_or_default();
        tables.extend(CORE);
        let rows = frame.complete::<tables::UnitLeaders>()?;
        let stale: Vec<_> = roles.iter().filter(|(key, owner, role)| self.proofs.get(key).is_none_or(|p|
            p.run != at.run || p.owner != *owner || p.role != *role || !(p.first..=p.through).contains(&at.hour)))
            .map(|(key, _, role)| rows.binary_search_by_key(key, handle)
                .map(|index| (&rows[index], *role)).map_err(|_| io::Error::other("Conditioned leader is not in the selected rows")))
            .collect::<io::Result<_>>()?;
        if !stale.is_empty() {
            let lexical = &frame.tables[TableId::LexicalTokens.index()];
            let literal = replay_data::rows::mask::<replay_data::rows::LexicalTokenRow>(&["literal"]);
            let reuse_lexical = matches!(lexical.keys(), Keys::All)
                && matches!(lexical.held(), TableView::Complete(_))
                && lexical.fields().is_none_or(|fields| fields & literal == literal);
            loop {
                let asked: Vec<_> = tables.iter().copied().filter(|&table| !reuse_lexical || table != TableId::LexicalTokens).collect();
                if self.cursor.as_ref().is_none_or(|(prior, _)| prior != &asked) {
                    self.cursor = Some((asked.clone(), HourCursor::new(replay.clone(), &asked, false)));
                }
                let SeekResult::Frame(mut recorded) = self.cursor.as_mut().expect("selected world").1.seek(frame.branch, at.hour)? else {
                    return Err(io::Error::other("Leader visibility hour is not recorded"));
                };
                if reuse_lexical { recorded.tables[TableId::LexicalTokens.index()] = lexical.clone(); }
                let world = world_of(&recorded, &at.context.content).map_err(io::Error::other)?;
                let gap = core_gap(&recorded);
                let mut bounds = HashMap::new();
                let mut bound = |read: TableRead| *bounds.entry(read).or_insert_with(|| self.dates.through(at, &recorded, read, first, last));
                let core = CORE.map(TableRead::keys).into_iter().map(&mut bound).min().unwrap_or(last);
                let mut made = Vec::new();
                for &(row, role) in &stale {
                    let mut through = core;
                    let decision = match &gap {
                        Some(reason) => Err(reason.clone()),
                        None => {
                            let (answer, inputs) = inputs::capture(&recorded, || Evaluator::new(&world, None)
                                .decide_leader_visibility(row, LeaderVisibility::Conditioned(role), true).decided());
                            if let Some(reads) = inputs.footprint() {
                                for &read in reads {
                                    if RECORDED_TABLES.contains(&read.table) { tables.insert(read.table); }
                                    through = through.min(bound(read));
                                    if let Some(owner) = read.owner {
                                        let table = match owner.by { inputs::Resolution::Links => TableId::CountryLinks, inputs::Resolution::Countries => TableId::Countries };
                                        through = through.min(bound(TableRead::fields(table, &["canonical_slot"])));
                                    }
                                }
                            } else {
                                tables.extend(RECORDED_TABLES.iter().copied());
                                through = at.hour;
                            }
                            answer
                        }
                    };
                    made.push((handle(row), Proof { run: at.run, owner: row.owner, role, first: at.hour, through, decision }));
                }
                if tables.iter().any(|table| !asked.contains(table) && (!reuse_lexical || *table != TableId::LexicalTokens)) { continue; }
                self.proofs.extend(made);
                break;
            }
        }
        let mut decisions = HashMap::new();
        let mut through = last;
        for (handle, _, _) in &roles {
            let proof = &self.proofs[handle];
            through = through.min(proof.through);
            decisions.insert(*handle, proof.decision.clone());
        }
        self.proofs.retain(|key, _| decisions.contains_key(key));
        Ok(Dated { run: at.run, first: at.hour, through, tables, roles, decisions })
    }
}

pub(super) fn prepare(params: &UnitLeadersParams, frame: &DecodedFrame, at: &ReadsAt<'_, LeadersContext>) -> (Vec<Read>, i32) {
    const INPUTS: [TableId; 5] = [TableId::UnitLeaders, TableId::CountryLinks, TableId::Countries, TableId::Characters, TableId::LexicalTokens];
    let mut visibility = at.context.visibility.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let same = at.since.is_some() && visibility.role_inputs.as_ref().is_some_and(|(views, end)| at.hour <= *end
        && INPUTS.iter().zip(views).all(|(id, prior)| prior.is(&frame.tables[id.index()])));
    let roles = if same {
        if let Some(held) = &visibility.held {
            if held.covers(at.run, at.hour) { return (Vec::new(), held.through.min(at.context.through(at.hour))); }
            held.roles.clone()
        } else { Vec::new() }
    } else {
        let mut world = None;
        let roles = UnitLeaders::kept(params, frame, at).into_iter().filter(|row| {
            needs_world(row) && !(row.kind == super::KIND_ARMY && (row.category as u32) < 2 && row.captured != 0)
        }).filter_map(|row| match world.get_or_insert_with(|| world_of(frame, &at.context.content)).as_ref().ok()?.decoded_leader_visibility(row) {
            LeaderVisibility::Conditioned(role) => Some((handle(row), row.owner, role)),
            _ => None,
        }).collect();
        visibility.role_inputs = Some((INPUTS.map(|id| frame.tables[id.index()].identity()), at.context.through(at.hour)));
        roles
    };
    if roles.is_empty() {
        if let Some(prior) = visibility.held.take() {
            visibility.changed = prior.decisions.into_keys().collect();
            visibility.proofs.clear();
            visibility.revision += 1;
        }
    } else if !visibility.held.as_ref().is_some_and(|held| held.covers(at.run, at.hour) && roles == held.roles) {
        match visibility.dated(roles, frame, at) {
            Ok(proof) => {
                let prior = visibility.held.as_ref().map(|v| &v.decisions);
                if prior != Some(&proof.decisions) {
                    visibility.changed = proof.decisions.keys().chain(prior.into_iter().flat_map(|p| p.keys()))
                        .filter(|key| proof.decisions.get(key) != prior.and_then(|p| p.get(key))).copied().collect();
                    visibility.revision += 1;
                }
                visibility.held = Some(proof);
            }
            Err(_) => {
                visibility.changed = visibility.proofs.keys().copied().collect();
                visibility.proofs.clear();
                visibility.revision += 1;
                visibility.held = None;
                visibility.role_inputs = None;
                return (RECORDED_TABLES.iter().copied().filter(|table| !super::BASE_TABLES.contains(table)).map(Read::all).collect(), at.hour);
            }
        }
    }
    let through = visibility.held.as_ref().map_or(i32::MAX, |held| held.through);
    (Vec::new(), through.min(at.context.through(at.hour)))
}
