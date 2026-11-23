
use recorder_derive_framework::framework::{Hour, columns};
use replay_columnar::store::TableView;
use replay_data::rows::{NavalCombatParticipantRow, NavalCombatRow, NavalResultSourceRow, TableId};
use std::io;

pub(super) fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

pub(super) const PROVINCE: [usize; 1] = columns::<NavalCombatRow, 1>(["province"]);
pub(super) const MEMBER: [usize; 2] = columns::<NavalCombatParticipantRow, 2>(["ship", "state"]);
const SOURCE: [usize; 6] = columns::<NavalResultSourceRow, 6>(["game_hour", "result", "combat", "merged", "transient", "combat_created_hour"]);

const SUNK: i64 = 0x20;

pub(super) fn province(view: &TableView<'_>, slot: u32) -> io::Result<u32> {
    view.number(PROVINCE[0], slot).map(|province| province as u32).ok_or_else(|| invalid(format!("NavalCombats slot {slot} does not hold its province")))
}

pub(super) struct Member {
    pub combat: u64,
    pub side: usize,
    pub flags: Option<(bool, bool)>,
}

pub(super) fn member(view: &TableView<'_>, slot: u32) -> Option<Member> {
    let key = view.key(slot)?;
    let flags = view.number(MEMBER[0], slot).zip(view.number(MEMBER[1], slot)).map(|(ship, state)| (ship == 0, state & SUNK != 0));
    Some(Member { combat: (key >> 33) as u64, side: ((key >> 32) & 1) as usize, flags })
}

pub(super) fn member_combat(key: u128) -> u64 {
    (key >> 33) as u64
}

pub(super) struct Source {
    pub id: u64,
    pub game_hour: i32,
    pub result: u64,
    pub combat: u64,
    pub merged: bool,
    pub transient: bool,
    pub created: Option<i32>,
}

pub(super) fn sources(hour: &Hour<'_>) -> io::Result<Option<Vec<Source>>> {
    let table = TableId::NavalResultSources;
    if !hour.complete(table) {
        return Ok(None);
    }
    let logged = hour.appended(table);
    let Some(view) = logged.cells() else { return Ok(Some(Vec::new())) };
    let slots: Vec<u32> = logged.rows().map(|row| row.slot).collect();
    let mut out = Vec::with_capacity(slots.len());
    for slot in slots {
        let unread = || invalid(format!("NavalResultSources slot {slot} does not hold every cell it is known by"));
        let id = view.key(slot).ok_or_else(unread)? as u64;
        let [game_hour, result, combat, merged, transient, _] = SOURCE.map(|column| view.number(column, slot));
        let (Some(game_hour), Some(result), Some(combat), Some(merged), Some(transient)) = (game_hour, result, combat, merged, transient) else { return Err(unread()) };
        let created = view.number(SOURCE[5], slot).map(|hour| hour as i32);
        out.push(Source { id, game_hour: game_hour as i32, result: result as u64, combat: combat as u64, merged: merged != 0, transient: transient != 0, created });
    }
    out.sort_unstable_by_key(|source| source.id);
    Ok(Some(out))
}

pub(super) struct Held {
    pub first: i32,
    pub peak: Option<(u32, i32)>,
    pub sunk: [Option<u32>; 2],
    pub convoys_sunk: [Option<u32>; 2],
}

pub(super) fn held(view: &TableView<'_>, slot: u32) -> Option<Held> {
    let number = |column| view.number(column, slot).map(|value| value as u32);
    Some(Held {
        first: view.number(0, slot)? as i32,
        peak: number(1).zip(view.number(2, slot).map(|hour| hour as i32)),
        sunk: [number(3), number(4)],
        convoys_sunk: [number(5), number(6)],
    })
}
