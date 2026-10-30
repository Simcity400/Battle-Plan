use super::*;
use crate::prepared::frame_changes::{FrameChanges, field_mask};
use replay_data::rows::{
    CountryProductionModifiersRow, ProductionLineModifiersRow, RowCodec, full_mask,
};

#[derive(Clone, Copy)]
pub(super) struct ReadEvidence {
    pub hour: i32,
    pub lines_read: bool,
    pub output_read: bool,
    pub modifiers_read: bool,
    pub design_revision: u64,
    pub design_read_hour: Option<i32>,
    pub organisation_read_hour: Option<i32>,
    pub organisation_changed: bool,
    pub author_names_due: bool,
}

pub(super) struct HourInput {
    pub rows_complete: bool,
    pub linked: BTreeSet<(u32, u32)>,
    pub identities_changed: bool,
    pub author_names_due: bool,
    pub hour: i32,
    pub last_hour: i32,
    pub countries: Option<BTreeMap<String, Country>>,
    pub holders: BTreeMap<(u32, u32), Option<String>>,
    pub lines_complete: bool,
    pub lines_read: bool,
    pub output_read: bool,
    pub output_complete: bool,
    pub organisations_complete: bool,
    pub organisations_named: bool,
    pub modifiers_complete: bool,
    pub modifiers_read: bool,
    pub modifier_refusal: Option<String>,
    pub lines: BTreeMap<u32, Vec<ProductionLineRow>>,
    pub unlinked_lines: Vec<ProductionLineRow>,
    pub output: BTreeMap<u32, Vec<ProductionLineOutputRow>>,
    pub unlinked_output: Vec<ProductionLineOutputRow>,
    pub modifier_countries: BTreeMap<u32, CountryProductionModifiersRow>,
    pub modifier_lines: BTreeMap<u64, ProductionLineModifiersRow>,
    pub organisations: BTreeMap<u64, RawOrganisation>,
    pub author_names: BTreeMap<u64, RawOrganisation>,
    pub organisation_names: BTreeMap<u64, Option<String>>,
    pub designs: BTreeMap<u64, DesignRead>,
    pub design_revision: u64,
    pub design_read_hour: Option<i32>,
    pub organisation_read_hour: Option<i32>,
    pub icons: BTreeMap<u64, IconChoice>,
}

impl HourInput {
    pub fn design_bytes(&self, variant: u64) -> usize {
        std::mem::size_of::<DesignRead>()
            + self
                .designs
                .get(&variant)
                .and_then(|read| read.author)
                .and_then(|author| self.author_names.get(&author))
                .map_or(0, record_bytes)
    }

    pub fn output_due(&self, country: Option<Country>) -> Option<bool> {
        self.output_read.then_some(match country {
            Some(Country::Absent) => true,
            Some(Country::Found(..)) => {
                self.lines_read && self.lines_complete && self.output_complete
            }
            Some(Country::Unresolved) | None => false,
        })
    }
}

#[derive(Default)]
pub(super) struct World {
    pub variants: VariantCatalogue,
    pub pending_bytes: usize,
    pub icon_events: Vec<(u64, i32, IconChoice, bool)>,
    definitions: BTreeMap<u64, VariantDefinitionRow>,
    definition_view: Option<ViewIdentity>,
    definition_pending: Option<BTreeSet<u128>>,
    designs: Option<ViewIdentity>,
    design_hour: Option<i32>,
    revision: u64,
    views: BTreeMap<TableId, ViewIdentity>,
    tracker: crate::prepared::frame_changes::FrameChangeTracker,
    country_keys: Vec<(u32, u32, Vec<u8>)>,
    // Countries rows use raw-ID order; owner names resolve through canonical slots.
    country_rows: BTreeMap<u32, usize>,
    last_hour: Option<i32>,
    mio_valid: bool,
    mio_rows: BTreeMap<u64, IndustrialOrganisationRow>,
    mio_details: BTreeMap<u64, IndustrialOrganisationDetailsRow>,
    mio_tokens: BTreeMap<u32, BTreeSet<u64>>,
    mio_countries: BTreeMap<u32, BTreeSet<u64>>,
    mio_owners: BTreeMap<u32, BTreeSet<u64>>,
    mio_owner_slots: BTreeMap<u64, u32>,
    /// Names a refused quiet check already took from this hour's variant events, for its extract.
    naming: Option<(i32, Vec<(u64, crate::products::equipment::VariantVersion)>)>,
    /// An icon rule's table changed at a quiet hour without a census since the last extract.
    pictured_changed: bool,
    /// Variants whose versions changed since the last chunk, and since the last ledger record.
    flushed: BTreeSet<u64>,
    saved_variants: BTreeSet<u64>,
    /// Definitions the cache took since the last ledger record.
    saved_definitions: BTreeSet<u64>,
}

/// The world in one ledger record: the latest version of each variant and the definitions
/// the cache took since the record before, or all of them in a full record. A chunk is
/// written before every record, so no icon event or pending version is left.
#[derive(Clone, Debug, Default)]
pub(super) struct WorldSaved {
    full: bool,
    variants: BTreeMap<u64, Vec<crate::products::equipment::VariantVersion>>,
    last_hour: Option<i32>,
    definitions: Vec<u8>,
    definition_count: usize,
    revision: u64,
}

impl World {
    /// The variants whose versions the next chunk holds.
    pub fn take_flushed(&mut self) -> BTreeSet<u64> {
        std::mem::take(&mut self.flushed)
    }

    /// Marks a variant whose versions a test edited as the next chunk's and ledger record's.
    #[cfg(test)]
    pub fn touch_variant(&mut self, id: u64) {
        self.flushed.insert(id);
        self.saved_variants.insert(id);
    }

    pub fn save(&mut self, full: bool) -> io::Result<WorldSaved> {
        let variants = std::mem::take(&mut self.saved_variants);
        let definitions = std::mem::take(&mut self.saved_definitions);
        let rows: Vec<VariantDefinitionRow> = if full {
            self.definitions.values().cloned().collect()
        } else {
            definitions
                .iter()
                .filter_map(|id| self.definitions.get(id).cloned())
                .collect()
        };
        Ok(WorldSaved {
            full,
            variants: if full {
                self.variants.versions.clone()
            } else {
                variants
                    .iter()
                    .filter_map(|id| Some((*id, self.variants.versions.get(id)?.clone())))
                    .collect()
            },
            last_hour: self.variants.last_hour(),
            definition_count: rows.len(),
            definitions: index::encode_row(replay_data::rows::TableRows::VariantDefinitions(rows))?,
            revision: self.revision,
        })
    }

    /// The world the ledger records hold, applied in order from the last full one.
    pub fn resume(saved: Vec<WorldSaved>) -> io::Result<Self> {
        let mut versions = BTreeMap::new();
        let mut definitions = BTreeMap::new();
        let mut last_hour = None;
        let mut revision = 0;
        for saved in saved {
            if saved.full {
                versions.clear();
                definitions.clear();
            }
            let mut decoder = replay_data::codec::Decoder::new(&saved.definitions);
            let replay_data::rows::TableRows::VariantDefinitions(rows) =
                TableId::VariantDefinitions.decode_rows(saved.definition_count, &mut decoder)?
            else {
                return Err(crate::invalid("Invalid production definitions"));
            };
            decoder.finish()?;
            definitions.extend(rows.into_iter().map(|row| (row.id, row)));
            versions.extend(saved.variants);
            last_hour = saved.last_hour;
            revision = saved.revision;
        }
        Ok(Self {
            variants: VariantCatalogue::resumed(versions, last_hour),
            definitions,
            revision,
            ..Self::default()
        })
    }
}

impl World {
    pub fn changes(&mut self, replay: &Replay, frame: &DecodedFrame) -> FrameChanges {
        self.tracker.observe(frame, replay.format)
    }
    pub fn recorded_changes(
        &mut self,
        replay: &Replay,
        frame: &DecodedFrame,
        updates: replay_frame::frame::RecordedFrame,
    ) -> FrameChanges {
        self.tracker.observe_updates(frame, updates, replay.format)
    }
    fn track_definitions(&mut self, changes: &FrameChanges) {
        if let Some(pending) = &mut self.definition_pending {
            if let Some((changed, deleted)) = changes.rows(TableId::VariantDefinitions) {
                pending.extend(
                    changed
                        .iter()
                        .map(|row| row.key)
                        .chain(deleted.iter().copied()),
                );
                if pending
                    .len()
                    .saturating_mul(std::mem::size_of::<u128>() * 8)
                    > crate::prepared::pages::BATCH_BYTES
                {
                    self.definition_pending = None;
                }
            } else {
                self.definition_pending = None;
            }
        }
    }
    pub fn identity_changed(&self, frame: &DecodedFrame, changes: &FrameChanges) -> bool {
        [TableId::Countries, TableId::CountryLinks]
            .into_iter()
            .any(|table| {
                relevant(changes, table)
                    && self
                        .views
                        .get(&table)
                        .is_none_or(|old| !old.is(&frame.tables[table.index()]))
            })
    }
    pub fn quiet(
        &mut self,
        replay: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        evidence: bool,
    ) -> bool {
        self.track_definitions(changes);
        if self.last_hour.and_then(|hour| hour.checked_add(1)) != Some(frame.hour) {
            return false;
        }
        let reads = slot_reads(replay.format, &frame.metrics, frame.hour);
        let empty_catalogue = self.variants.versions.is_empty()
            && self.definitions.is_empty()
            && match frame.complete::<tables::VariantDefinitions>() {
                Ok(rows) => rows.is_empty(),
                Err(_) => true,
            };
        if (!evidence
            && (reads[TableId::ProductionLines.index()]
                || reads[TableId::ProductionLineOutput.index()]
                || self.design_hour
                    != frame.tables[TableId::VariantStats.index()]
                        .sampled_at()
                        .or_else(|| frame.tables[TableId::VariantModules.index()].sampled_at())))
            || (reads[TableId::VariantDefinitions.index()] && !(evidence && empty_catalogue))
        {
            return false;
        }
        let census = reads[TableId::ProductionLines.index()];
        for table in PreparedTables::tables() {
            // Variant events are judged below by what they name.
            if table == TableId::VariantChanges {
                continue;
            }
            if self
                .views
                .get(&table)
                .is_some_and(|old| old.is(&frame.tables[table.index()]))
            {
                continue;
            }
            let unchanged = !relevant(changes, table)
                || match table {
                    TableId::Countries => frame.complete::<tables::Countries>().is_ok_and(|rows| {
                        rows.len() == self.country_keys.len()
                            && rows.iter().zip(&self.country_keys).all(|(row, old)| {
                                (row.canonical_slot, row.id, row.tag.as_slice())
                                    == (old.0, old.1, old.2.as_slice())
                            })
                    }),
                    _ => false,
                };
            if unchanged {
                self.views
                    .insert(table, frame.tables[table.index()].identity());
            } else if !census && super::prepared::PICTURED.contains(&table) {
                // An hour without a census that stays quiet chooses no icon; the next census
                // chooses them again.
                self.pictured_changed = true;
                self.views
                    .insert(table, frame.tables[table.index()].identity());
            } else {
                return false;
            }
        }
        if census && self.pictured_changed {
            return false;
        }
        if equipment::design_changed(frame) {
            // Variant events that change no definition and name no new version leave every input
            // as it was; names they do produce go to this hour's extract.
            let definitions = &frame.tables[TableId::VariantDefinitions.index()];
            if !self
                .definition_view
                .as_ref()
                .is_some_and(|old| old.is(definitions))
            {
                return false;
            }
            let naming = self.variants.observe_changed(replay.format, frame, changes);
            if !naming.is_empty() {
                self.naming = Some((frame.hour, naming));
                return false;
            }
        } else {
            self.variants.advance_changed(frame, changes);
        }
        self.last_hour = Some(frame.hour);
        crate::prepared::prepass::note("production-lines.quiet_hour");
        true
    }

    pub fn evidence(&mut self, replay: &Replay, frame: &DecodedFrame) -> ReadEvidence {
        let reads = slot_reads(replay.format, &frame.metrics, frame.hour);
        let design_hour = frame.complete::<tables::VariantStats>().ok().and_then(|_| {
            frame.tables[TableId::VariantStats.index()]
                .sampled_at()
                .or_else(|| frame.tables[TableId::VariantModules.index()].sampled_at())
        });
        if self.design_hour != design_hour {
            self.revision += 1;
            self.design_hour = design_hour;
        }
        ReadEvidence {
            hour: frame.hour,
            lines_read: reads[TableId::ProductionLines.index()],
            output_read: reads[TableId::ProductionLineOutput.index()],
            modifiers_read: reads[TableId::CountryProductionModifiers.index()]
                && reads[TableId::ProductionLineModifiers.index()],
            design_revision: self.revision,
            design_read_hour: design_hour,
            organisation_read_hour: frame.tables[TableId::IndustrialOrganisationDetails.index()]
                .sampled_at(),
            organisation_changed: false,
            author_names_due: VariantCatalogue::hour_read(
                frame,
                reads[TableId::VariantDefinitions.index()],
            ) == equipment::VariantHour::Read
                || reads[TableId::ProductionLines.index()]
                || reads[TableId::VariantStats.index()]
                || reads[TableId::VariantModules.index()],
        }
    }

    #[cfg(test)]
    pub fn extract(
        &mut self,
        replay: &Replay,
        frame: &DecodedFrame,
        icons: Option<&IconContent>,
        demanded: &BTreeSet<u64>,
    ) -> io::Result<HourInput> {
        let changes = self.changes(replay, frame);
        self.extract_cached(replay, frame, icons, demanded, None, &changes)
    }
    pub fn extract_cached(
        &mut self,
        replay: &Replay,
        frame: &DecodedFrame,
        icons: Option<&IconContent>,
        _demanded: &BTreeSet<u64>,
        previous: Option<HourInput>,
        changes: &FrameChanges,
    ) -> io::Result<HourInput> {
        self.track_definitions(changes);
        let hour = frame.hour;
        let reads = slot_reads(replay.format, &frame.metrics, hour);
        let read = |table: TableId| reads[table.index()];
        let countries = frame.complete::<tables::Countries>().ok();
        let links = frame.complete::<tables::CountryLinks>().ok();
        let lexical = frame.complete::<tables::LexicalTokens>().ok();
        let variant_hour = VariantCatalogue::hour_read(frame, read(TableId::VariantDefinitions));
        let naming = match self.naming.take() {
            Some((named, naming)) if named == hour => naming,
            _ => self.variants.observe_changed(replay.format, frame, changes),
        };
        self.pending_bytes += naming
            .iter()
            .map(|(_, version)| record_bytes(version))
            .sum::<usize>();
        for (id, _) in &naming {
            self.flushed.insert(*id);
            self.saved_variants.insert(*id);
        }
        let changed = |table: TableId| {
            relevant(changes, table)
                && self
                    .views
                    .get(&table)
                    .is_none_or(|old| !old.is(&frame.tables[table.index()]))
        };
        let countries_changed = previous.is_none()
            || [TableId::Countries, TableId::CountryLinks]
                .into_iter()
                .any(changed);
        let names_changed = previous.is_none()
            || !self.mio_valid
            || [
                TableId::Countries,
                TableId::CountryLinks,
                TableId::LexicalTokens,
                TableId::IndustrialOrganisations,
                TableId::IndustrialOrganisationDetails,
                TableId::IndustrialOrganisationPolicies,
            ]
            .into_iter()
            .any(changed);
        let stats_changed = previous.is_none() || changed(TableId::VariantStats);
        let definitions_read = (equipment::design_changed(frame)
            || read(TableId::VariantDefinitions))
            && frame.complete::<tables::VariantDefinitions>().is_ok();
        let mut refresh = BTreeSet::new();
        let mut altered = BTreeSet::new();
        if definitions_read {
            let view = &frame.tables[TableId::VariantDefinitions.index()];
            if !self
                .definition_view
                .as_ref()
                .is_some_and(|old| old.is(view))
            {
                let rows = frame.complete::<tables::VariantDefinitions>()?;
                let selected = self
                    .definition_pending
                    .as_ref()
                    .cloned()
                    .unwrap_or_else(|| rows.iter().map(RowCodec::key).collect());
                for id in selected {
                    let Ok(at) = rows.binary_search_by_key(&id, RowCodec::key) else {
                        continue;
                    };
                    let definition = &rows[at];
                    if self.definitions.get(&definition.id) != Some(definition) {
                        refresh.insert(definition.id);
                        if self.definitions.contains_key(&definition.id) {
                            altered.insert(definition.id);
                        }
                    }
                    if refresh.contains(&definition.id) {
                        self.definitions.insert(definition.id, definition.clone());
                        self.saved_definitions.insert(definition.id);
                        crate::prepared::prepass::note("production-lines.definition_projection");
                    }
                }
                self.definition_view = Some(view.identity());
            }
            self.definition_pending = Some(BTreeSet::new());
        }
        let design_view = frame
            .complete::<tables::VariantStats>()
            .ok()
            .map(|_| &frame.tables[TableId::VariantStats.index()]);
        let design_hour = design_view.and_then(|view| {
            view.sampled_at()
                .or_else(|| frame.tables[TableId::VariantModules.index()].sampled_at())
        });
        if self.design_hour != design_hour
            || match (&self.designs, design_view) {
                (Some(old), Some(now)) => !old.is(now),
                (None, None) => false,
                _ => true,
            }
        {
            self.revision += 1;
        }
        self.designs = design_view.map(|view| view.identity());
        self.design_hour = design_hour;
        let mut holders = BTreeMap::new();
        let mut selected = BTreeMap::new();
        if countries_changed && let Some(countries) = countries {
            if let Some(links) = links {
                for (tag, country) in crate::metadata::tagged_countries(countries, links) {
                    let Ok(tag) = std::str::from_utf8(tag) else {
                        continue;
                    };
                    selected.insert(
                        tag.to_owned(),
                        match country {
                            TaggedCountry::Absent => Country::Absent,
                            TaggedCountry::Unresolved => Country::Unresolved,
                            TaggedCountry::Found(row, slot) => Country::Found(slot, row.id),
                        },
                    );
                }
            }
            for country in countries {
                holders.insert(
                    (country.canonical_slot, country.id),
                    std::str::from_utf8(&country.tag).ok().map(str::to_owned),
                );
            }
        }
        let catalogue = icons.and_then(|icons| icons.catalogue.as_deref());
        let modifiers_read =
            read(TableId::CountryProductionModifiers) && read(TableId::ProductionLineModifiers);
        let modifiers_complete = frame
            .complete::<tables::CountryProductionModifiers>()
            .is_ok()
            && frame.complete::<tables::ProductionLineModifiers>().is_ok();
        let modifier_refusal = (read(TableId::ProductionLines)
            && frame.complete::<tables::ProductionLines>().is_ok()
            && modifiers_read
            && modifiers_complete)
            .then(|| {
                replay_derive::fixed_layout::validate_recording(
                    frame,
                    replay_derive::fixed_layout::PRODUCTION_MODIFIERS,
                )
                .err()
            })
            .flatten();
        let linked = countries
            .filter(|_| countries_changed)
            .zip(links)
            .map(|(countries, links)| {
                countries
                    .iter()
                    .filter(|country| link_slot(links, country.id) == Some(country.canonical_slot))
                    .map(|country| (country.canonical_slot, country.id))
                    .collect()
            })
            .unwrap_or_default();
        let mut input = previous.unwrap_or_else(|| HourInput {
            rows_complete: false,
            linked: BTreeSet::new(),
            identities_changed: true,
            author_names_due: false,
            hour,
            last_hour: hour,
            countries: None,
            holders: BTreeMap::new(),
            lines_complete: frame.complete::<tables::ProductionLines>().is_ok(),
            lines_read: read(TableId::ProductionLines),
            output_read: read(TableId::ProductionLineOutput),
            output_complete: frame.complete::<tables::ProductionLineOutput>().is_ok(),
            organisations_complete: frame.complete::<tables::IndustrialOrganisations>().is_ok(),
            organisations_named: frame.complete::<tables::IndustrialOrganisations>().is_ok()
                && lexical.is_some(),
            modifiers_complete,
            modifiers_read,
            modifier_refusal: modifier_refusal.clone(),
            lines: BTreeMap::new(),
            unlinked_lines: Vec::new(),
            output: BTreeMap::new(),
            unlinked_output: Vec::new(),
            modifier_countries: BTreeMap::new(),
            modifier_lines: BTreeMap::new(),
            organisations: BTreeMap::new(),
            author_names: BTreeMap::new(),
            organisation_names: BTreeMap::new(),
            designs: BTreeMap::new(),
            design_revision: self.revision,
            design_read_hour: design_hour,
            organisation_read_hour: None,
            icons: BTreeMap::new(),
        });
        input.hour = hour;
        input.last_hour = hour;
        input.rows_complete = countries.is_some();
        input.identities_changed = countries_changed
            && (input.holders != holders
                || input.linked != linked
                || input.countries.as_ref() != countries.zip(links).map(|_| &selected));
        if countries_changed {
            input.holders = holders;
            input.countries = countries.zip(links).map(|_| selected);
            input.linked = linked;
        }
        input.lines_complete = frame.complete::<tables::ProductionLines>().is_ok();
        input.lines_read = read(TableId::ProductionLines);
        input.output_read = read(TableId::ProductionLineOutput);
        input.output_complete = frame.complete::<tables::ProductionLineOutput>().is_ok();
        input.organisations_complete = frame.complete::<tables::IndustrialOrganisations>().is_ok();
        input.organisations_named = input.organisations_complete
            && lexical.is_some()
            && countries.is_some()
            && links.is_some();
        input.modifiers_complete = modifiers_complete;
        input.modifiers_read = modifiers_read;
        input.modifier_refusal = modifier_refusal;
        input.design_revision = self.revision;
        input.design_read_hour = design_hour;
        input.organisation_read_hour =
            frame.tables[TableId::IndustrialOrganisationDetails.index()].sampled_at();
        input.author_names_due = variant_hour == equipment::VariantHour::Read
            || definitions_read
            || (input.lines_read && input.lines_complete)
            || read(TableId::VariantStats)
            || read(TableId::VariantModules);
        if stats_changed && frame.complete::<tables::VariantStats>().is_err() {
            input.designs.clear();
        }
        if stats_changed && let Ok(stats) = frame.complete::<tables::VariantStats>() {
            if changes.rows(TableId::VariantStats).is_none() {
                input.designs.clear();
            }
            for id in candidates(changes, TableId::VariantStats, stats) {
                let Ok(at) = stats.binary_search_by_key(&id, RowCodec::key) else {
                    input.designs.remove(&(id as u64));
                    continue;
                };
                let row = &stats[at];
                crate::prepared::prepass::note("production-lines.design_projection");
                input.designs.insert(
                    row.variant,
                    DesignRead {
                        first_hour: hour,
                        read_hour: design_hour,
                        author: Some(row.organisation),
                        trait_count: Some(row.organisation_trait_count),
                        bonus_count: Some(row.organisation_bonuses.len() as u32),
                    },
                );
            }
        }
        if countries_changed {
            self.country_rows = countries
                .into_iter()
                .flatten()
                .enumerate()
                .map(|(at, row)| (row.canonical_slot, at))
                .collect();
        }
        if names_changed
            && let (Some(countries), Some(links), Some(lexical), Ok(rows)) = (
                countries,
                links,
                lexical,
                frame.complete::<tables::IndustrialOrganisations>(),
            )
        {
            let reseed = !self.mio_valid;
            let mut affected: BTreeSet<u64> = if reseed {
                rows.iter().map(RowCodec::key).collect()
            } else {
                candidates(changes, TableId::IndustrialOrganisations, rows)
            }
            .into_iter()
            .map(|id| id as u64)
            .collect();
            if changes.rows(TableId::IndustrialOrganisations).is_none() {
                affected.extend(self.mio_rows.keys().copied());
            }
            let details = frame
                .complete::<tables::IndustrialOrganisationDetails>()
                .ok();
            if changed(TableId::IndustrialOrganisationDetails) {
                if changes
                    .rows(TableId::IndustrialOrganisationDetails)
                    .is_none()
                {
                    affected.extend(self.mio_details.keys().copied());
                }
                affected.extend(
                    candidates(
                        changes,
                        TableId::IndustrialOrganisationDetails,
                        details.unwrap_or_default(),
                    )
                    .into_iter()
                    .map(|id| id as u64),
                );
                if details.is_none() {
                    affected.extend(self.mio_details.keys().copied());
                }
            }
            let policies = frame
                .complete::<tables::IndustrialOrganisationPolicies>()
                .ok();
            if changed(TableId::IndustrialOrganisationPolicies) {
                for id in candidates(
                    changes,
                    TableId::IndustrialOrganisationPolicies,
                    policies.unwrap_or_default(),
                ) {
                    affected.extend(
                        self.mio_countries
                            .get(&(id as u32))
                            .into_iter()
                            .flatten()
                            .copied(),
                    );
                }
                if policies.is_none()
                    || changes
                        .rows(TableId::IndustrialOrganisationPolicies)
                        .is_none()
                {
                    affected.extend(self.mio_rows.keys().copied());
                }
            }
            if changed(TableId::LexicalTokens) {
                if changes.rows(TableId::LexicalTokens).is_none() {
                    affected.extend(self.mio_rows.keys().copied());
                }
                for token in candidates(changes, TableId::LexicalTokens, lexical) {
                    affected.extend(
                        self.mio_tokens
                            .get(&(token as u32))
                            .into_iter()
                            .flatten()
                            .copied(),
                    );
                }
            }
            if changed(TableId::CountryLinks) {
                affected.extend(self.mio_rows.keys().copied());
            } else if changed(TableId::Countries) {
                if changes.rows(TableId::Countries).is_none() {
                    affected.extend(self.mio_rows.keys().copied());
                } else {
                    for key in candidates(changes, TableId::Countries, countries) {
                        let old = self
                            .country_keys
                            .binary_search_by_key(&(key as u32), |row| row.1)
                            .ok()
                            .map(|at| self.country_keys[at].0);
                        let now = countries
                            .binary_search_by_key(&key, RowCodec::key)
                            .ok()
                            .map(|at| countries[at].canonical_slot);
                        for slot in old.into_iter().chain(now) {
                            affected
                                .extend(self.mio_owners.get(&slot).into_iter().flatten().copied());
                        }
                    }
                }
            }
            for id in affected {
                crate::prepared::prepass::note("production-lines.mio_candidate");
                let old_row = self.mio_rows.get(&id);
                let old_details = self.mio_details.get(&id);
                let tokens = organisation_tokens(old_row, old_details);
                for token in tokens {
                    remove_user(&mut self.mio_tokens, token, id);
                }
                if let Some(row) = old_row {
                    remove_user(&mut self.mio_countries, row.country, id);
                }
                if let Some(slot) = self.mio_owner_slots.remove(&id) {
                    remove_user(&mut self.mio_owners, slot, id);
                }
                let Ok(at) = rows.binary_search_by_key(&id, |row| row.organisation) else {
                    self.mio_rows.remove(&id);
                    self.mio_details.remove(&id);
                    input.organisations.remove(&id);
                    input.author_names.remove(&id);
                    input.organisation_names.remove(&id);
                    continue;
                };
                let row = &rows[at];
                let detail = details
                    .and_then(|rows| {
                        rows.binary_search_by_key(&id, |row| row.organisation)
                            .ok()
                            .map(|at| &rows[at])
                    })
                    .filter(|detail| detail.country == row.country);
                self.mio_countries
                    .entry(row.country)
                    .or_default()
                    .insert(id);
                let owner_slot = link_slot(links, row.owner);
                if let Some(slot) = owner_slot {
                    self.mio_owners.entry(slot).or_default().insert(id);
                    self.mio_owner_slots.insert(id, slot);
                }
                for token in organisation_tokens(Some(row), detail) {
                    self.mio_tokens.entry(token).or_default().insert(id);
                }
                crate::prepared::prepass::note("production-lines.mio_value_projection");
                let owner: Vec<_> = owner_slot
                    .and_then(|slot| self.country_rows.get(&slot))
                    .map(|&at| countries[at].clone())
                    .into_iter()
                    .collect();
                let tags = Tags::new(&owner, links, None);
                let mut raw = raw_organisation(row, hour, &tags, lexical);
                if input.author_names.get(&id).is_none_or(|old| {
                    RawOrganisation {
                        first_hour: hour,
                        ..old.clone()
                    } != raw
                }) {
                    crate::prepared::prepass::note("production-lines.organisation_projection");
                    input.organisation_names.insert(
                        id,
                        organisation_name(&raw, catalogue.map(|catalogue| catalogue as &dyn Texts)),
                    );
                    input.author_names.insert(id, raw.clone());
                }
                raw.state = detail.map(|detail| {
                    let mut state = organisation_state(
                        detail,
                        frame.tables[TableId::IndustrialOrganisationDetails.index()].sampled_at(),
                        lexical,
                    );
                    state.policy_name = policies
                        .and_then(|rows| {
                            rows.binary_search_by_key(&detail.country, |row| row.country)
                                .ok()
                                .map(|at| &rows[at])
                        })
                        .and_then(|row| {
                            row.policies
                                .iter()
                                .find(|policy| policy.token == detail.state.policy)
                        })
                        .map(|policy| text(&policy.name));
                    state
                });
                input.organisations.insert(id, raw);
                self.mio_rows.insert(id, row.clone());
                if let Some(detail) = detail {
                    self.mio_details.insert(id, detail.clone());
                } else {
                    self.mio_details.remove(&id);
                }
            }
            self.mio_valid = true;
        } else if !input.organisations_named {
            self.mio_valid = false;
            input.organisations.clear();
            input.author_names.clear();
            input.organisation_names.clear();
            self.mio_rows.clear();
            self.mio_details.clear();
            self.mio_tokens.clear();
            self.mio_countries.clear();
            self.mio_owners.clear();
            self.mio_owner_slots.clear();
        }
        if input.lines_read
            && let Some(links) = links
        {
            input.lines.clear();
            input.output.clear();
            input.unlinked_lines.clear();
            input.unlinked_output.clear();
            input.modifier_countries.clear();
            input.modifier_lines.clear();
            if let Ok(rows) = frame.complete::<tables::ProductionLines>() {
                for row in rows {
                    match link_slot(links, row.country) {
                        Some(slot) => input.lines.entry(slot).or_default().push(row.clone()),
                        None => input.unlinked_lines.push(row.clone()),
                    }
                }
            }
            if let Ok(rows) = frame.complete::<tables::ProductionLineOutput>() {
                for row in rows {
                    match link_slot(links, row.country) {
                        Some(slot) => input.output.entry(slot).or_default().push(row.clone()),
                        None => input.unlinked_output.push(row.clone()),
                    }
                }
            }
            if modifiers_read && modifiers_complete && input.modifier_refusal.is_none() {
                input.modifier_countries.extend(
                    frame
                        .complete::<tables::CountryProductionModifiers>()?
                        .iter()
                        .map(|row| (row.country, row.clone())),
                );
                input.modifier_lines.extend(
                    frame
                        .complete::<tables::ProductionLineModifiers>()?
                        .iter()
                        .map(|row| (row.line, row.clone())),
                );
            }
        }
        if (definitions_read || input.lines_read)
            && let (Some(content), Some(lexical)) = (icons, lexical)
        {
            let mut produced = refresh.clone();
            produced.extend(naming.iter().map(|(id, _)| *id));
            if input.lines_read {
                produced.extend(
                    input
                        .lines
                        .values()
                        .flatten()
                        .chain(&input.unlinked_lines)
                        .map(|row| row.variant),
                );
                produced.extend(
                    input
                        .output
                        .values()
                        .flatten()
                        .chain(&input.unlinked_output)
                        .map(|row| row.variant),
                );
            }
            let state = IconHour::read(frame, content);
            for id in produced {
                let known = self.variants.current(id).is_some();
                let icon = match (&state, self.definitions.get(&id)) {
                    (Err(error), _) => IconChoice::unknown(error.to_string()),
                    (Ok(_), None) => IconChoice::unknown(
                        "the variant's definition is not recorded yet".to_owned(),
                    ),
                    (Ok(_), Some(_)) if !known => IconChoice::unknown(
                        "the variant's definition is not recorded at this time".to_owned(),
                    ),
                    (Ok(state), Some(definition)) => icon_choice(
                        content,
                        state,
                        definition,
                        &equipment::variant_token(definition, lexical),
                    ),
                };
                if definitions_read
                    && known
                    && self.definitions.contains_key(&id)
                    && (refresh.contains(&id) || naming.iter().any(|(changed, _)| *changed == id))
                {
                    self.pending_bytes += record_bytes(&icon);
                    self.icon_events
                        .push((id, hour, icon.clone(), altered.contains(&id)));
                }
                input.icons.insert(id, icon);
            }
        }
        self.views = PreparedTables::tables()
            .into_iter()
            .map(|table| (table, frame.tables[table.index()].identity()))
            .collect();
        self.pictured_changed = false;
        if countries_changed {
            self.country_keys = countries
                .into_iter()
                .flatten()
                .map(|row| (row.canonical_slot, row.id, row.tag.clone()))
                .collect();
        }
        self.last_hour = Some(hour);
        Ok(input)
    }
}

struct PreparedTables;
impl PreparedTables {
    fn tables() -> Vec<TableId> {
        super::prepared::Prepared::tables(true)
    }
}

fn relevant(changes: &FrameChanges, table: TableId) -> bool {
    // A country's id is its row key: a new country is a row carrying every field.
    let fields: &[&str] = match table {
        TableId::Countries => &["canonical_slot", "tag"],
        _ => return changes.rows_changed[table.index()],
    };
    let identity = field_mask(table, fields);
    changes.rows(table).is_none_or(|(rows, deleted)| {
        !deleted.is_empty()
            || rows
                .iter()
                .any(|row| row.mask & identity != 0 || row.mask == full_mask(row.recorded_fields))
    })
}
fn candidates<T: RowCodec>(changes: &FrameChanges, table: TableId, rows: &[T]) -> BTreeSet<u128> {
    match changes.rows(table) {
        Some((changed, deleted)) => changed
            .iter()
            .map(|row| row.key)
            .chain(deleted.iter().copied())
            .collect(),
        None => rows.iter().map(RowCodec::key).collect(),
    }
}
fn remove_user(index: &mut BTreeMap<u32, BTreeSet<u64>>, key: u32, id: u64) {
    if let Some(users) = index.get_mut(&key) {
        users.remove(&id);
        if users.is_empty() {
            index.remove(&key);
        }
    }
}
fn organisation_tokens(
    row: Option<&IndustrialOrganisationRow>,
    detail: Option<&IndustrialOrganisationDetailsRow>,
) -> BTreeSet<u32> {
    row.into_iter()
        .map(|row| row.token)
        .chain(detail.into_iter().flat_map(|detail| {
            detail
                .unlocked
                .iter()
                .chain(&detail.queued)
                .copied()
                .chain(std::iter::once(detail.state.policy))
        }))
        .collect()
}

product_codec::product! { impl WorldSaved {
    full: bool,
    variants: BTreeMap<u64, Vec<crate::products::equipment::VariantVersion>>,
    last_hour: Option<i32>,
    definitions: Vec<u8>,
    definition_count: usize,
    revision: u64,
} default {} }
