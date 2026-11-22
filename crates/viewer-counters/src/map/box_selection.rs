use viewer_native_gui::counter_hovers::unit_hits::Kind;
use viewer_runtime::unit_handle::Unit;

pub const DRAG_START: f32 = 1.0;
pub const MIN_EXTENT: f32 = 30.0;

pub const THEATRE_NOT_RECORDED: &str =
    "unit selection: units' theatres are not recorded, so a box or a model click takes no division or railway gun";
pub const THEATRE_REFUSED: &str =
    "unit selection: a unit's theatre was refused when read, so a box or a model click takes no division or railway gun";
pub const THEATRE_UNREAD: &str =
    "unit selection: the hour's divisions or railway guns did not read, so a box or a model click takes none of them";
pub const THEATRE_ABSENT: &str =
    "unit selection: a unit is absent from the hour's rows, so a box or a model click takes no division or railway gun";

pub fn dragging(from: [f32; 2], at: [f32; 2]) -> bool {
    (at[0] - from[0]).hypot(at[1] - from[1]) >= DRAG_START
}

pub fn boxes(from: [f32; 2], to: [f32; 2]) -> bool {
    dragging(from, to) && (to[0] - from[0]).abs() + (to[1] - from[1]).abs() >= MIN_EXTENT
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Member {
    AirWing,
    Operative,
    Unit(Kind),
    Fleet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    StrategicAir,
    StrategicNavy,
    Operatives,
    Other,
}

pub fn order(mode: Mode, ctrl: bool) -> Vec<Member> {
    let mut order = match mode {
        Mode::StrategicAir => vec![Member::AirWing],
        Mode::Operatives => vec![Member::Operative],
        Mode::StrategicNavy | Mode::Other => Vec::new(),
    };
    let land = [Kind::Division, Kind::RailwayGun, Kind::TaskForce];
    let naval = [Kind::TaskForce, Kind::Division, Kind::RailwayGun];
    let units = if ctrl == (mode == Mode::StrategicNavy) { land } else { naval };
    order.extend(units.map(Member::Unit));
    order
}

pub struct Candidate<'a> {
    pub rect: [f32; 4],
    pub member: Member,
    pub members: &'a [u64],
    pub ours: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Taken {
    pub member: Option<Member>,
    pub members: Vec<u64>,
    pub withheld: Option<&'static str>,
}

fn intersects(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] <= b[0] + b[2] && b[0] <= a[0] + a[2] && a[1] <= b[1] + b[3] && b[1] <= a[1] + a[3]
}

fn unselected(_: u64) -> bool {
    false
}

pub fn rect(from: [f32; 2], to: [f32; 2]) -> [f32; 4] {
    [from[0].min(to[0]), from[1].min(to[1]), (to[0] - from[0]).abs(), (to[1] - from[1]).abs()]
}

pub fn take(
    candidates: &[Candidate<'_>],
    area: [f32; 4],
    order: &[Member],
    shift: bool,
    selected: Option<(Member, &dyn Fn(u64) -> bool)>,
    anchor: Option<Unit>,
    theatre: &dyn Fn(Unit) -> Result<u64, &'static str>,
) -> Taken {
    let (start, admitted): (usize, &dyn Fn(u64) -> bool) = match selected.filter(|_| shift) {
        Some((member, admitted)) => match order.iter().position(|&kind| kind == member) {
            Some(at) => (at, admitted),
            None => return Taken::default(),
        },
        None => (0, &unselected),
    };
    let fixed = selected.is_some() && shift;
    for &member in &order[start..] {
        let mut members: Vec<u64> = Vec::new();
        for candidate in candidates.iter().filter(|candidate| {
            candidate.member == member && candidate.ours && intersects(candidate.rect, area)
        }) {
            for &handle in candidate.members {
                if !admitted(handle) && !members.contains(&handle) {
                    members.push(handle);
                }
            }
        }
        if members.is_empty() {
            if fixed {
                return Taken::default();
            }
            continue;
        }
        let mut withheld = None;
        if let Member::Unit(kind @ (Kind::Division | Kind::RailwayGun)) = member {
            let anchor = anchor.filter(|_| fixed).unwrap_or(kind.unit(members[0]));
            let theatres = members.iter().map(|&handle| theatre(kind.unit(handle))).collect::<Result<Vec<_>, _>>();
            match theatre(anchor).and_then(|home| Ok((home, theatres?))) {
                Ok((home, theatres)) => {
                    members = members.into_iter().zip(theatres).filter(|&(_, at)| at == home).map(|(handle, _)| handle).collect();
                }
                Err(why) => {
                    members.clear();
                    withheld = Some(why);
                }
            }
        }
        return Taken { member: Some(member), members, withheld };
    }
    Taken::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_press_drags_from_one_pixel_and_boxes_from_thirty_across_and_down() {
        assert!(!dragging([0.0, 0.0], [0.99, 0.0]));
        assert!(dragging([0.0, 0.0], [1.0, 0.0]));
        assert!(!boxes([0.0, 0.0], [1.0, 0.0]));
        assert!(!boxes([10.0, 10.0], [24.0, -5.0]));
        assert!(boxes([10.0, 10.0], [25.0, -5.0]));
    }

    #[test]
    fn the_order_follows_the_mode_and_ctrl() {
        use {Kind::*, Member::Unit as U};
        let land = [U(Division), U(RailwayGun), U(TaskForce)];
        let naval = [U(TaskForce), U(Division), U(RailwayGun)];
        assert_eq!(order(Mode::Other, false), land);
        assert_eq!(order(Mode::Other, true), naval);
        assert_eq!(order(Mode::StrategicNavy, false), naval);
        assert_eq!(order(Mode::StrategicNavy, true), land);
        assert_eq!(order(Mode::StrategicAir, false), [vec![Member::AirWing], land.to_vec()].concat());
        assert_eq!(order(Mode::StrategicAir, true), [vec![Member::AirWing], naval.to_vec()].concat());
        assert_eq!(order(Mode::Operatives, false), [vec![Member::Operative], land.to_vec()].concat());
    }

    fn candidate(member: Member, x: f32, members: &[u64], ours: bool) -> Candidate<'_> {
        Candidate { rect: [x, 0.0, 10.0, 10.0], member, members, ours }
    }

    const AREA: [f32; 4] = [0.0, 0.0, 100.0, 100.0];

    #[test]
    fn a_box_takes_the_first_kind_with_our_members_in_draw_order_and_never_mixes_kinds() {
        let (division, gun, force) = (Member::Unit(Kind::Division), Member::Unit(Kind::RailwayGun), Member::Unit(Kind::TaskForce));
        let candidates = [
            candidate(force, 0.0, &[9], true),
            candidate(division, 20.0, &[3], false),
            candidate(gun, 40.0, &[5], true),
            candidate(division, 60.0, &[2, 1], true),
            candidate(division, 100.0, &[4], true),
            candidate(division, 101.0, &[6], true),
        ];
        let known = |_: Unit| -> Result<u64, &'static str> { Ok(7) };
        let taken = take(&candidates, AREA, &order(Mode::Other, false), false, None, None, &known);
        assert_eq!(taken, Taken { member: Some(division), members: vec![2, 1, 4], withheld: None });
        let taken = take(&candidates, AREA, &order(Mode::Other, true), false, None, None, &known);
        assert_eq!(taken.members, [9]);
        let ours_only = take(&candidates[1..3], AREA, &order(Mode::Other, false), false, None, None, &known);
        assert_eq!((ours_only.member, ours_only.members), (Some(gun), vec![5]));
        assert_eq!(take(&candidates[..0], AREA, &order(Mode::Other, false), false, None, None, &known), Taken::default());
    }

    #[test]
    fn shift_keeps_to_the_selected_kind_and_skips_what_is_selected() {
        let (division, force) = (Member::Unit(Kind::Division), Member::Unit(Kind::TaskForce));
        let candidates = [candidate(division, 0.0, &[1, 2], true), candidate(force, 20.0, &[8], true)];
        let known = |_: Unit| -> Result<u64, &'static str> { Ok(7) };
        let order = order(Mode::Other, false);
        let selected_one = |handle: u64| handle == 1;
        let taken = take(&candidates, AREA, &order, true, Some((division, &selected_one)), Some(Unit::Division(1)), &known);
        assert_eq!(taken.members, [2]);
        let taken = take(&candidates, AREA, &order, true, Some((force, &|handle| handle == 8)), None, &known);
        assert_eq!(taken, Taken::default());
        let taken = take(&candidates, AREA, &order, true, Some((Member::AirWing, &|_| false)), None, &known);
        assert_eq!(taken, Taken::default());
    }

    #[test]
    fn divisions_keep_to_the_anchors_theatre_and_an_unrecorded_theatre_takes_none() {
        let division = Member::Unit(Kind::Division);
        let candidates = [candidate(division, 0.0, &[1, 2, 3, 4], true)];
        let order = order(Mode::Other, false);
        let theatres = |unit: Unit| -> Result<u64, &'static str> { match unit {
            Unit::Division(2) => Ok(8),
            Unit::Division(3 | 4) => Ok(0),
            _ => Ok(7),
        } };
        assert_eq!(take(&candidates, AREA, &order, false, None, None, &theatres).members, [1]);
        let unanchored = take(&candidates, AREA, &order, true, Some((division, &|_| false)), Some(Unit::Division(3)), &theatres);
        assert_eq!(unanchored.members, [3, 4], "no theatre matches only no theatre");
        let unknown = |unit: Unit| -> Result<u64, &'static str> { if unit == Unit::Division(3) { Err(THEATRE_NOT_RECORDED) } else { Ok(7) } };
        let taken = take(&candidates, AREA, &order, false, None, None, &unknown);
        assert_eq!(taken, Taken { member: Some(division), members: Vec::new(), withheld: Some(THEATRE_NOT_RECORDED) });
        let forces = [candidate(Member::Unit(Kind::TaskForce), 0.0, &[1, 2], true)];
        let taken = take(&forces, AREA, &order, false, None, None, &|_| Err(THEATRE_NOT_RECORDED));
        assert_eq!((taken.members, taken.withheld), (vec![1, 2], None));
    }
}
