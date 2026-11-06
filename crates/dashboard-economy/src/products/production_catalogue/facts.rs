//! The catalogue's facts, projected hour by hour from a columnar walk ([`replay::segments`]).
//! Each domain is read where what it reads may have changed, and handed on as what changed
//! since the segment last handed it on, whole at the segment's first hour and after a gap:
//!
//! - licences: every record, at an hour that wrote one or changed the identities;
//! - organisations: those whose roster row, state, country's policies or token spellings the
//!   hour wrote, every one where the identities or a table's status changed; an organisation is
//!   named again only where its roster row or a spelling it reads changed;
//! - designs: the definitions whose creator or obsolete flag the hour wrote, every one where
//!   the identities or the table's status changed;
//! - the variant names' evidence every hour, and at a complete read the operands of every
//!   definition whose naming inputs changed ([`Naming`]).
//!
//! With them go the tags of the holders party to a row read, where they changed, and the
//! refusals holding from the hour, where they changed. What came before the segment is the
//! ordered emit's ([`super::emit`]).

use super::{Field, LICENCES, ORGANISATIONS};
use dashboard_base::products::{
    equipment::{
        RawVariant, Tags, VariantHour,
        columnar::{Naming, slots},
    },
    production_lines::{self, RawOrganisation},
};
use dashboard_world::{Hold, Naming as Names, Needs, Roster, World};
use replay::{
    columns,
    segments::{self, Operator, Sink, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{
        CountryRow, IndustrialOrganisationDetailsRow, IndustrialOrganisationPoliciesRow,
        IndustrialOrganisationRow, LexicalTokenRow, ProductionLicenseRecordRow, TABLE_COUNT,
        TableId, VariantDefinitionRow, mask, tables,
    },
    scope::CaptureMode,
};
use replay_frame::frame::{ColumnFrame, Rowset, Tables};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    mem::size_of,
    sync::Arc,
};

/// The tables a projection reads.
pub const TABLES: &[TableId] = super::TABLES;

const LICENSEE_UNIDENTIFIED: &str = "a licence's licensee has no identity";
const LICENSOR_UNIDENTIFIED: &str = "a licence's licensor has no identity";
const ORGANISATION_COUNTRY: &str = "an organisation's country has no identity";
const POLICY_UNSPELLED: &str = "a selected policy's token is not spelled";
const DESIGN_CREATOR: &str = "a design's creator has no identity";

/// The frame's tables a domain reads as rows, whole.
fn asks(tables: &[TableId]) -> Vec<(TableId, Hold)> {
    tables.iter().map(|&table| (table, Hold::Whole)).collect()
}

/// What a projection reads of its walk's world: the countries, the token spellings and the
/// cosmetic tags variant names are read with, and the licence and organisation tables.
pub fn needs() -> Needs {
    let mut frame = asks(&LICENCES);
    frame.extend(asks(&ORGANISATIONS));
    Needs {
        countries: true,
        naming: true,
        frame,
        ..Needs::default()
    }
}

/// A value handed on: a number, or a text the emit numbers in the order it first sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub enum Val {
    Int(i128),
    Text(Arc<str>),
}

impl Val {
    fn heap(&self) -> usize {
        match self {
            Self::Int(_) => 0,
            Self::Text(text) => text.len(),
        }
    }
}

/// A domain's values handed on: every value it holds where `whole`, else what changed since
/// the segment last handed it on, none ending a value. In the order they were read.
#[derive(replay_spool::Spooled)]
pub struct Sent {
    pub domain: u8,
    pub whole: bool,
    pub values: Vec<(Field, Option<Val>)>,
}

/// What the ordered emit reads of an hour that changes something.
#[derive(Default)]
#[derive(replay_spool::Spooled)]
pub struct Facts {
    /// The tags of the holders party to a row read, where they changed in the segment.
    pub tags: Vec<(u32, Arc<str>)>,
    pub sent: Vec<Sent>,
    /// The organisations named, by id, ascending, each with its naming operands.
    pub names: Vec<(u64, RawOrganisation)>,
    /// The designs were read whole and complete, their identities known.
    pub designs: bool,
    /// The refusals holding from this hour, each with its count, where they changed.
    pub refusals: Option<Vec<(String, u64)>>,
}

/// One hour of a segment.
#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// The variant table's evidence at the hour.
    pub variants: VariantHour,
    /// At a complete read, the operands of each definition read anew, by id ascending.
    pub built: Vec<(u64, RawVariant)>,
    /// None when the hour changes nothing else.
    pub facts: Option<Box<Facts>>,
}

/// Why `table` is not read complete this hour, as the row decoder names it, without the hour.
fn unread(state: &State, table: TableId, mode: CaptureMode) -> String {
    match columns::status(state, table) {
        columns::Status::Complete => format!("{table:?} is complete"),
        columns::Status::Truncated { missing } => {
            format!("{table:?} is truncated ({missing} missing rows)")
        }
        columns::Status::Unavailable { reason } => {
            let reason = state
                .dictionary()
                .get(reason)
                .map_or_else(|| "unrecorded reason".into(), String::from_utf8_lossy);
            format!("{table:?} is unavailable: {reason}")
        }
        columns::Status::NotRecorded => {
            format!("{table:?} is not recorded in a {mode} recording")
        }
    }
}

/// Whether the hour inserted, deleted or wrote a row of `table`, or read it in part.
fn touched(state: &State, table: TableId) -> io::Result<bool> {
    let changes = columns::changes(state, table)?;
    Ok(changes.truncated
        || !changes.changed.is_empty()
        || !changes.fill_slots.is_empty()
        || !changes.inserted.is_empty()
        || !changes.deleted.is_empty())
}

/// The keys of `table`'s rows the hour wrote, inserted or deleted.
fn written(state: &State, table: TableId) -> io::Result<Vec<u128>> {
    let held = columns::table(state, table)?;
    let mut keys: Vec<u128> = slots::edits(state, table, u64::MAX)?
        .into_iter()
        .filter_map(|slot| held.key(slot))
        .collect();
    keys.extend_from_slice(&columns::changes(state, table)?.deleted_keys);
    Ok(keys)
}

/// The tag the countries name canonical slot `slot` by: its last country's by id, where it
/// spells one.
fn tag(roster: &Roster, slot: u32) -> Option<&str> {
    let country = roster.at_slot(slot).last()?;
    std::str::from_utf8(roster.tag(country.tag))
        .ok()
        .filter(|tag| !tag.is_empty())
}

/// An organisation as last handed on.
#[derive(Default)]
struct Organisation {
    values: Vec<(Field, Val)>,
    reasons: Vec<&'static str>,
    /// The country whose stored policy names it reads, and the tokens it spells.
    country: Option<u32>,
    tokens: Vec<u32>,
}

/// The tables an hour's organisations read.
struct Read<'f> {
    roster: Rowset<'f, IndustrialOrganisationRow>,
    details: Option<Rowset<'f, IndustrialOrganisationDetailsRow>>,
    policies: Option<Rowset<'f, IndustrialOrganisationPoliciesRow>>,
    tokens: bool,
}

/// The fields of a licence record the census reads.
const LICENCE_FIELDS: u64 = mask::<ProductionLicenseRecordRow>(&[
    "giver",
    "lended_cic",
    "required_cic",
    "start_date",
    "equipment",
]);

/// What [`Projector::save`] writes: nothing but that it restarts, since everything it holds is
/// read again from the walk's tables, as at a segment's first hour.
const SAVED: &[u8] = b"production catalogue projector: cold";

/// One segment's projection: every segment starts cold.
pub struct Projector {
    format: FormatVersion,
    mode: CaptureMode,
    /// VariantDefinitions' creator_link and obsolete.
    creator: usize,
    obsolete: usize,
    cold: bool,
    last: Option<i32>,
    shapes: [Option<columns::Status>; TABLE_COUNT],
    /// Why the countries' identities are not read; none while they are.
    unidentified: Option<String>,
    /// Each domain's table refusals at its last read.
    tables: [Vec<String>; 3],
    /// Each licensee holder and position's values as last handed on.
    licences: HashMap<(u32, u32), Vec<(Field, Val)>>,
    /// Each refusing record's reason, by record key, and how many records refuse for each.
    licence_records: HashMap<u128, &'static str>,
    licence_rows: BTreeMap<&'static str, u64>,
    /// Whether the licences were read complete, and whether their roster is known.
    licences_read: bool,
    licences_known: bool,
    organisations: BTreeMap<u64, Organisation>,
    /// Each organisation id by the country whose policies it reads, and by the tokens it
    /// spells.
    by_country: HashMap<u32, BTreeSet<u64>>,
    by_token: HashMap<u32, BTreeSet<u64>>,
    organisation_rows: BTreeMap<&'static str, u64>,
    organisations_known: bool,
    /// Each design's creator holder and obsolete flag; none where its creator has no identity.
    designs: HashMap<u64, Option<(u32, bool)>>,
    unknown_creators: u64,
    designs_known: bool,
    sent: Option<BTreeMap<String, u64>>,
    told: HashMap<u32, Arc<str>>,
    naming: Naming,
}

impl Projector {
    pub fn new(format: FormatVersion, mode: CaptureMode) -> io::Result<Self> {
        let table = TableId::VariantDefinitions;
        Ok(Self {
            format,
            mode,
            creator: columns::field(table, "creator_link")?,
            obsolete: columns::field(table, "obsolete")?,
            cold: true,
            last: None,
            shapes: [None; TABLE_COUNT],
            unidentified: None,
            tables: Default::default(),
            licences: HashMap::new(),
            licence_records: HashMap::new(),
            licence_rows: BTreeMap::new(),
            licences_read: false,
            licences_known: false,
            organisations: BTreeMap::new(),
            by_country: HashMap::new(),
            by_token: HashMap::new(),
            organisation_rows: BTreeMap::new(),
            organisations_known: false,
            designs: HashMap::new(),
            unknown_creators: 0,
            designs_known: false,
            sent: None,
            told: HashMap::new(),
            naming: Naming::new()?,
        })
    }

    /// Forgets what the segment handed on: the next reads are whole.
    fn reset(&mut self) {
        self.unidentified = None;
        self.tables = Default::default();
        self.licences.clear();
        self.licence_records.clear();
        self.licence_rows.clear();
        self.licences_read = false;
        self.licences_known = false;
        self.organisations.clear();
        self.by_country.clear();
        self.by_token.clear();
        self.organisation_rows.clear();
        self.organisations_known = false;
        self.designs.clear();
        self.unknown_creators = 0;
        self.designs_known = false;
        self.sent = None;
        self.told.clear();
    }

    /// The tags of `parties` the segment has not handed on as they are now.
    fn tell(&mut self, roster: &Roster, parties: BTreeSet<u32>, out: &mut Vec<(u32, Arc<str>)>) {
        for slot in parties {
            let Some(tag) = tag(roster, slot) else {
                continue;
            };
            if self.told.get(&slot).is_none_or(|held| **held != *tag) {
                let tag: Arc<str> = Arc::from(tag);
                self.told.insert(slot, Arc::clone(&tag));
                out.push((slot, tag));
            }
        }
    }

    /// The licence records the hour wrote, or every one where `whole`, as the row census reads
    /// them: the records of one licensee holder and position share its fields, a later
    /// record's overwriting an earlier's, so each position written is read again from every
    /// raw id of its holder.
    fn licences(
        &mut self,
        frame: &ColumnFrame<'_>,
        state: &State,
        roster: &Roster,
        whole: bool,
        parties: &mut BTreeSet<u32>,
    ) -> io::Result<Sent> {
        self.tables[0].clear();
        let mut values = Vec::new();
        let table = LICENCES[0];
        let rows = columns::complete(state, table)
            .then(|| {
                frame
                    .complete_rows::<tables::ProductionLicenseRecords>(LICENCE_FIELDS)
                    .ok()
            })
            .flatten();
        let Some(rows) = rows else {
            self.tables[0].push(format!("licences unavailable: {}", unread(state, table, self.mode)));
            if !whole {
                for (_, fields) in self.licences.drain() {
                    values.extend(fields.into_iter().map(|(field, _)| (field, None)));
                }
                if self.licences_known {
                    values.push((Field::Known(0), None));
                }
            }
            self.licences.clear();
            self.licence_records.clear();
            self.licence_rows.clear();
            self.licences_read = false;
            self.licences_known = false;
            return Ok(Sent {
                domain: 0,
                whole,
                values,
            });
        };
        // A record's key is its licensee's raw id and its ordinal.
        let key = |owner: u32, at: u32| u128::from(owner) << 32 | u128::from(at);
        let whole = whole || !self.licences_read;
        let records: Vec<u128> = if whole {
            self.licences.clear();
            self.licence_records.clear();
            self.licence_rows.clear();
            rows.keys().collect()
        } else {
            written(state, table)?
        };
        let mut positions = BTreeSet::new();
        for &at in &records {
            let row = rows.get(at)?;
            let reason = match row {
                None => None,
                Some(row) => match roster.slot_of(row.owner) {
                    None => Some(LICENSEE_UNIDENTIFIED),
                    Some(_) => roster.slot_of(row.giver).is_none().then_some(LICENSOR_UNIDENTIFIED),
                },
            };
            let held = match reason {
                Some(reason) => self.licence_records.insert(at, reason),
                None => self.licence_records.remove(&at),
            };
            if let Some(held) = held
                && let Some(count) = self.licence_rows.get_mut(held)
            {
                *count -= 1;
                if *count == 0 {
                    self.licence_rows.remove(held);
                }
            }
            if let Some(reason) = reason {
                *self.licence_rows.entry(reason).or_default() += 1;
            }
            // The position the record holds, or held: its fields are its holder's records'.
            if let Some(slot) = roster.slot_of((at >> 32) as u32) {
                positions.insert((slot, at as u32));
            }
        }
        for (owner, at) in positions {
            let mut fields: BTreeMap<Field, i128> = BTreeMap::new();
            for &raw in roster.aliases(owner) {
                let Some(row) = rows.get(key(raw, at))? else {
                    continue;
                };
                parties.insert(owner);
                let licensor = match roster.slot_of(row.giver) {
                    Some(giver) => {
                        parties.insert(giver);
                        (Field::LicenceGiver(owner, at), i128::from(giver))
                    }
                    None => (Field::LicenceUnidentified(owner, at), 1),
                };
                for (field, value) in [
                    licensor,
                    (Field::LicenceLent(owner, at), i128::from(row.lended_cic)),
                    (Field::LicenceRequired(owner, at), i128::from(row.required_cic)),
                    (Field::LicenceStart(owner, at), i128::from(row.start_date)),
                ] {
                    fields.insert(field, value);
                }
                for (index, &variant) in row.equipment.iter().enumerate() {
                    fields.insert(
                        Field::LicenceVariant(owner, at, index as u32),
                        i128::from(variant),
                    );
                }
            }
            let now: Vec<(Field, Val)> = fields
                .into_iter()
                .map(|(field, value)| (field, Val::Int(value)))
                .collect();
            let held = self.licences.remove(&(owner, at)).unwrap_or_default();
            for (field, value) in &now {
                if whole || !held.iter().any(|(was, held)| was == field && held == value) {
                    values.push((*field, Some(value.clone())));
                }
            }
            for (field, _) in &held {
                if !now.iter().any(|(kept, _)| kept == field) {
                    values.push((*field, None));
                }
            }
            if !now.is_empty() {
                self.licences.insert((owner, at), now);
            }
        }
        // A licence whose licensee has no identity is left out: the roster is not known whole.
        let known = !self.licence_rows.contains_key(LICENSEE_UNIDENTIFIED);
        if whole || known != self.licences_known {
            values.push((Field::Known(0), known.then_some(Val::Int(1))));
        }
        self.licences_read = true;
        self.licences_known = known;
        Ok(Sent {
            domain: 0,
            whole,
            values,
        })
    }

    /// The organisations the hour may have changed, or every one where `whole`; none where
    /// their roster is not read.
    #[allow(clippy::too_many_arguments)]
    fn organisations(
        &mut self,
        frame: &ColumnFrame<'_>,
        state: &State,
        (roster, names): (&Roster, &Names),
        at: i32,
        whole: bool,
        parties: &mut BTreeSet<u32>,
        named: &mut Vec<(u64, RawOrganisation)>,
    ) -> io::Result<Sent> {
        self.tables[1].clear();
        let mut values = Vec::new();
        let complete = |table| columns::complete(state, table);
        let all = |table: TableId| table.full_mask();
        let list = complete(ORGANISATIONS[0])
            .then(|| {
                frame
                    .complete_rows::<tables::IndustrialOrganisations>(all(ORGANISATIONS[0]))
                    .ok()
            })
            .flatten();
        let Some(list) = list else {
            self.tables[1].push(format!(
                "organisations unavailable: {}",
                unread(state, ORGANISATIONS[0], self.mode)
            ));
            let held = std::mem::take(&mut self.organisations);
            self.by_country.clear();
            self.by_token.clear();
            self.organisation_rows.clear();
            self.organisations_known = false;
            if !whole {
                for (_, organisation) in held {
                    values.extend(organisation.values.into_iter().map(|(field, _)| (field, None)));
                }
                values.push((Field::Known(1), None));
            }
            return Ok(Sent {
                domain: 1,
                whole,
                values,
            });
        };
        let tokens = names.tokens_complete();
        if !tokens {
            self.tables[1].push(format!(
                "organisation names unavailable: {}",
                unread(state, TableId::LexicalTokens, self.mode)
            ));
        }
        let details = complete(ORGANISATIONS[1])
            .then(|| {
                frame
                    .complete_rows::<tables::IndustrialOrganisationDetails>(all(ORGANISATIONS[1]))
                    .ok()
            })
            .flatten();
        if details.is_none() {
            self.tables[1].push(format!(
                "organisation state unavailable: {}",
                unread(state, ORGANISATIONS[1], self.mode)
            ));
        }
        let policies = complete(ORGANISATIONS[2])
            .then(|| {
                frame
                    .complete_rows::<tables::IndustrialOrganisationPolicies>(all(ORGANISATIONS[2]))
                    .ok()
            })
            .flatten();
        if policies.is_none() {
            self.tables[1].push(format!(
                "organisation policies unavailable: {}",
                unread(state, ORGANISATIONS[2], self.mode)
            ));
        }
        let read = Read {
            roster: list,
            details,
            policies,
            tokens,
        };
        // Each organisation read, and whether its naming may have changed.
        let mut changed: BTreeMap<u64, bool> = BTreeMap::new();
        let whole = whole || !self.organisations_known;
        if whole {
            self.organisations.clear();
            self.by_country.clear();
            self.by_token.clear();
            self.organisation_rows.clear();
            changed.extend(list.keys().map(|key| (key as u64, true)));
        } else {
            for key in written(state, ORGANISATIONS[0])? {
                changed.insert(key as u64, true);
            }
            for token in names.changed_tokens() {
                for &id in self.by_token.get(token).into_iter().flatten() {
                    changed.insert(id, true);
                }
            }
            for key in written(state, ORGANISATIONS[1])? {
                changed.entry(key as u64).or_insert(false);
            }
            for key in written(state, ORGANISATIONS[2])? {
                for &id in self.by_country.get(&(key as u32)).into_iter().flatten() {
                    changed.entry(id).or_insert(false);
                }
            }
        }
        if whole {
            values.push((Field::Known(1), Some(Val::Int(1))));
        }
        self.organisations_known = true;
        for (id, renamed) in changed {
            self.organisation(state, (roster, names), &read, id, renamed, at, whole, parties, named, &mut values)?;
        }
        Ok(Sent {
            domain: 1,
            whole,
            values,
        })
    }

    /// One organisation read again, its naming too where `renamed`, its changes into
    /// `values`.
    #[allow(clippy::too_many_arguments)]
    fn organisation(
        &mut self,
        state: &State,
        (roster, names): (&Roster, &Names),
        read: &Read<'_>,
        id: u64,
        renamed: bool,
        at: i32,
        whole: bool,
        parties: &mut BTreeSet<u32>,
        named: &mut Vec<(u64, RawOrganisation)>,
        values: &mut Vec<(Field, Option<Val>)>,
    ) -> io::Result<()> {
        let row = read.roster.get(u128::from(id))?;
        let mut now = Organisation::default();
        if let Some(row) = row {
            match roster.slot_of(row.country) {
                None => now.reasons.push(ORGANISATION_COUNTRY),
                Some(holder) => {
                    parties.insert(holder);
                    read_organisation(state, (roster, names), read, &row, holder, renamed, at, named, &mut now)?;
                }
            }
        }
        let held = self.organisations.remove(&id).unwrap_or_default();
        for reason in &held.reasons {
            if let Some(count) = self.organisation_rows.get_mut(reason) {
                *count -= 1;
                if *count == 0 {
                    self.organisation_rows.remove(reason);
                }
            }
        }
        for &reason in &now.reasons {
            *self.organisation_rows.entry(reason).or_default() += 1;
        }
        if let Some(country) = held.country
            && let Some(ids) = self.by_country.get_mut(&country)
        {
            ids.remove(&id);
            if ids.is_empty() {
                self.by_country.remove(&country);
            }
        }
        for token in &held.tokens {
            if let Some(ids) = self.by_token.get_mut(token) {
                ids.remove(&id);
                if ids.is_empty() {
                    self.by_token.remove(token);
                }
            }
        }
        if let Some(country) = now.country {
            self.by_country.entry(country).or_default().insert(id);
        }
        for &token in &now.tokens {
            self.by_token.entry(token).or_default().insert(id);
        }
        for (field, value) in &now.values {
            let same = !whole
                && held
                    .values
                    .iter()
                    .any(|(was, held)| was == field && held == value);
            if !same {
                values.push((*field, Some(value.clone())));
            }
        }
        if !whole {
            for (field, _) in &held.values {
                if !now.values.iter().any(|(kept, _)| kept == field) {
                    values.push((*field, None));
                }
            }
        }
        if !now.values.is_empty() || !now.reasons.is_empty() || !now.tokens.is_empty() {
            self.organisations.insert(id, now);
        }
        Ok(())
    }
}

/// An organisation of its identified `holder`, as the row census reads it: its values, the
/// spellings and policies it reads, and its naming operands where `renamed`.
#[allow(clippy::too_many_arguments)]
fn read_organisation(
    state: &State,
    (roster, names): (&Roster, &Names),
    read: &Read<'_>,
    row: &IndustrialOrganisationRow,
    holder: u32,
    renamed: bool,
    at: i32,
    named: &mut Vec<(u64, RawOrganisation)>,
    now: &mut Organisation,
) -> io::Result<()> {
    let id = row.organisation;
    now.values.push((Field::MioHolder(id), Val::Int(i128::from(holder))));
    let detail = match read.details {
        Some(details) => details
            .get(u128::from(id))?
            .filter(|detail| detail.country == row.country),
        None => None,
    };
    let detail = detail.as_deref();
    // The spellings it reads, as the row census's dictionary holds them.
    let mut wanted = vec![row.token];
    wanted.extend(detail.map(|detail| detail.state.policy));
    wanted.sort_unstable();
    wanted.dedup();
    let mut lexical = Vec::with_capacity(wanted.len());
    if read.tokens {
        for &token in &wanted {
            if let Some(literal) = names.spelling(state, token)? {
                lexical.push(LexicalTokenRow {
                    token,
                    literal: literal.into_owned(),
                });
            }
        }
    }
    if renamed {
        // Without the dictionary the names are unknown from this read on.
        let raw = if read.tokens {
            let rows: Vec<CountryRow> = roster
                .slot_of(row.owner)
                .into_iter()
                .flat_map(|slot| roster.at_slot(slot))
                .map(|country| CountryRow {
                    id: country.id,
                    canonical_slot: country.slot,
                    tag: roster.tag(country.tag).to_vec(),
                    ..Default::default()
                })
                .collect();
            let tags = Tags::new(&rows, roster.links(), None);
            production_lines::raw_organisation(row, at, &tags, &lexical)
        } else {
            RawOrganisation {
                first_hour: at,
                owner_unknown: true,
                ..Default::default()
            }
        };
        named.push((id, raw));
    }
    let Some(detail) = detail else {
        now.tokens = vec![row.token];
        return Ok(());
    };
    // Without the dictionary a selected policy reads as unspelled.
    let read_state = production_lines::organisation_state(detail, None, &lexical);
    now.values.extend([
        (Field::MioSize(id), Val::Int(i128::from(detail.state.size))),
        (Field::MioFunds(id), Val::Int(i128::from(detail.state.funds_raw))),
        (Field::MioPoints(id), Val::Int(i128::from(detail.state.points))),
        (Field::MioUnlocked(id), Val::Int(detail.unlocked.len() as i128)),
    ]);
    if let Some(cooldown) = detail.state.cooldown_hour {
        now.values
            .push((Field::MioCooldown(id), Val::Int(i128::from(cooldown))));
    }
    if read_state.policy_token.as_deref() == Some("") {
        now.reasons.push(POLICY_UNSPELLED);
        now.values.push((Field::MioPolicyUnresolved(id), Val::Int(1)));
    }
    if let Some(token) = read_state
        .policy_token
        .as_deref()
        .filter(|token| !token.is_empty())
    {
        now.values.push((Field::MioPolicy(id), Val::Text(Arc::from(token))));
    }
    // The country's stored policy name joins by the raw token, spelled or not.
    if read_state.policy_token.is_some() {
        let policies = match read.policies {
            Some(rows) => rows.get(u128::from(detail.country))?,
            None => None,
        };
        let name = policies.as_deref().and_then(|row| {
            row.policies
                .iter()
                .find(|policy| policy.token == detail.state.policy)
                .map(|policy| String::from_utf8_lossy(&policy.name))
        });
        if let Some(name) = name.filter(|name| !name.is_empty()) {
            now.values
                .push((Field::MioPolicyName(id), Val::Text(Arc::from(&*name))));
        }
        now.country = Some(detail.country);
    }
    now.tokens = vec![row.token, detail.state.policy];
    Ok(())
}

impl Projector {
    /// The id, raw creator link and obsolete flag of the definition at `slot`.
    fn definition(&self, state: &State, slot: u32) -> io::Result<(u64, u32, bool)> {
        let held = columns::table(state, TableId::VariantDefinitions)?;
        let key = held.key(slot).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "A live definition holds no row")
        })?;
        if held.recorded(self.creator, slot) && held.recorded(self.obsolete, slot) {
            let at = slot as usize;
            return Ok((
                key as u64,
                columns::plain::<u32>(held, self.creator)?[at],
                columns::plain::<u8>(held, self.obsolete)?[at] != 0,
            ));
        }
        let row: VariantDefinitionRow =
            columns::row(state, TableId::VariantDefinitions, slot, self.format.version())?;
        Ok((key as u64, row.creator_link, row.obsolete != 0))
    }

    /// The designs the hour wrote, or every one where `whole`; none where their table is not
    /// read complete.
    fn designs(
        &mut self,
        state: &State,
        roster: &Roster,
        whole: bool,
        parties: &mut BTreeSet<u32>,
    ) -> io::Result<(Sent, bool)> {
        let table = TableId::VariantDefinitions;
        self.tables[2].clear();
        let mut values = Vec::new();
        if !columns::complete(state, table) {
            self.tables[2].push(format!("designs unavailable: {}", unread(state, table, self.mode)));
            let held = std::mem::take(&mut self.designs);
            self.unknown_creators = 0;
            if !whole {
                for (id, design) in held {
                    if design.is_some() {
                        values.push((Field::DesignCreator(id), None));
                        values.push((Field::DesignObsolete(id), None));
                    }
                }
                if self.designs_known {
                    values.push((Field::Known(2), None));
                }
            }
            self.designs_known = false;
            return Ok((
                Sent {
                    domain: 2,
                    whole,
                    values,
                },
                false,
            ));
        }
        let held = columns::table(state, table)?;
        let whole = whole || !self.designs_known;
        let mut read = Vec::new();
        if whole {
            self.designs.clear();
            self.unknown_creators = 0;
            values.push((Field::Known(2), Some(Val::Int(1))));
            for slot in held.live().ones() {
                read.push(self.definition(state, slot)?);
            }
        } else {
            let mask = 1_u64 << self.creator | 1_u64 << self.obsolete;
            for slot in slots::edits(state, table, mask)? {
                read.push(self.definition(state, slot)?);
            }
            for &key in &columns::changes(state, table)?.deleted_keys {
                if held.slot(key).is_none()
                    && let Some(design) = self.designs.remove(&(key as u64))
                {
                    self.forget(key as u64, design, &mut values);
                }
            }
        }
        self.designs_known = true;
        for (id, link, obsolete) in read {
            let now = roster.slot_of(link).map(|creator| (creator, obsolete));
            let was = self.designs.insert(id, now);
            if let Some((creator, _)) = now {
                parties.insert(creator);
            }
            if !whole && was == Some(now) {
                continue;
            }
            if let Some(was) = was {
                self.unknown_creators -= u64::from(was.is_none());
            }
            self.unknown_creators += u64::from(now.is_none());
            match now {
                Some((creator, obsolete)) => {
                    values.push((Field::DesignCreator(id), Some(Val::Int(i128::from(creator)))));
                    values.push((Field::DesignObsolete(id), Some(Val::Int(i128::from(obsolete)))));
                }
                None if !whole && was.flatten().is_some() => {
                    values.push((Field::DesignCreator(id), None));
                    values.push((Field::DesignObsolete(id), None));
                }
                None => {}
            }
        }
        // The first complete read since the designs were last unknown is a whole one.
        Ok((
            Sent {
                domain: 2,
                whole,
                values,
            },
            whole,
        ))
    }

    /// A design gone from the table: its values end, and it no longer refuses.
    fn forget(&mut self, id: u64, design: Option<(u32, bool)>, values: &mut Vec<(Field, Option<Val>)>) {
        match design {
            Some(_) => {
                values.push((Field::DesignCreator(id), None));
                values.push((Field::DesignObsolete(id), None));
            }
            None => self.unknown_creators -= 1,
        }
    }

    /// The refusals holding now, where they differ from the last handed on.
    fn refused(&mut self, whole: bool) -> Option<Vec<(String, u64)>> {
        let mut now = BTreeMap::<String, u64>::new();
        match &self.unidentified {
            Some(reason) => {
                now.insert(reason.clone(), 1);
            }
            None => {
                for reason in self.tables.iter().flatten() {
                    *now.entry(reason.clone()).or_default() += 1;
                }
                for (&reason, &count) in self.licence_rows.iter().chain(&self.organisation_rows) {
                    *now.entry(reason.to_owned()).or_default() += count;
                }
                if self.unknown_creators > 0 {
                    *now.entry(DESIGN_CREATOR.to_owned()).or_default() += self.unknown_creators;
                }
            }
        }
        if !whole && self.sent.as_ref() == Some(&now) {
            return None;
        }
        let refusals = now
            .iter()
            .map(|(reason, &count)| (reason.clone(), count))
            .collect();
        self.sent = Some(now);
        Some(refusals)
    }

    /// Every domain ends: the countries' identities are not read.
    fn unidentified(&mut self, reason: String, whole: bool) -> Vec<Sent> {
        let mut sent = Vec::new();
        for domain in 0..3_u8 {
            let mut values = Vec::new();
            if !whole {
                match domain {
                    0 => {
                        for (_, fields) in self.licences.drain() {
                            values.extend(fields.into_iter().map(|(field, _)| (field, None)));
                        }
                        if self.licences_known {
                            values.push((Field::Known(0), None));
                        }
                    }
                    1 => {
                        for (_, organisation) in std::mem::take(&mut self.organisations) {
                            values.extend(
                                organisation.values.into_iter().map(|(field, _)| (field, None)),
                            );
                        }
                        if self.organisations_known {
                            values.push((Field::Known(1), None));
                        }
                    }
                    _ => {
                        for (id, design) in std::mem::take(&mut self.designs) {
                            if design.is_some() {
                                values.push((Field::DesignCreator(id), None));
                                values.push((Field::DesignObsolete(id), None));
                            }
                        }
                        if self.designs_known {
                            values.push((Field::Known(2), None));
                        }
                    }
                }
            }
            sent.push(Sent {
                domain,
                whole,
                values,
            });
        }
        let told = std::mem::take(&mut self.told);
        let held_sent = self.sent.take();
        self.reset();
        self.told = told;
        self.sent = held_sent;
        self.unidentified = Some(reason);
        sent
    }

    fn heap(&self) -> usize {
        let value = size_of::<(Field, Val)>() + 1;
        self.licences
            .values()
            .map(|fields| {
                size_of::<((u32, u32), Vec<(Field, Val)>)>()
                    + fields.capacity() * value
                    + fields.iter().map(|(_, value)| value.heap()).sum::<usize>()
            })
            .sum::<usize>()
            + self.licence_records.capacity() * (size_of::<(u128, &str)>() + 1)
            + self
                .organisations
                .values()
                .map(|organisation| {
                    size_of::<(u64, Organisation)>()
                        + organisation.values.capacity() * size_of::<(Field, Val)>()
                        + organisation
                            .values
                            .iter()
                            .map(|(_, value)| value.heap())
                            .sum::<usize>()
                        + organisation.reasons.capacity() * size_of::<&str>()
                        + organisation.tokens.capacity() * 4
                })
                .sum::<usize>()
            + (self.by_country.len() + self.by_token.len()) * 64
            + self.designs.capacity() * (size_of::<(u64, Option<(u32, bool)>)>() + 1)
            + self.told.capacity() * (size_of::<(u32, Arc<str>)>() + 1)
            + self.told.values().map(|tag| tag.len()).sum::<usize>()
            + self.sent.as_ref().map_or(0, |sent| {
                sent.keys().map(|reason| reason.capacity() + 64).sum::<usize>()
            })
            + self
                .tables
                .iter()
                .flatten()
                .map(|reason| reason.capacity() + size_of::<String>())
                .sum::<usize>()
    }
}

// Every segment starts cold: its first hour reads every domain whole; what came before is the
// emit's.
impl Operator<World> for Projector {
    type Output = Hour;

    fn start(&mut self, _: segments::Start<'_>, _: u64, _: i32) -> io::Result<()> {
        self.cold = true;
        Ok(())
    }

    fn hour(&mut self, hour: &segments::Hour<'_, World>, out: &mut dyn Sink<Hour>) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("production_catalogue.columnar", Some("production_catalogue"));
        let state = hour.state;
        let at = hour.hour;
        let world = hour.world;
        let whole = self.cold
            || hour.starts
            || hour.opens
            || self.last.and_then(|last| last.checked_add(1)) != Some(at);
        self.cold = false;
        self.last = Some(at);
        if whole {
            self.reset();
        }
        let mut shaped = [false; TABLE_COUNT];
        let mut changed = [false; TABLE_COUNT];
        for &table in TABLES {
            let status = columns::status(state, table);
            let index = table.index();
            shaped[index] = whole || self.shapes[index] != Some(status);
            changed[index] = shaped[index] || touched(state, table)?;
            self.shapes[index] = Some(status);
        }
        let roster = world.countries()?;
        let names = world.naming()?;
        let read = dashboard_text::metadata::slot_read(
            self.format,
            world.metrics(),
            TableId::VariantDefinitions,
            at,
        );
        let variants = self.naming.read(
            state,
            (roster, names),
            self.format,
            at,
            whole,
            &shaped,
            &changed,
            read,
        )?;
        let built = if variants.evidence == VariantHour::Read {
            variants.built
        } else {
            Vec::new()
        };
        let identity = whole || roster.changed();
        let complete = roster.countries_complete() && roster.links_complete();
        let licences = identity || changed[LICENCES[0].index()];
        let organisations = identity
            || ORGANISATIONS.iter().any(|table| changed[table.index()])
            || names.tokens_rebuilt()
            || names
                .changed_tokens()
                .iter()
                .any(|token| self.by_token.contains_key(token));
        let designs = identity || changed[TableId::VariantDefinitions.index()];
        let mut facts = Facts::default();
        if identity && !complete {
            let table = if roster.countries_complete() {
                TableId::CountryLinks
            } else {
                TableId::Countries
            };
            let reason = format!(
                "country identities unavailable: {}",
                unread(state, table, self.mode)
            );
            facts.sent = self.unidentified(reason, whole);
        } else if !complete {
            // Still unidentified: every domain stays ended.
        } else {
            let whole_domains = whole || self.unidentified.take().is_some();
            let mut parties = BTreeSet::new();
            if licences || organisations || whole_domains {
                let frame = world.columns(state);
                if licences || whole_domains {
                    let whole = whole_domains || identity || shaped[LICENCES[0].index()];
                    let sent = self.licences(&frame, state, roster, whole, &mut parties)?;
                    facts.sent.push(sent);
                }
                if organisations || whole_domains {
                    let whole = whole_domains
                        || identity
                        || names.tokens_rebuilt()
                        || ORGANISATIONS.iter().any(|table| shaped[table.index()]);
                    let sent = self.organisations(
                        &frame,
                        state,
                        (roster, names),
                        at,
                        whole,
                        &mut parties,
                        &mut facts.names,
                    )?;
                    facts.sent.push(sent);
                }
            }
            if designs || whole_domains {
                let table = TableId::VariantDefinitions.index();
                let (sent, read) =
                    self.designs(state, roster, whole_domains || identity || shaped[table], &mut parties)?;
                facts.designs = read;
                facts.sent.push(sent);
            }
            self.tell(roster, parties, &mut facts.tags);
        }
        // Refusals move only where the identities or a domain were read.
        if identity || licences || organisations || designs {
            facts.refusals = self.refused(whole);
        }
        facts.sent.retain(|sent| sent.whole || !sent.values.is_empty());
        let quiet = facts.tags.is_empty()
            && facts.sent.is_empty()
            && facts.names.is_empty()
            && !facts.designs
            && facts.refusals.is_none();
        out.push(Hour {
            branch: hour.branch,
            hour: at,
            variants: variants.evidence,
            built,
            facts: (!quiet).then(|| Box::new(facts)),
        })
    }

    fn carried(&self) -> Option<usize> {
        Some(self.heap())
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool> {
        out.extend_from_slice(SAVED);
        Ok(true)
    }

    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes != SAVED {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not a saved production catalogue projector",
            ));
        }
        *self = Self::new(self.format, self.mode)?;
        Ok(())
    }
}
