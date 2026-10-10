//! Bilateral war relations; all field offsets use the primary object base.
//!
//! Saves contain bilateral fields and goal references. Recognized active and
//! previous-war blocks are skipped on load and provide no separate war roster.
//! Overview groups are temporary pairs of country arrays derived for one viewing
//! country. They have no stored identity, name, start/end or participant join dates.
//! Their sides are not intrinsically attacking and defending sides. A representative
//! bilateral relation supplies an instigator and content-dependent naming operands.
//!
//! Recordings retain the operands in the overview reader. Group continuity and
//! observation history belong to the app; this reader captures no persistent war object.

use crate::{
    Memory, ccountry, cdiplomacy_status,
    cpdx_array::Array,
    crelation, crelation_status,
    memory::{Raw, RecordField, RecordLayout, RecordSources},
    read,
    walk::Outcome,
};

pub mod overview;

/// Ordered goal handles for each stored endpoint.
pub const GOALS: [usize; 2] = [0x168, 0x180];
/// Installed default naming-prefix location.
pub const NAME_PREFIX_RVA: usize = 0x0294_a860;
/// Installed naming-prefix byte count before its terminating NUL.
pub const NAME_PREFIX_LEN: usize = 8;

/// CWarRelation primary vtable.
pub const VTABLE_RVA: usize = 0x0294_9798;
/// CWarRelation native type word.
pub const TYPE: u32 = 0x380a;
/// CWarRelation first known casualties, signed i64 integer.
pub const FIRST_CASUALTIES: usize = 0x50;
/// CWarRelation second known casualties, signed i64 integer.
pub const SECOND_CASUALTIES: usize = 0x58;
/// CWarRelation first unknown casualties, signed i64 integer.
pub const FIRST_UNKNOWN_CASUALTIES: usize = 0x60;
/// CWarRelation second unknown casualties, signed i64 integer.
pub const SECOND_UNKNOWN_CASUALTIES: usize = 0x68;
/// First echoed raw country reference, u32.
pub const ECHO_FIRST_COUNTRY: usize = 0x78;
/// Second echoed raw country reference, u32.
pub const ECHO_SECOND_COUNTRY: usize = 0x7c;
/// Threat, signed i64 with denominator 100,000.
pub const THREAT: usize = 0x150;
/// CWarRelation first-was-instigator byte.
pub const FIRST_WAS_INSTIGATOR: usize = 0x158;
/// Defender hostility-reason enum, u32.
pub const DEFENDER_HOSTILITY_REASON: usize = 0x15c;
/// Instigator hostility-reason enum, u32.
pub const INSTIGATOR_HOSTILITY_REASON: usize = 0x160;
/// CWarRelation threat fixed-point denominator.
pub const THREAT_SCALE: i64 = 100_000;
/// CWarRelation hostility discriminator for war.
pub const HOSTILITY_WAR: u32 = 0;
/// CWarRelation hostility discriminator for puppet.
pub const HOSTILITY_PUPPET: u32 = 1;
/// CWarRelation hostility discriminator for ally.
pub const HOSTILITY_ALLY: u32 = 2;
/// CWarRelation hostility discriminator for asked-to-join.
pub const HOSTILITY_ASKED_TO_JOIN: u32 = 3;
/// CWarRelation hostility discriminator for guarantee.
pub const HOSTILITY_GUARANTEE: u32 = 4;
/// CWarRelation hostility discriminator for not-applicable.
pub const HOSTILITY_NOT_APPLICABLE: u32 = 5;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Stored endpoint order is native; the pair is not a registered war handle.
    pub first_country: u32,
    pub second_country: u32,
    pub start_hour: i32,
    pub end_hour: i32,
    pub cancel: u8,
    pub first_casualties: i64,
    pub second_casualties: i64,
    pub first_unknown_casualties: i64,
    pub second_unknown_casualties: i64,
    pub first_was_instigator: u8,
}

/// Enumerates each canonical country's owned relations and emits active wars
/// once, from their first endpoint. Both endpoints must contain the same pointer
/// exactly once, with matching matrix and native ownership cross-links.
/// No pointers escape the read; native country reference order and bytes remain.
/// Without caller-owned storage every country and diplomacy occurrence is
/// authenticated with the original uncached readers.
pub fn walk(memory: &impl Memory, base: usize, game_state: usize, output: &mut [Row]) -> Outcome {
    walk_from(
        memory,
        memory,
        base,
        game_state,
        crate::walk::Buffer::Fixed(output),
        &mut crelation_status::Resolver::default(),
        None,
    )
}

/// Owned-vector acquisition and successful endpoint-status validation within
/// the quiescent callback. Every new pass discards the retained results.
/// Country resolution uses the same callback's authentication view, retaining
/// cached native-id aliases while payload reads bypass unrelated cache probes.
pub fn walk_owned(
    memory: &impl Memory,
    country_memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    scratch: &mut OwnedSnapshotScratch,
) -> Outcome {
    walk_owned_buffer(
        memory,
        country_memory,
        base,
        game_state,
        crate::walk::Buffer::Fixed(output),
        scratch,
    )
}

pub fn walk_owned_growing(
    memory: &impl Memory,
    country_memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    scratch: &mut OwnedSnapshotScratch,
) -> Outcome {
    walk_owned_buffer(
        memory,
        country_memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(output),
        scratch,
    )
}

/// Original owned-vector counts bound filtered war rows without reading values.
pub fn row_demand(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<usize, &'static str> {
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    let fields = Fields { memory, base };
    let mut required = 0_usize;
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        let diplomacy = fields.pointer(country.pointer, ccountry::DIPLOMACY_STATUS)?;
        if diplomacy == 0 {
            return Err("war demand diplomacy is null");
        }
        fields.vtable(
            diplomacy,
            cdiplomacy_status::VTABLE_RVA,
            "war demand diplomacy vtable mismatch",
        )?;
        if fields.field::<u64>(diplomacy, cdiplomacy_status::COUNTRY)? != country.pointer as u64 {
            return Err("war demand diplomacy country mismatch");
        }
        // Overview grouping consumes this owned vector.
        let relations = diplomacy
            .checked_add(cdiplomacy_status::OWNED_RELATIONS)
            .and_then(|address| Array::read(memory, address, 8))
            .ok_or("war demand owned vector invalid")?;
        required = required
            .checked_add(relations.count as usize)
            .ok_or("war demand row count overflow")?;
    }
    Ok(required)
}

fn walk_owned_buffer(
    memory: &impl Memory,
    country_memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: crate::walk::Buffer<'_, Row>,
    scratch: &mut OwnedSnapshotScratch,
) -> Outcome {
    scratch.begin();
    scratch.worker_growth(output.is_growing());
    // Moving the vectors out and back keeps the scratch one borrow for the
    // membership snapshot; no storage is allocated or freed.
    let mut resolver = std::mem::take(&mut scratch.resolver);
    let outcome = walk_from(
        memory,
        country_memory,
        base,
        game_state,
        output,
        &mut resolver,
        Some(&mut *scratch),
    );
    scratch.resolver = resolver;
    outcome
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct OwnedSnapshotStats {
    pub queries: usize,
    pub hits: usize,
    pub installed: usize,
    pub fallbacks: usize,
    /// Header attempts, including checked-address/validation failures.
    pub acquisition_headers: usize,
    /// Completed nonnull values consumed by membership scans, not native read
    /// attempts: a PointerBatch may already have read its unconsumed suffix.
    pub acquisition_words: usize,
    pub required_vectors: usize,
    pub required_words: usize,
    pub required_cells: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct OwnedSnapshotCapacity {
    pub vectors: usize,
    pub root_slots: usize,
    pub root_capacity: usize,
    pub words: usize,
    pub initialized_count_cells: usize,
    pub count_capacity: usize,
    pub allocated_bytes: usize,
}

#[derive(Clone, Copy, Default)]
struct OwnedVector {
    root: usize,
    header: [u8; 16],
    cells: usize,
    cell_count: usize,
    wanted_words: usize,
    wanted_cells: usize,
    captured: bool,
}

#[derive(Clone, Copy, Default)]
struct PointerCount {
    pointer: usize,
    count: u32,
}

/// Failure reasons of one owned-membership scan, so each relation walker keeps
/// its own original messages while sharing the snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MembershipReasons {
    pub vector: &'static str,
    pub pointer: &'static str,
    pub null: &'static str,
    pub mismatch: &'static str,
    pub changed: &'static str,
}

pub(crate) const WAR_MEMBERSHIP: MembershipReasons = MembershipReasons {
    vector: "war ownership vector unreadable or invalid",
    pointer: "war ownership pointer unreadable",
    null: "null war ownership entry",
    mismatch: "war ownership cross-link count mismatch",
    changed: "war ownership snapshot changed or became unreadable",
};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct StatusGraph {
    war: usize,
    endpoints: [u32; 2],
    canonical: [u32; 2],
    diplomacies: [usize; 2],
}

/// Pooled initialized heap storage. Worker acquisitions may also grow from
/// their original membership headers; fixed callback readers never allocate.
/// Keys and logical snapshots are discarded at every optimized pass.
/// Default owns no heap storage, allowing allocation-free ownership transfers.
///
/// Every size is taken from what a walk observed: the live country census and
/// link headers, the owned vectors' counts and the wars validated. There is no
/// chosen country, word or war budget. Fixed storage keeps the original complete
/// scan for an uncovered vector. Worker storage
/// grows before that original scan; optional status-graph cache misses retain
/// the complete native validation and never limit recorded rows.
#[derive(Default)]
pub struct OwnedSnapshotScratch {
    worker_growth: bool,
    vectors: Vec<OwnedVector>,
    roots: Vec<usize>,
    words: Vec<usize>,
    counts: Vec<PointerCount>,
    status_graphs: Vec<StatusGraph>,
    used_cells: usize,
    stats: OwnedSnapshotStats,
    /// Status graphs freshly validated by the last pass; at most two per war.
    required_graphs: usize,
    /// Words and count cells of every country's owned-relation header seen by
    /// the walk's outer loop. Unlike `required_*`, which needs a vector slot
    /// to deduplicate a root, this total needs no storage, so one pass is
    /// enough to size every later snapshot, however small the storage was.
    observed_words: usize,
    observed_cells: usize,
    /// Words consumed by complete native rescans when optional storage is short.
    rescanned_words: usize,
    /// Dense country authentication cells, sized from the observed headers.
    pub(crate) resolver: crelation_status::Resolver,
}

/// Power-of-two open-addressing table with a load factor of at most one half.
fn open_table_slots(entries: usize) -> std::io::Result<usize> {
    if entries == 0 {
        return Ok(0);
    }
    entries
        .checked_mul(2)
        .and_then(usize::checked_next_power_of_two)
        .ok_or_else(crate::allocation::oom)
}

/// Grow initialized storage to `len`, reporting allocation failure.
fn resize_to<T: Copy>(cells: &mut Vec<T>, len: usize, value: T) -> std::io::Result<()> {
    if cells.len() < len {
        crate::allocation::reserve_exact(cells, len - cells.len())?;
        cells.resize_with(len, || value);
    }
    Ok(())
}

impl OwnedSnapshotScratch {
    pub fn try_with_capacity(countries: usize) -> std::io::Result<Self> {
        let slots = open_table_slots(countries)?;
        Ok(Self {
            worker_growth: false,
            vectors: crate::allocation::vec(countries)?,
            roots: crate::allocation::filled(slots, usize::MAX)?,
            words: Vec::new(),
            counts: Vec::new(),
            status_graphs: crate::allocation::filled(slots, StatusGraph::default())?,
            used_cells: 0,
            stats: OwnedSnapshotStats::default(),
            required_graphs: 0,
            observed_words: 0,
            observed_cells: 0,
            rescanned_words: 0,
            resolver: crelation_status::Resolver::try_with_capacity(countries)?,
        })
    }

    #[cfg(test)]
    pub fn with_capacity(countries: usize) -> Self {
        Self::try_with_capacity(countries).unwrap()
    }

    pub fn stats(&self) -> OwnedSnapshotStats {
        self.stats
    }

    /// Current physical capacities, not requested limits or multiplied pool
    /// estimates. Only initialized root/count slots are used for admission.
    pub fn capacity(&self) -> OwnedSnapshotCapacity {
        OwnedSnapshotCapacity {
            vectors: self.vectors.capacity(),
            root_slots: self.roots.len(),
            root_capacity: self.roots.capacity(),
            words: self.words.capacity(),
            initialized_count_cells: self.counts.len(),
            count_capacity: self.counts.capacity(),
            allocated_bytes: self.allocated_bytes(),
        }
    }

    /// Heap payload only; the caller accounts the inline scratch object.
    pub fn allocated_bytes(&self) -> usize {
        self.vectors.capacity() * std::mem::size_of::<OwnedVector>()
            + self.roots.capacity() * std::mem::size_of::<usize>()
            + self.words.capacity() * std::mem::size_of::<usize>()
            + self.counts.capacity() * std::mem::size_of::<PointerCount>()
            + self.status_graphs.capacity() * std::mem::size_of::<StatusGraph>()
            + self.resolver.allocated_bytes()
    }

    /// Called only while the spool owns the containing frame. `countries` is
    /// the owner's roster capacity; the census header, owned-vector counts and
    /// validated wars observed by the last pass raise every size further.
    /// Allocation failure is reported before admitting another pass.
    pub fn grow(&mut self, countries: usize) -> std::io::Result<()> {
        self.resolver.grow()?;
        // Every owned-vector root is one country's diplomacy object.
        let vectors = countries
            .max(self.resolver.required()[1])
            .max(self.stats.required_vectors);
        let additional = vectors.saturating_sub(self.vectors.len());
        crate::allocation::reserve_exact(&mut self.vectors, additional)?;
        resize_to(&mut self.roots, open_table_slots(vectors)?, usize::MAX)?;
        resize_to(
            &mut self.status_graphs,
            open_table_slots(self.required_graphs)?,
            StatusGraph::default(),
        )?;
        let words = self.stats.required_words.max(self.observed_words);
        if self.words.capacity() < words {
            let additional = words.saturating_sub(self.words.len());
            crate::allocation::reserve_exact(&mut self.words, additional)?;
        }
        resize_to(
            &mut self.counts,
            self.stats.required_cells.max(self.observed_cells),
            PointerCount::default(),
        )?;
        Ok(())
    }

    /// Allocating acquisitions enable this before the original membership walk.
    /// Fixed callback storage keeps its bounded fallback and shortage outcomes.
    pub(crate) fn worker_growth(&mut self, enabled: bool) {
        self.worker_growth = enabled;
    }

    fn grow_membership(&mut self, words: usize, cells: usize) -> Result<(), &'static str> {
        let required = self
            .vectors
            .len()
            .checked_add(1)
            .ok_or("owned membership vector demand overflow")?;
        crate::allocation::reserve(&mut self.vectors, 1)
            .map_err(|_| "owned membership vector allocation failed")?;
        let slots =
            open_table_slots(required).map_err(|_| "owned membership root demand overflow")?;
        if self.roots.len() < slots {
            resize_to(&mut self.roots, slots, usize::MAX)
                .map_err(|_| "owned membership root allocation failed")?;
            self.roots.fill(usize::MAX);
            // Resizing changes the probe mask: reindex the already acquired roots.
            for index in 0..self.vectors.len() {
                let slot = self
                    .root_slot(self.vectors[index].root)
                    .ok_or("owned membership root index exhausted")?;
                self.roots[slot] = index;
            }
        }
        crate::allocation::reserve(&mut self.words, words)
            .map_err(|_| "owned membership word allocation failed")?;
        let required = self
            .used_cells
            .checked_add(cells)
            .ok_or("owned membership count demand overflow")?;
        resize_to(&mut self.counts, required, PointerCount::default())
            .map_err(|_| "owned membership count allocation failed")?;
        Ok(())
    }

    pub(crate) fn begin(&mut self) {
        self.vectors.clear();
        self.roots.fill(usize::MAX);
        self.words.clear();
        self.status_graphs.fill(StatusGraph::default());
        self.used_cells = 0;
        self.stats = OwnedSnapshotStats::default();
        self.required_graphs = 0;
        self.observed_words = 0;
        self.observed_cells = 0;
        self.rescanned_words = 0;
    }

    /// Records one country's owned-relation header count, as read by the
    /// walk's outer loop, toward the next pass's storage. Only vectors the
    /// snapshot would install (more than one entry) need count cells.
    pub(crate) fn observe_owned(&mut self, count: u32) {
        let words = count as usize;
        self.observed_words = self.observed_words.saturating_add(words);
        if words > 1 {
            self.observed_cells = self.observed_cells.saturating_add(
                words
                    .checked_mul(2)
                    .and_then(usize::checked_next_power_of_two)
                    .unwrap_or(usize::MAX),
            );
        }
    }

    fn hash(value: usize) -> usize {
        let mut value = value as u64;
        value ^= value >> 30;
        value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value ^= value >> 27;
        value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
        (value ^ (value >> 31)) as usize
    }

    fn root_slot(&self, root: usize) -> Option<usize> {
        if self.roots.is_empty() {
            return None;
        }
        let mask = self.roots.len() - 1;
        let first = Self::hash(root) & mask;
        (0..self.roots.len())
            .map(|step| (first + step) & mask)
            .find(|&slot| {
                let index = self.roots[slot];
                index == usize::MAX || self.vectors[index].root == root
            })
    }

    fn status_graph_slot(&self, war: usize) -> Option<usize> {
        if self.status_graphs.is_empty() {
            return None;
        }
        let mask = self.status_graphs.len() - 1;
        let first = Self::hash(war) & mask;
        (0..self.status_graphs.len())
            .map(|step| (first + step) & mask)
            .find(|&slot| {
                let previous = self.status_graphs[slot].war;
                previous == 0 || previous == war
            })
    }

    fn count(&self, vector: OwnedVector, pointer: usize) -> u32 {
        let mask = vector.cell_count - 1;
        let first = Self::hash(pointer) & mask;
        for step in 0..vector.cell_count {
            let cell = self.counts[vector.cells + ((first + step) & mask)];
            if cell.count == 0 {
                return 0;
            }
            if cell.pointer == pointer {
                return cell.count;
            }
        }
        0
    }

    fn occurrence(
        &mut self,
        memory: &impl Memory,
        root: usize,
        war: usize,
    ) -> Result<(), &'static str> {
        self.membership(memory, root, war, WAR_MEMBERSHIP)
    }

    /// Checks that `root`'s owned-relation vector holds `war` exactly once.
    /// The first query of a root performs the original complete scan and, when
    /// storage allows, installs a counted snapshot; later queries compare the
    /// fresh header with the snapshot and look the pointer up in O(1).
    pub(crate) fn membership(
        &mut self,
        memory: &impl Memory,
        root: usize,
        war: usize,
        reasons: MembershipReasons,
    ) -> Result<(), &'static str> {
        self.stats.queries += 1;
        self.stats.acquisition_headers += 1;
        let header = root
            .checked_add(cdiplomacy_status::OWNED_RELATIONS)
            .and_then(|address| read::<[u8; 16]>(memory, address));
        let Some((header, array)) =
            header.and_then(|header| Array::from_header(header, 8).map(|array| (header, array)))
        else {
            return Err(reasons.vector);
        };
        let existing = self.root_slot(root).and_then(|slot| {
            let index = self.roots[slot];
            (index != usize::MAX).then_some(index)
        });
        if let Some(index) = existing
            && self.vectors[index].captured
        {
            if self.vectors[index].header != header {
                return Err(reasons.changed);
            }
            self.stats.hits += 1;
            return expected_occurrence(self.count(self.vectors[index], war), 1, reasons);
        }
        let wanted_words = array.count as usize;
        let wanted_cells = wanted_words
            .max(1)
            .checked_mul(2)
            .and_then(usize::checked_next_power_of_two)
            .unwrap_or(usize::MAX);
        if self.worker_growth && wanted_words > 1 {
            self.grow_membership(wanted_words, wanted_cells)?;
        }
        let root_slot = self.root_slot(root);
        let index = existing.or_else(|| {
            if self.vectors.len() == self.vectors.capacity() {
                return None;
            }
            let slot = root_slot?;
            let index = self.vectors.len();
            self.vectors.push(OwnedVector {
                root,
                ..OwnedVector::default()
            });
            self.roots[slot] = index;
            Some(index)
        });
        self.stats.required_vectors = self
            .stats
            .required_vectors
            .max(self.vectors.len() + usize::from(index.is_none()));
        if let Some(index) = index {
            let entry = &mut self.vectors[index];
            self.stats.required_words = self
                .stats
                .required_words
                .saturating_add(wanted_words.saturating_sub(entry.wanted_words));
            self.stats.required_cells = self
                .stats
                .required_cells
                .saturating_add(wanted_cells.saturating_sub(entry.wanted_cells));
            entry.wanted_words = entry.wanted_words.max(wanted_words);
            entry.wanted_cells = entry.wanted_cells.max(wanted_cells);
        }
        let can_capture = index.is_some()
            && wanted_words > 1
            && wanted_words <= self.words.capacity().saturating_sub(self.words.len())
            && wanted_cells <= self.counts.len().saturating_sub(self.used_cells);
        if !can_capture {
            self.stats.fallbacks += 1;
            // Zero- and one-entry vectors cost the same scanned or snapshotted.
            if wanted_words > 1 {
                self.rescanned_words = self.rescanned_words.saturating_add(wanted_words);
            }
            return scan_occurrence(memory, array, war, 1, reasons, |_| {
                self.stats.acquisition_words += 1
            });
        }
        let start = self.words.len();
        // Complete count/null/fault semantics are still performed at this first
        // original observation. A failed scan never publishes partial storage.
        let result = scan_occurrence(memory, array, war, 1, reasons, |pointer| {
            self.stats.acquisition_words += 1;
            self.words.push(pointer);
        });
        if result.is_err() {
            self.words.truncate(start);
            return result;
        }
        let cells = self.used_cells;
        self.counts[cells..cells + wanted_cells].fill(PointerCount::default());
        let mask = wanted_cells - 1;
        for &pointer in &self.words[start..] {
            let first = Self::hash(pointer) & mask;
            for step in 0..wanted_cells {
                let cell = &mut self.counts[cells + ((first + step) & mask)];
                if cell.count == 0 || cell.pointer == pointer {
                    cell.pointer = pointer;
                    cell.count += 1;
                    break;
                }
            }
        }
        self.used_cells += wanted_cells;
        let entry = &mut self.vectors[index.unwrap()];
        entry.header = header;
        entry.cells = cells;
        entry.cell_count = wanted_cells;
        entry.captured = true;
        self.stats.installed += 1;
        Ok(())
    }
}

fn walk_from(
    memory: &impl Memory,
    country_memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: crate::walk::Buffer<'_, Row>,
    resolver: &mut crelation_status::Resolver,
    ownership: Option<&mut OwnedSnapshotScratch>,
) -> Outcome {
    let result = (|| {
        let countries = ccountry::Reader::new(country_memory, base, game_state)?;
        resolver.begin(&countries);
        if output.is_growing() {
            resolver
                .grow()
                .map_err(|_| "war resolver allocation failed")?;
        }
        let ownership = ownership;
        let mut reader = Reader {
            memory,
            base,
            countries,
            resolver: &mut *resolver,
            current_matrix: (0, None),
            ownership,
        };
        let mut found_country = false;
        let mut len = 0_usize;
        let mut unread_rows = 0_u32;
        let mut batch = RelationBatch::default();
        for slot in 1..reader.countries.count() {
            let Some(country) = reader.countries.at(slot)? else {
                continue;
            };
            found_country = true;
            let diplomacy = reader.diplomacy(country)?;
            reader.current_matrix = (diplomacy, None);
            let source = reader.array(
                diplomacy,
                cdiplomacy_status::OWNED_RELATIONS,
                "owned relations unreadable or invalid",
            )?;
            // Reserve the owned vector's full remaining population before filtering
            // relation types, preserving overview occurrence demand.
            output.ensure(
                len.checked_add(source.count as usize)
                    .ok_or("war row count overflow")?,
            )?;
            if let Some(scratch) = reader.ownership.as_deref_mut() {
                scratch.observe_owned(source.count);
            }
            for index in 0..source.count {
                let offset = index as usize % BATCH_SIZE;
                if offset == 0 {
                    batch.read(memory, source, index);
                }
                let pointer = batch
                    .pointers
                    .get(offset, "relation root pointer unreadable")?;
                if pointer == 0 {
                    return Err("null owned relation entry");
                }
                if batch.relation_type(offset)? != TYPE {
                    continue;
                }
                let row = match reader.relation(pointer, country) {
                    Ok(Some(row)) => row,
                    Ok(None) => continue,
                    Err(reason) => return Err(reason),
                };
                if len == output.len() {
                    unread_rows = unread_rows
                        .checked_add(1)
                        .ok_or("war unread row count overflow")?;
                } else {
                    output[len] = row;
                    len += 1;
                }
            }
        }
        if !found_country {
            Err("country table has no readable countries")
        } else if unread_rows == 0 {
            Ok(Outcome::Rows { len })
        } else {
            Ok(Outcome::Truncated { len, unread_rows })
        }
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

fn expected_occurrence(
    found: u32,
    expected: u32,
    reasons: MembershipReasons,
) -> Result<(), &'static str> {
    if found == expected {
        Ok(())
    } else {
        Err(reasons.mismatch)
    }
}

/// The original complete scan, also used after an optimization capacity miss.
/// The observer receives completed nonnull values without changing source reads.
pub(crate) fn scan_occurrence(
    memory: &impl Memory,
    array: Array,
    war: usize,
    expected: u32,
    reasons: MembershipReasons,
    mut observed: impl FnMut(usize),
) -> Result<(), &'static str> {
    if array.count <= 1 {
        let found = if array.count == 0 {
            0
        } else {
            let pointer = array
                .element(0, 8)
                .and_then(|address| read::<u64>(memory, address))
                .and_then(|pointer| usize::try_from(pointer).ok())
                .ok_or(reasons.pointer)?;
            if pointer == 0 {
                return Err(reasons.null);
            }
            observed(pointer);
            u32::from(pointer == war)
        };
        return expected_occurrence(found, expected, reasons);
    }
    let mut found = 0_u32;
    let mut batch = PointerBatch::default();
    for index in 0..array.count {
        let offset = index as usize % BATCH_SIZE;
        if offset == 0 {
            batch.read(memory, array, index);
        }
        let pointer = batch.get(offset, reasons.pointer)?;
        if pointer == 0 {
            return Err(reasons.null);
        }
        observed(pointer);
        // Saturation cannot turn a duplicate into an exact expected count.
        found = found.saturating_add(u32::from(pointer == war));
    }
    expected_occurrence(found, expected, reasons)
}

struct Reader<'a, M: Memory, C: Memory = M> {
    memory: &'a M,
    base: usize,
    countries: ccountry::Reader<'a, C>,
    /// Dense country and diplomacy authentication cells; an empty resolver
    /// validates every occurrence with the original uncached readers.
    resolver: &'a mut crelation_status::Resolver,
    // Only the outer country's successful header is reused, within its owned
    // relation loop. The optimized walk also reuses successful status graphs;
    // diplomacy-owned vector guards remain fresh on every occurrence.
    current_matrix: (usize, Option<Array>),
    ownership: Option<&'a mut OwnedSnapshotScratch>,
}

const BATCH_SIZE: usize = 64;

struct PointerBatch {
    values: [[u8; 8]; BATCH_SIZE],
    copied: usize,
    len: usize,
}

impl Default for PointerBatch {
    fn default() -> Self {
        Self {
            values: [[0; 8]; BATCH_SIZE],
            copied: 0,
            len: 0,
        }
    }
}

impl PointerBatch {
    fn read(&mut self, memory: &impl Memory, array: Array, first: u32) {
        self.len = (array.count - first).min(BATCH_SIZE as u32) as usize;
        match self.len {
            0 => self.copied = 0,
            1 => {
                self.copied = usize::from(array.element(first, 8).is_some_and(|address| {
                    crate::memory::read_slice(memory, address, &mut self.values[0])
                }));
            }
            _ => self.copy(memory, array, first),
        }
    }

    fn copy(&mut self, memory: &impl Memory, array: Array, first: u32) {
        const FIELDS: [RecordField; 1] = [RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 8,
        }];
        const LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&FIELDS, 8).expect("valid record layout");
        // Keep each exact pointer observation and its failure prefix without
        // building padded address and destination-descriptor arrays.
        self.copied = memory
            .copy_records(
                RecordSources::Strided {
                    base: array.element(first, 8).unwrap_or(0),
                    stride: 8,
                    count: self.len,
                },
                &LAYOUT,
                &mut self.values.as_flattened_mut()[..self.len * 8],
            )
            .min(self.len);
    }

    fn get(&self, index: usize, reason: &'static str) -> Result<usize, &'static str> {
        if index >= self.copied {
            return Err(reason);
        }
        usize::try_from(u64::from_le_bytes(self.values[index])).map_err(|_| reason)
    }
}

/// The native relation type that filters an owned-relation vector.
const TYPE_FIELD: [crate::memory::RecordField; 1] = [crate::memory::RecordField {
    source_offset: crelation::TYPE_ID,
    destination_offset: 0,
    len: 4,
}];
const TYPE_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&TYPE_FIELD, 4).expect("valid record layout");

struct RelationBatch {
    pointers: PointerBatch,
    types: [u8; BATCH_SIZE * 4],
    types_copied: usize,
}

impl Default for RelationBatch {
    fn default() -> Self {
        Self {
            pointers: PointerBatch::default(),
            types: [0; BATCH_SIZE * 4],
            types_copied: 0,
        }
    }
}

impl RelationBatch {
    fn read(&mut self, memory: &impl Memory, array: Array, first: u32) {
        self.pointers.read(memory, array, first);
        // Only readable, nonnull pointers in this batch can supply a type field.
        let entries = (0..self.pointers.copied)
            .take_while(|&index| self.pointers.get(index, "").is_ok_and(|value| value != 0))
            .count();
        let mut addresses = [0_usize; BATCH_SIZE];
        for (slot, address) in addresses[..entries].iter_mut().enumerate() {
            *address = u64::from_le_bytes(self.pointers.values[slot]) as usize;
        }
        self.types_copied = memory.copy_records(
            RecordSources::Pointers(&addresses[..entries]),
            &TYPE_LAYOUT,
            &mut self.types[..entries * 4],
        );
    }

    fn relation_type(&self, index: usize) -> Result<u32, &'static str> {
        if index >= self.types_copied {
            return Err("war relation field unreadable or address overflow");
        }
        Ok(u32::from_le_bytes(
            self.types[index * 4..index * 4 + 4].try_into().unwrap(),
        ))
    }
}

// Call only after pair_status authenticated this fresh status. These hints
// cover two exact future headers, never the pointer values or array contents.
#[inline]
fn hint_owned_status_headers(status: usize) {
    for offset in [
        crelation_status::SUBRELATIONS,
        crelation_status::SERIALIZABLE_RELATIONS,
    ] {
        if let Some(address) = status.checked_add(offset)
            && address.checked_add(16).is_some()
        {
            crate::prefetch::span(address, 16);
        }
    }
}

/// Guarded scalar reads shared by the reader and its cached validators.
struct Fields<'a, M: Memory> {
    memory: &'a M,
    base: usize,
}

impl<M: Memory> Fields<'_, M> {
    fn field<T: Raw>(&self, pointer: usize, offset: usize) -> Result<T, &'static str> {
        pointer
            .checked_add(offset)
            .and_then(|address| read::<T>(self.memory, address))
            .ok_or("war relation field unreadable or address overflow")
    }

    fn pointer(&self, pointer: usize, offset: usize) -> Result<usize, &'static str> {
        usize::try_from(self.field::<u64>(pointer, offset)?)
            .map_err(|_| "war relation pointer overflow")
    }

    fn vtable(&self, pointer: usize, rva: usize, reason: &'static str) -> Result<(), &'static str> {
        let expected = self
            .base
            .checked_add(rva)
            .ok_or("module address overflow")?;
        if read::<u64>(self.memory, pointer) == Some(expected as u64) {
            Ok(())
        } else {
            Err(reason)
        }
    }
}

impl<M: Memory, C: Memory> Reader<'_, M, C> {
    fn resolve(&mut self, raw: u32) -> Result<ccountry::Country, &'static str> {
        self.resolver.resolve(&self.countries, raw)
    }

    fn field<T: Raw>(&self, pointer: usize, offset: usize) -> Result<T, &'static str> {
        Fields {
            memory: self.memory,
            base: self.base,
        }
        .field(pointer, offset)
    }

    fn array(
        &self,
        pointer: usize,
        offset: usize,
        reason: &'static str,
    ) -> Result<Array, &'static str> {
        pointer
            .checked_add(offset)
            .and_then(|address| Array::read(self.memory, address, 8))
            .ok_or(reason)
    }

    fn element(
        &self,
        array: Array,
        index: u32,
        reason: &'static str,
    ) -> Result<usize, &'static str> {
        array
            .element(index, 8)
            .and_then(|address| read::<u64>(self.memory, address))
            .and_then(|pointer| usize::try_from(pointer).ok())
            .ok_or(reason)
    }

    fn diplomacy(&mut self, country: ccountry::Country) -> Result<usize, &'static str> {
        let fields = Fields {
            memory: self.memory,
            base: self.base,
        };
        self.resolver.diplomacy(country, || {
            let pointer = fields.pointer(country.pointer, ccountry::DIPLOMACY_STATUS)?;
            if pointer == 0 {
                return Err("country diplomacy is null");
            }
            fields.vtable(
                pointer,
                cdiplomacy_status::VTABLE_RVA,
                "diplomacy vtable mismatch",
            )?;
            if fields.field::<u64>(pointer, cdiplomacy_status::COUNTRY)? != country.pointer as u64 {
                return Err("diplomacy country mismatch");
            }
            Ok(pointer)
        })
    }

    fn active(&self, pointer: usize) -> Result<bool, &'static str> {
        let expected = self
            .base
            .checked_add(VTABLE_RVA)
            .ok_or("module address overflow")?;
        let mut vtable = [0; 8];
        let mut relation_type = [0; 4];
        let mut retired = [0; 1];
        // Installed inactive placeholders still require the vtable, native type
        // and retired byte. No endpoints or intervening bytes are copied here.
        let copied = self.memory.copy_scattered_prefetched(
            [
                pointer,
                pointer.checked_add(crelation::TYPE_ID).unwrap_or(0),
                pointer.checked_add(crelation::RETIRED).unwrap_or(0),
            ],
            &mut [&mut vtable, &mut relation_type, &mut retired],
        );
        if copied == 0 || u64::from_le_bytes(vtable) != expected as u64 {
            return Err("war relation vtable mismatch");
        }
        if copied < 2 {
            return Err("war relation field unreadable or address overflow");
        }
        if u32::from_le_bytes(relation_type) != TYPE {
            return Err("war matrix contains another relation type");
        }
        if copied < 3 {
            return Err("war relation field unreadable or address overflow");
        }
        Ok(retired[0] == 0)
    }

    fn pair_status(
        &mut self,
        diplomacy: usize,
        target: ccountry::Country,
        war: usize,
    ) -> Result<usize, &'static str> {
        let matrix = match self.current_matrix {
            (root, Some(matrix)) if root == diplomacy => matrix,
            _ => {
                let matrix = self.array(
                    diplomacy,
                    cdiplomacy_status::RELATIONS,
                    "war matrix unreadable or invalid",
                )?;
                if self.current_matrix.0 == diplomacy {
                    self.current_matrix.1 = Some(matrix);
                }
                matrix
            }
        };
        let status = self.element(
            matrix,
            target.canonical_slot,
            "war endpoint outside matrix or unreadable",
        )?;
        if status == 0 {
            return Err("null relation matrix entry");
        }
        const FIELDS: [RecordField; 3] = [
            RecordField {
                source_offset: 0,
                destination_offset: 0,
                len: 8,
            },
            RecordField {
                source_offset: crelation_status::DIPLOMACY_STATUS,
                destination_offset: 8,
                len: 8,
            },
            RecordField {
                source_offset: crelation_status::TARGET_COUNTRY,
                destination_offset: 16,
                len: 4,
            },
        ];
        const LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&FIELDS, 20).expect("valid record layout");
        let expected = self
            .base
            .checked_add(crelation_status::VTABLE_RVA)
            .ok_or("module address overflow")?;
        let mut bytes = [0; 20];
        let copied =
            self.memory
                .copy_records(RecordSources::Pointers(&[status]), &LAYOUT, &mut bytes);
        // Keep the original validation order even when a later field failed.
        // The war pointer remains a separate observation after target-country
        // resolution, which can itself perform uncached authentication reads.
        if copied == 0 || u64::from_le_bytes(bytes[..8].try_into().unwrap()) != expected as u64 {
            return Err("relation status vtable mismatch");
        }
        if copied < 2 {
            return Err("war relation field unreadable or address overflow");
        }
        if u64::from_le_bytes(bytes[8..16].try_into().unwrap()) != diplomacy as u64 {
            return Err("relation status diplomacy mismatch");
        }
        if copied < 3 {
            return Err("war relation field unreadable or address overflow");
        }
        let target_ref = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
        if self.resolve(target_ref)?.canonical_slot != target.canonical_slot {
            return Err("relation status target differs from matrix slot");
        }
        if self.field::<u64>(status, crelation_status::WAR_RELATION)? != war as u64 {
            return Err("war endpoint matrix pointers disagree");
        }
        Ok(status)
    }

    fn occurrence(
        &mut self,
        root: usize,
        offset: usize,
        war: usize,
        expected: u32,
    ) -> Result<(), &'static str> {
        if offset == cdiplomacy_status::OWNED_RELATIONS
            && expected == 1
            && let Some(scratch) = self.ownership.as_deref_mut()
        {
            return scratch.occurrence(self.memory, root, war);
        }
        let array = self.array(root, offset, WAR_MEMBERSHIP.vector)?;
        scan_occurrence(self.memory, array, war, expected, WAR_MEMBERSHIP, |_| {})
    }

    fn relation(
        &mut self,
        pointer: usize,
        owner: ccountry::Country,
    ) -> Result<Option<Row>, &'static str> {
        if !self.active(pointer)? {
            return Ok(None);
        }
        const ENDPOINT_FIELDS: [RecordField; 2] = [
            RecordField {
                source_offset: crelation::FIRST_COUNTRY,
                destination_offset: 0,
                len: 4,
            },
            RecordField {
                source_offset: crelation::SECOND_COUNTRY,
                destination_offset: 4,
                len: 4,
            },
        ];
        const ENDPOINT_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&ENDPOINT_FIELDS, 8).expect("valid record layout");
        let mut endpoints = [0; 8];
        if self.memory.copy_records(
            RecordSources::Pointers(&[pointer]),
            &ENDPOINT_LAYOUT,
            &mut endpoints,
        ) != ENDPOINT_FIELDS.len()
        {
            return Err("war relation field unreadable or address overflow");
        }
        let first_country = u32::from_le_bytes(endpoints[..4].try_into().unwrap());
        let second_country = u32::from_le_bytes(endpoints[4..].try_into().unwrap());
        let first = self.resolve(first_country)?;
        let second = self.resolve(second_country)?;
        if first.canonical_slot == second.canonical_slot {
            return Err("war relation endpoints resolve to same country");
        }
        if owner.canonical_slot != first.canonical_slot
            && owner.canonical_slot != second.canonical_slot
        {
            return Err("war relation does not include owning country");
        }
        let first_diplomacy = self.diplomacy(first)?;
        let second_diplomacy = self.diplomacy(second)?;
        let graph = StatusGraph {
            war: pointer,
            endpoints: [first_country, second_country],
            canonical: [first.canonical_slot, second.canonical_slot],
            diplomacies: [first_diplomacy, second_diplomacy],
        };
        let graph_slot = self
            .ownership
            .as_deref()
            .and_then(|scratch| scratch.status_graph_slot(pointer));
        let reused = graph_slot
            .is_some_and(|slot| self.ownership.as_deref().unwrap().status_graphs[slot] == graph);
        // The two owner visits authenticate the same ordered status graph.
        // Reuse only a complete success in this quiescent walk; active fields,
        // raw endpoints and owner inclusion above remain fresh observations.
        // Like country/diplomacy authentication, this cannot detect a newly
        // unreadable or mutated status graph between those visits.
        let statuses = if reused {
            None
        } else {
            let first_status = self.pair_status(first_diplomacy, second, pointer)?;
            hint_owned_status_headers(first_status);
            let second_status = self.pair_status(second_diplomacy, first, pointer)?;
            hint_owned_status_headers(second_status);
            Some([first_status, second_status])
        };
        // Attachment places the pointer in both the status matrix and diplomacy-owned
        // vectors; only the first endpoint receives serializable ownership.
        for diplomacy in [first_diplomacy, second_diplomacy] {
            self.occurrence(diplomacy, cdiplomacy_status::OWNED_RELATIONS, pointer, 1)?;
        }
        if let Some([first_status, second_status]) = statuses {
            for status in [first_status, second_status] {
                self.occurrence(status, crelation_status::SUBRELATIONS, pointer, 1)?;
            }
            self.occurrence(
                first_status,
                crelation_status::SERIALIZABLE_RELATIONS,
                pointer,
                1,
            )?;
            self.occurrence(
                second_status,
                crelation_status::SERIALIZABLE_RELATIONS,
                pointer,
                0,
            )?;
            if let Some(scratch) = self.ownership.as_deref_mut() {
                // Demand for the reuse table, counted per fresh validation;
                // it sizes the next pass's table and never limits this one.
                scratch.required_graphs = scratch.required_graphs.saturating_add(1);
                if let Some(slot) = graph_slot {
                    scratch.status_graphs[slot] = graph;
                }
            }
        }
        if owner.canonical_slot != first.canonical_slot {
            return Ok(None);
        }
        const ROW_FIELDS: [RecordField; 8] = [
            RecordField {
                source_offset: crelation::START_HOUR,
                destination_offset: 0,
                len: 4,
            },
            RecordField {
                source_offset: crelation::END_HOUR,
                destination_offset: 4,
                len: 4,
            },
            RecordField {
                source_offset: crelation::CANCEL,
                destination_offset: 8,
                len: 1,
            },
            RecordField {
                source_offset: FIRST_CASUALTIES,
                destination_offset: 9,
                len: 8,
            },
            RecordField {
                source_offset: SECOND_CASUALTIES,
                destination_offset: 17,
                len: 8,
            },
            RecordField {
                source_offset: FIRST_UNKNOWN_CASUALTIES,
                destination_offset: 25,
                len: 8,
            },
            RecordField {
                source_offset: SECOND_UNKNOWN_CASUALTIES,
                destination_offset: 33,
                len: 8,
            },
            RecordField {
                source_offset: FIRST_WAS_INSTIGATOR,
                destination_offset: 41,
                len: 1,
            },
        ];
        const ROW_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&ROW_FIELDS, 42).expect("valid record layout");
        let mut bytes = [0; 42];
        if self
            .memory
            .copy_records(RecordSources::Pointers(&[pointer]), &ROW_LAYOUT, &mut bytes)
            != ROW_FIELDS.len()
        {
            return Err("war relation field unreadable or address overflow");
        }
        Ok(Some(Row {
            first_country,
            second_country,
            start_hour: i32::from_le_bytes(bytes[..4].try_into().unwrap()),
            end_hour: i32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            cancel: bytes[8],
            first_casualties: i64::from_le_bytes(bytes[9..17].try_into().unwrap()),
            second_casualties: i64::from_le_bytes(bytes[17..25].try_into().unwrap()),
            first_unknown_casualties: i64::from_le_bytes(bytes[25..33].try_into().unwrap()),
            second_unknown_casualties: i64::from_le_bytes(bytes[33..41].try_into().unwrap()),
            first_was_instigator: bytes[41],
        }))
    }
}

#[cfg(test)]
mod tests;
