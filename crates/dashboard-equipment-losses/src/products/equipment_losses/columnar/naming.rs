use super::*;
use dashboard_base::products::equipment::{Tags, read_raw};

pub(super) struct Naming {
    definition_fields: [usize; 6],
    lexical_literal: usize,
    pending: HashSet<u64>,
    all: bool,
    modules: slots::Index<u64>,
    upgrades: slots::Index<u64>,
    token_variants: HashMap<u32, HashSet<u64>>,
    variant_tokens: HashMap<u64, Vec<u32>>,
    #[cfg(test)]
    pub(super) reads: usize,
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
            pending: HashSet::new(),
            all: true,
            modules: slots::Index::default(),
            upgrades: slots::Index::default(),
            token_variants: HashMap::new(),
            variant_tokens: HashMap::new(),
            #[cfg(test)]
            reads: 0,
        })
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
        #[cfg(test)]
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

    pub fn project(
        &mut self,
        state: &segments::State,
        context: &Context,
        fields: &Fields,
        format: FormatVersion,
        hour: i32,
        whole: bool,
        shaped: &[bool; TABLE_COUNT],
        changed: &[bool; TABLE_COUNT],
        read: bool,
    ) -> io::Result<(VariantHour, Vec<(u64, RawVariant)>)> {
        let cpu = replay_frame::cpu::scope("equipment_losses.columnar.naming_changes", None);
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
            || relevant(
                state,
                TableId::Countries,
                slots::mask(&[fields.country_slot, fields.country_tag]),
            )?
            || relevant(
                state,
                TableId::CountryAppearance,
                1_u64 << fields.appearance_name,
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
        if !matches!(evidence, VariantHour::Read) {
            self.all |= matches!(
                evidence,
                VariantHour::Incomplete | VariantHour::Carried { complete: false }
            );
            return Ok((evidence, Vec::new()));
        }
        let held = columns::table(state, TableId::VariantDefinitions)?;
        let mut ids: Vec<_> = if self.all {
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
        if ids.is_empty() {
            return Ok((evidence, Vec::new()));
        }
        let _cpu = replay_frame::cpu::scope("equipment_losses.columnar.naming_read", None);
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
                continue;
            };
            let definition = self.definition(state, slot, format.version())?;
            let raw = if definition.origin_link as i32 >= 1 {
                definition.origin_link
            } else {
                definition.creator_link
            };
            countries.extend(context.raw_slots.get(&raw).copied());
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
            return Ok((evidence, Vec::new()));
        }
        countries.sort_unstable();
        countries.dedup();
        let mut rows = Vec::with_capacity(countries.len());
        let mut appearance = context.appearance.as_ref().map(|_| Vec::new());
        for country in countries {
            if let Some(&slot) = context.countries.get(&country) {
                rows.push(context.country(state, fields, slot, format.version(), true)?);
            }
            if let (Some(index), Some(appearances)) = (&context.appearance, &mut appearance) {
                if let Some(&slot) = index.get(&country) {
                    let table = TableId::CountryAppearance;
                    let held = columns::table(state, table)?;
                    let row = if held.recorded(fields.appearance_name, slot) {
                        CountryAppearanceRow {
                            country: held.key(slot).expect("naming appearance") as u32,
                            name_tag: slots::text(state, table, fields.appearance_name, slot)?,
                            ..Default::default()
                        }
                    } else {
                        columns::row(state, table, slot, format.version())?
                    };
                    appearances.push(row);
                }
            }
        }
        let tags = Tags::new(&rows, &context.links, appearance.as_deref());
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
        let mut built = Vec::with_capacity(definitions.len());
        for ((definition, modules), upgrades) in definitions
            .iter()
            .zip(&design_modules)
            .zip(&design_upgrades)
        {
            let modules: Vec<_> = modules.iter().collect();
            let upgrades: Vec<_> = upgrades.iter().collect();
            built.push((
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
        Ok((evidence, built))
    }
}
