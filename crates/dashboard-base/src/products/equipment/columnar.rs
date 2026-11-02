//! Variant names read a segment at a time from a columnar walk ([`replay::segments`]): at each
//! complete variant read, the raw operands of every definition a field it is named by changed
//! since the read before, as [`super::read_raw`] reads them from a row frame.

use super::{RawVariant, Tags, VariantHour, read_raw};
use dashboard_world::{Naming as Names, Roster};
use replay::{columns, segments};
use replay_data::rows::{
    CountryAppearanceRow, CountryRow, LexicalTokenRow, TABLE_COUNT, TableId, VariantDefinitionRow,
    VariantModuleRow, VariantUpgradeRow,
};
use std::{
    collections::{HashMap, HashSet},
    io,
    mem::size_of,
};

/// Slots of a walked table grouped by a key, and what an hour changed of a table.
pub mod slots {
    use super::*;
    use std::hash::Hash;

    struct Group {
        slots: Vec<u32>,
        ordered: bool,
    }

    /// Each live slot's key, and each key's slots.
    pub struct Index<K> {
        positions: Vec<Option<(K, usize)>>,
        groups: HashMap<K, Group>,
    }

    impl<K> Default for Index<K> {
        fn default() -> Self {
            Self {
                positions: Vec::new(),
                groups: HashMap::new(),
            }
        }
    }

    impl<K: Copy + Eq + Hash> Index<K> {
        pub fn clear(&mut self) {
            self.positions.clear();
            self.groups.clear();
        }

        /// The bytes it holds on the heap, about.
        pub fn heap(&self) -> usize {
            self.positions.capacity() * size_of::<Option<(K, usize)>>()
                + self.groups.capacity() * size_of::<(K, Group)>()
                + self
                    .groups
                    .values()
                    .map(|group| group.slots.capacity() * size_of::<u32>())
                    .sum::<usize>()
        }

        pub fn remove(&mut self, slot: u32) {
            let Some(position) = self.positions.get_mut(slot as usize).and_then(Option::take)
            else {
                return;
            };
            let (key, at) = position;
            let group = self.groups.get_mut(&key).expect("indexed slot group");
            group.slots.swap_remove(at);
            group.ordered = false;
            if let Some(&moved) = group.slots.get(at) {
                self.positions[moved as usize] = Some((key, at));
            }
            if group.slots.is_empty() {
                self.groups.remove(&key);
            }
        }

        pub fn insert(&mut self, slot: u32, key: K) {
            if self
                .positions
                .get(slot as usize)
                .copied()
                .flatten()
                .is_some_and(|old| old.0 == key)
            {
                return;
            }
            self.remove(slot);
            self.positions
                .resize(self.positions.len().max(slot as usize + 1), None);
            let group = self.groups.entry(key).or_insert_with(|| Group {
                slots: Vec::new(),
                ordered: false,
            });
            self.positions[slot as usize] = Some((key, group.slots.len()));
            group.slots.push(slot);
            group.ordered = false;
        }

        pub fn active(&self, slot: u32) -> bool {
            self.positions
                .get(slot as usize)
                .is_some_and(Option::is_some)
        }

        pub fn contains(&self, key: K) -> bool {
            self.groups.contains_key(&key)
        }

        pub fn get(&self, key: K) -> &[u32] {
            self.groups
                .get(&key)
                .map_or(&[], |group| group.slots.as_slice())
        }

        pub fn take(&mut self, key: K) -> Vec<u32> {
            let Some(group) = self.groups.remove(&key) else {
                return Vec::new();
            };
            for &slot in &group.slots {
                self.positions[slot as usize] = None;
            }
            group.slots
        }

        /// `key`'s slots in their rows' key order.
        pub fn ordered(
            &mut self,
            state: &segments::State,
            table: TableId,
            key: K,
        ) -> io::Result<&[u32]> {
            let Some(group) = self.groups.get_mut(&key) else {
                return Ok(&[]);
            };
            if !group.ordered {
                let held = columns::table(state, table)?;
                group.slots.sort_unstable_by_key(|&slot| held.key(slot));
                for (at, &slot) in group.slots.iter().enumerate() {
                    self.positions[slot as usize] = Some((key, at));
                }
                group.ordered = true;
            }
            Ok(&group.slots)
        }
    }

    /// Keeps `index` to `table`'s live rows: rebuilt whole at `reset` or a partial read, else
    /// by the hour's inserted and deleted rows. A truncated table counts only when `partial`.
    pub fn structural<K: Copy + Eq + Hash>(
        index: &mut Index<K>,
        state: &segments::State,
        table: TableId,
        reset: bool,
        partial: bool,
        key: impl Fn(u128) -> K,
    ) -> io::Result<()> {
        let readable = columns::complete(state, table)
            || (partial && columns::truncated(state, table).is_some());
        let changes = columns::changes(state, table)?;
        if reset || changes.truncated {
            index.clear();
            if readable {
                let held = columns::table(state, table)?;
                for slot in live(state, table)? {
                    index.insert(slot, key(held.key(slot).expect("live indexed row")));
                }
            }
        } else if readable {
            for &slot in &changes.deleted {
                index.remove(slot);
            }
            let held = columns::table(state, table)?;
            for &slot in &changes.inserted {
                index.insert(slot, key(held.key(slot).expect("inserted indexed row")));
            }
        }
        Ok(())
    }

    /// `table`'s live slots: those a partial read read, else every live one.
    pub fn live(state: &segments::State, table: TableId) -> io::Result<Vec<u32>> {
        Ok(match columns::truncated(state, table) {
            Some(slots) => slots.to_vec(),
            None => columns::table(state, table)?.live().ones().collect(),
        })
    }

    /// The slots the hour wrote one of `mask`'s fields at, or inserted, ascending.
    pub fn edits(state: &segments::State, table: TableId, mask: u64) -> io::Result<Vec<u32>> {
        let changes = columns::changes(state, table)?;
        let mut slots: Vec<_> = changes
            .changed
            .iter()
            .zip(&changes.masks)
            .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
            .filter_map(|(&slot, &written)| (written & mask != 0).then_some(slot))
            .chain(changes.inserted.iter().copied())
            .collect();
        slots.sort_unstable();
        slots.dedup();
        Ok(slots)
    }

    /// Text `field`'s bytes at `slot`.
    pub fn text(
        state: &segments::State,
        table: TableId,
        field: usize,
        slot: u32,
    ) -> io::Result<Vec<u8>> {
        let held = columns::table(state, table)?;
        let id = columns::text(held, field)?[slot as usize];
        state
            .dictionary()
            .get(id)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| io::Error::other(format!("{table:?} text has no dictionary entry")))
    }

    pub fn mask(fields: &[usize]) -> u64 {
        fields.iter().fold(0, |mask, &field| mask | 1_u64 << field)
    }

    /// Whether the hour inserted or deleted a row of `table`, wrote one of `mask`'s fields or
    /// read it in part.
    pub fn relevant(state: &segments::State, table: TableId, mask: u64) -> io::Result<bool> {
        let changes = columns::changes(state, table)?;
        Ok(changes.truncated
            || !changes.inserted.is_empty()
            || !changes.deleted.is_empty()
            || changes
                .masks
                .iter()
                .chain(&changes.fill_masks)
                .any(|written| written & mask != 0))
    }
}

/// A complete variant read's raw operands ([`Naming::read`]).
pub struct Read {
    pub evidence: VariantHour,
    /// The raw operands of each definition read anew, by id ascending.
    pub built: Vec<(u64, RawVariant)>,
    /// Whether every live definition was read: none is held but those built.
    pub whole: bool,
    /// Definitions gone since the read before, ascending.
    pub dropped: Vec<u64>,
}

/// One segment's variant names, which start whole.
pub struct Naming {
    definition_fields: [usize; 6],
    lexical_literal: usize,
    country_slot: usize,
    country_tag: usize,
    appearance_name: usize,
    pending: HashSet<u64>,
    all: bool,
    modules: slots::Index<u64>,
    upgrades: slots::Index<u64>,
    token_variants: HashMap<u32, HashSet<u64>>,
    variant_tokens: HashMap<u64, Vec<u32>>,
    #[cfg(any(test, feature = "test-support"))]
    reads: usize,
}

impl Naming {
    pub fn new() -> io::Result<Self> {
        let table = TableId::VariantDefinitions;
        Ok(Self {
            definition_fields: [
                columns::field(table, "equipment_token")?,
                columns::field(table, "creator_link")?,
                columns::field(table, "origin_link")?,
                columns::field(table, "append_variant_name")?,
                columns::field(table, "display_name")?,
                columns::field(table, "variant_name")?,
            ],
            lexical_literal: columns::field(TableId::LexicalTokens, "literal")?,
            country_slot: columns::field(TableId::Countries, "canonical_slot")?,
            country_tag: columns::field(TableId::Countries, "tag")?,
            appearance_name: columns::field(TableId::CountryAppearance, "name_tag")?,
            pending: HashSet::new(),
            all: true,
            modules: slots::Index::default(),
            upgrades: slots::Index::default(),
            token_variants: HashMap::new(),
            variant_tokens: HashMap::new(),
            #[cfg(any(test, feature = "test-support"))]
            reads: 0,
        })
    }

    /// The bytes it holds on the heap, about.
    pub fn heap(&self) -> usize {
        self.pending.capacity() * size_of::<u64>()
            + self.modules.heap()
            + self.upgrades.heap()
            + self.token_variants.capacity() * size_of::<(u32, HashSet<u64>)>()
            + self
                .token_variants
                .values()
                .map(|variants| variants.capacity() * size_of::<u64>())
                .sum::<usize>()
            + self.variant_tokens.capacity() * size_of::<(u64, Vec<u32>)>()
            + self
                .variant_tokens
                .values()
                .map(|tokens| tokens.capacity() * size_of::<u32>())
                .sum::<usize>()
    }

    /// The definitions read so far.
    #[cfg(any(test, feature = "test-support"))]
    pub fn reads(&self) -> usize {
        self.reads
    }

    fn invalidate(
        &mut self,
        state: &segments::State,
        table: TableId,
        mask: u64,
        id: impl Fn(u128) -> u64,
    ) -> io::Result<()> {
        let held = columns::table(state, table)?;
        for slot in slots::edits(state, table, mask)? {
            if let Some(key) = held.key(slot) {
                self.pending.insert(id(key));
            }
        }
        for &key in &columns::changes(state, table)?.deleted_keys {
            self.pending.insert(id(key));
        }
        Ok(())
    }

    fn definition(
        &mut self,
        state: &segments::State,
        slot: u32,
        format: u16,
    ) -> io::Result<VariantDefinitionRow> {
        #[cfg(any(test, feature = "test-support"))]
        {
            self.reads += 1;
        }
        // read_raw uses only these six definition fields.
        let table = TableId::VariantDefinitions;
        let held = columns::table(state, table)?;
        let f = self.definition_fields;
        if f.iter().any(|&field| !held.recorded(field, slot)) {
            return columns::row(state, table, slot, format);
        }
        Ok(VariantDefinitionRow {
            id: held.key(slot).expect("named definition") as u64,
            equipment_token: columns::plain::<u32>(held, f[0])?[slot as usize],
            creator_link: columns::plain::<u32>(held, f[1])?[slot as usize],
            origin_link: columns::plain::<u32>(held, f[2])?[slot as usize],
            append_variant_name: columns::plain::<u8>(held, f[3])?[slot as usize],
            display_name: slots::text(state, table, f[4], slot)?,
            variant_name: slots::text(state, table, f[5], slot)?,
            ..Default::default()
        })
    }

    fn forget_dependencies(&mut self, id: u64) {
        if let Some(tokens) = self.variant_tokens.remove(&id) {
            for token in tokens {
                if let Some(variants) = self.token_variants.get_mut(&token) {
                    variants.remove(&id);
                    if variants.is_empty() {
                        self.token_variants.remove(&token);
                    }
                }
            }
        }
    }

    /// [`Self::read`]'s evidence and the operands it built.
    pub fn project(
        &mut self,
        state: &segments::State,
        world: (&Roster, &Names),
        format: replay_data::format::FormatVersion,
        hour: i32,
        whole: bool,
        shaped: &[bool; TABLE_COUNT],
        changed: &[bool; TABLE_COUNT],
        read: bool,
    ) -> io::Result<(VariantHour, Vec<(u64, RawVariant)>)> {
        let read = self.read(state, world, format, hour, whole, shaped, changed, read)?;
        Ok((read.evidence, read.built))
    }

    /// The hour's variant evidence and, at a complete read, the operands of every definition
    /// whose naming inputs changed since the read before (every live one after `whole`, a
    /// table whose status moved, or an incomplete read). `shaped` and `changed` are the
    /// tables whose status, and whose rows, the hour moved; `read` whether the variant
    /// table's read is dated at the hour.
    pub fn read(
        &mut self,
        state: &segments::State,
        (roster, names): (&Roster, &Names),
        format: replay_data::format::FormatVersion,
        hour: i32,
        whole: bool,
        shaped: &[bool; TABLE_COUNT],
        changed: &[bool; TABLE_COUNT],
        read: bool,
    ) -> io::Result<Read> {
        let cpu = replay_frame::cpu::scope("variants.columnar.naming_changes", None);
        slots::structural(
            &mut self.modules,
            state,
            TableId::VariantModules,
            whole || shaped[TableId::VariantModules.index()],
            false,
            |key| (key >> 32) as u64,
        )?;
        slots::structural(
            &mut self.upgrades,
            state,
            TableId::VariantUpgrades,
            whole || shaped[TableId::VariantUpgrades.index()],
            false,
            |key| (key >> 32) as u64,
        )?;
        self.all |= whole
            || [
                TableId::VariantDefinitions,
                TableId::VariantModules,
                TableId::VariantUpgrades,
                TableId::LexicalTokens,
                TableId::Countries,
                TableId::CountryLinks,
                TableId::CountryAppearance,
            ]
            .iter()
            .any(|table| shaped[table.index()]);
        self.all |= changed[TableId::CountryLinks.index()]
            || slots::relevant(
                state,
                TableId::Countries,
                slots::mask(&[self.country_slot, self.country_tag]),
            )?
            || slots::relevant(
                state,
                TableId::CountryAppearance,
                1_u64 << self.appearance_name,
            )?;
        if !self.all {
            self.invalidate(
                state,
                TableId::VariantDefinitions,
                slots::mask(&self.definition_fields),
                |key| key as u64,
            )?;
            self.invalidate(state, TableId::VariantModules, u64::MAX, |key| {
                (key >> 32) as u64
            })?;
            self.invalidate(state, TableId::VariantUpgrades, u64::MAX, |key| {
                (key >> 32) as u64
            })?;
            if changed[TableId::LexicalTokens.index()] {
                let table = TableId::LexicalTokens;
                let held = columns::table(state, table)?;
                for token in slots::edits(state, table, 1_u64 << self.lexical_literal)?
                    .into_iter()
                    .filter_map(|slot| held.key(slot).map(|key| key as u32))
                    .chain(
                        columns::changes(state, table)?
                            .deleted_keys
                            .iter()
                            .map(|&key| key as u32),
                    )
                {
                    self.pending.extend(
                        self.token_variants
                            .get(&token)
                            .into_iter()
                            .flatten()
                            .copied(),
                    );
                }
            }
        }
        // VariantChanges is a nonempty witness; none of its payload is a naming operand.
        let read = read
            || (columns::complete(state, TableId::VariantChanges)
                && columns::table(state, TableId::VariantChanges)?
                    .live()
                    .ones()
                    .next()
                    .is_some());
        let complete = columns::complete(state, TableId::VariantDefinitions);
        let evidence = if !read {
            VariantHour::Carried { complete }
        } else if !complete
            || !columns::complete(state, TableId::Countries)
            || !columns::complete(state, TableId::CountryLinks)
            || !columns::complete(state, TableId::LexicalTokens)
        {
            VariantHour::Incomplete
        } else {
            VariantHour::Read
        };
        let unread = |evidence| Read {
            evidence,
            built: Vec::new(),
            whole: false,
            dropped: Vec::new(),
        };
        if !matches!(evidence, VariantHour::Read) {
            self.all |= matches!(
                evidence,
                VariantHour::Incomplete | VariantHour::Carried { complete: false }
            );
            return Ok(unread(evidence));
        }
        let held = columns::table(state, TableId::VariantDefinitions)?;
        let whole = self.all;
        let mut ids: Vec<_> = if whole {
            self.pending.clear();
            self.token_variants.clear();
            self.variant_tokens.clear();
            held.live()
                .ones()
                .filter_map(|slot| held.key(slot).map(|key| key as u64))
                .collect()
        } else {
            self.pending.drain().collect()
        };
        self.all = false;
        ids.sort_unstable();
        drop(cpu);
        let mut read = Read {
            whole,
            ..unread(evidence)
        };
        if ids.is_empty() {
            return Ok(read);
        }
        let _cpu = replay_frame::cpu::scope("variants.columnar.naming_read", None);
        let design_known = columns::complete(state, TableId::VariantModules)
            && columns::complete(state, TableId::VariantUpgrades);
        let mut definitions = Vec::with_capacity(ids.len());
        let mut design_modules = Vec::with_capacity(ids.len());
        let mut design_upgrades = Vec::with_capacity(ids.len());
        let mut tokens = Vec::new();
        // Slot 0's tag localises every variant's type line.
        let mut countries = vec![0_u32];
        for id in ids {
            self.forget_dependencies(id);
            let Some(slot) = held.slot(u128::from(id)) else {
                read.dropped.push(id);
                continue;
            };
            let definition = self.definition(state, slot, format.version())?;
            let raw = if definition.origin_link as i32 >= 1 {
                definition.origin_link
            } else {
                definition.creator_link
            };
            countries.extend(roster.slot_of(raw));
            let mut modules = Vec::new();
            let mut upgrades = Vec::new();
            if design_known {
                for &slot in self.modules.ordered(state, TableId::VariantModules, id)? {
                    modules.push(columns::row::<VariantModuleRow>(
                        state,
                        TableId::VariantModules,
                        slot,
                        format.version(),
                    )?);
                }
                for &slot in self.upgrades.ordered(state, TableId::VariantUpgrades, id)? {
                    upgrades.push(columns::row::<VariantUpgradeRow>(
                        state,
                        TableId::VariantUpgrades,
                        slot,
                        format.version(),
                    )?);
                }
            }
            let mut own: Vec<_> = std::iter::once(definition.equipment_token)
                .chain(
                    modules
                        .iter()
                        .flat_map(|row| [row.slot, row.definition_token]),
                )
                .chain(upgrades.iter().map(|row| row.definition_token as u32))
                .collect();
            own.sort_unstable();
            own.dedup();
            for &token in &own {
                self.token_variants.entry(token).or_default().insert(id);
            }
            tokens.extend_from_slice(&own);
            self.variant_tokens.insert(id, own);
            definitions.push(definition);
            design_modules.push(modules);
            design_upgrades.push(upgrades);
        }
        if definitions.is_empty() {
            return Ok(read);
        }
        countries.sort_unstable();
        countries.dedup();
        let mut rows = Vec::with_capacity(countries.len());
        let mut appearance = names.appearances_complete().then(Vec::new);
        for country in countries {
            // A slot's last country by id names it.
            if let Some(held) = roster.at_slot(country).last() {
                rows.push(CountryRow {
                    id: held.id,
                    canonical_slot: held.slot,
                    tag: roster.tag(held.tag).to_vec(),
                    ..Default::default()
                });
            }
            if let (Some(Some(raw)), Some(appearances)) =
                (names.appearance(country), &mut appearance)
            {
                let table = TableId::CountryAppearance;
                let held = columns::table(state, table)?;
                let slot = held
                    .slot(u128::from(raw))
                    .ok_or_else(|| io::Error::other("A naming appearance holds no row"))?;
                let row = if held.recorded(self.appearance_name, slot) {
                    CountryAppearanceRow {
                        country: raw,
                        name_tag: slots::text(state, table, self.appearance_name, slot)?,
                        ..Default::default()
                    }
                } else {
                    columns::row(state, table, slot, format.version())?
                };
                appearances.push(row);
            }
        }
        let tags = Tags::new(&rows, roster.links(), appearance.as_deref());
        tokens.sort_unstable();
        tokens.dedup();
        let table = TableId::LexicalTokens;
        let held = columns::table(state, table)?;
        let mut lexical = Vec::with_capacity(tokens.len());
        for token in tokens {
            if let Some(slot) = held.slot(u128::from(token)) {
                lexical.push(columns::row::<LexicalTokenRow>(
                    state,
                    table,
                    slot,
                    format.version(),
                )?);
            }
        }
        read.built.reserve_exact(definitions.len());
        for ((definition, modules), upgrades) in definitions
            .iter()
            .zip(&design_modules)
            .zip(&design_upgrades)
        {
            let modules: Vec<_> = modules.iter().collect();
            let upgrades: Vec<_> = upgrades.iter().collect();
            read.built.push((
                definition.id,
                read_raw(
                    definition,
                    hour,
                    &tags,
                    &lexical,
                    design_known.then_some((modules.as_slice(), upgrades.as_slice())),
                ),
            ));
        }
        Ok(read)
    }
}
