use super::*;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ModelInputsRow {
    pub division: u64,
    pub model_number: Option<i32>,
    pub camouflage: Option<Camouflage>,
    pub exercising: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelInputsReport {
    pub outcome: Outcome,
    pub training: u32,
}

impl StatusPacket {
    pub fn capture_model_inputs<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        state: usize,
        seeds: &[DivisionSeed],
        source_outcome: Outcome,
        output: &mut [ModelInputsRow],
    ) -> ModelInputsReport {
        self.clear();
        let mut training = 0;
        let result = (|| {
            if let Outcome::Unavailable { reason } = source_outcome {
                return Err(reason);
            }
            let seeds = seeds
                .get(..source_outcome.len())
                .ok_or("division model input source count invalid")?;
            self.grow(seeds.len(), MembershipCapacity::default())
                .map_err(|_| "division model input allocation failed")?;
            let countries = ccountry::Reader::new(memory, base, state);
            let identity = engine_sites::at(base, VTABLE_RVA).map(|at| at as u64);
            let len = seeds.len().min(output.len());
            let mut entries = self.acclimatization.take();
            for (unit, seed) in seeds[..len].iter().enumerate() {
                if let Some(entries) = entries.as_deref_mut()
                    && unit.is_multiple_of(64)
                {
                    entries.capture(memory, base, &seeds[unit..(unit + 64).min(len)]);
                }
                self.units.push(StatusUnitInputs {
                    division: seed.division,
                    maxima: usize::MAX,
                    ..StatusUnitInputs::default()
                });
                let row = &mut output[unit];
                *row = ModelInputsRow {
                    division: seed.division,
                    ..ModelInputsRow::default()
                };
                if seed.primary_vtable != identity
                    || seed.handle != Some(seed.division)
                    || seed.division == 0
                {
                    continue;
                }
                row.model_number = seed.model_number;
                let first = entries
                    .as_deref()
                    .and_then(|entries| entries.first(unit % 64));
                row.camouflage = acclimatization_winner(memory, base, seed, first)
                    .and_then(|(object, winner)| {
                        self.read_camouflage(memory, base, &object, winner)
                    })
                    .ok();
                row.exercising = (|| {
                    let group = seed
                        .orders_group
                        .ok_or("division presentation field unreadable")?;
                    self.units[unit].group = group;
                    if self.model_training(memory, base, group)? == 0 {
                        return Ok(0);
                    }
                    training += 1;
                    let countries = countries.as_ref().map_err(|reason| *reason)?;
                    let inputs = self.capture_exercise(
                        memory,
                        base,
                        seed,
                        unit,
                        None,
                        |packet, raw| {
                            packet.exercise_autonomous(memory, base, state, countries, raw)
                        },
                        |packet| {
                            let pointer = seed
                                .stat_block
                                .ok_or("division presentation field unreadable")?;
                            let maxima = engine_native_parts::csub_unit_definition::capture_division_maxima(
                                memory, base, pointer,
                            )?;
                            packet.units[unit].maxima = packet.maxima.len();
                            packet.maxima.push(maxima);
                            Ok(maxima.effective().0)
                        },
                    )?;
                    self.units[unit].exercise = Some(inputs);
                    self.exercising_at(unit)
                })()
                .ok();
            }
            self.acclimatization = entries;
            let source_missing = match source_outcome {
                Outcome::Truncated { unread_rows, .. } => unread_rows,
                _ => 0,
            };
            let missing = u32::try_from(seeds.len() - len)
                .ok()
                .and_then(|omitted| source_missing.checked_add(omitted))
                .ok_or("division model input count overflow")?;
            Ok(if missing == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated {
                    len,
                    unread_rows: missing,
                }
            })
        })();
        self.outcome = result.unwrap_or_else(|reason| Outcome::Unavailable { reason });
        ModelInputsReport {
            outcome: self.outcome,
            training,
        }
    }

    fn model_training(
        &mut self,
        memory: &impl Memory,
        base: usize,
        group: usize,
    ) -> Result<u8, &'static str> {
        if group == 0 {
            return Ok(0);
        }
        let slot = status_pointer_slot(&self.group_index, group, |i| self.groups[i].pointer);
        if self.group_index[slot] == 0
            && let Err(reason) = crate::corders_group_member::group_identity(memory, base, group)
        {
            self.groups.push(StatusGroupInputs {
                pointer: group,
                training: Err(reason),
                stop_at_maximum: None,
            });
            self.group_index[slot] = self.groups.len();
        }
        self.training(memory, group)
    }
}
