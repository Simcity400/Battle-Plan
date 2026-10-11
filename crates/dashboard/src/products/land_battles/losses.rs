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

const NOT_RECORDED: &str = "Not in this recording";

/// Equipment loss labels, each kept once for the branch however often it is read.
#[derive(Default)]
pub(super) struct Labels {
    pub(super) values: Vec<String>,
    indices: HashMap<String, usize>,
}
impl Labels {
    fn intern(&mut self, label: String) -> usize {
        if let Some(&index) = self.indices.get(&label) {
            return index;
        }
        self.values.push(label.clone());
        self.indices.insert(label, self.values.len() - 1);
        self.values.len() - 1
    }
}
/// One side's counters as read; variants pair a branch label with the signed fixed5 amount.
pub(super) struct SideRead {
    pub(super) manpower: Recorded<i32>,
    pub(super) equipment: Recorded<Vec<(usize, i64)>>,
}
/// Recorded cumulative observer counters, with explicit unknown reads at gaps.
pub(super) struct LossRead {
    pub(super) hour: i32,
    /// True only for the post-flush FinalizeCombat read, never inferred from absence.
    pub(super) terminal: bool,
    /// Attacker then defender, or why neither side's counters were read.
    pub(super) sides: Result<[SideRead; 2], &'static str>,
}
impl LossRead {
    pub(super) fn casualties(&self) -> Option<f64> {
        self.sides
            .as_ref()
            .ok()?
            .iter()
            .map(|side| match side.manpower {
                Recorded::Value(value) => Some(f64::from(value)),
                Recorded::NotRecorded => None,
            })
            .sum()
    }
    /// A side lists every variant when its labels or order differ from its
    /// previous read in the reply, else only the positions whose amounts changed.
    pub(super) fn wire<'a>(
        &'a self,
        previous: &mut [Option<&'a [(usize, i64)]>; 2],
        labels: &mut Numbered<usize>,
    ) -> LandBattleLossRead {
        let sides = match &self.sides {
            Ok(sides) => LandLossSides::Read(std::array::from_fn(|index| {
                let side = &sides[index];
                let manpower = match side.manpower {
                    Recorded::Value(value) => LandManpower::Known(value),
                    Recorded::NotRecorded => LandManpower::Unknown(NOT_RECORDED.into()),
                };
                let Recorded::Value(variants) = &side.equipment else {
                    previous[index] = None;
                    return LandLossSide::Unknown {
                        manpower,
                        unknown: NOT_RECORDED.into(),
                    };
                };
                match previous[index].replace(variants).filter(|before| {
                    before.len() == variants.len()
                        && before
                            .iter()
                            .zip(variants)
                            .all(|(then, now)| then.0 == now.0)
                }) {
                    Some(before) => LandLossSide::Changed {
                        manpower,
                        changed: before
                            .iter()
                            .zip(variants)
                            .enumerate()
                            .filter(|(_, (then, now))| then.1 != now.1)
                            .map(|(at, (_, now))| (at, now.1.to_string()))
                            .collect(),
                    },
                    None => LandLossSide::Variants {
                        manpower,
                        variants: variants
                            .iter()
                            .map(|&(label, amount)| (labels.index(label), amount.to_string()))
                            .collect(),
                    },
                }
            })),
            Err(reason) => {
                *previous = [None, None];
                LandLossSides::Unread((*reason).into())
            }
        };
        LandBattleLossRead {
            hour: self.hour,
            terminal: self.terminal,
            sides,
        }
    }
}
impl Episode {
    pub(super) fn lose_losses(&mut self, hour: i32) {
        self.loss_inputs = None;
        self.losses.push(LossRead {
            hour,
            terminal: false,
            sides: Err("Losses after the last census not recorded"),
        });
    }
}
impl Census {
    fn side_reads(&mut self, hour: i32, losses: &[SideLossInputs; 2]) -> [SideRead; 2] {
        losses.each_ref().map(|loss| SideRead {
            manpower: loss.manpower,
            equipment: match &loss.equipment {
                Recorded::Value(equipment) => Recorded::Value(
                    equipment
                        .iter()
                        .map(|entry| (self.label(hour, entry.variant), entry.amount_raw))
                        .collect(),
                ),
                Recorded::NotRecorded => Recorded::NotRecorded,
            },
        })
    }
    fn label(&mut self, hour: i32, variant: u64) -> usize {
        let name = self.variants.current(variant).and_then(|raw| {
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
                format!("Equipment variant {variant}: name not recorded or content unavailable"),
            );
        }
        // The handle distinguishes designs with identical display names.
        self.labels.intern(format!(
            "Equipment lost · {} (variant {variant})",
            name.as_deref().unwrap_or("Name not recorded"),
        ))
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
                let sides = self.side_reads(frame.hour, &losses);
                self.episodes[at].loss_inputs = Some(losses);
                self.episodes[at].losses.push(LossRead {
                    hour: frame.hour,
                    terminal: false,
                    sides: Ok(sides),
                });
            } else {
                let reason = "Hourly losses not recorded";
                let episode = &mut self.episodes[at];
                episode.loss_inputs = None;
                if episode
                    .losses
                    .last()
                    .is_some_and(|read| matches!(read.sides, Err(why) if why == reason))
                {
                    continue;
                }
                episode.losses.push(LossRead {
                    hour: frame.hour,
                    terminal: false,
                    sides: Err(reason),
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
            let sides = self.side_reads(result.end_hour, &losses);
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
            episode.losses.push(LossRead {
                hour: result.end_hour,
                terminal: true,
                sides: Ok(sides),
            });
            if self.open.get(&result.combat) == Some(&at) {
                self.open.remove(&result.combat);
            }
        }
    }
}
