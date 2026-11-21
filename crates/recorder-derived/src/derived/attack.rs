
use replay_data::{rows::Recorded, rows::tables};
use replay_frame::frame::DecodedFrame;

const LISTED: usize = 40;

pub(super) struct Checker {
    terrains: Result<Vec<i64>, String>,
    rows: u64,
    terrain_equal: u64,
    scalar_equal: u64,
    mismatches: u64,
    underivable: u64,
    unavailable_hours: u64,
    listed: Vec<String>,
}

impl Checker {
    pub(super) fn new(terrains: Result<Vec<i64>, String>) -> Self {
        Self {
            terrains,
            rows: 0,
            terrain_equal: 0,
            scalar_equal: 0,
            mismatches: 0,
            underivable: 0,
            unavailable_hours: 0,
            listed: Vec::new(),
        }
    }

    fn mismatch(&mut self, frame: &DecodedFrame, division: u64, what: String) {
        self.mismatches += 1;
        if self.listed.len() < LISTED {
            self.listed.push(format!(
                "branch {} hour {} division {division:#x}: {what}",
                frame.branch, frame.hour
            ));
        }
    }

    pub(super) fn observe(&mut self, frame: &DecodedFrame) {
        let Ok(rows) = frame.complete::<tables::DivisionStatus>() else {
            self.unavailable_hours += 1;
            return;
        };
        let terrains = self.terrains.clone();
        for row in rows {
            let Recorded::Value(witness) = &row.attack else {
                continue;
            };
            let Some(witness) = witness.get() else {
                continue;
            };
            self.rows += 1;
            let (Ok(terrains), Recorded::Value(Some(stats))) = (&terrains, &row.attack_stats)
            else {
                self.underivable += 1;
                continue;
            };
            self.underivable += u64::from(witness.terrain.is_none());
            for terrain in witness.terrain.iter().flatten() {
                let Some(witnessed) = terrain.attack else {
                    self.underivable += 1;
                    continue;
                };
                let at = usize::try_from(terrain.terrain)
                    .ok()
                    .and_then(|index| index.checked_sub(1));
                let derived =
                    at.and_then(|at| Some(stats.terrain.get(at)?.wrapping_add(*terrains.get(at)?)));
                match derived {
                    None => self.underivable += 1,
                    Some(derived) if derived == witnessed => self.terrain_equal += 1,
                    Some(derived) => self.mismatch(
                        frame,
                        row.division,
                        format!(
                            "terrain {}: derived {derived}, witnessed {witnessed}",
                            terrain.terrain
                        ),
                    ),
                }
            }
            let scalars = [
                ("river", Some(stats.river[0]), witness.stats.river[0]),
                ("river second", Some(stats.river[1]), witness.stats.river[1]),
                ("fort", Some(stats.fort[0]), witness.stats.fort[0]),
                ("fort second", Some(stats.fort[1]), witness.stats.fort[1]),
                ("snow", Some(stats.snow), witness.stats.snow),
                ("night", Some(stats.night), witness.stats.night),
                (
                    "acclimatization",
                    match row.weather_factor {
                        Recorded::Value(factor) => factor,
                        Recorded::NotRecorded => None,
                    },
                    witness.acclimatization,
                ),
            ];
            for (name, recorded, witnessed) in scalars {
                if recorded.is_none() && witnessed.is_none() {
                    self.underivable += 1;
                } else if recorded == witnessed {
                    self.scalar_equal += 1;
                } else {
                    self.mismatch(
                        frame,
                        row.division,
                        format!("{name}: recorded {recorded:?}, witnessed {witnessed:?}"),
                    );
                }
            }
        }
    }

    pub(super) fn failures(&self) -> [u64; 2] {
        [self.mismatches, self.underivable + self.unavailable_hours]
    }

    pub(super) fn print(&self) {
        match &self.terrains {
            Err(reason) => println!("attack operands: content terrains unavailable ({reason})"),
            Ok(_) => {
                println!(
                    "attack operands: {} witnessed rows; terrain values equal {}, scalars equal {}, differing {}, underivable terrains {}",
                    self.rows,
                    self.terrain_equal,
                    self.scalar_equal,
                    self.mismatches,
                    self.underivable
                );
                for line in &self.listed {
                    println!("  mismatch: {line}");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{
        rows::AttackStats, rows::AttackTerrain, rows::DivisionAttackInputs,
        rows::DivisionAttackStats, rows::DivisionStatusRow, rows::PacketHour, rows::TableId,
        rows::TableRows, rows::Witnessed,
    };
    use replay_frame::{frame::Metrics, frame::TableView};

    fn frame(rows: Vec<DivisionStatusRow>) -> DecodedFrame {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 10,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "fixture".into(),
                }
                .into()
            }),
        };
        *frame.tables[TableId::DivisionStatus.index()] =
            TableView::Complete(TableRows::DivisionStatus(rows));
        frame
    }

    fn row(entry: i64, witnessed: i64) -> DivisionStatusRow {
        DivisionStatusRow {
            division: 7 << 32,
            attack_stats: Recorded::Value(Some(DivisionAttackStats {
                terrain: vec![entry, 0],
                river: [1, 2],
                fort: [3, 4],
                snow: 5,
                night: 6,
            })),
            weather_factor: Recorded::Value(Some(100_000)),
            attack: Recorded::Value(Witnessed(Some(DivisionAttackInputs {
                sample_hour: PacketHour::Host,
                template: None,
                stats: AttackStats {
                    river: [Some(1), Some(2)],
                    fort: [Some(3), Some(4)],
                    snow: Some(5),
                    night: Some(6),
                },
                terrain: Some(vec![
                    AttackTerrain {
                        terrain: 1,
                        attack: Some(witnessed),
                    },
                    AttackTerrain {
                        terrain: 3,
                        attack: Some(0),
                    },
                ]),
                acclimatization: Some(100_000),
            }))),
            ..Default::default()
        }
    }

    #[test]
    fn derived_terrain_operands_add_the_content_terrains_own_attack() {
        let mut checker = Checker::new(Ok(vec![-5_000, 0]));
        checker.observe(&frame(vec![row(-10_000, -15_000)]));
        assert_eq!(
            (checker.rows, checker.terrain_equal, checker.scalar_equal),
            (1, 1, 7)
        );
        assert_eq!((checker.mismatches, checker.underivable), (0, 1));
        checker.observe(&frame(vec![row(-10_000, -20_000)]));
        assert_eq!(checker.mismatches, 1);
        assert!(checker.listed[0].contains("terrain 1"));
        let mut unwitnessed = row(0, 0);
        unwitnessed.attack = Recorded::Value(Witnessed(None));
        checker.observe(&frame(vec![unwitnessed]));
        assert_eq!(checker.rows, 2);
        assert_eq!(checker.failures(), [1, 2]);
        let mut missing_operands = row(0, 0);
        missing_operands.attack_stats = Recorded::Value(None);
        checker.observe(&frame(vec![missing_operands]));
        assert_eq!(checker.failures(), [1, 3]);
        let mut missing_content = Checker::new(Err("terrain content refused".into()));
        missing_content.observe(&frame(vec![row(0, 0)]));
        assert_eq!(missing_content.failures(), [0, 1]);
        let mut checker = Checker::new(Ok(vec![-5_000, 0]));
        for _ in 0..=LISTED {
            checker.observe(&frame(vec![row(-10_000, -20_000)]));
        }
        assert_eq!(checker.failures(), [LISTED as u64 + 1, LISTED as u64 + 1]);
        assert_eq!(checker.listed.len(), LISTED);
    }
}
