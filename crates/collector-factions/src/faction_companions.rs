use crate::faction_companion_changes::Changes;
use engine_allocation as allocation;
use engine_native::Memory;
use replay_data::{
    rows, rows::RowCodec, rows::TableId, rows::TableRows, rows::TableSchema, rows::tables,
};
use replay_frame::frame::TableInput;
use std::sync::Arc;
const N: usize = 12;
pub const TABLES: [TableId; N] = [
    TableId::FactionRules,
    TableId::FactionPrograms,
    TableId::FactionManpower,
    TableId::FactionUnlocks,
    TableId::FactionAccounts,
    TableId::FactionVariables,
    TableId::FactionResearch,
    TableId::FactionMinimumUpgrades,
    TableId::FactionDoctrine,
    TableId::FactionProjection,
    TableId::FactionResources,
    TableId::FactionAdvisorModifiers,
];
#[derive(Default)]
pub struct Captured {
    rows: [Option<Arc<TableRows>>; N],
    failure: [Option<&'static str>; N],
}
impl Captured {
    pub fn inputs(&self) -> [TableInput; N] {
        std::array::from_fn(|kind| match (&self.rows[kind], self.failure[kind]) {
            (Some(rows), None) => TableInput::SharedComplete(Arc::clone(rows)),
            _ => collector_projection::table_conversion::unavailable(
                self.failure[kind].unwrap_or("faction companion acquisition unavailable"),
            )
            .into(),
        })
    }
    pub fn refuse(&mut self, reason: &'static str) {
        self.failure.fill(Some(reason));
    }
}
trait List: Default {
    type Value: Default;
    fn retained_bytes(&self) -> usize {
        0
    }
    type Schema: TableSchema;
    const KIND: usize;
    const OFFSET: usize;
    fn heap(_value: &Self::Value) -> usize {
        0
    }
    fn read(
        &mut self,
        memory: &impl Memory,
        base: usize,
        faction: usize,
        output: &mut Vec<Self::Value>,
    ) -> Result<(), &'static str>;
    fn assign(
        target: &mut <Self::Schema as TableSchema>::Row,
        faction: u64,
        ordinal: u32,
        value: &Self::Value,
    ) -> Result<(), &'static str> {
        *target = Self::row(faction, ordinal, value)?;
        Ok(())
    }
    fn row(
        faction: u64,
        ordinal: u32,
        value: &Self::Value,
    ) -> Result<<Self::Schema as TableSchema>::Row, &'static str>;
}
fn copy<T: Copy>(target: &mut Vec<T>, source: &[T]) -> Result<(), &'static str> {
    allocation::reserve(target, source.len().saturating_sub(target.len()))
        .map_err(|_| "faction companion nested replay allocation failed")?;
    target.clear();
    target.extend_from_slice(source);
    Ok(())
}
#[derive(Default)]
struct Rules;
impl List for Rules {
    type Value = u32;
    type Schema = tables::FactionRules;
    const KIND: usize = 0;
    const OFFSET: usize = engine_native::cfaction_rule_status::STATUS;
    fn read(
        &mut self,
        memory: &impl Memory,
        base: usize,
        faction: usize,
        output: &mut Vec<u32>,
    ) -> Result<(), &'static str> {
        engine_native::cfaction_rule_status::read_into(memory, base, faction, output)
    }
    fn row(faction: u64, ordinal: u32, token: &u32) -> Result<rows::FactionRuleRow, &'static str> {
        Ok(rows::FactionRuleRow {
            faction,
            ordinal,
            token: *token,
        })
    }
}
#[derive(Default)]
struct Programmes;
impl List for Programmes {
    type Value = u64;
    type Schema = tables::FactionPrograms;
    const KIND: usize = 1;
    const OFFSET: usize = engine_native::cfaction_program_status::STATUS;
    fn read(
        &mut self,
        memory: &impl Memory,
        base: usize,
        faction: usize,
        output: &mut Vec<u64>,
    ) -> Result<(), &'static str> {
        engine_native::cfaction_program_status::read_into(memory, base, faction, output)
    }
    fn row(
        faction: u64,
        ordinal: u32,
        reference: &u64,
    ) -> Result<rows::FactionProgramRow, &'static str> {
        Ok(rows::FactionProgramRow {
            faction,
            ordinal,
            reference: *reference,
        })
    }
}
#[derive(Default)]
struct Manpower;
impl List for Manpower {
    type Value = engine_native::cfaction::ManpowerContribution;
    type Schema = tables::FactionManpower;
    const KIND: usize = 2;
    const OFFSET: usize = 0;
    fn read(
        &mut self,
        memory: &impl Memory,
        base: usize,
        faction: usize,
        output: &mut Vec<Self::Value>,
    ) -> Result<(), &'static str> {
        engine_native::cfaction::manpower_into(memory, base, faction, output)
    }
    fn row(
        faction: u64,
        ordinal: u32,
        value: &Self::Value,
    ) -> Result<rows::FactionManpowerRow, &'static str> {
        Ok(rows::FactionManpowerRow {
            faction,
            ordinal,
            country: value.country,
            value: value.value,
        })
    }
}
#[derive(Default)]
struct Unlocks;
impl List for Unlocks {
    type Value = [u8; 2];
    type Schema = tables::FactionUnlocks;
    const KIND: usize = 3;
    const OFFSET: usize = 0;
    fn read(
        &mut self,
        memory: &impl Memory,
        _base: usize,
        faction: usize,
        output: &mut Vec<Self::Value>,
    ) -> Result<(), &'static str> {
        let address = faction
            .checked_add(engine_native::cfaction::RESEARCH_UNLOCKED)
            .ok_or("faction unlock address overflow")?;
        let value =
            engine_native::read(memory, address).ok_or("faction unlock bytes unreadable")?;
        allocation::resize_default(output, 1).map_err(|_| "faction unlock allocation failed")?;
        output[0] = value;
        Ok(())
    }
    fn row(
        faction: u64,
        _: u32,
        value: &Self::Value,
    ) -> Result<rows::FactionUnlockRow, &'static str> {
        Ok(rows::FactionUnlockRow {
            faction,
            research: value[0],
            manpower: value[1],
        })
    }
}
#[derive(Default)]
struct Variables {
    entries: Vec<engine_scripts::cvariables::Row>,
    names: Vec<u8>,
}
impl List for Variables {
    fn retained_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<engine_scripts::cvariables::Row>()
            + self.names.capacity()
    }
    fn assign(
        target: &mut rows::FactionVariableRow,
        faction: u64,
        ordinal: u32,
        value: &Self::Value,
    ) -> Result<(), &'static str> {
        target.faction = faction;
        target.ordinal = ordinal;
        target.seed = value.seed;
        target.value_raw = value.value_raw;
        copy(&mut target.name, &value.name)
    }
    type Value = rows::FactionVariableRow;
    fn heap(value: &Self::Value) -> usize {
        value.name.capacity()
    }
    type Schema = tables::FactionVariables;
    const KIND: usize = 5;
    const OFFSET: usize = engine_native::cfaction::VARIABLES;
    fn read(
        &mut self,
        memory: &impl Memory,
        base: usize,
        faction: usize,
        output: &mut Vec<Self::Value>,
    ) -> Result<(), &'static str> {
        let store = faction
            .checked_add(Self::OFFSET)
            .ok_or("faction variable store address overflow")?;
        let walk = engine_scripts::cvariables::walk(
            memory,
            base,
            store,
            &mut self.entries,
            &mut self.names,
        );
        let engine_native::walk::Outcome::Rows { len } = walk.outcome else {
            return Err(match walk.outcome {
                engine_native::walk::Outcome::Unavailable { reason } => reason,
                _ => "faction variables incomplete",
            });
        };
        let seed = engine_native::read(memory, store + engine_scripts::cvariables::SEED)
            .ok_or("faction variable seed unreadable")?;
        let count = len
            .checked_add(1)
            .filter(|count| u32::try_from(*count).is_ok())
            .ok_or("faction variable count overflow")?;
        allocation::resize_default(output, count)
            .map_err(|_| "faction variable row allocation failed")?;
        output[0] = rows::FactionVariableRow {
            seed: Some(seed),
            ..Default::default()
        };
        for (ordinal, entry) in self.entries[..len].iter().enumerate() {
            let value = &mut output[ordinal + 1];
            value.name.clear();
            allocation::reserve(&mut value.name, entry.name_len)
                .map_err(|_| "faction variable name allocation failed")?;
            value.name.extend_from_slice(
                &self.names[entry.name_start..entry.name_start + entry.name_len],
            );
            value.value_raw = entry.value_raw;
            value.seed = None;
        }
        Ok(())
    }
    fn row(
        faction: u64,
        ordinal: u32,
        value: &Self::Value,
    ) -> Result<rows::FactionVariableRow, &'static str> {
        Ok(rows::FactionVariableRow {
            faction,
            ordinal,
            seed: value.seed,
            value_raw: value.value_raw,
            name: allocation::copy_vec(&value.name)
                .map_err(|_| "faction variable name allocation failed")?,
        })
    }
}
#[derive(Default)]
struct Research {
    scratch: engine_native::ctechnology_sharing_faction::Snapshot,
}
impl List for Research {
    type Value = rows::FactionResearchRow;
    type Schema = tables::FactionResearch;
    const KIND: usize = 6;
    const OFFSET: usize = engine_stability::cfaction_upgrade_status::STATUS
        + engine_stability::cfaction_upgrade_status::RESEARCH_SHARING;
    fn heap(value: &Self::Value) -> usize {
        value.countries.capacity() * 4
    }
    fn retained_bytes(&self) -> usize {
        self.scratch.countries.capacity() * 4 + self.scratch.bonuses.capacity() * 8
    }
    fn assign(
        target: &mut Self::Value,
        faction: u64,
        ordinal: u32,
        value: &Self::Value,
    ) -> Result<(), &'static str> {
        target.faction = faction;
        target.ordinal = ordinal;
        target.scientist_base_raw = value.scientist_base_raw;
        target.bonus_raw = value.bonus_raw;
        copy(&mut target.countries, &value.countries)
    }
    fn read(
        &mut self,
        memory: &impl Memory,
        base: usize,
        faction: usize,
        output: &mut Vec<Self::Value>,
    ) -> Result<(), &'static str> {
        engine_stability::cfaction_upgrade_status::authenticate(memory, base, faction)?
            .research_into(memory, &mut self.scratch)?;
        allocation::resize_default(output, 1 + self.scratch.bonuses.len())
            .map_err(|_| "faction research occurrence allocation failed")?;
        for (ordinal, target) in output.iter_mut().enumerate() {
            target.ordinal = ordinal as u32;
            target.scientist_base_raw = if ordinal == 0 {
                self.scratch.base_raw
            } else {
                0
            };
            target.bonus_raw = if ordinal == 0 {
                0
            } else {
                self.scratch.bonuses[ordinal - 1]
            };
            if ordinal == 0 {
                copy(&mut target.countries, &self.scratch.countries)?;
            } else {
                target.countries.clear();
            }
        }
        Ok(())
    }
    fn row(faction: u64, ordinal: u32, value: &Self::Value) -> Result<Self::Value, &'static str> {
        Ok(Self::Value {
            faction,
            ordinal,
            scientist_base_raw: value.scientist_base_raw,
            bonus_raw: value.bonus_raw,
            countries: allocation::copy_vec(&value.countries)
                .map_err(|_| "faction research country allocation failed")?,
        })
    }
}
#[derive(Default)]
struct Minimum;
impl List for Minimum {
    type Value = engine_native::cfaction_member_status::Upgrade;
    type Schema = tables::FactionMinimumUpgrades;
    const KIND: usize = 7;
    const OFFSET: usize = engine_stability::cfaction_upgrade_status::STATUS;
    fn read(
        &mut self,
        memory: &impl Memory,
        base: usize,
        faction: usize,
        output: &mut Vec<Self::Value>,
    ) -> Result<(), &'static str> {
        engine_stability::cfaction_upgrade_status::authenticate(memory, base, faction)?
            .member_upgrades_into(memory, output)
    }
    fn row(
        faction: u64,
        ordinal: u32,
        value: &Self::Value,
    ) -> Result<rows::FactionMinimumUpgradeRow, &'static str> {
        Ok(rows::FactionMinimumUpgradeRow {
            faction,
            ordinal,
            group: value.group,
            token: value.token,
        })
    }
}
#[derive(Default)]
struct Doctrine;
impl List for Doctrine {
    type Value = u32;
    type Schema = tables::FactionDoctrine;
    const KIND: usize = 8;
    const OFFSET: usize = engine_stability::cfaction_upgrade_status::STATUS
        + engine_stability::cfaction_upgrade_status::DOCTRINE_SHARING;
    fn read(
        &mut self,
        memory: &impl Memory,
        base: usize,
        faction: usize,
        output: &mut Vec<u32>,
    ) -> Result<(), &'static str> {
        engine_stability::cfaction_upgrade_status::authenticate(memory, base, faction)?
            .doctrine_into(memory, output)
    }
    fn row(
        faction: u64,
        ordinal: u32,
        token: &u32,
    ) -> Result<rows::FactionDoctrineRow, &'static str> {
        Ok(rows::FactionDoctrineRow {
            faction,
            ordinal,
            token: *token,
        })
    }
}
#[derive(Default)]
struct Held<T> {
    id: u64,
    pointer: usize,
    seen: u64,
    certificate: Option<(u64, u64, u64)>,
    failure: Option<&'static str>,
    values: Vec<T>,
}
#[derive(Default)]
struct ListStore<S: List> {
    held: Vec<Held<S::Value>>,
    rows: Option<Arc<TableRows>>,
    spares: Vec<Arc<TableRows>>,
    before: Option<(u64, u64, u64)>,
    quiet: Option<(u64, u64, u64)>,
    quiet_carried: bool,
    failure: Option<&'static str>,
    changed: bool,
    unpublished: bool,
    generation: u64,
    seen: usize,
    force: bool,
    bytes: usize,
    ticks: u64,
    reader: S,
}
impl<S: List> ListStore<S> {
    fn begin(&mut self, changes: &impl Changes, force: bool) {
        self.before = changes.quiet(S::KIND);
        self.failure = self
            .before
            .is_none()
            .then_some("faction companions could not be followed in the game");
        self.force = force;
        self.quiet_carried = false;
        self.changed = force || self.unpublished;
        self.ticks = 0;
        self.seen = 0;
        if let Some(next) = self.generation.checked_add(1) {
            self.generation = next;
        } else {
            self.before = None;
            self.failure = Some("faction acquisition generation exhausted");
        }
    }
    fn visit(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
        id: u64,
        changes: &impl Changes,
    ) {
        if self.before.is_none() {
            return;
        }
        let index = match self.held.binary_search_by_key(&id, |held| held.id) {
            Ok(index) => index,
            Err(index) => {
                if allocation::reserve(&mut self.held, 1).is_err() {
                    self.failure
                        .get_or_insert("faction companion owner allocation failed");
                    return;
                }
                self.held.insert(
                    index,
                    Held {
                        id,
                        pointer,
                        ..Default::default()
                    },
                );
                self.changed = true;
                index
            }
        };
        let held = &mut self.held[index];
        if held.seen != self.generation {
            self.seen += 1;
            held.seen = self.generation;
        }
        let Some(receiver) = pointer.checked_add(S::OFFSET) else {
            self.failure
                .get_or_insert("faction companion receiver overflow");
            return;
        };
        let family_quiet = self.before == self.quiet;
        if !self.force
            && held.pointer == pointer
            && held.certificate.is_some()
            && (family_quiet || held.certificate == changes.sequence(S::KIND, receiver))
        {
            self.quiet_carried |= family_quiet;
            if let Some(reason) = held.failure {
                self.failure.get_or_insert(reason);
            }
            return;
        }
        held.pointer = pointer;
        held.certificate = None;
        held.failure = None;
        self.changed = true;
        let before = changes.sequence(S::KIND, receiver);
        if before.is_none() {
            self.failure
                .get_or_insert("faction companion registration unavailable");
            return;
        }
        let _reads = collector_memory::read_accounting::table(TABLES[S::KIND]);
        let started = collector_primitives::clock::qpc();
        let result = self.reader.read(memory, base, pointer, &mut held.values);
        self.ticks += collector_primitives::clock::qpc()
            .saturating_sub(started)
            .max(0) as u64;
        if before != changes.sequence(S::KIND, receiver) {
            self.failure
                .get_or_insert("faction companion writer changed during acquisition");
            return;
        }
        held.certificate = before;
        if let Err(reason) = result {
            held.failure = Some(reason);
            self.failure.get_or_insert(reason);
        }
    }
    fn finish(&mut self, complete: bool, changes: &impl Changes, output: &mut Captured) {
        self.unpublished |= self.changed;
        let started = self.changed.then(collector_primitives::clock::qpc);
        if !complete {
            self.failure
                .get_or_insert("faction companion identity roster incomplete");
        }
        if self.before.is_none() || (self.quiet_carried && self.before != changes.quiet(S::KIND)) {
            self.failure
                .get_or_insert("faction companion observation changed during acquisition");
        }
        if self.failure.is_none() {
            if self.seen != self.held.len() {
                self.changed = true;
                self.held.retain(|held| held.seen == self.generation);
            }
            self.unpublished |= self.changed;
            if (self.unpublished || self.rows.is_none())
                && let Err(reason) = self.rebuild()
            {
                self.failure = Some(reason);
            }
        }
        if self.failure.is_none() {
            self.unpublished = false;
        }
        if self.changed {
            self.bytes = self.reader.retained_bytes()
                + self.held.capacity() * size_of::<Held<S::Value>>()
                + self
                    .held
                    .iter()
                    .map(|held| {
                        held.values.capacity() * size_of::<S::Value>()
                            + held.values.iter().map(S::heap).sum::<usize>()
                    })
                    .sum::<usize>()
                + self
                    .rows
                    .iter()
                    .chain(&self.spares)
                    .map(|rows| size_of::<TableRows>() + rows.allocated_bytes())
                    .sum::<usize>()
                + self.spares.capacity() * size_of::<Arc<TableRows>>();
        }
        self.quiet = self.failure.is_none().then_some(self.before).flatten();
        output.rows[S::KIND] = self
            .failure
            .is_none()
            .then(|| self.rows.as_ref().map(Arc::clone))
            .flatten();
        output.failure[S::KIND] = self.failure;
        if let Some(started) = started {
            self.ticks += collector_primitives::clock::qpc()
                .saturating_sub(started)
                .max(0) as u64;
        }
    }
    fn rebuild(&mut self) -> Result<(), &'static str> {
        let mut table = crate::faction_goals::writable(&mut self.rows, &mut self.spares, || {
            TableRows::empty(S::Schema::ID)
        })?;
        let rows = Arc::get_mut(&mut table)
            .unwrap()
            .rows_mut::<S::Schema>()
            .unwrap();
        let count = self
            .held
            .iter()
            .try_fold(0usize, |count, held| count.checked_add(held.values.len()))
            .ok_or("faction companion row count overflow")?;
        allocation::resize_default(rows, count)
            .map_err(|_| "faction companion row allocation failed")?;
        let mut next = 0;
        for held in &self.held {
            for (ordinal, value) in held.values.iter().enumerate() {
                S::assign(&mut rows[next], held.id, ordinal as u32, value)?;
                next += 1;
            }
        }
        rows.sort_unstable_by_key(RowCodec::key);
        self.rows = Some(table);
        Ok(())
    }
}
#[derive(Default)]
pub struct Store {
    rules: ListStore<Rules>,
    programmes: ListStore<Programmes>,
    manpower: ListStore<Manpower>,
    unlocks: ListStore<Unlocks>,
    variables: ListStore<Variables>,
    research: ListStore<Research>,
    minimum: ListStore<Minimum>,
    doctrine: ListStore<Doctrine>,
    accounts: crate::faction_accounts::Store,
    advisors: crate::faction_advisors::Store,
    pools: crate::faction_pools::Store,
    accounts_force: bool,
    invalidated: bool,
}
impl Store {
    pub fn begin(&mut self, changes: &impl Changes, force: bool) {
        let force = force || std::mem::take(&mut self.invalidated);
        self.accounts_force = force;
        self.pools
            .begin(force, changes.quiet(9).map(|value| value.0));
        self.advisors.begin(changes, force);
        self.rules.begin(changes, force);
        self.programmes.begin(changes, force);
        self.manpower.begin(changes, force);
        self.unlocks.begin(changes, force);
        self.variables.begin(changes, force);
        self.research.begin(changes, force);
        self.minimum.begin(changes, force);
        self.doctrine.begin(changes, force);
    }
    pub fn visit(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
        id: u64,
        changes: &impl Changes,
    ) {
        self.advisors.visit(memory, base, pointer, id, changes);
        self.rules.visit(memory, base, pointer, id, changes);
        self.programmes.visit(memory, base, pointer, id, changes);
        self.manpower.visit(memory, base, pointer, id, changes);
        self.unlocks.visit(memory, base, pointer, id, changes);
        self.variables.visit(memory, base, pointer, id, changes);
        self.research.visit(memory, base, pointer, id, changes);
        self.minimum.visit(memory, base, pointer, id, changes);
        self.doctrine.visit(memory, base, pointer, id, changes);
        self.pools.visit(memory, base, pointer, id);
    }
    /// # Safety
    /// Caller holds hourly capture admission for the active session (or isolated fixture control)
    /// so global `faction_companion_changes` account demand updates belong to this pass.
    pub unsafe fn acquire_accounts(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        changes: &impl Changes,
        members: &[engine_native::cfaction_system::MemberRow],
    ) {
        // SAFETY: Caller upholds the hourly capture admission contract.
        unsafe {
            self.accounts.acquire_members(
                memory,
                base,
                state,
                self.accounts_force,
                changes,
                members,
            );
        }
        self.pools.system(memory, base, state);
    }
    /// # Safety
    /// Caller holds hourly capture admission for the active session (or isolated fixture control)
    /// so global `faction_companion_changes` demand updates belong to this pass.
    pub unsafe fn finish(&mut self, complete: bool, changes: &impl Changes, output: &mut Captured) {
        // SAFETY: Admitted faction-companion capture records measured owner demand.
        unsafe {
            crate::faction_companion_changes::demand(
                self.minimum
                    .held
                    .len()
                    .max(self.doctrine.held.len())
                    .max(self.rules.held.len())
                    .max(self.programmes.held.len())
                    .max(self.manpower.held.len())
                    .max(self.unlocks.held.len()),
            );
        }
        // SAFETY: Caller holds hourly capture admission for this faction-advisor finish pass.
        unsafe { self.advisors.finish(complete, changes) };
        (output.rows[11], output.failure[11]) = self.advisors.output();
        self.rules.finish(complete, changes, output);
        self.programmes.finish(complete, changes, output);
        self.manpower.finish(complete, changes, output);
        self.unlocks.finish(complete, changes, output);
        collector_intel::variable_changes::Observer.demand_embedded(self.variables.held.len());
        self.variables.finish(complete, changes, output);
        collector_observers::country_input_changes::sharing_demand(self.research.held.len());
        self.research.finish(complete, changes, output);
        self.minimum.finish(complete, changes, output);
        self.doctrine.finish(complete, changes, output);
        let (rows, failure) = self.accounts.output();
        self.pools.finish(
            complete,
            changes.quiet(9).map(|value| value.0),
            Some(&self.accounts),
        );
        (output.rows[9], output.failure[9]) = self.pools.output(0);
        (output.rows[10], output.failure[10]) = self.pools.output(1);
        if rows.is_some() || failure.is_some() {
            (output.rows[4], output.failure[4]) = (rows, failure);
        }
    }
    pub fn invalidate(&mut self) {
        self.invalidated = true;
    }
    pub fn ticks(&self) -> u64 {
        self.rules.ticks
            + self.programmes.ticks
            + self.manpower.ticks
            + self.unlocks.ticks
            + self.variables.ticks
            + self.research.ticks
            + self.minimum.ticks
            + self.doctrine.ticks
            + self.pools.ticks()
    }
    pub fn allocated_bytes(&self) -> usize {
        self.advisors.allocated_bytes()
            + self.rules.bytes
            + self.programmes.bytes
            + self.manpower.bytes
            + self.unlocks.bytes
            + self.variables.bytes
            + self.research.bytes
            + self.minimum.bytes
            + self.doctrine.bytes
            + self.accounts.allocated_bytes()
            + self.pools.allocated_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use collector_certificates::watch_registry::WatchRegistry;
    use engine_native::memory::SliceMemory;
    use std::cell::Cell;
    const FIRST: usize = 0x1000;
    const STRIDE: usize = 0x1000;
    struct TestChanges([WatchRegistry; N]);
    impl Changes for TestChanges {
        fn take_accounts(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
            self.0[4].take(visit)
        }
        fn quiet(&self, family: usize) -> Option<(u64, u64, u64)> {
            self.0[family].quiet().map(|(e, s)| (e, s, 0))
        }
        fn sequence(&self, family: usize, receiver: usize) -> Option<(u64, u64, u64)> {
            self.0[family].sequence(receiver).map(|(e, s)| (e, s, 0))
        }
    }
    struct Counted<'a> {
        bytes: &'a [u8],
        reads: Cell<usize>,
    }
    impl Memory for Counted<'_> {
        fn copy(&self, address: usize, out: &mut [u8]) -> bool {
            self.reads.set(self.reads.get() + 1);
            SliceMemory {
                base: 0,
                bytes: self.bytes,
            }
            .copy(address, out)
        }
    }
    fn fixture(owners: usize) -> Vec<u8> {
        let mut bytes = vec![0; FIRST + STRIDE * owners];
        for index in 0..owners {
            let faction = FIRST + STRIDE * index;
            for (offset, value) in [
                (
                    engine_native::cfaction::EXTRACTED_RESOURCES,
                    engine_resources::cstrategic_resource_pool::VTABLE_RVA.0,
                ),
                (
                    engine_stability::cfaction_upgrade_status::STATUS
                        + engine_stability::cfaction_upgrade_status::DOCTRINE_SHARING,
                    engine_native::cdoctrine_sharing_status::VTABLE_RVA.0,
                ),
                (
                    engine_stability::cfaction_upgrade_status::STATUS,
                    engine_stability::cfaction_upgrade_status::VTABLE_RVA.0,
                ),
                (
                    engine_stability::cfaction_upgrade_status::STATUS
                        + engine_stability::cfaction_upgrade_status::OWNER,
                    faction,
                ),
                (
                    engine_stability::cfaction_upgrade_status::STATUS
                        + engine_stability::cfaction_upgrade_status::RESEARCH_SHARING,
                    engine_native::ctechnology_sharing_faction::VTABLE_RVA.0,
                ),
                (
                    engine_stability::cfaction_upgrade_status::STATUS
                        + engine_stability::cfaction_upgrade_status::RESEARCH_SHARING
                        + engine_native::ctechnology_sharing_faction::OWNER,
                    faction,
                ),
                (
                    engine_native::cfaction::VARIABLES,
                    engine_scripts::cvariables::VTABLE_RVA.0,
                ),
                (
                    engine_native::cfaction::VARIABLES + engine_scripts::cvariables::STORAGE,
                    engine_scripts::cvariables::EMPTY_STORAGE_RVA.0,
                ),
                (
                    engine_native::cfaction_rule_status::STATUS,
                    engine_native::cfaction_rule_status::VTABLE_RVA.0,
                ),
                (
                    engine_native::cfaction_rule_status::STATUS
                        + engine_native::cfaction_rule_status::OWNER,
                    faction,
                ),
                (
                    engine_native::cfaction_program_status::STATUS,
                    engine_native::cfaction_program_status::VTABLE_RVA.0,
                ),
                (
                    engine_native::cfaction::MANPOWER_POOL,
                    engine_native::carmy_manpower_values::VTABLE_RVA.0,
                ),
            ] {
                bytes[faction + offset..faction + offset + 8]
                    .copy_from_slice(&(value as u64).to_le_bytes());
            }
        }
        bytes
    }
    fn changes(owners: usize) -> TestChanges {
        TestChanges(std::array::from_fn(|_| {
            let watches = WatchRegistry::retaining();
            watches.offer(owners);
            watches.boundary(false);
            watches.set_enabled(true);
            watches
        }))
    }
    fn acquire(
        store: &mut Store,
        output: &mut Captured,
        memory: &impl Memory,
        changes: &impl Changes,
        owners: usize,
    ) {
        store.begin(changes, false);
        for index in 0..owners {
            store.visit(memory, 0, FIRST + index * STRIDE, index as u64 + 1, changes);
        }
        // SAFETY: Unit test runs against an isolated fixture.
        unsafe { store.finish(true, changes, output) };
    }
    #[test]
    fn faction_companion_certificates_carry_owners_without_reads_or_allocations_and_select_one_list()
     {
        let _lock = crate::faction_companion_changes::TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let bytes = fixture(40);
        let memory = Counted {
            bytes: &bytes,
            reads: Cell::new(0),
        };
        let changes = changes(40);
        let mut store = Store::default();
        let mut output = Captured::default();
        acquire(&mut store, &mut output, &memory, &changes, 40);
        assert_eq!(output.failure, [None; N]);
        let first = memory.reads.get();
        let manpower = Arc::clone(output.rows[2].as_ref().unwrap());
        collector_test_support::with_allocations_denied(|| {
            acquire(&mut store, &mut output, &memory, &changes, 40)
        });
        assert_eq!(memory.reads.get(), first);
        drop(
            changes.0[0].notify(FIRST + 39 * STRIDE + engine_native::cfaction_rule_status::STATUS),
        );
        acquire(&mut store, &mut output, &memory, &changes, 40);
        assert_eq!(output.failure, [None; N]);
        assert!(memory.reads.get() > first && memory.reads.get() - first < 8);
        assert!(Arc::ptr_eq(&manpower, output.rows[2].as_ref().unwrap()));
    }
    #[test]
    fn faction_companion_stable_rule_failure_reads_no_siblings_again_and_keeps_their_complete_output()
     {
        let _lock = crate::faction_companion_changes::TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut bytes = fixture(1);
        bytes[FIRST + engine_native::cfaction_rule_status::STATUS] ^= 1;
        let memory = Counted {
            bytes: &bytes,
            reads: Cell::new(0),
        };
        let changes = changes(1);
        let mut store = Store::default();
        let mut output = Captured::default();
        acquire(&mut store, &mut output, &memory, &changes, 1);
        assert!(output.failure[0].is_some());
        assert!(output.rows[1].is_some() && output.rows[2].is_some());
        let first = memory.reads.get();
        collector_test_support::with_allocations_denied(|| {
            acquire(&mut store, &mut output, &memory, &changes, 1)
        });
        assert_eq!(memory.reads.get(), first);
    }
    #[test]
    fn faction_variable_seed_changes_select_only_the_notified_embedded_store() {
        let _lock = crate::faction_companion_changes::TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut bytes = fixture(2);
        let seed =
            FIRST + STRIDE + engine_native::cfaction::VARIABLES + engine_scripts::cvariables::SEED;
        bytes[seed..seed + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        let memory = Counted {
            bytes: &bytes,
            reads: Cell::new(0),
        };
        let changes = changes(2);
        let mut store = Store::default();
        let mut output = Captured::default();
        acquire(&mut store, &mut output, &memory, &changes, 2);
        let first = memory.reads.get();
        let unlocks = Arc::clone(output.rows[3].as_ref().unwrap());
        drop(changes.0[5].notify(FIRST + STRIDE + engine_native::cfaction::VARIABLES));
        acquire(&mut store, &mut output, &memory, &changes, 2);
        assert!(memory.reads.get() > first && memory.reads.get() - first < 8);
        assert!(Arc::ptr_eq(&unlocks, output.rows[3].as_ref().unwrap()));
        let rows = output.rows[5]
            .as_ref()
            .unwrap()
            .rows::<tables::FactionVariables>()
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].seed, Some(u32::MAX));
        assert!(rows[1].name.is_empty());
    }
    #[test]
    fn faction_stores_read_under_their_own_certificates_survive_other_writers_during_the_walk() {
        let _lock = crate::faction_companion_changes::TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let bytes = fixture(2);
        let memory = Counted {
            bytes: &bytes,
            reads: Cell::new(0),
        };
        let changes = changes(2);
        let unrelated = 0x10_0000;
        let mut store = Store::default();
        let mut output = Captured::default();
        store.begin(&changes, true);
        store.visit(&memory, 0, FIRST, 1, &changes);
        drop(changes.0[5].notify(unrelated));
        store.visit(&memory, 0, FIRST + STRIDE, 2, &changes);
        // SAFETY: Unit test runs against an isolated fixture.
        unsafe { store.finish(true, &changes, &mut output) };
        assert_eq!(output.failure[5], None);
        let rows = output.rows[5].as_ref().unwrap();
        assert_eq!(rows.rows::<tables::FactionVariables>().unwrap().len(), 2);

        acquire(&mut store, &mut output, &memory, &changes, 2);
        assert_eq!(output.failure[5], None);
        let reads = memory.reads.get();
        store.begin(&changes, false);
        store.visit(&memory, 0, FIRST, 1, &changes);
        store.visit(&memory, 0, FIRST + STRIDE, 2, &changes);
        assert_eq!(memory.reads.get(), reads, "a quiet family carries both stores");
        drop(changes.0[5].notify(unrelated));
        // SAFETY: Unit test runs against an isolated fixture.
        unsafe { store.finish(true, &changes, &mut output) };
        assert_eq!(
            output.failure[5],
            Some("faction companion observation changed during acquisition")
        );
    }

    #[test]
    fn companion_lists_retain_unpublished_changes_across_a_refused_hour() {
        let _lock = crate::faction_companion_changes::TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let changes = changes(1);
        let bytes = fixture(1);
        let memory = Counted {
            bytes: &bytes,
            reads: Cell::new(0),
        };
        let mut store = Store::default();
        let mut output = Captured::default();
        acquire(&mut store, &mut output, &memory, &changes, 1);
        store.rules.begin(&changes, false);
        store.rules.held[0].values.push(991);
        store.rules.changed = true;
        store.rules.finish(false, &changes, &mut output);
        assert!(output.failure[0].is_some());
        let before = memory.reads.get();
        acquire(&mut store, &mut output, &memory, &changes, 1);
        assert_eq!(memory.reads.get(), before);
        assert_eq!(
            output.rows[0]
                .as_ref()
                .unwrap()
                .rows::<tables::FactionRules>()
                .unwrap()[0]
                .token,
            991
        );
    }

    #[test]
    fn faction_companion_unlock_writer_reads_only_two_bytes_and_preserves_nonboolean_values() {
        let _lock = crate::faction_companion_changes::TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut bytes = fixture(2);
        bytes[FIRST + engine_native::cfaction::RESEARCH_UNLOCKED] = 2;
        bytes[FIRST + engine_native::cfaction::MANPOWER_UNLOCKED] = 255;
        let memory = Counted {
            bytes: &bytes,
            reads: Cell::new(0),
        };
        let changes = changes(2);
        let mut store = Store::default();
        let mut output = Captured::default();
        acquire(&mut store, &mut output, &memory, &changes, 2);
        let rows = output.rows[3]
            .as_ref()
            .unwrap()
            .rows::<tables::FactionUnlocks>()
            .unwrap();
        assert_eq!((rows[0].research, rows[0].manpower), (2, 255));
        let before = memory.reads.get();
        drop(changes.0[3].notify(FIRST));
        acquire(&mut store, &mut output, &memory, &changes, 2);
        assert_eq!(memory.reads.get() - before, 1);
    }
}

#[cfg(test)]
mod nested_buffer_tests {
    use super::*;
    #[test]
    fn faction_changed_variable_and_research_rows_reuse_nested_storage() {
        let mut variables = ListStore::<Variables>::default();
        variables.held.push(Held {
            id: 7,
            values: vec![
                rows::FactionVariableRow {
                    seed: Some(1),
                    ..Default::default()
                },
                rows::FactionVariableRow {
                    name: b"mod_literal".to_vec(),
                    value_raw: 3,
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        variables.rebuild().unwrap();
        variables.rebuild().unwrap();
        let mut research = ListStore::<Research>::default();
        research.held.push(Held {
            id: 7,
            values: vec![
                rows::FactionResearchRow {
                    countries: vec![12, 19],
                    ..Default::default()
                },
                rows::FactionResearchRow {
                    bonus_raw: 3,
                    ..Default::default()
                },
                rows::FactionResearchRow {
                    bonus_raw: -5,
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        research.rebuild().unwrap();
        research.rebuild().unwrap();
        variables.held[0].values[1].value_raw = 9;
        research.held[0].values[2].bonus_raw = 11;
        collector_test_support::with_allocations_denied(|| {
            variables.rebuild().unwrap();
            research.rebuild().unwrap();
        });
        assert_eq!(
            variables
                .rows
                .as_ref()
                .unwrap()
                .rows::<tables::FactionVariables>()
                .unwrap()[1]
                .value_raw,
            9
        );
        assert_eq!(
            research
                .rows
                .as_ref()
                .unwrap()
                .rows::<tables::FactionResearch>()
                .unwrap()[2]
                .bonus_raw,
            11
        );
    }
}
