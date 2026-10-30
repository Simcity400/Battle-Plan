//! The variant catalogue's operands from columns. A complete read builds a definition's
//! operands again only where an input it reads changed since the last build here: its own
//! row, modules or upgrades, or a spelling it reads; every definition at the segment's
//! first read and after the catalogue forgot names, as it then asks for them all.
use super::*;
use dashboard_base::products::equipment::{Tags, read_raw};
use replay_data::rows::{
    CountryAppearanceRow, CountryRow, VariantDefinitionRow, VariantModuleRow, VariantUpgradeRow,
};

pub(super) struct Naming {
    /// VariantDefinitions' equipment_token, creator_link, origin_link, append_variant_name,
    /// display_name and variant_name: what `read_raw` reads of a definition.
    definition: [usize; 6],
    pending: HashSet<u64>,
    all: bool,
    modules: slots::Index<u64>,
    upgrades: slots::Index<u64>,
    token_variants: HashMap<u32, HashSet<u64>>,
    variant_tokens: HashMap<u64, Vec<u32>>,
}

impl Naming {
    pub(super) fn new() -> io::Result<Self> {
        let table = TableId::VariantDefinitions;
        Ok(Self {
            definition: [
                columns::field(table, "equipment_token")?,
                columns::field(table, "creator_link")?,
                columns::field(table, "origin_link")?,
                columns::field(table, "append_variant_name")?,
                columns::field(table, "display_name")?,
                columns::field(table, "variant_name")?,
            ],
            pending: HashSet::new(),
            all: true,
            modules: slots::Index::default(),
            upgrades: slots::Index::default(),
            token_variants: HashMap::new(),
            variant_tokens: HashMap::new(),
        })
    }

    pub(super) fn heap(&self) -> usize {
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
        &self,
        state: &segments::State,
        slot: u32,
        format: u16,
    ) -> io::Result<VariantDefinitionRow> {
        let table = TableId::VariantDefinitions;
        let held = columns::table(state, table)?;
        let fields = self.definition;
        if fields.iter().any(|&field| !held.recorded(field, slot)) {
            return columns::row(state, table, slot, format);
        }
        let at = slot as usize;
        Ok(VariantDefinitionRow {
            id: held.key(slot).expect("named definition") as u64,
            equipment_token: columns::plain::<u32>(held, fields[0])?[at],
            creator_link: columns::plain::<u32>(held, fields[1])?[at],
            origin_link: columns::plain::<u32>(held, fields[2])?[at],
            append_variant_name: columns::plain::<u8>(held, fields[3])?[at],
            display_name: slots::text(state, table, fields[4], slot)?,
            variant_name: slots::text(state, table, fields[5], slot)?,
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

    /// The hour's evidence for the catalogue, and the operands of the definitions its
    /// complete read builds.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn project(
        &mut self,
        state: &segments::State,
        (roster, names): (&Roster, &Names),
        fields: &Fields,
        format: FormatVersion,
        hour: i32,
        whole: bool,
        shaped: &[bool; TABLE_COUNT],
        changed: &[bool; TABLE_COUNT],
        read: bool,
    ) -> io::Result<(VariantHour, Vec<(u64, RawVariant)>)> {
        slots::structural(
            &mut self.modules,
            state,
            TableId::VariantModules,
            whole || shaped[TableId::VariantModules.index()],
            |key| (key >> 32) as u64,
        )?;
        slots::structural(
            &mut self.upgrades,
            state,
            TableId::VariantUpgrades,
            whole || shaped[TableId::VariantUpgrades.index()],
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
                slots::mask(&self.definition),
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
                for token in slots::edits(state, table, 1_u64 << fields.literal)?
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
            // The catalogue forgets here: its next read asks for every definition.
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
        if ids.is_empty() {
            return Ok((evidence, Vec::new()));
        }
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
            return Ok((evidence, Vec::new()));
        }
        countries.sort_unstable();
        countries.dedup();
        let mut rows = Vec::with_capacity(countries.len());
        let mut appearance = names.appearances_complete().then(Vec::new);
        for country in countries {
            // A slot's last country by id names it, as `Tags` reads them.
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
                appearances.push(if held.recorded(fields.appearance_name, slot) {
                    CountryAppearanceRow {
                        country: raw,
                        name_tag: slots::text(state, table, fields.appearance_name, slot)?,
                        ..Default::default()
                    }
                } else {
                    columns::row(state, table, slot, format.version())?
                });
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
