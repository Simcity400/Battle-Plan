//! The naval battle title and naval result name: 1415B06C0.
//!
//! With no port-strike result (RefreshCombatDetails 1417DA2F0 passes none; a
//! result passes itself and 1415B06C0 tests its +0x128 byte) the name is the
//! battle province's strategic region name, replaced by the localised name of
//! the best-scoring province of the region's states once that score reaches
//! `NNavy.BATTLE_NAME_VP_CUTOFF`, all through `NAVAL_BATTLE_TITLE` with the
//! text argument `NAME` (142727E7C). A port strike localises
//! `NAVAL_PORT_STRIKE_TITLE` with the region name alone.
use super::binding::{TextIcons, builtin, token};
use crate::{
    document::Document,
    table_access::{Available, rows},
};
use installed_content::{
    localisation::{Argument, Localisation},
    regions::StrategicRegion,
    states::StateDefinition,
};
use replay::{frame::DecodedFrame, rows::tables};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

/// Every strategic region's candidate provinces in 1415B06C0's order.
///
/// - A state's province list (CState+0x18) is its template's list
///   (140AAF580 appends each valid province of every `provinces` block, in
///   order) with repeats dropped: CState::InitializeProvinces (1409D1F20)
///   logs a repeat and does not append it again.
/// - A province's state (CProvince+0xC0) is the last state to list it:
///   CGameState::InitializeProvinces (1401CD490) initialises the states in
///   id order (the state table is indexed by id), and 1409D1F20 overwrites
///   the backlink ("thus overriding first state") while the province stays
///   in both lists.
/// - The region's states (CStrategicRegion+0x68) are appended by the region
///   rebuild 140E0F2B0 at the first of their provinces in the region
///   template's province order, and each brings its whole list, whether or
///   not those provinces lie in the region.
///
/// `valid` is whether a province id names a map province: the template
/// reader and the region rebuild skip any other.
pub(crate) fn battle_name_candidates(
    regions: &BTreeMap<u32, StrategicRegion>,
    states: &BTreeMap<u32, StateDefinition>,
    valid: &dyn Fn(u32) -> bool,
) -> BTreeMap<u32, Vec<u32>> {
    let lists: BTreeMap<u32, Vec<u32>> = states
        .iter()
        .map(|(id, state)| {
            let mut seen = BTreeSet::new();
            let list = state
                .provinces
                .iter()
                .copied()
                .filter(|province| valid(*province) && seen.insert(*province))
                .collect();
            (*id, list)
        })
        .collect();
    let mut state_of = BTreeMap::new();
    for (id, list) in &lists {
        for province in list {
            state_of.insert(*province, *id);
        }
    }
    regions
        .iter()
        .map(|(id, region)| {
            let mut seen = BTreeSet::new();
            let mut provinces = Vec::new();
            for province in region.provinces.iter().filter(|province| valid(**province)) {
                if let Some(state) = state_of.get(province)
                    && seen.insert(*state)
                {
                    provinces.extend(&lists[state]);
                }
            }
            (*id, provinces)
        })
        .collect()
}

/// 1424DAE00: the fixed-point Newton square root, in native wrapping i64.
fn newton_root(value: i64) -> i64 {
    if value < 2 {
        return 0;
    }
    let mut root = if value < i64::MAX {
        value.wrapping_mul(100_000) / 200_000
    } else {
        0xffff_ffff
    };
    let tolerance = value.wrapping_mul(1_000) / 100_000;
    loop {
        let error = (root.wrapping_mul(root) / 100_000).wrapping_sub(value);
        let twice = root.wrapping_mul(2);
        let step = if twice == 0 || matches!(error, i64::MAX | i64::MIN | -0x7fff_ffff_ffff_ffff) {
            0xffff_ffff
        } else {
            error.wrapping_mul(100_000).wrapping_div(twice)
        };
        root = root.wrapping_sub(step);
        if !((tolerance < error || error < tolerance.wrapping_neg()) && step != 0) {
            return root;
        }
    }
}

/// 1424D8800: the square root of a fixed5 value, scaling small and large
/// inputs around 1424DAE00.
fn fixed_root(value: i64) -> i64 {
    if value < 0x2711 {
        return newton_root(value.wrapping_mul(100)).wrapping_mul(10_000) / 100_000;
    }
    let mut value = value;
    let mut scale = 0;
    while value >= 3_000_000_000 {
        value = value.wrapping_mul(1_000) / 100_000;
        scale += 1;
    }
    let mut root = newton_root(value);
    for _ in 0..scale {
        root = root.wrapping_mul(10);
    }
    root
}

/// 1424D88E0: the fixed5 square root of an integer; a value with any of its
/// top eleven bits set is shifted right ten first and the root left five.
fn integer_root(value: u32) -> i64 {
    if (value as i32) < 1 {
        return 0;
    }
    if value & 0xffe0_0000 != 0 {
        let scaled = (((value as i32) as i64 as u64) >> 10) as i64 * 100_000;
        return fixed_root(scaled) << 5;
    }
    fixed_root(i64::from(value as i32) * 100_000)
}

/// 140A51B90 (through CProvince::GetWrappedMapDistance 140E67850): zero for
/// the same province, else the fixed5 length of the integer position deltas,
/// the horizontal one wrapped across the map width, in native i32 arithmetic.
fn wrapped_distance(width: i32, from: (u32, [i32; 2]), to: (u32, [i32; 2])) -> i64 {
    if from.0 == to.0 {
        return 0;
    }
    let dx = from.1[0].wrapping_sub(to.1[0]);
    let dy = from.1[1].wrapping_sub(to.1[1]);
    let across = dx.wrapping_abs();
    let dx = if across <= width / 2 {
        dx
    } else {
        width.wrapping_sub(across)
    };
    integer_root(dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx)) as u32)
}

/// What 1415B06C0's province pick reads besides the candidates.
pub(super) struct Picker<'a> {
    /// CMap+0x40, the map width the horizontal delta wraps across.
    pub(super) width: i32,
    /// `NNavy.BATTLE_NAME_VP_FACTOR` and `NNavy.BATTLE_NAME_VP_CUTOFF`, fixed5.
    pub(super) factor: i64,
    pub(super) cutoff: i64,
    /// The province definition's map position (+0xB0).
    pub(super) position: &'a dyn Fn(u32) -> [i32; 2],
    /// The province's current victory points (CProvince+0x38), or why unknown.
    pub(super) victory_points: &'a dyn Fn(u32) -> Result<i32, String>,
}

impl Picker<'_> {
    /// The province whose name replaces the region's, if any: the first
    /// candidate is taken, a later one only with a strictly greater score
    /// (victory points times the factor over the distance; a zero distance or
    /// an unrepresentable product scores 0xFFFFFFFF), and it names the battle
    /// once its score reaches the cutoff.
    pub(super) fn pick(&self, battle: u32, candidates: &[u32]) -> Result<Option<u32>, String> {
        let at = (battle, (self.position)(battle));
        let mut best: Option<(u32, i64)> = None;
        for &candidate in candidates {
            let distance =
                wrapped_distance(self.width, at, (candidate, (self.position)(candidate)));
            let raw = i64::from((self.victory_points)(candidate)?)
                .wrapping_mul(100_000)
                .wrapping_mul(self.factor)
                / 100_000;
            let score =
                if distance == 0 || matches!(raw, i64::MAX | i64::MIN | -0x7fff_ffff_ffff_ffff) {
                    0xffff_ffff
                } else {
                    raw.wrapping_mul(100_000) / distance
                };
            if best.is_none_or(|(_, top)| top < score) {
                best = Some((candidate, score));
            }
        }
        Ok(best
            .filter(|(_, score)| self.cutoff <= *score)
            .map(|(province, _)| province))
    }
}

/// The recorded province facts 140E69320's name ladder reads.
struct Province<'a> {
    id: u32,
    /// The recorded rename (CProvince+0x40, `ProvinceNames`): `Ok(None)` for
    /// none, or why the hour cannot say.
    rename: Result<Option<&'a [u8]>, String>,
    /// The tag 140E68E30 qualifies the victory point key with: the
    /// province's state owner's original tag or tag (CCountry::
    /// GetOriginalTagOrTag 1406DF830), or why it is unavailable.
    vp_tag: Result<&'a str, String>,
    /// The province's recorded state (CProvince+0xC0, `Provinces.state`):
    /// `None` for no state, else the state's recorded names and the name key
    /// of its definition, or why they are unavailable.
    state: Option<Result<StateNames<'a>, String>>,
    /// The key of the province's strategic region name (CProvince+0xC8),
    /// which the ladder takes for a province without a state.
    region_name_key: Option<&'a str>,
}

/// What CState::display_name (1409CC380) reads.
struct StateNames<'a> {
    id: u32,
    /// CState+0x38 (`States.custom_name`) and +0xE8 (`States.owner_name`).
    custom_name: &'a [u8],
    owner_name: &'a [u8],
    /// The definition's `name` key (CStateTemplate+0x40), or why the
    /// definition's reading cannot be followed.
    name_key: Result<&'a str, &'a str>,
}

/// CState::display_name (1409CC380): the +0x38 string when it has a length,
/// else the +0xE8 string when it has one, else the template's name (+0x20),
/// which CStateTemplate's reader (140AAF580) localises from the `name` key
/// when the file is read (a key the localisation lacks is its own text).
/// The two recorded strings are already localised text and are copied as
/// they are (140E69320 copies the returned string).
fn state_display_name(
    icons: &dyn TextIcons,
    loc: &Localisation,
    state: &StateNames<'_>,
) -> Result<String, String> {
    let id = state.id;
    for (field, bytes) in [
        ("custom_name", state.custom_name),
        ("owner_name", state.owner_name),
    ] {
        if !bytes.is_empty() {
            return String::from_utf8(bytes.to_vec())
                .map_err(|_| format!("state {id}'s recorded {field} is not UTF-8"));
        }
    }
    let key = state
        .name_key
        .map_err(|reason| format!("state {id}'s definition name is unknown: {reason}"))?;
    token(icons, loc, key.as_bytes()).map_err(|reason| format!("state {id}'s name: {reason}"))
}

/// 140E69320 for a province of a state: a nonempty recorded rename through
/// the localiser (a key the localisation lacks is its own text); else
/// ResolveVictoryPointLocalization (140E68E30), which localises the first of
/// `<TAG>_VICTORY_POINTS_<id>` and `VICTORY_POINTS_<id>` the localisation
/// holds, or gives nothing; a nonempty result is the name. Otherwise the
/// state's display name ([`state_display_name`]), else for a province
/// without a state its strategic region's name, else its id.
fn province_name(
    icons: &dyn TextIcons,
    loc: &Localisation,
    province: &Province<'_>,
) -> Result<String, String> {
    let id = province.id;
    let rename = province.rename.as_ref().map_err(|reason| {
        format!("province {id}'s recorded rename (CProvince+0x40) is unknown: {reason}")
    })?;
    if let Some(bytes) = rename.filter(|bytes| !bytes.is_empty()) {
        return token(icons, loc, bytes)
            .map_err(|reason| format!("province {id}'s recorded rename: {reason}"));
    }
    let tag = province.vp_tag.as_deref().map_err(|reason| {
        format!("province {id}'s qualified victory point key (140E68E30) needs a tag, but {reason}")
    })?;
    // 140E68E30 picks one key, and its localised text, empty included, is
    // the result; 140E69320 goes on only when that is empty.
    let chosen = [
        format!("{tag}_VICTORY_POINTS_{id}"),
        format!("VICTORY_POINTS_{id}"),
    ]
    .into_iter()
    .find(|key| loc.get(key).is_some());
    if let Some(key) = chosen {
        let text = token(icons, loc, key.as_bytes())?;
        if !text.is_empty() {
            return Ok(text);
        }
    }
    match &province.state {
        Some(Ok(state)) => state_display_name(icons, loc, state),
        Some(Err(reason)) => Err(format!(
            "province {id}'s state display name (CState::display_name 1409CC380): {reason}"
        )),
        None => match province.region_name_key {
            Some(key) => token(icons, loc, key.as_bytes())
                .map_err(|reason| format!("province {id}'s region name: {reason}")),
            None => Ok(id.to_string()),
        },
    }
}

/// The naval battle title for `province`, or for a result whose port-strike
/// byte (+0x128) is `port_strike`; the error names the missing operand.
pub(super) fn title(
    document: &Document,
    frame: &DecodedFrame,
    province: u32,
    port_strike: bool,
) -> io::Result<Result<String, String>> {
    let (icons, loc) = (&document.icons.combat, &document.names.localisation);
    let Some(region) = document.counters.region(province) else {
        return Ok(Err(format!(
            "province {province} is in no content strategic region"
        )));
    };
    let Some(definition) = document.strategic_regions.get(&region) else {
        return Ok(Err(format!(
            "strategic region {region} has no content definition"
        )));
    };
    // CStrategicRegion::GetName (140E0EB10): the template's name, localised
    // when the definition is read (1415938B0); the instance override is only
    // a nudger edit or a copy of that same name.
    let region_name = match token(icons, loc, definition.name_key.as_bytes()) {
        Ok(name) => name,
        Err(reason) => return Ok(Err(format!("strategic region {region}'s name: {reason}"))),
    };
    if port_strike {
        return Ok(builtin(
            icons,
            loc,
            "NAVAL_PORT_STRIKE_TITLE",
            &[("NAME", Argument::Text(&region_name))],
        ));
    }
    let [factor, cutoff] = match &document.icons.combat.naval_name_defines {
        Ok(defines) => *defines,
        Err(reason) => {
            return Ok(Err(format!(
                "the NNavy battle name defines are unavailable ({reason})"
            )));
        }
    };
    let provinces = rows::<tables::Provinces>(frame)?;
    let complete = provinces.is_complete();
    let provinces: BTreeMap<_, _> = provinces.rows().iter().map(|row| (row.id, row)).collect();
    let victory_points = |id: u32| {
        provinces.get(&id).map(|row| row.victory_points).ok_or_else(|| {
            if complete {
                format!("province {id} has no Provinces row")
            } else {
                format!("the Provinces table is incomplete this hour, so province {id}'s victory points are unknown")
            }
        })
    };
    let geometry = &document.province_geometry;
    // A province with no pixels keeps the zero position 1413F4FD0 leaves.
    let position = |id: u32| geometry.map_position(id).unwrap_or([0, 0]);
    let picker = Picker {
        width: document.map.width as i32,
        factor,
        cutoff,
        position: &position,
        victory_points: &victory_points,
    };
    let candidates = document
        .battle_name_provinces
        .get(&region)
        .map_or(&[][..], Vec::as_slice);
    let name = match picker.pick(province, candidates) {
        Err(reason) => return Ok(Err(reason)),
        Ok(None) => region_name,
        Ok(Some(picked)) => {
            let renames = rows::<tables::ProvinceNames>(frame)?;
            // The sparse table proves no rename only when complete; a row
            // present in a partial one is still the recorded rename.
            let rename = match renames
                .rows()
                .iter()
                .rev()
                .find(|row| row.province == picked)
            {
                Some(row) => Ok(Some(row.name.as_slice())),
                None => match &renames {
                    Available::Complete(_) => Ok(None),
                    Available::Truncated { missing, .. } => Err(format!(
                        "the ProvinceNames table is truncated this hour ({missing} rows not reached) and holds no row for it"
                    )),
                    Available::Unavailable { reason } => Err(format!(
                        "the ProvinceNames table is unavailable this hour ({reason})"
                    )),
                },
            };
            // The picked province's state from Provinces.state, its names
            // from the States row and its definition's `name` key.
            let states = rows::<tables::States>(frame)?;
            let states_complete = states.is_complete();
            let vp_tag = match provinces.get(&picked) {
                None => Err(format!("province {picked} has no Provinces row")),
                Some(row) => vp_tag(frame, row.state, states.rows(), states_complete)?,
            };
            let state = provinces.get(&picked).and_then(|row| {
                (row.state != 0).then(|| {
                    let id = row.state;
                    let recorded = states.rows().iter().find(|state| state.id == id);
                    let recorded = recorded.ok_or_else(|| {
                        if states_complete {
                            format!("state {id} has no States row")
                        } else {
                            format!("the States table is incomplete this hour, so state {id}'s names are unknown")
                        }
                    })?;
                    let name_key = document
                        .state_name_keys
                        .get(&id)
                        .ok_or_else(|| format!("state {id} has no content definition"))?;
                    Ok(StateNames {
                        id,
                        custom_name: &recorded.custom_name,
                        owner_name: &recorded.owner_name,
                        name_key: name_key.as_deref().map_err(String::as_str),
                    })
                })
            });
            let state = match provinces.get(&picked) {
                Some(_) => state,
                None => Some(Err(format!("province {picked} has no Provinces row"))),
            };
            let region_name_key = document
                .counters
                .region(picked)
                .and_then(|region| document.strategic_regions.get(&region))
                .map(|region| region.name_key.as_str());
            match province_name(
                icons,
                loc,
                &Province {
                    id: picked,
                    rename,
                    vp_tag: vp_tag.as_deref().map_err(Clone::clone),
                    state,
                    region_name_key,
                },
            ) {
                Ok(name) => name,
                Err(reason) => return Ok(Err(reason)),
            }
        }
    };
    Ok(builtin(
        icons,
        loc,
        "NAVAL_BATTLE_TITLE",
        &[("NAME", Argument::Text(&name))],
    ))
}

/// The tag 140E68E30 qualifies a province's victory point key with, through
/// CCountry::GetOriginalTagOrTag (1406DF830), which takes the original tag
/// (+0x130C) when it is positive. CountryAppearance.name_tag records that
/// original tag, empty when it is not positive (engine::ccountry, whose
/// recorder cites the writers). The country is the province's state owner
/// (CState+0xC8), resolved by resolve_country_reference (140BA56A0): a null
/// link is country index 0, any other its canonical slot (140BA6240). A
/// province without a state takes ConstructCountryLinkFromCountryIndex(0)
/// (140BA4BB0): FindCountryLinkFromSlot (1401DA340) gives the first link to
/// index 0, or 0 itself, which both resolve to the country at index 0.
fn vp_tag(
    frame: &DecodedFrame,
    state: u32,
    states: &[replay::rows::StateRow],
    states_complete: bool,
) -> io::Result<Result<String, String>> {
    let (subject, slot) = if state == 0 {
        (
            "the country at index 0, which a province without a state qualifies with (140BA4BB0)"
                .to_owned(),
            0,
        )
    } else {
        let Some(row) = states.iter().find(|row| row.id == state) else {
            return Ok(Err(if states_complete {
                format!("state {state} has no States row")
            } else {
                format!(
                    "the States table is incomplete this hour, so state {state}'s owner is unknown"
                )
            }));
        };
        let owner = row.owner;
        let slot = if owner == 0 {
            0
        } else {
            let links = rows::<tables::CountryLinks>(frame)?;
            let Some(slot) = links
                .rows()
                .iter()
                .find(|link| link.id == owner)
                .map(|link| link.canonical_slot)
            else {
                return Ok(Err(if links.is_complete() {
                    format!("state {state}'s owner link {owner} has no CountryLinks row")
                } else {
                    format!(
                        "the CountryLinks table is incomplete this hour, so state {state}'s owner link {owner} is unresolved"
                    )
                }));
            };
            slot
        };
        (format!("state {state}'s owner (slot {slot})"), slot)
    };
    let countries = rows::<tables::Countries>(frame)?;
    let Some(country) = countries
        .rows()
        .iter()
        .find(|country| country.canonical_slot == slot)
    else {
        return Ok(Err(if countries.is_complete() {
            format!("{subject} has no Countries row")
        } else {
            format!("the Countries table is incomplete this hour, so {subject} has no tag")
        }));
    };
    let appearance = rows::<tables::CountryAppearance>(frame)?;
    let tag = match appearance
        .rows()
        .iter()
        .find(|row| row.country == country.id)
    {
        Some(row) if !row.name_tag.is_empty() => &row.name_tag,
        Some(_) => &country.tag,
        None => {
            return Ok(Err(if appearance.is_complete() {
                format!(
                    "{subject} has no CountryAppearance row, so its original tag (CCountry+0x130C) is unknown"
                )
            } else {
                format!(
                    "the CountryAppearance table is incomplete this hour, so the original tag (CCountry+0x130C) of {subject} is unknown"
                )
            }));
        }
    };
    Ok(String::from_utf8(tag.clone()).map_err(|_| format!("{subject}'s tag is not UTF-8")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::binding::NoIcons;

    #[test]
    fn the_native_root_gives_fixed5_pixel_distances() {
        assert_eq!(integer_root(0), 0);
        // 1424DAE00 stops once the square is within a hundredth of the input
        // (value * 1000 / 100000), so a root sits near, not at, the true one.
        let close = |value: i64, expected: i64| (value - expected).abs() * 10_000 <= expected;
        assert!(close(integer_root(25), 500_000), "{}", integer_root(25));
        assert!(close(integer_root(1), 100_000), "{}", integer_root(1));
        // x 95 and 5 on a 100-wide map are 10 apart across the wrap.
        let wrapped = wrapped_distance(100, (1, [95, 0]), (2, [5, 4]));
        assert!(close(wrapped, 1_077_033), "{wrapped}");
        let plain = wrapped_distance(100, (1, [0, 0]), (2, [3, 4]));
        assert!(close(plain, 500_000), "{plain}");
        assert_eq!(wrapped_distance(100, (7, [0, 0]), (7, [3, 4])), 0);
        // Past 2^21 the input is shifted ten bits and the root five.
        let large = integer_root(4_000_000);
        assert!((199_900_000..=200_100_000).contains(&large), "{large}");
    }

    #[test]
    fn the_first_candidate_is_taken_and_only_a_greater_score_replaces_it() {
        let positions = BTreeMap::from([(1, [0, 0]), (10, [3, 4]), (11, [6, 8]), (12, [0, 1])]);
        let points = BTreeMap::from([(10, 1), (11, 1), (12, 0)]);
        let position = |id: u32| positions.get(&id).copied().unwrap_or([0, 0]);
        let victory_points = |id: u32| {
            points
                .get(&id)
                .copied()
                .ok_or_else(|| format!("province {id} has no Provinces row"))
        };
        let picker = Picker {
            width: 1_000,
            factor: 10_000_000,
            cutoff: 100_000,
            position: &position,
            victory_points: &victory_points,
        };
        // 1 VP at distance 5 scores 20.0; at 10 it scores 10.0.
        assert_eq!(picker.pick(1, &[11, 10]), Ok(Some(10)));
        // No victory points score zero, below the cutoff: the region's name.
        assert_eq!(picker.pick(1, &[12]), Ok(None));
        assert_eq!(picker.pick(1, &[]), Ok(None));
        // The battle province itself is at distance zero and scores 0xFFFFFFFF.
        let own = BTreeMap::from([(1, 0)]);
        let own_points = |id: u32| Ok(own.get(&id).copied().unwrap_or(1));
        let picker_own = Picker {
            victory_points: &own_points,
            ..picker
        };
        assert_eq!(picker_own.pick(1, &[10, 1]), Ok(Some(1)));
        assert_eq!(
            picker.pick(1, &[99]),
            Err("province 99 has no Provinces row".into())
        );
    }

    #[test]
    fn candidates_are_the_regions_states_in_first_appearance_order() {
        let region = |id, provinces: &[u32]| StrategicRegion {
            id,
            provinces: provinces.to_vec(),
            ..Default::default()
        };
        let state = |id, provinces: &[u32]| StateDefinition {
            id,
            provinces: provinces.to_vec(),
            ..Default::default()
        };
        let regions = BTreeMap::from([(1, region(1, &[30, 20, 21, 5, 404]))]);
        let states = BTreeMap::from([
            (7, state(7, &[20, 21, 22, 21, 404])),
            (8, state(8, &[31, 30])),
        ]);
        let valid = |province: u32| province != 404;
        // Sea province 5 has no state and 404 is no map province; state 8
        // comes first through province 30 and brings 31, which lies outside
        // the region; state 7's repeat of 21 is dropped.
        assert_eq!(
            battle_name_candidates(&regions, &states, &valid)[&1],
            vec![31, 30, 20, 21, 22]
        );
        // Province 20 listed by states 7 and 9 belongs to state 9, the later
        // one initialised, yet stays in both lists.
        let states = BTreeMap::from([(7, state(7, &[20, 21])), (9, state(9, &[20, 23]))]);
        let regions = BTreeMap::from([(1, region(1, &[20, 21]))]);
        assert_eq!(
            battle_name_candidates(&regions, &states, &valid)[&1],
            vec![20, 23, 20, 21]
        );
    }

    /// 140E68E30 qualifies with the state owner's original tag or tag
    /// (1406DF830), from CountryAppearance.name_tag, never the current tag of
    /// a dynamic country; missing rows are refused by table.
    #[test]
    fn the_victory_point_tag_is_the_state_owners_original_tag() {
        use replay::{
            frame::TableView,
            rows::{
                CountryAppearanceRow, CountryLinkRow, CountryRow, StateRow, TableId, TableRows,
            },
        };
        let mut frame = DecodedFrame {
            branch: 0,
            hour: 1,
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "missing".into(),
                }
                .into()
            }),
            metrics: Default::default(),
        };
        let states = [
            StateRow {
                id: 7,
                owner: 70,
                ..Default::default()
            },
            StateRow {
                id: 8,
                owner: 80,
                ..Default::default()
            },
        ];
        assert_eq!(
            vp_tag(&frame, 7, &states, true).unwrap(),
            Err("the CountryLinks table is incomplete this hour, so state 7's owner link 70 is unresolved".into())
        );
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 70,
                    canonical_slot: 700,
                },
                CountryLinkRow {
                    id: 80,
                    canonical_slot: 800,
                },
            ]));
        *frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![
                CountryRow {
                    id: 70,
                    canonical_slot: 700,
                    tag: b"D01".to_vec(),
                    ..Default::default()
                },
                CountryRow {
                    id: 80,
                    canonical_slot: 800,
                    tag: b"JAP".to_vec(),
                    ..Default::default()
                },
            ]));
        assert!(
            vp_tag(&frame, 7, &states, true)
                .unwrap()
                .unwrap_err()
                .starts_with("the CountryAppearance table is incomplete this hour")
        );
        *frame.tables[TableId::CountryAppearance.index()] =
            TableView::Complete(TableRows::CountryAppearance(vec![
                CountryAppearanceRow {
                    country: 70,
                    name_tag: b"CHI".to_vec(),
                    ..Default::default()
                },
                CountryAppearanceRow {
                    country: 80,
                    ..Default::default()
                },
            ]));
        assert_eq!(vp_tag(&frame, 7, &states, true).unwrap(), Ok("CHI".into()));
        assert_eq!(vp_tag(&frame, 8, &states, true).unwrap(), Ok("JAP".into()));
        assert_eq!(
            vp_tag(&frame, 9, &states, true).unwrap(),
            Err("state 9 has no States row".into())
        );
        // No state: the country at index 0 (140BA4BB0), here without an
        // original tag; a null owner link resolves there too (140BA56A0).
        let states = [
            states[0].clone(),
            StateRow {
                id: 5,
                owner: 0,
                ..Default::default()
            },
        ];
        assert_eq!(
            vp_tag(&frame, 0, &states, true).unwrap(),
            Err("the country at index 0, which a province without a state qualifies with (140BA4BB0) has no Countries row".into())
        );
        let TableView::Complete(TableRows::Countries(countries)) =
            &mut *frame.tables[TableId::Countries.index()]
        else {
            unreachable!()
        };
        countries.push(CountryRow {
            id: 1,
            canonical_slot: 0,
            tag: b"REB".to_vec(),
            ..Default::default()
        });
        let TableView::Complete(TableRows::CountryAppearance(appearance)) =
            &mut *frame.tables[TableId::CountryAppearance.index()]
        else {
            unreachable!()
        };
        appearance.push(CountryAppearanceRow {
            country: 1,
            ..Default::default()
        });
        assert_eq!(vp_tag(&frame, 0, &states, true).unwrap(), Ok("REB".into()));
        assert_eq!(vp_tag(&frame, 5, &states, true).unwrap(), Ok("REB".into()));
    }

    #[test]
    fn a_picked_province_is_named_through_the_native_ladder() {
        let loc: Localisation = [
            ("VICTORY_POINTS_10", "Midway"),
            ("JAP_VICTORY_POINTS_10", "Midowei"),
            ("RENAMED", "New Name"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
        let province = |rename: Option<&'static [u8]>, vp_tag| Province {
            id: 10,
            rename: Ok(rename),
            vp_tag,
            state: None,
            region_name_key: Some("STRATEGICREGION_5"),
        };
        assert_eq!(
            province_name(&NoIcons, &loc, &province(None, Ok("USA"))).as_deref(),
            Ok("Midway")
        );
        assert_eq!(
            province_name(&NoIcons, &loc, &province(None, Ok("JAP"))).as_deref(),
            Ok("Midowei")
        );
        assert_eq!(
            province_name(&NoIcons, &loc, &province(Some(b"RENAMED"), Ok("JAP"))).as_deref(),
            Ok("New Name")
        );
        assert_eq!(
            province_name(&NoIcons, &loc, &province(Some(b"Literal"), Err("x".into()))).as_deref(),
            Ok("Literal")
        );
        // Without a victory point name and without a state, the region name.
        assert_eq!(
            province_name(
                &NoIcons,
                &Localisation::default(),
                &province(None, Ok("USA"))
            )
            .as_deref(),
            Ok("STRATEGICREGION_5")
        );
        // 140E68E30 picks the qualified key once: its empty text goes to the
        // state ladder, never to the generic key.
        let empty: Localisation = [
            ("VICTORY_POINTS_10", "Midway"),
            ("JAP_VICTORY_POINTS_10", ""),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
        assert_eq!(
            province_name(&NoIcons, &empty, &province(None, Ok("JAP"))).as_deref(),
            Ok("STRATEGICREGION_5")
        );
        // A rename the hour cannot rule out is refused by province and cause.
        let unknown = Province {
            rename: Err("the ProvinceNames table is unavailable this hour (x)".into()),
            ..province(None, Ok("USA"))
        };
        assert_eq!(
            province_name(&NoIcons, &loc, &unknown),
            Err("province 10's recorded rename (CProvince+0x40) is unknown: the ProvinceNames table is unavailable this hour (x)".into())
        );
        // CState::display_name (1409CC380): the rename, else the owner name,
        // else the definition's localised name key.
        let loc: Localisation = [("STATE_7", "Bohemia")]
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect();
        let named = |custom_name: &'static [u8], owner_name: &'static [u8]| Province {
            id: 10,
            rename: Ok(None),
            vp_tag: Ok("USA"),
            state: Some(Ok(StateNames {
                id: 7,
                custom_name,
                owner_name,
                name_key: Ok("STATE_7"),
            })),
            region_name_key: None,
        };
        assert_eq!(
            province_name(&NoIcons, &loc, &named(b"Sudetenland", b"Bohmen")).as_deref(),
            Ok("Sudetenland")
        );
        assert_eq!(
            province_name(&NoIcons, &loc, &named(b"", b"Bohmen")).as_deref(),
            Ok("Bohmen")
        );
        assert_eq!(
            province_name(&NoIcons, &loc, &named(b"", b"")).as_deref(),
            Ok("Bohemia")
        );
        assert_eq!(
            province_name(&NoIcons, &Localisation::default(), &named(b"", b"")).as_deref(),
            Ok("STATE_7")
        );
        assert_eq!(
            province_name(&NoIcons, &loc, &named(b"\xff", b"")),
            Err("state 7's recorded custom_name is not UTF-8".into())
        );
        let unrecorded = Province {
            state: Some(Err("state 7 has no States row".into())),
            ..named(b"", b"")
        };
        assert!(
            province_name(&NoIcons, &loc, &unrecorded)
                .unwrap_err()
                .contains("state 7 has no States row")
        );
    }
}
