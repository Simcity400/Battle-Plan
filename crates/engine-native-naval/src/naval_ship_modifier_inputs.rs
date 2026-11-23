pub use engine_native::cship::modifier_sources::{
    DoctrineKeys,
    doctrine_keys,
    doctrine_keys_kept,
    selected_class,
    terrain,
    terrain_class,
    terrain_from_template_kept,
    terrain_kept,
    terrain_with_policy,
};
use engine_native::cship::modifier_sources::{add, array, auth, element, ptr, site, text};
use engine_native_memory::{Memory, capture_text::Owned, cpdx_array::Array};
use engine_native::cnaval_combatant::field;
use engine_sites::Rva;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ContextRow {
    pub task_force: u64,
    pub country: u32,
    pub commander: u64,
    pub ship_class: u32,
    pub subunit: u32,
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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SourceRow {
    pub scope: u8,
    pub index: u32,
    pub selector: u32,
    pub reference: u64,
    pub weights: [i64; 2],
    pub movement: i64,
    pub dynamic_keys: [u32; 2],
    pub terms: [usize; 2],
    pub accumulator: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ModifierTerm {
    pub key: u32,
    pub raw: i64,
    pub metadata_flags: u32,
    pub feature_selector: u32,
}

pub struct InfluenceBuffers<'a> {
    pub sources: &'a mut Vec<SourceRow>,
    pub terms: &'a mut Vec<ModifierTerm>,
    pub lengths: &'a mut [usize; 2],
    pub required: &'a mut [usize; 2],
}

pub struct TermMetadata {
    keep: bool,
    table: Option<Array>,
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
        let table = engine_native_parts::modifier_definition::table(memory, base)?;
        self.table = Some(table).filter(|_| self.keep);
        Ok(table)
    }

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
        let bytes: [u8; 12] = field(memory, at, engine_native_parts::modifier_definition::FLAGS)?;
        if !self.keep {
            return Ok(bytes);
        }
        if !self.sized {
            self.sized = true;
            if engine_allocation::resize_default(&mut self.rows, table.count as usize).is_err() {
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

struct Capture<'a, 'b, M> {
    memory: &'a M,
    base: usize,
    buffers: InfluenceBuffers<'b>,
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
            engine_allocation::reserve(self.buffers.sources, 1)
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
        engine_allocation::reserve_push_batch(self.buffers.terms, count)
            .map_err(|_| "collector allocation failed")?;
        self.buffers.sources.truncate(source);
        engine_allocation::reserve(self.buffers.sources, 1)
            .map_err(|_| "collector allocation failed")?;
        if let Some(entries) = data {
            use engine_native_parts::modifier_definition as md;
            let table = match self.metadata.as_deref_mut() {
                Some(metadata) => metadata.table(self.memory, self.base)?,
                None => md::table(self.memory, self.base)?,
            };
            let mut previous = 0;
            let mut bytes = [0u8; 64 * 16];
            for start in (0..count).step_by(64) {
                let n = (count - start).min(64);
                if !engine_native_memory::read_slice(
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

#[derive(Default)]
pub struct SourceMemo {
    tasks: engine_native_memory::recent::Recent<usize, (usize, usize), 16>,
    rows: Vec<(SourceRow, Option<usize>)>,
    leaders: engine_native_memory::recent::Recent<usize, usize, 8>,
    classes: engine_native_memory::recent::Recent<(usize, u32, bool), usize, 16>,
    countries: engine_native_memory::recent::Recent<(usize, u32), engine_native::ccountry::Country, 8>,
    locations: Vec<((usize, usize), LocationTerms, Owned)>,
    terrain_classes: engine_native_memory::recent::Recent<(usize, u32), TerrainClass, 16>,
    influences: engine_native_memory::recent::Recent<usize, (usize, usize), 16>,
    globals: Option<([i64; 2], i64, u32)>,
}

const LOCATION_SLOTS: usize = 16;

#[derive(Clone, Copy)]
struct LocationTerms {
    owner_id: u32,
    target: u32,
    relation: Option<usize>,
    selected: usize,
    region: usize,
    record: usize,
    night: i64,
}

#[derive(Clone, Copy)]
struct TerrainClass {
    modifier: usize,
    weights: [i64; 2],
    movement: i64,
}

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
        } else if engine_allocation::reserve(&mut self.locations, 1).is_err() {
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

fn mixed_terrain(
    memory: &impl Memory,
    base: usize,
    task: usize,
    selected: usize,
    emit: &mut impl FnMut(SourceRow, Option<usize>) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    use engine_native::cship;
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

#[allow(clippy::too_many_arguments)]
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
    use engine_native::ccountry;
    auth(memory, base, location, engine_native::cprovince::VTABLE_RVA)?;
    let province = field::<u32>(memory, location, engine_native::cprovince::ID)?;
    let expeditionary = field::<i32>(memory, task, engine_native::cunit::EXPEDITIONARY_OWNER)?;
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
        engine_native::cdiplomacy_status::VTABLE_RVA,
    )?;
    let relations = array(memory, diplomacy, engine_native::cdiplomacy_status::RELATIONS, 8)?;
    let relation = ptr(memory, element(relations, target_slot, 8)?, 0)?;
    let relation = if relation != 0 {
        auth(memory, base, relation, engine_native::crelation::status::VTABLE_RVA)?;
        if ptr(memory, relation, engine_native::crelation::status::DIPLOMACY_STATUS)? != diplomacy {
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
    let weather = ptr(memory, game, engine_native::cgame_state::WEATHER_MANAGER)?;
    auth(memory, base, weather, engine_native::cweather_manager::VTABLE_RVA)?;
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
        if kept && engine_allocation::reserve(rows, 1).is_ok() {
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
        if kept && engine_allocation::reserve(rows, 1).is_ok() {
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

fn globals_kept(
    memory: &impl Memory,
    base: usize,
    memo: &mut Option<&mut SourceMemo>,
    emit: &mut impl FnMut(SourceRow, Option<usize>) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    use engine_native_parts::modifier_definition as md;
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
        field(memory, site(base, Rva(0x033d_fa90))?, 0)?,
        field(memory, site(base, Rva(0x033d_fb30))?, 0)?,
    ];
    emit(defines(weights), None)?;
    let callback = field::<u64>(memory, site(base, md::ELIGIBILITY_CALLBACK_RVA)?, 0)?;
    let eligibility = if callback == 0 {
        0
    } else if callback == site(base, md::ELIGIBILITY_FLAGS_GETTER_RVA)? as u64 {
        1
    } else {
        return Err("naval weapon modifier callback unsupported");
    };
    let flags = field::<u32>(memory, site(base, md::TRADE_FEATURE_FLAGS_RVA)?, 0)?;
    emit(features(flags, eligibility), None)?;
    if let Some(memo) = memo.as_deref_mut() {
        memo.globals = Some((weights, eligibility, flags));
    }
    Ok(())
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
    let Some(character) = engine_native_parts::creference_registry::resolve(memory, base, reference)? else {
        return Ok(());
    };
    let mut country = ptr(memory, character, 0xa8)?;
    if country == 0 {
        country =
            engine_native_parts::creference_registry::resolve(memory, base, field(memory, character, 0xc8)?)?
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
    let mut tree = engine_native_memory::msvc_tree_node::Cursor::read(memory, task, 0x640)
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

pub(crate) fn read(
    memory: &impl Memory,
    base: usize,
    game: usize,
    ship: usize,
    side: usize,
    buffers: &mut crate::naval_member_details::DetailBuffers<'_>,
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

#[allow(clippy::too_many_arguments)]
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
    use engine_native::{ccountry, cship, ctask_force, cunit};
    use engine_native_parts::cnavy_leader;
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
            engine_native_parts::creference_registry::resolve(memory, base, field(memory, fleet, 0xb0)?)?
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
        auth(memory, base, air_base, engine_native_air::cair_base::VTABLE_RVA)?;
        row.air_base = field(memory, air_base, engine_native_air::cair_base::HANDLE)?;
        row.air_base_maximum = field(memory, air_base, engine_native_air::cair_base::CAPACITY)?;
    }
    let equipment = add(ship, engine_native::cship::EQUIPMENT_POOL)?;
    let variants = engine_native::cequipment_variant::pool::entries(memory, base, equipment)?;
    for index in 0..variants.count {
        let entry = element(variants, index, 16)?;
        let variant = ptr(memory, entry, 0)?;
        auth(memory, base, variant, engine_native::cequipment_variant::VTABLE_RVA)?;
        emit(
            SourceRow {
                scope: 27,
                index,
                reference: field(memory, variant, 8)?,
                weights: [
                    field(memory, entry, 8)?,
                    field::<u64>(memory, variant, engine_native::cequipment_variant::CLASSIFIER_FLAGS)?
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
        let root = ptr(memory, site(base, Rva(0x033d_e590))?, 0)?;
        visit_modifier(emit, 1, ptr(memory, root, 0xb0)?)?;
    }
    visit_modifier(emit, 2, add(ship, 0x740)?)?;
    let medals = ptr(memory, ship, 0x8f8)?;
    visit_modifier(emit, 3, add(medals, 0x1a0)?)?;
    doctrine_kept::<TERMINAL>(memory, base, task, doctrine_keys, &mut memo, emit)?;
    let location = ptr(memory, task, cunit::LOCATION)?;
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
