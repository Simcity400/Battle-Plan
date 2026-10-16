//! Simulation occupant order, HQ portrait holders and ship counter operands.
//! Rules are pure over recorded simulation rows and recorded content.
//!
//! Unit sources walk `ProvinceUnitOrder.mixed` in array order, excluding units absent
//! from that province's array. Port counters walk `ProvinceUnitOrder.naval` before
//! sorting. Source activation order, grouping cursors and groups are local
//! presentation history ([`crate::map::counter_groups::SourceLayer`]).
//!
//! Ship rules use [`is_capital`], [`better_hull`], [`task_force_carrier`] and
//! [`port_carrier`]. HQ portrait selection uses [`hq_portrait`].

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use replay::{
    frame::DecodedFrame,
    rows::{CharacterPortraitRow, LeaderPortraitRow, ShipModelRow, portrait_entry, tables},
};

use crate::map::tables::rows;

/// Native simulation occupant order for one hour.
#[derive(Clone, Debug, Default)]
pub struct RecordedOrder {
    /// Unit handle to (province, first ordinal in that province's mixed array).
    mixed: BTreeMap<u64, (u32, u32)>,
    /// Unit handle to (province, first ordinal in that province's naval array).
    naval: BTreeMap<u64, (u32, u32)>,
    /// Each province's naval occupant array, in native order.
    naval_rosters: BTreeMap<u32, Vec<u64>>,
}

impl RecordedOrder {
    /// `None` when the hour holds no complete `ProvinceUnitOrder`: missing
    /// entries cannot establish native insertion order.
    pub fn read(frame: &DecodedFrame) -> io::Result<Option<Self>> {
        let order = rows::<tables::ProvinceUnitOrder>(frame)?;
        if !order.is_complete() {
            return Ok(None);
        }
        Ok(Some(Self::from_rows(order.rows())))
    }

    /// Indexes the recorded occupant arrays; the first occurrence of a unit wins.
    pub fn from_rows(provinces: &[replay::rows::ProvinceUnitOrderRow]) -> Self {
        let mut out = Self::default();
        for row in provinces {
            for (ordinal, unit) in row.mixed.iter().enumerate() {
                out.mixed
                    .entry(*unit)
                    .or_insert((row.province, ordinal as u32));
            }
            for (ordinal, unit) in row.naval.iter().enumerate() {
                out.naval
                    .entry(*unit)
                    .or_insert((row.province, ordinal as u32));
            }
            if !row.naval.is_empty() {
                out.naval_rosters.insert(row.province, row.naval.clone());
            }
        }
        out
    }

    /// The unit's ordinal in `province`'s mixed occupant array; `None` means the engine does not
    /// walk it there.
    pub fn mixed(&self, unit: u64, province: u32) -> Option<u32> {
        self.mixed
            .get(&unit)
            .filter(|(at, _)| *at == province)
            .map(|(_, ordinal)| *ordinal)
    }

    /// The task force's ordinal in `province`'s naval occupant array.
    pub fn naval(&self, unit: u64, province: u32) -> Option<u32> {
        self.naval
            .get(&unit)
            .filter(|(at, _)| *at == province)
            .map(|(_, ordinal)| *ordinal)
    }

    /// `province`'s whole naval occupant array, empty when it holds none.
    pub fn naval_roster(&self, province: u32) -> &[u64] {
        self.naval_rosters.get(&province).map_or(&[], Vec::as_slice)
    }
}

/// Capital-ship flag.
pub const CAPITAL_FLAG: u64 = 1 << 6;
/// Carrier bit, tested on the ship and on positive pool variants.
pub const CARRIER_FLAG: u64 = 1 << 22;
/// Missile-carrier bit.
pub const MISSILE_CARRIER_FLAG: u64 = 1 << 37;
/// The capital-slot mask on the first roster ship, bits 6 and 22.
pub const FORMATION_CAPITAL_MASK: u64 = 0x40_0040;

/// Carrier status includes the ship's missile-carrier bit and positive variants
/// with that bit. Missing model operands remain unavailable.
pub fn is_carrier(row: &ShipModelRow) -> Option<bool> {
    let counter = row.counter.as_ref()?;
    let mask = CARRIER_FLAG | MISSILE_CARRIER_FLAG;
    Some(row.formation_flags & mask != 0 || counter.positive_equipment_flags & mask != 0)
}

/// Whether a roster's first carrier ([`is_carrier`]) has a carrier air base; `false` without a
/// carrier, `None` when a ship before it lacks model operands.
pub fn first_carrier_has_air_base(roster: &[&ShipModelRow]) -> Option<bool> {
    for ship in roster {
        if is_carrier(ship)? {
            return Some(
                ship.counter
                    .as_ref()
                    .is_some_and(|c| c.carrier_air_base != 0),
            );
        }
    }
    Some(false)
}

/// A ship is capital when its capital bit or qualifying carrier status holds.
pub fn is_capital(row: &ShipModelRow) -> Option<bool> {
    let carrier = is_carrier(row)?;
    Some(row.formation_flags & CAPITAL_FLAG != 0 || carrier)
}

/// One ship as the counter's hull selector sees it.
#[derive(Clone, Copy, Debug)]
pub struct Hull<'a> {
    /// The recorded `Ships.strength`.
    pub strength: i64,
    pub model: &'a ShipModelRow,
}

/// Whether candidate `a` replaces current best `b`: capital status, positive
/// variant availability, greater strength, then sub-unit ordering. A comparison
/// with a missing operand remains unavailable.
pub fn better_hull(
    a: &Hull<'_>,
    b: &Hull<'_>,
    equipment: Result<&installed_content::equipment::Database, &str>,
) -> Result<bool, String> {
    let air = |hull: &Hull<'_>| {
        hull.model
            .counter
            .as_ref()
            .map(|c| c.carrier_air_base)
            .ok_or_else(|| format!("ship {} counter operands are unrecorded", hull.model.ship))
    };
    let (air_a, air_b) = (air(a)?, air(b)?);
    if (air_a == 0) != (air_b == 0) {
        return Ok(air_a != 0);
    }
    if a.strength != b.strength {
        return Ok(a.strength > b.strength);
    }
    match (a.model.variant, b.model.variant) {
        (_, 0) => Ok(true),
        (0, _) => Ok(false),
        (x, y) if x == y => Ok(false),
        _ => preferred_variant(a.model, b.model, equipment),
    }
}

fn preferred_variant(
    a: &ShipModelRow,
    b: &ShipModelRow,
    equipment: Result<&installed_content::equipment::Database, &str>,
) -> Result<bool, String> {
    let (a, b) = (variant_packet(a)?, variant_packet(b)?);
    let definition = |index: Option<i32>| -> Result<(i32, Option<usize>), String> {
        let index = index.ok_or("current variant definition index is unreadable")?;
        // The writer authenticates the null sub-unit definition
        // before emitting -1. Its initial state sets
        // depth to zero and self-linked categories to zero.
        if index == -1 {
            return Ok((0, None));
        }
        let ordinal = index
            .checked_sub(1)
            .and_then(|index| usize::try_from(index).ok())
            .ok_or_else(|| {
                format!("current variant definition index {index} is not a real definition")
            })?;
        let equipment = equipment.map_err(str::to_owned)?;
        if let Some(reason) = &equipment.lost {
            return Err(format!(
                "equipment definition content is incomplete: {reason}"
            ));
        }
        equipment
            .equipment
            .get(ordinal)
            .map(|definition| (definition.depth, Some(ordinal)))
            .ok_or_else(|| {
                format!(
                    "current variant definition index {index} is absent from recorded equipment"
                )
            })
    };
    let ((a_depth, a_index), (b_depth, _)) = (
        definition(a.definition_index)?,
        definition(b.definition_index)?,
    );
    if a_depth != b_depth {
        return Ok(a_depth > b_depth);
    }
    let score = |packet: &replay::rows::HullVariantInputs| -> Result<i32, String> {
        Ok(packet
            .module_count
            .ok_or("current variant module count is unreadable")?
            .wrapping_add(
                packet
                    .upgrade_level_sum
                    .ok_or("current variant upgrade level sum is unreadable")?,
            )
            .wrapping_add(
                packet
                    .module_priority_sum
                    .ok_or("current variant module priority sum is unreadable")?,
            ))
    };
    let (a_score, b_score) = (score(a)?, score(b)?);
    if a_score != b_score {
        let plane = match a_index {
            Some(index) => plane_definition(equipment.map_err(str::to_owned)?, index)?,
            None => false,
        };
        if plane {
            return Ok(a.version.ok_or("current variant version is unreadable")?
                > b.version.ok_or("current variant version is unreadable")?);
        }
        return Ok(a_score > b_score);
    }
    Ok(a.organisation_marker
        .ok_or("current variant organisation marker is unreadable")?
        > b.organisation_marker
            .ok_or("current variant organisation marker is unreadable")?)
}

fn variant_packet(model: &ShipModelRow) -> Result<&replay::rows::HullVariantInputs, String> {
    model
        .hull_variant
        .require("ShipModels.hull_variant")?
        .as_ref()
        .ok_or_else(|| {
            format!(
                "ship {} current variant operands are unreadable",
                model.ship
            )
        })
}

/// The game visits the archetype before its own effective categories. Following
/// that order also refuses a broken/cyclic authored link before testing own bits.
fn plane_definition(
    equipment: &installed_content::equipment::Database,
    index: usize,
) -> Result<bool, String> {
    let mut chain = Vec::new();
    let mut current = index;
    loop {
        if chain.contains(&current) {
            return Err("equipment plane classifier has an archetype cycle".into());
        }
        let definition = equipment
            .equipment
            .get(current)
            .ok_or("equipment plane classifier archetype is absent")?;
        chain.push(current);
        match definition.archetype {
            Some(next) if next != current => current = next,
            _ => break,
        }
    }
    Ok(chain.into_iter().rev().any(|index| {
        equipment.category(&equipment.equipment[index]) & engine::cequipment_type::PLANE_MASK != 0
    }))
}

/// What the naval `type` child resolves to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HullIcon {
    /// The navy fallback sprite: `GFX_unit_` plus `ShipModels.fallback_sprite` when
    /// `definition_valid` is set. `None` draws nothing.
    Resolved(Option<Vec<u8>>),
    /// Unrecorded variant ties leave candidates with different names.
    Ambiguous,
    /// A member lacks format-23 ship operands.
    Unrecorded,
}

/// Select the best hull in task-force and roster order. Each candidate replaces
/// the current hull only when [`better_hull`] holds.
pub fn hull_icon<'a>(
    ships: impl IntoIterator<Item = Hull<'a>>,
    equipment: Result<&installed_content::equipment::Database, &str>,
) -> HullIcon {
    let mut best: Vec<Hull<'a>> = Vec::new();
    for ship in ships {
        if ship.model.counter.is_none() {
            return HullIcon::Unrecorded;
        }
        if best.is_empty() {
            best.push(ship);
            continue;
        }
        let mut next: Vec<Hull<'a>> = Vec::new();
        let push = |hull: Hull<'a>, next: &mut Vec<Hull<'a>>| {
            if !next.iter().any(|h| h.model.ship == hull.model.ship) {
                next.push(hull);
            }
        };
        for current in &best {
            match better_hull(&ship, current, equipment) {
                Ok(true) => push(ship, &mut next),
                Ok(false) => push(*current, &mut next),
                Err(_) => {
                    push(*current, &mut next);
                    push(ship, &mut next);
                }
            }
        }
        best = next;
    }
    let names: BTreeSet<Option<Vec<u8>>> = best
        .iter()
        .map(|hull| {
            let valid = hull
                .model
                .counter
                .as_ref()
                .is_some_and(|c| c.definition_valid != 0);
            (valid && !hull.model.fallback_sprite.is_empty())
                .then(|| hull.model.fallback_sprite.clone())
        })
        .collect();
    match names.len() {
        0 => HullIcon::Resolved(None),
        1 => HullIcon::Resolved(names.into_iter().next().flatten()),
        _ => HullIcon::Ambiguous,
    }
}

/// What the counter knows about one carrier's own air base.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CarrierBase {
    /// Whether the viewer's canonical slot has an `AirBaseRegistrations` container at this base.
    pub access: bool,
    /// The recorded `AirBases.kind`.
    pub kind: i32,
    /// A registered nonempty wing pool in the viewing country's container proves
    /// stationing. Otherwise unrecorded owner registrations leave it unknown.
    pub stationed: Option<bool>,
}

/// Scan one task force's roster. For each carrier ([`is_carrier`]) with a non-null
/// air base that passes access, a stationed base returns 4 (non-zero kind) or 2
/// immediately. An unstationed base sets 3 or 1 and keeps scanning. Return every
/// value unrecorded operands leave possible; an empty set means a ship lacks
/// format-23 operands or its base is unrecorded.
pub fn task_force_carrier(
    roster: &[&ShipModelRow],
    base: &impl Fn(u64) -> Option<CarrierBase>,
) -> BTreeSet<u32> {
    // Each branch is (result so far, still scanning).
    let mut states: BTreeSet<(u32, bool)> = BTreeSet::from([(0, true)]);
    for ship in roster {
        let Some(carrier) = is_carrier(ship) else {
            return BTreeSet::new();
        };
        let air = ship.counter.as_ref().map_or(0, |c| c.carrier_air_base);
        if !carrier || air == 0 {
            continue;
        }
        let Some(info) = base(air) else {
            return BTreeSet::new();
        };
        if !info.access {
            continue;
        }
        let (stationed, unstationed) = if info.kind != 0 { (4, 3) } else { (2, 1) };
        let mut next = BTreeSet::new();
        for (value, scanning) in states {
            if !scanning {
                next.insert((value, false));
                continue;
            }
            if info.stationed != Some(false) {
                next.insert((stationed, false));
            }
            if info.stationed != Some(true) {
                next.insert((unstationed, true));
            }
        }
        states = next;
    }
    states.into_iter().map(|(value, _)| value).collect()
}

/// the ordinary naval counter keeps the maximum over its members.
pub fn counter_carrier(members: &[BTreeSet<u32>]) -> BTreeSet<u32> {
    let mut out = BTreeSet::from([0]);
    for member in members {
        if member.is_empty() {
            return BTreeSet::new();
        }
        out = out
            .iter()
            .flat_map(|a| member.iter().map(move |b| *a.max(b)))
            .collect();
    }
    out
}

/// The `air_base_icon` child of an ordinary naval counter: `GFX_mapicon_air_base` for 1 and 2,
/// `GFX_mapicon_rocket_site` for 3 and 4,
/// with one-based frame 1 for 1 and 3 and 2 for 2 and 4, returned zero-based. The selected-base
/// frames 6 and 7 need a UI selection a replay does not have.
pub fn carrier_icon(value: u32) -> Option<(&'static str, u32)> {
    match value {
        1 | 2 => Some((AIR_BASE_ICON, u32::from(value == 2))),
        3 | 4 => Some((ROCKET_SITE_ICON, u32::from(value == 4))),
        _ => None,
    }
}

pub const AIR_BASE_ICON: &str = "GFX_mapicon_air_base";
pub const ROCKET_SITE_ICON: &str = "GFX_mapicon_rocket_site";

/// Port carrier selection folds members' possible carrier states in item order.
/// Air-base and rocket-site states have separate latches. The first selects the
/// air-base sprite; otherwise rocket-site status selects that sprite. No admitted
/// state hides the child. Unstationed states select the first frame.
pub fn port_carrier(members: &[BTreeSet<u32>]) -> BTreeSet<Option<(&'static str, u32)>> {
    let mut states: BTreeSet<[bool; 4]> = BTreeSet::from([[false, true, false, true]]);
    for member in members {
        if member.is_empty() {
            return BTreeSet::new();
        }
        let mut next = BTreeSet::new();
        for state in &states {
            if !state[1] {
                next.insert(*state);
                continue;
            }
            for value in member {
                let mut s = *state;
                match value {
                    1 | 2 => {
                        s[0] = true;
                        s[1] = *value == 1;
                    }
                    3 | 4 => {
                        s[2] = true;
                        s[3] = *value == 3;
                    }
                    _ => {}
                }
                next.insert(s);
            }
        }
        states = next;
    }
    states
        .into_iter()
        .map(|[air, first, rocket, third]| {
            (air || rocket).then_some((
                if air { AIR_BASE_ICON } else { ROCKET_SITE_ICON },
                u32::from(!(first || third)),
            ))
        })
        .collect()
}

/// The recorded `UnitLeaders.category`: 0 and 1 are army (1), 2 navy (2),
/// 3 operative (4); anything else keeps the initial 1.
pub fn leader_category(category: i32) -> u32 {
    match category {
        2 => 2,
        3 => 4,
        _ => 1,
    }
}

/// Headquarters portrait size.
pub const HQ_PORTRAIT_SIZE: u32 = 1;
/// the portrait the game falls back to.
pub const UNKNOWN_LEADER_PORTRAIT: &str = "GFX_leader_unknown";

/// What an HQ counter's `general_portrait` child shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Portrait {
    /// A `GFX_...` sprite name (descriptor kind 0).
    Sprite(Vec<u8>),
    /// A texture path (descriptor kind 1).
    File(Vec<u8>),
    /// The native fallback, [`UNKNOWN_LEADER_PORTRAIT`], when the named descriptor does not resolve.
    FallbackIfUnresolved(Box<Portrait>),
    /// A native branch whose input is not recorded.
    Unrecorded(&'static str),
}

/// Select the recorded headquarters portrait. No leader uses the unknown portrait.
/// A character reference uses its recorded category and size-one holder entry,
/// accepting a declared sprite or loaded texture. Unrecorded generated portraits
/// remain unavailable. Non-character leaders try their explicit path, then their
/// country-qualified picture path, then sprite name and the unknown portrait.
/// Texture paths must belong to the loaded authored portrait set.
pub fn hq_portrait(
    leader: Option<&LeaderPortraitRow>,
    category: Option<i32>,
    character: Option<&CharacterPortraitRow>,
    tag: Option<&[u8]>,
) -> Portrait {
    let Some(leader) = leader else {
        return Portrait::Sprite(UNKNOWN_LEADER_PORTRAIT.as_bytes().to_vec());
    };
    if leader.character != 0 {
        // A non-zero character reference takes the holder branch and never reaches the
        // leader's portrait strings, so a reference with no row stays undrawn.
        let Some(character) = character else {
            return Portrait::Unrecorded("character row");
        };
        let Some(category) = category else {
            return Portrait::Unrecorded("leader category");
        };
        return match portrait_entry(
            &character.portraits,
            leader_category(category),
            HQ_PORTRAIT_SIZE,
        ) {
            Some(entry) if entry.is_set != 0 && entry.kind == 0 => {
                Portrait::Sprite(entry.identity.clone())
            }
            Some(entry) if entry.is_set != 0 && entry.kind == 1 => {
                Portrait::File(entry.identity.clone())
            }
            _ => Portrait::Unrecorded("generated culture portrait"),
        };
    }
    if !leader.portrait_c0.is_empty() {
        return Portrait::FallbackIfUnresolved(Box::new(Portrait::File(
            leader.portrait_c0.clone(),
        )));
    }
    if !leader.portrait_e0.is_empty() {
        return match tag.filter(|tag| !tag.is_empty()) {
            Some(tag) => Portrait::FallbackIfUnresolved(Box::new(Portrait::File(
                crate::map::unit_rules::leader_directory_path(tag, &leader.portrait_e0)
                    .expect("a non-empty tag and picture always build a path"),
            ))),
            None => Portrait::Unrecorded("leader picture directory"),
        };
    }
    if !leader.portrait_100.is_empty() {
        return Portrait::FallbackIfUnresolved(Box::new(Portrait::Sprite(
            leader.portrait_100.clone(),
        )));
    }
    Portrait::Sprite(UNKNOWN_LEADER_PORTRAIT.as_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::{
        CharacterPortrait, HullVariantInputs, ProvinceUnitOrderRow, Recorded, ShipCounterInputs,
    };

    fn ship(
        ship: u64,
        flags: u64,
        positive: u64,
        air: u64,
        variant: u64,
        name: &str,
    ) -> ShipModelRow {
        ShipModelRow {
            ship,
            variant,
            hull_variant: Recorded::NotRecorded,
            fallback_sprite: name.as_bytes().to_vec(),
            formation_flags: flags,
            counter: Some(ShipCounterInputs {
                definition_valid: 1,
                carrier_air_base: air,
                positive_equipment_flags: positive,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn units_follow_their_own_province_array() {
        let order = RecordedOrder::from_rows(&[ProvinceUnitOrderRow {
            province: 7,
            mixed: vec![72, 70, 71],
            naval: vec![5],
        }]);
        assert_eq!(order.mixed(70, 7), Some(1));
        assert_eq!(order.mixed(72, 7), Some(0));
        // Not walked in another province's array.
        assert_eq!(order.mixed(70, 8), None);
        assert_eq!(order.naval(5, 7), Some(0));
    }

    #[test]
    fn capital_counts_follow_the_full_flag_word_and_positive_pool() {
        assert_eq!(
            is_capital(&ship(1, CAPITAL_FLAG, 0, 0, 0, "bb")),
            Some(true)
        );
        assert_eq!(
            is_capital(&ship(1, CARRIER_FLAG, 0, 0, 0, "cv")),
            Some(true)
        );
        assert_eq!(
            is_capital(&ship(1, 0, MISSILE_CARRIER_FLAG, 0, 0, "x")),
            Some(true)
        );
        // Support bits 7/8/39 are not capital bits.
        assert_eq!(
            is_capital(&ship(1, (1 << 7) | (1 << 8) | (1 << 39), 0, 0, 0, "dd")),
            Some(false)
        );
        let mut legacy = ship(1, CAPITAL_FLAG, 0, 0, 0, "bb");
        legacy.counter = None;
        assert_eq!(is_capital(&legacy), None);
    }

    #[test]
    fn the_hull_selector_prefers_carriers_then_strength_then_variant_ties() {
        let carrier = ship(1, CARRIER_FLAG, 0, 900, 11, "carrier");
        let battleship = ship(2, CAPITAL_FLAG, 0, 0, 12, "battleship");
        let cruiser = ship(3, 0, 0, 0, 13, "light_cruiser");
        let hull = |model, strength| Hull { strength, model };
        // Air base beats strength.
        assert_eq!(
            hull_icon(
                [hull(&battleship, 900), hull(&carrier, 1)],
                Err("unreached equipment content"),
            ),
            HullIcon::Resolved(Some(b"carrier".to_vec()))
        );
        // Strength beats roster order.
        assert_eq!(
            hull_icon(
                [hull(&cruiser, 5), hull(&battleship, 9)],
                Err("unreached equipment content"),
            ),
            HullIcon::Resolved(Some(b"battleship".to_vec()))
        );
        // Historical distinct-variant operands are absent: unresolved unless names agree.
        assert_eq!(
            hull_icon(
                [hull(&cruiser, 5), hull(&battleship, 5)],
                Err("unreached equipment content"),
            ),
            HullIcon::Ambiguous
        );
        let sister = ship(4, 0, 0, 0, 14, "light_cruiser");
        assert_eq!(
            hull_icon(
                [hull(&cruiser, 5), hull(&sister, 5)],
                Err("unreached equipment content"),
            ),
            HullIcon::Resolved(Some(b"light_cruiser".to_vec()))
        );
        // The same variant is not better: the first stays.
        let twin = ship(5, 0, 0, 0, 13, "other_name");
        assert_eq!(
            hull_icon(
                [hull(&cruiser, 5), hull(&twin, 5)],
                Err("unreached equipment content"),
            ),
            HullIcon::Resolved(Some(b"light_cruiser".to_vec()))
        );
        // An invalid definition draws nothing.
        let mut invalid = cruiser.clone();
        invalid.counter.as_mut().unwrap().definition_valid = 0;
        assert_eq!(
            hull_icon([hull(&invalid, 5)], Err("unreached equipment content")),
            HullIcon::Resolved(None)
        );
    }

    fn selected(id: u64, packet: HullVariantInputs) -> ShipModelRow {
        ShipModelRow {
            hull_variant: Recorded::Value(Some(packet)),
            ..ship(id, 0, 0, 0, id, &format!("mod_hull_{id}"))
        }
    }

    fn current_variant(definition_index: i32) -> HullVariantInputs {
        HullVariantInputs {
            definition_index: Some(definition_index),
            module_count: Some(0),
            upgrade_level_sum: Some(0),
            module_priority_sum: Some(0),
            version: Some(0),
            organisation_marker: Some(0),
        }
    }

    fn definition_index(database: &installed_content::equipment::Database, name: &[u8]) -> i32 {
        database
            .equipment
            .iter()
            .position(|definition| definition.name == name)
            .unwrap() as i32
            + 1
    }

    #[test]
    fn current_modded_parent_depth_resolves_equal_strength_before_later_fields() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let content = crate::map::test_support::equipment_from_documents(&[br"
            equipments = {
                mod_root = { is_archetype = yes type = destroyer }
                mod_deep = { archetype = mod_root parent = mod_root }
            }
        "]);
        let database = &content;
        let root = selected(
            1,
            HullVariantInputs {
                definition_index: Some(definition_index(database, b"mod_root")),
                ..Default::default()
            },
        );
        let deep = selected(
            2,
            HullVariantInputs {
                definition_index: Some(definition_index(database, b"mod_deep")),
                ..Default::default()
            },
        );
        let hull = |model| Hull { strength: 7, model };
        assert_eq!(
            better_hull(&hull(&deep), &hull(&root), Ok(database)),
            Ok(true)
        );
        assert_eq!(
            hull_icon([hull(&root), hull(&deep)], Ok(database)),
            HullIcon::Resolved(Some(b"mod_hull_2".to_vec()))
        );
    }

    #[test]
    fn current_raw_score_wraps_and_equal_scores_use_the_signed_marker() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let content = crate::map::test_support::equipment_from_documents(&[br"
            equipments = { mod_root = { is_archetype = yes type = destroyer } }
        "]);
        let database = &content;
        let index = definition_index(database, b"mod_root");
        let a = HullVariantInputs {
            module_count: Some(i32::MAX),
            upgrade_level_sum: Some(1),
            version: None,
            organisation_marker: None,
            ..current_variant(index)
        };
        let b = HullVariantInputs {
            version: None,
            organisation_marker: None,
            ..current_variant(index)
        };
        assert_eq!(
            preferred_variant(&selected(1, a), &selected(2, b), Ok(database)),
            Ok(false)
        );

        let a = HullVariantInputs {
            module_count: Some(1),
            upgrade_level_sum: Some(2),
            module_priority_sum: Some(3),
            version: None,
            organisation_marker: Some(-2),
            ..current_variant(index)
        };
        let b = HullVariantInputs {
            module_count: Some(6),
            version: None,
            organisation_marker: Some(-5),
            ..current_variant(index)
        };
        assert_eq!(
            preferred_variant(&selected(1, a), &selected(2, b), Ok(database)),
            Ok(true)
        );
        let missing = HullVariantInputs {
            organisation_marker: None,
            ..a
        };
        assert!(
            preferred_variant(&selected(1, missing), &selected(2, b), Ok(database))
                .unwrap_err()
                .contains("organisation marker")
        );
        let missing = HullVariantInputs {
            upgrade_level_sum: None,
            ..a
        };
        assert!(
            preferred_variant(&selected(1, missing), &selected(2, b), Ok(database))
                .unwrap_err()
                .contains("upgrade level sum")
        );
    }

    #[test]
    fn left_recursive_plane_classification_uses_unsigned_versions_without_marker_fallthrough() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let content = crate::map::test_support::equipment_from_documents(&[br"
            equipments = {
                mod_plane = { is_archetype = yes type = fighter }
                mod_mixed = { archetype = mod_plane type = armor }
                mod_ship = { is_archetype = yes type = destroyer }
            }
        "]);
        let database = &content;
        let mixed = definition_index(database, b"mod_mixed");
        let ship_index = definition_index(database, b"mod_ship");
        // Own non-plane categories do not hide a plane archetype. The predicate is on the left.
        let a = HullVariantInputs {
            module_count: Some(1),
            version: Some(255),
            organisation_marker: None,
            ..current_variant(mixed)
        };
        let b = HullVariantInputs {
            module_count: Some(2),
            version: Some(1),
            organisation_marker: None,
            ..current_variant(ship_index)
        };
        assert_eq!(
            preferred_variant(&selected(1, a), &selected(2, b), Ok(database)),
            Ok(true)
        );
        assert_eq!(
            preferred_variant(&selected(2, b), &selected(1, a), Ok(database)),
            Ok(true)
        );
        let tied = HullVariantInputs {
            version: Some(1),
            ..a
        };
        assert_eq!(
            preferred_variant(&selected(1, tied), &selected(2, b), Ok(database)),
            Ok(false)
        );
        let unreadable = HullVariantInputs { version: None, ..a };
        assert!(
            preferred_variant(&selected(1, unreadable), &selected(2, b), Ok(database))
                .unwrap_err()
                .contains("version")
        );
        let mut cyclic = database.clone();
        let root = definition_index(database, b"mod_plane") as usize - 1;
        cyclic.equipment[root].archetype = Some(mixed as usize - 1);
        assert!(
            preferred_variant(&selected(1, a), &selected(2, b), Ok(&cyclic))
                .unwrap_err()
                .contains("archetype cycle")
        );
    }

    #[test]
    fn earlier_hull_branches_do_not_demand_current_variant_or_content() {
        let a = ship(1, 0, 0, 0, 11, "a");
        let mut b = ship(2, 0, 0, 0, 12, "b");
        fn hull(model: &ShipModelRow, strength: i64) -> Hull<'_> {
            Hull { model, strength }
        }
        assert_eq!(
            better_hull(&hull(&a, 8), &hull(&b, 7), Err("absent content")),
            Ok(true)
        );
        b.variant = 0;
        assert_eq!(
            better_hull(&hull(&a, 7), &hull(&b, 7), Err("absent content")),
            Ok(true)
        );
        assert_eq!(
            better_hull(&hull(&b, 7), &hull(&a, 7), Err("absent content")),
            Ok(false)
        );
        b.variant = a.variant;
        assert_eq!(
            better_hull(&hull(&a, 7), &hull(&b, 7), Err("absent content")),
            Ok(false)
        );
        b.variant = 12;
        assert!(
            better_hull(&hull(&a, 7), &hull(&b, 7), Err("absent content"))
                .unwrap_err()
                .contains("ShipModels.hull_variant")
        );
    }

    #[test]
    // Database's private indices require constructing this fixture via Default.
    #[allow(clippy::field_reassign_with_default)]
    fn authenticated_null_definition_compares_without_content_and_preserves_faults() {
        let null = HullVariantInputs {
            organisation_marker: Some(-2),
            version: None,
            ..current_variant(-1)
        };
        let other = HullVariantInputs {
            organisation_marker: Some(-5),
            ..null
        };
        let mut incomplete = installed_content::equipment::Database::default();
        incomplete.lost = Some("fixture definition gap".into());
        for equipment in [Err("absent content"), Ok(&incomplete)] {
            assert_eq!(
                preferred_variant(&selected(1, null), &selected(2, other), equipment),
                Ok(true)
            );
            let scored = HullVariantInputs {
                module_count: Some(1),
                organisation_marker: None,
                ..null
            };
            assert_eq!(
                preferred_variant(&selected(1, scored), &selected(2, other), equipment),
                Ok(true)
            );
        }
        for invalid in [None, Some(0), Some(-2)] {
            let packet = HullVariantInputs {
                definition_index: invalid,
                ..null
            };
            assert!(
                preferred_variant(
                    &selected(1, packet),
                    &selected(2, other),
                    Err("absent content")
                )
                .unwrap_err()
                .contains("definition index")
            );
        }
        let mut unreadable = selected(1, null);
        unreadable.hull_variant = Recorded::Value(None);
        assert!(
            preferred_variant(&unreadable, &selected(2, other), Err("absent content"))
                .unwrap_err()
                .contains("unreadable")
        );
    }

    #[test]
    fn null_and_real_definitions_keep_depth_and_left_plane_precedence() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let content = crate::map::test_support::equipment_from_documents(&[br"
            equipments = {
                mod_plane = { is_archetype = yes type = fighter }
                mod_deep = { archetype = mod_plane parent = mod_plane }
            }
        "]);
        let database = &content;
        let null = HullVariantInputs {
            module_count: Some(2),
            version: Some(0),
            organisation_marker: None,
            ..current_variant(-1)
        };
        let plane = HullVariantInputs {
            module_count: Some(1),
            version: Some(255),
            organisation_marker: None,
            ..current_variant(definition_index(database, b"mod_plane"))
        };
        assert_eq!(
            preferred_variant(&selected(1, null), &selected(2, plane), Ok(database)),
            Ok(true)
        );
        assert_eq!(
            preferred_variant(&selected(2, plane), &selected(1, null), Ok(database)),
            Ok(true)
        );
        let deep = HullVariantInputs {
            definition_index: Some(definition_index(database, b"mod_deep")),
            ..Default::default()
        };
        let null = HullVariantInputs {
            definition_index: Some(-1),
            ..Default::default()
        };
        assert_eq!(
            preferred_variant(&selected(2, deep), &selected(1, null), Ok(database)),
            Ok(true)
        );
        let mut incomplete = database.clone();
        incomplete.lost = Some("fixture definition gap".into());
        for equipment in [Err("absent content"), Ok(&incomplete)] {
            assert!(preferred_variant(&selected(2, deep), &selected(1, null), equipment).is_err());
        }
    }

    #[test]
    fn the_air_map_mode_tests_the_first_carrier_for_an_air_base() {
        let cruiser = ship(1, CAPITAL_FLAG, 0, 0, 0, "cruiser");
        let carrier = ship(2, CARRIER_FLAG, 0, 900, 0, "carrier");
        let empty_carrier = ship(3, 0, MISSILE_CARRIER_FLAG, 0, 0, "missile carrier");
        assert_eq!(
            first_carrier_has_air_base(&[&cruiser, &carrier]),
            Some(true)
        );
        // The first carrier decides, even without an air base of its own.
        assert_eq!(
            first_carrier_has_air_base(&[&empty_carrier, &carrier]),
            Some(false)
        );
        assert_eq!(first_carrier_has_air_base(&[&cruiser]), Some(false));
        assert_eq!(first_carrier_has_air_base(&[]), Some(false));
        // A ship before the first carrier without model operands leaves it open.
        let unknown = ShipModelRow {
            ship: 4,
            ..Default::default()
        };
        assert_eq!(first_carrier_has_air_base(&[&unknown, &carrier]), None);
    }

    #[test]
    fn carrier_icons_follow_access_kind_and_stationed_wings() {
        let carrier = ship(1, CARRIER_FLAG, 0, 900, 0, "carrier");
        let base = |access, kind, stationed| {
            move |_: u64| {
                Some(CarrierBase {
                    access,
                    kind,
                    stationed,
                })
            }
        };
        assert_eq!(
            task_force_carrier(&[&carrier], &base(true, 0, Some(true))),
            BTreeSet::from([2])
        );
        assert_eq!(
            task_force_carrier(&[&carrier], &base(true, 1, Some(false))),
            BTreeSet::from([3])
        );
        assert_eq!(
            task_force_carrier(&[&carrier], &base(false, 0, Some(true))),
            BTreeSet::from([0])
        );
        assert_eq!(
            task_force_carrier(&[&carrier], &base(true, 0, None)),
            BTreeSet::from([1, 2])
        );
        assert_eq!(carrier_icon(2), Some((AIR_BASE_ICON, 1)));
        assert_eq!(carrier_icon(3), Some((ROCKET_SITE_ICON, 0)));
        assert_eq!(carrier_icon(0), None);
        // The ordinary counter keeps the maximum.
        assert_eq!(
            counter_carrier(&[BTreeSet::from([1]), BTreeSet::from([4])]),
            BTreeSet::from([4])
        );
        // The port counter stops scanning after a stationed air base.
        assert_eq!(
            port_carrier(&[BTreeSet::from([2]), BTreeSet::from([4])]),
            BTreeSet::from([Some((AIR_BASE_ICON, 0))])
        );
        assert_eq!(
            port_carrier(&[BTreeSet::from([4]), BTreeSet::from([2])]),
            BTreeSet::from([Some((AIR_BASE_ICON, 1))])
        );
        assert_eq!(port_carrier(&[BTreeSet::from([0])]), BTreeSet::from([None]));
    }

    #[test]
    fn the_portrait_holder_search_is_native_and_the_leader_fallbacks_follow_it() {
        let entry = |category, size, name: &str| CharacterPortrait {
            category,
            size,
            is_set: 1,
            kind: 0,
            identity: name.as_bytes().to_vec(),
        };
        let holder = vec![
            entry(1, 0, "army_small"),
            entry(0, 1, "civ_large"),
            entry(2, 1, "navy_large"),
        ];
        assert_eq!(
            portrait_entry(&holder, 2, 1).unwrap().identity,
            b"navy_large"
        );
        // No army large: the first entry of that size.
        assert_eq!(
            portrait_entry(&holder, 1, 1).unwrap().identity,
            b"civ_large"
        );
        assert!(portrait_entry(&holder, 1, 2).is_none());
        let character = CharacterPortraitRow {
            character: 7,
            portraits: holder,
        };
        let leader = LeaderPortraitRow {
            leader: 1,
            character: (7 << 32) | 73,
            ..Default::default()
        };
        assert_eq!(
            hq_portrait(Some(&leader), Some(2), Some(&character), None),
            Portrait::Sprite(b"navy_large".to_vec())
        );
        assert_eq!(leader_category(0), 1);
        assert_eq!(leader_category(3), 4);
        let generic = LeaderPortraitRow {
            leader: 2,
            portrait_c0: b"gfx/leaders/Europe/portrait_europe_generic_land_17.dds".to_vec(),
            portrait_100: b"GFX_ignored".to_vec(),
            ..Default::default()
        };
        assert_eq!(
            hq_portrait(Some(&generic), Some(0), None, None),
            Portrait::FallbackIfUnresolved(Box::new(Portrait::File(generic.portrait_c0.clone())))
        );
        assert_eq!(
            hq_portrait(None, None, None, None),
            Portrait::Sprite(UNKNOWN_LEADER_PORTRAIT.as_bytes().to_vec())
        );
        // Format 24's recorded tag completes the portrait directory branch.
        let pictured = LeaderPortraitRow {
            leader: 3,
            portrait_e0: b"portrait_generic_land_4.dds".to_vec(),
            ..Default::default()
        };
        assert_eq!(
            hq_portrait(Some(&pictured), Some(0), None, Some(b"ENG")),
            Portrait::FallbackIfUnresolved(Box::new(Portrait::File(
                b"gfx/leaders/ENG/portrait_generic_land_4.dds".to_vec()
            )))
        );
        assert_eq!(
            hq_portrait(Some(&pictured), Some(0), None, None),
            Portrait::Unrecorded("leader picture directory")
        );
        // A character reference with no row never reaches the leader's portrait strings.
        let dangling = LeaderPortraitRow {
            leader: 4,
            character: (9 << 32) | 73,
            portrait_c0: b"gfx/leaders/Europe/portrait_europe_generic_land_17.dds".to_vec(),
            ..Default::default()
        };
        assert_eq!(
            hq_portrait(Some(&dangling), Some(0), None, None),
            Portrait::Unrecorded("character row")
        );
    }
}
