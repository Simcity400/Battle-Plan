//! The division log projected from a columnar walk ([`replay::segments`]): each hour's
//! candidate first sightings, witnessed removals and warnings, from the walker's tables,
//! with the presentations they carry, for [`super::Scan::observe_projected`] to keep in
//! order. The census is the walker's live divisions, moved by the hour's changed slots when
//! the last census was the hour before and read whole otherwise. Each division's
//! presentation operands are kept as the census that last built them read them, as the row
//! path keeps its presentations, and presentations are built only for the divisions an event
//! names. Names and link tags read a frame of the naming tables, moved by the hour's changes,
//! through the row path's own functions.
//!
//! A walk starts only where the divisions and their removal witnesses were read whole
//! ([`Operator::starts_at`]), so the census and witnesses before its first hour are the
//! restart's, and goes on into its branch's later runs, carrying them as the row path's scan
//! carries them across. What only the hours before a walk know is left for the ordered scan: the
//! template read hour is dated [`super::block::INHERITED`], and the presentations named from
//! the restart's census are marked with the hour that census is taken to have been built,
//! for the ordered scan to name again when it was built earlier ([`Hour::names`]).

use super::{
    ArmyName, DivisionLogKind, Event, IDENTITY_UNAVAILABLE, Presentation, TemplateName,
    block::INHERITED, deletion_signature, display_naming, disposition, entry,
};
use dashboard_text::metadata::{LinkTags, slot_read};
use replay::{
    columnar_frames::{Views, frame_heap_bytes, metrics},
    columns,
    segments::{self, Item, Operator, Sink, Start, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{DivisionStatusRow, EquipmentDestructionCause as Cause, TableId, tables},
    scope::CaptureMode,
};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{HashMap, HashSet},
    io,
    mem::size_of,
};

/// The tables names and link tags read.
const NAMING: [TableId; 4] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::DivisionTemplates,
    TableId::DivisionTemplateIcons,
];

/// The tables removals read.
const REMOVALS: [TableId; 3] = [
    TableId::DivisionEquipmentDeletions,
    TableId::DestroyedDivisions,
    TableId::LandEventCoverage,
];

/// The tables whose changes the projection follows: those the row path's census
/// presentations read, whose change builds them again, and the removal witnesses.
const FOLLOWED: [TableId; 7] = [
    TableId::Divisions,
    TableId::DivisionStatus,
    TableId::DivisionTemplates,
    TableId::DivisionTemplateIcons,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::DivisionEquipmentDeletions,
];
const DIVISIONS: usize = 0;
const STATUS: usize = 1;
const DELETIONS: usize = 6;

/// One event of a projected hour: a candidate first sighting names its division. An event
/// presented from the restart's census names the hour that census is taken to have been
/// built.
pub(super) struct Projected {
    pub(super) event: Event,
    pub(super) candidate: Option<u64>,
    pub(super) restored: Option<i32>,
}

/// One projected hour.
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// Whether Divisions held every row.
    pub census: bool,
    /// Whether DivisionEquipmentDeletions held every row.
    pub losses: bool,
    /// DivisionTemplates' read this hour: whether it held every row.
    pub template_read: Option<bool>,
    /// Whether the census built its presentations at this hour.
    pub built: bool,
    pub(super) events: Vec<Projected>,
    pub(super) warnings: Vec<(String, usize)>,
    /// A walk's last hour, when its census was built before it: the name of each template its
    /// presentations name, as that build named it, for the walk after.
    pub(super) names: Option<HashMap<u64, Option<TemplateName>>>,
}

impl Item for Hour {
    fn heap(&self) -> usize {
        // What events' rows and presentations own is not counted.
        let names = self.names.as_ref().map_or(0, |names| {
            names.capacity() * size_of::<(u64, Option<TemplateName>)>()
        });
        self.warnings
            .iter()
            .map(|(reason, _)| reason.capacity())
            .sum::<usize>()
            + self.warnings.capacity() * size_of::<(String, usize)>()
            + self.events.capacity() * size_of::<Projected>()
            + names
    }
}

/// One division's presentation operands.
#[derive(Clone)]
struct Operands {
    province: u32,
    template: u64,
    status: Option<ArmyName>,
}

/// The columns a projection reads by name.
struct Fields {
    province: usize,
    template: usize,
    reveal_skip: usize,
    owners: [usize; 2],
    status: [usize; 2],
    country: [usize; 2],
    link: usize,
    icon: [usize; 5],
}

/// The Divisions columns an hour reads, by slot.
struct Divisions<'a> {
    reveal_skip: &'a [u8],
    province: &'a [u32],
    template: &'a [u64],
    roster: &'a [u32],
    expeditionary: &'a [u32],
}

impl<'a> Divisions<'a> {
    fn new(state: &'a State, fields: &Fields) -> io::Result<Self> {
        let table = columns::table(state, TableId::Divisions)?;
        Ok(Self {
            reveal_skip: columns::plain(table, fields.reveal_skip)?,
            province: columns::plain(table, fields.province)?,
            template: columns::plain(table, fields.template)?,
            roster: columns::plain(table, fields.owners[0])?,
            expeditionary: columns::plain(table, fields.owners[1])?,
        })
    }

    /// Whether the division at `slot` is a division, not an intel copy.
    fn shown(&self, slot: u32) -> bool {
        self.reveal_skip.get(slot as usize) == Some(&0)
    }

    /// The division at `slot`'s owner: its positive expeditionary owner, else its roster
    /// owner.
    fn owner(&self, slot: u32) -> u32 {
        let at = slot as usize;
        match self.expeditionary.get(at).copied() {
            Some(owner) if owner as i32 > 0 => owner,
            _ => self.roster.get(at).copied().unwrap_or_default(),
        }
    }

    fn operands(&self, slot: u32, status: Option<ArmyName>) -> Operands {
        let at = slot as usize;
        Operands {
            province: self.province.get(at).copied().unwrap_or_default(),
            template: self.template.get(at).copied().unwrap_or_default(),
            status,
        }
    }
}

/// The census presentations, as the census that last built them read them.
struct Built {
    hour: i32,
    /// The naming frame then, dated the build's hour, once what naming reads has changed
    /// since: until then the naming frame names alike.
    frame: Option<DecodedFrame>,
    /// Every Divisions row's operands then, and whether DivisionStatus held every row.
    operands: HashMap<u64, Operands>,
    status: bool,
    /// Whether Divisions or DivisionStatus changed since, at an hour that built nothing.
    drifted: bool,
    /// For the restart's census: the hour it is taken to have been built.
    restored: Option<i32>,
}

/// One segment's projection.
pub struct Projector {
    format: FormatVersion,
    fields: Fields,
    naming: Views,
    removals: Views,
    /// The naming tables' frame this hour.
    frame: Option<DecodedFrame>,
    built: Option<Built>,
    links: LinkTags,
    /// The last census's divisions and hour.
    live: HashSet<u64>,
    census_hour: Option<i32>,
    deletions: HashMap<u64, (u32, u32, u8, Option<u8>)>,
    template_hour: Option<i32>,
    /// The template read hour at the last census.
    census_template: Option<i32>,
    previous_hour: Option<i32>,
    shapes: [Option<Shape>; FOLLOWED.len()],
    /// The hour projected last, held back for the segment's end to name its templates.
    pending: Option<Hour>,
}

/// A table's status at an hour, its missing rows and unavailable reason included: the row
/// path's views differ at an hour any of them changes.
type Shape = columns::Status;

fn shape(state: &State, table: TableId) -> Shape {
    columns::status(state, table)
}

fn names(mask: u64, fields: &[usize]) -> bool {
    fields
        .iter()
        .any(|&column| column >= 64 || mask >> column & 1 != 0)
}

/// Whether the hour changed any of `fields` (every field when empty) of `table`.
fn touched(state: &State, table: TableId, fields: &[usize]) -> io::Result<bool> {
    let changes = columns::changes(state, table)?;
    let named = |mask: u64| fields.is_empty() || names(mask, fields);
    Ok(!changes.inserted.is_empty()
        || !changes.deleted.is_empty()
        || changes.truncated
        || changes.masks.iter().any(|&mask| named(mask))
        || changes.fill_masks.iter().any(|&mask| named(mask)))
}

/// The slots of `table` the hour changed or filled with any of `fields`, and those it
/// inserted.
fn written<'a>(
    state: &'a State,
    table: TableId,
    fields: &'a [usize],
) -> io::Result<impl Iterator<Item = u32> + 'a> {
    let changes = columns::changes(state, table)?;
    Ok(changes
        .changed
        .iter()
        .zip(&changes.masks)
        .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
        .filter(|&(_, &mask)| names(mask, fields))
        .map(|(&slot, _)| slot)
        .chain(changes.inserted.iter().copied()))
}

/// A presentation from `operands`, named through `frame` at its hour and dated `hour`, as
/// the row path's census builds it.
fn present(
    frame: &DecodedFrame,
    id: u64,
    operands: &Operands,
    hour: i32,
    template_hour: Option<i32>,
) -> Presentation {
    let templates = frame
        .complete::<tables::DivisionTemplates>()
        .unwrap_or_default();
    let icons = frame
        .complete::<tables::DivisionTemplateIcons>()
        .unwrap_or_default();
    let template = super::keyed(templates, operands.template, |row| row.template)
        .map(|_| TemplateName(display_naming(frame, operands.template)));
    let icon = super::keyed(icons, operands.template, |row| row.template).map(|row| {
        (
            row.priority_unit,
            row.custom_icon,
            row.priority_unit_name.clone(),
        )
    });
    Presentation {
        hour,
        division: id,
        province: operands.province,
        template_id: operands.template,
        status: operands.status.clone(),
        template,
        template_hour,
        icon,
    }
}

impl Projector {
    pub fn new(format: FormatVersion, mode: CaptureMode) -> io::Result<Self> {
        let field = |table, name| columns::field(table, name);
        let icon = |name| field(TableId::DivisionTemplateIcons, name);
        Ok(Self {
            format,
            fields: Fields {
                province: field(TableId::Divisions, "province")?,
                template: field(TableId::Divisions, "template")?,
                reveal_skip: field(TableId::Divisions, "reveal_skip")?,
                owners: [
                    field(TableId::Divisions, "roster_owner")?,
                    field(TableId::Divisions, "expeditionary_owner")?,
                ],
                status: [
                    field(TableId::DivisionStatus, "name")?,
                    field(TableId::DivisionStatus, "naming")?,
                ],
                country: [
                    field(TableId::Countries, "tag")?,
                    field(TableId::Countries, "canonical_slot")?,
                ],
                link: field(TableId::CountryLinks, "canonical_slot")?,
                // What a presentation and its template's name read of an icon row.
                icon: [
                    icon("priority_unit")?,
                    icon("priority_unit_name")?,
                    icon("custom_icon")?,
                    icon("country")?,
                    icon("display_name")?,
                ],
            },
            naming: Views::new(&NAMING, format, mode)?,
            removals: Views::new(&REMOVALS, format, mode)?,
            frame: None,
            built: None,
            links: LinkTags::default(),
            live: HashSet::new(),
            census_hour: None,
            deletions: HashMap::new(),
            template_hour: Some(INHERITED),
            census_template: Some(INHERITED),
            previous_hour: None,
            shapes: [None; FOLLOWED.len()],
            pending: None,
        })
    }

    /// The name operands of division `id` now; None when DivisionStatus is not complete or
    /// holds no row for it.
    fn status(&self, state: &State, id: u64) -> io::Result<Option<ArmyName>> {
        if !columns::complete(state, TableId::DivisionStatus) {
            return Ok(None);
        }
        let status = columns::table(state, TableId::DivisionStatus)?;
        match status.slot(u128::from(id)) {
            Some(slot) => {
                let row: DivisionStatusRow =
                    columns::row(state, TableId::DivisionStatus, slot, self.format.version())?;
                Ok(Some(ArmyName::new(&row)))
            }
            None => Ok(None),
        }
    }

    /// Every Divisions row's operands now.
    fn operands(
        &self,
        state: &State,
        divisions: &Divisions<'_>,
    ) -> io::Result<HashMap<u64, Operands>> {
        let table = columns::table(state, TableId::Divisions)?;
        let mut operands = HashMap::with_capacity(table.live().count());
        for slot in table.live().ones() {
            if let Some(id) = table.key(slot).map(|key| key as u64) {
                operands.insert(id, divisions.operands(slot, self.status(state, id)?));
            }
        }
        Ok(operands)
    }

    /// The naming frame moved to this hour. The census keeps the frame it named its
    /// presentations through once what naming reads changes, or a naming table's status.
    fn refresh_naming(&mut self, hour: &segments::Hour<'_>, reshaped: bool) -> io::Result<()> {
        let state = hour.state;
        let renamed = reshaped
            || touched(state, TableId::DivisionTemplates, &[])?
            || touched(state, TableId::DivisionTemplateIcons, &self.fields.icon)?
            || touched(state, TableId::Countries, &self.fields.country)?
            || touched(state, TableId::CountryLinks, &[self.fields.link])?;
        if renamed || hour.starts {
            self.keep_built_frame();
        }
        // No other holder, so the views move in place.
        self.frame = None;
        self.frame = Some(self.naming.advance(state, hour.manifest, hour.starts)?);
        Ok(())
    }

    /// The census keeps its own naming frame, the one it named its presentations through.
    fn keep_built_frame(&mut self) {
        if let Some(built) = &mut self.built
            && built.frame.is_none()
            && let Some(frame) = &self.frame
        {
            let mut frame = frame.clone();
            frame.hour = built.hour;
            built.frame = Some(frame);
        }
    }

    /// The naming frame the last census build named its presentations through, dated then.
    fn built_frame(&self) -> Option<std::borrow::Cow<'_, DecodedFrame>> {
        let built = self.built.as_ref()?;
        Some(match &built.frame {
            Some(frame) => std::borrow::Cow::Borrowed(frame),
            None => {
                let mut frame = self.frame.clone()?;
                frame.hour = built.hour;
                std::borrow::Cow::Owned(frame)
            }
        })
    }

    /// The census's divisions this hour, moved from the last census: those it gained, in
    /// ascending order, and those it lost.
    fn census(
        &mut self,
        state: &State,
        at: i32,
        starts: bool,
        divisions: &Divisions<'_>,
    ) -> io::Result<(Vec<(u64, u32)>, Vec<u64>)> {
        let table = columns::table(state, TableId::Divisions)?;
        let mut gained = Vec::new();
        let mut lost = Vec::new();
        if starts || self.census_hour != at.checked_sub(1) {
            // Hours since the last census may have read rows into the walker, and a walk's
            // first hour holds none of the hours before: read it whole.
            let mut current = HashSet::with_capacity(self.live.len());
            for slot in table.live().ones() {
                if let Some(id) = table.key(slot).map(|key| key as u64)
                    && divisions.shown(slot)
                {
                    current.insert(id);
                    if !self.live.contains(&id) {
                        gained.push((id, slot));
                    }
                }
            }
            lost.extend(self.live.iter().filter(|id| !current.contains(id)));
            self.live = current;
        } else {
            let changes = columns::changes(state, TableId::Divisions)?;
            let mut ids: Vec<u64> = changes.deleted_keys.iter().map(|&key| key as u64).collect();
            ids.extend(
                written(state, TableId::Divisions, &[self.fields.reveal_skip])?
                    .filter_map(|slot| table.key(slot).map(|key| key as u64)),
            );
            ids.sort_unstable();
            ids.dedup();
            for id in ids {
                let slot = table.slot(u128::from(id));
                let now = slot.is_some_and(|slot| divisions.shown(slot));
                match (self.live.contains(&id), now, slot) {
                    (false, true, Some(slot)) => {
                        self.live.insert(id);
                        gained.push((id, slot));
                    }
                    (true, false, _) => {
                        self.live.remove(&id);
                        lost.push(id);
                    }
                    _ => {}
                }
            }
        }
        gained.sort_unstable();
        Ok((gained, lost))
    }

    /// The census presentations built again this hour: moved by the hour's changes when
    /// nothing drifted since the last build the hour before, else read whole.
    fn build(&mut self, state: &State, at: i32, divisions: &Divisions<'_>) -> io::Result<()> {
        let status = columns::complete(state, TableId::DivisionStatus);
        let moved = self
            .built
            .take()
            .filter(|built| !built.drifted && built.hour.checked_add(1) == Some(at));
        let operands = match moved {
            Some(built) => {
                let mut operands = built.operands;
                let table = columns::table(state, TableId::Divisions)?;
                for &key in &columns::changes(state, TableId::Divisions)?.deleted_keys {
                    operands.remove(&(key as u64));
                }
                let changes = columns::changes(state, TableId::Divisions)?;
                for &slot in &changes.inserted {
                    if let Some(id) = table.key(slot).map(|key| key as u64) {
                        operands.insert(id, divisions.operands(slot, self.status(state, id)?));
                    }
                }
                for slot in written(
                    state,
                    TableId::Divisions,
                    &[self.fields.province, self.fields.template],
                )? {
                    if let Some(id) = table.key(slot).map(|key| key as u64)
                        && let Some(held) = operands.get_mut(&id)
                    {
                        let now = divisions.operands(slot, None);
                        (held.province, held.template) = (now.province, now.template);
                    }
                }
                // Names the hour changed; every name once DivisionStatus is whole again.
                let renamed: Vec<u64> = if status && !built.status {
                    operands.keys().copied().collect()
                } else if status {
                    let names = columns::table(state, TableId::DivisionStatus)?;
                    written(state, TableId::DivisionStatus, &self.fields.status)?
                        .filter_map(|slot| names.key(slot).map(|key| key as u64))
                        .chain(
                            columns::changes(state, TableId::DivisionStatus)?
                                .deleted_keys
                                .iter()
                                .map(|&key| key as u64),
                        )
                        .collect()
                } else {
                    Vec::new()
                };
                for id in renamed {
                    if operands.contains_key(&id) {
                        let name = self.status(state, id)?;
                        if let Some(held) = operands.get_mut(&id) {
                            held.status = name;
                        }
                    }
                }
                operands
            }
            None => self.operands(state, divisions)?,
        };
        self.built = Some(Built {
            hour: at,
            frame: None,
            operands,
            status,
            drifted: false,
            restored: None,
        });
        Ok(())
    }

    /// Division `id`'s presentation as the last census built it, dated that census.
    fn census_presentation(&self, id: u64) -> Option<(Presentation, Option<i32>)> {
        let built = self.built.as_ref()?;
        let held = built.operands.get(&id)?;
        let held = Operands {
            status: held.status.clone().filter(|_| built.status),
            ..held.clone()
        };
        let frame = self.built_frame()?;
        Some((
            present(&frame, id, &held, self.census_hour?, self.census_template),
            built.restored,
        ))
    }
}

impl Operator for Projector {
    type Output = Hour;

    fn starts_at(&self, start: Start<'_>) -> bool {
        match start {
            Start::Restart(state) => {
                columns::complete(state, TableId::Divisions)
                    && columns::complete(state, TableId::DivisionEquipmentDeletions)
            }
            Start::Run { continues } => !continues,
        }
    }

    fn start(&mut self, start: Start<'_>, branch: u64, first: i32) -> io::Result<()> {
        let Start::Restart(state) = start else {
            // A branch's first run knows no earlier read.
            self.template_hour = None;
            self.census_template = None;
            return Ok(());
        };
        let before = first.checked_sub(1).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "A walk starts at no hour")
        })?;
        self.previous_hour = Some(before);
        // The restart's census and removal witnesses, read whole ([`Operator::starts_at`]).
        let divisions = Divisions::new(state, &self.fields)?;
        let table = columns::table(state, TableId::Divisions)?;
        for slot in table.live().ones() {
            if let Some(id) = table.key(slot).map(|key| key as u64)
                && divisions.shown(slot)
            {
                self.live.insert(id);
            }
        }
        self.census_hour = Some(before);
        self.built = Some(Built {
            hour: before,
            frame: Some(self.naming.restored(state, branch, before)?),
            operands: self.operands(state, &divisions)?,
            status: columns::complete(state, TableId::DivisionStatus),
            drifted: false,
            restored: Some(before),
        });
        let deletions = columns::table(state, TableId::DivisionEquipmentDeletions)?;
        for slot in deletions.live().ones() {
            let row: replay_data::rows::DivisionEquipmentDeletionRow = columns::row(
                state,
                TableId::DivisionEquipmentDeletions,
                slot,
                self.format.version(),
            )?;
            self.deletions
                .insert(row.division_handle, deletion_signature(&row));
        }
        Ok(())
    }

    fn cross(&mut self, first: i32) -> io::Result<()> {
        // The walker's tables start empty: the census is read whole and built again.
        self.keep_built_frame();
        if let Some(built) = &mut self.built {
            built.drifted = true;
        }
        // A gap forgets what dates the hours after it, as the row path's scan does.
        if self.previous_hour.and_then(|last| last.checked_add(1)) != Some(first) {
            self.previous_hour = None;
            self.template_hour = None;
        }
        Ok(())
    }

    fn hour(&mut self, hour: &segments::Hour<'_>, out: &mut dyn Sink<Hour>) -> io::Result<()> {
        // The hour before is whole: it reaches the emit even if this hour refuses.
        if let Some(before) = self.pending.take() {
            out.push(before)?;
        }
        let state = hour.state;
        let at = hour.hour;
        let mut warnings: Vec<(String, usize)> = Vec::new();
        let mut warn = |reason: &str, count: usize| warnings.push((reason.to_owned(), count));
        // Each followed table's rows or status changed, as the row path's change facts say:
        // every table at a segment's first hour, as at a row block's.
        let mut changed = [false; FOLLOWED.len()];
        let mut reshaped = hour.opens;
        for (index, &table) in FOLLOWED.iter().enumerate() {
            let now = Some(shape(state, table));
            let status = std::mem::replace(&mut self.shapes[index], now) != now;
            reshaped |= status && NAMING.contains(&table);
            changed[index] = hour.opens || status || touched(state, table, &[])?;
        }
        let cpu = ::replay_frame::cpu::scope("division log naming", None);
        self.refresh_naming(hour, reshaped)?;
        let frame = self.frame.as_ref().expect("a naming frame");
        let labels = self.links.update(frame).is_ok();
        if !labels {
            warn(IDENTITY_UNAVAILABLE, 1);
        }
        let read = slot_read(
            self.format,
            &metrics(hour.manifest),
            TableId::DivisionTemplates,
            at,
        );
        let template_read = read.then(|| columns::complete(state, TableId::DivisionTemplates));
        if let Some(complete) = template_read {
            self.template_hour = complete.then_some(at);
        }
        drop(cpu);
        let cpu = ::replay_frame::cpu::scope("division log removals view", None);
        let removals = self.removals.advance(state, hour.manifest, hour.starts)?;
        drop(cpu);
        let cpu = ::replay_frame::cpu::scope("division log census", None);
        let mut events = Vec::new();
        let census = columns::complete(state, TableId::Divisions);
        let divisions = Divisions::new(state, &self.fields)?;
        let table = columns::table(state, TableId::Divisions)?;
        let mut lost = Vec::new();
        if census && changed[DIVISIONS] {
            let (gained, gone) = self.census(state, at, hour.starts, &divisions)?;
            lost = gone;
            let frame = self.frame.as_ref().expect("a naming frame");
            for (id, slot) in gained {
                // A division absent from the last census; the ordered scan keeps it once.
                let operands = divisions.operands(slot, self.status(state, id)?);
                let presentation = present(frame, id, &operands, at, self.template_hour);
                let mut row = entry(
                    &self.links,
                    id,
                    at,
                    DivisionLogKind::Created,
                    "first_observed",
                    divisions.owner(slot),
                    Some(&presentation),
                );
                if !labels {
                    row.tag = None;
                }
                events.push(Projected {
                    event: Event {
                        row,
                        presentation: Some(presentation),
                    },
                    candidate: Some(id),
                    restored: None,
                });
            }
        } else if !census {
            warn(
                "Divisions unavailable or incomplete; appearances unknown",
                1,
            );
        }
        drop(cpu);
        let cpu = ::replay_frame::cpu::scope("division log removals", None);
        let deletion_rows = removals
            .complete::<tables::DivisionEquipmentDeletions>()
            .ok();
        if changed[DELETIONS] {
            let mut deaths = HashMap::<u64, Vec<_>>::new();
            for death in removals
                .complete::<tables::DestroyedDivisions>()
                .unwrap_or_default()
            {
                if death.hour <= at
                    && self.previous_hour.is_some_and(|last| death.hour >= last)
                    && removals
                        .complete::<tables::LandEventCoverage>()
                        .is_ok_and(|coverage| {
                            coverage.iter().any(|span| {
                                span.complete
                                    && span.from_hour < death.hour
                                    && span.to_hour >= death.hour
                            })
                        })
                {
                    deaths.entry(death.division).or_default().push(death);
                }
            }
            if let Some(rows) = deletion_rows {
                let frame = self.frame.as_ref().expect("a naming frame");
                for deletion in rows {
                    if self.deletions.get(&deletion.division_handle)
                        == Some(&deletion_signature(deletion))
                    {
                        continue;
                    }
                    let id = deletion.division_handle;
                    // The last census's presentation, else this hour's.
                    let (presentation, restored) = match self.census_presentation(id) {
                        Some((presentation, restored)) => (Some(presentation), restored),
                        None => (
                            table
                                .slot(u128::from(id))
                                .filter(|_| census)
                                .map(|slot| {
                                    let operands =
                                        divisions.operands(slot, self.status(state, id)?);
                                    io::Result::Ok(present(
                                        frame,
                                        id,
                                        &operands,
                                        at,
                                        self.template_hour,
                                    ))
                                })
                                .transpose()?,
                            None,
                        ),
                    };
                    let (kind, status) = disposition(deletion);
                    let raw = if (deletion.expeditionary_owner as i32) > 0 {
                        deletion.expeditionary_owner
                    } else {
                        deletion.country
                    };
                    let mut row = entry(
                        &self.links,
                        id,
                        at,
                        kind,
                        status,
                        raw,
                        presentation.as_ref(),
                    );
                    if !labels {
                        row.tag = None;
                    }
                    if matches!(
                        deletion.cause,
                        Some(Cause::DivisionDestroyed | Cause::DivisionOverrun)
                    ) {
                        let matched = deaths.get(&id).map(Vec::as_slice).unwrap_or_default();
                        if let [death] = matched {
                            row.hour = death.hour;
                            row.by = false;
                            row.province = Some(death.province);
                            row.place_hour = Some(death.hour);
                            row.place_at_event = true;
                            row.killer_tag = labels
                                .then(|| self.links.get(death.killer_country).cloned())
                                .flatten();
                            row.killer_hour = row.killer_tag.as_ref().map(|_| at);
                            row.killer_division = death
                                .killer_division
                                .filter(|id| *id != 0)
                                .map(|id| id.to_string());
                        } else if matched.len() > 1 {
                            warn(
                                "Ambiguous DestroyedDivisions interval; killer not attributed",
                                1,
                            );
                        }
                    }
                    events.push(Projected {
                        event: Event { row, presentation },
                        candidate: None,
                        restored,
                    });
                }
                self.deletions = rows
                    .iter()
                    .map(|row| (row.division_handle, deletion_signature(row)))
                    .collect();
            }
        }
        if deletion_rows.is_none() {
            warn(
                "DivisionEquipmentDeletions unavailable; removal causes and totals incomplete",
                1,
            );
        }
        drop(cpu);
        let _cpu = ::replay_frame::cpu::scope("division log operands", None);
        if census && changed[DIVISIONS] {
            let witnessed: HashSet<u64> = deletion_rows
                .into_iter()
                .flatten()
                .map(|row| row.division_handle)
                .collect();
            let missing = lost.iter().filter(|id| !witnessed.contains(id)).count();
            if missing > 0 {
                warn(
                    "Division disappeared without a removal witness; no loss inferred",
                    missing,
                );
            }
        }
        // The row path builds its census presentations again when a table they read changed,
        // and when it holds none.
        let built = census && (changed[..DELETIONS].contains(&true) || self.built.is_none());
        if built {
            self.build(state, at, &divisions)?;
        } else if changed[DIVISIONS] || changed[STATUS] {
            if let Some(built) = &mut self.built {
                built.drifted = true;
            }
        }
        if census {
            self.census_hour = Some(at);
            self.census_template = self.template_hour;
        }
        self.previous_hour = Some(at);
        let projected = Hour {
            branch: hour.branch,
            hour: at,
            census,
            losses: deletion_rows.is_some(),
            template_read,
            built,
            events,
            warnings,
            names: None,
        };
        self.pending = Some(projected);
        Ok(())
    }

    fn end(&mut self, out: &mut dyn Sink<Hour>) -> io::Result<()> {
        // A census the walk built before the segment's last census hour named its
        // presentations then; one it never built is the restart's, which the walk before named.
        let last = self
            .pending
            .as_ref()
            .filter(|last| last.census)
            .map(|last| last.hour);
        let names = self
            .built
            .as_ref()
            .filter(|built| built.restored.is_none() && last.is_some_and(|last| built.hour != last))
            .zip(self.built_frame())
            .map(|(built, frame)| {
                let templates = frame
                    .complete::<tables::DivisionTemplates>()
                    .unwrap_or_default();
                let mut names = HashMap::new();
                for operands in built.operands.values() {
                    names.entry(operands.template).or_insert_with(|| {
                        super::keyed(templates, operands.template, |row| row.template)
                            .map(|_| TemplateName(display_naming(&*frame, operands.template)))
                    });
                }
                names
            });
        match self.pending.take() {
            Some(mut last) => {
                last.names = names;
                out.push(last)
            }
            None => Ok(()),
        }
    }

    fn carried(&self) -> Option<usize> {
        // The current frame's views are the naming views'.
        let built = self.built.as_ref().map_or(0, |built| {
            built.operands.capacity() * size_of::<(u64, Operands)>()
                + built.frame.as_ref().map_or(0, frame_heap_bytes)
        });
        Some(
            self.naming.heap_bytes()
                + self.removals.heap_bytes()
                + built
                + self.live.capacity() * size_of::<u64>()
                + self.deletions.capacity() * size_of::<(u64, (u32, u32, u8, Option<u8>))>(),
        )
    }
}
