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
use replay_frame::frame::{TableInput, TableView};
use std::sync::atomic::{AtomicBool, Ordering};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::NuclearStrikes];
pub type Capacities = usize;
pub struct Captured {
    rows: CaptureRows<Entry>,
    loss_recorded: AtomicBool,
}
impl Captured {
    pub fn vacant() -> Self {
        Self::with_capacities(&0)
    }
    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(*c),
            loss_recorded: AtomicBool::new(false),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.loss_recorded.store(false, Ordering::Relaxed);
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
        self.loss_recorded.store(false, Ordering::Relaxed);
    }
    /// # Safety
    /// Caller must hold hourly capture admission as the sole consumer of nuclear-strike events
    /// while observer storage stays live and teardown is excluded.
    pub unsafe fn capture(
        &mut self,
        _memory: &impl Memory,
        _base: usize,
        _state: usize,
    ) -> [u64; N] {
        self.reset();
        let started = qpc();
        // SAFETY: Caller holds hourly capture admission as the sole nuclear-strikes consumer.
        self.rows.outcome = match unsafe { producer::drain(&mut self.rows.rows) } {
            Ok(len) => Outcome::Rows { len },
            Err(reason) => Outcome::Unavailable { reason },
        };
        [qpc().saturating_sub(started).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }
    fn project_tables(&self, record_loss: bool) -> [TableView; N] {
        let Outcome::Rows { len } = self.rows.outcome else {
            return [unavailable(match self.rows.outcome {
                Outcome::Unavailable { reason } => reason,
                _ => "nuclear strike incomplete drain",
            })];
        };
        match project(&self.rows.rows[..len]) {
            Ok(rows) => [TableView::Complete(TableRows::NuclearStrikes(rows))],
            Err(reason) => {
                if record_loss && !self.loss_recorded.swap(true, Ordering::AcqRel) {
                    // SAFETY: Caller is the owning publication path recording loss once.
                    unsafe { producer::lost_output(len) };
                }
                [unavailable(reason)]
            }
        }
    }
    pub fn tables(&self) -> [TableView; N] {
        self.project_tables(false)
    }
    /// # Safety
    /// Caller must be the owning spool publication path converting this frame while observer
    /// storage stays live; conversion failure records `lost_output` at most once per frame.
    pub unsafe fn table_inputs(&self) -> [TableInput; N] {
        self.project_tables(true).map(TableInput::Sampled)
    }
}

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

const UNPAIRED: &[u8] = b"nuclear strike unit after read missing";
const DIFFERS: &[u8] = b"nuclear strike unit read before and after differ";

fn damage(before: &Unit, after: &Unit) -> Option<NuclearUnitDamage> {
    (before.class == after.class).then_some(NuclearUnitDamage {
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

struct Before {
    unit: Unit,
    position: usize,
    paired: bool,
}

struct Assembly {
    row: NuclearStrikeRow,
    call: Option<u32>,
    before: Vec<Before>,
    sorted: bool,
}
impl Assembly {
    fn refuse(
        &mut self,
        part: NuclearStrikePart,
        reason: &[u8],
        count: u32,
    ) -> Result<(), &'static str> {
        match self
            .row
            .refusals
            .iter_mut()
            .find(|refusal| refusal.part == part && refusal.reason == reason)
        {
            Some(refusal) => {
                refusal.count = refusal
                    .count
                    .checked_add(count)
                    .ok_or("nuclear strike refusal count overflow")?;
            }
            None => push(
                &mut self.row.refusals,
                NuclearStrikeRefusal {
                    part,
                    reason: allocation::copy_vec(reason)
                        .map_err(|_| "collector allocation failed")?,
                    count,
                },
            )?,
        }
        Ok(())
    }
    fn enter(&mut self, call: Option<u32>) -> Result<(), &'static str> {
        if self.call == call {
            return Ok(());
        }
        let unpaired = self.before.iter().filter(|before| !before.paired).count();
        if unpaired != 0 {
            let unpaired =
                u32::try_from(unpaired).map_err(|_| "nuclear strike refusal count overflow")?;
            self.refuse(NuclearStrikePart::Units, UNPAIRED, unpaired)?;
        }
        self.before.clear();
        self.sorted = false;
        self.call = call;
        Ok(())
    }
    fn push_before(&mut self, call: u32, unit: Unit) -> Result<(), &'static str> {
        self.enter(Some(call))?;
        let position = self.before.len();
        self.sorted = false;
        push(
            &mut self.before,
            Before {
                unit,
                position,
                paired: false,
            },
        )
    }
    fn pair_after(&mut self, call: u32, unit: Unit) -> Result<(), &'static str> {
        self.enter(Some(call))?;
        if !self.sorted {
            self.before
                .sort_unstable_by_key(|before| (before.unit.handle, before.position));
            self.sorted = true;
        }
        let first = self
            .before
            .partition_point(|before| before.unit.handle < unit.handle);
        let Some(before) = self.before[first..]
            .iter_mut()
            .take_while(|before| before.unit.handle == unit.handle)
            .find(|before| !before.paired)
        else {
            return Ok(());
        };
        before.paired = true;
        let damaged = damage(&before.unit, &unit);
        match damaged {
            Some(damaged) => push(&mut self.row.units, damaged),
            None => self.refuse(NuclearStrikePart::Units, DIFFERS, 1),
        }
    }
    fn finish(mut self) -> Result<NuclearStrikeRow, &'static str> {
        self.enter(None)?;
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
                _ => merged.push(loss),
            }
        }
        merged.retain(|loss| loss.amount_raw != 0);
        self.row.equipment = merged;
        Ok(self.row)
    }
}

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
                call: None,
                before: Vec::new(),
                sorted: false,
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
                call,
                unit,
            } => strike.push_before(call, unit)?,
            Item::Unit {
                phase: Phase::After,
                call,
                unit,
            } => strike.pair_after(call, unit)?,
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
            Item::Refused { part, reason } => strike.refuse(part, reason.as_bytes(), 1)?,
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

    fn before(id: u64, call: u32, unit: Unit) -> Entry {
        entry(
            id,
            Item::Unit {
                phase: Phase::Before,
                call,
                unit,
            },
        )
    }
    fn after(id: u64, call: u32, unit: Unit) -> Entry {
        entry(
            id,
            Item::Unit {
                phase: Phase::After,
                call,
                unit,
            },
        )
    }
    fn refusals(row: &NuclearStrikeRow) -> Vec<(&[u8], u32)> {
        row.refusals
            .iter()
            .map(|refusal| (refusal.reason.as_slice(), refusal.count))
            .collect()
    }
    fn paired(row: &NuclearStrikeRow) -> Vec<(u64, i64, i64)> {
        row.units
            .iter()
            .map(|unit| (unit.handle, unit.strength_before, unit.strength_after))
            .collect()
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
            before(6, 0, unit(1, 900_000)),
            before(7, 0, gun),
            after(8, 0, unit(1, 300_000)),
            after(9, 0, Unit { strength: 0, ..gun }),
            before(10, 1, unit(1, 300_000)),
            after(11, 1, unit(1, 100_000)),
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
    fn a_call_whose_after_walk_failed_leaves_the_next_calls_units_paired() {
        let entries = [
            strike(1),
            before(2, 0, unit(10, 900_000)),
            entry(
                3,
                Item::Refused {
                    part: NuclearStrikePart::Units,
                    reason: "nuclear strike occupants unreadable",
                },
            ),
            before(4, 1, unit(20, 800_000)),
            after(5, 1, unit(20, 200_000)),
        ];
        let row = &project(&entries).unwrap()[0];
        assert_eq!(paired(row), [(20, 800_000, 200_000)]);
        assert_eq!(
            refusals(row),
            [
                (&b"nuclear strike occupants unreadable"[..], 1),
                (UNPAIRED, 1),
            ]
        );
    }

    #[test]
    fn units_pair_by_handle_within_their_call_whichever_read_failed() {
        let entries = [
            strike(1),
            before(2, 0, unit(1, 900)),
            before(3, 0, unit(2, 800)),
            before(4, 0, unit(3, 700)),
            after(5, 0, unit(1, 90)),
            after(6, 0, unit(3, 70)),
            before(7, 1, unit(5, 500)),
            before(8, 1, unit(6, 400)),
            after(9, 1, unit(4, 6)),
            after(10, 1, unit(5, 50)),
            after(11, 1, unit(6, 40)),
            before(12, 2, unit(7, 300)),
            after(
                13,
                2,
                Unit {
                    class: RAILWAY_GUN_CLASS,
                    ..unit(7, 30)
                },
            ),
        ];
        let row = &project(&entries).unwrap()[0];
        assert_eq!(
            paired(row),
            [(1, 900, 90), (3, 700, 70), (5, 500, 50), (6, 400, 40)]
        );
        assert_eq!(refusals(row), [(UNPAIRED, 1), (DIFFERS, 1)]);
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
            before(4, 0, unit(1, 5)),
            before(5, 0, unit(2, 5)),
            before(6, 0, unit(3, 5)),
            after(7, 0, unit(2, 5)),
        ];
        let row = &project(&entries).unwrap()[0];
        assert_eq!(paired(row), [(2, 5, 5)]);
        assert!(!row.complete(NuclearStrikePart::Units));
        assert!(row.complete(NuclearStrikePart::Buildings));
        assert_eq!(
            refusals(row),
            [
                (&b"nuclear strike unit class unsupported"[..], 2),
                (UNPAIRED, 2),
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
