use std::collections::HashMap;
use viewer_runtime::unit_handle::Unit;
use viewer_layers_forces::units::PickDivision;

pub(crate) const UNSETTLED: &str =
    "unit model pick: a division behind the model has no recorded stack kind, transfer, route hop or roster owner this hour";
pub(crate) const VIEWER_UNKNOWN: &str =
    "unit model pick: the viewing country, whose own unit list the game's picker walks, is not known this hour";
pub(crate) const ORDER_NOT_RECORDED: &str =
    "unit model pick: the models the pointer meets stand for more than one outcome, and the game takes the first in the viewing country's unit list, whose order this recording does not hold";
pub(crate) const ORDER_REFUSED: &str =
    "unit model pick: the models the pointer meets stand for more than one outcome, and the game takes the first in the viewing country's unit list, whose order was refused when read";
pub(crate) const ORDER_LOADING: &str =
    "unit model pick: the models the pointer meets stand for more than one outcome, and the game takes the first in the viewing country's unit list, whose order is loading";
pub(crate) const CLIENT_UNKNOWN: &str =
    "unit model pick: the models the pointer meets stand for more than one outcome, and the game takes the first in the viewing country's unit list, whose order is held for the recording client's country, which this hour does not name";
pub(crate) const OTHER_VIEWPOINT: &str =
    "unit model pick: the models the pointer meets stand for more than one outcome, and the game takes the first in the viewing country's unit list, whose order is held only for the recording client's country, and the map is viewed as another";
pub(crate) const TASK_FORCE_START: &str =
    "unit model pick: the models the pointer meets stand for more than one outcome, and the click's walk starts just after the one selected task force, whose place in the unit list recordings skip";
pub(crate) const PROVINCE_ORDER: &str =
    "unit model pick: the models the pointer meets in its province stand for more than one outcome, and the game takes the first in the province's unit order, which recordings do not hold";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnitOrder<'a> {
    Recorded(&'a [u64]),
    NotRecorded(&'static str),
}

pub(crate) fn order_for<'a>(list: Result<&'a [u64], &'static str>, client: Option<u32>, ours: Option<u32>) -> UnitOrder<'a> {
    match (list, client) {
        (Err(why), _) => UnitOrder::NotRecorded(why),
        (Ok(_), None) => UnitOrder::NotRecorded(CLIENT_UNKNOWN),
        (Ok(list), Some(client)) if Some(client) == ours => UnitOrder::Recorded(list),
        (Ok(_), Some(_)) => UnitOrder::NotRecorded(OTHER_VIEWPOINT),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Walk {
    pub candidates: Vec<PickDivision>,
    pub unordered: &'static str,
}

pub(crate) fn walk(met: Vec<PickDivision>, order: UnitOrder<'_>, ours: Option<u32>, after: Option<Unit>) -> Result<Walk, &'static str> {
    if met.is_empty() {
        return Ok(Walk { candidates: met, unordered: ORDER_NOT_RECORDED });
    }
    match order {
        UnitOrder::Recorded(list) => {
            let mut listed: Vec<(usize, PickDivision)> = met
                .into_iter()
                .filter_map(|division| Some((list.iter().position(|&handle| handle == division.id)?, division)))
                .collect();
            let start = match after {
                None => 0,
                Some(Unit::Division(handle) | Unit::RailwayGun(handle)) => list.iter().position(|&listed| listed == handle).map_or(0, |at| at + 1),
                Some(Unit::TaskForce(_)) => {
                    return Ok(Walk { candidates: listed.into_iter().map(|(_, division)| division).collect(), unordered: TASK_FORCE_START });
                }
            };
            let count = list.len();
            listed.sort_unstable_by_key(|&(at, _)| (at + count - start) % count);
            Ok(Walk { candidates: listed.into_iter().take(1).map(|(_, division)| division).collect(), unordered: ORDER_NOT_RECORDED })
        }
        UnitOrder::NotRecorded(why) => {
            let ours = ours.ok_or(VIEWER_UNKNOWN)?;
            let mut listed = Vec::new();
            for division in met {
                if division.roster.ok_or(UNSETTLED)? == ours {
                    listed.push(division);
                }
            }
            if let Some(Unit::Division(selected) | Unit::RailwayGun(selected)) = after
                && listed.len() > 1
            {
                listed.retain(|division| division.id != selected);
            }
            Ok(Walk { candidates: listed, unordered: why })
        }
    }
}

pub(crate) fn behind(
    met: &[(u32, u32)],
    divisions: &HashMap<u32, Vec<PickDivision>>,
    fallback: bool,
) -> Result<Vec<PickDivision>, &'static str> {
    let mut found: Vec<PickDivision> = Vec::new();
    for &(province, order) in met {
        if province <= u32::from(!fallback) {
            continue;
        }
        for division in divisions.get(&province).into_iter().flatten() {
            let kind = division.kind.ok_or(UNSETTLED)?;
            let unrouted = if fallback { false } else { division.unrouted.ok_or(UNSETTLED)? };
            if kind == order && !unrouted && !found.iter().any(|held| held.id == division.id) {
                found.push(*division);
            }
        }
    }
    Ok(found)
}

pub(crate) fn location(
    candidates: &[PickDivision],
    divisions: &HashMap<u32, Vec<PickDivision>>,
    unordered: &'static str,
) -> Result<u32, &'static str> {
    let mut provinces = candidates.iter().filter_map(|candidate| province_of(candidate.id, divisions));
    let first = provinces.next().ok_or(unordered)?;
    provinces.all(|province| province == first).then_some(first).ok_or(unordered)
}

fn province_of(id: u64, divisions: &HashMap<u32, Vec<PickDivision>>) -> Option<u32> {
    divisions.iter().find(|(_, list)| list.iter().any(|division| division.id == id)).map(|(&province, _)| province)
}

pub(crate) fn roster_owner(candidates: &[PickDivision]) -> Option<u32> {
    let first = candidates.first()?.roster?;
    candidates.iter().all(|candidate| candidate.roster == Some(first)).then_some(first)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Click {
    pub province: u32,
    pub picked: u64,
    pub selected: bool,
    pub ours: bool,
    pub mates: Vec<u64>,
}

pub(crate) fn click(
    candidates: &[PickDivision],
    divisions: &HashMap<u32, Vec<PickDivision>>,
    ours: Option<u32>,
    selected: &dyn Fn(u64) -> bool,
    unordered: &'static str,
) -> Result<Click, &'static str> {
    let first = candidates.first().ok_or(unordered)?;
    let province = location(candidates, divisions, unordered)?;
    let hop = first.hop.ok_or(UNSETTLED)?;
    for candidate in candidates {
        if candidate.hop.ok_or(UNSETTLED)? != hop || candidate.owner != first.owner {
            return Err(unordered);
        }
    }
    let picked_selected = selected(first.id);
    if candidates.iter().any(|candidate| selected(candidate.id) != picked_selected)
        || (picked_selected && candidates.len() > 1)
    {
        return Err(unordered);
    }
    let ours_of = |division: &PickDivision| division.roster.map(|roster| Some(roster) == ours);
    let picked_ours = ours_of(first).ok_or(UNSETTLED)?;
    if candidates.iter().any(|candidate| ours_of(candidate) != Some(picked_ours)) {
        return Err(unordered);
    }
    let mut mates = Vec::new();
    for division in divisions.get(&province).into_iter().flatten() {
        if division.owner != first.owner {
            continue;
        }
        if division.hop.ok_or(UNSETTLED)? == hop && ours_of(division).ok_or(UNSETTLED)? {
            mates.push(division.id);
        }
    }
    Ok(Click { province, picked: first.id, selected: picked_selected, ours: picked_ours, mates })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn division(id: u64, owner: u32, kind: u32, hop: u32) -> PickDivision {
        PickDivision { id, owner, roster: Some(owner), kind: Some(kind), hop: Some(hop), unrouted: Some(false) }
    }

    fn at(province: u32, list: Vec<PickDivision>) -> HashMap<u32, Vec<PickDivision>> {
        HashMap::from([(province, list)])
    }

    #[test]
    fn the_units_behind_a_model_share_its_province_and_stack_kind_and_skip_routeless_transfers() {
        let mut transfer = division(4, 7, 19, 0);
        transfer.unrouted = Some(true);
        let divisions = at(9, vec![division(1, 7, 0, 0), division(2, 7, 3, 12), division(3, 7, 0, 0), transfer]);
        let ids = |found: Vec<PickDivision>| found.iter().map(|division| division.id).collect::<Vec<_>>();
        assert_eq!(ids(behind(&[(9, 0)], &divisions, false).unwrap()), [1, 3]);
        assert_eq!(ids(behind(&[(9, 19)], &divisions, false).unwrap()), Vec::<u64>::new());
        assert_eq!(ids(behind(&[(9, 19)], &divisions, true).unwrap()), [4], "the hover's second walk tests transfers");
        let low = at(1, vec![division(1, 7, 0, 0)]);
        assert!(behind(&[(1, 0)], &low, false).unwrap().is_empty(), "province 1 is passed over");
        assert_eq!(behind(&[(1, 0)], &low, true).unwrap().len(), 1);
        let mut unknown = division(5, 7, 0, 0);
        unknown.kind = None;
        assert_eq!(behind(&[(9, 0)], &at(9, vec![unknown]), false), Err(UNSETTLED));
    }

    #[test]
    fn a_click_selects_the_mates_of_one_group_and_withholds_what_the_unit_order_would_decide() {
        let divisions = at(9, vec![division(1, 7, 0, 0), division(2, 7, 3, 12), division(3, 7, 0, 0), division(6, 8, 0, 0)]);
        let none = |_: u64| false;
        let click = |candidates: &[PickDivision], selected: &dyn Fn(u64) -> bool| click(candidates, &divisions, Some(7), selected, OTHER_VIEWPOINT);
        let one = click(&[division(1, 7, 0, 0), division(3, 7, 0, 0)], &none).unwrap();
        assert_eq!(one, Click { province: 9, picked: 1, selected: false, ours: true, mates: vec![1, 3] });
        let moving = click(&[division(2, 7, 3, 12)], &none).unwrap();
        assert_eq!(moving.mates, [2], "a different first hop is another group");
        let unselectable = click(&[division(6, 8, 0, 0)], &none).unwrap();
        assert_eq!((unselectable.ours, unselectable.mates), (false, Vec::new()), "a listed unit not ours adds nothing");
        assert_eq!(click(&[division(1, 7, 0, 0), division(6, 8, 0, 0)], &none), Err(OTHER_VIEWPOINT));
        let some = |id: u64| id == 1;
        assert_eq!(click(&[division(1, 7, 0, 0), division(3, 7, 0, 0)], &some), Err(OTHER_VIEWPOINT));
        let both = |id: u64| id == 1 || id == 3;
        assert_eq!(click(&[division(1, 7, 0, 0), division(3, 7, 0, 0)], &both), Err(OTHER_VIEWPOINT), "which one Shift takes away");
        assert!(click(&[division(1, 7, 0, 0)], &some).unwrap().selected);
    }

    #[test]
    fn the_order_is_the_recorded_list_only_while_the_map_is_viewed_as_the_recording_clients_country() {
        let list = [5_u64, 3];
        assert_eq!(order_for(Ok(&list), Some(7), Some(7)), UnitOrder::Recorded(&list));
        assert_eq!(order_for(Ok(&list), Some(7), Some(8)), UnitOrder::NotRecorded(OTHER_VIEWPOINT));
        assert_eq!(order_for(Ok(&list), None, Some(7)), UnitOrder::NotRecorded(CLIENT_UNKNOWN));
        assert_eq!(order_for(Err(ORDER_NOT_RECORDED), Some(7), Some(7)), UnitOrder::NotRecorded(ORDER_NOT_RECORDED));
    }

    #[test]
    fn a_walk_takes_the_first_listed_unit_and_a_click_cycles_past_the_one_selected() {
        let ids = |walk: Result<Walk, &'static str>| walk.unwrap().candidates.iter().map(|division| division.id).collect::<Vec<_>>();
        let met = || vec![division(1, 7, 0, 0), division(3, 7, 0, 0), division(4, 7, 0, 0), division(6, 8, 0, 0)];
        let list = [9_u64, 4, 1, 3];
        let order = UnitOrder::Recorded(&list);
        assert_eq!(ids(walk(met(), order, Some(7), None)), [4], "the list's first, not Divisions order");
        assert_eq!(ids(walk(met(), order, Some(7), Some(Unit::Division(4)))), [1], "a click starts after the one selected");
        assert_eq!(ids(walk(met(), order, Some(7), Some(Unit::Division(3)))), [4], "and wraps to the start");
        assert_eq!(ids(walk(vec![division(4, 7, 0, 0)], order, Some(7), Some(Unit::Division(4)))), [4], "the selected one is tested last");
        assert_eq!(ids(walk(met(), order, Some(7), Some(Unit::Division(77)))), [4], "an unlisted selection starts at the first");
        let guns = [4_u64, 20, 1, 3];
        assert_eq!(ids(walk(met(), UnitOrder::Recorded(&guns), Some(7), Some(Unit::RailwayGun(20)))), [1], "a selected railway gun starts the walk after it");
        assert_eq!(ids(walk(vec![division(6, 8, 0, 0)], order, Some(7), None)), Vec::<u64>::new(), "another country's model is not in the list");
        let task_force = walk(met(), order, Some(7), Some(Unit::TaskForce(2))).unwrap();
        assert_eq!((task_force.candidates.len(), task_force.unordered), (3, TASK_FORCE_START), "a selected task force's place is not recorded");
    }

    #[test]
    fn a_walk_without_the_order_keeps_every_unit_of_ours_it_may_take() {
        let ids = |walk: &Walk| walk.candidates.iter().map(|division| division.id).collect::<Vec<_>>();
        let met = || vec![division(1, 7, 0, 0), division(3, 7, 0, 0), division(6, 8, 0, 0)];
        let order = UnitOrder::NotRecorded(OTHER_VIEWPOINT);
        let walked = walk(met(), order, Some(7), None).unwrap();
        assert_eq!((ids(&walked), walked.unordered), (vec![1, 3], OTHER_VIEWPOINT));
        assert_eq!(ids(&walk(met(), order, Some(7), Some(Unit::Division(1))).unwrap()), [3], "the one selected is tested last");
        assert_eq!(ids(&walk(vec![division(1, 7, 0, 0)], order, Some(7), Some(Unit::Division(1))).unwrap()), [1]);
        assert!(walk(vec![division(6, 8, 0, 0)], order, Some(7), None).unwrap().candidates.is_empty(), "another country's model leaves the click to the ground");
        let mut unlinked = division(2, 7, 0, 0);
        unlinked.roster = None;
        assert_eq!(walk(vec![unlinked], order, Some(7), None), Err(UNSETTLED));
        assert_eq!(walk(met(), order, None, None), Err(VIEWER_UNKNOWN));
    }

    #[test]
    fn the_hover_describes_one_location_and_the_cursor_one_roster_owner() {
        let mut divisions = at(9, vec![division(1, 7, 0, 0), division(3, 8, 0, 0)]);
        divisions.insert(10, vec![division(5, 7, 0, 0)]);
        assert_eq!(location(&[division(1, 7, 0, 0), division(3, 8, 0, 0)], &divisions, PROVINCE_ORDER), Ok(9));
        assert_eq!(location(&[division(1, 7, 0, 0), division(5, 7, 0, 0)], &divisions, ORDER_NOT_RECORDED), Err(ORDER_NOT_RECORDED));
        assert_eq!(roster_owner(&[division(1, 7, 0, 0), division(5, 7, 0, 0)]), Some(7));
        assert_eq!(roster_owner(&[division(1, 7, 0, 0), division(3, 8, 0, 0)]), None);
    }
}
