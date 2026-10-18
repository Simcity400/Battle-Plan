//! Worker drain of owned strike entries; conversion never follows game pointers.
use crate::{
    clock::qpc,
    table_capture::{CaptureRows, grow_vec},
    table_conversion::unavailable,
};
use collector_observers::nuclear_strikes::{self as producer, Entry, Item, Phase};
use engine_allocation as allocation;
use engine_events::{event_feed_sites::CountryIdentity, nuclear_strike_sites::Unit};
use engine_native::{Memory, walk::Outcome};
use replay_data::{
    rows::NuclearBuildingDamage, rows::NuclearEquipmentLoss, rows::NuclearStrikePart,
    rows::NuclearStrikeRefusal, rows::NuclearStrikeRow, rows::NuclearUnitDamage,
    rows::NuclearWarSupport, rows::NuclearWingLoss, rows::TableId, rows::TableRows,
};
use replay_frame::frame::TableView;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::NuclearStrikes];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];
pub type Capacities = usize;
pub struct Captured {
    rows: CaptureRows<Entry>,
}
impl Captured {
    pub fn vacant() -> Self {
        Self::with_capacities(&0)
    }
    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(*c),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
    }
    pub fn grow(&mut self, c: &mut Capacities) {
        *c = (*c).max(producer::capacity());
        grow_vec(&mut self.rows.rows, *c);
    }
    pub fn fail(&mut self, reason: &'static str) {
        self.rows.fail(Outcome::Unavailable { reason });
    }
    pub fn capture_growing(
        &mut self,
        _memory: &impl Memory,
        _base: usize,
        _state: usize,
    ) -> [u64; N] {
        self.reset();
        let started = qpc();
        self.rows.outcome = match producer::drain_growing(&mut self.rows.rows) {
            Ok(len) => Outcome::Rows { len },
            Err(reason) => Outcome::Unavailable { reason },
        };
        [qpc().saturating_sub(started).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }
    pub fn tables(&self) -> [TableView; N] {
        let Outcome::Rows { len } = self.rows.outcome else {
            return [unavailable(match self.rows.outcome {
                Outcome::Unavailable { reason } => reason,
                _ => "nuclear strike incomplete drain",
            })];
        };
        match project(&self.rows.rows[..len]) {
            Ok(rows) => [TableView::Complete(TableRows::NuclearStrikes(rows))],
            Err(reason) => {
                producer::lost_output(len);
                [unavailable(reason)]
            }
        }
    }
}

/// Conversion runs on the spool thread; growth keeps allocation failure terminal.
fn push<T>(values: &mut Vec<T>, value: T) -> Result<(), &'static str> {
    allocation::reserve(values, 1).map_err(|_| "collector allocation failed")?;
    values.push(value);
    Ok(())
}

fn country(identity: CountryIdentity) -> replay_data::event_feed_rows::EventFeedCountry {
    replay_data::event_feed_rows::EventFeedCountry {
        reference: identity.reference,
        id: identity.id,
        instance: identity.instance,
    }
}

/// One unit's damage, when its after read is the same unit as its before read.
fn damage(before: &Unit, after: &Unit) -> Option<NuclearUnitDamage> {
    (before.handle == after.handle && before.class == after.class).then_some(NuclearUnitDamage {
        handle: before.handle,
        class: before.class,
        owner: before.owner,
        strength_before: before.strength,
        strength_after: after.strength,
        organisation_before: before.organisation,
        organisation_after: after.organisation,
        manpower_before: before.manpower,
        manpower_after: after.manpower,
    })
}

/// One strike being assembled from its entries.
struct Assembly {
    row: NuclearStrikeRow,
    /// The current damage call's before reads, and how many afters it paired.
    before: Vec<Unit>,
    paired: usize,
}
impl Assembly {
    fn refuse(&mut self, part: NuclearStrikePart, reason: &[u8]) -> Result<(), &'static str> {
        match self
            .row
            .refusals
            .iter_mut()
            .find(|refusal| refusal.part == part && refusal.reason == reason)
        {
            Some(refusal) => {
                refusal.count = refusal
                    .count
                    .checked_add(1)
                    .ok_or("nuclear strike refusal count overflow")?;
            }
            None => push(
                &mut self.row.refusals,
                NuclearStrikeRefusal {
                    part,
                    reason: allocation::copy_vec(reason)
                        .map_err(|_| "collector allocation failed")?,
                    count: 1,
                },
            )?,
        }
        Ok(())
    }
    /// Close the current damage call: a before read left without its after
    /// read is a unit whose damage was not observed.
    fn end_call(&mut self) -> Result<(), &'static str> {
        if self.paired < self.before.len() {
            self.refuse(
                NuclearStrikePart::Units,
                b"nuclear strike unit read before and after differ",
            )?;
        }
        self.before.clear();
        self.paired = 0;
        Ok(())
    }
    fn finish(mut self) -> Result<NuclearStrikeRow, &'static str> {
        self.end_call()?;
        // One row per country, pool kind and variant, in key order.
        let equipment = &mut self.row.equipment;
        equipment.sort_unstable_by_key(|loss| (loss.country, loss.wing, loss.variant));
        let mut merged: Vec<NuclearEquipmentLoss> =
            allocation::vec(equipment.len()).map_err(|_| "collector allocation failed")?;
        for loss in equipment.drain(..) {
            match merged.last_mut() {
                Some(last)
                    if (last.country, last.wing, last.variant)
                        == (loss.country, loss.wing, loss.variant) =>
                {
                    last.amount_raw = last
                        .amount_raw
                        .checked_add(loss.amount_raw)
                        .ok_or("nuclear strike equipment sum overflow")?;
                }
                _ => merged.push(loss), // within the reserved length
            }
        }
        merged.retain(|loss| loss.amount_raw != 0);
        self.row.equipment = merged;
        Ok(self.row)
    }
}

/// Group the drained entries into strikes: each strike entry starts a row and
/// the entries after it, until the next, are its outcome.
fn project(entries: &[Entry]) -> Result<Vec<NuclearStrikeRow>, &'static str> {
    let mut rows = Vec::new();
    let mut current: Option<Assembly> = None;
    for entry in entries {
        if let Item::Strike { hour, strike } = entry.item {
            if let Some(done) = current.take() {
                push(&mut rows, done.finish()?)?;
            }
            current = Some(Assembly {
                row: NuclearStrikeRow {
                    id: entry.id,
                    hour,
                    launcher: country(strike.launcher),
                    nuke_type: strike.nuke_type,
                    province: strike.province,
                    state: strike.state,
                    owner: country(strike.owner),
                    controller: strike.controller,
                    ..Default::default()
                },
                before: Vec::new(),
                paired: 0,
            });
            continue;
        }
        let strike = current
            .as_mut()
            .ok_or("nuclear strike outcome precedes its strike")?;
        match entry.item {
            Item::Strike { .. } => unreachable!("handled above"),
            Item::WarSupport {
                country,
                before,
                after,
            } => push(
                &mut strike.row.war_support,
                NuclearWarSupport {
                    country,
                    before_raw: before,
                    after_raw: after,
                },
            )?,
            Item::Building {
                state,
                province,
                controller,
                building_type,
                amount_raw,
            } => push(
                &mut strike.row.buildings,
                NuclearBuildingDamage {
                    state,
                    province,
                    controller,
                    building_type,
                    amount_raw,
                },
            )?,
            Item::Unit {
                phase: Phase::Before,
                unit,
            } => {
                if strike.paired != 0 {
                    strike.end_call()?;
                }
                push(&mut strike.before, unit)?;
            }
            Item::Unit {
                phase: Phase::After,
                unit,
            } => {
                let paired = strike
                    .before
                    .get(strike.paired)
                    .and_then(|before| damage(before, &unit));
                strike.paired += 1;
                match paired {
                    Some(damage) => push(&mut strike.row.units, damage)?,
                    None => strike.refuse(
                        NuclearStrikePart::Units,
                        b"nuclear strike unit read before and after differ",
                    )?,
                }
            }
            Item::Equipment {
                wing,
                country,
                variant,
                amount_raw,
            } => push(
                &mut strike.row.equipment,
                NuclearEquipmentLoss {
                    country,
                    wing,
                    variant,
                    amount_raw,
                },
            )?,
            Item::Crew {
                country,
                before,
                after,
            } => push(
                &mut strike.row.wings,
                NuclearWingLoss {
                    country,
                    manpower_before: before.into(),
                    manpower_after: after.into(),
                },
            )?,
            Item::Refused { part, reason } => strike.refuse(part, reason.as_bytes())?,
        }
    }
    if let Some(done) = current.take() {
        push(&mut rows, done.finish()?)?;
    }
    if rows.windows(2).any(|pair| pair[0].id >= pair[1].id) {
        return Err("nuclear strike invalid event order");
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_events::{nuclear_strike_sites::RAILWAY_GUN_CLASS, nuclear_strike_sites::Strike};

    fn identity(id: u32) -> CountryIdentity {
        CountryIdentity {
            reference: id,
            id,
            instance: id + 7,
        }
    }

    fn entry(id: u64, item: Item) -> Entry {
        Entry { id, item }
    }
    fn strike(id: u64) -> Entry {
        entry(
            id,
            Item::Strike {
                hour: 60_827_795,
                strike: Strike {
                    launcher: identity(4_999),
                    nuke_type: 1,
                    province: 13_000,
                    state: 1_204,
                    owner: identity(1_500),
                    controller: 77,
                },
            },
        )
    }
    fn unit(handle: u64, strength: i64) -> Unit {
        Unit {
            handle,
            class: 0,
            owner: 3,
            strength,
            organisation: Some(strength / 2),
            manpower: strength / 10,
        }
    }

    #[test]
    fn strikes_keep_their_target_and_outcome_and_pair_each_unit_call() {
        let gun = Unit {
            class: RAILWAY_GUN_CLASS,
            organisation: None,
            ..unit(9, 100_000)
        };
        let entries = [
            strike(4),
            entry(
                5,
                Item::WarSupport {
                    country: 1_500,
                    before: 50_000,
                    after: 0,
                },
            ),
            entry(
                6,
                Item::Unit {
                    phase: Phase::Before,
                    unit: unit(1, 900_000),
                },
            ),
            entry(
                7,
                Item::Unit {
                    phase: Phase::Before,
                    unit: gun,
                },
            ),
            entry(
                8,
                Item::Unit {
                    phase: Phase::After,
                    unit: unit(1, 300_000),
                },
            ),
            entry(
                9,
                Item::Unit {
                    phase: Phase::After,
                    unit: Unit { strength: 0, ..gun },
                },
            ),
            // The second province's call reaches the first unit again.
            entry(
                10,
                Item::Unit {
                    phase: Phase::Before,
                    unit: unit(1, 300_000),
                },
            ),
            entry(
                11,
                Item::Unit {
                    phase: Phase::After,
                    unit: unit(1, 100_000),
                },
            ),
            entry(
                12,
                Item::Building {
                    state: 1_204,
                    province: None,
                    controller: 77,
                    building_type: 0xf004,
                    amount_raw: 20_000_000,
                },
            ),
            entry(
                13,
                Item::Crew {
                    country: 1_500,
                    before: 400,
                    after: 0,
                },
            ),
            strike(14),
        ];
        let rows = project(&entries).unwrap();
        assert_eq!(rows.len(), 2);
        let first = &rows[0];
        assert_eq!(
            (
                first.id,
                first.launcher.instance,
                first.nuke_type,
                first.state,
                first.owner.id,
                first.controller
            ),
            (4, 5_006, 1, 1_204, 1_500, 77)
        );
        assert_eq!(first.war_support[0].after_raw, 0);
        assert_eq!(first.units.len(), 3);
        assert_eq!(
            (
                first.units[0].strength_before,
                first.units[0].strength_after
            ),
            (900_000, 300_000)
        );
        assert_eq!(first.units[1].organisation_before, None);
        assert_eq!(first.units[1].strength_after, 0);
        assert_eq!(first.units[2].strength_after, 100_000);
        assert_eq!(first.buildings[0].amount_raw, 20_000_000);
        assert_eq!(
            (
                first.wings[0].manpower_before,
                first.wings[0].manpower_after
            ),
            (400, 0)
        );
        assert!(first.refusals.is_empty());
        assert_eq!(rows[1].id, 14);
        assert!(rows[1].units.is_empty() && rows[1].refusals.is_empty());
    }

    #[test]
    fn equipment_sums_per_country_pool_and_variant_keeping_gains() {
        let loss = |country, wing, variant, amount_raw| Item::Equipment {
            wing,
            country,
            variant,
            amount_raw,
        };
        let entries = [
            strike(1),
            entry(2, loss(3, false, 7, 200_000)),
            entry(3, loss(3, true, 7, 50_000)),
            entry(4, loss(3, false, 7, 100_000)),
            entry(5, loss(2, false, 8, -5)),
            entry(6, loss(3, false, 9, 10)),
            entry(7, loss(3, false, 9, -10)),
        ];
        let rows = project(&entries).unwrap();
        let summed: Vec<_> = rows[0]
            .equipment
            .iter()
            .map(|loss| (loss.country, loss.wing, loss.variant, loss.amount_raw))
            .collect();
        assert_eq!(
            summed,
            [
                (2, false, 8, -5),
                (3, false, 7, 300_000),
                (3, true, 7, 50_000)
            ]
        );
    }

    #[test]
    fn refusals_are_counted_by_part_and_reason_and_unpaired_units_refuse() {
        let entries = [
            strike(1),
            entry(
                2,
                Item::Refused {
                    part: NuclearStrikePart::Units,
                    reason: "nuclear strike unit class unsupported",
                },
            ),
            entry(
                3,
                Item::Refused {
                    part: NuclearStrikePart::Units,
                    reason: "nuclear strike unit class unsupported",
                },
            ),
            entry(
                4,
                Item::Unit {
                    phase: Phase::Before,
                    unit: unit(1, 5),
                },
            ),
            entry(
                5,
                Item::Unit {
                    phase: Phase::Before,
                    unit: unit(2, 5),
                },
            ),
            entry(
                6,
                Item::Unit {
                    phase: Phase::After,
                    unit: unit(2, 5),
                },
            ),
        ];
        let row = &project(&entries).unwrap()[0];
        assert!(row.units.is_empty());
        assert!(!row.complete(NuclearStrikePart::Units));
        assert!(row.complete(NuclearStrikePart::Buildings));
        let counts: Vec<_> = row
            .refusals
            .iter()
            .map(|refusal| (refusal.reason.as_slice(), refusal.count))
            .collect();
        assert_eq!(
            counts,
            [
                (&b"nuclear strike unit class unsupported"[..], 2),
                (&b"nuclear strike unit read before and after differ"[..], 2),
            ]
        );
    }

    #[test]
    fn an_outcome_without_its_strike_refuses_the_table() {
        let entries = [entry(
            1,
            Item::WarSupport {
                country: 1,
                before: 0,
                after: 0,
            },
        )];
        assert!(project(&entries).is_err());
        assert!(project(&[strike(2), strike(1)]).is_err());
        assert!(project(&[]).unwrap().is_empty());
    }
}
