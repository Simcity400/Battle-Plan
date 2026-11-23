use super::*;
use std::ops::Range;

pub struct StatusPart {
    packet: StatusPacket,
    membership_cache: super::status_membership::Cache,
    pub range: Range<usize>,
    pub organisation_refused: u64,
    outcome: Outcome,
}

impl StatusPart {
    pub fn vacant() -> Self {
        Self {
            packet: StatusPacket::vacant(),
            membership_cache: Default::default(),
            range: 0..0,
            organisation_refused: 0,
            outcome: Outcome::Rows { len: 0 },
        }
    }

    pub fn allocated_bytes(&self) -> usize {
        self.packet.allocated_bytes() + self.membership_cache.allocated_bytes()
    }

    pub fn fail(&mut self, reason: &'static str) {
        self.outcome = Outcome::Unavailable { reason };
        self.organisation_refused = 0;
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        seeds: &[DivisionSeed],
        prepared: &StatusPacket,
    ) {
        self.capture_signaled(memory, base, state, seeds, prepared, &());
    }

    pub fn capture_signaled(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        seeds: &[DivisionSeed],
        prepared: &StatusPacket,
        signals: &impl crate::corders_group_member::MembershipSignals,
    ) {
        signals.prepare(seeds.len());
        self.organisation_refused = 0;
        let result = (|| {
            let units = prepared
                .units
                .get(self.range.clone())
                .ok_or("division status part range invalid")?;
            let seeds = seeds
                .get(self.range.clone())
                .ok_or("division status part sources invalid")?;
            self.packet
                .grow(units.len(), MembershipCapacity::default())
                .map_err(|_| "status seed allocation failed")?;
            self.packet.clear();
            let _ = engine_allocation::advisory(|| {
                self.packet.select_supply(
                    prepared
                        .supply_requested
                        .get(self.range.clone())
                        .unwrap_or(&[]),
                )
            });
            for unit in units {
                let mut unit = *unit;
                unit.maxima = match prepared.maxima.get(unit.maxima) {
                    Some(value) => {
                        let index = self.packet.maxima.len();
                        self.packet.maxima.push(*value);
                        index
                    }
                    None => usize::MAX,
                };
                self.packet.units.push(unit);
            }
            self.packet.outcome = match prepared.outcome {
                Outcome::Unavailable { reason } => Outcome::Unavailable { reason },
                _ => Outcome::Rows { len: units.len() },
            };
            if !matches!(prepared.outcome, Outcome::Unavailable { .. }) {
                self.membership_cache
                    .acquire(memory, base, seeds, &mut self.packet, signals)?;
                let experience = prepared
                    .prepared_experience
                    .get(self.range.start..self.range.end.min(prepared.prepared_experience.len()))
                    .unwrap_or(&[]);
                let countries = ccountry::Reader::new(memory, base, state)
                    .map(|countries| (units.len(), countries));
                self.packet.finish_holding(
                    memory,
                    base,
                    state,
                    seeds,
                    Outcome::Rows { len: units.len() },
                    experience,
                    countries,
                    false,
                );
                for index in 0..self.packet.units.len() {
                    let has_order = self.packet.units[index].has_order.unwrap_or_else(|| {
                        self.packet
                            .membership
                            .outcome(self.packet.units[index].membership)
                    });
                    let exercising = self.packet.exercising_at(index);
                    self.packet.units[index].has_order = Some(has_order);
                    self.packet.units[index].exercising = Some(exercising);
                }
            }
            self.organisation_refused = self
                .packet
                .capture_movement_organisation(memory, base, state, seeds);
            Ok(self.packet.outcome)
        })();
        self.outcome = result.unwrap_or_else(|reason| Outcome::Unavailable { reason });
    }
}

pub(super) fn census_authenticated(seed: &DivisionSeed) -> bool {
    seed.selected
        .is_some_and(|selected| selected.group_identity)
}

impl StatusPacket {
    pub fn prepare_status_stitch(&mut self, parts: &[StatusPart]) -> Outcome {
        let result = (|| {
            if let Outcome::Unavailable { reason } = self.outcome {
                return Err(reason);
            }
            let mut end = 0;
            for part in parts {
                if part.range.start != end || part.range.end > self.units.len() {
                    return Err("division status part range invalid");
                }
                if let Outcome::Unavailable { reason } = part.outcome {
                    return Err(reason);
                }
                if part.outcome.len() != part.range.len() {
                    return Err("division status part length invalid");
                }
                end = part.range.end;
            }
            if end != self.units.len() || end != self.outcome.len() {
                return Err("division status parts incomplete");
            }
            let origins: usize = parts
                .iter()
                .map(|part| part.packet.supply_origins_len)
                .sum();
            if origins > self.supply_origins.len() {
                let additional = origins - self.supply_origins.len();
                engine_allocation::reserve(&mut self.supply_origins, additional)
                    .map_err(|_| "division supply origins allocation failed")?;
                self.supply_origins
                    .resize(origins, engine_native_parts::cunit_supply_inputs::Origin::default());
            }
            Ok(self.outcome)
        })();
        self.outcome = result.unwrap_or_else(|reason| Outcome::Unavailable { reason });
        self.prepare_border_index();
        self.outcome
    }

    pub fn stitch_status_parts(&mut self, parts: &[StatusPart]) -> Outcome {
        self.prepare_status_stitch(parts);
        self.finish_status_stitch(parts)
    }

    pub fn finish_status_stitch(&mut self, parts: &[StatusPart]) -> Outcome {
        let result = (|| {
            if let Outcome::Unavailable { reason } = self.outcome {
                return Err(reason);
            }
            let end = self.units.len();
            let origins = parts
                .iter()
                .map(|part| part.packet.supply_origins_len)
                .sum();
            if origins > self.supply_origins.len() {
                let additional = origins - self.supply_origins.len();
                engine_allocation::reserve(&mut self.supply_origins, additional)
                    .map_err(|_| "division supply origins allocation failed")?;
                self.supply_origins
                    .resize(origins, engine_native_parts::cunit_supply_inputs::Origin::default());
            }
            self.supply_len = end;
            self.supply_origins_len = 0;
            for part in parts {
                for (index, unit) in part.packet.units.iter().enumerate() {
                    let destination = &mut self.units[part.range.start + index];
                    let maxima = destination.maxima;
                    let threshold = destination.threshold;
                    let border_combat_stats = destination.border_combat_stats;
                    *destination = *unit;
                    destination.border_combat_stats = border_combat_stats;
                    destination.maxima = maxima;
                    destination.threshold = threshold;
                    let mut supply = part.packet.supply[index];
                    if supply.origins_admitted {
                        supply.origins_start += self.supply_origins_len;
                    }
                    self.supply[part.range.start + index] = supply;
                }
                let len = part.packet.supply_origins_len;
                self.supply_origins[self.supply_origins_len..self.supply_origins_len + len]
                    .copy_from_slice(&part.packet.supply_origins[..len]);
                self.supply_origins_len += len;
            }
            self.required.supply_origins = self.required.supply_origins.max(origins);
            Ok(self.outcome)
        })();
        self.outcome = result.unwrap_or_else(|reason| Outcome::Unavailable { reason });
        self.outcome
    }
    pub(super) fn prepare_border_index(&mut self) {
        if self.border_index.len() == self.units.len() {
            return;
        }
        self.border_index.clear();
        self.border_index.extend(
            self.units
                .iter()
                .enumerate()
                .map(|(i, unit)| (unit.division, i)),
        );
        self.border_index.sort_unstable_by_key(|entry| entry.0);
    }
}
