//! Hourly observer counters and post-flush FinalizeCombat results.
use super::*;
use replay::rows::{LandSideLosses, Recorded};

#[derive(Clone, PartialEq, Eq)]
pub(super) struct SideLossInputs {
    manpower: Recorded<i32>,
    equipment: Recorded<Vec<replay::rows::LandEquipmentLoss>>,
}
impl From<&LandSideLosses> for SideLossInputs {
    fn from(loss: &LandSideLosses) -> Self {
        Self {
            manpower: loss.manpower.into(),
            equipment: loss.equipment.clone().into(),
        }
    }
}

fn known(label: &str, value: f64) -> BattleValue {
    BattleValue::Known {
        label: label.into(),
        value,
        at_least: None,
    }
}
fn unknown(label: &str, reason: &str) -> BattleValue {
    BattleValue::Unknown {
        label: label.into(),
        value: (),
        unknown: reason.into(),
    }
}
fn equipment_value(label: &str, raw: i128) -> BattleValue {
    if raw.unsigned_abs() <= ((1u128 << 53) - 1) {
        known(
            label,
            raw as f64 / engine::equipment_entry::FIXED_POINT_SCALE as f64,
        )
    } else {
        // Keep the exact amount in the detail instead of rounding a wide counter.
        let sign = if raw < 0 { "-" } else { "" };
        let amount = raw.unsigned_abs();
        unknown(
            label,
            &format!(
                "Exact amount: {sign}{}.{:05}",
                amount / 100_000,
                amount % 100_000
            ),
        )
    }
}
fn equipment_raw(losses: &[SideLossInputs; 2]) -> [Vec<String>; 2] {
    losses.each_ref().map(|loss| {
        let Recorded::Value(equipment) = &loss.equipment else {
            return Vec::new();
        };
        let total: i128 = equipment
            .iter()
            .map(|entry| i128::from(entry.amount_raw))
            .sum();
        std::iter::once(total.to_string())
            .chain(equipment.iter().map(|entry| entry.amount_raw.to_string()))
            .collect()
    })
}
impl Episode {
    pub(super) fn lose_losses(&mut self, hour: i32) {
        self.loss_inputs = None;
        self.losses.push(LandBattleLossRead {
            hour,
            terminal: false,
            equipment_raw: None,
            sides: std::array::from_fn(|_| {
                vec![
                    unknown("Manpower lost", "Losses after the last census not recorded"),
                    unknown(
                        "Equipment lost",
                        "Losses after the last census not recorded",
                    ),
                ]
            }),
        });
    }
}
impl Census {
    fn loss_values(&mut self, hour: i32, losses: &[SideLossInputs; 2]) -> [Vec<BattleValue>; 2] {
        losses.each_ref().map(|loss| {
            let manpower = match loss.manpower {
                Recorded::Value(value) => known("Manpower lost", f64::from(value)),
                Recorded::NotRecorded => unknown("Manpower lost", "Not in this recording"),
            };
            let Recorded::Value(equipment) = &loss.equipment else {
                return vec![manpower, unknown("Equipment lost", "Not in this recording")];
            };
            let total = equipment
                .iter()
                .map(|entry| i128::from(entry.amount_raw))
                .sum();
            let mut values = vec![manpower, equipment_value("Equipment lost", total)];
            for entry in equipment {
                let name = self.variants.current(entry.variant).and_then(|raw| {
                    super::super::equipment::variant_names(
                        raw,
                        self.catalogue
                            .as_deref()
                            .map(|cat| cat as &dyn super::super::equipment::Texts),
                    )
                    .0
                });
                if name.is_none() {
                    self.warn(
                        hour,
                        format!(
                            "Equipment variant {}: name not recorded or content unavailable",
                            entry.variant
                        ),
                    );
                }
                // The handle distinguishes designs with identical display names.
                let label = format!(
                    "Equipment lost · {} (variant {})",
                    name.as_deref().unwrap_or("Name not recorded"),
                    entry.variant,
                );
                values.push(equipment_value(&label, i128::from(entry.amount_raw)));
            }
            values
        })
    }
    pub(super) fn observe_losses(&mut self, frame: &DecodedFrame) {
        let progress = slot_read(&frame.metrics, TableId::LandCombatProgress, frame.hour)
            .then(|| frame.complete::<tables::LandCombatProgress>().ok())
            .flatten();
        let by_combat: HashMap<_, _> = progress
            .into_iter()
            .flatten()
            .map(|row| (row.combat, row))
            .collect();
        let open: Vec<_> = self
            .open
            .iter()
            .map(|(&combat, &at)| (combat, at))
            .collect();
        for (combat, at) in open {
            let losses = by_combat.get(&combat).map(|row| {
                [
                    SideLossInputs {
                        manpower: row.attacker_manpower_loss,
                        equipment: row.attacker_equipment_losses.clone(),
                    },
                    SideLossInputs {
                        manpower: row.defender_manpower_loss,
                        equipment: row.defender_equipment_losses.clone(),
                    },
                ]
            });
            if let Some(losses) = losses {
                // Repeated confirmations need no repeated values in the reply.
                if self.episodes[at].loss_inputs.as_ref() == Some(&losses) {
                    continue;
                }
                let sides = self.loss_values(frame.hour, &losses);
                let equipment_raw = Some(equipment_raw(&losses));
                self.episodes[at].loss_inputs = Some(losses);
                self.episodes[at].losses.push(LandBattleLossRead {
                    hour: frame.hour,
                    terminal: false,
                    sides,
                    equipment_raw,
                });
            } else {
                let reason = "Hourly losses not recorded";
                let episode = &mut self.episodes[at];
                episode.loss_inputs = None;
                if episode.losses.last().is_some_and(|read| {
                    matches!(&read.sides[0][0], BattleValue::Unknown { unknown, .. } if unknown == reason)
                }) {
                    continue;
                }
                episode.losses.push(LandBattleLossRead {
                    hour: frame.hour,
                    terminal: false,
                    equipment_raw: None,
                    sides: std::array::from_fn(|_| {
                        vec![
                            unknown("Manpower lost", reason),
                            unknown("Equipment lost", reason),
                        ]
                    }),
                });
            }
        }
    }
    pub(super) fn observe_results(&mut self, frame: &DecodedFrame, labels: bool) {
        if !slot_read(&frame.metrics, TableId::LandCombatResults, frame.hour) {
            return;
        }
        let Ok(results) = frame.complete::<tables::LandCombatResults>() else {
            return;
        };
        let Some(coverage) = frame
            .complete::<tables::LandEventCoverage>()
            .ok()
            .and_then(|rows| rows.first())
            .filter(|row| row.complete && row.to_hour == frame.hour)
        else {
            return;
        };
        for result in results {
            if result.end_hour <= coverage.from_hour || result.end_hour > coverage.to_hour {
                continue;
            }
            let mut countries = [Vec::new(), Vec::new()];
            let mut countries_unknown = [!labels; 2];
            for (index, side) in result.sides.iter().enumerate() {
                for raw in side.countries.iter().chain(&side.secondary_countries) {
                    match labels.then(|| self.links.get(*raw)).flatten() {
                        Some(tag) => countries[index].push(tag.clone()),
                        None => {
                            countries_unknown[index] = true;
                            self.warn(
                                result.end_hour,
                                format!("Participant country reference {raw} unresolved"),
                            );
                        }
                    }
                }
                countries[index].sort_unstable();
                countries[index].dedup();
                countries_unknown[index] |= countries[index].is_empty();
            }
            let existing = self.latest.get(&result.combat).copied().filter(|&at| {
                let episode = &self.episodes[at];
                episode.province == result.province
                    && episode.result.is_none()
                    && episode.start.hour <= result.end_hour
                    && episode.runs.last().is_some_and(|run| {
                        run.last_hour >= coverage.from_hour
                            && run.last_hour <= result.end_hour
                            && run
                                .last_hour
                                .checked_add(1)
                                .is_some_and(|next| result.end_hour <= next)
                    })
            });
            let at = existing.unwrap_or_else(|| {
                let at = self.episodes.len();
                self.episodes.push(Episode {
                    id: format!("{}:{}:result:{}", frame.branch, result.combat, result.id),
                    province: result.province,
                    start: BattleMoment {
                        hour: result.end_hour,
                        bound: BattleBound::By,
                    },
                    end: None,
                    counter_base: 0,
                    losses: Vec::new(),
                    loss_inputs: None,
                    result: None,
                    runs: vec![LandBattleRun {
                        first_hour: result.end_hour,
                        last_hour: result.end_hour,
                        counts: [0; 2],
                        roster_unknown: true,
                        countries: countries.clone(),
                        countries_unknown,
                        duration: result.duration,
                    }],
                });
                at
            });
            let losses = result.losses.each_ref().map(SideLossInputs::from);
            let sides = self.loss_values(result.end_hour, &losses);
            let episode = &mut self.episodes[at];
            // The combat finalizer mask is distinct from the leader on-action verdict.
            let winner = match result.winners {
                1 => battles::BattleWinner::Side { side: 0 },
                2 => battles::BattleWinner::Side { side: 1 },
                0 => battles::BattleWinner::Unknown {
                    unknown: "No winning side".into(),
                },
                _ => battles::BattleWinner::Unknown {
                    unknown: "Both sides awarded a win".into(),
                },
            };
            episode.result = Some(LandBattleResultRead {
                hour: result.end_hour,
                duration: result.duration,
                winner,
                countries,
                countries_unknown,
            });
            episode.end = Some((
                frame.hour,
                BattleEnd::Moment(BattleMoment {
                    hour: result.end_hour,
                    bound: BattleBound::Exact,
                }),
            ));
            episode.losses.push(LandBattleLossRead {
                hour: result.end_hour,
                terminal: true,
                sides,
                equipment_raw: Some(equipment_raw(&losses)),
            });
            if self.open.get(&result.combat) == Some(&at) {
                self.open.remove(&result.combat);
            }
        }
    }
}
