
use super::engaged;
use viewer_foundation::{perspective::{SharingUnknown, Vision}, tables::rows};
use replay_data::{rows::LandCombatSideRow, rows::TableId, rows::tables};
use replay_frame::frame::{DecodedFrame, Tables};
use std::{collections::BTreeMap, io};

pub const FIXED: i64 = 100_000;

pub struct Progresses(BTreeMap<u64, Result<i64, u64>>);

impl Progresses {
    pub fn read(frame: &DecodedFrame) -> io::Result<Self> {
        let organisation: BTreeMap<_, _> = rows::<tables::Divisions>(frame)?
            .rows()
            .iter()
            .map(|row| (row.id, row.organisation))
            .collect();
        let sum = |members: &[u64]| {
            members.iter().try_fold(0_i64, |total, id| {
                Ok(total.wrapping_add(*organisation.get(id).ok_or(*id)?))
            })
        };
        Ok(Self(
            rows::<tables::LandCombats>(frame)?
                .rows()
                .iter()
                .map(|combat| {
                    let progress = sum(&combat.attackers).and_then(|attacker| {
                        Ok(attacker_progress(attacker, sum(&combat.defenders)?))
                    });
                    (combat.id, progress)
                })
                .collect(),
        ))
    }

    pub fn attacker(&self, combat: u64) -> Option<i64> {
        self.0.get(&combat)?.ok()
    }

    pub fn unrecorded(&self, combat: u64) -> Option<u64> {
        self.0.get(&combat)?.err()
    }
}

pub fn attacker_progress(attacker: i64, defender: i64) -> i64 {
    let total = attacker.wrapping_add(defender);
    if total < 1 {
        return FIXED / 2;
    }
    if (attacker as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
        return FIXED;
    }
    (attacker.wrapping_mul(FIXED) / total).clamp(0, FIXED)
}

pub fn undefended(cause: SharingUnknown) -> &'static str {
    match cause {
        SharingUnknown::Reference => {
            "defending side unknown (a defender country reference is not in CountryLinks)"
        }
        SharingUnknown::Relations(_) => {
            "defending side unknown (no defender is proven to share vision: FactionMembers, WarRelations or Subjects is unavailable, truncated or names a country CountryLinks lacks this hour)"
        }
        SharingUnknown::Census(_) => {
            "defending side unknown (a census the defender sets are read with is not whole)"
        }
    }
}

pub fn defending(
    sets: &LandCombatSideRow,
    vision: &Vision,
    resolve: &impl Fn(u32) -> Option<u32>,
) -> Result<bool, SharingUnknown> {
    defending_of(&sets.defender_primary, &sets.defender_secondary, vision, resolve)
}

pub fn defending_of(
    primary: &[u32],
    secondary: &[u32],
    vision: &Vision,
    resolve: &impl Fn(u32) -> Option<u32>,
) -> Result<bool, SharingUnknown> {
    vision.sees_combat(
        primary.iter().chain(secondary).map(|raw| resolve(*raw)),
        None,
    )
}

pub struct LandSides;

impl engaged::Sides for LandSides {
    fn tables(&self) -> &'static [TableId] {
        &[TableId::LandCombats, TableId::LandCombatSides]
    }

    fn combats(&self, frame: &DecodedFrame) -> io::Result<Vec<u64>> {
        let mut combats: Vec<u64> = rows::<tables::LandCombats>(frame)?
            .rows()
            .iter()
            .map(|combat| combat.id)
            .collect();
        combats.sort_unstable();
        combats.dedup();
        Ok(combats)
    }

    fn sides(&self, frame: &DecodedFrame) -> io::Result<engaged::HourSides> {
        land_sides(frame)
    }
}

pub fn land_sides<F: Tables + ?Sized>(frame: &F) -> io::Result<engaged::HourSides> {
    let sides = sides(frame)?;
    let combats = viewer_foundation::fold_tables::available::<tables::LandCombats, F>(frame)?;
    let mut out = engaged::HourSides {
        sides: BTreeMap::new(),
        complete: combats.is_complete(),
        removal: engaged::Removal::BothSidesUnlessRoundRetreat,
    };
    for combat in combats.rows() {
        let sets = sides.get(&combat.id);
        for (side, units, lists) in [
            (
                0,
                &combat.attackers,
                sets.map(|s| (s.attacker_primary.clone(), s.attacker_secondary.clone())),
            ),
            (
                1,
                &combat.defenders,
                sets.map(|s| (s.defender_primary.clone(), s.defender_secondary.clone())),
            ),
        ] {
            let mut units = units.clone();
            units.sort_unstable();
            out.sides.insert(
                (combat.id, side),
                engaged::Side {
                    units: Some(units),
                    added: engaged::Added::NoTrigger,
                    unit_lists: lists.clone(),
                    lists,
                },
            );
        }
    }
    Ok(out)
}

pub fn sides<F: Tables + ?Sized>(frame: &F) -> io::Result<BTreeMap<u64, &LandCombatSideRow>> {
    Ok(viewer_foundation::fold_tables::available::<tables::LandCombatSides, F>(frame)?
        .rows()
        .iter()
        .map(|row| (row.combat, row))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{rows::DivisionRow, rows::LandCombatRow, rows::TableId, rows::TableRows};
    use replay_frame::{frame::Metrics, frame::TableView};

    fn frame(divisions: Vec<(u64, i64)>) -> DecodedFrame {
        let mut frame = DecodedFrame {
            branch: 0,
            hour: 7,
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "not needed".into(),
                }
                .into()
            }),
            metrics: Metrics::default(),
        };
        *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(
            divisions
                .into_iter()
                .map(|(id, organisation)| DivisionRow {
                    id,
                    organisation,
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::LandCombats.index()] =
            TableView::Complete(TableRows::LandCombats(vec![LandCombatRow {
                id: 62,
                attacker_classes: vec![0; 2],
                attackers: vec![1, 2],
                defenders: vec![3],
                ..Default::default()
            }]));
        frame
    }

    #[test]
    fn progress_sums_every_member_and_a_missing_one_is_unknown() {
        let full = frame(vec![(1, 1), (2, 2), (3, 6)]);
        assert_eq!(Progresses::read(&full).unwrap().attacker(62), Some(33_333));
        let short = frame(vec![(1, 1), (3, 6)]);
        assert_eq!(Progresses::read(&short).unwrap().attacker(62), None);
        assert_eq!(Progresses::read(&short).unwrap().unrecorded(62), Some(2));
        let defender = frame(vec![(1, 1), (2, 2)]);
        assert_eq!(Progresses::read(&defender).unwrap().unrecorded(62), Some(3));
        assert_eq!(Progresses::read(&full).unwrap().unrecorded(62), None);
    }

    #[test]
    fn defending_mirrors_on_any_sharing_defender_of_either_set() {
        let vision = Vision::sharing(1, [2]);
        let resolve = |raw: u32| (raw < 100).then_some(raw);
        let sets = |defender_primary: Vec<u32>, defender_secondary: Vec<u32>| LandCombatSideRow {
            combat: 62,
            attacker_primary: vec![1],
            defender_primary,
            defender_secondary,
            ..Default::default()
        };
        assert_eq!(
            defending(&sets(vec![6], vec![]), &vision, &resolve),
            Ok(false)
        );
        assert_eq!(
            defending(&sets(vec![6, 2], vec![]), &vision, &resolve),
            Ok(true)
        );
        assert_eq!(
            defending(&sets(vec![6], vec![2]), &vision, &resolve),
            Ok(true)
        );
        assert_eq!(
            defending(&sets(vec![500], vec![]), &vision, &resolve),
            Err(SharingUnknown::Reference)
        );
        assert_eq!(
            defending(&sets(vec![500, 1], vec![]), &vision, &resolve),
            Ok(true)
        );
    }

    #[test]
    fn a_land_sample_keys_both_sides_with_sorted_units_and_their_lists() {
        let mut frame = frame(vec![]);
        *frame.tables[TableId::LandCombatSides.index()] =
            TableView::Complete(TableRows::LandCombatSides(vec![LandCombatSideRow {
                combat: 62,
                attacker_primary: vec![4],
                defender_secondary: vec![9],
                ..Default::default()
            }]));
        if let TableView::Complete(TableRows::LandCombats(rows)) =
            &mut *frame.tables[TableId::LandCombats.index()]
        {
            rows[0].attackers = vec![2, 1];
        }
        let sample = land_sides(&frame).unwrap();
        assert!(sample.complete);
        assert_eq!(
            sample.sides[&(62, 0)],
            engaged::Side {
                units: Some(vec![1, 2]),
                added: engaged::Added::NoTrigger,
                lists: Some((vec![4], vec![])),
                unit_lists: Some((vec![4], vec![])),
            }
        );
        assert_eq!(sample.sides[&(62, 1)].lists, Some((vec![], vec![9])));
    }

    #[test]
    fn progress_keeps_native_wrapping_arithmetic() {
        for (attacker, defender, expected) in [
            (0, 0, 50_000),
            (1, 2, 33_333),
            (2, 1, 66_666),
            (-1, 10, 0),
            (10, -1, 100_000),
            (i64::MAX, 0, 100_000),
            (i64::MAX, 1, 50_000),
            (i64::MAX / 2, 1, 0),
        ] {
            assert_eq!(attacker_progress(attacker, defender), expected);
        }
    }
}
