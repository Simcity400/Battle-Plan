use std::{collections::BTreeMap, io};

use replay_data::{rows::Recorded, rows::tables};
use replay_frame::frame::{DecodedFrame, DecodedSource};

use super::{
    tables::{Available, rows},
    visibility::Relations,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tints {
    pub ours: [f32; 4],
    pub allied: [f32; 4],
    pub enemy: [f32; 4],
    pub other: [f32; 4],
}

impl Tints {
    pub fn read(defines: &content_reader_defines::defines::Defines) -> io::Result<Self> {
        let color = |name: &str| -> io::Result<[f32; 4]> {
            let values = defines.numbers("NGraphics", name)?;
            <[f64; 4]>::try_from(values)
                .map(|rgba| rgba.map(|value| value as f32))
                .map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("NDefines.NGraphics.{name} is not four numbers"),
                    )
                })
        };
        Ok(Self {
            ours: color("COUNTER_MODE_ALLEGIANCE_OURS")?,
            allied: color("COUNTER_MODE_ALLEGIANCE_ALLIED")?,
            enemy: color("COUNTER_MODE_ALLEGIANCE_ENEMY")?,
            other: color("COUNTER_MODE_ALLEGIANCE_OTHER")?,
        })
    }

    pub fn of(&self, side: Allegiance) -> [f32; 4] {
        match side {
            Allegiance::Enemy => self.enemy,
            Allegiance::Allied => self.allied,
            Allegiance::Ours => self.ours,
            Allegiance::Other => self.other,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub const FIXTURE: Self = Self {
        ours: [0.15, 0.75, 0.25, 1.0],
        allied: [0.20, 0.45, 0.85, 1.0],
        enemy: [0.85, 0.15, 0.20, 1.0],
        other: [0.65, 0.65, 0.65, 1.0],
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Allegiance {
    Enemy,
    Allied,
    Ours,
    Other,
}

impl Allegiance {
    pub fn index(self) -> u32 {
        match self {
            Allegiance::Enemy => 0,
            Allegiance::Allied => 1,
            Allegiance::Ours => 2,
            Allegiance::Other => 4,
        }
    }
}

pub fn perspective(frame: &DecodedFrame, links: &BTreeMap<u32, u32>) -> io::Result<Option<u32>> {
    let Ok([player]) = frame.complete::<tables::LocalPlayer>() else {
        return Ok(None);
    };
    Ok(player
        .viewing_country()
        .and_then(|raw| links.get(&raw).copied()))
}

pub use dashboard_text::triggers::ongoing;

pub fn war_table(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    ours: u32,
) -> io::Result<BTreeMap<u32, Allegiance>> {
    let mut table = BTreeMap::new();
    table.insert(ours, Allegiance::Ours);
    for war in rows::<tables::WarRelations>(frame)?.rows() {
        if !ongoing(war.start_hour, war.end_hour, frame.hour) {
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

pub fn counter_table(
    relations: &Relations,
    links: &BTreeMap<u32, u32>,
    ours: u32,
) -> BTreeMap<u32, Allegiance> {
    let mut table = BTreeMap::from([(ours, Allegiance::Ours)]);
    for &slot in links.values() {
        let side = if slot == ours {
            Allegiance::Ours
        } else if relations.shared(ours, slot) {
            Allegiance::Allied
        } else if relations.at_war(ours, slot) {
            Allegiance::Enemy
        } else {
            Allegiance::Other
        };
        table.insert(slot, side);
    }
    table
}

pub struct NativeRelations {
    countries: BTreeMap<u32, NativeCountryRelations>,
    missing: String,
}

struct NativeCountryRelations {
    co_belligerents: Result<Vec<u32>, String>,
    zero_war: Result<[u8; 2], String>,
}

impl NativeRelations {
    pub fn read(frame: &dyn DecodedSource, links: &BTreeMap<u32, u32>) -> io::Result<Self> {
        let table = rows::<tables::CountryCounterInputs>(frame)?;
        let missing = match &table {
            Available::Complete(_) => "CountryCounterInputs has no row for this owner".into(),
            Available::Truncated { missing, .. } => format!(
                "CountryCounterInputs is truncated ({missing} rows not reached) and has no row for this owner"
            ),
            Available::Unavailable { reason } => {
                format!("CountryCounterInputs is unavailable ({reason})")
            }
            Available::NotRecorded { mode } => {
                format!("CountryCounterInputs is not recorded in a {mode} recording")
            }
        };
        let mut countries = BTreeMap::new();
        for row in table.rows() {
            let Some(&slot) = links.get(&row.country) else {
                continue;
            };
            let co_belligerents =
                native_input(&row.co_belligerents, "CountryCounterInputs.co_belligerents")
                    .and_then(|targets| {
                        if !targets.windows(2).all(|pair| pair[0] < pair[1]) {
                            return Err(
                                "CountryCounterInputs.co_belligerents is not sorted and unique"
                                    .into(),
                            );
                        }
                        Ok(targets)
                    });
            let zero_war =
                native_input(&row.zero_war, "CountryCounterInputs.zero_war").and_then(|value| {
                    match value {
                        [0, 0] | [1, _] => Ok(value),
                        _ => {
                            Err("CountryCounterInputs.zero_war has invalid pointer presence".into())
                        }
                    }
                });
            match countries.entry(slot) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(NativeCountryRelations {
                        co_belligerents,
                        zero_war,
                    });
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    let why = "CountryCounterInputs has multiple rows for this canonical owner";
                    entry.insert(NativeCountryRelations {
                        co_belligerents: Err(why.into()),
                        zero_war: Err(why.into()),
                    });
                }
            }
        }
        Ok(Self { countries, missing })
    }

    fn country(&self, owner: u32) -> Result<&NativeCountryRelations, String> {
        self.countries
            .get(&owner)
            .ok_or_else(|| format!("country slot {owner}: {}", self.missing))
    }

    pub fn co_belligerent(&self, owner: u32, target: u32) -> Result<bool, String> {
        let targets = self
            .country(owner)?
            .co_belligerents
            .as_ref()
            .map_err(|why| format!("country slot {owner}: {why}"))?;
        Ok(targets.binary_search(&target).is_ok())
    }

    pub fn zero_at_war(&self, owner: u32) -> Result<bool, String> {
        let &[present, retired] = self
            .country(owner)?
            .zero_war
            .as_ref()
            .map_err(|why| format!("country slot {owner}: {why}"))?;
        Ok(present != 0 && retired == 0)
    }
}

fn native_input<T: Clone>(value: &Recorded<Option<T>>, field: &str) -> Result<T, String> {
    value
        .require(field)?
        .clone()
        .ok_or_else(|| format!("{field} was refused during capture"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::allegiance::*;
    use replay_data::{rows::CountryLinkRow, rows::PlayerRow, rows::TableId, rows::TableRows};
    use replay_frame::frame::TableView;

    #[test]
    fn multiplayer_switch_and_observer_use_only_the_clients_stored_viewpoint() {
        use replay_data::rows::LocalPlayerRow;
        let mut frame = frame(Vec::new(), Vec::new());
        let links: BTreeMap<_, _> = (1..=50).map(|slot| (slot + 10_000, slot)).collect();
        *frame.tables[TableId::Players.index()] = TableView::Complete(TableRows::Players(
            (1..=50)
                .map(|slot| PlayerRow {
                    id: slot,
                    country: slot as u32 + 10_000,
                    ..Default::default()
                })
                .collect(),
        ));
        for (country, observer_country, expected) in [
            (10_050, 10_001, Some(50)),
            (10_002, 0, Some(2)),
            (0, 10_049, Some(49)),
            (0, 0, None),
            (u32::MAX, 10_003, Some(3)),
            (50_000, 10_001, None),
        ] {
            *frame.tables[TableId::LocalPlayer.index()] =
                TableView::Complete(TableRows::LocalPlayer(vec![LocalPlayerRow {
                    country,
                    observer_country,
                }]));
            assert_eq!(perspective(&frame, &links).unwrap(), expected);
            frame.hour += 1;
        }
        for table in [
            TableView::Unavailable {
                reason: "read failed".into(),
            },
            TableView::Truncated {
                rows: TableRows::LocalPlayer(vec![LocalPlayerRow {
                    country: 10_001,
                    observer_country: 0,
                }]),
                missing: 1,
            },
        ] {
            *frame.tables[TableId::LocalPlayer.index()] = table;
            assert_eq!(perspective(&frame, &links).unwrap(), None);
        }
    }

    #[test]
    fn a_war_with_no_end_date_runs_whatever_the_hour() {
        assert!(ongoing(60_759_371, NO_END_HOUR, NIGHT_HOUR));
        assert!(ongoing(60_759_371, NO_END_HOUR, 0));
        assert!(ongoing(60_759_371, NO_END_HOUR, i32::MAX));
        assert!(ongoing(NIGHT_HOUR, NIGHT_HOUR, NIGHT_HOUR));
        assert!(ongoing(60_759_371, 0, NIGHT_HOUR));
    }

    #[test]
    fn a_war_that_carries_an_end_date_stops_at_it() {
        let (start, end) = (60_759_371, 60_800_000);
        assert!(ongoing(start, end, start));
        assert!(ongoing(start, end, end - 1));
        assert!(!ongoing(start, end, end));
        assert!(!ongoing(start, end, NIGHT_HOUR));
    }

    #[test]
    fn null_end_hour_keeps_wars_active_at_later_hours() {
        const { assert!(NO_END_HOUR < NIGHT_HOUR) };
        let frame = frame(
            vec![
                war(100, 102, 60_759_371, NO_END_HOUR),
                war(101, 100, 60_760_000, NO_END_HOUR),
            ],
            Vec::new(),
        );
        let links = links(&frame);
        let ours = perspective(&frame, &links).unwrap();
        assert_eq!(ours, Some(0));
        let table = war_table(&frame, &links, ours.unwrap()).unwrap();
        assert_eq!(table.get(&0), Some(&Allegiance::Ours));
        assert_eq!(table.get(&2), Some(&Allegiance::Enemy));
        assert_eq!(table.get(&1), Some(&Allegiance::Enemy));
    }

    fn relations_of(
        wars: &[(u32, u32)],
        members: &[(u32, u64)],
    ) -> (Relations, BTreeMap<u32, u32>) {
        let mut frame = frame(
            wars.iter()
                .map(|&(a, b)| war(a, b, 60_759_371, NO_END_HOUR))
                .collect(),
            members.iter().map(|&(c, f)| member(c, f)).collect(),
        );
        let links: BTreeMap<u32, u32> = (0..8).map(|slot| (100 + slot, slot)).collect();
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(
                links
                    .iter()
                    .map(|(&id, &canonical_slot)| CountryLinkRow { id, canonical_slot })
                    .collect(),
            ));
        *frame.tables[TableId::Subjects.index()] = TableView::Complete(TableRows::Subjects(vec![]));
        (Relations::read(&frame, &links).unwrap(), links)
    }

    #[test]
    fn a_faction_partner_is_blue_and_a_country_at_war_is_red() {
        let (relations, links) = relations_of(&[(100, 102)], &[(100, 1), (101, 1), (102, 2)]);
        let table = counter_table(&relations, &links, 0);
        assert_eq!(table.get(&0), Some(&Allegiance::Ours));
        assert_eq!(table.get(&1), Some(&Allegiance::Allied));
        assert_eq!(table.get(&2), Some(&Allegiance::Enemy));
        assert_eq!(table.get(&3), Some(&Allegiance::Other));
    }

    #[test]
    fn a_faction_partner_still_at_war_with_us_stays_red() {
        let (relations, links) = relations_of(&[(101, 100)], &[(100, 1), (101, 1)]);
        assert_eq!(
            counter_table(&relations, &links, 0).get(&1),
            Some(&Allegiance::Enemy)
        );
    }

    #[test]
    fn native_cache_and_zero_war_refusals_are_independent_and_named() {
        use replay_data::rows::CountryCounterInputRow;
        let frame = native_frame(vec![
            CountryCounterInputRow {
                co_belligerents: Recorded::NotRecorded,
                ..native_row(101, vec![], [0, 0])
            },
            CountryCounterInputRow {
                zero_war: Recorded::Value(None),
                ..native_row(102, vec![1], [0, 0])
            },
        ]);
        let native = NativeRelations::read(&frame, &links(&frame)).unwrap();
        assert!(
            native
                .co_belligerent(1, 2)
                .unwrap_err()
                .contains("country slot 1: CountryCounterInputs.co_belligerents not recorded")
        );
        assert_eq!(native.zero_at_war(1), Ok(false));
        assert_eq!(native.co_belligerent(2, 1), Ok(true));
        assert!(
            native.zero_at_war(2).unwrap_err().contains(
                "country slot 2: CountryCounterInputs.zero_war was refused during capture"
            )
        );
    }

    #[test]
    fn truncated_native_rows_remain_known_and_missing_owners_stay_unknown() {
        let mut frame = native_frame(vec![]);
        let links = links(&frame);
        *frame.tables[TableId::CountryCounterInputs.index()] = TableView::Truncated {
            rows: TableRows::CountryCounterInputs(vec![native_row(101, vec![], [0, 0])]),
            missing: 1,
        };
        let native = NativeRelations::read(&frame, &links).unwrap();
        assert_eq!(native.co_belligerent(1, 2), Ok(false));
        assert!(
            native
                .co_belligerent(2, 1)
                .unwrap_err()
                .contains("truncated")
        );
        *frame.tables[TableId::CountryCounterInputs.index()] = TableView::Unavailable {
            reason: "native owner census failed".into(),
        };
        let native = NativeRelations::read(&frame, &links).unwrap();
        assert!(
            native
                .co_belligerent(1, 2)
                .unwrap_err()
                .contains("CountryCounterInputs is unavailable (native owner census failed)")
        );
    }

    #[test]
    fn malformed_native_operands_and_alias_collisions_do_not_poison_other_owners() {
        let frame = native_frame(vec![
            native_row(101, vec![2, 0], [0, 0]),
            native_row(102, vec![1], [9, 0]),
        ]);
        let native = NativeRelations::read(&frame, &links(&frame)).unwrap();
        assert!(
            native
                .co_belligerent(1, 2)
                .unwrap_err()
                .contains("sorted and unique")
        );
        assert_eq!(native.zero_at_war(1), Ok(false));
        assert_eq!(native.co_belligerent(2, 1), Ok(true));
        assert!(
            native
                .zero_at_war(2)
                .unwrap_err()
                .contains("invalid pointer presence")
        );

        let frame = native_frame(vec![
            native_row(101, vec![2], [0, 0]),
            native_row(102, vec![], [0, 0]),
            native_row(201, vec![], [0, 0]),
        ]);
        let mut links = links(&frame);
        links.insert(201, 1);
        let native = NativeRelations::read(&frame, &links).unwrap();
        assert!(
            native
                .co_belligerent(1, 2)
                .unwrap_err()
                .contains("multiple rows")
        );
        assert!(native.zero_at_war(1).unwrap_err().contains("multiple rows"));
        assert_eq!(native.co_belligerent(2, 1), Ok(false));
    }

    #[test]
    fn native_cache_keeps_the_complete_recorded_target_domain() {
        let frame = native_frame(vec![native_row(101, (0..=50).collect(), [0, 0])]);
        let mut links = links(&frame);
        links.extend((3..=50).map(|slot| (10_000 + slot, slot)));
        let native = NativeRelations::read(&frame, &links).unwrap();
        assert_eq!(native.co_belligerent(1, 50), Ok(true));
        assert_eq!(native.co_belligerent(1, 51), Ok(false));
    }

}
