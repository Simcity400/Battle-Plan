//! The division log projected from a columnar walk ([`replay::segments`]): each hour's
//! candidate first sightings, witnessed removals and warnings, read from the walker's typed
//! columns at the rows the hour wrote, with the presentations they carry, for
//! [`super::Scan::observe_projected`] to keep in order. The census is the walker's live
//! divisions, moved by the hour's changed slots when the last census was the hour before and
//! read whole otherwise. Each division's presentation operands are kept as the census that last
//! built them read them, as the row path keeps its presentations, moved by the rows the hour
//! wrote, and presentations are built only for the divisions an event names. Names and link
//! tags read the naming tables through the row path's own functions: the walk's shared frame
//! holds them by the fields they read ([`dashboard_world`]), and the projection keeps the
//! hour's views until the next hour that moves them, to name its census as its build did.
//!
//! A walk starts only where the divisions and their removal witnesses were read whole
//! ([`Operator::starts_at`]), so the census and witnesses before its first hour are the
//! restart's, and goes on into its branch's later runs, carrying them as the row path's scan
//! carries them across, resident or written whole ([`Operator::save`]). What only the hours
//! before a walk know is left for the ordered scan: the template read hour is dated
//! [`super::INHERITED`], and the presentations named from the restart's census are
//! marked with the hour that census is taken to have been built, for the ordered scan to name
//! again when it was built earlier ([`Hour::names`]).

use super::{
    ArmyName, DivisionLogKind, Event, IDENTITY_UNAVAILABLE, INHERITED, Presentation, TemplateName,
    TemplateNaming, deletion_signature, display_naming, disposition, entry,
};
use dashboard_text::metadata::{
    CountryKeys, LinkProjection, LinkTags, country_tables, link_slots, slot_read,
};
use dashboard_world::{Hold, Needs, World};
use replay::{
    columnar_frames::{Views, frame_heap_bytes, metrics},
    columns::{self, Status},
    segments::{self, Item, Operator, Sink, Start, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{
        DestroyedDivisionsRow, DivisionEquipmentDeletionRow, DivisionStatusRow,
        EquipmentDestructionCause as Cause, LandEventCoverageRow, RowCodec, TableId, TableRows,
        tables,
    },
    scope::CaptureMode,
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::{HashMap, HashSet},
    io,
    mem::size_of,
};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

/// The tables names, icons and link tags read.
const NAMING: [TableId; 4] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::DivisionTemplates,
    TableId::DivisionTemplateIcons,
];
const COUNTRIES: usize = 0;
const LINKS: usize = 1;

/// What names, icons and link tags read of each of [`NAMING`], as the walk's frame is asked
/// for them.
fn naming() -> io::Result<Vec<(TableId, Hold)>> {
    let read = [
        mask(TableId::Countries, &["tag", "canonical_slot"])?,
        mask(TableId::CountryLinks, &["canonical_slot"])?,
        mask(TableId::DivisionTemplates, &["name", "name_key"])?,
        // What a presentation and its template's name read of an icon row.
        mask(
            TableId::DivisionTemplateIcons,
            &[
                "priority_unit",
                "priority_unit_name",
                "custom_icon",
                "country",
                "display_name",
            ],
        )?,
    ];
    Ok(NAMING
        .into_iter()
        .zip(read)
        .map(|(table, fields)| (table, Hold::Fields(fields).of(table)))
        .collect())
}

/// The tables a projection reads.
pub const TABLES: &[TableId] = &super::TABLES;

/// What a projection reads of its walk's world: the naming tables by the fields it reads.
pub fn needs() -> io::Result<Needs> {
    Ok(Needs {
        frame: naming()?,
        ..Needs::default()
    })
}

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

/// A removal witness's signature ([`deletion_signature`]).
type Signature = (u32, u32, u8, Option<u8>);

/// One event of a projected hour: a candidate first sighting names its division. An event
/// presented from the restart's census names the hour that census is taken to have been
/// built.
#[derive(Serialize, Deserialize)]
pub(super) struct Projected {
    pub(super) event: Event,
    pub(super) candidate: Option<u64>,
    pub(super) restored: Option<i32>,
}

/// One projected hour.
#[derive(Serialize, Deserialize)]
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

impl Operands {
    fn heap(&self) -> usize {
        self.status.as_ref().map_or(0, army_name_heap)
    }
}

fn army_name_heap(name: &ArmyName) -> usize {
    let text = match &name.name {
        Ok(Some(bytes)) => bytes.capacity(),
        _ => 0,
    };
    let operands = match &name.naming {
        Ok(Some((.., custom, group))) => custom.capacity() + group.capacity(),
        _ => 0,
    };
    text + operands
}

fn template_name_heap(name: &TemplateName) -> usize {
    match &name.0 {
        Err(reason) => reason.capacity(),
        Ok(TemplateNaming::Text(text)) => text.capacity(),
        Ok(TemplateNaming::Localised { tag, key }) => tag.capacity() + key.capacity(),
    }
}

/// What a presentation reads of its template through the naming tables: its name, and the
/// icon it is drawn from.
#[derive(Clone)]
struct Named {
    template: Option<TemplateName>,
    icon: Option<(u32, i32, Vec<u8>)>,
}

/// Template `template`'s name as `frame` names it at its hour, when a complete DivisionTemplates
/// holds it.
fn template_name(frame: &DecodedFrame, template: u64) -> Option<TemplateName> {
    let templates = frame
        .complete::<tables::DivisionTemplates>()
        .unwrap_or_default();
    super::keyed(templates, template, |row| row.template)
        .map(|_| TemplateName(display_naming(frame, template)))
}

impl Named {
    /// Template `template` as `frame` names and draws it at its hour, as the row path's census
    /// reads it.
    fn of(frame: &DecodedFrame, template: u64) -> Self {
        let icons = frame
            .complete::<tables::DivisionTemplateIcons>()
            .unwrap_or_default();
        Self {
            template: template_name(frame, template),
            icon: super::keyed(icons, template, |row| row.template).map(|row| {
                (
                    row.priority_unit,
                    row.custom_icon,
                    row.priority_unit_name.clone(),
                )
            }),
        }
    }

    /// Division `id`'s presentation from `operands`, dated `hour`.
    fn present(
        self,
        id: u64,
        operands: &Operands,
        hour: i32,
        template_hour: Option<i32>,
    ) -> Presentation {
        Presentation {
            hour,
            division: id,
            province: operands.province,
            template_id: operands.template,
            status: operands.status.clone(),
            template: self.template,
            template_hour,
            icon: self.icon,
        }
    }

    fn heap(&self) -> usize {
        self.template.as_ref().map_or(0, template_name_heap)
            + self.icon.as_ref().map_or(0, |(.., name)| name.capacity())
    }
}

/// The columns a projection reads by name, and the fields it follows as change-mask bits.
struct Fields {
    province: usize,
    template: usize,
    reveal_skip: usize,
    owners: [usize; 2],
    /// Divisions' reveal-skip byte.
    shown: u64,
    /// Divisions' province and template.
    placed: u64,
    /// DivisionStatus's name operands.
    names: u64,
    /// What names, icons and link tags read of [`NAMING`], as the walk's frame is asked.
    naming: Vec<(TableId, Hold)>,
}

/// `fields` of `table` as change-mask bits.
fn mask(table: TableId, fields: &[&str]) -> io::Result<u64> {
    fields.iter().try_fold(0, |mask, &name| {
        let field = columns::field(table, name)?;
        let bit = u32::try_from(field)
            .ok()
            .and_then(|field| 1_u64.checked_shl(field))
            .ok_or_else(|| invalid(format!("{table:?}.{name} has no change-mask bit")))?;
        Ok(mask | bit)
    })
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

/// Whether the hour changed `table`'s rows.
fn touched(state: &State, table: TableId) -> io::Result<bool> {
    let changes = columns::changes(state, table)?;
    Ok(!changes.inserted.is_empty()
        || !changes.deleted.is_empty()
        || changes.truncated
        || !changes.masks.is_empty()
        || !changes.fill_masks.is_empty())
}

/// The slots of `table` the hour changed or filled in any of `fields`, and those it
/// inserted.
fn written(
    state: &State,
    table: TableId,
    fields: u64,
) -> io::Result<impl Iterator<Item = u32> + '_> {
    let changes = columns::changes(state, table)?;
    Ok(changes
        .changed
        .iter()
        .zip(&changes.masks)
        .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
        .filter(move |&(_, &mask)| mask & fields != 0)
        .map(|(&slot, _)| slot)
        .chain(changes.inserted.iter().copied()))
}

/// Every live row of `table`, built from its columns.
fn every<R: RowCodec>(state: &State, table: TableId, format: u16) -> io::Result<Vec<R>> {
    let held = columns::table(state, table)?;
    held.live()
        .ones()
        .map(|slot| columns::row(state, table, slot, format))
        .collect()
}

/// The census presentations, as the census that last built them read them.
struct Built {
    hour: i32,
    /// Its templates' names and icons as its build named them, once what naming reads has
    /// changed since: until then the naming tables name them alike.
    kept: Option<HashMap<u64, Named>>,
    /// Every Divisions row's operands then, and whether DivisionStatus held every row.
    operands: HashMap<u64, Operands>,
    status: bool,
    /// Whether Divisions or DivisionStatus changed since, at an hour that built nothing.
    drifted: bool,
    /// For the restart's census: the hour it is taken to have been built.
    restored: Option<i32>,
}

impl Built {
    fn heap(&self) -> usize {
        self.operands.capacity() * size_of::<(u64, Operands)>()
            + self.operands.values().map(Operands::heap).sum::<usize>()
            + self.kept.as_ref().map_or(0, |kept| {
                kept.capacity() * size_of::<(u64, Named)>()
                    + kept.values().map(Named::heap).sum::<usize>()
            })
    }
}

const KEPT: &str = "The census keeps no name for a template it names";

/// One segment's projection, and the segments after it in its branch it goes on through.
pub struct Projector {
    format: FormatVersion,
    mode: CaptureMode,
    fields: Fields,
    /// The naming tables as the last hour that moved them left them, dated as they name.
    named: Option<DecodedFrame>,
    built: Option<Built>,
    links: LinkTags,
    /// Whether the link tags were last read from complete country tables.
    linked: bool,
    /// The last census's divisions and hour.
    live: HashSet<u64>,
    census_hour: Option<i32>,
    deletions: HashMap<u64, Signature>,
    template_hour: Option<i32>,
    /// The template read hour at the last census.
    census_template: Option<i32>,
    previous_hour: Option<i32>,
    /// Each followed table's status the hour before, its missing rows and unavailable reason
    /// included: the row path's views differ at an hour any of them changes.
    shapes: [Option<Status>; FOLLOWED.len()],
    /// A DivisionStatus row that division names are read into.
    names: TableRows,
    /// The hour projected last, held back while the segment's end would name its templates.
    pending: Option<Hour>,
    #[cfg(test)]
    names_read: usize,
}

impl Projector {
    pub fn new(format: FormatVersion, mode: CaptureMode) -> io::Result<Self> {
        let field = |table, name| columns::field(table, name);
        Ok(Self {
            format,
            mode,
            fields: Fields {
                province: field(TableId::Divisions, "province")?,
                template: field(TableId::Divisions, "template")?,
                reveal_skip: field(TableId::Divisions, "reveal_skip")?,
                owners: [
                    field(TableId::Divisions, "roster_owner")?,
                    field(TableId::Divisions, "expeditionary_owner")?,
                ],
                shown: mask(TableId::Divisions, &["reveal_skip"])?,
                placed: mask(TableId::Divisions, &["province", "template"])?,
                names: mask(TableId::DivisionStatus, &["name", "naming"])?,
                naming: naming()?,
            },
            named: None,
            built: None,
            links: LinkTags::default(),
            linked: false,
            live: HashSet::new(),
            census_hour: None,
            deletions: HashMap::new(),
            template_hour: Some(INHERITED),
            census_template: Some(INHERITED),
            previous_hour: None,
            shapes: [None; FOLLOWED.len()],
            names: TableRows::DivisionStatus(vec![DivisionStatusRow::default()]),
            pending: None,
            #[cfg(test)]
            names_read: 0,
        })
    }

    /// The division names it has read.
    #[cfg(test)]
    pub(super) fn reads(&self) -> usize {
        self.names_read
    }

    /// The naming tables dated `hour`, the hour their names are dated.
    fn dated(&mut self, hour: i32) -> io::Result<&DecodedFrame> {
        let named = self
            .named
            .as_mut()
            .ok_or_else(|| invalid("The naming tables are unread"))?;
        named.hour = hour;
        Ok(named)
    }

    /// The name operands of division `id` now, read from DivisionStatus's name columns alone;
    /// None when DivisionStatus is not complete or holds no row for it.
    fn status(&mut self, state: &State, id: u64) -> io::Result<Option<ArmyName>> {
        if !columns::complete(state, TableId::DivisionStatus) {
            return Ok(None);
        }
        let key = u128::from(id);
        let Some(slot) = columns::table(state, TableId::DivisionStatus)?.slot(key) else {
            return Ok(None);
        };
        let TableRows::DivisionStatus(rows) = &mut self.names else {
            return Err(invalid("Division names are read into another table's row"));
        };
        let row = rows
            .first_mut()
            .ok_or_else(|| invalid("Division names are read into no row"))?;
        row.division = id;
        columns::patch(
            state,
            TableId::DivisionStatus,
            &mut self.names,
            &[(key, slot, self.fields.names)],
            self.format.version(),
        )?;
        #[cfg(test)]
        {
            self.names_read += 1;
        }
        match &self.names {
            TableRows::DivisionStatus(rows) => Ok(rows.first().map(ArmyName::new)),
            _ => Err(invalid("Division names are read into another table's row")),
        }
    }

    /// Every Divisions row's operands now.
    fn operands(
        &mut self,
        state: &State,
        divisions: &Divisions<'_>,
    ) -> io::Result<HashMap<u64, Operands>> {
        let table = columns::table(state, TableId::Divisions)?;
        let mut operands = HashMap::with_capacity(table.live().count());
        for slot in table.live().ones() {
            if let Some(id) = table.key(slot).map(|key| key as u64) {
                let name = self.status(state, id)?;
                operands.insert(id, divisions.operands(slot, name));
            }
        }
        Ok(operands)
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
                written(state, TableId::Divisions, self.fields.shown)?
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

    /// The census presentations built again this hour: moved by the rows the hour wrote when
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
                let changes = columns::changes(state, TableId::Divisions)?;
                for &key in &changes.deleted_keys {
                    operands.remove(&(key as u64));
                }
                let mut inserted = Vec::with_capacity(changes.inserted.len());
                for &slot in &changes.inserted {
                    if let Some(id) = table.key(slot).map(|key| key as u64) {
                        let name = self.status(state, id)?;
                        operands.insert(id, divisions.operands(slot, name));
                        inserted.push(id);
                    }
                }
                for slot in written(state, TableId::Divisions, self.fields.placed)? {
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
                    written(state, TableId::DivisionStatus, self.fields.names)?
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
                    // A division the hour inserted was named as it was read.
                    if operands.contains_key(&id) && !inserted.contains(&id) {
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
            kept: None,
            operands,
            status,
            drifted: false,
            restored: None,
        });
        Ok(())
    }

    /// Template `template` as the census's build named and drew it.
    fn built_named(&mut self, template: u64) -> io::Result<Named> {
        let built = self
            .built
            .as_ref()
            .ok_or_else(|| invalid("No census names templates"))?;
        match &built.kept {
            Some(kept) => kept.get(&template).cloned().ok_or_else(|| invalid(KEPT)),
            None => {
                let hour = built.hour;
                Ok(Named::of(self.dated(hour)?, template))
            }
        }
    }

    /// Division `id`'s presentation as the last census built it, dated that census, and the
    /// hour a restart's census is taken to have been built.
    fn census_presentation(&mut self, id: u64) -> io::Result<Option<(Presentation, Option<i32>)>> {
        let Some(built) = &self.built else {
            return Ok(None);
        };
        let Some(held) = built.operands.get(&id) else {
            return Ok(None);
        };
        let Some(hour) = self.census_hour else {
            return Ok(None);
        };
        let held = Operands {
            province: held.province,
            template: held.template,
            status: held.status.clone().filter(|_| built.status),
        };
        let restored = built.restored;
        let named = self.built_named(held.template)?;
        Ok(Some((
            named.present(id, &held, hour, self.census_template),
            restored,
        )))
    }

    /// Division `id`'s presentation from `operands`, named and dated this hour.
    fn present(&mut self, at: i32, id: u64, operands: &Operands) -> io::Result<Presentation> {
        let template_hour = self.template_hour;
        Ok(Named::of(self.dated(at)?, operands.template).present(id, operands, at, template_hour))
    }

    /// The census keeps its templates' names and icons as its build named them, before the
    /// naming tables move past it.
    fn keep_naming(&mut self) {
        let (Some(built), Some(frame)) = (&mut self.built, &mut self.named) else {
            return;
        };
        if built.kept.is_some() {
            return;
        }
        frame.hour = built.hour;
        let frame = &*frame;
        let mut kept = HashMap::new();
        for operands in built.operands.values() {
            kept.entry(operands.template)
                .or_insert_with(|| Named::of(frame, operands.template));
        }
        built.kept = Some(kept);
    }

    /// The name of each template the census names, as its build named them.
    fn names(&mut self) -> io::Result<HashMap<u64, Option<TemplateName>>> {
        let Some(built) = &self.built else {
            return Ok(HashMap::new());
        };
        let mut templates: Vec<u64> = built.operands.values().map(|held| held.template).collect();
        templates.sort_unstable();
        templates.dedup();
        let mut names = HashMap::with_capacity(templates.len());
        for template in templates {
            let name = match &built.kept {
                Some(kept) => kept
                    .get(&template)
                    .ok_or_else(|| invalid(KEPT))?
                    .template
                    .clone(),
                None => {
                    let frame = self
                        .named
                        .as_mut()
                        .ok_or_else(|| invalid("The naming tables are unread"))?;
                    frame.hour = built.hour;
                    template_name(frame, template)
                }
            };
            names.insert(template, name);
        }
        Ok(names)
    }

    /// The removal witnesses whose signature differs from the one held for their division,
    /// in key order, each held now: every row's when `whole`, else those of the rows the hour
    /// wrote, the others holding the hour before's.
    fn removed(
        &mut self,
        state: &State,
        whole: bool,
    ) -> io::Result<Vec<DivisionEquipmentDeletionRow>> {
        let table = columns::table(state, TableId::DivisionEquipmentDeletions)?;
        let changes = columns::changes(state, TableId::DivisionEquipmentDeletions)?;
        let mut slots = Vec::new();
        if whole {
            for slot in table.live().ones() {
                let key = table
                    .key(slot)
                    .ok_or_else(|| invalid("A live slot holds no key"))?;
                slots.push((key, slot));
            }
        } else {
            for &slot in changes
                .changed
                .iter()
                .chain(&changes.fill_slots)
                .chain(&changes.inserted)
            {
                if let Some(key) = table.key(slot) {
                    slots.push((key, slot));
                }
            }
        }
        slots.sort_unstable();
        slots.dedup();
        let format = self.format.version();
        let mut rows = Vec::with_capacity(slots.len());
        for &(_, slot) in &slots {
            rows.push(columns::row::<DivisionEquipmentDeletionRow>(
                state,
                TableId::DivisionEquipmentDeletions,
                slot,
                format,
            )?);
        }
        if whole {
            let held = std::mem::replace(
                &mut self.deletions,
                rows.iter()
                    .map(|row| (row.division_handle, deletion_signature(row)))
                    .collect(),
            );
            rows.retain(|row| held.get(&row.division_handle) != Some(&deletion_signature(row)));
        } else {
            rows.retain(|row| {
                let signature = deletion_signature(row);
                self.deletions.insert(row.division_handle, signature) != Some(signature)
            });
            for &key in &changes.deleted_keys {
                if table.slot(key).is_none() {
                    self.deletions.remove(&(key as u64));
                }
            }
        }
        Ok(rows)
    }

    /// The deaths of each of `divisions` since the hour before that a complete interval
    /// covers, by division, as the row path joins them.
    fn deaths(
        &self,
        state: &State,
        at: i32,
        divisions: &[u64],
    ) -> io::Result<HashMap<u64, Vec<DestroyedDivisionsRow>>> {
        let mut deaths: HashMap<u64, Vec<DestroyedDivisionsRow>> = HashMap::new();
        if !columns::complete(state, TableId::DestroyedDivisions) {
            return Ok(deaths);
        }
        let format = self.format.version();
        let coverage: Option<Vec<LandEventCoverageRow>> =
            columns::complete(state, TableId::LandEventCoverage)
                .then(|| every(state, TableId::LandEventCoverage, format))
                .transpose()?;
        for death in every::<DestroyedDivisionsRow>(state, TableId::DestroyedDivisions, format)? {
            if death.hour <= at
                && self.previous_hour.is_some_and(|last| death.hour >= last)
                && coverage.as_ref().is_some_and(|coverage| {
                    coverage.iter().any(|span| {
                        span.complete && span.from_hour < death.hour && span.to_hour >= death.hour
                    })
                })
                && divisions.contains(&death.division)
            {
                deaths.entry(death.division).or_default().push(death);
            }
        }
        Ok(deaths)
    }
}

/// Whether a removal names its division's killer when a covered death matches it.
fn killed(row: &DivisionEquipmentDeletionRow) -> bool {
    matches!(
        row.cause,
        Some(Cause::DivisionDestroyed | Cause::DivisionOverrun)
    )
}

impl Operator<World> for Projector {
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
        let _cpu = ::replay_frame::cpu::scope("division log restart", None);
        let before = first
            .checked_sub(1)
            .ok_or_else(|| invalid("A walk starts at no hour"))?;
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
        // The restart's naming tables name its census: read once, before the walk's hours.
        self.named =
            Some(Views::new(&NAMING, self.format, self.mode)?.restored(state, branch, before)?);
        let operands = self.operands(state, &divisions)?;
        self.built = Some(Built {
            hour: before,
            kept: None,
            operands,
            status: columns::complete(state, TableId::DivisionStatus),
            drifted: false,
            restored: Some(before),
        });
        for row in every::<DivisionEquipmentDeletionRow>(
            state,
            TableId::DivisionEquipmentDeletions,
            self.format.version(),
        )? {
            self.deletions
                .insert(row.division_handle, deletion_signature(&row));
        }
        Ok(())
    }

    fn cross(&mut self, first: i32) -> io::Result<()> {
        // The walker's tables start empty: the census and naming tables are read whole again.
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

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        // The hour before is whole: it reaches the emit even if this hour refuses.
        if let Some(before) = self.pending.take() {
            out.push(before)?;
        }
        let state = hour.state;
        let at = hour.hour;
        let mut warnings: Vec<(String, usize)> = Vec::new();
        // Each followed table's rows or status changed, as the row path's change facts say:
        // every table at a segment's first hour, as at a row block's.
        let mut changed = [false; FOLLOWED.len()];
        let mut reshaped = [false; FOLLOWED.len()];
        for (index, &table) in FOLLOWED.iter().enumerate() {
            let now = Some(columns::status(state, table));
            reshaped[index] = std::mem::replace(&mut self.shapes[index], now) != now;
            changed[index] = hour.opens || reshaped[index] || touched(state, table)?;
        }
        let census = columns::complete(state, TableId::Divisions);
        // The row path builds its census presentations again when a table they read changed,
        // and when it holds none.
        let built = census && (changed[..DELETIONS].contains(&true) || self.built.is_none());
        let losses = columns::complete(state, TableId::DivisionEquipmentDeletions);

        let cpu = ::replay_frame::cpu::scope("division log removals", None);
        let removed = if changed[DELETIONS] && losses {
            self.removed(state, hour.opens || reshaped[DELETIONS])?
        } else {
            Vec::new()
        };
        // Named as the census's build named them, before the naming tables move.
        let mut presented = Vec::with_capacity(removed.len());
        for row in &removed {
            presented.push(self.census_presentation(row.division_handle)?);
        }
        drop(cpu);

        let cpu = ::replay_frame::cpu::scope("division log naming", None);
        // The walk's first hour, and an hour any read field or row of a table moved, moves
        // the naming tables.
        let unread = self.named.is_none();
        let moves: [bool; NAMING.len()] = std::array::from_fn(|at| {
            let (table, hold) = self.fields.naming[at];
            let fields = match hold {
                Hold::Whole => u64::MAX,
                Hold::Fields(fields) => fields,
            };
            unread || hour.opens || hour.world.touched(table, fields)
        });
        if !built && (hour.opens || moves.contains(&true)) {
            self.keep_naming();
        }
        if moves.contains(&true) {
            self.named = Some(hour.world.frame_held(state, &self.fields.naming)?);
        }
        let labels = columns::complete(state, TableId::Countries)
            && columns::complete(state, TableId::CountryLinks);
        if !labels {
            // The tags stay held, read again from the next complete tables.
            let _ = self.links.update_projected(None, None);
            self.linked = false;
            warnings.push((IDENTITY_UNAVAILABLE.into(), 1));
        } else if !self.linked || moves[COUNTRIES] || moves[LINKS] {
            let named = self
                .named
                .as_ref()
                .ok_or_else(|| invalid("The naming tables are unread"))?;
            let (countries, links) = country_tables(named)?;
            self.links.update_projected(
                Some(&CountryKeys::read(countries)),
                Some(&link_slots(links)),
            )?;
            self.linked = true;
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

        let cpu = ::replay_frame::cpu::scope("division log census", None);
        let mut events = Vec::new();
        let divisions = Divisions::new(state, &self.fields)?;
        let table = columns::table(state, TableId::Divisions)?;
        let mut lost = Vec::new();
        if census && changed[DIVISIONS] {
            let (gained, gone) = self.census(state, at, hour.starts, &divisions)?;
            lost = gone;
            for (id, slot) in gained {
                // A division absent from the last census; the ordered scan keeps it once.
                let name = self.status(state, id)?;
                let operands = divisions.operands(slot, name);
                let presentation = self.present(at, id, &operands)?;
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
            warnings.push((
                "Divisions unavailable or incomplete; appearances unknown".into(),
                1,
            ));
        }
        drop(cpu);

        let cpu = ::replay_frame::cpu::scope("division log removals", None);
        let destroyed: Vec<u64> = removed
            .iter()
            .filter(|row| killed(row))
            .map(|row| row.division_handle)
            .collect();
        let deaths = if destroyed.is_empty() {
            HashMap::new()
        } else {
            self.deaths(state, at, &destroyed)?
        };
        for (deletion, presented) in removed.iter().zip(presented) {
            let id = deletion.division_handle;
            // The last census's presentation, else this hour's.
            let (presentation, restored) = match presented {
                Some((presentation, restored)) => (Some(presentation), restored),
                None => match table.slot(u128::from(id)).filter(|_| census) {
                    Some(slot) => {
                        let name = self.status(state, id)?;
                        let operands = divisions.operands(slot, name);
                        (Some(self.present(at, id, &operands)?), None)
                    }
                    None => (None, None),
                },
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
            if killed(deletion) {
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
                    warnings.push((
                        "Ambiguous DestroyedDivisions interval; killer not attributed".into(),
                        1,
                    ));
                }
            }
            events.push(Projected {
                event: Event { row, presentation },
                candidate: None,
                restored,
            });
        }
        if !losses {
            warnings.push((
                "DivisionEquipmentDeletions unavailable; removal causes and totals incomplete"
                    .into(),
                1,
            ));
        }
        drop(cpu);

        let _cpu = ::replay_frame::cpu::scope("division log operands", None);
        if census && changed[DIVISIONS] {
            let witnesses = columns::table(state, TableId::DivisionEquipmentDeletions)?;
            let missing = lost
                .iter()
                .filter(|&&id| !losses || witnesses.slot(u128::from(id)).is_none())
                .count();
            if missing > 0 {
                warnings.push((
                    "Division disappeared without a removal witness; no loss inferred".into(),
                    missing,
                ));
            }
        }
        if built {
            self.build(state, at, &divisions)?;
        } else if (changed[DIVISIONS] || changed[STATUS])
            && let Some(held) = &mut self.built
        {
            held.drifted = true;
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
            losses,
            template_read,
            built,
            events,
            warnings,
            names: None,
        };
        // Held back while the segment's end would name the census's templates.
        if census
            && self
                .built
                .as_ref()
                .is_some_and(|held| held.restored.is_none() && held.hour != at)
        {
            self.pending = Some(projected);
            Ok(())
        } else {
            out.push(projected)
        }
    }

    fn end(&mut self, out: &mut dyn Sink<Hour>) -> io::Result<()> {
        // A census the walk built before the segment's last census hour named its
        // presentations then; one it never built is the restart's, which the walk before named.
        let Some(mut last) = self.pending.take() else {
            return Ok(());
        };
        last.names = Some(self.names()?);
        out.push(last)
    }

    fn carried(&self) -> Option<usize> {
        Some(
            self.built.as_ref().map_or(0, Built::heap)
                + self.named.as_ref().map_or(0, frame_heap_bytes)
                + self.links.heap_bytes()
                + self.live.capacity() * size_of::<u64>()
                + self.deletions.capacity() * size_of::<(u64, Signature)>()
                + self.names.heap_bytes()
                + self.pending.as_ref().map_or(0, |pending| pending.heap()),
        )
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool> {
        let mut write = Writer(out);
        for hour in [
            self.previous_hour,
            self.census_hour,
            self.template_hour,
            self.census_template,
        ] {
            write.hour(hour);
        }
        for &shape in &self.shapes {
            write.status(shape);
        }
        write.json(&(&self.links, self.links.projection()))?;
        let mut live: Vec<u64> = self.live.iter().copied().collect();
        live.sort_unstable();
        write.len(live.len());
        for id in live {
            write.u64(id);
        }
        let mut deletions: Vec<(u64, Signature)> = self
            .deletions
            .iter()
            .map(|(&id, &held)| (id, held))
            .collect();
        deletions.sort_unstable();
        write.len(deletions.len());
        for (id, (country, owner, disposition, cause)) in deletions {
            write.u64(id);
            write.u32(country);
            write.u32(owner);
            write.u8(disposition);
            write.optional(cause);
        }
        match &self.built {
            None => write.u8(0),
            Some(built) => {
                write.u8(1);
                write.i32(built.hour);
                write.flag(built.status);
                write.flag(built.drifted);
                write.hour(built.restored);
                let mut operands: Vec<_> = built.operands.iter().collect();
                operands.sort_unstable_by_key(|&(&id, _)| id);
                write.len(operands.len());
                for (&id, held) in operands {
                    write.u64(id);
                    write.u32(held.province);
                    write.u64(held.template);
                    match &held.status {
                        None => write.u8(0),
                        Some(name) => {
                            write.u8(1);
                            write.army_name(name);
                        }
                    }
                }
                // Read back, the naming tables are read anew: the census keeps its names.
                let mut templates: Vec<u64> =
                    built.operands.values().map(|held| held.template).collect();
                templates.sort_unstable();
                templates.dedup();
                write.len(templates.len());
                let mut frame = None;
                for template in templates {
                    write.u64(template);
                    match &built.kept {
                        Some(kept) => {
                            write.named(kept.get(&template).ok_or_else(|| invalid(KEPT))?)
                        }
                        None => {
                            let frame = match &mut frame {
                                Some(frame) => frame,
                                held => held.insert({
                                    let mut frame = self
                                        .named
                                        .clone()
                                        .ok_or_else(|| invalid("The naming tables are unread"))?;
                                    frame.hour = built.hour;
                                    frame
                                }),
                            };
                            write.named(&Named::of(frame, template));
                        }
                    }
                }
            }
        }
        write.json(&self.pending)?;
        Ok(true)
    }

    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut read = Reader(bytes);
        let [previous_hour, census_hour, template_hour, census_template] =
            [read.hour()?, read.hour()?, read.hour()?, read.hour()?];
        let mut shapes = [None; FOLLOWED.len()];
        for shape in &mut shapes {
            *shape = read.status()?;
        }
        let (links, projection): (LinkTags, Option<LinkProjection>) = read.json()?;
        let linked = projection.is_some();
        let links = match projection {
            Some(projection) => LinkTags::from_projection(projection),
            None => links,
        };
        let count = read.len()?;
        let mut live = HashSet::with_capacity(count);
        for _ in 0..count {
            live.insert(read.u64()?);
        }
        let count = read.len()?;
        let mut deletions = HashMap::with_capacity(count);
        for _ in 0..count {
            let id = read.u64()?;
            let signature = (read.u32()?, read.u32()?, read.u8()?, read.optional()?);
            deletions.insert(id, signature);
        }
        let built = match read.u8()? {
            0 => None,
            1 => {
                let hour = read.i32()?;
                let status = read.flag()?;
                let drifted = read.flag()?;
                let restored = read.hour()?;
                let count = read.len()?;
                let mut operands = HashMap::with_capacity(count);
                for _ in 0..count {
                    let id = read.u64()?;
                    let province = read.u32()?;
                    let template = read.u64()?;
                    let status = match read.u8()? {
                        0 => None,
                        1 => Some(read.army_name()?),
                        _ => {
                            return Err(invalid(
                                "A saved division name is neither held nor absent",
                            ));
                        }
                    };
                    operands.insert(
                        id,
                        Operands {
                            province,
                            template,
                            status,
                        },
                    );
                }
                let count = read.len()?;
                let mut kept = HashMap::with_capacity(count);
                for _ in 0..count {
                    let template = read.u64()?;
                    kept.insert(template, read.named()?);
                }
                Some(Built {
                    hour,
                    kept: Some(kept),
                    operands,
                    status,
                    drifted,
                    restored,
                })
            }
            _ => return Err(invalid("A saved census is neither held nor absent")),
        };
        let pending: Option<Hour> = read.json()?;
        if !read.0.is_empty() {
            return Err(invalid("A saved division log projection runs past its end"));
        }
        self.previous_hour = previous_hour;
        self.census_hour = census_hour;
        self.template_hour = template_hour;
        self.census_template = census_template;
        self.shapes = shapes;
        self.links = links;
        self.linked = linked;
        self.live = live;
        self.deletions = deletions;
        self.built = built;
        self.pending = pending;
        self.named = None;
        Ok(())
    }
}

/// What [`Operator::save`] writes: little-endian numbers, lengths before what they count, and
/// every collection in key order.
struct Writer<'a>(&'a mut Vec<u8>);

impl Writer<'_> {
    fn u8(&mut self, value: u8) {
        self.0.push(value);
    }

    fn flag(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    fn u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn i32(&mut self, value: i32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn len(&mut self, len: usize) {
        self.u64(len as u64);
    }

    fn bytes(&mut self, bytes: &[u8]) {
        self.len(bytes.len());
        self.0.extend_from_slice(bytes);
    }

    fn optional(&mut self, value: Option<u8>) {
        match value {
            Some(value) => {
                self.u8(1);
                self.u8(value);
            }
            None => self.u8(0),
        }
    }

    fn hour(&mut self, hour: Option<i32>) {
        match hour {
            Some(hour) => {
                self.u8(1);
                self.i32(hour);
            }
            None => self.u8(0),
        }
    }

    fn status(&mut self, status: Option<Status>) {
        match status {
            None => self.u8(0),
            Some(Status::NotRecorded) => self.u8(1),
            Some(Status::Complete) => self.u8(2),
            Some(Status::Truncated { missing }) => {
                self.u8(3);
                self.u32(missing);
            }
            Some(Status::Unavailable { reason }) => {
                self.u8(4);
                self.u32(reason);
            }
        }
    }

    fn army_name(&mut self, name: &ArmyName) {
        self.u64(name.division);
        match &name.name {
            Err(()) => self.u8(0),
            Ok(None) => self.u8(1),
            Ok(Some(text)) => {
                self.u8(2);
                self.bytes(text);
            }
        }
        match &name.naming {
            Err(()) => self.u8(0),
            Ok(None) => self.u8(1),
            Ok(Some((kind, order, ordered, programmatic, custom, group))) => {
                self.u8(2);
                self.i32(*kind);
                self.i32(*order);
                self.u8(*ordered);
                self.u8(*programmatic);
                self.bytes(custom);
                self.bytes(group);
            }
        }
    }

    fn named(&mut self, named: &Named) {
        match &named.template {
            None => self.u8(0),
            Some(TemplateName(Err(reason))) => {
                self.u8(1);
                self.bytes(reason.as_bytes());
            }
            Some(TemplateName(Ok(TemplateNaming::Text(text)))) => {
                self.u8(2);
                self.bytes(text.as_bytes());
            }
            Some(TemplateName(Ok(TemplateNaming::Localised { tag, key }))) => {
                self.u8(3);
                self.bytes(tag.as_bytes());
                self.bytes(key.as_bytes());
            }
        }
        match &named.icon {
            None => self.u8(0),
            Some((unit, custom, name)) => {
                self.u8(1);
                self.u32(*unit);
                self.i32(*custom);
                self.bytes(name);
            }
        }
    }

    fn json(&mut self, value: &impl Serialize) -> io::Result<()> {
        let bytes = serde_json::to_vec(value)?;
        self.bytes(&bytes);
        Ok(())
    }
}

/// What [`Writer`] wrote, read back.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> io::Result<&'a [u8]> {
        if count > self.0.len() {
            return Err(invalid("A saved division log projection ends early"));
        }
        let (taken, rest) = self.0.split_at(count);
        self.0 = rest;
        Ok(taken)
    }

    fn array<const N: usize>(&mut self) -> io::Result<[u8; N]> {
        let mut bytes = [0; N];
        bytes.copy_from_slice(self.take(N)?);
        Ok(bytes)
    }

    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.array::<1>()?[0])
    }

    fn flag(&mut self) -> io::Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(invalid("A saved flag is neither set nor clear")),
        }
    }

    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn i32(&mut self) -> io::Result<i32> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    /// A count of items each saved in a byte or more, so never more than the bytes left.
    fn len(&mut self) -> io::Result<usize> {
        usize::try_from(self.u64()?)
            .ok()
            .filter(|&len| len <= self.0.len())
            .ok_or_else(|| invalid("A saved division log projection counts past its end"))
    }

    fn bytes(&mut self) -> io::Result<Vec<u8>> {
        let len = self.len()?;
        Ok(self.take(len)?.to_vec())
    }

    fn text(&mut self) -> io::Result<String> {
        String::from_utf8(self.bytes()?).map_err(|_| invalid("A saved name is not text"))
    }

    fn optional(&mut self) -> io::Result<Option<u8>> {
        Ok(match self.u8()? {
            0 => None,
            1 => Some(self.u8()?),
            _ => return Err(invalid("A saved value is neither held nor absent")),
        })
    }

    fn hour(&mut self) -> io::Result<Option<i32>> {
        Ok(match self.u8()? {
            0 => None,
            1 => Some(self.i32()?),
            _ => return Err(invalid("A saved hour is neither held nor absent")),
        })
    }

    fn status(&mut self) -> io::Result<Option<Status>> {
        Ok(match self.u8()? {
            0 => None,
            1 => Some(Status::NotRecorded),
            2 => Some(Status::Complete),
            3 => Some(Status::Truncated {
                missing: self.u32()?,
            }),
            4 => Some(Status::Unavailable {
                reason: self.u32()?,
            }),
            _ => return Err(invalid("A saved table status is unknown")),
        })
    }

    fn army_name(&mut self) -> io::Result<ArmyName> {
        let division = self.u64()?;
        let name = match self.u8()? {
            0 => Err(()),
            1 => Ok(None),
            2 => Ok(Some(self.bytes()?)),
            _ => return Err(invalid("A saved division name is unknown")),
        };
        let naming = match self.u8()? {
            0 => Err(()),
            1 => Ok(None),
            2 => Ok(Some((
                self.i32()?,
                self.i32()?,
                self.u8()?,
                self.u8()?,
                self.bytes()?,
                self.bytes()?,
            ))),
            _ => return Err(invalid("A saved division naming is unknown")),
        };
        Ok(ArmyName {
            division,
            name,
            naming,
        })
    }

    fn named(&mut self) -> io::Result<Named> {
        let template = match self.u8()? {
            0 => None,
            1 => Some(TemplateName(Err(self.text()?))),
            2 => Some(TemplateName(Ok(TemplateNaming::Text(self.text()?)))),
            3 => Some(TemplateName(Ok(TemplateNaming::Localised {
                tag: self.text()?,
                key: self.text()?,
            }))),
            _ => return Err(invalid("A saved template name is unknown")),
        };
        let icon = match self.u8()? {
            0 => None,
            1 => Some((self.u32()?, self.i32()?, self.bytes()?)),
            _ => return Err(invalid("A saved template icon is neither held nor absent")),
        };
        Ok(Named { template, icon })
    }

    fn json<T: DeserializeOwned>(&mut self) -> io::Result<T> {
        let len = self.len()?;
        Ok(serde_json::from_slice(self.take(len)?)?)
    }
}
