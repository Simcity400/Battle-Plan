//! Names: `OWNER|<regionId>` is a nonzero signed fixed5 mine amount;
//! `OWNER|extent` is the owner's complete vector extent; `OWNER|refused` marks
//! an owner whose mine vector was read but not admitted at that hour, so the
//! inventory is incomplete rather than empty; `?|refused` counts owners the
//! hour holds vectors for but cannot name. OWNER is TAG@slot.
//! No amount below a witnessed extent proves zero; no extent proves nothing.
//! Standing reports retain occurrences and never infer strike dates.
use dashboard_base::products::built::Built;
use crate::shared::{self, Choice};
use dashboard_base::{
    catalogue::{self, Refusals},
    prepared::{prepass::HeldBytes, query::native},
    products::{Product, ReplyWarnings},
};
#[cfg(test)]
use dashboard_base::prepared::prepass::Family;
use dashboard_keyed::{
    Delta, Observer, Values,
    delta::same_rows,
    countries::Countries,
    query::{KeyedWindow, Opened, Query},
};
use replay::campaign::TableFields;
use replay_data::{
    format::FormatVersion,
    rows::{NavalAccidentReportRow, NavalRegionalMineRow, TableId, tables},
};
use replay_derive::naval_mine_levels::Levels;
use replay_frame::frame::{DecodedFrame, SharedView};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};
use ts_rs::TS;

mod served;

pub struct Minefields;
#[derive(Clone, Debug, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(rename = "MinefieldsView")]
pub enum View {
    #[default]
    History,
    Reports,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename = "MinefieldsParams")]
pub struct Params {
    pub branch_id: String,
    #[ts(optional)]
    pub tags: Option<Vec<String>>,
    #[ts(optional)]
    pub first_hour: Option<i32>,
    #[ts(optional)]
    pub last_hour: Option<i32>,
    #[serde(default)]
    pub view: View,
    #[ts(optional)]
    pub region: Option<u32>,
    #[ts(optional)]
    pub hour: Option<i32>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "MinefieldsHistoryReply")]
pub struct HistoryReply {
    pub window: KeyedWindow,
    pub regions: Vec<Choice>,
    pub scale: i32,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "MinefieldReport")]
pub struct Report {
    pub kind: u8,
    pub ordinal: u32,
    pub victim: Option<String>,
    pub mine_owner: Option<String>,
    pub ship: Option<String>,
    pub equipment: Option<String>,
    pub damage: String,
    pub sunk: bool,
    pub first_seen: Option<i32>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "MinefieldsReportsReply")]
pub struct ReportsReply {
    pub hour: i32,
    pub read_hour: Option<i32>,
    pub region: u32,
    pub reports: Vec<Report>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(untagged)]
#[ts(rename = "MinefieldsReply")]
pub enum Reply {
    History(HistoryReply),
    Reports(ReportsReply),
}
impl ReplyWarnings for Reply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        match self {
            Self::History(r) => &mut r.warnings,
            Self::Reports(r) => &mut r.warnings,
        }
    }
}
/// What the reports view reads at its hour.
const REPORT_TABLES: &[TableId] = &[
    TableId::NavalAccidentReports,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::VariantDefinitions,
    TableId::LexicalTokens,
    TableId::CountryCounterInputs,
    TableId::ReservedCountryNaming,
];
impl Product for Minefields {
    const KEY: &'static str = "minefields";
    const CONTENT_DEPENDENT: bool = true;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
    type Params = Params;
    type Reply = Reply;
    fn validate(p: &Params) -> Result<(), &'static str> {
        shared::validate(&p.branch_id, p.first_hour, p.last_hour)?;
        for tag in p.tags.iter().flatten() {
            shared::tag(tag)?;
        }
        if matches!(p.view, View::Reports) && (p.hour.is_none() || p.region.is_none()) {
            return Err("Reports require a region and hour");
        }
        Ok(())
    }
    fn types() -> Vec<String> {
        let mut t = vec![
            Params::decl(),
            Reply::decl(),
            View::decl(),
            HistoryReply::decl(),
            ReportsReply::decl(),
            Report::decl(),
        ];
        t.extend(shared::types());
        t
    }
}
#[derive(Default, Serialize, Deserialize)]
pub struct MinefieldsObserver {
    pub refusals: BTreeMap<String, u64>,
    /// The last known observation, which the next hour's changes are found against; never published.
    #[serde(skip)]
    last: Option<Box<Last>>,
}

/// The last known observation: its inputs and each owner's part of it.
struct Last {
    mines: SharedView,
    links: SharedView,
    countries: SharedView,
    identities: Countries,
    /// Each raw reference's canonical slot.
    slots: BTreeMap<u32, u32>,
    /// The raw references linked to each canonical slot, ascending.
    raws: BTreeMap<u32, Vec<u32>>,
    /// Each slot holding vectors, with what they read.
    owners: BTreeMap<u32, Owner>,
    /// Rows of each raw reference without a canonical slot.
    unlinked: BTreeMap<u32, u64>,
    /// The refusals each observation of these inputs notes.
    notes: BTreeMap<String, u64>,
}

/// One canonical slot's vectors as an observation reads them.
struct Owner {
    values: Vec<(String, i128)>,
    /// What it notes, if refused.
    note: Option<String>,
    /// Whether it counts among the vectors the hour cannot name.
    unnamed: bool,
}

impl Last {
    fn read(frame: &DecodedFrame) -> Option<Self> {
        let identities = Countries::read(frame)?;
        let rows = frame.complete::<tables::NavalRegionalMines>().ok()?;
        let links = frame.complete::<tables::CountryLinks>().ok()?;
        let slots: BTreeMap<_, _> = links.iter().map(|r| (r.id, r.canonical_slot)).collect();
        let mut raws = BTreeMap::<u32, Vec<u32>>::new();
        for (&raw, &slot) in &slots {
            raws.entry(slot).or_default().push(raw);
        }
        let mut groups = BTreeMap::<u32, Vec<NavalRegionalMineRow>>::new();
        let mut unlinked = BTreeMap::<u32, u64>::new();
        for row in rows {
            if let Some(slot) = identities.slot(row.country) {
                groups.entry(slot).or_default().push(*row);
            } else {
                *unlinked.entry(row.country).or_default() += 1;
            }
        }
        let owners = groups
            .into_iter()
            .map(|(slot, group)| (slot, owner(&identities, &slots, slot, &group)))
            .collect();
        let mut last = Self {
            mines: frame.tables[TableId::NavalRegionalMines.index()].clone(),
            links: frame.tables[TableId::CountryLinks.index()].clone(),
            countries: frame.tables[TableId::Countries.index()].clone(),
            identities,
            slots,
            raws,
            owners,
            unlinked,
            notes: BTreeMap::new(),
        };
        let unlinked: u64 = last.unlinked.values().sum();
        count(&mut last.notes, UNLINKED, unlinked, true);
        for owner in last.owners.values() {
            if let Some(note) = &owner.note {
                count(&mut last.notes, note, 1, true);
            }
        }
        Some(last)
    }

    /// The vectors the hour holds but cannot name.
    fn unnamed(&self) -> i128 {
        (self.unlinked.len() + self.owners.values().filter(|owner| owner.unnamed).count()) as i128
    }

    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .add(size_of::<Self>())
            .view(&self.mines)
            .view(&self.links)
            .view(&self.countries)
            .tree::<u32, u32>(self.slots.len())
            .tree::<u32, Vec<u32>>(self.raws.len())
            .tree::<u32, Owner>(self.owners.len())
            .tree::<u32, u64>(self.unlinked.len())
            .heap(&self.notes);
        self.identities.held(bytes);
        for raws in self.raws.values() {
            bytes.heap(raws);
        }
        for owner in self.owners.values() {
            bytes.add(owner.values.capacity() * size_of::<(String, i128)>());
            for (name, _) in &owner.values {
                bytes.heap(name);
            }
            if let Some(note) = &owner.note {
                bytes.heap(note);
            }
        }
    }
}

/// What one canonical slot's vectors read: `group`, its rows in table order.
fn owner(
    identities: &Countries,
    slots: &BTreeMap<u32, u32>,
    slot: u32,
    group: &[NavalRegionalMineRow],
) -> Owner {
    let Some(owner) = identities.slotted(slot).filter(|s| !s.contains('|')) else {
        return Owner {
            values: Vec::new(),
            note: Some("Minefield owner tag is unavailable".into()),
            unnamed: true,
        };
    };
    let raw = group[0].country;
    let admitted = (|| -> Result<u32, String> {
        let levels = Levels::from_rows(group, slots)?;
        let witnesses: Vec<_> = group.iter().filter(|r| r.kind == 0).collect();
        let [witness] = witnesses.as_slice() else {
            return Err("Regional mine owner completeness witness is missing or duplicated".into());
        };
        for row in group {
            if row.country != raw {
                return Err("Regional mine vector duplicates a canonical country".into());
            }
            if row.kind == 1 {
                let _ = levels.amount(raw, row.region)?;
            }
        }
        Ok(witness.region)
    })();
    match admitted {
        Ok(extent) => {
            // names: OWNER|extent and OWNER|region; zeros stay sparse.
            let mut values = vec![(format!("{owner}|extent"), i128::from(extent))];
            for row in group.iter().filter(|r| r.kind == 1 && r.amount != 0) {
                values.push((format!("{owner}|{}", row.region), i128::from(row.amount)));
            }
            Owner {
                values,
                note: None,
                unnamed: false,
            }
        }
        // Dated: the owner is known to hold an unadmitted vector.
        Err(why) => Owner {
            values: vec![(format!("{owner}|refused"), 1)],
            note: Some(why),
            unnamed: false,
        },
    }
}

/// Add or take away `count` notes of `reason`, keeping no reason at zero.
fn count(notes: &mut BTreeMap<String, u64>, reason: &str, count: u64, add: bool) {
    if count == 0 {
        return;
    }
    let held = notes.entry(reason.to_owned()).or_default();
    if add {
        *held += count;
    } else {
        *held -= count;
        if *held == 0 {
            notes.remove(reason);
        }
    }
}

const UNLINKED: &str = "Minefield owner country link is unavailable";

#[cfg(test)]
pub static FAMILY: Family = dashboard_keyed::block::family::<MinefieldsObserver>(
    "minefields",
    "minefields.extract",
    "minefields.sequential",
    "minefields.fallback",
);
impl Observer for MinefieldsObserver {
    type Content = ();

    fn content(
        _source: &dashboard_base::Source,
        _options: Option<&dashboard_core::config::ContentOptions>,
        _check: &mut dyn FnMut() -> std::io::Result<()>,
    ) -> std::io::Result<Self::Content> {
        Ok(())
    }

    const NAME: &'static str = "minefields";
    const TABLES: &'static [TableId] = &[
        TableId::NavalRegionalMines,
        TableId::CountryLinks,
        TableId::Countries,
    ];
    const READ: TableId = TableId::NavalRegionalMines;
    #[cfg(test)]
    fn family() -> &'static Family {
        &FAMILY
    }
    fn fields() -> io::Result<Vec<TableFields>> {
        Ok(vec![
            TableFields::all(TableId::NavalRegionalMines),
            TableFields::all(TableId::CountryLinks),
            TableFields::named(TableId::Countries, &["canonical_slot", "tag"])?,
        ])
    }
    fn observe(
        &mut self,
        _: FormatVersion,
        frame: &DecodedFrame,
        values: &mut Values,
    ) -> io::Result<bool> {
        self.last = None;
        let Some(last) = Last::read(frame) else {
            shared::note(
                &mut self.refusals,
                "Minefield amounts or country identities are not recorded",
            );
            return Ok(false);
        };
        for owner in last.owners.values() {
            for (name, value) in &owner.values {
                values.push(name.clone(), *value);
            }
        }
        let unnamed = last.unnamed();
        if unnamed > 0 {
            values.push(format!("{UNNAMED}|refused"), unnamed);
        }
        self.note(&last.notes);
        self.last = Some(Box::new(last));
        Ok(true)
    }
    fn observe_changes(
        &mut self,
        _: FormatVersion,
        frame: &DecodedFrame,
        _: &dashboard_keyed::delta::Names<'_>,
        delta: &mut Delta,
    ) -> io::Result<Option<bool>> {
        let Some(mut last) = self.last.take() else {
            return Ok(None);
        };
        let view = |table: TableId| &frame.tables[table.index()];
        // Identities that name any owner anew observe the hour whole.
        if !(same_rows::<tables::CountryLinks>(
            Some(&last.links),
            view(TableId::CountryLinks),
            PartialEq::eq,
        ) && same_rows::<tables::Countries>(
            Some(&last.countries),
            view(TableId::Countries),
            |a, b| a.canonical_slot == b.canonical_slot && a.tag == b.tag,
        )) {
            return Ok(None);
        }
        last.links = view(TableId::CountryLinks).clone();
        last.countries = view(TableId::Countries).clone();
        let Ok(rows) = frame.complete::<tables::NavalRegionalMines>() else {
            return Ok(None);
        };
        if !SharedView::ptr_eq(&last.mines, view(TableId::NavalRegionalMines)) {
            let Some(before) = dashboard_keyed::delta::rows::<tables::NavalRegionalMines>(&last.mines)
            else {
                return Ok(None);
            };
            let mut changed = BTreeSet::new();
            let ordered = dashboard_keyed::delta::changed_rows(before, rows, PartialEq::eq, |old, new| {
                changed.extend(old.or(new).map(|row| row.country));
            });
            if ordered.is_none() {
                return Ok(None);
            }
            let of = |raw: u32| {
                let first = u128::from(raw) << 33;
                dashboard_keyed::delta::key_range(rows, first..first + (1 << 33))
            };
            let mut slots = BTreeSet::new();
            for raw in changed {
                match last.identities.slot(raw) {
                    Some(slot) => {
                        slots.insert(slot);
                    }
                    None => {
                        let held = last.unlinked.remove(&raw).unwrap_or(0);
                        let now = of(raw).len() as u64;
                        if now > 0 {
                            last.unlinked.insert(raw, now);
                        }
                        count(&mut last.notes, UNLINKED, held, false);
                        count(&mut last.notes, UNLINKED, now, true);
                    }
                }
            }
            for slot in slots {
                let group: Vec<NavalRegionalMineRow> = last
                    .raws
                    .get(&slot)
                    .into_iter()
                    .flatten()
                    .flat_map(|&raw| of(raw).iter().copied())
                    .collect();
                let now = (!group.is_empty())
                    .then(|| owner(&last.identities, &last.slots, slot, &group));
                let held = match now {
                    Some(now) => last.owners.insert(slot, now),
                    None => last.owners.remove(&slot),
                };
                if let Some(held) = &held {
                    for (name, _) in &held.values {
                        delta.remove(name.clone());
                    }
                    if let Some(note) = &held.note {
                        count(&mut last.notes, note, 1, false);
                    }
                }
                if let Some(now) = last.owners.get(&slot) {
                    for (name, value) in &now.values {
                        delta.set(name.clone(), *value);
                    }
                    if let Some(note) = &now.note {
                        count(&mut last.notes, note, 1, true);
                    }
                }
            }
            let name = format!("{UNNAMED}|refused");
            match last.unnamed() {
                0 => delta.remove(name),
                unnamed => delta.set(name, unnamed),
            }
            last.mines = view(TableId::NavalRegionalMines).clone();
        }
        self.note(&last.notes);
        self.last = Some(last);
        Ok(Some(true))
    }
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.heap(&self.refusals);
        if let Some(last) = &self.last {
            last.held(bytes);
        }
    }
}

impl MinefieldsObserver {
    /// Note what an observation of the held inputs refuses.
    fn note(&mut self, notes: &BTreeMap<String, u64>) {
        for (reason, &count) in notes {
            *self.refusals.entry(reason.clone()).or_default() += count;
        }
    }
}
// The owner part of the count of vectors the hour holds but cannot name.
const UNNAMED: &str = "?";

/// The reply to `q` from the family's native publication.
pub fn query(q: &Query<'_>) -> io::Result<Built> {
    answer(q, || q.open_native::<MinefieldsObserver>())
}

/// The reply to `q` from the walk's pages.
#[cfg(test)]
pub(crate) fn row_query(q: &Query<'_>) -> io::Result<Built> {
    answer(q, || q.open::<MinefieldsObserver>())
}

/// The reply to `q` from the family's publication `open` opens.
pub(crate) fn answer<'q>(
    q: &'q Query<'_>,
    open: impl FnOnce() -> io::Result<Option<Opened<'q, MinefieldsObserver>>>,
) -> io::Result<Built> {
    let params = shared::params::<Minefields>(q)?;
    if matches!(params.view, View::Reports) {
        return reports(q, &params);
    }
    let (first, last) = q.window()?;
    let labels = shared::labels(q);
    let mut warnings = labels
        .as_ref()
        .map_or_else(|_| Vec::new(), |l| l.warnings.clone());
    let mut ids = BTreeSet::new();
    let window = match open()? {
        Some(mut o) => {
            warnings.extend(shared::warnings(&o.scan.observer.refusals));
            for name in o.scan.names() {
                if let Some((_, r)) = name
                    .split_once('|')
                    .and_then(|(c, r)| r.parse::<u32>().ok().map(|r| (c, r)))
                {
                    ids.insert(r);
                }
            }
            o.window(first, last, q.through(), |name| {
                name.split_once('|').is_some_and(|(owner, _)| {
                    // Owners that could not be named may be any selected country.
                    owner == UNNAMED
                        || params
                            .tags
                            .as_ref()
                            .is_none_or(|tags| tags.iter().any(|t| shared::selected(owner, t)))
                })
            })?
        }
        None => shared::empty(first, last),
    };
    if let Ok(l) = &labels
        && let Ok(sea) = &l.sea_regions
    {
        ids.extend(sea);
    }
    let mut notes = Refusals::default();
    let regions = shared::choices(&ids, &labels, &mut notes);
    warnings.extend(notes.counts("minefield region names"));
    shared::finish(
        q,
        &HistoryReply {
            window,
            regions,
            scale: shared::SCALE,
            warnings,
        },
    )
}

fn reports(q: &Query<'_>, p: &Params) -> io::Result<Built> {
    let hour = p.hour.expect("validated reports hour");
    let region = p.region.expect("validated reports region");
    let (first, last) = q.bounds()?;
    if hour < first || hour > last {
        return Err(dashboard_core::invalid(
            "Reports hour is outside the branch",
        ));
    }
    let mut reply = ReportsReply {
        hour,
        read_hour: None,
        region,
        reports: Vec::new(),
        warnings: Vec::new(),
    };
    let Some(frame) = native::frame_at(q.source, q.branch, hour, REPORT_TABLES, q.cancelled)? else {
        reply
            .warnings
            .push("1 standing mine report list not shown: requested hour is not recorded".into());
        return shared::finish(q, &reply);
    };
    let rows = match frame.complete::<tables::NavalAccidentReports>() {
        Ok(rows) => rows,
        Err(why) => {
            reply
                .warnings
                .push(format!("1 standing mine report list not shown: {why}"));
            return shared::finish(q, &reply);
        }
    };
    reply.read_hour = frame.tables[TableId::NavalAccidentReports.index()]
        .sampled_at()
        .filter(|h| *h <= hour);
    let cat = q
        .content
        .and_then(|c| catalogue::catalogue(q.source, c).ok());
    let countries = Countries::read(&frame);
    let mut notes = Refusals::default();
    for (at, row) in standing(rows, region).enumerate() {
        dashboard_text::metadata::check_cancel(q.cancelled)?;
        let mut country = |raw, field| {
            shared::country_part(&frame, countries.as_ref(), raw).or_else(|| {
                notes.note(
                    format!("Mine report {field} country is unresolved"),
                    at.to_string(),
                );
                None
            })
        };
        let victim = country(row.owner, "victim");
        let mine_owner = country(row.mine_owner, "mine owner");
        let ship = ship_name(q, &frame, row, cat.as_deref())
            .map_err(|why| notes.note(why, at.to_string()))
            .ok()
            .flatten();
        let equipment = equipment_name(&frame, row, cat.as_deref())
            .map_err(|why| notes.note(why, at.to_string()))
            .ok()
            .flatten();
        reply.reports.push(Report {
            kind: row.kind,
            ordinal: row.ordinal,
            victim,
            mine_owner,
            ship,
            equipment,
            damage: row.damage.to_string(),
            sunk: row.sunk != 0,
            first_seen: None,
        });
    }
    reply
        .warnings
        .extend(notes.counts("standing mine report fields"));
    shared::finish(q, &reply)
}
pub(crate) fn standing(
    rows: &[NavalAccidentReportRow],
    region: u32,
) -> impl Iterator<Item = &NavalAccidentReportRow> {
    rows.iter()
        .filter(move |r| r.kind == 1 && r.region == region)
}
fn ship_name(
    q: &Query<'_>,
    frame: &DecodedFrame,
    row: &NavalAccidentReportRow,
    cat: Option<&catalogue::Catalogue>,
) -> Result<Option<String>, String> {
    if row.ship == 0 {
        return Ok(None);
    }
    let bytes = frame
        .complete::<tables::CountryCounterInputs>()
        .ok()
        .and_then(|rows| rows.iter().find(|r| r.pride_of_the_fleet == row.ship))
        .and_then(|r| r.pride_name.require("CountryCounterInputs.pride_name").ok())
        .and_then(Option::as_ref)
        .ok_or("Standing mine report ship name is not recorded for its full handle")?;
    let cat = cat.ok_or("Standing mine report ship naming content is unavailable")?;
    let language = &q.source.replay_ref().identity.language;
    let recorded = language.strip_prefix("l_").unwrap_or(language);
    dashboard_text::scripted_text::stored_text(
        &cat.localisation,
        recorded,
        bytes,
        "Mine report ship name",
    )
    .and_then(|text| catalogue::plain_text(&text))
    .map(Some)
}
fn equipment_name(
    frame: &DecodedFrame,
    row: &NavalAccidentReportRow,
    cat: Option<&catalogue::Catalogue>,
) -> Result<Option<String>, String> {
    use dashboard_text::variant_names::{self, Tags, Texts};
    if row.equipment == 0 {
        return Ok(None);
    }
    let rows = frame
        .complete::<tables::VariantDefinitions>()
        .map_err(|e| e.to_string())?;
    let definition = rows
        .iter()
        .find(|r| r.id == row.equipment)
        .ok_or("Standing mine report equipment variant is not recorded")?;
    let countries = frame
        .complete::<tables::Countries>()
        .map_err(|e| e.to_string())?;
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(|e| e.to_string())?;
    let tags = Tags::new(
        countries,
        links,
        frame.complete::<tables::CountryAppearance>().ok(),
    );
    let lexical = frame
        .complete::<tables::LexicalTokens>()
        .map_err(|e| e.to_string())?;
    let raw = variant_names::raw_variant(definition, frame.hour, &tags, lexical);
    variant_names::variant_name(&raw, cat.map(|c| c as &dyn Texts))?
        .ok_or_else(|| "Standing mine report equipment name is unavailable".into())
        .map(Some)
}
