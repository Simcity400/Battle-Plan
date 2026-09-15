//! Ordered raw source operands of ship modifier getter140C29020 and weapon
//! getter140C25310. No modifier total, doctrine weight, fuel fraction or weapon
//! damage is evaluated. Source indices preserve native list order.
use crate::{Memory, cnaval_combatant::field, cpdx_array::Array, msvc_string::Text};

const LIMIT: u32 = 4096;
pub const CAPACITY: &str = crate::cfex_member::DETAIL_ARENA_CAPACITY;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ContextRow {
    pub task_force: u64,
    pub country: u32,
    pub commander: u64,
    pub ship_class: u32,
    pub subunit: u32,
    /// Native getter cases0..4 at ship+1E8,+1F8,+210,+208,+318.
    pub weapon_bases: [i64; 5],
    pub weather_factor: i64,
    pub fuel: [i64; 2],
    pub night: i64,
    pub formation_flags: u64,
    pub detection_base: i64,
    pub air_base: u64,
    pub air_base_maximum: i32,
    pub air_detection_base: i64,
    pub terrain_name: Text<256>,
    pub sources: [usize; 2],
}

/// Scopes0 country,1 pride,2 ship,3 experience,4 doctrine,5 terrain,
///6 region weather,7 province weather,8 task force,9 commander,
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
    /// Doctrine country modifiers selected by141956CB0/141956D40.
    pub dynamic_keys: [u32; 2],
    pub terms: [usize; 2],
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ModifierTerm {
    pub key: u32,
    pub raw: i64,
    pub metadata_flags: u32,
    pub feature_selector: u32,
}

/// Independent caller-owned storage for active or retained influence sources.
pub struct InfluenceBuffers<'a> {
    pub sources: &'a mut [SourceRow],
    pub terms: &'a mut [ModifierTerm],
    pub lengths: &'a mut [usize; 2],
    pub required: &'a mut [usize; 2],
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
    };
    capture.influence(wrapper, index)?;
    Ok([start, capture.buffers.lengths[0] - start])
}

fn ptr(memory: &impl Memory, p: usize, o: usize) -> Result<usize, &'static str> {
    field::<u64>(memory, p, o).map(|n| n as usize)
}
fn add(p: usize, o: usize) -> Result<usize, &'static str> {
    p.checked_add(o)
        .filter(|_| p != 0)
        .ok_or("naval weapon pointer invalid")
}
fn auth(memory: &impl Memory, base: usize, p: usize, rva: usize) -> Result<(), &'static str> {
    if p == 0 || field::<u64>(memory, p, 0)? != add(base, rva)? as u64 {
        return Err("naval weapon source identity mismatch");
    }
    Ok(())
}
fn array(
    memory: &impl Memory,
    p: usize,
    o: usize,
    stride: usize,
) -> Result<(Array, [u8; 16]), &'static str> {
    let header = field(memory, p, o)?;
    let a = Array::from_header(header, stride).ok_or("naval weapon array invalid")?;
    if a.count > 65536 {
        return Err("naval weapon array bound exceeded");
    }
    Ok((a, header))
}
fn element(a: Array, i: u32, stride: usize) -> Result<usize, &'static str> {
    a.element(i, stride)
        .ok_or("naval weapon array element invalid")
}

struct Capture<'a, 'b, M> {
    memory: &'a M,
    base: usize,
    buffers: InfluenceBuffers<'b>,
}
impl<M: Memory> Capture<'_, '_, M> {
    fn influence(&mut self, wrapper: usize, index: u32) -> Result<(), &'static str> {
        let memory = self.memory;
        let influence = ptr(memory, wrapper, 0x40)?;
        if influence == 0 {
            return Ok(());
        }
        let reference = field(memory, influence, 8)?;
        let Some(character) = crate::creference_registry::resolve(memory, self.base, reference)?
        else {
            return Ok(());
        };
        let mut country = ptr(memory, character, 0xa8)?;
        if country == 0 {
            country = crate::creference_registry::resolve(
                memory,
                self.base,
                field(memory, character, 0xc8)?,
            )?
            .unwrap_or(0);
        }
        if country == 0 || field::<i32>(memory, country, 0xe7c)? != 2 {
            return Ok(());
        }
        let mut insufficient = false;
        for (lane, offset) in [0xf70, 0xf80, 0xfa0, 0xf90].into_iter().enumerate() {
            let definition = ptr(memory, country, offset)?;
            match self.source(
                SourceRow {
                    scope: 12 + lane as u8,
                    index,
                    reference,
                    ..Default::default()
                },
                Some(add(definition, 0xe0)?),
            ) {
                Ok(()) => {}
                Err(CAPACITY) => insufficient = true,
                Err(reason) => return Err(reason),
            }
        }
        if field::<u64>(memory, influence, 8)? != reference
            || ptr(memory, wrapper, 0x40)? != influence
        {
            return Err("naval influence source changed");
        }
        if insufficient { Err(CAPACITY) } else { Ok(()) }
    }
    fn source(
        &mut self,
        mut row: SourceRow,
        accumulator: Option<usize>,
    ) -> Result<(), &'static str> {
        let source = self.buffers.lengths[0];
        self.buffers.required[0] = self.buffers.required[0]
            .checked_add(1)
            .ok_or("naval weapon source demand overflow")?;
        let data = accumulator
            .map(|p| array(self.memory, p, 0, 16))
            .transpose()?;
        let count = data.map_or(0, |(a, _)| a.count as usize);
        self.buffers.required[1] = self.buffers.required[1]
            .checked_add(count)
            .ok_or("naval weapon term demand overflow")?;
        let first = self.buffers.lengths[1];
        let end = first
            .checked_add(count)
            .ok_or("naval weapon term length overflow")?;
        if source >= self.buffers.sources.len() || end > self.buffers.terms.len() {
            return Err(CAPACITY);
        }
        row.terms = [first, count];
        if let Some((entries, header)) = data {
            use crate::modifier_definition as md;
            let table = ptr(self.memory, self.base, md::TABLE_RVA)?;
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
                for (index, b) in bytes[..n * 16].chunks_exact(16).enumerate() {
                    let key = u32::from_le_bytes(b[..4].try_into().unwrap());
                    if key <= previous || key > 65536 {
                        return Err("naval weapon modifier keys invalid");
                    }
                    previous = key;
                    let at = add(table, key as usize * md::STRIDE)?;
                    let metadata: [u8; 12] = field(self.memory, at, md::FLAGS)?;
                    self.buffers.terms[first + start + index] = ModifierTerm {
                        key,
                        raw: i64::from_le_bytes(b[8..16].try_into().unwrap()),
                        metadata_flags: u32::from_le_bytes(metadata[..4].try_into().unwrap()),
                        feature_selector: u32::from_le_bytes(metadata[8..].try_into().unwrap()),
                    };
                }
            }
            if field::<[u8; 16]>(self.memory, accumulator.unwrap(), 0)? != header
                || ptr(self.memory, self.base, md::TABLE_RVA)? != table
            {
                return Err("naval weapon modifier source changed");
            }
        }
        self.buffers.sources[source] = row;
        *self.buffers.lengths = [source + 1, end];
        Ok(())
    }
    fn modifier(&mut self, scope: u8, p: usize) -> Result<(), &'static str> {
        self.source(
            SourceRow {
                scope,
                ..Default::default()
            },
            Some(add(p, 0x10)?),
        )
    }
}

/// Native terrain selector14142AF80/14142C0E0; selecting a source is not
/// composition. Both selected and default records preserve their raw values.
fn terrain_class(
    memory: &impl Memory,
    terrain: usize,
    class: u32,
) -> Result<(usize, usize), &'static str> {
    let header: [u8; 17] = field(memory, terrain, 0x1b8)?;
    let data = u64::from_le_bytes(header[..8].try_into().unwrap()) as usize;
    let mask = u32::from_le_bytes(header[12..16].try_into().unwrap());
    let buckets = mask
        .checked_add(1)
        .filter(|n| n.is_power_of_two() && *n <= 65536)
        .ok_or("naval terrain class hash invalid")?;
    let extent = buckets + u32::from(header[16]);
    let mut hash = (class >> 16 ^ class).wrapping_mul(0x45d9f3b);
    hash = (hash >> 16 ^ hash).wrapping_mul(0x45d9f3b);
    hash ^= hash >> 16;
    let mut selected = None;
    for probe in 1..=255u32 {
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
        if probe == 255 {
            return Err("naval terrain class probe bound exceeded");
        }
    }
    let value = if let Some(index) = selected {
        let (a, _) = array(memory, terrain, 0x198, 0x110)?;
        let entry = element(a, index, 0x110)?;
        (add(entry, 0x28)?, add(entry, 0xe8)?)
    } else {
        (add(terrain, 0xa8)?, add(terrain, 0x168)?)
    };
    if field::<[u8; 17]>(memory, terrain, 0x1b8)? != header {
        return Err("naval terrain class changed");
    }
    Ok(value)
}

pub(crate) fn terrain(
    memory: &impl Memory,
    base: usize,
    definition: usize,
) -> Result<usize, &'static str> {
    let terrain = ptr(memory, ptr(memory, definition, 0x30)?, 0x130)?;
    if terrain != 0 && field::<u8>(memory, terrain, 0x10)? != 0 {
        auth(memory, base, terrain, crate::cterrain_type::VTABLE_RVA)?;
        return Ok(terrain);
    }
    //141593150 falls back to the five-byte terrain key at142962540. Capture
    // that key from the pinned module instead of assuming English content.
    let key: [u8; 5] = field(memory, base, 0x0296_2540)?;
    let root = ptr(memory, base, crate::cterrain_database::SINGLETON_RVA)?;
    let (types, header) = array(memory, root, 0x40, 8)?;
    let mut found = None;
    for index in 0..types.count {
        let p = ptr(memory, element(types, index, 8)?, 0)?;
        if field::<u8>(memory, p, 0x10)? == 0 {
            continue;
        }
        auth(memory, base, p, crate::cterrain_type::VTABLE_RVA)?;
        let name: Text<256> = crate::msvc_string::read_text(memory, add(p, 0x18)?)
            .ok_or("naval terrain name invalid")?;
        if name.as_bytes() == key && found.replace(p).is_some() {
            return Err("naval terrain duplicate fallback");
        }
    }
    if field::<[u8; 16]>(memory, root, 0x40)? != header
        || ptr(memory, base, crate::cterrain_database::SINGLETON_RVA)? != root
    {
        return Err("naval terrain database changed");
    }
    found.ok_or("naval terrain fallback absent")
}

fn selected_class(
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
    let (a, header) = array(memory, leader, offset, stride)?;
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
    if field::<[u8; 16]>(memory, leader, offset)? != header {
        return Err("naval commander class changed");
    }
    Ok(found.unwrap_or(add(leader, fallback)?))
}

/// Select the two native country keys without calculating either modifier.
///141471D80 constructs prefix+lexer(type)+suffix and maps its lexer id through
/// the immutable modifier-key dictionary. We compare the raw string segments
/// while walking that bounded dictionary; no engine lexer method or lock runs.
fn doctrine_keys(
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
    let text = |token: u32| -> Result<Text<256>, &'static str> {
        if token > maximum {
            return Err("naval doctrine token outside dictionary");
        }
        crate::msvc_string::read_text(memory, add(table, token as usize * 32)?)
            .ok_or("naval doctrine token string invalid")
    };
    let selected = text(selector)?;
    let generators = [
        ptr(memory, base, 0x0337_2490)?,
        ptr(memory, base, 0x0337_2488)?,
    ];
    let mut prefixes = [Text::<256>::default(); 2];
    let mut suffixes = [Text::<256>::default(); 2];
    for lane in 0..2 {
        prefixes[lane] = crate::msvc_string::read_text(memory, add(generators[lane], 0x10)?)
            .ok_or("naval doctrine prefix invalid")?;
        suffixes[lane] = crate::msvc_string::read_text(memory, add(generators[lane], 0x30)?)
            .ok_or("naval doctrine suffix invalid")?;
    }
    let head = ptr(memory, base, 0x0306_ebd8)?;
    let mut node = ptr(memory, head, 8)?;
    let mut previous = head;
    let mut result = [0; 2];
    let mut ended = false;
    for _ in 0..65536 {
        if node == head {
            ended = true;
            break;
        }
        if ptr(memory, node, 0)? != previous {
            return Err("naval doctrine modifier dictionary link invalid");
        }
        let name = text(field(memory, node, 0x10)?)?;
        for lane in 0..2 {
            let p = prefixes[lane].as_bytes();
            let s = selected.as_bytes();
            let t = suffixes[lane].as_bytes();
            let n = name.as_bytes();
            if n.len() == p.len() + s.len() + t.len()
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
    if !ended {
        return Err("naval doctrine modifier dictionary bound exceeded");
    }
    if field::<u32>(memory, base, lexer::WRITER_FLAG_RVA)? != 0
        || field::<[u8; 64]>(memory, base, lexer::INITIALIZED_RVA)? != witness
        || ptr(memory, base, 0x0306_ebd8)? != head
        || [
            ptr(memory, base, 0x0337_2490)?,
            ptr(memory, base, 0x0337_2488)?,
        ] != generators
    {
        return Err("naval doctrine dictionary changed");
    }
    Ok(result)
}

impl<M: Memory> Capture<'_, '_, M> {
    fn doctrine(&mut self, task: usize) -> Result<(), &'static str> {
        let head = ptr(self.memory, task, 0x640)?;
        if field::<u8>(self.memory, head, 0x19)? != 1 {
            return Err("naval doctrine tree sentinel invalid");
        }
        let mut node = ptr(self.memory, head, 0)?;
        let mut previous = None;
        for index in 0..LIMIT {
            if node == head {
                if ptr(self.memory, task, 0x640)? != head {
                    return Err("naval doctrine tree changed");
                }
                return Ok(());
            }
            if field::<u8>(self.memory, node, 0x19)? != 0 {
                return Err("naval doctrine node invalid");
            }
            let key = field::<i32>(self.memory, node, 0x20)?;
            if previous.is_some_and(|v| v >= key) {
                return Err("naval doctrine order invalid");
            }
            previous = Some(key);
            let modifier = ptr(self.memory, node, 0x30)?;
            let dynamic_keys = doctrine_keys(self.memory, self.base, key as u32)?;
            self.source(
                SourceRow {
                    scope: 4,
                    index,
                    selector: key as u32,
                    dynamic_keys,
                    weights: [
                        i64::from(field::<i32>(self.memory, node, 0x2c)?),
                        i64::from(field::<i32>(self.memory, node, 0x28)?),
                    ],
                    ..Default::default()
                },
                Some(add(modifier, 0x10)?),
            )?;
            let right = ptr(self.memory, node, 0x10)?;
            if field::<u8>(self.memory, right, 0x19)? == 0 {
                node = right;
                let mut bounded = false;
                for _ in 0..LIMIT {
                    let left = ptr(self.memory, node, 0)?;
                    if field::<u8>(self.memory, left, 0x19)? != 0 {
                        bounded = true;
                        break;
                    }
                    node = left;
                }
                if !bounded {
                    return Err("naval doctrine descent bound exceeded");
                }
            } else {
                let mut bounded = false;
                for _ in 0..LIMIT {
                    let parent = ptr(self.memory, node, 8)?;
                    if field::<u8>(self.memory, parent, 0x19)? != 0
                        || ptr(self.memory, parent, 0x10)? != node
                    {
                        node = parent;
                        bounded = true;
                        break;
                    }
                    node = parent;
                }
                if !bounded {
                    return Err("naval doctrine ascent bound exceeded");
                }
            }
        }
        Err("naval doctrine count bound exceeded")
    }
}

/// Called only for an authenticated live CShip. References, source selection
/// and arrays are checked again before publishing the containing member row.
#[allow(clippy::too_many_arguments)]
pub(crate) fn read(
    memory: &impl Memory,
    base: usize,
    game: usize,
    ship: usize,
    side: usize,
    buffers: &mut crate::cfex_member::DetailBuffers<'_>,
) -> Result<ContextRow, &'static str> {
    use crate::{ccountry, cnavy_leader, cship, ctask_force, cunit};
    auth(memory, base, ship, cship::VTABLE_RVA)?;
    let task = ptr(memory, ship, 0x728)?;
    auth(memory, base, task, ctask_force::VTABLE_RVA)?;
    let task_ref = field(memory, task, cunit::HANDLE)?;
    let country_id = field(memory, task, cunit::ROSTER_OWNER)?;
    let country = ccountry::Reader::new(memory, base, game)?.resolve(country_id)?;
    let fleet = ptr(memory, task, 0x340)?;
    let default_leader = if fleet == 0 {
        0
    } else {
        crate::creference_registry::resolve(memory, base, field(memory, fleet, 0xb0)?)?.unwrap_or(0)
    };
    let mut leader = ptr(memory, side, 0x110)?;
    if leader == 0 {
        leader = default_leader;
    }
    let commander = if leader == 0 {
        0
    } else {
        auth(memory, base, leader, cnavy_leader::VTABLE_RVA)?;
        field(memory, leader, 8)?
    };
    let start = buffers.weapon_lengths[0];
    let mut row = ContextRow {
        task_force: task_ref,
        country: country_id,
        commander,
        ship_class: field(memory, ship, 0x88)?,
        subunit: field(memory, ship, 0x60c)?,
        weather_factor: field(memory, ship, 0x270)?,
        fuel: [field(memory, task, 0x4f8)?, field(memory, task, 0x500)?],
        formation_flags: field(memory, ship, 0x628)?,
        detection_base: field(memory, ship, 0x1b8)?,
        air_detection_base: field(memory, ship, 0x1e0)?,
        ..Default::default()
    };
    for (index, offset) in [0x1e8, 0x1f8, 0x210, 0x208, 0x318].into_iter().enumerate() {
        row.weapon_bases[index] = field(memory, ship, offset)?;
    }
    let mut cap = Capture {
        memory,
        base,
        buffers: InfluenceBuffers {
            sources: buffers.weapon_sources,
            terms: buffers.weapon_terms,
            lengths: &mut buffers.weapon_lengths,
            required: &mut buffers.weapon_required,
        },
    };
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
        cap.source(
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
    if crate::cequipment_variant_pool::entries(memory, base, equipment)? != variants {
        return Err("naval ship equipment variants changed");
    }
    cap.modifier(0, add(country.pointer, 0x5a8)?)?;
    let pride = field::<u64>(memory, country.pointer, 0x250)?;
    if pride == field::<u64>(memory, ship, 8)? {
        let root = ptr(memory, base, 0x0331_7cc8)?;
        cap.modifier(1, ptr(memory, root, 0xb0)?)?;
        if ptr(memory, base, 0x0331_7cc8)? != root {
            return Err("naval pride modifier root changed");
        }
    }
    cap.modifier(2, add(ship, 0x740)?)?;
    let experience = ptr(memory, ship, 0x8f8)?;
    cap.modifier(3, add(experience, 0x1a0)?)?;
    cap.doctrine(task)?;
    let location = ptr(memory, task, cunit::LOCATION)?;
    if location != 0 {
        auth(memory, base, location, crate::cprovince::VTABLE_RVA)?;
        let province = field::<u32>(memory, location, crate::cprovince::ID)?;
        //140C25310 chooses expeditionary owner when positive, then the
        // relation matrix entry for the location's controller (ordinal0 when
        // unowned). Keep this separate from roster-country modifier scope0.
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
        let (relations, relations_header) =
            array(memory, diplomacy, crate::cdiplomacy_status::RELATIONS, 8)?;
        let relation = ptr(memory, element(relations, target_slot, 8)?, 0)?;
        if relation != 0 {
            auth(memory, base, relation, crate::crelation_status::VTABLE_RVA)?;
            if ptr(memory, relation, crate::crelation_status::DIPLOMACY_STATUS)? != diplomacy {
                return Err("naval weapon relation owner mismatch");
            }
            cap.source(
                SourceRow {
                    scope: 16,
                    index: owner_id,
                    selector: target,
                    ..Default::default()
                },
                Some(add(relation, 0x140)?),
            )?;
        }
        if field::<[u8; 16]>(memory, diplomacy, crate::cdiplomacy_status::RELATIONS)?
            != relations_header
            || ptr(memory, owner.pointer, ccountry::DIPLOMACY_STATUS)? != diplomacy
        {
            return Err("naval weapon diplomacy source changed");
        }
        let definition = ptr(memory, location, 0xc8)?;
        let selected = terrain(memory, base, definition)?;
        row.terrain_name = crate::msvc_string::read_text(memory, add(selected, 0x18)?)
            .ok_or("naval selected terrain name invalid")?;
        let (terrain_modifier, terrain_adjuster) = terrain_class(memory, selected, row.ship_class)?;
        cap.modifier(5, terrain_modifier)?;
        cap.source(
            SourceRow {
                scope: 20,
                selector: row.ship_class,
                weights: [
                    field(memory, terrain_adjuster, 0x10)?,
                    field(memory, terrain_adjuster, 0x18)?,
                ],
                ..Default::default()
            },
            None,
        )?;
        // Negative class delegates to an unweighted average of each task-force
        // ship's terrain adjuster (140D56980/140D56D60), not an opaque cache.
        if (row.ship_class as i32) < 0 {
            let (ships, header) = array(memory, task, 0x348, 8)?;
            for index in 0..ships.count {
                let p = ptr(memory, element(ships, index, 8)?, 0)?;
                auth(memory, base, p, cship::VTABLE_RVA)?;
                let class = field(memory, p, 0x88)?;
                let (_, adjuster) = terrain_class(memory, selected, class)?;
                cap.source(
                    SourceRow {
                        scope: 25,
                        index,
                        selector: class,
                        reference: field(memory, p, 8)?,
                        weights: [
                            field(memory, adjuster, 0x10)?,
                            field(memory, adjuster, 0x18)?,
                        ],
                        ..Default::default()
                    },
                    None,
                )?;
            }
            if field::<[u8; 16]>(memory, task, 0x348)? != header {
                return Err("naval terrain fleet list changed");
            }
        }
        let weather = ptr(memory, game, crate::cgame_state::WEATHER_MANAGER)?;
        auth(memory, base, weather, crate::cweather_manager::VTABLE_RVA)?;
        let region = field::<u32>(memory, definition, 0x58)?;
        let (regions, region_header) = array(memory, weather, 0x40, 0x160)?;
        cap.modifier(6, add(element(regions, region, 0x160)?, 0x48)?)?;
        let (provinces, province_header) = array(memory, weather, 0x370, 0x330)?;
        let p = element(provinces, province, 0x330)?;
        cap.modifier(7, add(p, 0x270)?)?;
        row.night = field(memory, p, 0x240)?;
        cap.modifier(8, add(task, 0x658)?)?;
        if leader != 0 {
            let (a, header) = array(memory, leader, 0x248, 0x28)?;
            let terrain_index = field::<i32>(memory, selected, 0x88)?;
            let index = if terrain_index >= 0 && (terrain_index as u32) < a.count {
                terrain_index as u32
            } else {
                0
            };
            let adjuster = element(a, index, 0x28)?;
            cap.source(
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
            if field::<[u8; 16]>(memory, leader, 0x248)? != header {
                return Err("naval commander terrain changed");
            }
        }
        if field::<[u8; 16]>(memory, weather, 0x40)? != region_header
            || field::<[u8; 16]>(memory, weather, 0x370)? != province_header
            || ptr(memory, game, crate::cgame_state::WEATHER_MANAGER)? != weather
            || ptr(memory, task, cunit::LOCATION)? != location
        {
            return Err("naval weapon weather location changed");
        }
    }
    if leader != 0 {
        cap.modifier(9, add(leader, 0x408)?)?;
        cap.modifier(10, selected_class(memory, leader, row.subunit, true)?)?;
        let adjuster = selected_class(memory, leader, row.subunit, false)?;
        cap.source(
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
        cap.source(
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
        cap.source(
            SourceRow {
                scope: 31,
                reference,
                ..Default::default()
            },
            Some(add(default_leader, 0x418)?),
        )?;
        cap.source(
            SourceRow {
                scope: 32,
                reference,
                selector: row.subunit,
                ..Default::default()
            },
            Some(add(
                selected_class(memory, default_leader, row.subunit, true)?,
                0x10,
            )?),
        )?;
    }
    let (criticals, crit_header) = array(memory, ship, 0x918, 16)?;
    for index in 0..criticals.count {
        let p = element(criticals, index, 16)?;
        let definition = ptr(memory, p, 0)?;
        cap.source(
            SourceRow {
                scope: 11,
                index,
                weights: [i64::from(field::<i32>(memory, p, 8)?), 0],
                ..Default::default()
            },
            Some(add(definition, 0x98)?),
        )?;
    }
    let (influences, influence_header) = array(memory, task, 0x728, 8)?;
    for index in 0..influences.count {
        let wrapper = ptr(memory, element(influences, index, 8)?, 0)?;
        cap.influence(wrapper, index)?;
    }
    cap.source(
        SourceRow {
            scope: 24,
            weights: [
                field(memory, base, 0x0331_bea0)?,
                field(memory, base, 0x0331_bf40)?,
            ],
            ..Default::default()
        },
        None,
    )?;
    use crate::modifier_definition as md;
    let callback = field::<u64>(memory, base, md::ELIGIBILITY_CALLBACK_RVA)?;
    let eligibility = if callback == 0 {
        0
    } else if callback == add(base, md::ELIGIBILITY_FLAGS_GETTER_RVA)? as u64 {
        1
    } else {
        return Err("naval weapon modifier callback unsupported");
    };
    cap.source(
        SourceRow {
            scope: 26,
            weights: [
                i64::from(field::<u32>(memory, base, md::TRADE_FEATURE_FLAGS_RVA)?),
                eligibility,
            ],
            ..Default::default()
        },
        None,
    )?;
    row.sources = [start, cap.buffers.lengths[0] - start];
    if field::<[u8; 16]>(memory, ship, 0x918)? != crit_header
        || field::<[u8; 16]>(memory, task, 0x728)? != influence_header
        || ptr(memory, ship, 0x728)? != task
        || ptr(memory, ship, 0x8f8)? != experience
        || field::<u64>(memory, task, cunit::HANDLE)? != task_ref
        || field::<u32>(memory, task, cunit::ROSTER_OWNER)? != country_id
    {
        return Err("naval weapon source graph changed");
    }
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    const IMAGE: usize = 0x1000_0000;
    struct Fixture {
        bytes: Vec<u8>,
        calls: Cell<usize>,
        copied: Cell<usize>,
    }
    impl Memory for Fixture {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.calls.set(self.calls.get() + 1);
            self.copied.set(self.copied.get() + output.len());
            if address == IMAGE + crate::modifier_definition::TABLE_RVA && output.len() == 8 {
                output.copy_from_slice(&0x4000u64.to_le_bytes());
                return true;
            }
            crate::memory::SliceMemory {
                base: 0x1000,
                bytes: &self.bytes,
            }
            .copy(address, output)
        }
    }
    fn fixture() -> Fixture {
        let mut f = Fixture {
            bytes: vec![0; 0x7000],
            calls: Cell::new(0),
            copied: Cell::new(0),
        };
        let mut put =
            |p: usize, b: &[u8]| f.bytes[p - 0x1000..p - 0x1000 + b.len()].copy_from_slice(b);
        put(0x1800, &0x2000u64.to_le_bytes());
        put(0x1808, &3u32.to_le_bytes());
        put(0x180c, &2u32.to_le_bytes());
        for (i, key, value) in [(0usize, 0x2au32, i64::MIN), (1, 0x2b, i64::MAX)] {
            put(0x2000 + i * 16, &key.to_le_bytes());
            put(0x2008 + i * 16, &value.to_le_bytes());
            put(
                0x4000
                    + key as usize * crate::modifier_definition::STRIDE
                    + crate::modifier_definition::FLAGS,
                &0x81234567u32.to_le_bytes(),
            );
        }
        f
    }
    #[test]
    fn native_modifier_sources_preserve_raw_values_metadata_and_exact_capacity_demand() {
        let memory = fixture();
        let mut sources = [SourceRow::default(); 1];
        let mut terms = [ModifierTerm::default(); 2];
        let mut lengths = [0; 2];
        let mut required = [0; 2];
        let mut cap = Capture {
            memory: &memory,
            base: IMAGE,
            buffers: InfluenceBuffers {
                sources: &mut sources,
                terms: &mut terms,
                lengths: &mut lengths,
                required: &mut required,
            },
        };
        cap.source(
            SourceRow {
                scope: 2,
                ..Default::default()
            },
            Some(0x1800),
        )
        .unwrap();
        assert_eq!(lengths, [1, 2]);
        assert_eq!(required, [1, 2]);
        assert_eq!(sources[0].terms, [0, 2]);
        assert_eq!([terms[0].raw, terms[1].raw], [i64::MIN, i64::MAX]);
        assert_eq!(terms[0].metadata_flags, 0x81234567);
        println!(
            "naval weapon synthetic one source/two terms: {} copies/{} bytes; source+term pool {} bytes",
            memory.calls.get(),
            memory.copied.get(),
            std::mem::size_of_val(&sources) + std::mem::size_of_val(&terms)
        );
        let mut lengths = [4, 3];
        let mut required = [0; 2];
        let before = memory.calls.get();
        let mut cap = Capture {
            memory: &memory,
            base: IMAGE,
            buffers: InfluenceBuffers {
                sources: &mut [],
                terms: &mut [],
                lengths: &mut lengths,
                required: &mut required,
            },
        };
        assert_eq!(
            cap.source(SourceRow::default(), Some(0x1800)),
            Err(CAPACITY)
        );
        assert_eq!(required, [1, 2]);
        assert_eq!(lengths, [4, 3]);
        assert_eq!(
            memory.calls.get() - before,
            1,
            "capacity preflight reads only source header"
        );
    }
}
