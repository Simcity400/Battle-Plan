
use recorder_derive_framework::framework::{Hour, columns};
use replay_columnar::store::{Kept, TableView, Value as Held};
use replay_data::{
    format::FormatVersion,
    rows::{CountryLinkRow, DestroyedDivisionsRow, DivisionEquipmentDeletionRow, DivisionRow, DivisionStatusRow, DivisionTemplateIconRow, DivisionTemplateRow, LandEventCoverageRow, RowCodec, TableId},
};
use std::io;

pub(super) fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

const DIVISION: [usize; 5] = columns::<DivisionRow, 5>(["roster_owner", "expeditionary_owner", "province", "template", "reveal_skip"]);
pub(super) const SHOWN: [usize; 1] = [DIVISION[4]];
pub(super) const NAMES: [usize; 2] = columns::<DivisionStatusRow, 2>(["name", "naming"]);
pub(super) const ICON: [usize; 3] = columns::<DivisionTemplateIconRow, 3>(["priority_unit", "custom_icon", "priority_unit_name"]);
pub(super) const LINK: [usize; 1] = columns::<CountryLinkRow, 1>(["canonical_slot"]);
const NAMING: [usize; 2] = columns::<DivisionTemplateIconRow, 2>(["display_name", "country"]);

fn row<R: RowCodec>(hour: &Hour<'_>, table: TableId, slot: u32, mask: u64) -> io::Result<R> {
    let held = hour.table(table);
    let key = held.key(slot).ok_or_else(|| invalid(format!("{table:?} slot {slot} holds no row")))?;
    let format = FormatVersion::CURRENT.version();
    R::from_wire_fields(key, mask, &mut |column| replay_frame::frame::field(&held, slot, column, format))
}

fn number(value: Held<'_>) -> Option<i64> {
    match value {
        Held::Number(bits) => Some(bits as i64),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Placed {
    pub owner: Option<u32>,
    pub province: Option<u32>,
    pub template: Option<u64>,
}

fn placed(cell: impl Fn(usize) -> Option<i64>) -> Placed {
    let roster = cell(DIVISION[0]).map(|value| value as u32);
    let expeditionary = cell(DIVISION[1]).map(|value| value as u32);
    Placed {
        owner: expeditionary.filter(|&owner| owner as i32 > 0).or(roster),
        province: cell(DIVISION[2]).map(|value| value as u32),
        template: cell(DIVISION[3]).map(|value| value as u64),
    }
}

pub(super) fn shown(view: &TableView<'_>, slot: u32) -> bool {
    view.number(DIVISION[4], slot) == Some(0)
}

pub(super) fn dropped_shown(cells: &[Kept]) -> bool {
    cells.get(DIVISION[4]).and_then(|cell| number(cell.value())) == Some(0)
}

pub(super) fn division(view: &TableView<'_>, slot: u32) -> Placed {
    placed(|column| view.number(column, slot))
}

pub(super) fn dropped_division(cells: &[Kept]) -> Placed {
    placed(|column| cells.get(column).and_then(|cell| number(cell.value())))
}

pub(super) type Names<'a> = [Option<&'a [u8]>; 2];

pub(super) fn names<'a>(hour: &Hour<'a>, division: u64) -> Option<Names<'a>> {
    if !hour.complete(TableId::DivisionStatus) {
        return None;
    }
    let view = hour.table(TableId::DivisionStatus);
    let slot = view.slot(u128::from(division))?;
    Some(NAMES.map(|column| view.list(column, slot)))
}

pub(super) fn dropped_names(cells: &[Kept]) -> Names<'_> {
    NAMES.map(|column| match cells.get(column) {
        Some(Kept::List(bytes)) => Some(&bytes[..]),
        _ => None,
    })
}

pub(super) struct Icon<'a> {
    pub unit: Option<u32>,
    pub custom: Option<i32>,
    pub name: Option<&'a [u8]>,
}

pub(super) fn icon<'a>(hour: &Hour<'a>, template: u64) -> Option<Icon<'a>> {
    if !hour.complete(TableId::DivisionTemplateIcons) {
        return None;
    }
    let view = hour.table(TableId::DivisionTemplateIcons);
    let slot = view.slot(u128::from(template))?;
    Some(Icon {
        unit: view.number(ICON[0], slot).map(|value| value as u32),
        custom: view.number(ICON[1], slot).map(|value| value as i32),
        name: view.text(ICON[2], slot).and_then(|id| hour.text(id)),
    })
}

pub(super) fn dropped_icon<'a>(hour: &Hour<'a>, cells: &[Kept]) -> Icon<'a> {
    let cell = |column: usize| cells.get(column).map(Kept::value);
    Icon {
        unit: cell(ICON[0]).and_then(number).map(|value| value as u32),
        custom: cell(ICON[1]).and_then(number).map(|value| value as i32),
        name: match cell(ICON[2]) {
            Some(Held::Text(id)) => hour.text(id),
            _ => None,
        },
    }
}

pub(super) struct TemplateName {
    pub literal: Vec<u8>,
    pub key: Option<Vec<u8>>,
    pub country: i32,
}

pub(super) fn template_name(hour: &Hour<'_>, template: u64) -> Option<TemplateName> {
    if !hour.complete(TableId::DivisionTemplateIcons) {
        return None;
    }
    let key = u128::from(template);
    let slot = hour.table(TableId::DivisionTemplateIcons).slot(key)?;
    let mask = NAMING.iter().fold(0, |mask, column| mask | 1 << column);
    let icon: DivisionTemplateIconRow = row(hour, TableId::DivisionTemplateIcons, slot, mask).ok()?;
    let daily: Option<DivisionTemplateRow> = hour
        .complete(TableId::DivisionTemplates)
        .then(|| hour.table(TableId::DivisionTemplates).slot(key))
        .flatten()
        .and_then(|slot| row(hour, TableId::DivisionTemplates, slot, u64::MAX).ok());
    let parts = icon.display_name_inputs(hour.hour, daily.as_ref()).ok()?;
    Some(TemplateName { literal: parts.literal.to_vec(), key: parts.fallback_key.map(<[u8]>::to_vec), country: icon.country })
}

pub(super) type Signature = (u32, u32, u8, Option<u8>);

pub(super) fn signature(row: &DivisionEquipmentDeletionRow) -> Signature {
    (row.country, row.expeditionary_owner, row.disposition as u8, row.cause.map(|cause| cause as u8))
}

pub(super) fn witness(hour: &Hour<'_>, slot: u32) -> io::Result<DivisionEquipmentDeletionRow> {
    row(hour, TableId::DivisionEquipmentDeletions, slot, u64::MAX)
}

pub(super) fn deaths(hour: &Hour<'_>) -> io::Result<Vec<(u64, DestroyedDivisionsRow)>> {
    let table = TableId::DestroyedDivisions;
    if !hour.complete(table) {
        return Ok(Vec::new());
    }
    let logged = hour.appended(table);
    let mut deaths = Vec::with_capacity(logged.len());
    for logged in logged.rows() {
        deaths.push((logged.ordinal, row(hour, table, logged.slot, u64::MAX)?));
    }
    Ok(deaths)
}

pub(super) fn covered(hour: &Hour<'_>) -> io::Result<Vec<(i32, i32)>> {
    let mut spans = Vec::new();
    if !hour.complete(TableId::LandEventCoverage) {
        return Ok(spans);
    }
    let view = hour.table(TableId::LandEventCoverage);
    for slot in view.rows() {
        let span: LandEventCoverageRow = row(hour, TableId::LandEventCoverage, slot, u64::MAX)?;
        if span.complete {
            spans.push((span.from_hour, span.to_hour));
        }
    }
    Ok(spans)
}

pub(super) fn resolve(hour: &Hour<'_>, country: u32) -> Option<u32> {
    if !hour.complete(TableId::CountryLinks) {
        return None;
    }
    let links = hour.table(TableId::CountryLinks);
    links.slot(u128::from(country)).and_then(|slot| links.number(LINK[0], slot)).map(|canonical| canonical as u32)
}
