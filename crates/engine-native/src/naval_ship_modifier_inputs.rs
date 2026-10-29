//! Ordered raw source operands for ship and weapon modifiers. No modifier
//! total, doctrine weight, fuel fraction or weapon damage is evaluated.
//! Source indices preserve stored list order.
//!
//! These inputs require fresh reads: effective stats, medals, doctrine, fuel,
//! headquarters membership, leader adjustment and skill selection have independent
//! writers. Equal source pointers or counts cannot establish unchanged values or
//! receiver lifetime.
use crate::{
    Memory,
    capture_text::{Owned, Value},
    cnaval_combatant::field,
    cpdx_array::Array,
};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ContextRow {
    pub task_force: u64,
    pub country: u32,
    pub commander: u64,
    pub ship_class: u32,
    pub subunit: u32,
    /// Five stored ship-base operands in getter-case order.
    pub weapon_bases: [i64; 5],
    pub weather_factor: i64,
    pub fuel: [i64; 2],
    pub night: i64,
    pub formation_flags: u64,
    pub detection_base: i64,
    pub air_base: u64,
    pub air_base_maximum: i32,
    pub air_detection_base: i64,
    pub terrain_name: Owned,
    pub sources: [usize; 2],
}

/// Scopes0 country,1 pride,2 ship experience,3 medals,4 doctrine,5 terrain,
///6 region weather,7 province night,8 task force,9 commander,
///10 commander class,11 critical,12..15 support influence definitions.
/// Scopes20 terrain weapon adjuster,21 commander terrain adjuster,
///22 commander class adjuster,23 commander night adjuster,24 fuel defines.
/// Weights carry raw counts/denominators or the named adjuster's gun/depth pair.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SourceRow {
    pub scope: u8,
    pub index: u32,
    pub selector: u32,
    pub reference: u64,
    pub weights: [i64; 2],
    /// Terrain speed-adjustment operand, populated only for scopes 20 and 25.
    pub movement: i64,
    /// Selected doctrine country-modifier keys.
    pub dynamic_keys: [u32; 2],
    pub terms: [usize; 2],
    /// Transient source identity for shared term spans and this hour's child-tree
    /// capture. Never projected to replay.
    pub accumulator: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ModifierTerm {
    pub key: u32,
    pub raw: i64,
    pub metadata_flags: u32,
    pub feature_selector: u32,
}

/// Independent caller-owned storage for active or retained influence sources.
/// Worker capture grows these vectors from the original validated native counts.
pub struct InfluenceBuffers<'a> {
    pub sources: &'a mut Vec<SourceRow>,
    pub terms: &'a mut Vec<ModifierTerm>,
    pub lengths: &'a mut [usize; 2],
    pub required: &'a mut [usize; 2],
}

/// One capture's modifier definition table header and successful metadata
/// reads (flags, category mask and feature selector, the 12 bytes at
/// `FLAGS`) by key. Both are static content, so in the stopped world a
/// capture reads each once rather than once per accumulator entry; a failed
/// read is not kept, so it recurs where it would.
pub struct TermMetadata {
    /// Whether anything is kept; [`Self::unkept`] reads every time.
    keep: bool,
    table: Option<Array>,
    /// Indexed by key, sized to the live table's row count on first use; an
    /// allocation failure leaves it unsized, and every read direct.
    rows: Vec<Option<[u8; 12]>>,
    sized: bool,
}

impl Default for TermMetadata {
    fn default() -> Self {
        Self {
            keep: true,
            table: None,
            rows: Vec::new(),
            sized: false,
        }
    }
}

impl TermMetadata {
    /// A memo that keeps nothing: the table header and each entry's
    /// metadata are read every time (benchmarks compare against it).
    pub fn unkept() -> Self {
        Self {
            keep: false,
            ..Self::default()
        }
    }

    fn table(&mut self, memory: &impl Memory, base: usize) -> Result<Array, &'static str> {
        if let Some(table) = self.table {
            return Ok(table);
        }
        let table = crate::modifier_definition::table(memory, base)?;
        self.table = Some(table).filter(|_| self.keep);
        Ok(table)
    }

    /// The metadata of `key`, whose row `at` lies in `table`.
    fn flags(
        &mut self,
        memory: &impl Memory,
        table: Array,
        key: u32,
        at: usize,
    ) -> Result<[u8; 12], &'static str> {
        if let Some(Some(bytes)) = self.rows.get(key as usize) {
            return Ok(*bytes);
        }
        let bytes: [u8; 12] = field(memory, at, crate::modifier_definition::FLAGS)?;
        if !self.keep {
            return Ok(bytes);
        }
        if !self.sized {
            self.sized = true;
            if crate::allocation::resize_default(&mut self.rows, table.count as usize).is_err() {
                self.rows = Vec::new();
            }
        }
        if let Some(slot) = self.rows.get_mut(key as usize) {
            *slot = Some(bytes);
        }
        Ok(bytes)
    }
}

pub fn capture_influence(
    memory: &impl Memory,
    base: usize,
    wrapper: usize,
    index: u32,
    buffers: InfluenceBuffers<'_>,
) -> Result<[usize; 2], &'static str> {
    let start = buffers.lengths[0];
    let mut capture = Capture {
        memory,
        base,
        buffers,
        metadata: None,
    };
    visit_influence(memory, base, wrapper, index, &mut |row, accumulator| {
        capture.source(row, accumulator)
    })?;
    Ok([start, capture.buffers.lengths[0] - start])
}

/// A resolved naval headquarters influence reaches four skill-definition
/// accumulators. Count the shared visitor's source and term headers only.
pub fn influence_demand(
    memory: &impl Memory,
    base: usize,
    wrapper: usize,
) -> Result<[usize; 2], &'static str> {
    let mut demand = [0usize; 2];
    visit_influence(memory, base, wrapper, 0, &mut |_, accumulator| {
        demand[0] = demand[0]
            .checked_add(1)
            .ok_or("naval influence demand overflow")?;
        if let Some(accumulator) = accumulator {
            demand[1] = demand[1]
                .checked_add(array(memory, accumulator, 0, 16)?.count as usize)
                .ok_or("naval influence demand overflow")?;
        }
        Ok(())
    })?;
    Ok(demand)
}

fn ptr(memory: &impl Memory, p: usize, o: usize) -> Result<usize, &'static str> {
    field::<u64>(memory, p, o).map(|n| n as usize)
}
fn add(p: usize, o: usize) -> Result<usize, &'static str> {
    p.checked_add(o)
        .filter(|_| p != 0)
        .ok_or("naval weapon pointer invalid")
}
// Names use exact string segments and content-based fallback, without a byte cap.
fn text<const TERMINAL: bool>(
    memory: &impl Memory,
    address: usize,
    reason: &'static str,
) -> Result<Owned, &'static str> {
    let result = if TERMINAL {
        Owned::read(memory, address, false)
    } else {
        Owned::read_hint(memory, address, false)
    };
    result.map_err(|error| {
        if error == "string allocation failed" {
            error
        } else {
            reason
        }
    })
}
fn auth(memory: &impl Memory, base: usize, p: usize, rva: usize) -> Result<(), &'static str> {
    if p == 0 || field::<u64>(memory, p, 0)? != add(base, rva)? as u64 {
        return Err("naval weapon source identity mismatch");
    }
    Ok(())
}
fn array(memory: &impl Memory, p: usize, o: usize, stride: usize) -> Result<Array, &'static str> {
    let header = field(memory, p, o)?;
    let a = Array::from_header(header, stride).ok_or("naval weapon array invalid")?;
    Ok(a)
}
fn element(a: Array, i: u32, stride: usize) -> Result<usize, &'static str> {
    a.element(i, stride)
        .ok_or("naval weapon array element invalid")
}

struct Capture<'a, 'b, M> {
    memory: &'a M,
    base: usize,
    buffers: InfluenceBuffers<'b>,
    /// The capture's metadata memo, when the caller keeps one.
    metadata: Option<&'b mut TermMetadata>,
}
impl<M: Memory> Capture<'_, '_, M> {
    fn source(
        &mut self,
        mut row: SourceRow,
        accumulator: Option<usize>,
    ) -> Result<(), &'static str> {
        let source = self.buffers.lengths[0];
        self.buffers.required[0] = self.buffers.required[0]
            .checked_add(1)
            .ok_or("naval weapon source demand overflow")?;
        // Adjacent ships often select the same country and task-force accumulators.
        // Share a recent owned term span within this callback, retaining every source
        // occurrence in order. Bound the search because each ship's accumulator is unique.
        if let Some(accumulator) = accumulator
            && let Some(previous) = self.buffers.sources.get(..source).and_then(|rows| {
                rows.iter()
                    .rev()
                    .take(64)
                    .find(|previous| previous.accumulator == accumulator)
            })
        {
            row.accumulator = accumulator;
            row.terms = previous.terms;
            self.buffers.sources.truncate(source);
            crate::allocation::reserve(self.buffers.sources, 1)
                .map_err(|_| "collector allocation failed")?;
            self.buffers.sources.push(row);
            self.buffers.lengths[0] = source
                .checked_add(1)
                .ok_or("naval weapon source length overflow")?;
            return Ok(());
        }
        let data = accumulator
            .map(|p| array(self.memory, p, 0, 16))
            .transpose()?;
        let count = data.map_or(0, |a| a.count as usize);
        self.buffers.required[1] = self.buffers.required[1]
            .checked_add(count)
            .ok_or("naval weapon term demand overflow")?;
        let first = self.buffers.lengths[1];
        let end = first
            .checked_add(count)
            .ok_or("naval weapon term length overflow")?;
        let sources_end = source
            .checked_add(1)
            .ok_or("naval weapon source length overflow")?;
        row.terms = [first, count];
        row.accumulator = accumulator.unwrap_or(0);
        self.buffers.terms.truncate(first);
        crate::allocation::reserve_push_batch(self.buffers.terms, count)
            .map_err(|_| "collector allocation failed")?;
        self.buffers.sources.truncate(source);
        crate::allocation::reserve(self.buffers.sources, 1)
            .map_err(|_| "collector allocation failed")?;
        if let Some(entries) = data {
            use crate::modifier_definition as md;
            let table = match self.metadata.as_deref_mut() {
                Some(metadata) => metadata.table(self.memory, self.base)?,
                None => md::table(self.memory, self.base)?,
            };
            let mut previous = 0;
            let mut bytes = [0u8; 64 * 16];
            for start in (0..count).step_by(64) {
                let n = (count - start).min(64);
                if !crate::read_slice(
                    self.memory,
                    element(entries, start as u32, 16)?,
                    &mut bytes[..n * 16],
                ) {
                    return Err("naval weapon modifier bytes unreadable");
                }
                for b in bytes[..n * 16].chunks_exact(16) {
                    let key = u32::from_le_bytes(b[..4].try_into().unwrap());
                    if key <= previous {
                        return Err("naval weapon modifier keys invalid");
                    }
                    previous = key;
                    let at = table
                        .element(key, md::STRIDE)
                        .ok_or("naval weapon modifier key outside definition table")?;
                    let metadata: [u8; 12] = match self.metadata.as_deref_mut() {
                        Some(memo) => memo.flags(self.memory, table, key, at)?,
                        None => field(self.memory, at, md::FLAGS)?,
                    };
                    let term = ModifierTerm {
                        key,
                        raw: i64::from_le_bytes(b[8..16].try_into().unwrap()),
                        metadata_flags: u32::from_le_bytes(metadata[..4].try_into().unwrap()),
                        feature_selector: u32::from_le_bytes(metadata[8..].try_into().unwrap()),
                    };
                    self.buffers.terms.push(term);
                }
            }
        }
        self.buffers.sources.truncate(source);
        self.buffers.sources.push(row);
        *self.buffers.lengths = [sources_end, end];
        Ok(())
    }
}

/// Terrain source selection preserves both selected and default raw values.
pub(crate) fn terrain_class(
    memory: &impl Memory,
    terrain: usize,
    class: u32,
) -> Result<(usize, usize), &'static str> {
    let header: [u8; 17] = field(memory, terrain, 0x1b8)?;
    let data = u64::from_le_bytes(header[..8].try_into().unwrap()) as usize;
    let mask = u32::from_le_bytes(header[12..16].try_into().unwrap());
    let buckets = mask
        .checked_add(1)
        .filter(|n| n.is_power_of_two())
        .ok_or("naval terrain class hash invalid")?;
    let extent = buckets + u32::from(header[16]);
    let mut hash = (class >> 16 ^ class).wrapping_mul(0x45d9f3b);
    hash = (hash >> 16 ^ hash).wrapping_mul(0x45d9f3b);
    hash ^= hash >> 16;
    let mut selected = None;
    // Probe distance and extension are stored u8 fields.
    for probe in 1..=u32::from(u8::MAX) {
        let index = (hash & mask)
            .checked_add(probe - 1)
            .filter(|n| *n < extent)
            .ok_or("naval terrain class hash extent invalid")?;
        let at = add(data, index as usize * 12)?;
        let distance = field::<u8>(memory, at, 0)?;
        if u32::from(distance) < probe {
            break;
        }
        if field::<u32>(memory, at, 4)? == class {
            selected = Some(field::<u32>(memory, at, 8)?);
            break;
        }
        if probe == u32::from(u8::MAX) {
            return Err("naval terrain class probe bound exceeded");
        }
    }
    let value = if let Some(index) = selected {
        let a = array(memory, terrain, 0x198, 0x110)?;
        let entry = element(a, index, 0x110)?;
        (add(entry, 0x28)?, add(entry, 0xe8)?)
    } else {
        (add(terrain, 0xa8)?, add(terrain, 0x168)?)
    };
    Ok(value)
}

pub fn terrain(
    memory: &impl Memory,
    base: usize,
    definition: usize,
) -> Result<usize, &'static str> {
    terrain_with_policy::<true>(memory, base, definition)
}

pub(crate) fn terrain_with_policy<const TERMINAL: bool>(
    memory: &impl Memory,
    base: usize,
    definition: usize,
) -> Result<usize, &'static str> {
    terrain_from_template_with_policy::<TERMINAL>(memory, base, ptr(memory, definition, 0x30)?)
}

/// [`terrain`] keeping the ocean fallback in `ocean` for the capture. The
/// fallback is content, fixed after load, so every fallback of one stopped
/// capture resolves the same terrain; only a complete one is kept.
pub(crate) fn terrain_kept(
    memory: &impl Memory,
    base: usize,
    definition: usize,
    ocean: &mut Option<usize>,
) -> Result<usize, &'static str> {
    terrain_from_template_kept(memory, base, ptr(memory, definition, 0x30)?, ocean)
}

/// The same native terrain selector, consuming a template pointer already
/// acquired by the caller (which avoids reacquiring that selecting control),
/// keeping the ocean fallback as [`terrain_kept`].
pub(crate) fn terrain_from_template_kept(
    memory: &impl Memory,
    base: usize,
    template: usize,
    ocean: &mut Option<usize>,
) -> Result<usize, &'static str> {
    if let Some(terrain) = template_terrain(memory, base, template)? {
        return Ok(terrain);
    }
    if let Some(terrain) = *ocean {
        return Ok(terrain);
    }
    let terrain = ocean_terrain::<true>(memory, base)?;
    *ocean = Some(terrain);
    Ok(terrain)
}

/// Terrain lookup: the template's terrain when present and active, else the
/// terrain database's ocean.
fn terrain_from_template_with_policy<const TERMINAL: bool>(
    memory: &impl Memory,
    base: usize,
    template: usize,
) -> Result<usize, &'static str> {
    match template_terrain(memory, base, template)? {
        Some(terrain) => Ok(terrain),
        None => ocean_terrain::<TERMINAL>(memory, base),
    }
}

/// The template's own terrain when present and active.
fn template_terrain(
    memory: &impl Memory,
    base: usize,
    template: usize,
) -> Result<Option<usize>, &'static str> {
    let terrain = ptr(memory, template, 0x130)?;
    if terrain != 0 && field::<u8>(memory, terrain, 0x10)? != 0 {
        auth(memory, base, terrain, crate::cterrain_type::VTABLE_RVA)?;
        return Ok(Some(terrain));
    }
    Ok(None)
}

/// Terrain lookup's fallback: the one active terrain named by the
/// installation's five-byte ocean key, found by hashed-name lookup over the
/// database.
fn ocean_terrain<const TERMINAL: bool>(
    memory: &impl Memory,
    base: usize,
) -> Result<usize, &'static str> {
    // Capture the key from the pinned module instead of assuming English
    // content.
    let key: [u8; 5] = field(memory, base, 0x029f_59c8)?;
    let root = ptr(memory, base, crate::cterrain_database::SINGLETON_RVA)?;
    let types = array(memory, root, 0x40, 8)?;
    let mut found = None;
    for index in 0..types.count {
        let p = ptr(memory, element(types, index, 8)?, 0)?;
        if field::<u8>(memory, p, 0x10)? == 0 {
            continue;
        }
        auth(memory, base, p, crate::cterrain_type::VTABLE_RVA)?;
        let name = text::<TERMINAL>(memory, add(p, 0x18)?, "naval terrain name invalid")?;
        if name.as_bytes() == key && found.replace(p).is_some() {
            return Err("naval terrain duplicate fallback");
        }
    }
    found.ok_or("naval terrain fallback absent")
}

pub(crate) fn selected_class(
    memory: &impl Memory,
    leader: usize,
    subunit: u32,
    modifiers: bool,
) -> Result<usize, &'static str> {
    let (offset, stride, fallback) = if modifiers {
        (0x148, 0xd0, 0x188)
    } else {
        (0x130, 0x38, 0x160)
    };
    let a = array(memory, leader, offset, stride)?;
    let mut found = None;
    let mut previous = None;
    for index in 0..a.count {
        let p = element(a, index, stride)?;
        let key = field::<i32>(memory, p, 8)?;
        if previous.is_some_and(|v| v >= key) {
            return Err("naval commander class order invalid");
        }
        previous = Some(key);
        if key == subunit as i32 {
            found = Some(add(p, 0x10)?);
        }
    }
    Ok(found.unwrap_or(add(leader, fallback)?))
}

/// One capture's task-force-level sources for the weapon contexts: the
/// rows each task force's doctrine walk emitted, in order, each fleet's
/// default leader and each commander's selected class records, each roster
/// country, each task force's location terms and each terrain's class
/// records. Every ship of a task force in the stopped capture reaches the
/// same ones, so later ships re-emit the doctrine rows instead of walking
/// the tree and take the other observations instead of reading them again,
/// in the same order as the reads they replace. A walk, search or read that
/// fails keeps nothing, so its failure recurs where it would.
#[derive(Default)]
pub struct SourceMemo {
    /// Task force -> its doctrine rows' span of `rows`.
    tasks: crate::recent::Recent<usize, (usize, usize), 16>,
    rows: Vec<(SourceRow, Option<usize>)>,
    /// Fleet -> its default leader (zero: none).
    leaders: crate::recent::Recent<usize, usize, 8>,
    /// (commander, subunit, modifier or adjuster) -> the selected record.
    classes: crate::recent::Recent<(usize, u32, bool), usize, 16>,
    /// (game, native country id) -> the resolved country.
    countries: crate::recent::Recent<(usize, u32), crate::ccountry::Country, 8>,
    /// (task force, location) -> its location terms and terrain name, the
    /// latest [`LOCATION_SLOTS`] task forces.
    locations: Vec<((usize, usize), LocationTerms, Owned)>,
    /// (selected terrain, ship class) -> its class records.
    terrain_classes: crate::recent::Recent<(usize, u32), TerrainClass, 16>,
    /// Task force -> its HQ influence rows' span of `rows`.
    influences: crate::recent::Recent<usize, (usize, usize), 16>,
    /// The fuel defines, eligibility answer and trade feature flags (scopes
    /// 24 and 26), module globals.
    globals: Option<([i64; 2], i64, u32)>,
}

/// Task forces whose location terms a [`SourceMemo`] keeps.
const LOCATION_SLOTS: usize = 16;

/// A task force's location terms in ship modifier source selection's location
/// branch, as the first ship of it read them: the relation the
/// expeditionary-or-roster owner holds with the location's controller, the
/// selected terrain and the weather region and province records.
#[derive(Clone, Copy)]
struct LocationTerms {
    owner_id: u32,
    target: u32,
    /// The relation's accumulator, when the matrix holds one.
    relation: Option<usize>,
    selected: usize,
    /// The weather region record's modifiers.
    region: usize,
    /// The province weather record.
    record: usize,
    night: i64,
}

/// A terrain's class records for one ship class: the modifier and the
/// adjuster's gun, depth and movement words.
#[derive(Clone, Copy)]
struct TerrainClass {
    modifier: usize,
    weights: [i64; 2],
    movement: i64,
}

/// A copy of `text` in its own storage; an allocation failure fails as a
/// read of it would.
fn owned_copy(text: &Owned) -> Result<Owned, &'static str> {
    let mut copy = Owned::default();
    copy.prepare(text.len)
        .map_err(|_| "string allocation failed")?;
    copy.copy_prepared(text.as_bytes())?;
    Ok(copy)
}

impl SourceMemo {
    fn location(&self, task: usize, location: usize) -> Option<(LocationTerms, &Owned)> {
        self.locations
            .iter()
            .rev()
            .find(|(key, _, _)| *key == (task, location))
            .map(|(_, terms, name)| (*terms, name))
    }

    fn keep_location(&mut self, task: usize, location: usize, terms: LocationTerms, name: &Owned) {
        let Ok(name) = owned_copy(name) else {
            return;
        };
        if self.locations.len() == LOCATION_SLOTS {
            self.locations.remove(0);
        } else if crate::allocation::reserve(&mut self.locations, 1).is_err() {
            return;
        }
        self.locations.push(((task, location), terms, name));
    }

    fn selected_class(
        &mut self,
        memory: &impl Memory,
        leader: usize,
        subunit: u32,
        modifiers: bool,
    ) -> Result<usize, &'static str> {
        let key = (leader, subunit, modifiers);
        if let Some(found) = self.classes.get(&key) {
            return Ok(found);
        }
        let found = selected_class(memory, leader, subunit, modifiers)?;
        self.classes.insert(key, found);
        Ok(found)
    }
}

/// `selected`'s class records for `class` (`terrain_class`) through `memo`
/// when the caller keeps one, emitting its modifier (scope 5) between the
/// selection and the adjuster's words as the location branch does.
fn terrain_class_kept(
    memory: &impl Memory,
    memo: &mut Option<&mut SourceMemo>,
    selected: usize,
    class: u32,
    emit: &mut impl FnMut(SourceRow, Option<usize>) -> Result<(), &'static str>,
) -> Result<TerrainClass, &'static str> {
    if let Some(kept) = memo
        .as_deref()
        .and_then(|memo| memo.terrain_classes.get(&(selected, class)))
    {
        visit_modifier(emit, 5, kept.modifier)?;
        return Ok(kept);
    }
    let (modifier, adjuster) = terrain_class(memory, selected, class)?;
    visit_modifier(emit, 5, modifier)?;
    let found = TerrainClass {
        modifier,
        weights: [
            field(memory, adjuster, 0x10)?,
            field(memory, adjuster, 0x18)?,
        ],
        movement: field(memory, adjuster, 0x20)?,
    };
    if let Some(memo) = memo.as_deref_mut() {
        memo.terrain_classes.insert((selected, class), found);
    }
    Ok(found)
}

/// A negative ship class delegates to an unweighted average of each task-force
/// ship's terrain adjuster, not an opaque cache: one scope 25 row per roster
/// ship.
fn mixed_terrain(
    memory: &impl Memory,
    base: usize,
    task: usize,
    selected: usize,
    emit: &mut impl FnMut(SourceRow, Option<usize>) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    use crate::cship;
    let ships = array(memory, task, 0x348, 8)?;
    for index in 0..ships.count {
        let p = ptr(memory, element(ships, index, 8)?, 0)?;
        auth(memory, base, p, cship::VTABLE_RVA)?;
        let class = field(memory, p, 0x88)?;
        let (_, adjuster) = terrain_class(memory, selected, class)?;
        emit(
            SourceRow {
                scope: 25,
                index,
                selector: class,
                reference: field(memory, p, 8)?,
                weights: [
                    field(memory, adjuster, 0x10)?,
                    field(memory, adjuster, 0x18)?,
                ],
                movement: field(memory, adjuster, 0x20)?,
                ..Default::default()
            },
            None,
        )?;
    }
    Ok(())
}

/// Ship modifier source selection's location branch for the first ship of a
/// task force at `location`: the relation (scope 16), terrain (5, 20, 25),
/// weather region (6), province (7) and task-force (8) sources, the terrain
/// name and night, read in native order and kept in `memo` for the task force's
/// later ships. Returns the selected terrain.
#[allow(clippy::too_many_arguments)] // The ship's context row, the capture's memo and visitor.
fn location_terms<const TERMINAL: bool>(
    memory: &impl Memory,
    base: usize,
    game: usize,
    task: usize,
    location: usize,
    country_id: u32,
    row: &mut ContextRow,
    memo: &mut Option<&mut SourceMemo>,
    emit: &mut impl FnMut(SourceRow, Option<usize>) -> Result<(), &'static str>,
) -> Result<usize, &'static str> {
    use crate::ccountry;
    auth(memory, base, location, crate::cprovince::VTABLE_RVA)?;
    let province = field::<u32>(memory, location, crate::cprovince::ID)?;
    // Ship modifier source selection chooses expeditionary owner when positive,
    // then the relation matrix entry for the location's controller (ordinal0
    // when unowned). Keep this separate from roster-country modifier scope0.
    let expeditionary = field::<i32>(memory, task, crate::cunit::EXPEDITIONARY_OWNER)?;
    let owner_id = if expeditionary > 0 {
        expeditionary as u32
    } else {
        country_id
    };
    let countries = ccountry::Reader::new(memory, base, game)?;
    let owner = countries.resolve(owner_id)?;
    let target = field::<u32>(memory, location, 0x188)?;
    let target_slot = if target == 0 {
        0
    } else {
        countries.resolve(target)?.canonical_slot
    };
    let diplomacy = ptr(memory, owner.pointer, ccountry::DIPLOMACY_STATUS)?;
    auth(
        memory,
        base,
        diplomacy,
        crate::cdiplomacy_status::VTABLE_RVA,
    )?;
    let relations = array(memory, diplomacy, crate::cdiplomacy_status::RELATIONS, 8)?;
    let relation = ptr(memory, element(relations, target_slot, 8)?, 0)?;
    let relation = if relation != 0 {
        auth(memory, base, relation, crate::crelation_status::VTABLE_RVA)?;
        if ptr(memory, relation, crate::crelation_status::DIPLOMACY_STATUS)? != diplomacy {
            return Err("naval weapon relation owner mismatch");
        }
        let accumulator = add(relation, 0x140)?;
        emit(
            SourceRow {
                scope: 16,
                index: owner_id,
                selector: target,
                ..Default::default()
            },
            Some(accumulator),
        )?;
        Some(accumulator)
    } else {
        None
    };
    let definition = ptr(memory, location, 0xc8)?;
    let selected = terrain_with_policy::<TERMINAL>(memory, base, definition)?;
    row.terrain_name = text::<TERMINAL>(
        memory,
        add(selected, 0x18)?,
        "naval selected terrain name invalid",
    )?;
    let class = terrain_class_kept(memory, memo, selected, row.ship_class, emit)?;
    emit(
        SourceRow {
            scope: 20,
            selector: row.ship_class,
            weights: class.weights,
            movement: class.movement,
            ..Default::default()
        },
        None,
    )?;
    if (row.ship_class as i32) < 0 {
        mixed_terrain(memory, base, task, selected, emit)?;
    }
    let weather = ptr(memory, game, crate::cgame_state::WEATHER_MANAGER)?;
    auth(memory, base, weather, crate::cweather_manager::VTABLE_RVA)?;
    let region = field::<u32>(memory, definition, 0x58)?;
    let regions = array(memory, weather, 0x40, 0x160)?;
    let region = add(element(regions, region, 0x160)?, 0x48)?;
    visit_modifier(emit, 6, region)?;
    let provinces = array(memory, weather, 0x370, 0x330)?;
    let record = element(provinces, province, 0x330)?;
    visit_modifier(emit, 7, add(record, 0x270)?)?;
    row.night = field(memory, record, 0x240)?;
    visit_modifier(emit, 8, add(task, 0x658)?)?;
    if let Some(memo) = memo.as_deref_mut() {
        memo.keep_location(
            task,
            location,
            LocationTerms {
                owner_id,
                target,
                relation,
                selected,
                region,
                record,
                night: row.night,
            },
            &row.terrain_name,
        );
    }
    Ok(selected)
}

/// [`selected_class`] through `memo` when the caller keeps one.
fn class_kept(
    memory: &impl Memory,
    memo: &mut Option<&mut SourceMemo>,
    leader: usize,
    subunit: u32,
    modifiers: bool,
) -> Result<usize, &'static str> {
    match memo.as_deref_mut() {
        Some(memo) => memo.selected_class(memory, leader, subunit, modifiers),
        None => selected_class(memory, leader, subunit, modifiers),
    }
}

/// `task`'s doctrine sources: re-emitted from `memo` when an earlier ship of
/// the capture walked them, else walked ([`visit_doctrine`]) and kept.
fn doctrine_kept<const TERMINAL: bool>(
    memory: &impl Memory,
    base: usize,
    task: usize,
    keys: &mut DoctrineKeys,
    memo: &mut Option<&mut SourceMemo>,
    emit: &mut impl FnMut(SourceRow, Option<usize>) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let Some(memo) = memo.as_deref_mut() else {
        return visit_doctrine::<TERMINAL>(memory, base, task, keys, emit);
    };
    if let Some((start, len)) = memo.tasks.get(&task) {
        for at in start..start + len {
            let (row, accumulator) = memo.rows[at];
            emit(row, accumulator)?;
        }
        return Ok(());
    }
    let start = memo.rows.len();
    let mut kept = true;
    let rows = &mut memo.rows;
    let result = visit_doctrine::<TERMINAL>(memory, base, task, keys, &mut |row, accumulator| {
        if kept && crate::allocation::reserve(rows, 1).is_ok() {
            rows.push((row, accumulator));
        } else {
            kept = false;
        }
        emit(row, accumulator)
    });
    if result.is_ok() && kept {
        let len = memo.rows.len() - start;
        memo.tasks.insert(task, (start, len));
    } else {
        memo.rows.truncate(start);
    }
    result
}

/// `task`'s HQ influence sources (scopes 12 to 15): re-emitted from `memo` when an earlier ship of
/// the capture read them, else read ([`visit_influence`] over the task force's
/// attached-headquarters array) and kept.
fn influences_kept(
    memory: &impl Memory,
    base: usize,
    task: usize,
    memo: &mut Option<&mut SourceMemo>,
    emit: &mut impl FnMut(SourceRow, Option<usize>) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let walk = |emit: &mut dyn FnMut(SourceRow, Option<usize>) -> Result<(), &'static str>| {
        let mut emit = emit;
        let influences = array(memory, task, 0x728, 8)?;
        for index in 0..influences.count {
            let wrapper = ptr(memory, element(influences, index, 8)?, 0)?;
            visit_influence(memory, base, wrapper, index, &mut emit)?;
        }
        Ok(())
    };
    let Some(memo) = memo.as_deref_mut() else {
        return walk(emit);
    };
    if let Some((start, len)) = memo.influences.get(&task) {
        for at in start..start + len {
            let (row, accumulator) = memo.rows[at];
            emit(row, accumulator)?;
        }
        return Ok(());
    }
    let start = memo.rows.len();
    let mut kept = true;
    let rows = &mut memo.rows;
    let result = walk(&mut |row, accumulator| {
        if kept && crate::allocation::reserve(rows, 1).is_ok() {
            rows.push((row, accumulator));
        } else {
            kept = false;
        }
        emit(row, accumulator)
    });
    if result.is_ok() && kept {
        let len = memo.rows.len() - start;
        memo.influences.insert(task, (start, len));
    } else {
        memo.rows.truncate(start);
    }
    result
}

/// The fuel defines (scope 24) and the modifier eligibility answer and
/// trade feature flags (scope 26), module globals read once per capture when
/// the caller keeps a memo.
fn globals_kept(
    memory: &impl Memory,
    base: usize,
    memo: &mut Option<&mut SourceMemo>,
    emit: &mut impl FnMut(SourceRow, Option<usize>) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    use crate::modifier_definition as md;
    let defines = |weights: [i64; 2]| SourceRow {
        scope: 24,
        weights,
        ..Default::default()
    };
    let features = |flags: u32, eligibility: i64| SourceRow {
        scope: 26,
        weights: [i64::from(flags), eligibility],
        ..Default::default()
    };
    if let Some((weights, eligibility, flags)) = memo.as_deref().and_then(|memo| memo.globals) {
        emit(defines(weights), None)?;
        return emit(features(flags, eligibility), None);
    }
    let weights = [
        field(memory, base, 0x033d_fa90)?,
        field(memory, base, 0x033d_fb30)?,
    ];
    emit(defines(weights), None)?;
    let callback = field::<u64>(memory, base, md::ELIGIBILITY_CALLBACK_RVA)?;
    let eligibility = if callback == 0 {
        0
    } else if callback == add(base, md::ELIGIBILITY_FLAGS_GETTER_RVA)? as u64 {
        1
    } else {
        return Err("naval weapon modifier callback unsupported");
    };
    let flags = field::<u32>(memory, base, md::TRADE_FEATURE_FLAGS_RVA)?;
    emit(features(flags, eligibility), None)?;
    if let Some(memo) = memo.as_deref_mut() {
        memo.globals = Some((weights, eligibility, flags));
    }
    Ok(())
}

/// Doctrine selector -> its two country keys ([`doctrine_keys`]) for one
/// capture or sizing pass.
pub type DoctrineKeys = crate::recent::Recent<u32, [u32; 2], 64>;

/// [`doctrine_keys`] keeping each selector's complete answer in `keys`. The
/// lexer's dictionary does not change within a stopped capture, so a capture
/// walks it once per selector rather than once per doctrine node of every
/// ship.
pub(crate) fn doctrine_keys_kept<const TERMINAL: bool>(
    memory: &impl Memory,
    base: usize,
    selector: u32,
    keys: &mut DoctrineKeys,
) -> Result<[u32; 2], &'static str> {
    if let Some(found) = keys.get(&selector) {
        return Ok(found);
    }
    let found = doctrine_keys::<TERMINAL>(memory, base, selector)?;
    keys.insert(selector, found);
    Ok(found)
}

/// Select the two native country keys without calculating either modifier.
/// Country key selection constructs prefix+lexer(type)+suffix and maps its
/// lexer id through the immutable modifier-key dictionary. We compare the raw
/// string segments while walking that bounded dictionary; no engine lexer
/// method or lock runs.
pub(crate) fn doctrine_keys<const TERMINAL: bool>(
    memory: &impl Memory,
    base: usize,
    selector: u32,
) -> Result<[u32; 2], &'static str> {
    use crate::cpdx_lexer as lexer;
    if field::<u32>(memory, base, lexer::WRITER_FLAG_RVA)? != 0 {
        return Err("naval doctrine lexer writer active");
    }
    let witness: [u8; 64] = field(memory, base, lexer::INITIALIZED_RVA)?;
    let _ = lexer::PreparedWitness::from_header(witness)?;
    let maximum = field::<u32>(memory, base, lexer::MAX_TOKEN_ID_RVA)?;
    let table = ptr(memory, base, lexer::TOKEN_NAMES_RVA)?;
    let token_text = |token: u32| -> Result<Owned, &'static str> {
        if token > maximum {
            return Err("naval doctrine token outside dictionary");
        }
        text::<TERMINAL>(
            memory,
            add(table, token as usize * 32)?,
            "naval doctrine token string invalid",
        )
    };
    let selected = token_text(selector)?;
    let generators = [
        ptr(memory, base, 0x0343_9158)?,
        ptr(memory, base, 0x0343_9150)?,
    ];
    let mut prefixes = std::array::from_fn::<_, 2, _>(|_| Owned::default());
    let mut suffixes = std::array::from_fn::<_, 2, _>(|_| Owned::default());
    for lane in 0..2 {
        prefixes[lane] = text::<TERMINAL>(
            memory,
            add(generators[lane], 0x10)?,
            "naval doctrine prefix invalid",
        )?;
        suffixes[lane] = text::<TERMINAL>(
            memory,
            add(generators[lane], 0x30)?,
            "naval doctrine suffix invalid",
        )?;
    }
    let head = ptr(memory, base, 0x030f_eba8)?;
    let mut node = ptr(memory, head, 8)?;
    let mut previous = head;
    let mut result = [0; 2];
    // Predecessor/next links reject a revisited node because its predecessor cannot match both visits.
    while node != head {
        if ptr(memory, node, 0)? != previous {
            return Err("naval doctrine modifier dictionary link invalid");
        }
        let name = token_text(field(memory, node, 0x10)?)?;
        for lane in 0..2 {
            let p = prefixes[lane].as_bytes();
            let s = selected.as_bytes();
            let t = suffixes[lane].as_bytes();
            let n = name.as_bytes();
            let length = p
                .len()
                .checked_add(s.len())
                .and_then(|len| len.checked_add(t.len()))
                .ok_or("naval doctrine key length overflow")?;
            if n.len() == length
                && n.starts_with(p)
                && n.ends_with(t)
                && &n[p.len()..p.len() + s.len()] == s
            {
                if result[lane] != 0 {
                    return Err("naval doctrine modifier key duplicate");
                }
                result[lane] = field(memory, node, 0x14)?;
            }
        }
        previous = node;
        node = ptr(memory, node, 8)?;
    }
    if ptr(memory, head, 0)? != previous {
        return Err("naval doctrine modifier dictionary link invalid");
    }
    Ok(result)
}

fn visit_influence(
    memory: &impl Memory,
    base: usize,
    wrapper: usize,
    index: u32,
    emit: &mut impl FnMut(SourceRow, Option<usize>) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let influence = ptr(memory, wrapper, 0x40)?;
    if influence == 0 {
        return Ok(());
    }
    let reference = field(memory, influence, 8)?;
    let Some(character) = crate::creference_registry::resolve(memory, base, reference)? else {
        return Ok(());
    };
    let mut country = ptr(memory, character, 0xa8)?;
    if country == 0 {
        country =
            crate::creference_registry::resolve(memory, base, field(memory, character, 0xc8)?)?
                .unwrap_or(0);
    }
    if country == 0 || field::<i32>(memory, country, 0xe7c)? != 2 {
        return Ok(());
    }
    for (lane, offset) in [0xf70, 0xf80, 0xfa0, 0xf90].into_iter().enumerate() {
        let definition = ptr(memory, country, offset)?;
        emit(
            SourceRow {
                scope: 12 + lane as u8,
                index,
                reference,
                ..Default::default()
            },
            Some(add(definition, 0xe0)?),
        )?;
    }
    Ok(())
}

fn visit_modifier(
    emit: &mut impl FnMut(SourceRow, Option<usize>) -> Result<(), &'static str>,
    scope: u8,
    p: usize,
) -> Result<(), &'static str> {
    emit(
        SourceRow {
            scope,
            ..Default::default()
        },
        Some(add(p, 0x10)?),
    )
}

fn visit_doctrine<const TERMINAL: bool>(
    memory: &impl Memory,
    base: usize,
    task: usize,
    keys: &mut DoctrineKeys,
    emit: &mut impl FnMut(SourceRow, Option<usize>) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    // Task-force doctrine uses successor order and a stored u64 count, without
    // a chosen node ceiling. Reuse its count-bounded cursor and link validation.
    let mut tree = crate::msvc_tree_node::Cursor::read(memory, task, 0x640)
        .map_err(|_| "naval doctrine tree header invalid")?;
    let mut previous = None;
    let mut index = 0;
    while let Some(node) = tree
        .next(memory)
        .map_err(|_| "naval doctrine tree links or count invalid")?
    {
        let key = field::<i32>(memory, node, 0x20)?;
        if previous.is_some_and(|v| v >= key) {
            return Err("naval doctrine order invalid");
        }
        previous = Some(key);
        let modifier = ptr(memory, node, 0x30)?;
        let dynamic_keys = doctrine_keys_kept::<TERMINAL>(memory, base, key as u32, keys)?;
        emit(
            SourceRow {
                scope: 4,
                index,
                selector: key as u32,
                dynamic_keys,
                weights: [
                    i64::from(field::<i32>(memory, node, 0x2c)?),
                    i64::from(field::<i32>(memory, node, 0x28)?),
                ],
                ..Default::default()
            },
            Some(add(modifier, 0x10)?),
        )?;
        index += 1;
    }
    Ok(())
}

/// Authenticates a live CShip and captures the shared source visitor's operands.
pub(crate) fn read(
    memory: &impl Memory,
    base: usize,
    game: usize,
    ship: usize,
    side: usize,
    buffers: &mut crate::cfex_member::DetailBuffers<'_>,
) -> Result<ContextRow, &'static str> {
    let start = buffers.weapon_lengths[0];
    let mut capture = Capture {
        memory,
        base,
        buffers: InfluenceBuffers {
            sources: buffers.weapon_sources,
            terms: buffers.weapon_terms,
            lengths: &mut buffers.weapon_lengths,
            required: &mut buffers.weapon_required,
        },
        metadata: Some(&mut buffers.term_metadata),
    };
    let mut row = visit_sources::<true>(
        memory,
        base,
        game,
        ship,
        side,
        &mut buffers.doctrine_keys,
        buffers.source_memo.as_mut(),
        &mut |row, accumulator| capture.source(row, accumulator),
    )?;
    row.sources = [start, capture.buffers.lengths[0] - start];
    Ok(row)
}

/// Shared ship modifier source selection, in native order, without reading
/// accumulator entries. Capture and off-thread sizing supply separate visitors.
#[allow(clippy::too_many_arguments)] // The ship, its side, and the capture's memos and visitor.
pub fn visit_sources<const TERMINAL: bool>(
    memory: &impl Memory,
    base: usize,
    game: usize,
    ship: usize,
    side: usize,
    doctrine_keys: &mut DoctrineKeys,
    mut memo: Option<&mut SourceMemo>,
    emit: &mut impl FnMut(SourceRow, Option<usize>) -> Result<(), &'static str>,
) -> Result<ContextRow, &'static str> {
    use crate::{ccountry, cnavy_leader, cship, ctask_force, cunit};
    auth(memory, base, ship, cship::VTABLE_RVA)?;
    let task = ptr(memory, ship, 0x728)?;
    auth(memory, base, task, ctask_force::VTABLE_RVA)?;
    let task_ref = field(memory, task, cunit::HANDLE)?;
    let country_id = field(memory, task, cunit::ROSTER_OWNER)?;
    let country = match memo
        .as_deref()
        .and_then(|memo| memo.countries.get(&(game, country_id)))
    {
        Some(country) => country,
        None => {
            let country = ccountry::Reader::new(memory, base, game)?.resolve(country_id)?;
            if let Some(memo) = memo.as_deref_mut() {
                memo.countries.insert((game, country_id), country);
            }
            country
        }
    };
    let fleet = ptr(memory, task, 0x340)?;
    let default_leader = if fleet == 0 {
        0
    } else if let Some(leader) = memo.as_deref().and_then(|memo| memo.leaders.get(&fleet)) {
        leader
    } else {
        let leader =
            crate::creference_registry::resolve(memory, base, field(memory, fleet, 0xb0)?)?
                .unwrap_or(0);
        if let Some(memo) = memo.as_deref_mut() {
            memo.leaders.insert(fleet, leader);
        }
        leader
    };
    let mut leader = ptr(memory, side, 0x130)?;
    if leader == 0 {
        leader = default_leader;
    }
    let commander = if leader == 0 {
        0
    } else {
        auth(memory, base, leader, cnavy_leader::VTABLE_RVA)?;
        field(memory, leader, 8)?
    };
    let mut row = ContextRow {
        task_force: task_ref,
        country: country_id,
        commander,
        ship_class: field(memory, ship, 0x88)?,
        subunit: field(memory, ship, 0x60c)?,
        ..Default::default()
    };
    row.weather_factor = field(memory, ship, 0x270)?;
    row.fuel = [field(memory, task, 0x4f8)?, field(memory, task, 0x500)?];
    row.formation_flags = field(memory, ship, 0x628)?;
    row.detection_base = field(memory, ship, 0x1b8)?;
    row.air_detection_base = field(memory, ship, 0x1e0)?;
    for (index, offset) in [0x1e8, 0x1f8, 0x210, 0x208, 0x318].into_iter().enumerate() {
        row.weapon_bases[index] = field(memory, ship, offset)?;
    }
    let air_base = ptr(memory, ship, 0x730)?;
    if air_base != 0 {
        auth(memory, base, air_base, crate::cair_base::VTABLE_RVA)?;
        row.air_base = field(memory, air_base, crate::cair_base::HANDLE)?;
        row.air_base_maximum = field(memory, air_base, crate::cair_base::CAPACITY)?;
    }
    let equipment = add(ship, crate::cship::EQUIPMENT_POOL)?;
    let variants = crate::cequipment_variant_pool::entries(memory, base, equipment)?;
    for index in 0..variants.count {
        let entry = element(variants, index, 16)?;
        let variant = ptr(memory, entry, 0)?;
        auth(memory, base, variant, crate::cequipment_variant::VTABLE_RVA)?;
        emit(
            SourceRow {
                scope: 27,
                index,
                reference: field(memory, variant, 8)?,
                weights: [
                    field(memory, entry, 8)?,
                    field::<u64>(memory, variant, crate::cequipment_variant::CLASSIFIER_FLAGS)?
                        as i64,
                ],
                ..Default::default()
            },
            None,
        )?;
    }
    visit_modifier(emit, 0, add(country.pointer, 0x5a8)?)?;
    let pride = field::<u64>(memory, country.pointer, 0x250)?;
    if pride == field::<u64>(memory, ship, 8)? {
        let root = ptr(memory, base, 0x033d_e590)?;
        visit_modifier(emit, 1, ptr(memory, root, 0xb0)?)?;
    }
    visit_modifier(emit, 2, add(ship, 0x740)?)?;
    // Medals select their own accumulator and label, rebuilt independently of ship experience.
    let medals = ptr(memory, ship, 0x8f8)?;
    visit_modifier(emit, 3, add(medals, 0x1a0)?)?;
    doctrine_kept::<TERMINAL>(memory, base, task, doctrine_keys, &mut memo, emit)?;
    let location = ptr(memory, task, cunit::LOCATION)?;
    // A later ship of a task force whose location terms an earlier ship
    // read: the same rows and the same remaining reads, in the same order.
    let kept = memo
        .as_deref()
        .filter(|_| location != 0)
        .and_then(|memo| memo.location(task, location))
        .map(|(terms, _)| terms);
    let selected_terrain = if location == 0 {
        None
    } else if let Some(terms) = kept {
        if let Some(accumulator) = terms.relation {
            emit(
                SourceRow {
                    scope: 16,
                    index: terms.owner_id,
                    selector: terms.target,
                    ..Default::default()
                },
                Some(accumulator),
            )?;
        }
        // The terrain name where the first ship read it: after the relation.
        if let Some((_, name)) = memo
            .as_deref()
            .and_then(|memo| memo.location(task, location))
        {
            row.terrain_name = owned_copy(name)?;
        }
        let class = terrain_class_kept(memory, &mut memo, terms.selected, row.ship_class, emit)?;
        emit(
            SourceRow {
                scope: 20,
                selector: row.ship_class,
                weights: class.weights,
                movement: class.movement,
                ..Default::default()
            },
            None,
        )?;
        if (row.ship_class as i32) < 0 {
            mixed_terrain(memory, base, task, terms.selected, emit)?;
        }
        visit_modifier(emit, 6, terms.region)?;
        visit_modifier(emit, 7, add(terms.record, 0x270)?)?;
        row.night = terms.night;
        visit_modifier(emit, 8, add(task, 0x658)?)?;
        Some(terms.selected)
    } else {
        Some(location_terms::<TERMINAL>(
            memory, base, game, task, location, country_id, &mut row, &mut memo, emit,
        )?)
    };
    if let Some(selected) = selected_terrain
        && leader != 0
    {
        let a = array(memory, leader, 0x248, 0x28)?;
        let terrain_index = field::<i32>(memory, selected, 0x88)?;
        let index = if terrain_index >= 0 && (terrain_index as u32) < a.count {
            terrain_index as u32
        } else {
            0
        };
        let adjuster = element(a, index, 0x28)?;
        emit(
            SourceRow {
                scope: 21,
                selector: terrain_index as u32,
                weights: [
                    field(memory, adjuster, 0x10)?,
                    field(memory, adjuster, 0x18)?,
                ],
                ..Default::default()
            },
            None,
        )?;
    }
    if leader != 0 {
        visit_modifier(emit, 9, add(leader, 0x408)?)?;
        visit_modifier(
            emit,
            10,
            class_kept(memory, &mut memo, leader, row.subunit, true)?,
        )?;
        let adjuster = class_kept(memory, &mut memo, leader, row.subunit, false)?;
        emit(
            SourceRow {
                scope: 22,
                selector: row.subunit,
                weights: [
                    field(memory, adjuster, 0x10)?,
                    field(memory, adjuster, 0x18)?,
                ],
                ..Default::default()
            },
            None,
        )?;
        emit(
            SourceRow {
                scope: 23,
                weights: [field(memory, leader, 0x270)?, field(memory, leader, 0x278)?],
                ..Default::default()
            },
            None,
        )?;
    }
    // Strength/org/detection getters pass commander=0; gun/carrier bonus
    // getters pass the combat's leader. Preserve both when they differ.
    if default_leader != 0 {
        auth(memory, base, default_leader, cnavy_leader::VTABLE_RVA)?;
        let reference = field(memory, default_leader, 8)?;
        emit(
            SourceRow {
                scope: 31,
                reference,
                ..Default::default()
            },
            Some(add(default_leader, 0x418)?),
        )?;
        emit(
            SourceRow {
                scope: 32,
                reference,
                selector: row.subunit,
                ..Default::default()
            },
            Some(add(
                class_kept(memory, &mut memo, default_leader, row.subunit, true)?,
                0x10,
            )?),
        )?;
    }
    let criticals = array(memory, ship, 0x918, 16)?;
    for index in 0..criticals.count {
        let p = element(criticals, index, 16)?;
        let definition = ptr(memory, p, 0)?;
        emit(
            SourceRow {
                scope: 11,
                index,
                weights: [i64::from(field::<i32>(memory, p, 8)?), 0],
                ..Default::default()
            },
            Some(add(definition, 0x98)?),
        )?;
    }
    influences_kept(memory, base, task, &mut memo, emit)?;
    globals_kept(memory, base, &mut memo, emit)?;
    Ok(row)
}

#[cfg(test)]
mod tests;
