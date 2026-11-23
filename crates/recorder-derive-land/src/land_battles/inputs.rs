
use recorder_derive_framework::framework::{Hour, columns};
use replay_columnar::store::TableView;
use replay_data::{
    format::FormatVersion,
    rows::{CountryLinkRow, LandCombatProgressRow, LandEquipmentLoss, LandCombatResultsRow, LandCombatRow, LandCombatSideRow, LandEventCoverageRow, RowCodec, TableId},
};
use std::io;

pub(super) fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

pub(super) const COMBAT: [usize; 4] = columns::<LandCombatRow, 4>(["province", "duration", "attackers", "defenders"]);
pub(super) const SIDES: [usize; 4] = columns::<LandCombatSideRow, 4>(["attacker_primary", "attacker_secondary", "defender_primary", "defender_secondary"]);
pub(super) const PROGRESS: [usize; 2] = columns::<LandCombatProgressRow, 2>(["attacker_manpower_loss", "defender_manpower_loss"]);
pub(super) const LINK_SLOT: [usize; 1] = columns::<CountryLinkRow, 1>(["canonical_slot"]);
const COVERAGE: [usize; 3] = columns::<LandEventCoverageRow, 3>(["from_hour", "to_hour", "complete"]);
const RESULT: [usize; 7] = columns::<LandCombatResultsRow, 7>(["combat", "province", "duration", "end_hour", "sides", "winners", "losses"]);

pub(super) struct Combat {
    pub province: u32,
    pub duration: i32,
    pub counts: Option<[u32; 2]>,
}

fn distinct(bytes: &[u8]) -> io::Result<u32> {
    if bytes.len() % 8 != 0 {
        return Err(invalid("A LandCombats roster is not whole handles"));
    }
    let mut handles: Vec<u64> = bytes.chunks_exact(8).map(|item| u64::from_le_bytes(item.try_into().expect("8 bytes"))).collect();
    handles.sort_unstable();
    handles.dedup();
    u32::try_from(handles.len()).map_err(|_| invalid("A LandCombats roster exceeds its native count"))
}

fn u32s(bytes: &[u8]) -> io::Result<impl Iterator<Item = u32> + '_> {
    if bytes.len() % 4 != 0 {
        return Err(invalid("A country list is not whole references"));
    }
    Ok(bytes.chunks_exact(4).map(|item| u32::from_le_bytes(item.try_into().expect("4 bytes"))))
}

pub(super) fn combat(view: &TableView<'_>, slot: u32, rosters: bool) -> io::Result<Combat> {
    let unread = || invalid(format!("LandCombats slot {slot} does not hold its province and duration"));
    let province = view.number(COMBAT[0], slot).ok_or_else(unread)? as u32;
    let duration = view.number(COMBAT[1], slot).ok_or_else(unread)? as i32;
    let counts = match (rosters, view.list(COMBAT[2], slot), view.list(COMBAT[3], slot)) {
        (true, Some(attackers), Some(defenders)) => Some([distinct(attackers)?, distinct(defenders)?]),
        _ => None,
    };
    Ok(Combat { province, duration, counts })
}

pub(super) fn sides(hour: &Hour<'_>, combat: u64) -> io::Result<Option<[Vec<u32>; 2]>> {
    if !hour.complete(TableId::LandCombatSides) {
        return Ok(None);
    }
    let view = hour.table(TableId::LandCombatSides);
    let Some(slot) = view.slot(u128::from(combat)) else { return Ok(None) };
    let mut sides = [Vec::new(), Vec::new()];
    for (at, &column) in SIDES.iter().enumerate() {
        let Some(bytes) = view.list(column, slot) else { return Ok(None) };
        sides[at / 2].extend(u32s(bytes)?);
    }
    Ok(Some(sides))
}

pub(super) fn manpower(hour: &Hour<'_>, combat: u64) -> [Option<i32>; 2] {
    if !hour.complete(TableId::LandCombatProgress) {
        return [None; 2];
    }
    let view = hour.table(TableId::LandCombatProgress);
    let Some(slot) = view.slot(u128::from(combat)) else { return [None; 2] };
    PROGRESS.map(|column| view.number(column, slot).map(|value| value as i32))
}

pub(super) fn link(hour: &Hour<'_>, raw: u32) -> Option<u32> {
    if !hour.complete(TableId::CountryLinks) {
        return None;
    }
    let links = hour.table(TableId::CountryLinks);
    links.slot(u128::from(raw)).and_then(|slot| links.number(LINK_SLOT[0], slot)).map(|slot| slot as u32)
}

pub(super) fn drain(hour: &Hour<'_>) -> Option<(i32, i32)> {
    if !hour.complete(TableId::LandEventCoverage) {
        return None;
    }
    let view = hour.table(TableId::LandEventCoverage);
    view.rows().find_map(|slot| {
        let [from, to, complete] = COVERAGE.map(|column| view.number(column, slot));
        (to == Some(i64::from(hour.hour)) && complete == Some(1)).then_some(())?;
        Some((from? as i32, hour.hour))
    })
}

pub(super) struct Finalizer {
    pub id: u64,
    pub combat: u64,
    pub province: u32,
    pub duration: i32,
    pub end_hour: i32,
    pub winners: u8,
    pub countries: [Vec<u32>; 2],
    pub manpower: [i32; 2],
    pub equipment: [Vec<LandEquipmentLoss>; 2],
}

pub(super) fn finalizers(hour: &Hour<'_>) -> io::Result<Vec<Finalizer>> {
    let table = TableId::LandCombatResults;
    if !hour.complete(table) {
        return Ok(Vec::new());
    }
    let logged = hour.appended(table);
    let Some(view) = logged.cells() else { return Ok(Vec::new()) };
    let slots: Vec<u32> = logged.rows().map(|row| row.slot).collect();
    let mask = RESULT.iter().fold(0_u64, |mask, &column| mask | 1 << column);
    let format = FormatVersion::CURRENT.version();
    let mut out = Vec::with_capacity(slots.len());
    for slot in slots {
        let key = view.key(slot).ok_or_else(|| invalid(format!("LandCombatResults slot {slot} holds no row")))?;
        let row = LandCombatResultsRow::from_wire_fields(key, mask, &mut |column| replay_frame::frame::field(&view, slot, column, format))?;
        out.push(Finalizer {
            id: row.id,
            combat: row.combat,
            province: row.province,
            duration: row.duration,
            end_hour: row.end_hour,
            winners: row.winners,
            countries: row.sides.each_ref().map(|side| side.countries.iter().chain(&side.secondary_countries).copied().collect()),
            manpower: row.losses.each_ref().map(|side| side.manpower),
            equipment: row.losses.each_ref().map(|side| side.equipment.clone()),
        });
    }
    out.sort_unstable_by_key(|finalizer| finalizer.id);
    Ok(out)
}

pub(super) struct Held {
    pub start: i32,
    pub exact: bool,
    pub counter_base: i32,
    pub peaks: [Option<(u32, i32)>; 3],
}

pub(super) fn held(view: &TableView<'_>, slot: u32) -> Option<Held> {
    let peak = |value: usize| Some((view.number(value, slot)? as u32, view.number(value + 1, slot)? as i32));
    Some(Held {
        start: view.number(0, slot)? as i32,
        exact: view.number(1, slot)? != 0,
        counter_base: view.number(2, slot)? as i32,
        peaks: [peak(3), peak(5), peak(7)],
    })
}

pub(super) fn end(view: &TableView<'_>, slot: u32) -> Option<(u64, u8)> {
    Some(((view.key(slot)? >> 33) as u64, view.number(0, slot)? as u8))
}
