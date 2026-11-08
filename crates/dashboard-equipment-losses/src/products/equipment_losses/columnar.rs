use super::*;
use dashboard_base::products::equipment::{
    RawVariant, VariantHour,
    columnar::{Naming, slots},
};
use dashboard_text::metadata::slot_read;
use dashboard_world::{Needs, Roster, World};
use replay::{
    columnar_frames::metrics,
    columns,
    segments::{self, Operator, Sink},
};
use replay_data::{format::FormatVersion, rows::*};
use replay_frame::frame::TableView;
use std::{collections::HashSet, mem::size_of, sync::Arc};

/// What a projection reads of its walk's world: the countries and their cosmetic tags.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        naming: true,
        ..Needs::default()
    }
}

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

#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    pub(super) input: Facts,
}

/// What an item holds of its hour.
#[derive(replay_spool::Spooled)]
pub(super) enum Facts {
    Read(Input),
    /// An hour of another campaign: nothing of it is the branch's.
    Other,
}

#[derive(replay_spool::Spooled)]
pub(super) struct Metadata {
    pub table: TableView,
    pub drain: Arc<DestructionInputs>,
    pub reads: [bool; 4],
    pub land: Arc<CoverageInputs>,
}

/// What an `Arc`'s allocation adds to its value: the strong and weak counts.
pub(super) const ARC: usize = 2 * size_of::<usize>();

/// A conservative size of a `BTreeMap<K, V>` (or set) of `len` entries: whole nodes, each of up
/// to 11 entries and 12 edges, at least half full.
pub(super) fn btree_bytes<K, V>(len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    (len.div_ceil(5) + 1) * (11 * size_of::<(K, V)>() + 14 * size_of::<usize>())
}

impl Metadata {
    /// The bytes it owns, itself and its `Arc` included, apart from its drain and land inputs.
    pub(super) fn heap(&self) -> usize {
        ARC + size_of::<Self>()
            + match &self.table {
                TableView::Unavailable { reason } => reason.capacity(),
                _ => 0,
            }
    }
}

/// One country's reads at an hour. Ships and convoys hand on only counts unlike those the
/// segment last handed on since its last identity change, which every current country took;
/// an equal count changes no country.
#[derive(Default)]
#[derive(replay_spool::Spooled)]
pub(super) struct Records {
    pub ships: Option<Vec<ShipRead>>,
    pub convoys: Vec<ConvoyRead>,
    /// Every pool of the country's exchanges, a giver's aliases' last row standing for a key.
    pub lend: Vec<LendRead>,
    pub combat: Vec<LogRead>,
}

/// An hour's destruction drain, by country slot ascending: each country's amounts by cause and
/// design, a pair repeating where its sum would overflow, or the error its rows hold.
#[derive(Default)]
#[derive(replay_spool::Spooled)]
pub(super) struct Drains {
    /// Each country's slot and the end of its amounts.
    pub slots: Vec<(u32, u32)>,
    pub amounts: Vec<Drained>,
    pub errors: Vec<(u32, String)>,
}

impl Drains {
    /// What `slot` drained: none where it had no rows.
    pub(super) fn of(&self, slot: u32) -> io::Result<&[Drained]> {
        let Ok(at) = self.slots.binary_search_by_key(&slot, |item| item.0) else {
            return Ok(&[]);
        };
        if let Ok(error) = self.errors.binary_search_by_key(&slot, |item| item.0) {
            return Err(io::Error::other(self.errors[error].1.clone()));
        }
        let first = at.checked_sub(1).map_or(0, |before| self.slots[before].1);
        Ok(&self.amounts[first as usize..self.slots[at].1 as usize])
    }

    /// The bytes it owns, itself and its `Arc` included.
    pub(super) fn heap(&self) -> usize {
        ARC + size_of::<Self>()
            + self.slots.capacity() * size_of::<(u32, u32)>()
            + self.amounts.capacity() * size_of::<Drained>()
            + self.errors.capacity() * size_of::<(u32, String)>()
            + self.errors.iter().map(|(_, error)| error.capacity()).sum::<usize>()
    }
}

#[derive(replay_spool::Spooled)]
pub(super) struct Input {
    pub format: FormatVersion,
    pub identities: Option<Vec<((u32, u32), bool)>>,
    pub complete: bool,
    pub membership_complete: bool,
    pub refresh: bool,
    pub metadata: Arc<Metadata>,
    pub drains: Arc<Drains>,
    pub memberships: Vec<(u32, Option<census::Membership>)>,
    pub memberships_whole: bool,
    pub records: Arc<Vec<(u32, Records)>>,
    /// Raw countries the ships, convoys, lend-lease and combat reads left unresolved: an
    /// unchanged set is the one handed on before, which a country takes in once.
    pub unresolved: [Arc<BTreeSet<u32>>; 4],
    pub lend_process: bool,
    pub lend_read: bool,
    pub names: VariantHour,
    pub variants: Vec<(u64, RawVariant)>,
    /// The bytes of the metadata, drains and their inputs made since an item last carried
    /// them: the first to carry them counts them.
    pub shared: usize,
}

/// The bytes `drain` owns, itself and its `Arc` included.
pub(super) fn drain_heap(drain: &DestructionInputs) -> usize {
    ARC + size_of::<DestructionInputs>()
        + drain.supplemental.as_ref().map_or(0, String::capacity)
        + btree_bytes::<u32, ()>(drain.unresolved.len())
}

/// The bytes `land` owns, itself and its `Arc` included.
pub(super) fn land_heap(land: &CoverageInputs) -> usize {
    ARC + size_of::<CoverageInputs>() + btree_bytes::<u32, ()>(land.players.len())
}

impl Records {
    pub(super) fn heap(&self) -> usize {
        let ships = self.ships.as_ref().map_or(0, |ships| {
            ships.capacity() * size_of::<ShipRead>()
                + ships.iter().map(|(key, _)| key.0.capacity()).sum::<usize>()
        });
        ships
            + self.convoys.capacity() * size_of::<ConvoyRead>()
            + self.lend.capacity() * size_of::<LendRead>()
            + self.combat.capacity() * size_of::<LogRead>()
    }
}

struct Fields {
    campaign: usize,
    country_slot: usize,
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
        roster: &Roster,
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
                countries.extend(roster.slot_of(key as u32));
            }
            slots::edits(state, TableId::DivisionCensus, u64::MAX)?
        };
        for slot in visit {
            let raw = held.key(slot).expect("current division census") as u32;
            if let Some(country) = roster.slot_of(raw) {
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
            countries.extend(roster.links().iter().map(|link| link.canonical_slot));
        }
        countries.sort_unstable();
        countries.dedup();
        let mut values = Vec::with_capacity(countries.len());
        for country in countries {
            let (mut grouped, mut ungrouped, mut found) = (0_u64, 0_u64, false);
            for raw in roster.aliases(country) {
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
    census: Census,
    naming: Naming,
    log: slots::Index<u32>,
    dates: slots::Index<i32>,
    log_pending: slots::Index<u32>,
    collisions: slots::Index<u32>,
    pending: [bool; 3],
    /// Each country's ship counts and convoy counts the segment last handed on, since the
    /// last identity change.
    ships_sent: HashMap<(u32, ShipKey), usize>,
    convoys_sent: HashMap<(u32, (u32, i32, u32)), i32>,
    drain: Arc<DestructionInputs>,
    drains: Arc<Drains>,
    land: Arc<CoverageInputs>,
    metadata: Option<Arc<Metadata>>,
    /// The unresolved sets handed on last.
    unresolved: [Arc<BTreeSet<u32>>; 4],
    /// The bytes of the shared inputs made since an item last carried them.
    charged: usize,
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
            census: Census::default(),
            naming: Naming::new()?,
            log: slots::Index::default(),
            dates: slots::Index::default(),
            log_pending: slots::Index::default(),
            collisions: slots::Index::default(),
            pending: [false; 3],
            ships_sent: HashMap::new(),
            convoys_sent: HashMap::new(),
            drain: Arc::default(),
            drains: Arc::default(),
            land: Arc::default(),
            metadata: None,
            unresolved: Default::default(),
            charged: 0,
            cold: true,
            #[cfg(test)]
            log_reads: 0,
        })
    }

    #[cfg(test)]
    pub(super) fn naming_reads(&self) -> usize {
        self.naming.reads()
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

    fn coverage(&self, state: &segments::State, roster: &Roster) -> io::Result<CoverageInputs> {
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
                if let Some(country) = roster.slot_of(raw) {
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

    fn destruction(&mut self, state: &segments::State, roster: &Roster) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("equipment_losses.columnar.drains", None);
        let mut inputs = DestructionInputs::default();
        let mut countries: HashMap<u32, (Vec<Drained>, Option<(u128, String)>)> = HashMap::new();
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
                let Some(country) = roster.slot_of(row.country) else {
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
                    events.push((row.cause, (row.variant >> 32) as u32, row.amount_raw));
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
        let mut countries: Vec<_> = countries.into_iter().collect();
        countries.sort_unstable_by_key(|item| item.0);
        let mut drains = Drains::default();
        for (country, (mut events, error)) in countries {
            events.sort_unstable_by_key(|event| (event.0, event.1));
            let first = drains.amounts.len();
            for (cause, variant, amount) in events {
                let summed = drains.amounts[first..]
                    .last_mut()
                    .filter(|last| (last.0, last.1) == (cause, variant))
                    .and_then(|last| {
                        last.2 = last.2.checked_add(amount)?;
                        Some(())
                    });
                if summed.is_none() {
                    drains.amounts.push((cause, variant, amount));
                }
            }
            let end = u32::try_from(drains.amounts.len())
                .map_err(|_| io::Error::other("An hour's destruction drain is too long"))?;
            drains.slots.push((country, end));
            if let Some((_, error)) = error {
                drains.errors.push((country, error));
            }
        }
        self.drain = Arc::new(inputs);
        self.drains = Arc::new(drains);
        self.charged += drain_heap(&self.drain) + self.drains.heap();
        Ok(())
    }

    fn collision(&mut self, state: &segments::State, roster: &Roster, key: u128) -> io::Result<()> {
        let raw = (key >> 88) as u32;
        let Some(country) = roster.slot_of(raw) else {
            return Ok(());
        };
        let aliases = roster.aliases(country);
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
        roster: &Roster,
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
            let linked: BTreeSet<u32> = roster
                .links()
                .iter()
                .map(|link| link.canonical_slot)
                .collect();
            for aliases in linked
                .into_iter()
                .map(|slot| roster.aliases(slot))
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
            self.collision(state, roster, key)?;
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
        roster: &Roster,
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
            let Some(country) = roster.slot_of(raw) else {
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
                let read = if held.recorded(self.fields.log_amount, slot) {
                    let key = held.key(slot).expect("supplying combat key");
                    (
                        (
                            (key >> 56) as u32,
                            (key >> 48) as u8,
                            (key >> 32) as u16,
                            key as u32,
                        ),
                        amounts[slot as usize],
                    )
                } else {
                    let row: CombatLogEquipmentRow = columns::row(
                        state,
                        TableId::CombatLogEquipment,
                        slot,
                        self.format.version(),
                    )?;
                    ((row.group, row.kind, row.month, row.variant), row.amount_raw)
                };
                records.entry(country).or_default().combat.push(read);
            }
        }
        Ok(())
    }

    /// Hands on each country's ship keys whose count differs from the segment's read before.
    fn ships(
        &mut self,
        state: &segments::State,
        roster: &Roster,
        records: &mut HashMap<u32, Records>,
        unresolved: &mut BTreeSet<u32>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("equipment_losses.columnar.ships", None);
        let slot = single(state, TableId::SunkShips)?.expect("complete sinking read");
        let row: SunkShipsRow =
            columns::row(state, TableId::SunkShips, slot, self.format.version())?;
        let mut counts: HashMap<(u32, ShipKey), (usize, i32, u64)> = HashMap::new();
        for entry in row.entries {
            if let Some(country) = roster.slot_of(entry.victim_country) {
                let key = (
                    entry.ship_name,
                    entry.sink_hour,
                    entry.victim_equipment_variant,
                    entry.battle,
                    entry.damage_source_reference,
                    entry.cause_record,
                );
                counts
                    .entry((country, key))
                    .or_insert((0, entry.sink_hour, entry.victim_equipment_variant))
                    .0 += 1;
            } else {
                unresolved.insert(entry.victim_country);
            }
        }
        let mut sent = HashMap::with_capacity(counts.len());
        for ((country, key), counted) in counts {
            let ships = records
                .entry(country)
                .or_default()
                .ships
                .get_or_insert_with(Vec::new);
            let key = (country, key);
            if self.ships_sent.get(&key) != Some(&counted.0) {
                ships.push((key.1.clone(), counted));
            }
            sent.insert(key, counted.0);
        }
        for own in records.values_mut() {
            if let Some(ships) = &mut own.ships {
                ships.sort_unstable_by(|a, b| a.0.cmp(&b.0));
            }
        }
        self.ships_sent = sent;
        Ok(())
    }

    /// Hands on each country's convoy counts that differ from the segment's read before.
    fn convoys(
        &mut self,
        state: &segments::State,
        roster: &Roster,
        records: &mut HashMap<u32, Records>,
        unresolved: &mut BTreeSet<u32>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("equipment_losses.columnar.convoys", None);
        let table = TableId::SunkConvoys;
        let mut sent = HashMap::new();
        for slot in columns::table(state, table)?.live().ones() {
            let row: SunkConvoyRow = columns::row(state, table, slot, self.format.version())?;
            if let Some(country) = roster.slot_of(row.owner_country) {
                let key = (row.owner_country, row.month, row.killer_country);
                let own = records.entry(country).or_default();
                if self.convoys_sent.get(&(country, key)) != Some(&row.convoys) {
                    own.convoys.push((key, row.convoys));
                }
                sent.insert((country, key), row.convoys);
            } else {
                unresolved.insert(row.owner_country);
            }
        }
        self.convoys_sent = sent;
        Ok(())
    }

    fn lend(
        &self,
        state: &segments::State,
        roster: &Roster,
        records: &mut HashMap<u32, Records>,
        unresolved: &mut BTreeSet<u32>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("equipment_losses.columnar.lend", None);
        let table = TableId::LendLeaseEquipment;
        let mut rows: HashMap<u32, Vec<LendLeaseEquipmentRow>> = HashMap::new();
        for slot in columns::table(state, table)?.live().ones() {
            let row: LendLeaseEquipmentRow =
                columns::row(state, table, slot, self.format.version())?;
            if let Some(country) = roster.slot_of(row.giver) {
                rows.entry(country).or_default().push(row);
            } else {
                unresolved.insert(row.giver);
            }
        }
        for (country, mut rows) in rows {
            rows.sort_unstable_by_key(RowCodec::key);
            records.entry(country).or_default().lend = lend_amounts(&rows);
        }
        Ok(())
    }
}

impl Operator<World> for Projector {
    type Output = Hour;

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("equipment_losses.columnar", Some("equipment_losses"));
        let state = hour.state;
        let (roster, names) = (hour.world.countries()?, hour.world.naming()?);
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
                        input: Facts::Other,
                    });
                }
            }
        }
        let mapping = whole
            || shaped[TableId::Countries.index()]
            || changed[TableId::CountryLinks.index()]
            || relevant(state, TableId::Countries, 1_u64 << self.fields.country_slot)?;
        // Each country's identity in id order, and whether the links resolve it.
        let identities = mapping.then(|| {
            roster
                .iter()
                .filter(|_| roster.complete())
                .map(|country| ((country.slot, country.id), country.resolved))
                .collect()
        });
        if mapping {
            self.ships_sent.clear();
            self.convoys_sent.clear();
        }
        let complete = columns::complete(state, TableId::Countries)
            && columns::complete(state, TableId::CountryLinks);
        let memberships_whole = mapping || shaped[TableId::DivisionCensus.index()];
        let memberships;
        {
            let _cpu = replay_frame::cpu::scope("equipment_losses.columnar.census", None);
            memberships = self.census.project(
                state,
                roster,
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
            self.land = Arc::new(self.coverage(state, roster)?);
            self.charged += land_heap(&self.land);
        }
        if mapping || changed[TableId::EquipmentDestruction.index()] {
            self.destruction(state, roster)?;
        }
        self.indexes(state, roster, whole, mapping, &shaped)?;
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
            let metadata = Arc::new(Metadata {
                table: self.status(state)?,
                drain: Arc::clone(&self.drain),
                reads,
                land: Arc::clone(&self.land),
            });
            self.charged += metadata.heap();
            metadata
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
            self.ships(state, roster, &mut records, &mut unresolved[0])?;
        }
        if process[1] {
            self.convoys(state, roster, &mut records, &mut unresolved[1])?;
        }
        if process[2] {
            self.lend(state, roster, &mut records, &mut unresolved[2])?;
        }
        if mapping
            || changed[TableId::SliceReads.index()]
            || evidence.table_ns[TableId::SliceReads.index()] != 0
        {
            self.combat(state, roster, hour.hour, &mut records, &mut unresolved[3])?;
        }
        for (pending, processed) in self.pending.iter_mut().zip(process) {
            *pending &= !processed;
        }
        let (names, variants) = self.naming.project(
            state,
            (roster, names),
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
            for (held, read) in self.unresolved.iter_mut().zip(unresolved) {
                if **held != read {
                    *held = Arc::new(read);
                }
            }
            let facts = Facts::Read(Input {
                    format: self.format,
                    identities,
                    complete,
                    membership_complete: columns::complete(state, TableId::DivisionCensus),
                    refresh,
                    metadata,
                    drains: Arc::clone(&self.drains),
                    memberships,
                    memberships_whole,
                    records: Arc::new(records),
                    unresolved: self.unresolved.clone(),
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
                    shared: std::mem::take(&mut self.charged),
                });
            self.cold = false;
            out.push(Hour {
                branch: hour.branch,
                hour: hour.hour,
                input: facts,
            })?;
        }
        Ok(())
    }
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
