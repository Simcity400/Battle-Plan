//! Hourly observer counters and post-flush FinalizeCombat results.
use super::*;
use replay_data::{rows::LandCombatResultsRow, rows::LandSideLosses, rows::Recorded};
use std::mem::size_of;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SideLossInputs {
    manpower: Option<i32>,
    equipment: Option<Vec<(u64, i64)>>,
}
impl SideLossInputs {
    /// The bytes it holds on the heap.
    pub(super) fn heap(&self) -> usize {
        self.equipment.as_ref().map_or(0, |equipment| {
            equipment.capacity() * size_of::<(u64, i64)>()
        })
    }
}
/// Each side's hourly counters as LandCombatProgress records them, attacker then defender.
pub(super) fn progress_inputs(
    row: &replay_data::rows::LandCombatProgressRow,
) -> [SideLossInputs; 2] {
    let manpower = |value: &Recorded<i32>| match value {
        Recorded::Value(value) => Some(*value),
        Recorded::NotRecorded => None,
    };
    [
        SideLossInputs {
            manpower: manpower(&row.attacker_manpower_loss),
            equipment: amounts(&row.attacker_equipment_losses),
        },
        SideLossInputs {
            manpower: manpower(&row.defender_manpower_loss),
            equipment: amounts(&row.defender_equipment_losses),
        },
    ]
}
fn amounts(value: &Recorded<Vec<replay_data::rows::LandEquipmentLoss>>) -> Option<Vec<(u64, i64)>> {
    match value {
        Recorded::Value(rows) => rows
            .iter()
            .map(|row| (row.variant, row.amount_raw))
            .collect::<Vec<_>>()
            .into(),
        Recorded::NotRecorded => None,
    }
}
impl From<&LandSideLosses> for SideLossInputs {
    fn from(loss: &LandSideLosses) -> Self {
        Self {
            manpower: Some(loss.manpower),
            equipment: loss
                .equipment
                .iter()
                .map(|row| (row.variant, row.amount_raw))
                .collect::<Vec<_>>()
                .into(),
        }
    }
}

const NOT_RECORDED: &str = "Not in this recording";

/// Equipment designs, each kept once for the branch however often it is read.
#[derive(Default, Serialize, Deserialize)]
pub(super) struct Designs {
    pub(super) values: Vec<LandEquipmentDesign>,
    #[serde(with = "dashboard_core::map_pairs")]
    indices: HashMap<LandEquipmentDesign, usize>,
}
impl Designs {
    fn intern(&mut self, design: LandEquipmentDesign) -> usize {
        if let Some(&index) = self.indices.get(&design) {
            return index;
        }
        self.values.push(design.clone());
        self.indices.insert(design, self.values.len() - 1);
        self.values.len() - 1
    }
}
/// One side's counters as read; variants pair a branch design with the signed fixed5 amount.
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct SideRead {
    pub(super) manpower: Option<i32>,
    pub(super) equipment: Option<Vec<(usize, i64)>>,
}
/// Recorded cumulative observer counters, with explicit unknown reads at gaps.
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct LossRead {
    pub(super) hour: i32,
    /// True only for the post-flush FinalizeCombat read, never inferred from absence.
    pub(super) terminal: bool,
    /// Attacker then defender, or why neither side's counters were read.
    pub(super) sides: Result<[SideRead; 2], String>,
}
impl LossRead {
    pub(super) fn casualties(&self) -> Option<f64> {
        self.sides
            .as_ref()
            .ok()?
            .iter()
            .map(|side| side.manpower.map(f64::from))
            .sum()
    }
    /// Each side's manpower and exact equipment total, for the census.
    pub(super) fn total(&self) -> LandBattleLossRead {
        let sides = match &self.sides {
            Ok(sides) => LandLossTotals::Read(sides.each_ref().map(|side| {
                let manpower = manpower(side.manpower);
                match &side.equipment {
                    // i64 amounts cannot overflow an i128 sum over any recorded list.
                    Some(variants) => LandLossTotal::Known {
                        manpower,
                        equipment: variants
                            .iter()
                            .map(|&(_, amount)| i128::from(amount))
                            .sum::<i128>()
                            .to_string(),
                    },
                    None => LandLossTotal::Unknown {
                        manpower,
                        unknown: NOT_RECORDED.into(),
                    },
                }
            })),
            Err(reason) => LandLossTotals::Unread(reason.clone()),
        };
        LandBattleLossRead {
            hour: self.hour,
            terminal: self.terminal,
            sides,
        }
    }
    /// A side lists every design when its designs or order differ from its
    /// previous read in the reply, else only the positions whose amounts changed.
    pub(super) fn wire(
        &self,
        previous: &mut [Option<Vec<(usize, i64)>>; 2],
        designs: &mut Numbered<usize>,
    ) -> LandBattleCourseRead {
        let sides = match &self.sides {
            Ok(sides) => LandLossSides::Read(std::array::from_fn(|index| {
                let side = &sides[index];
                let manpower = manpower(side.manpower);
                let Some(variants) = &side.equipment else {
                    previous[index] = None;
                    return LandLossSide::Unknown {
                        manpower,
                        unknown: NOT_RECORDED.into(),
                    };
                };
                match previous[index].replace(variants.clone()).filter(|before| {
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
                            .map(|&(design, amount)| (designs.index(design), amount.to_string()))
                            .collect(),
                    },
                }
            })),
            Err(reason) => {
                *previous = [None, None];
                LandLossSides::Unread(reason.clone())
            }
        };
        LandBattleCourseRead {
            hour: self.hour,
            terminal: self.terminal,
            sides,
        }
    }
}
fn manpower(value: Option<i32>) -> LandManpower {
    match value {
        Some(value) => LandManpower::Known(value),
        None => LandManpower::Unknown(NOT_RECORDED.into()),
    }
}
impl Episode {
    pub(super) fn lose_losses(&mut self, hour: i32) {
        self.loss_inputs = None;
        self.losses.push(LossRead {
            hour,
            terminal: false,
            sides: Err("Losses after the last census not recorded".into()),
        });
    }
}
impl Census {
    fn side_reads(&mut self, hour: i32, losses: &[SideLossInputs; 2]) -> [SideRead; 2] {
        losses.each_ref().map(|loss| SideRead {
            manpower: loss.manpower,
            equipment: loss.equipment.as_ref().map(|equipment| {
                equipment
                    .iter()
                    .map(|&(variant, amount)| (self.design(hour, variant), amount))
                    .collect()
            }),
        })
    }
    fn design(&mut self, hour: i32, variant: u64) -> usize {
        if let Some(&(design, named)) = self.named.get(&variant) {
            if !named {
                self.unnamed.push((hour, variant));
            }
            return design;
        }
        let name = self.variants.current(variant).and_then(|raw| {
            dashboard_base::products::equipment::variant_names(
                raw,
                self.catalogue
                    .as_deref()
                    .map(|cat| cat as &dyn dashboard_base::products::equipment::Texts),
            )
            .0
        });
        let named = name.is_some();
        if !named {
            self.unnamed.push((hour, variant));
        }
        // The handle keeps designs with identical display names apart.
        let design = self.designs.intern(LandEquipmentDesign {
            name,
            variant: variant.to_string(),
        });
        self.named.insert(variant, (design, named));
        design
    }
    /// Warns of each unnamed design read since, once an hour with the count of its reads.
    pub(super) fn warn_unnamed(&mut self) {
        if self.unnamed.is_empty() {
            return;
        }
        let mut unnamed = std::mem::take(&mut self.unnamed);
        unnamed.sort_unstable();
        for reads in unnamed.chunk_by(|left, right| left == right) {
            let (hour, variant) = reads[0];
            self.warnings.add(
                hour,
                format!("Equipment variant {variant}: name not recorded or content unavailable"),
                reads.len(),
            );
        }
        unnamed.clear();
        self.unnamed = unnamed;
    }
    /// Forgets the designs of the variants whose names `changed` moved.
    pub(super) fn renamed(
        &mut self,
        changed: &[(u64, dashboard_base::products::equipment::VariantVersion)],
    ) {
        for (variant, _) in changed {
            self.named.remove(variant);
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn observe_losses(&mut self, frame: &DecodedFrame, opened: Option<&[u64]>) {
        let progress = slot_read(
            self.format,
            &frame.metrics,
            TableId::LandCombatProgress,
            frame.hour,
        )
        .then(|| frame.complete::<tables::LandCombatProgress>().ok())
        .flatten();
        let open: Vec<_> = match opened {
            Some(opened) => opened
                .iter()
                .filter_map(|combat| self.open.get(combat).map(|at| (*combat, *at)))
                .collect(),
            None => {
                // Combat order, as LandCombatProgress lists them: a design's
                // first read numbers it for the branch, never hash order.
                let mut open: Vec<_> = self
                    .open
                    .iter()
                    .map(|(&combat, &at)| (combat, at))
                    .collect();
                open.sort_unstable();
                open
            }
        };
        #[cfg(test)]
        {
            self.work.row_visits += open
                .iter()
                .filter(|(combat, _)| {
                    progress.is_some_and(|progress| {
                        progress
                            .binary_search_by_key(combat, |row| row.combat)
                            .is_ok()
                    })
                })
                .count();
        }
        self.read_losses(frame.hour, open, |combat| {
            progress.and_then(|progress| {
                progress
                    .binary_search_by_key(&combat, |row| row.combat)
                    .ok()
                    .map(|at| progress_inputs(&progress[at]))
            })
        });
    }
    /// The hourly counters `progress` reads at `hour` for each of `open`'s battles, by its
    /// combat: none where they were not read.
    pub(super) fn read_losses(
        &mut self,
        hour: i32,
        open: Vec<(u64, usize)>,
        progress: impl Fn(u64) -> Option<[SideLossInputs; 2]>,
    ) {
        for (combat, at) in open {
            #[cfg(test)]
            {
                self.work.history_visits += 1;
            }
            if let Some(losses) = progress(combat) {
                // Repeated confirmations need no repeated values in the reply.
                if self.episodes[&at].loss_inputs.as_ref() == Some(&losses) {
                    continue;
                }
                let sides = self.side_reads(hour, &losses);
                let episode = held(&mut self.episodes, at);
                episode.loss_inputs = Some(losses);
                episode.losses.push(LossRead {
                    hour,
                    terminal: false,
                    sides: Ok(sides),
                });
            } else {
                let reason = "Hourly losses not recorded";
                self.dirty.insert(at);
                let episode = held(&mut self.episodes, at);
                episode.loss_inputs = None;
                if episode
                    .losses
                    .last()
                    .is_some_and(|read| matches!(&read.sides, Err(why) if why == reason))
                {
                    continue;
                }
                episode.losses.push(LossRead {
                    hour,
                    terminal: false,
                    sides: Err(reason.into()),
                });
            }
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn observe_results(&mut self, frame: &DecodedFrame, labels: bool) -> io::Result<()> {
        if !slot_read(
            self.format,
            &frame.metrics,
            TableId::LandCombatResults,
            frame.hour,
        ) {
            return Ok(());
        }
        let Ok(results) = frame.complete::<tables::LandCombatResults>() else {
            return Ok(());
        };
        let Some(coverage) = frame
            .complete::<tables::LandEventCoverage>()
            .ok()
            .and_then(|rows| rows.first())
            .filter(|row| row.complete && row.to_hour == frame.hour)
        else {
            return Ok(());
        };
        if results.is_empty() {
            return Ok(());
        }
        self.read_results(
            frame.branch,
            labels,
            (coverage.from_hour, coverage.to_hour),
            results,
        )
        .map(drop)
    }
    /// The finalizers a complete read under the coverage of the hours after `from_hour`
    /// through `to_hour`, the read's own hour, holds in table order. Returns the combats whose
    /// open battle they ended.
    pub(super) fn read_results<'r>(
        &mut self,
        branch: u64,
        labels: bool,
        (from_hour, to_hour): (i32, i32),
        results: impl IntoIterator<Item = &'r LandCombatResultsRow>,
    ) -> io::Result<Vec<u64>> {
        // Finalizers match the preceding accepted live endpoint, including
        // constant-counter reads deferred since the last publication.
        self.materialize()?;
        let mut ended = Vec::new();
        for result in results {
            #[cfg(test)]
            {
                self.work.row_visits += 1;
            }
            if result.end_hour <= from_hour || result.end_hour > to_hour {
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
            let latest = self.latest.get(&result.combat).copied();
            let existing = latest
                .filter(|at| self.episodes[at].ends(result.province, result.end_hour, from_hour));
            // A paged battle ended before `paged_before`, so only a finalizer
            // whose coverage reaches back past it can end one.
            let pending =
                latest.is_none() && self.paged_before.is_some_and(|before| from_hour < before);
            let at = existing.unwrap_or_else(|| {
                let at = self.reserve();
                if !pending {
                    self.started(result.end_hour);
                }
                self.episodes.insert(
                    at,
                    Episode {
                        id: format!("{branch}:{}:result:{}", result.combat, result.id),
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
                        prior_peaks: [None, None, None],
                        history: None,
                        runs: vec![LandBattleRun {
                            first_hour: result.end_hour,
                            last_hour: result.end_hour,
                            counts: [0; 2],
                            roster_unknown: true,
                            countries: countries.clone(),
                            countries_unknown,
                            duration: result.duration,
                        }],
                    },
                );
                at
            });
            let losses = result.losses.each_ref().map(SideLossInputs::from);
            let sides = self.side_reads(result.end_hour, &losses);
            self.dirty.insert(at);
            let episode = held(&mut self.episodes, at);
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
                to_hour,
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
                ended.push(result.combat);
            }
            if pending {
                self.dirty.remove(&at);
                let own = self.episodes.remove(&at).expect("finalizer's own battle");
                self.pending.push(Pending {
                    combat: result.combat,
                    from_hour,
                    at,
                    own,
                });
            }
        }
        Ok(ended)
    }
}
impl Episode {
    /// Whether a finalizer of this battle's combat, dated `end_hour` and
    /// covering the hours after `from_hour`, ends it.
    pub(super) fn ends(&self, province: u32, end_hour: i32, from_hour: i32) -> bool {
        self.province == province
            && self.result.is_none()
            && self.start.hour <= end_hour
            && self.runs.last().is_some_and(|run| {
                run.last_hour >= from_hour
                    && run.last_hour <= end_hour
                    && run
                        .last_hour
                        .checked_add(1)
                        .is_some_and(|next| end_hour <= next)
            })
    }
}
