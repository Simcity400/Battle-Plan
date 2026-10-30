use super::*;
use dashboard_base::products::equipment::{RawVariant, VariantHour};
use dashboard_text::metadata::slot_read;
use replay::{
    columnar_frames::metrics,
    columns,
    segments::{self, Item, Operator, Sink},
};
use replay_data::{format::FormatVersion, rows::*};
use replay_frame::frame::TableView;
use std::{collections::HashSet, mem::size_of, sync::Arc};

mod naming;
mod slots;

pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CombatLogEquipment,
    TableId::CombatLogState,
    TableId::DivisionCensus,
    TableId::EquipmentDestruction,
    TableId::LendLeaseEquipment,
    TableId::Players,
    TableId::SimulationFeatures,
    TableId::SliceReads,
    TableId::SunkConvoys,
    TableId::SunkShips,
    TableId::CountryAppearance,
    TableId::LexicalTokens,
    TableId::VariantDefinitions,
    TableId::VariantModules,
    TableId::VariantUpgrades,
    TableId::VariantChanges,
];

pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    pub(super) input: Option<Input>,
}

pub(super) struct Metadata {
    pub table: TableView,
    pub drain: Arc<DestructionInputs>,
    pub reads: [bool; 4],
    pub land: Arc<CoverageInputs>,
}

#[derive(Default)]
pub(super) struct Records {
    pub ships: Option<ShipCounts>,
    pub convoys: Vec<SunkConvoyRow>,
    pub lend: Vec<LendLeaseEquipmentRow>,
    pub combat: Vec<CombatLogEquipmentRow>,
}

pub(super) struct Amounts {
    pub events: Vec<(LeafKey, i128)>,
    pub error: Option<String>,
}

pub(super) struct Input {
    pub format: FormatVersion,
    pub identities: Option<Vec<((u32, u32), bool)>>,
    pub complete: bool,
    pub membership_complete: bool,
    pub refresh: bool,
    pub metadata: Arc<Metadata>,
    pub drains: Arc<Vec<(u32, Amounts)>>,
    pub memberships: Vec<(u32, Option<census::Membership>)>,
    pub memberships_whole: bool,
    pub records: Vec<(u32, Records)>,
    pub unresolved: [BTreeSet<u32>; 4],
    pub lend_process: bool,
    pub lend_read: bool,
    pub names: VariantHour,
    pub variants: Vec<(u64, RawVariant)>,
}

impl Item for Hour {
    fn heap(&self) -> usize {
        // Metadata and drains are Arcs the projector holds too.
        self.input.as_ref().map_or(0, |input| {
            let identities = input.identities.as_ref().map_or(0, |identities| {
                identities.capacity() * size_of::<((u32, u32), bool)>()
            });
            let records: usize = input
                .records
                .iter()
                .map(|(_, records)| records.heap())
                .sum();
            let unresolved: usize = input
                .unresolved
                .iter()
                .map(|slots| slots.len() * size_of::<u32>())
                .sum();
            let variants: usize = input
                .variants
                .iter()
                .map(|(_, variant)| {
                    [&variant.token, &variant.display_name, &variant.suffix]
                        .into_iter()
                        .chain(&variant.name_tag)
                        .chain(&variant.type_tag)
                        .chain(&variant.design)
                        .map(String::capacity)
                        .sum::<usize>()
                        + variant.refusals.capacity() * size_of::<(String, String)>()
                        + variant
                            .refusals
                            .iter()
                            .map(|(label, reason)| label.capacity() + reason.capacity())
                            .sum::<usize>()
                })
                .sum();
            identities
                + input.memberships.capacity() * size_of::<(u32, Option<census::Membership>)>()
                + input.records.capacity() * size_of::<(u32, Records)>()
                + records
                + unresolved
                + input.variants.capacity() * size_of::<(u64, RawVariant)>()
                + variants
        })
    }
}

impl Records {
    fn heap(&self) -> usize {
        let ships = self.ships.as_ref().map_or(0, |ships| {
            ships
                .keys()
                .map(|key| size_of::<(ShipKey, (usize, i32, u64))>() + key.0.capacity())
                .sum()
        });
        ships
            + self.convoys.capacity() * size_of::<SunkConvoyRow>()
            + self.convoys.iter().map(RowCodec::heap_bytes).sum::<usize>()
            + self.lend.capacity() * size_of::<LendLeaseEquipmentRow>()
            + self.lend.iter().map(RowCodec::heap_bytes).sum::<usize>()
            + self.combat.capacity() * size_of::<CombatLogEquipmentRow>()
            + self.combat.iter().map(RowCodec::heap_bytes).sum::<usize>()
    }
}

struct Fields {
    campaign: usize,
    country_slot: usize,
    country_tag: usize,
    appearance_name: usize,
    grouped: usize,
    ungrouped: usize,
    player_country: usize,
    features: usize,
    gate: usize,
    prune: usize,
    date: usize,
    log_amount: usize,
}

impl Fields {
    fn new() -> io::Result<Self> {
        Ok(Self {
            campaign: columns::field(TableId::Campaign, "id")?,
            country_slot: columns::field(TableId::Countries, "canonical_slot")?,
            country_tag: columns::field(TableId::Countries, "tag")?,
            appearance_name: columns::field(TableId::CountryAppearance, "name_tag")?,
            grouped: columns::field(TableId::DivisionCensus, "grouped")?,
            ungrouped: columns::field(TableId::DivisionCensus, "ungrouped")?,
            player_country: columns::field(TableId::Players, "country")?,
            features: columns::field(TableId::SimulationFeatures, "dlc_mask")?,
            gate: columns::field(TableId::CombatLogState, "viewing_country_raw")?,
            prune: columns::field(TableId::CombatLogState, "prune_observed")?,
            date: columns::field(TableId::SliceReads, "sample_hour")?,
            log_amount: columns::field(TableId::CombatLogEquipment, "amount_raw")?,
        })
    }
}

#[derive(Default)]
struct Context {
    links: Vec<CountryLinkRow>,
    raw_slots: HashMap<u32, u32>,
    aliases: HashMap<u32, Vec<u32>>,
    countries: HashMap<u32, u32>,
    appearance: Option<HashMap<u32, u32>>,
}

impl Context {
    fn links(&mut self, state: &segments::State, format: u16) -> io::Result<()> {
        self.links.clear();
        self.raw_slots.clear();
        self.aliases.clear();
        if columns::complete(state, TableId::CountryLinks) {
            let held = columns::table(state, TableId::CountryLinks)?;
            for slot in held.live().ones() {
                self.links
                    .push(columns::row(state, TableId::CountryLinks, slot, format)?);
            }
            self.links.sort_unstable_by_key(RowCodec::key);
            for link in &self.links {
                self.raw_slots.insert(link.id, link.canonical_slot);
                self.aliases
                    .entry(link.canonical_slot)
                    .or_default()
                    .push(link.id);
            }
        }
        Ok(())
    }

    fn country(
        &self,
        state: &segments::State,
        fields: &Fields,
        slot: u32,
        format: u16,
        name: bool,
    ) -> io::Result<CountryRow> {
        let held = columns::table(state, TableId::Countries)?;
        if !held.recorded(fields.country_slot, slot)
            || (name && !held.recorded(fields.country_tag, slot))
        {
            return columns::row(state, TableId::Countries, slot, format);
        }
        Ok(CountryRow {
            id: held.key(slot).expect("current country") as u32,
            canonical_slot: columns::plain::<u32>(held, fields.country_slot)?[slot as usize],
            tag: if name {
                slots::text(state, TableId::Countries, fields.country_tag, slot)?
            } else {
                Vec::new()
            },
            ..Default::default()
        })
    }

    fn countries(
        &mut self,
        state: &segments::State,
        fields: &Fields,
        format: u16,
    ) -> io::Result<Vec<((u32, u32), bool)>> {
        self.countries.clear();
        let mut identities = Vec::new();
        if columns::complete(state, TableId::Countries) {
            let held = columns::table(state, TableId::Countries)?;
            let mut slots: Vec<_> = held.live().ones().collect();
            slots.sort_unstable_by_key(|&slot| held.key(slot));
            for slot in slots {
                let row = self.country(state, fields, slot, format, false)?;
                self.countries.insert(row.canonical_slot, slot);
                if columns::complete(state, TableId::CountryLinks) {
                    identities.push((
                        (row.canonical_slot, row.id),
                        mapped_slot(&row, &self.links).is_ok(),
                    ));
                }
            }
        }
        Ok(identities)
    }

    fn appearance(&mut self, state: &segments::State) -> io::Result<()> {
        self.appearance = if columns::complete(state, TableId::CountryAppearance)
            && columns::complete(state, TableId::CountryLinks)
        {
            let held = columns::table(state, TableId::CountryAppearance)?;
            let mut slots: Vec<_> = held.live().ones().collect();
            slots.sort_unstable_by_key(|&slot| held.key(slot));
            Some(
                slots
                    .into_iter()
                    .filter_map(|slot| {
                        let raw = held.key(slot)? as u32;
                        self.raw_slots.get(&raw).map(|&country| (country, slot))
                    })
                    .collect(),
            )
        } else {
            None
        };
        Ok(())
    }
}

#[derive(Default)]
struct Census {
    unresolved: HashSet<u32>,
}

impl Census {
    fn counts(
        state: &segments::State,
        fields: &Fields,
        slot: u32,
        format: u16,
    ) -> io::Result<(u32, u32)> {
        let held = columns::table(state, TableId::DivisionCensus)?;
        if !held.recorded(fields.grouped, slot) || !held.recorded(fields.ungrouped, slot) {
            let row: DivisionCensusRow =
                columns::row(state, TableId::DivisionCensus, slot, format)?;
            return Ok((row.grouped, row.ungrouped));
        }
        Ok((
            columns::plain::<u32>(held, fields.grouped)?[slot as usize],
            columns::plain::<u32>(held, fields.ungrouped)?[slot as usize],
        ))
    }

    fn project(
        &mut self,
        state: &segments::State,
        context: &Context,
        fields: &Fields,
        format: u16,
        reset: bool,
        changed: bool,
    ) -> io::Result<Vec<(u32, Option<census::Membership>)>> {
        if !columns::complete(state, TableId::DivisionCensus) {
            self.unresolved.clear();
            return Ok(Vec::new());
        }
        if !reset && !changed {
            return Ok(Vec::new());
        }
        let held = columns::table(state, TableId::DivisionCensus)?;
        let before = !self.unresolved.is_empty();
        let mut countries = Vec::new();
        let visit = if reset {
            self.unresolved.clear();
            held.live().ones().collect()
        } else {
            let edits = columns::changes(state, TableId::DivisionCensus)?;
            for (&slot, &key) in edits.deleted.iter().zip(&edits.deleted_keys) {
                self.unresolved.remove(&slot);
                countries.extend(context.raw_slots.get(&(key as u32)).copied());
            }
            slots::edits(state, TableId::DivisionCensus, u64::MAX)?
        };
        for slot in visit {
            let raw = held.key(slot).expect("current division census") as u32;
            if let Some(&country) = context.raw_slots.get(&raw) {
                self.unresolved.remove(&slot);
                countries.push(country);
            } else {
                let (grouped, ungrouped) = Self::counts(state, fields, slot, format)?;
                if grouped != 0 || ungrouped != 0 {
                    self.unresolved.insert(slot);
                } else {
                    self.unresolved.remove(&slot);
                }
            }
        }
        if reset || before != !self.unresolved.is_empty() {
            countries.extend(context.aliases.keys().copied());
        }
        countries.sort_unstable();
        countries.dedup();
        let mut values = Vec::with_capacity(countries.len());
        for country in countries {
            let (mut grouped, mut ungrouped, mut found) = (0_u64, 0_u64, false);
            for raw in context.aliases.get(&country).into_iter().flatten() {
                if let Some(slot) = held.slot(u128::from(*raw)) {
                    let counts = Self::counts(state, fields, slot, format)?;
                    grouped += u64::from(counts.0);
                    ungrouped += u64::from(counts.1);
                    found = true;
                }
            }
            let membership = found
                .then(|| {
                    census::Membership::from_counts(grouped, ungrouped, !self.unresolved.is_empty())
                })
                .flatten();
            values.push((country, membership));
        }
        Ok(values)
    }
}

pub struct Projector {
    format: FormatVersion,
    campaign: Vec<u8>,
    mode: replay_data::scope::CaptureMode,
    shapes: [Option<columns::Status>; TABLE_COUNT],
    fields: Fields,
    context: Context,
    census: Census,
    naming: naming::Naming,
    log: slots::Index<u32>,
    dates: slots::Index<i32>,
    log_pending: slots::Index<u32>,
    collisions: slots::Index<u32>,
    pending: [bool; 3],
    drain: Arc<DestructionInputs>,
    drains: Arc<Vec<(u32, Amounts)>>,
    land: Arc<CoverageInputs>,
    metadata: Option<Arc<Metadata>>,
    cold: bool,
    #[cfg(test)]
    log_reads: usize,
}

impl Projector {
    pub fn new(replay: &Replay) -> io::Result<Self> {
        Ok(Self {
            format: replay.format,
            campaign: replay.campaign_id.clone(),
            mode: replay.capture_mode,
            shapes: [None; TABLE_COUNT],
            fields: Fields::new()?,
            context: Context::default(),
            census: Census::default(),
            naming: naming::Naming::new()?,
            log: slots::Index::default(),
            dates: slots::Index::default(),
            log_pending: slots::Index::default(),
            collisions: slots::Index::default(),
            pending: [false; 3],
            drain: Arc::default(),
            drains: Arc::default(),
            land: Arc::default(),
            metadata: None,
            cold: true,
            #[cfg(test)]
            log_reads: 0,
        })
    }

    #[cfg(test)]
    pub(super) fn naming_reads(&self) -> usize {
        self.naming.reads
    }

    #[cfg(test)]
    pub(super) fn log_reads(&self) -> usize {
        self.log_reads
    }

    fn status(&self, state: &segments::State) -> io::Result<TableView> {
        Ok(
            match columns::status(state, TableId::EquipmentDestruction) {
                columns::Status::Complete => {
                    TableView::Complete(TableRows::EquipmentDestruction(Vec::new()))
                }
                columns::Status::Truncated { missing } => TableView::Truncated {
                    rows: TableRows::EquipmentDestruction(Vec::new()),
                    missing,
                },
                columns::Status::NotRecorded => TableView::NotRecorded { mode: self.mode },
                columns::Status::Unavailable { reason } => {
                    TableView::Unavailable {
                        reason: std::str::from_utf8(state.dictionary().get(reason).ok_or_else(
                            || io::Error::other("Equipment loss status has no reason"),
                        )?)
                        .map_err(io::Error::other)?
                        .to_owned(),
                    }
                }
            },
        )
    }

    fn coverage(&self, state: &segments::State) -> io::Result<CoverageInputs> {
        if !columns::complete(state, TableId::CountryLinks) {
            return Ok(CoverageInputs::default());
        }
        let Some(slot) = single(state, TableId::CombatLogState)? else {
            return Ok(CoverageInputs::default());
        };
        let held = columns::table(state, TableId::CombatLogState)?;
        let (gate, prune) =
            if held.recorded(self.fields.gate, slot) && held.recorded(self.fields.prune, slot) {
                (
                    columns::plain::<i32>(held, self.fields.gate)?[slot as usize],
                    columns::plain::<u8>(held, self.fields.prune)?[slot as usize],
                )
            } else {
                let row: CombatLogStateRow =
                    columns::row(state, TableId::CombatLogState, slot, self.format.version())?;
                (row.viewing_country_raw, row.prune_observed)
            };
        let mut inputs = CoverageInputs {
            all: gate < 1,
            prune_observed: prune != 0,
            ..Default::default()
        };
        if !inputs.all {
            if !columns::complete(state, TableId::Players) {
                return Ok(inputs);
            }
            let held = columns::table(state, TableId::Players)?;
            let owners = columns::plain::<u32>(held, self.fields.player_country)?;
            for slot in held.live().ones() {
                let raw = if held.recorded(self.fields.player_country, slot) {
                    owners[slot as usize]
                } else {
                    columns::row::<PlayerRow>(state, TableId::Players, slot, self.format.version())?
                        .country
                };
                if let Some(&country) = self.context.raw_slots.get(&raw) {
                    inputs.players.insert(country);
                } else {
                    inputs.unresolved = true;
                }
            }
        }
        inputs.gate_known = true;
        if let Some(slot) = single(state, TableId::SimulationFeatures)? {
            let held = columns::table(state, TableId::SimulationFeatures)?;
            let mask = if held.recorded(self.fields.features, slot) {
                columns::plain::<u32>(held, self.fields.features)?[slot as usize]
            } else {
                columns::row::<SimulationFeatureRow>(
                    state,
                    TableId::SimulationFeatures,
                    slot,
                    self.format.version(),
                )?
                .dlc_mask
            };
            inputs.features =
                Some(mask & engine_dlc::dlc_feature_state::EQUIPMENT_LOSSES_MASK != 0);
        }
        Ok(inputs)
    }

    fn destruction(&mut self, state: &segments::State) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("equipment_losses.columnar.drains", None);
        let mut inputs = DestructionInputs::default();
        let mut countries: HashMap<u32, (Vec<(LeafKey, i128)>, Option<(u128, String)>)> =
            HashMap::new();
        if columns::complete(state, TableId::EquipmentDestruction) {
            let table = TableId::EquipmentDestruction;
            let held = columns::table(state, table)?;
            let first = held.live().ones().min_by_key(|&slot| held.key(slot));
            let head: Option<EquipmentDestructionRow> = first
                .map(|slot| columns::row(state, table, slot, self.format.version()))
                .transpose()?;
            let marker = head.as_ref().filter(|row| {
                row.coverage
                    == Recorded::Value(EquipmentDestructionCoverage::SupplementalUnavailable)
            });
            let mut missing = self.format.field_gaps(table).next().is_some()
                || head
                    .as_ref()
                    .is_some_and(|row| row.not_recorded_fields() != 0);
            inputs.nonempty = held.live().count() > usize::from(marker.is_some());
            for slot in held.live().ones() {
                if Some(slot) == first && marker.is_some() {
                    continue;
                }
                let row: EquipmentDestructionRow =
                    columns::row(state, table, slot, self.format.version())?;
                missing |= row.not_recorded_fields() != 0;
                let Some(&country) = self.context.raw_slots.get(&row.country) else {
                    inputs.unresolved.insert(row.country);
                    continue;
                };
                let (events, error) = countries.entry(country).or_default();
                let invalid = if row.variant as u32 != VARIANT_TYPE {
                    Some(format!(
                        "Equipment destruction design {:#x} has an unsupported reference type",
                        row.variant
                    ))
                } else if row.amount_raw == 0 {
                    Some("Equipment destruction contains a zero event amount".into())
                } else {
                    None
                };
                if let Some(message) = invalid {
                    if error.as_ref().is_none_or(|old| row.key() < old.0) {
                        *error = Some((row.key(), message));
                    }
                } else {
                    events.push((
                        (
                            LossSource::Destruction(row.cause),
                            Some((row.variant >> 32) as u32),
                        ),
                        i128::from(row.amount_raw),
                    ));
                }
            }
            inputs.supplemental = if missing {
                Some("EquipmentDestruction supplemental coverage not recorded".into())
            } else {
                marker
                    .map(|row| {
                        row.unavailable_reason
                            .require("EquipmentDestruction.unavailable_reason")
                            .cloned()
                            .map_err(io::Error::other)
                    })
                    .transpose()?
            };
        }
        let mut drains: Vec<_> = countries
            .into_iter()
            .map(|(country, (mut events, error))| {
                events.sort_unstable_by_key(|event| event.0);
                let mut amounts: Vec<(LeafKey, i128)> = Vec::with_capacity(events.len());
                for (leaf, amount) in events {
                    if let Some(last) = amounts.last_mut().filter(|last| last.0 == leaf) {
                        last.1 += amount;
                    } else {
                        amounts.push((leaf, amount));
                    }
                }
                (
                    country,
                    Amounts {
                        events: amounts,
                        error: error.map(|error| error.1),
                    },
                )
            })
            .collect();
        drains.sort_unstable_by_key(|item| item.0);
        self.drain = Arc::new(inputs);
        self.drains = Arc::new(drains);
        Ok(())
    }

    fn collision(&mut self, state: &segments::State, key: u128) -> io::Result<()> {
        let raw = (key >> 88) as u32;
        let Some(country) = self.context.raw_slots.get(&raw) else {
            return Ok(());
        };
        let aliases = &self.context.aliases[country];
        if aliases.len() < 2 {
            return Ok(());
        }
        let held = columns::table(state, TableId::CombatLogEquipment)?;
        let suffix = key & ((1_u128 << 88) - 1);
        let mut present = Vec::new();
        for &owner in aliases {
            if let Some(slot) = held
                .slot((u128::from(owner) << 88) | suffix)
                .filter(|&slot| self.log.active(slot))
            {
                present.push((owner, slot));
            }
        }
        let collides = present.len() > 1;
        for (owner, slot) in present {
            self.collisions.remove(slot);
            if collides {
                self.collisions.insert(slot, owner);
            }
            self.log_pending.insert(slot, owner);
        }
        Ok(())
    }

    fn indexes(
        &mut self,
        state: &segments::State,
        whole: bool,
        mapping: bool,
        shapes: &[bool; TABLE_COUNT],
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("equipment_losses.columnar.combat_index", None);
        slots::structural(
            &mut self.log,
            state,
            TableId::CombatLogEquipment,
            whole || shapes[TableId::CombatLogEquipment.index()],
            true,
            |key| (key >> 88) as u32,
        )?;
        let table = TableId::CombatLogEquipment;
        let changes = columns::changes(state, table)?;
        let reset = whole || mapping || shapes[table.index()] || changes.truncated;
        if reset {
            self.log_pending.clear();
        }
        if columns::complete(state, table) || columns::truncated(state, table).is_some() {
            let held = columns::table(state, table)?;
            let dirty = if reset {
                slots::live(state, table)?
            } else {
                for &slot in &changes.deleted {
                    self.log_pending.remove(slot);
                }
                slots::edits(state, table, 1_u64 << self.fields.log_amount)?
            };
            for slot in dirty {
                self.log_pending.insert(
                    slot,
                    (held.key(slot).expect("changed combat key") >> 88) as u32,
                );
            }
        }
        let mut collision_keys = Vec::new();
        if reset {
            self.collisions.clear();
            let held = columns::table(state, table)?;
            for aliases in self
                .context
                .aliases
                .values()
                .filter(|aliases| aliases.len() > 1)
            {
                for &raw in aliases {
                    collision_keys
                        .extend(self.log.get(raw).iter().filter_map(|&slot| held.key(slot)));
                }
            }
        } else {
            for &slot in &changes.deleted {
                self.collisions.remove(slot);
            }
            collision_keys.extend_from_slice(&changes.deleted_keys);
            let held = columns::table(state, table)?;
            collision_keys.extend(changes.inserted.iter().filter_map(|&slot| held.key(slot)));
        }
        for key in collision_keys {
            self.collision(state, key)?;
        }
        let table = TableId::SliceReads;
        if !columns::complete(state, table) {
            self.dates.clear();
            return Ok(());
        }
        let held = columns::table(state, table)?;
        let reset = whole || shapes[table.index()];
        let visit = if reset {
            self.dates.clear();
            held.live().ones().collect()
        } else {
            for &slot in &columns::changes(state, table)?.deleted {
                self.dates.remove(slot);
            }
            slots::edits(state, table, 1_u64 << self.fields.date)?
        };
        let dates = columns::plain::<i32>(held, self.fields.date)?;
        for slot in visit {
            if held.key(slot).expect("slice read date") >> 32 == TableId::CombatLogEquipment as u128
            {
                let date = if held.recorded(self.fields.date, slot) {
                    dates[slot as usize]
                } else {
                    columns::row::<SliceReadRow>(state, table, slot, self.format.version())?
                        .sample_hour
                };
                self.dates.insert(slot, date);
            } else {
                self.dates.remove(slot);
            }
        }
        Ok(())
    }

    fn combat(
        &mut self,
        state: &segments::State,
        hour: i32,
        records: &mut HashMap<u32, Records>,
        unresolved: &mut BTreeSet<u32>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("equipment_losses.columnar.combat_read", None);
        let dates = columns::table(state, TableId::SliceReads)?;
        let mut owners: Vec<_> = self
            .dates
            .get(hour)
            .iter()
            .filter_map(|&slot| dates.key(slot).map(|key| key as u32))
            .collect();
        owners.sort_unstable();
        owners.dedup();
        for raw in owners {
            let Some(&country) = self.context.raw_slots.get(&raw) else {
                if self.log.contains(raw) {
                    unresolved.insert(raw);
                }
                continue;
            };
            let mut selected = self.log_pending.take(raw);
            selected.extend_from_slice(self.collisions.get(raw));
            let held = columns::table(state, TableId::CombatLogEquipment)?;
            selected.sort_unstable_by_key(|&slot| held.key(slot));
            selected.dedup();
            let amounts = columns::plain::<i64>(held, self.fields.log_amount)?;
            #[cfg(test)]
            {
                self.log_reads += selected.len();
            }
            for slot in selected {
                let row = if held.recorded(self.fields.log_amount, slot) {
                    let key = held.key(slot).expect("supplying combat key");
                    // Combat Log preparation consumes only the replay key and signed amount.
                    CombatLogEquipmentRow {
                        country: raw,
                        group: (key >> 56) as u32,
                        kind: (key >> 48) as u8,
                        month: (key >> 32) as u16,
                        variant: key as u32,
                        amount_raw: amounts[slot as usize],
                        ..Default::default()
                    }
                } else {
                    columns::row(
                        state,
                        TableId::CombatLogEquipment,
                        slot,
                        self.format.version(),
                    )?
                };
                records.entry(country).or_default().combat.push(row);
            }
        }
        Ok(())
    }

    fn ships(
        &self,
        state: &segments::State,
        records: &mut HashMap<u32, Records>,
        unresolved: &mut BTreeSet<u32>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("equipment_losses.columnar.ships", None);
        let slot = single(state, TableId::SunkShips)?.expect("complete sinking read");
        let row: SunkShipsRow =
            columns::row(state, TableId::SunkShips, slot, self.format.version())?;
        for entry in row.entries {
            if let Some(&country) = self.context.raw_slots.get(&entry.victim_country) {
                let counts = records
                    .entry(country)
                    .or_default()
                    .ships
                    .get_or_insert_with(BTreeMap::new);
                let key = (
                    entry.ship_name,
                    entry.sink_hour,
                    entry.victim_equipment_variant,
                    entry.battle,
                    entry.damage_source_reference,
                    entry.cause_record,
                );
                counts
                    .entry(key)
                    .or_insert((0, entry.sink_hour, entry.victim_equipment_variant))
                    .0 += 1;
            } else {
                unresolved.insert(entry.victim_country);
            }
        }
        Ok(())
    }

    fn convoys(
        &self,
        state: &segments::State,
        records: &mut HashMap<u32, Records>,
        unresolved: &mut BTreeSet<u32>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("equipment_losses.columnar.convoys", None);
        let table = TableId::SunkConvoys;
        for slot in columns::table(state, table)?.live().ones() {
            let row: SunkConvoyRow = columns::row(state, table, slot, self.format.version())?;
            if let Some(&country) = self.context.raw_slots.get(&row.owner_country) {
                records.entry(country).or_default().convoys.push(row);
            } else {
                unresolved.insert(row.owner_country);
            }
        }
        Ok(())
    }

    fn lend(
        &self,
        state: &segments::State,
        records: &mut HashMap<u32, Records>,
        unresolved: &mut BTreeSet<u32>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("equipment_losses.columnar.lend", None);
        let table = TableId::LendLeaseEquipment;
        for slot in columns::table(state, table)?.live().ones() {
            let row: LendLeaseEquipmentRow =
                columns::row(state, table, slot, self.format.version())?;
            if let Some(&country) = self.context.raw_slots.get(&row.giver) {
                records.entry(country).or_default().lend.push(row);
            } else {
                unresolved.insert(row.giver);
            }
        }
        for own in records.values_mut() {
            own.lend.sort_unstable_by_key(RowCodec::key);
        }
        Ok(())
    }
}

impl Operator for Projector {
    type Output = Hour;

    fn hour(&mut self, hour: &segments::Hour<'_>, out: &mut dyn Sink<Hour>) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("equipment_losses.columnar", Some("equipment_losses"));
        let state = hour.state;
        let whole = hour.opens || self.cold;
        let mut changed = [false; TABLE_COUNT];
        let mut shaped = [false; TABLE_COUNT];
        {
            let _cpu = replay_frame::cpu::scope("equipment_losses.columnar.status", None);
            for &table in TABLES {
                let status = columns::status(state, table);
                let edits = columns::changes(state, table)?;
                shaped[table.index()] = self.shapes[table.index()] != Some(status);
                changed[table.index()] = whole
                    || shaped[table.index()]
                    || edits.truncated
                    || !edits.changed.is_empty()
                    || !edits.fill_slots.is_empty()
                    || !edits.inserted.is_empty()
                    || !edits.deleted.is_empty();
                self.shapes[table.index()] = Some(status);
            }
            if columns::complete(state, TableId::Campaign) {
                let valid = match single(state, TableId::Campaign)? {
                    Some(slot) => {
                        let held = columns::table(state, TableId::Campaign)?;
                        if held.recorded(self.fields.campaign, slot) {
                            let id = columns::text(held, self.fields.campaign)?[slot as usize];
                            state.dictionary().get(id).ok_or_else(|| {
                                io::Error::other("Equipment loss campaign has no dictionary entry")
                            })? == self.campaign.as_slice()
                        } else {
                            columns::row::<CampaignRow>(
                                state,
                                TableId::Campaign,
                                slot,
                                self.format.version(),
                            )?
                            .id == self.campaign
                        }
                    }
                    None => false,
                };
                if !valid {
                    self.cold = true;
                    return out.push(Hour {
                        branch: hour.branch,
                        hour: hour.hour,
                        input: None,
                    });
                }
            }
        }
        let mapping = whole
            || shaped[TableId::Countries.index()]
            || changed[TableId::CountryLinks.index()]
            || relevant(state, TableId::Countries, 1_u64 << self.fields.country_slot)?;
        let identities;
        {
            let _cpu = replay_frame::cpu::scope("equipment_losses.columnar.identities", None);
            if whole || changed[TableId::CountryLinks.index()] {
                self.context.links(state, self.format.version())?;
            }
            identities = if mapping {
                Some(
                    self.context
                        .countries(state, &self.fields, self.format.version())?,
                )
            } else {
                None
            };
            if mapping
                || shaped[TableId::CountryAppearance.index()]
                || structural(state, TableId::CountryAppearance)?
            {
                self.context.appearance(state)?;
            }
        }
        let complete = columns::complete(state, TableId::Countries)
            && columns::complete(state, TableId::CountryLinks);
        let memberships_whole = mapping || shaped[TableId::DivisionCensus.index()];
        let memberships;
        {
            let _cpu = replay_frame::cpu::scope("equipment_losses.columnar.census", None);
            memberships = self.census.project(
                state,
                &self.context,
                &self.fields,
                self.format.version(),
                memberships_whole,
                changed[TableId::DivisionCensus.index()],
            )?;
        }
        let land_changed = mapping
            || shaped[TableId::Players.index()]
            || shaped[TableId::CombatLogState.index()]
            || shaped[TableId::SimulationFeatures.index()]
            || relevant(state, TableId::Players, 1_u64 << self.fields.player_country)?
            || relevant(
                state,
                TableId::CombatLogState,
                slots::mask(&[self.fields.gate, self.fields.prune]),
            )?
            || relevant(
                state,
                TableId::SimulationFeatures,
                1_u64 << self.fields.features,
            )?;
        if land_changed {
            let _cpu = replay_frame::cpu::scope("equipment_losses.columnar.coverage", None);
            self.land = Arc::new(self.coverage(state)?);
        }
        if mapping || changed[TableId::EquipmentDestruction.index()] {
            self.destruction(state)?;
        }
        self.indexes(state, whole, mapping, &shaped)?;
        let evidence = metrics(hour.manifest);
        let reads = [
            columns::complete(state, TableId::CombatLogEquipment)
                && columns::complete(state, TableId::SliceReads),
            single(state, TableId::SunkShips)?.is_some(),
            columns::complete(state, TableId::SunkConvoys),
            columns::complete(state, TableId::LendLeaseEquipment),
        ];
        let refresh = whole
            || mapping
            || shaped[TableId::EquipmentDestruction.index()]
            || self.metadata.as_ref().is_none_or(|old| {
                old.reads != reads
                    || old.land != self.land
                    || old.drain.supplemental != self.drain.supplemental
                    || old.drain.unresolved != self.drain.unresolved
                    || old.drain.nonempty != self.drain.nonempty
            });
        let metadata = if refresh
            || !self
                .metadata
                .as_ref()
                .is_some_and(|old| Arc::ptr_eq(&old.drain, &self.drain))
        {
            Arc::new(Metadata {
                table: self.status(state)?,
                drain: Arc::clone(&self.drain),
                reads,
                land: Arc::clone(&self.land),
            })
        } else {
            Arc::clone(self.metadata.as_ref().expect("loss metadata"))
        };
        self.metadata = Some(Arc::clone(&metadata));
        self.pending[0] |= mapping || changed[TableId::SunkShips.index()];
        self.pending[1] |= mapping || changed[TableId::SunkConvoys.index()];
        self.pending[2] |= mapping || changed[TableId::LendLeaseEquipment.index()];
        let process = [
            self.pending[0] && reads[1],
            self.pending[1]
                && reads[2]
                && slot_read(self.format, &evidence, TableId::SunkConvoys, hour.hour),
            self.pending[2]
                && reads[3]
                && slot_read(
                    self.format,
                    &evidence,
                    TableId::LendLeaseEquipment,
                    hour.hour,
                ),
        ];
        let mut records = HashMap::new();
        let mut unresolved: [BTreeSet<u32>; 4] = Default::default();
        if process[0] {
            self.ships(state, &mut records, &mut unresolved[0])?;
        }
        if process[1] {
            self.convoys(state, &mut records, &mut unresolved[1])?;
        }
        if process[2] {
            self.lend(state, &mut records, &mut unresolved[2])?;
        }
        if mapping
            || changed[TableId::SliceReads.index()]
            || evidence.table_ns[TableId::SliceReads.index()] != 0
        {
            self.combat(state, hour.hour, &mut records, &mut unresolved[3])?;
        }
        for (pending, processed) in self.pending.iter_mut().zip(process) {
            *pending &= !processed;
        }
        let (names, variants) = self.naming.project(
            state,
            &self.context,
            &self.fields,
            self.format,
            hour.hour,
            whole,
            &shaped,
            &changed,
            slot_read(
                self.format,
                &evidence,
                TableId::VariantDefinitions,
                hour.hour,
            ),
        )?;
        {
            let _cpu = replay_frame::cpu::scope("equipment_losses.columnar.output", None);
            let mut records: Vec<_> = records.into_iter().collect();
            records.sort_unstable_by_key(|item| item.0);
            out.push(Hour {
                branch: hour.branch,
                hour: hour.hour,
                input: Some(Input {
                    format: self.format,
                    identities,
                    complete,
                    membership_complete: columns::complete(state, TableId::DivisionCensus),
                    refresh,
                    metadata,
                    drains: Arc::clone(&self.drains),
                    memberships,
                    memberships_whole,
                    records,
                    unresolved,
                    lend_process: process[2],
                    lend_read: reads[3]
                        && slot_read(
                            self.format,
                            &evidence,
                            TableId::LendLeaseEquipment,
                            hour.hour,
                        ),
                    names,
                    variants,
                }),
            })?;
        }
        self.cold = false;
        Ok(())
    }
}

fn structural(state: &segments::State, table: TableId) -> io::Result<bool> {
    let edits = columns::changes(state, table)?;
    Ok(edits.truncated || !edits.inserted.is_empty() || !edits.deleted.is_empty())
}

fn relevant(state: &segments::State, table: TableId, mask: u64) -> io::Result<bool> {
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

fn single(state: &segments::State, table: TableId) -> io::Result<Option<u32>> {
    if !columns::complete(state, table) {
        return Ok(None);
    }
    let held = columns::table(state, table)?;
    let mut slots = held.live().ones();
    let first = slots.next();
    Ok(if slots.next().is_none() { first } else { None })
}
