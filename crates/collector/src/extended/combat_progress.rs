//! Hourly combat-icon operands, sharing the existing land-combat census.
use crate::{
    spooling::table_iter,
    table_capture::{Arena, CaptureRows, grow_vec},
};
use engine::{Memory, cland_combat, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{LandCombatProgressRow, LandEquipmentLoss, TableId, tables},
};
use std::mem::size_of;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::LandCombatProgress];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[(
    "land combat progress and casualty operands (census charged to LandCombats)",
    0,
    &[0],
)];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    variants: usize,
}
impl Capacities {
    /// Attach/spool thread only: live headers size all side pools, with no cap.
    pub(crate) fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
    ) -> Result<(), &'static str> {
        let state = base
            .checked_add(engine::cgame_state::SINGLETON_RVA)
            .and_then(|address| engine::read::<u64>(memory, address))
            .and_then(|pointer| usize::try_from(pointer).ok())
            .filter(|pointer| *pointer != 0)
            .ok_or("land casualty sizing state unreadable")?;
        let (mut rows, mut variants) = (0usize, 0usize);
        cland_combat::visit_sides(memory, base, state, |sides| {
            let plans = engine::cstats_observer::LossPlan::read_sides(memory, base, sides)?;
            rows = rows.checked_add(1).ok_or("land loss row demand overflow")?;
            for plan in plans {
                variants = variants
                    .checked_add(plan.equipment.count as usize)
                    .ok_or("land loss arena demand overflow")?;
            }
            Ok(())
        })?;
        crate::table_capture::grow_capacity(&mut self.rows, rows);
        crate::table_capture::grow_capacity(&mut self.variants, variants);
        Ok(())
    }
}

pub struct Captured {
    rows: CaptureRows<cland_combat::ProgressRow>,
    losses: Vec<[engine::cstats_observer::LossSpan; 2]>,
    equipment: Arena<engine::cstats_observer::LossEntry>,
    pointers: Vec<usize>,
    identities: Vec<[u8; 16]>,
    len: usize,
    missing: u32,
    failure: Option<&'static str>,
}

impl Captured {
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            losses: engine::allocation::owned_vec(engine::allocation::default_vec(capacity.rows)),
            equipment: Arena::new(capacity.variants),
            pointers: engine::allocation::owned_vec(engine::allocation::default_vec(
                capacity.variants,
            )),
            identities: engine::allocation::owned_vec(engine::allocation::default_vec(
                capacity.variants,
            )),
            len: 0,
            missing: 0,
            failure: None,
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.equipment.reset();
        self.len = 0;
        self.missing = 0;
        self.failure = None;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.equipment.allocated_bytes()
            + self.losses.capacity() * size_of::<[engine::cstats_observer::LossSpan; 2]>()
            + self.pointers.capacity() * size_of::<usize>()
            + self.identities.capacity() * 16
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.rows);
        grow_vec(&mut self.losses, capacity.rows);
        capacity.variants = capacity.variants.max(self.equipment.first.len());
        self.equipment.grow(&mut capacity.variants);
        grow_vec(&mut self.pointers, capacity.variants);
        grow_vec(&mut self.identities, capacity.variants);
    }
    pub fn begin(&mut self) {
        self.equipment.reset();
        self.len = 0;
        self.missing = 0;
        self.failure = None;
    }
    pub fn visit(&mut self, memory: &impl Memory, base: usize, source: cland_combat::Source) {
        let _reads = crate::read_accounting::table(TABLES[0]);
        if self.failure.is_some() {
            return;
        }
        let [attacker, defender] =
            match engine::cstats_observer::LossPlan::read_sides(memory, base, source.sides) {
                Ok(plans) => plans,
                Err(reason) => {
                    self.failure = Some(reason);
                    return;
                }
            };
        let count = attacker.equipment.count as usize + defender.equipment.count as usize;
        let Some(required) = self.equipment.required.checked_add(count) else {
            self.failure = Some("land loss arena demand overflow");
            return;
        };
        self.equipment.required = required;
        // Keep complete demand even when storage is short. Grow only on recycle.
        if count
            > self
                .equipment
                .first
                .len()
                .saturating_sub(self.equipment.len)
            || count > self.pointers.len().min(self.identities.len())
            || self.len >= self.rows.rows.len().min(self.losses.len())
        {
            self.missing = self.missing.saturating_add(1);
            return;
        }
        let start = self.equipment.len;
        if let Err(reason) = engine::cstats_observer::LossPlan::copy_sides(
            [attacker, defender],
            memory,
            base,
            &mut self.equipment.first[start..start + count],
            &mut self.pointers[..count],
            &mut self.identities[..count],
        ) {
            self.failure = Some(reason);
            return;
        }
        self.losses[self.len] = [
            engine::cstats_observer::LossSpan {
                manpower: attacker.manpower,
                start,
                len: attacker.equipment.count as usize,
            },
            engine::cstats_observer::LossSpan {
                manpower: defender.manpower,
                start: start + attacker.equipment.count as usize,
                len: defender.equipment.count as usize,
            },
        ];
        self.equipment.len += count;
        match cland_combat::read_progress(memory, base, source) {
            Ok(row) => {
                self.rows.rows[self.len] = row;
                self.len += 1;
            }
            Err(reason) => self.failure = Some(reason),
        }
    }
    pub fn finish(&mut self, parent: Outcome) {
        let missing = self.missing.saturating_add(match parent {
            Outcome::Truncated { unread_rows, .. } => unread_rows,
            _ => 0,
        });
        self.rows.outcome = match (self.failure, parent) {
            (Some(reason), _) | (_, Outcome::Unavailable { reason }) => {
                Outcome::Unavailable { reason }
            }
            _ if self.len.saturating_add(self.missing as usize) != parent.len() => {
                Outcome::Unavailable {
                    reason: "combat progress visits differ from parent census",
                }
            }
            _ if missing != 0 => Outcome::Truncated {
                len: self.len,
                unread_rows: missing,
            },
            _ => Outcome::Rows { len: self.len },
        };
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }
    pub(crate) fn lost_reads(&self) -> u64 {
        match self.rows.outcome {
            Outcome::Rows { .. } => 0,
            Outcome::Truncated { unread_rows, .. } => u64::from(unread_rows),
            Outcome::Unavailable { .. } => u64::from(self.missing.max(1)),
        }
    }
    #[cfg(test)]
    pub fn losses_complete(&self) -> bool {
        matches!(self.rows.outcome, Outcome::Rows { .. })
    }
    pub fn tables(&self) -> [TableView; N] {
        [table_iter::<tables::LandCombatProgress, _>(
            self.outcomes()[0],
            self.rows
                .rows
                .iter()
                .zip(&self.losses)
                .take(self.rows.outcome.len()),
            |(r, losses)| LandCombatProgressRow {
                combat: r.id,
                attacker_progress: r.attacker_progress,
                defender_progress: r.defender_progress,
                direction_attacker: r.direction_attacker,
                direction_province: r.direction_province,
                attacker_manpower_loss: losses[0].manpower.into(),
                defender_manpower_loss: losses[1].manpower.into(),
                attacker_equipment_losses: self.equipment_rows(losses[0]).into(),
                defender_equipment_losses: self.equipment_rows(losses[1]).into(),
            },
        )]
    }
    fn equipment_rows(&self, span: engine::cstats_observer::LossSpan) -> Vec<LandEquipmentLoss> {
        engine::allocation::owned_vec(engine::allocation::collect_vec(
            self.equipment.first[span.start..span.start + span.len]
                .iter()
                .map(|entry| LandEquipmentLoss {
                    variant: entry.variant,
                    amount_raw: entry.amount_raw,
                }),
        ))
    }
}

#[cfg(test)]
#[path = "combat_progress/tests.rs"]
pub(crate) mod tests;
