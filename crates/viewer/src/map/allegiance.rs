//! Which side a country is on, measured against the replay's perspective.
//!
//! Two map layers need this: the victory-point strip picks one of five colours
//! in each shape group by it (R2 section 5.2), and the unit counters modulate
//! their plate with `COUNTER_MODE_ALLEGIANCE_*` (R2 section 6.6). Both are
//! answered from the same tables, so the perspective and the relation tables
//! live here rather than in either pass.
//!
//! # The perspective
//!
//! The replay is omniscient (`PLAN.md` section 2), so "ours" has to be chosen.
//! The `Players` table's human country is used when there is exactly one,
//! otherwise the first human controller, so a multiplayer replay still picks a
//! stable country rather than colouring the whole map grey.
//!
//! # Two readings, not one
//!
//! [`war_table`] is the reading the victory-point labels are fitted to: ours is
//! green, a country at war with ours is red, and **everything else is grey,
//! including our own faction partners**. `array_loaded.png` forces that: the
//! recorded `FactionMembers` put `AUS`, `ROM`, `YUG`, `BUL` and `HUN` in
//! Germany's faction, and Vienna, Budapest, Győr, Linz and Salzburg are all
//! grey in the screenshot while colouring faction members blue paints all five.
//!
//! [`counter_table`] adds faction membership as
//! `COUNTER_MODE_ALLEGIANCE_ALLIED`, because the counters in the same
//! screenshot **do** carry the blue plate. The German counters north of the
//! Ruhr sample `(37, 79, 43)` on the plate, which is the grey
//! `onmap_unit_counter.dds` body `(124, 124, 124)` times
//! `COUNTER_MODE_ALLEGIANCE_OURS = (0.32, 0.71, 0.39)`; the counters beside
//! them carrying Romanian and Hungarian flags are blue, and the Soviet, Belgian
//! and French ones are grey. So the two layers do not share a selector, and
//! neither reading is imposed on the other.
//!
//! **UNVERIFIED**: that faction membership is exactly what the counters select
//! blue on. Nothing in this frame separates "in our faction" from "at war on
//! our side": `WarRelations` records no German war at this hour, so a
//! fighting-with-us reading produces the same set. Red likewise has no positive
//! evidence in either reference frame, only the palette name
//! (`COUNTER_MODE_ALLEGIANCE_ENEMY`).

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use replay::{frame::DecodedFrame, rows::tables};

/// `00_graphics.lua:724`, the plate modulate of a counter of our own country.
pub const OURS_COLOR: [f32; 4] = [0.32, 0.71, 0.39, 1.0];
/// `00_graphics.lua:725`, `COUNTER_MODE_ALLEGIANCE_ALLIED`.
pub const ALLIED_COLOR: [f32; 4] = [0.31, 0.65, 0.94, 1.0];
/// `00_graphics.lua:726`, `COUNTER_MODE_ALLEGIANCE_ENEMY`.
pub const ENEMY_COLOR: [f32; 4] = [0.91, 0.30, 0.30, 1.0];
/// `00_graphics.lua:727`, `COUNTER_MODE_ALLEGIANCE_OTHER`.
pub const OTHER_COLOR: [f32; 4] = [0.8, 0.8, 0.8, 1.0];

/// Which side a country is on, relative to the chosen perspective.
///
/// The victory-point strip is four shapes by five colours with the shape
/// varying slowest, and the colours run red, blue, green, gold, grey (R2
/// section 5.2). [`Allegiance::index`] is written out rather than being a
/// discriminant because the gold slot has no input that selects it at all
/// (R2 open item 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Allegiance {
    /// `COUNTER_MODE_ALLEGIANCE_ENEMY`, red.
    Enemy,
    /// `COUNTER_MODE_ALLEGIANCE_ALLIED`, blue.
    Allied,
    /// `COUNTER_MODE_ALLEGIANCE_OURS`, green.
    Ours,
    /// `COUNTER_MODE_ALLEGIANCE_OTHER`, grey.
    Other,
}

impl Allegiance {
    /// The colour's position inside a victory-point shape group of five.
    pub fn index(self) -> u32 {
        match self {
            Allegiance::Enemy => 0,
            Allegiance::Allied => 1,
            Allegiance::Ours => 2,
            Allegiance::Other => 4,
        }
    }

    /// The straight RGBA a unit counter's plate is multiplied by.
    pub fn counter_color(self) -> [f32; 4] {
        match self {
            Allegiance::Enemy => ENEMY_COLOR,
            Allegiance::Allied => ALLIED_COLOR,
            Allegiance::Ours => OURS_COLOR,
            Allegiance::Other => OTHER_COLOR,
        }
    }
}

/// The canonical country slot every allegiance is measured against.
///
/// `Players` holds the session's human countries; when it names exactly one,
/// that is the perspective. Otherwise the first `HumanControllers` row with a
/// positive count is taken.
pub fn perspective(frame: &DecodedFrame, links: &BTreeMap<u32, u32>) -> io::Result<Option<u32>> {
    let players = frame.complete::<tables::Players>()?;
    let distinct: BTreeSet<u32> = players
        .iter()
        .filter_map(|player| links.get(&player.country).copied())
        .collect();
    if distinct.len() == 1 {
        return Ok(distinct.into_iter().next());
    }
    let mut humans: Vec<u32> = frame
        .complete::<tables::HumanControllers>()?
        .iter()
        .filter(|row| row.count > 0)
        .filter_map(|row| links.get(&row.country).copied())
        .collect();
    humans.sort_unstable();
    Ok(humans.first().copied())
}

/// Ours and everyone at war with us; every other country is absent.
///
/// This is the victory-point labels' reading; see the module docs for why
/// faction membership is deliberately not consulted here.
pub fn war_table(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    ours: Option<u32>,
) -> io::Result<BTreeMap<u32, Allegiance>> {
    let mut table = BTreeMap::new();
    let Some(ours) = ours else {
        return Ok(table);
    };
    table.insert(ours, Allegiance::Ours);
    // A war row stays in the table after the war ends, with `end_hour` set;
    // an ongoing war records `i32::MAX`.
    for war in frame.complete::<tables::WarRelations>()? {
        if war.end_hour <= frame.hour {
            continue;
        }
        let (first, second) = (
            links.get(&war.first_country).copied(),
            links.get(&war.second_country).copied(),
        );
        let other = match (first, second) {
            (Some(a), Some(b)) if a == ours => Some(b),
            (Some(a), Some(b)) if b == ours => Some(a),
            _ => None,
        };
        if let Some(other) = other.filter(|slot| *slot != ours) {
            table.insert(other, Allegiance::Enemy);
        }
    }
    Ok(table)
}

/// [`war_table`] plus our faction's other members as [`Allegiance::Allied`].
///
/// A country that is both at war with us and in our faction keeps `Enemy`: the
/// war row is the stronger statement, and the engine's own palette has no slot
/// for the combination.
pub fn counter_table(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    ours: Option<u32>,
) -> io::Result<BTreeMap<u32, Allegiance>> {
    let mut table = war_table(frame, links, ours)?;
    let Some(ours) = ours else {
        return Ok(table);
    };
    let members = frame.complete::<tables::FactionMembers>()?;
    // A country has at most one faction membership, so the faction we are in is
    // whichever row names us.
    let Some(faction) = members
        .iter()
        .find(|row| links.get(&row.country).copied() == Some(ours))
        .map(|row| row.faction)
    else {
        return Ok(table);
    };
    for row in members.iter().filter(|row| row.faction == faction) {
        let Some(slot) = links.get(&row.country).copied() else {
            continue;
        };
        if slot != ours {
            table.entry(slot).or_insert(Allegiance::Allied);
        }
    }
    Ok(table)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_colour_slots_follow_the_strip_and_the_palette() {
        assert_eq!(Allegiance::Enemy.index(), 0);
        assert_eq!(Allegiance::Allied.index(), 1);
        assert_eq!(Allegiance::Ours.index(), 2);
        // Slot 3 is the gold frame, which nothing selects (R2 open item 6).
        assert_eq!(Allegiance::Other.index(), 4);
        assert_eq!(Allegiance::Ours.counter_color(), OURS_COLOR);
        assert_eq!(Allegiance::Allied.counter_color(), ALLIED_COLOR);
        assert_eq!(Allegiance::Enemy.counter_color(), ENEMY_COLOR);
        assert_eq!(Allegiance::Other.counter_color(), OTHER_COLOR);
    }

    #[test]
    fn the_plate_modulate_reproduces_the_measured_screenshot_colour() {
        // R2 section 6.5: `onmap_unit_counter.dds` is a grey plate whose body
        // samples (124, 124, 124). A German counter in `array_loaded.png`
        // samples (37, 79, 43) on that body.
        let body = 124.0f32;
        let ours = Allegiance::Ours.counter_color();
        let modulated = [body * ours[0], body * ours[1], body * ours[2]];
        for (channel, measured) in modulated.into_iter().zip([37.0, 79.0, 43.0]) {
            assert!(
                (channel - measured).abs() < 10.0,
                "{channel} is not within a rounding of {measured}"
            );
        }
    }
}
