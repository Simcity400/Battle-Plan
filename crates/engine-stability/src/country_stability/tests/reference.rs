// Frozen stability reader functions from 32e69e3b01; the census phase
// markers and the extra Scratch field do not change their observations.
#![allow(dead_code)]

use super::super::*;

/// Reads one authenticated country. On error nothing is written.
#[allow(clippy::too_many_arguments)]
pub fn walk_country<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    country: ccountry::Country,
    context: &Context,
    scratch: &mut Scratch,
    input: &mut InputRow,
    lines: &mut [LineRow],
    bytes: &mut [u8],
) -> Result<CountryWalk, &'static str> {
    walk_country_buffers(
        memory,
        base,
        countries,
        country,
        context,
        scratch,
        input,
        Buffer::Fixed(lines),
        Buffer::Fixed(bytes),
        0,
        0,
    )
}

/// Worker-only append. Child/source storage follows the original headers and
/// emitted rows/text grow before copying each native occurrence.
#[allow(clippy::too_many_arguments)]
pub fn walk_country_growing<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    country: ccountry::Country,
    context: &Context,
    scratch: &mut Scratch,
    input: &mut InputRow,
    lines: &mut Vec<LineRow>,
    bytes: &mut Vec<u8>,
    line_start: usize,
    byte_start: usize,
) -> Result<CountryWalk, &'static str> {
    walk_country_buffers(
        memory,
        base,
        countries,
        country,
        context,
        scratch,
        input,
        Buffer::Growing(lines),
        Buffer::Growing(bytes),
        line_start,
        byte_start,
    )
}

#[allow(clippy::too_many_arguments)]
fn walk_country_buffers<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    country: ccountry::Country,
    context: &Context,
    scratch: &mut Scratch,
    input: &mut InputRow,
    lines: Buffer<'_, LineRow>,
    bytes: Buffer<'_, u8>,
    line_start: usize,
    byte_start: usize,
) -> Result<CountryWalk, &'static str> {
    *input = inputs(memory, base, countries, country, context)?;
    let children = read_children(memory, base, country.pointer, context, scratch)?;
    if children != input.children as usize {
        return Err("country modifier tree changed during the walk");
    }
    let candidates = candidates(memory, base, country, context, scratch)?;
    pair(memory, scratch, children, candidates)?;
    emit(
        memory, country.id, context, scratch, children, lines, bytes, line_start, byte_start,
    )
}
fn inputs<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    country: ccountry::Country,
    context: &Context,
) -> Result<InputRow, &'static str> {
    phase!(Fields);
    let c = country.pointer;
    let mut bases = [0_u8; 16];
    let mut penalties = [0_u8; 40];
    let mut coastal = [0_u8; 8];
    let mut children = [0_u8; 16];
    if memory.copy_scattered(
        [
            at(c, ccountry::STABILITY)?,
            at(c, PROPAGANDA_STABILITY_PENALTY)?,
            at(c, COASTAL_PROTECTION_STABILITY)?,
            at(c, MODIFIER_CHILDREN)?,
        ],
        &mut [&mut bases, &mut penalties, &mut coastal, &mut children],
    ) != 4
    {
        return Err("country stability fields unreadable");
    }
    let children = Array::from_header(children, 8).ok_or("country modifier children invalid")?;
    let (at_war, offensive_war, defensive_war, diplomacy) =
        war_flags(memory, base, countries, country)?;
    phase!(Totals);
    let mut totals = [None; TOTALS];
    let (sorted, len) = context.keys.sorted_totals();
    let mut values = [modifier_accumulator::Value::default(); TRACKED];
    modifier_accumulator::lookup_many_at(
        memory,
        at(c, ccountry::MODIFIER_ACCUMULATOR)?,
        &sorted[..len],
        &mut values[..len],
    )?;
    for ((total, key), operands) in totals
        .iter_mut()
        .zip(context.keys.keys)
        .zip(context.operands)
    {
        if let (Some(key), Some(operands)) = (key, operands) {
            let value = values[sorted[..len].iter().position(|k| *k == key).unwrap()];
            *total = Some(if value.present {
                operands.effective(value.raw)
            } else {
                0
            });
        }
    }
    Ok(InputRow {
        country: country.id,
        manpower: if context.manpower_enabled {
            crate::ccountry_manpower::read_authenticated(memory, c, diplomacy)
        } else {
            Default::default()
        },
        stability_raw: i64_at(&bases, 0),
        war_support_raw: i64_at(&bases, 8),
        at_war,
        offensive_war,
        defensive_war,
        ruling_party_popularity_raw: popularity(memory, base, c)?,
        propaganda_stability_q15: i64_at(&penalties, 0),
        bombing_q15: i64_at(&penalties, 8),
        heroes_q15: i64_at(&penalties, 16),
        convoys_q15: i64_at(&penalties, 24),
        propaganda_war_support_q15: i64_at(&penalties, 32),
        coastal_protection_stability_raw: i64_at(&coastal, 0),
        interception_sum_raw: interception(memory, c)?,
        totals,
        children: children.count,
    })
}

/// Ruling-party popularity reached through country political status (`ccountry::POLITICAL_STATUS`),
/// ruling party (`cpolitical_status::RULING_PARTY`) and popularity
/// (`cpolitical_party::POPULARITY`), with authenticated ownership.
fn popularity(memory: &impl Memory, base: usize, country: usize) -> Result<i64, &'static str> {
    phase!(Popularity);
    let status = pointer(memory, at(country, ccountry::POLITICAL_STATUS)?)?;
    let head = read::<[u8; 8]>(memory, status).ok_or("political status unreadable")?;
    if u64::from_le_bytes(head) != at(base, cpolitical_status::VTABLE_RVA)? as u64 {
        return Err("political status vtable mismatch");
    }
    let party = pointer(memory, at(status, cpolitical_status::RULING_PARTY)?)?;
    if read::<u64>(memory, party) != Some(at(base, cpolitical_party::VTABLE_RVA)? as u64) {
        return Err("ruling party vtable mismatch");
    }
    read::<i64>(memory, at(party, cpolitical_party::POPULARITY)?)
        .ok_or("ruling party popularity unreadable")
}

/// Mode-one war, offensive-war and defensive-war predicates over the owned relations (with their
/// count): a war relation (type 0x380a) whose attacker (first when `first_was_instigator`, else
/// second) or defender is this country by same-country identity, and for mode one whose second
/// country's diplomacy status's hosted-war count is below one.
fn war_flags<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    country: ccountry::Country,
) -> Result<(bool, bool, bool, usize), &'static str> {
    phase!(Wars);
    let diplomacy = pointer(memory, at(country.pointer, ccountry::DIPLOMACY_STATUS)?)?;
    let mut head = [0_u8; 8];
    let mut owned = [0_u8; 16];
    let mut owner = [0_u8; 8];
    if memory.copy_scattered(
        [
            diplomacy,
            at(diplomacy, cdiplomacy_status::OWNED_RELATIONS)?,
            at(diplomacy, cdiplomacy_status::COUNTRY)?,
        ],
        &mut [&mut head, &mut owned, &mut owner],
    ) != 3
    {
        return Err("country diplomacy unreadable");
    }
    if u64::from_le_bytes(head) != at(base, cdiplomacy_status::VTABLE_RVA)? as u64 {
        return Err("country diplomacy vtable mismatch");
    }
    if pointer_from(&owner)? != country.pointer {
        return Err("country diplomacy owner is another country");
    }
    let owned = Array::from_header(owned, 8).ok_or("owned relation array invalid")?;
    let war_vtable = at(base, cwar_relation::VTABLE_RVA)? as u64;
    let same = |raw: u32| -> Result<bool, &'static str> {
        Ok(raw == country.id
            || (raw != 0
                && country.id != 0
                && countries.link(raw)? == countries.link(country.id)?))
    };
    let (mut at_war, mut offensive, mut defensive) = (false, false, false);
    pointer_pages(memory, owned, |relation| {
        let header = read::<[u8; 0x18]>(memory, relation).ok_or("relation unreadable")?;
        if u32::from_le_bytes(
            header[crelation::TYPE_ID..crelation::TYPE_ID + 4]
                .try_into()
                .unwrap(),
        ) != cwar_relation::TYPE
        {
            return Ok(());
        }
        if u64::from_le_bytes(header[..8].try_into().unwrap()) != war_vtable {
            return Err("war relation vtable mismatch");
        }
        let first = u32::from_le_bytes(header[0x10..0x14].try_into().unwrap());
        let second = u32::from_le_bytes(header[0x14..0x18].try_into().unwrap());
        let instigator = read::<u8>(memory, at(relation, cwar_relation::FIRST_WAS_INSTIGATOR)?)
            .ok_or("war relation instigator unreadable")?;
        let (attacker, defender) = if instigator != 0 {
            (first, second)
        } else {
            (second, first)
        };
        offensive |= same(attacker)?;
        defensive |= same(defender)?;
        if !at_war {
            let counterpart = countries.resolve(second)?;
            let other = pointer(memory, at(counterpart.pointer, ccountry::DIPLOMACY_STATUS)?)?;
            let hosting = read::<i32>(memory, at(other, DIPLOMACY_HOSTING)?)
                .ok_or("war counterpart diplomacy unreadable")?;
            at_war = hosting < 1;
        }
        Ok(())
    })?;
    Ok((at_war, offensive, defensive, diplomacy))
}

/// Interception history sum from the country's interception history for the complete ring, without
/// calling the game.
fn interception(memory: &impl Memory, country: usize) -> Result<i64, &'static str> {
    phase!(Interception);
    let stats = pointer(memory, at(country, AIR_STATISTICS)?)?;
    let ring = pointer(memory, at(stats, RING)?)?;
    let header = read::<[u8; 0x29]>(memory, ring).ok_or("interception ring unreadable")?;
    let size = i32::from_le_bytes(header[RING_SIZE..RING_SIZE + 4].try_into().unwrap());
    let head = i32::from_le_bytes(header[RING_SIZE + 4..RING_SIZE + 8].try_into().unwrap());
    let wrapped = header[RING_SIZE + 8] != 0;
    let count = (if wrapped { size } else { head }).min(size);
    if count <= 0 {
        return Ok(0);
    }
    let data = pointer_from(&header[RING_DATA..RING_DATA + 8])?;
    let mut sum = 0_i64;
    for index in 0..count {
        let slot = if wrapped {
            head.wrapping_add(index) % size
        } else {
            index
        };
        let slot = usize::try_from(slot).map_err(|_| "interception ring slot negative")?;
        let row = pointer(memory, at(data, slot * 8)?)?;
        let values = pointer(memory, row)?;
        let value = read::<i64>(memory, at(values, RING_COLUMN * 8)?)
            .ok_or("interception value unreadable")?;
        sum = sum
            .wrapping_add(value)
            .clamp(-RING_SATURATION, RING_SATURATION);
    }
    Ok(sum)
}

/// Copies every child's header, display and entries: nonzero-entry
/// signature, display digest and tracked values.
fn read_children(
    memory: &impl Memory,
    base: usize,
    country: usize,
    context: &Context,
    scratch: &mut Scratch,
) -> Result<usize, &'static str> {
    phase!(Children);
    let children = Array::read(memory, at(country, MODIFIER_CHILDREN)?, 8)
        .ok_or("country modifier children unreadable")?;
    let count = children.count as usize;
    if scratch.children.len() < count {
        crate::allocation::resize_default(&mut scratch.children, count)
            .map_err(|_| "stability child storage allocation failed")?;
        crate::allocation::resize(&mut scratch.forward, count, NONE)
            .map_err(|_| "stability pairing storage allocation failed")?;
        crate::allocation::resize(&mut scratch.backward, count, NONE)
            .map_err(|_| "stability pairing storage allocation failed")?;
    }
    let vtable = at(base, cmodifier::VTABLE_RVA)? as u64;
    let mut index = 0;
    // Each page's child heads (vtable, accumulator header, display string
    // header) in one guarded gather instead of one scattered copy per
    // child. Children are processed in order, so a child whose head did not
    // read refuses where the per-child copy did, after every earlier child.
    let mut heads = [0_u8; 64 * CHILD_HEAD_BYTES];
    pointer_pages_gathered(memory, children, &mut heads, |child, head| {
        phase!(Children);
        // The per-child copy's own address checks, in its order.
        at(child, cmodifier::ACCUMULATOR)?;
        let address = at(child, cmodifier::DISPLAY_NAME)?;
        let head = head.ok_or("country modifier child unreadable")?;
        let vtable_word: [u8; 8] = head[..8].try_into().unwrap();
        let accumulator: [u8; 16] = head[8..24].try_into().unwrap();
        let display: [u8; 32] = head[24..56].try_into().unwrap();
        if u64::from_le_bytes(vtable_word) != vtable {
            return Err("country modifier child vtable mismatch");
        }
        let entries = Array::from_header(accumulator, modifier_entry::STRIDE)
            .ok_or("country modifier child entries invalid")?;
        let mut tracked = [0; TRACKED];
        let signature = signature(memory, entries, |key, value| {
            for (slot, tracked_key) in tracked.iter_mut().zip(context.keys.keys) {
                if tracked_key == Some(key) {
                    *slot = value;
                }
            }
        })?;
        let display = display_digest(memory, address, display)?;
        scratch.children[index] = Child {
            pointer: child,
            entries,
            signature,
            display: (address, display.1),
            lines: tracked.iter().filter(|value| **value != 0).count() as u32,
            tracked,
        };
        index += 1;
        Ok(())
    })?;
    Ok(index)
}

/// A child's gathered head: vtable, accumulator header and display string
/// header, as the per-child scattered copy read them.
const CHILD_HEAD_BYTES: usize = 56;
const CHILD_HEAD: [RecordField; 3] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: cmodifier::ACCUMULATOR,
        destination_offset: 8,
        len: 16,
    },
    RecordField {
        source_offset: cmodifier::DISPLAY_NAME,
        destination_offset: 24,
        len: 32,
    },
];
const CHILD_HEAD_LAYOUT: RecordLayout<'static> =
    match RecordLayout::new(&CHILD_HEAD, CHILD_HEAD_BYTES) {
        Some(layout) => layout,
        None => panic!("child head layout"),
    };

/// [`pointer_pages`] that also gathers each page's child heads
/// ([`CHILD_HEAD`]) in one guarded call: `visit` gets each child with its
/// head, `None` from the first child whose head did not read whole.
fn pointer_pages_gathered(
    memory: &impl Memory,
    array: Array,
    heads: &mut [u8; 64 * CHILD_HEAD_BYTES],
    mut visit: impl FnMut(usize, Option<&[u8]>) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let mut bytes = [0; 64 * 8];
    let mut pointers = [0_usize; 64];
    let mut first = 0;
    while first < array.count {
        phase!(Children);
        let count = (array.count - first).min(64) as usize;
        if !read_slice(
            memory,
            array.element(first, 8).ok_or("pointer page overflow")?,
            &mut bytes[..count * 8],
        ) {
            return Err("country stability pointers unreadable");
        }
        // Pointers are validated in order before any head is read, as the
        // per-child walk validated each before its copy.
        let mut valid = 0;
        let mut invalid = None;
        for (target, p) in pointers.iter_mut().zip(bytes[..count * 8].chunks_exact(8)) {
            match pointer_from(p) {
                Ok(pointer) => {
                    *target = pointer;
                    valid += 1;
                }
                Err(reason) => {
                    invalid = Some(reason);
                    break;
                }
            }
        }
        phase!(ChildHeads);
        let fields = memory.copy_records(
            RecordSources::Pointers(&pointers[..valid]),
            &CHILD_HEAD_LAYOUT,
            &mut heads[..valid * CHILD_HEAD_BYTES],
        );
        let complete = fields / CHILD_HEAD.len();
        for (slot, &child) in pointers[..valid].iter().enumerate() {
            let head = (slot < complete)
                .then(|| &heads[slot * CHILD_HEAD_BYTES..(slot + 1) * CHILD_HEAD_BYTES]);
            visit(child, head)?;
        }
        if let Some(reason) = invalid {
            return Err(reason);
        }
        first += count as u32;
    }
    Ok(())
}

/// A validated string's length and digest, streamed without a text-size cap.
fn display_digest(
    memory: &impl Memory,
    address: usize,
    header: [u8; 32],
) -> Result<(usize, u64), &'static str> {
    let descriptor = Descriptor::from_header(memory, address, header)?;
    let mut bytes = [0_u8; 1024];
    let mut digest = 0xcbf2_9ce4_8422_2325_u64;
    for start in (0..descriptor.len).step_by(bytes.len()) {
        let count = (descriptor.len - start).min(bytes.len());
        descriptor.copy_chunk(memory, start, &mut bytes[..count])?;
        for value in &bytes[..count] {
            for byte in u32::from(*value)
                .to_le_bytes()
                .into_iter()
                .chain(0_i64.to_le_bytes())
            {
                digest = (digest ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
            }
        }
    }
    Ok((descriptor.len, digest))
}

/// Digests only reject; matching digests still require exact payload equality.
fn same_text(
    memory: &impl Memory,
    left: usize,
    right: usize,
    expected_len: usize,
) -> Result<bool, &'static str> {
    let left = Descriptor::read(memory, left)?;
    let right = Descriptor::read(memory, right)?;
    if left.len != expected_len || right.len != expected_len {
        return Ok(false);
    }
    let mut a = [0_u8; 1024];
    let mut b = [0_u8; 1024];
    for start in (0..expected_len).step_by(a.len()) {
        let count = (expected_len - start).min(a.len());
        left.copy_chunk(memory, start, &mut a[..count])?;
        right.copy_chunk(memory, start, &mut b[..count])?;
        if a[..count] != b[..count] {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Walks the nonzero entries of a sorted accumulator array, validating the
/// whole array's key order.
fn nonzero(
    memory: &impl Memory,
    entries: Array,
    mut visit: impl FnMut(u32, i64) -> Result<bool, &'static str>,
) -> Result<(), &'static str> {
    let mut bytes = [0_u8; ENTRY_BATCH * modifier_entry::STRIDE];
    let mut previous = 0;
    let mut first = 0;
    while first < entries.count {
        let count = ((entries.count - first) as usize).min(ENTRY_BATCH);
        let address = entries
            .element(first, modifier_entry::STRIDE)
            .ok_or("modifier entry address overflow")?;
        let chunk = &mut bytes[..count * modifier_entry::STRIDE];
        if !read_slice(memory, address, chunk) {
            return Err("modifier entries unreadable");
        }
        for entry in chunk.chunks_exact(modifier_entry::STRIDE) {
            let key = u32::from_le_bytes(entry[..4].try_into().unwrap());
            if key <= previous {
                return Err("modifier entry keys not positive and increasing");
            }
            previous = key;
            let value = i64_at(entry, modifier_entry::VALUE);
            if value != 0 && !visit(key, value)? {
                return Ok(());
            }
        }
        first += count as u32;
    }
    Ok(())
}

fn signature(
    memory: &impl Memory,
    entries: Array,
    mut each: impl FnMut(u32, i64),
) -> Result<Signature, &'static str> {
    let mut signature = Signature::new();
    nonzero(memory, entries, |key, value| {
        signature.add(key, value);
        each(key, value);
        Ok(true)
    })?;
    Ok(signature)
}

/// The replayed sources in country modifier recalculation order.
fn candidates(
    memory: &impl Memory,
    base: usize,
    country: ccountry::Country,
    context: &Context,
    scratch: &mut Scratch,
) -> Result<usize, &'static str> {
    let mut list = Builder {
        candidates: &mut scratch.candidates,
        values: &mut scratch.values,
        keys: context.keys.keys,
        len: 0,
        values_len: 0,
    };
    let c = country.pointer;
    phase!(Ideas);
    // 1-3: ideas, the ruling party's first country leader, its ideology group.
    let status = pointer(memory, at(c, ccountry::POLITICAL_STATUS)?)?;
    let ideas = Array::read(memory, at(status, cpolitical_status::IDEAS)?, 8)
        .ok_or("active idea array invalid")?;
    let primary = at(base, cidea::VTABLE_RVA)? as u64;
    let secondary = at(base, cidea::SECONDARY_VTABLE_RVA)? as u64;
    pointer_pages(memory, ideas, |idea| {
        let vtable = read::<u64>(memory, idea).ok_or("active idea vtable unreadable")?;
        let object = if vtable == secondary {
            idea.checked_sub(cidea::SECONDARY_BASE)
                .ok_or("active idea primary underflow")?
        } else if vtable == primary {
            idea
        } else {
            return Err("active idea vtable mismatch");
        };
        if read::<u64>(memory, object) != Some(primary) {
            return Err("active idea primary vtable mismatch");
        }
        // Political modifier rebuilding adds the array's own pointer plus its entry offset, with a
        // built label.
        list.source(memory, at(idea, IDEA_MODIFIER)?, Kind::Idea, false)?
            .identity(at(object, cidea::TOKEN)?, 0, 0);
        Ok(())
    })?;
    phase!(Leader);
    let party = pointer(memory, at(status, cpolitical_status::RULING_PARTY)?)?;
    let mut leaders = [0_u8; 16];
    let mut group = [0_u8; 8];
    if memory.copy_scattered(
        [
            at(party, cpolitical_party::COUNTRY_LEADERS)?,
            at(party, cpolitical_party::IDEOLOGY_GROUP)?,
        ],
        &mut [&mut leaders, &mut group],
    ) != 2
    {
        return Err("ruling party unreadable");
    }
    let leaders = Array::from_header(leaders, 8).ok_or("ruling party leaders invalid")?;
    if leaders.count != 0 {
        // Country-leader selection: the first element when the count is
        // nonzero.
        let leader = pointer(memory, leaders.data)?;
        let mut head = [0_u8; 16];
        if !read_slice(memory, leader, &mut head)
            || u64::from_le_bytes(head[..8].try_into().unwrap())
                != at(base, ccountry_leader::VTABLE_RVA)? as u64
        {
            return Err("country leader unreadable or vtable mismatch");
        }
        let character = character_id(memory, base, pointer_from(&head[8..])?)?;
        list.source(
            memory,
            at(leader, LEADER_MODIFIER)?,
            Kind::CountryLeader,
            true,
        )?
        .id(character, 0);
    }
    phase!(Ideology);
    let group = pointer_from(&group)?;
    if read::<u64>(memory, group) != Some(at(base, cideology_group::VTABLE_RVA)? as u64) {
        return Err("ruling ideology group vtable mismatch");
    }
    list.source(
        memory,
        at(group, IDEOLOGY_GROUP_MODIFIER)?,
        Kind::Ideology,
        true,
    )?
    .identity(at(group, cideology_group::TOKEN)?, 0, 0);
    phase!(Advisors);
    // 8: appointed advisors, after faction, difficulty and balance-of-power
    // sources this reader does not replay.
    let object = read::<u64>(memory, at(c, ccountry::CHARACTERS)?)
        .ok_or("country characters pointer unreadable")?;
    if object != 0 {
        let object = usize::try_from(object).map_err(|_| "country characters overflow")?;
        let head = read::<[u8; 16]>(memory, object).ok_or("country characters unreadable")?;
        if u64::from_le_bytes(head[..8].try_into().unwrap())
            != at(base, ccountry_characters::VTABLE_RVA)? as u64
            || pointer_from(&head[8..])? != c
        {
            return Err("country characters vtable or owner mismatch");
        }
        let advisors = Array::read(
            memory,
            at(object, ccountry_characters::APPOINTED_ADVISORS)?,
            8,
        )
        .ok_or("appointed advisor array invalid")?;
        let vtable = at(base, cadvisor::VTABLE_RVA)? as u64;
        pointer_pages(memory, advisors, |advisor| {
            let mut head = [0_u8; 24];
            if !read_slice(memory, advisor, &mut head)
                || u64::from_le_bytes(head[..8].try_into().unwrap()) != vtable
            {
                return Err("appointed advisor unreadable or vtable mismatch");
            }
            let character =
                character_id(memory, base, pointer_from(&head[cadvisor::CHARACTER..24])?)?;
            list.source(memory, at(advisor, ADVISOR_MODIFIER)?, Kind::Advisor, true)?
                .identity(at(advisor, cadvisor::IDEA_TOKEN)?, character, 0);
            Ok(())
        })?;
    }
    // 14-15 and 22-24: named statics (their conditions are not replayed; a
    // static that was not added simply pairs with no child). An inactive
    // static could only take another source's child with its exact display
    // and entries; the unreplayed kinds label theirs (decisions and missions
    // with a localised decision or mission prefix, per the
    // country modifier recalculation walk), so that needs content localising a
    // static exactly as such a label.
    for phase in [1, 2] {
        list.statics(memory, context, phase)?;
    }
    phase!(Dynamic);
    // 28: enabled dynamic modifiers, the constant part then the values.
    let container = at(c, ccountry::DYNAMIC_MODIFIERS)?;
    let entries = Array::read(
        memory,
        at(container, cdynamic_modifier_container::ENTRIES)?,
        dynamic_modifier_entry::STRIDE,
    )
    .ok_or("dynamic modifier entries invalid")?;
    for index in 0..entries.count {
        let entry = entries
            .element(index, dynamic_modifier_entry::STRIDE)
            .ok_or("dynamic modifier entry overflow")?;
        let header = dynamic_modifier_entry::header(memory, base, entry)?;
        if header.enabled == 0 {
            continue;
        }
        let definition = header.definition;
        cdynamic_modifier::token(memory, base, definition)?;
        let id = at(definition, cdynamic_modifier::ID)?;
        let scope = (header.scope_country, i64::from(header.scope_state));
        list.source(
            memory,
            at(definition, DYNAMIC_CONSTANT)?,
            Kind::DynamicModifier,
            true,
        )?
        .identity(id, scope.0, scope.1);
        list.values(memory, definition, header.values)?
            .identity(id, scope.0, scope.1);
    }
    // 29-31: the scaled war support and stability statics.
    list.statics(memory, context, 3)?;
    Ok(list.len)
}

struct Builder<'a> {
    candidates: &'a mut Vec<Candidate>,
    values: &'a mut Vec<(u32, i64)>,
    keys: [Option<u32>; TRACKED],
    len: usize,
    values_len: usize,
}

impl Builder<'_> {
    fn push(&mut self, candidate: Candidate) -> Result<&mut Candidate, &'static str> {
        if self.candidates.len() <= self.len {
            crate::allocation::resize_default(
                self.candidates,
                self.len
                    .checked_add(1)
                    .ok_or("stability source count overflow")?,
            )
            .map_err(|_| "stability source storage allocation failed")?;
        }
        let slot = self
            .candidates
            .get_mut(self.len)
            .ok_or("stability source storage invalid")?;
        *slot = candidate;
        self.len += 1;
        Ok(slot)
    }

    /// A CModifier source at `modifier`: its accumulator entries, and its
    /// display string when the child copies it (no label passed).
    fn source(
        &mut self,
        memory: &impl Memory,
        modifier: usize,
        kind: Kind,
        display: bool,
    ) -> Result<&mut Candidate, &'static str> {
        let entries = Array::read(
            memory,
            at(modifier, cmodifier::ACCUMULATOR)?,
            modifier_entry::STRIDE,
        )
        .ok_or("modifier source entries invalid")?;
        let display = if display {
            let address = at(modifier, cmodifier::DISPLAY_NAME)?;
            let header = read::<[u8; 32]>(memory, address).ok_or("source display unreadable")?;
            let (len, digest) = display_digest(memory, address, header)?;
            Some((address, len, digest))
        } else {
            None
        };
        let keys = self.keys;
        let mut tracked = false;
        let signature = signature(memory, entries, |key, _| {
            tracked |= keys.contains(&Some(key));
        })?;
        self.push(Candidate {
            kind,
            content: Content::Entries(entries),
            signature,
            display,
            tracked,
            ..Candidate::default()
        })
    }

    fn statics(
        &mut self,
        memory: &impl Memory,
        context: &Context,
        phase: u8,
    ) -> Result<(), &'static str> {
        for (slot, address) in STATIC_SLOTS.iter().zip(context.statics) {
            if slot.scaled {
                phase!(ScaledStatics);
            } else {
                phase!(Statics);
            }
            if slot.phase != phase || address == 0 {
                continue;
            }
            let kind = if slot.scaled {
                Kind::StaticScaled
            } else {
                Kind::Static
            };
            let candidate = self.source(memory, address, kind, true)?;
            candidate.scaled = slot.scaled;
            candidate.identity = Some(at(address, STATIC_NAME)?);
        }
        Ok(())
    }

    /// The evaluated part of a dynamic modifier: each nonzero value keyed by its row's modifier
    /// key, summed per key in key order as the stack accumulator holds them during dynamic value
    /// recalculation. Its child's display is the localized definition id; the constructor stores
    /// the same localization of that key at the constant part's display, set even when the constant
    /// part is empty. The child must carry that text, so a same-valued child of another source (a
    /// landmark with the same bonus) cannot take it; after a runtime language change the two could
    /// differ and the line stays unpaired.
    fn values(
        &mut self,
        memory: &impl Memory,
        definition: usize,
        values: Array,
    ) -> Result<&mut Candidate, &'static str> {
        let count = cdynamic_modifier::value_count(memory, definition)?;
        if count != values.count {
            return Err("dynamic modifier values do not match their definition");
        }
        let rows = pointer(memory, at(definition, DYNAMIC_VALUE_ROWS)?)
            .or_else(|error| if count == 0 { Ok(0) } else { Err(error) })?;
        let start = self.values_len;
        let required = start
            .checked_add(count as usize)
            .ok_or("dynamic modifier value count overflow")?;
        if self.values.len() < required {
            crate::allocation::resize_default(self.values, required)
                .map_err(|_| "stability value storage allocation failed")?;
        }
        for index in 0..count as usize {
            let value = read::<i64>(memory, at(values.data, index * 8)?)
                .ok_or("dynamic modifier value unreadable")?;
            if value == 0 {
                continue;
            }
            let key = read::<i32>(memory, at(rows, index * DYNAMIC_VALUE_ROW_STRIDE)?)
                .ok_or("dynamic modifier value key unreadable")?;
            let key = u32::try_from(key).map_err(|_| "dynamic modifier value key negative")?;
            let entries = &mut self.values[start..self.values_len];
            if let Some(entry) = entries.iter_mut().find(|entry| entry.0 == key) {
                entry.1 = entry.1.wrapping_add(value);
                continue;
            }
            *self
                .values
                .get_mut(self.values_len)
                .ok_or("dynamic modifier value storage invalid")? = (key, value);
            self.values_len += 1;
        }
        let entries = &mut self.values[start..self.values_len];
        entries.sort_unstable_by_key(|entry| entry.0);
        let mut signature = Signature::new();
        let mut tracked = false;
        for (key, value) in entries.iter().filter(|entry| entry.1 != 0) {
            signature.add(*key, *value);
            tracked |= self.keys.contains(&Some(*key));
        }
        let len = self.values_len - start;
        let address = at(at(definition, DYNAMIC_CONSTANT)?, cmodifier::DISPLAY_NAME)?;
        let header =
            read::<[u8; 32]>(memory, address).ok_or("dynamic modifier display unreadable")?;
        let (display_len, digest) = display_digest(memory, address, header)?;
        let display = Some((address, display_len, digest));
        self.push(Candidate {
            kind: Kind::DynamicModifierValues,
            content: Content::Values { start, len },
            signature,
            display,
            tracked,
            ..Candidate::default()
        })
    }
}
/// A CCharacter's id behind a role's character pointer, authenticated.
fn character_id(memory: &impl Memory, base: usize, character: usize) -> Result<u32, &'static str> {
    let identity = read::<[u8; 16]>(memory, character).ok_or("character unreadable")?;
    if u64::from_le_bytes(identity[..8].try_into().unwrap())
        != at(base, ccharacter::VTABLE_RVA)? as u64
        || u32::from_le_bytes(
            identity[ccharacter::TYPE..ccharacter::ID]
                .try_into()
                .unwrap(),
        ) != ccharacter::REFERENCE_TYPE
    {
        return Err("character identity mismatch");
    }
    Ok(u32::from_le_bytes(
        identity[ccharacter::ID..16].try_into().unwrap(),
    ))
}

/// Whether a child is the candidate's clone: equal nonzero entries (and equal
/// display text where the child copies it); a scaled static by display alone.
fn matches(
    memory: &impl Memory,
    values: &[(u32, i64)],
    child: &Child,
    candidate: &Candidate,
) -> Result<bool, &'static str> {
    if let Some((address, len, digest)) = candidate.display {
        let (child_address, child_digest) = child.display;
        if child_digest != digest {
            return Ok(false);
        }
        if !same_text(memory, child_address, address, len)? {
            return Ok(false);
        }
    }
    if !candidate.scaled && child.signature != candidate.signature {
        return Ok(false);
    }
    // Exact verification of every nonzero entry, in order.
    let mut expected = [(0_u32, 0_i64); 64];
    let mut equal = true;
    let mut seen = 0_usize;
    match candidate.content {
        Content::None => return Ok(false),
        Content::Values { start, len } => {
            let mut wanted = values[start..start + len].iter().filter(|e| e.1 != 0);
            nonzero(memory, child.entries, |key, value| {
                equal = wanted.next() == Some(&(key, value));
                Ok(equal)
            })?;
            equal &= wanted.next().is_none();
        }
        Content::Entries(source) => {
            // Compare in windows of the source's nonzero entries.
            let mut offset = 0;
            while equal {
                let mut filled = 0;
                let mut skipped = 0;
                nonzero(memory, source, |key, value| {
                    if skipped < offset {
                        skipped += 1;
                        return Ok(true);
                    }
                    expected[filled] = (key, value);
                    filled += 1;
                    Ok(filled < expected.len())
                })?;
                let mut position = 0;
                let mut index = 0;
                nonzero(memory, child.entries, |key, value| {
                    if position < offset {
                        position += 1;
                        return Ok(true);
                    }
                    if index == filled {
                        return Ok(false);
                    }
                    equal &= expected[index].0 == key
                        && (candidate.scaled || expected[index].1 == value);
                    index += 1;
                    position += 1;
                    Ok(equal)
                })?;
                if equal && index != filled {
                    equal = false;
                }
                seen += filled;
                offset += filled;
                if filled < expected.len() {
                    break;
                }
            }
            equal &= seen == child.signature.count as usize;
        }
    }
    Ok(equal)
}

/// Leftmost and rightmost order-preserving assignments; a child is paired
/// only when both choose the same source.
fn pair(
    memory: &impl Memory,
    scratch: &mut Scratch,
    children: usize,
    candidates: usize,
) -> Result<(), &'static str> {
    phase!(Matching);
    let Scratch {
        children: child_rows,
        candidates: sources,
        values,
        forward,
        backward,
        ..
    } = scratch;
    // Only children with lines and candidates with a tracked key take part: a
    // child with a tracked value can only equal such a candidate, while the
    // others (empty signatures above all: a tooltip-only source still gets a
    // child, the effective-empty predicate ignores values, and an idea with no
    // modifier or a dynamic modifier with no nonzero value gets none) would
    // otherwise match each other and move the bounds across real sources.
    let takes_part = |source: &Candidate| source.tracked;
    let mut next = 0;
    for index in 0..children {
        forward[index] = NONE;
        if child_rows[index].lines == 0 {
            continue;
        }
        for (source, candidate) in sources.iter().enumerate().take(candidates).skip(next) {
            if takes_part(candidate) && matches(memory, values, &child_rows[index], candidate)? {
                forward[index] = source as u32;
                next = source + 1;
                break;
            }
        }
    }
    let mut limit = candidates;
    for index in (0..children).rev() {
        backward[index] = NONE;
        if child_rows[index].lines == 0 {
            continue;
        }
        for (source, candidate) in sources[..limit].iter().enumerate().rev() {
            if takes_part(candidate) && matches(memory, values, &child_rows[index], candidate)? {
                backward[index] = source as u32;
                limit = source;
                break;
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit(
    memory: &impl Memory,
    country: u32,
    context: &Context,
    scratch: &Scratch,
    children: usize,
    mut lines: Buffer<'_, LineRow>,
    bytes: Buffer<'_, u8>,
    line_start: usize,
    byte_start: usize,
) -> Result<CountryWalk, &'static str> {
    phase!(Emission);
    let wanted = scratch.children[..children]
        .iter()
        .try_fold(0_u32, |total, child| {
            total
                .checked_add(child.lines)
                .ok_or("stability line count overflow")
        })?;
    let mut len = 0;
    let required_lines = line_start
        .checked_add(wanted as usize)
        .ok_or("stability line count overflow")?;
    let fits = lines.ensure(required_lines)?;
    let mut arena = Arena {
        bytes,
        used: byte_start,
        required: byte_start,
        fits,
    };
    for (line, child) in scratch.children[..children].iter().enumerate() {
        if child.lines == 0 {
            continue;
        }
        let paired = (scratch.forward[line] != NONE
            && scratch.forward[line] == scratch.backward[line])
            .then(|| scratch.candidates[scratch.forward[line] as usize]);
        let (kind, identity_address, identity_id, secondary) = match paired {
            Some(source) => (
                source.kind,
                source.identity,
                source.identity_id,
                source.secondary,
            ),
            None => (Kind::Unpaired, None, 0, 0),
        };
        let identity = match identity_address {
            Some(address) => arena.copy(memory, address)?,
            None => Span::default(),
        };
        let text = if kind == Kind::Unpaired {
            arena.copy(memory, at(child.pointer, cmodifier::DISPLAY_NAME)?)?
        } else {
            Span::default()
        };
        let fits = arena.fits;
        for (value, key) in child.tracked.iter().zip(context.keys.keys) {
            let (Some(key), true) = (key, *value != 0) else {
                continue;
            };
            if fits {
                lines[line_start + len] = LineRow {
                    country,
                    line: line as u32,
                    key,
                    kind,
                    identity,
                    identity_id,
                    secondary,
                    value: *value,
                    text,
                };
                len += 1;
            }
        }
    }
    Ok(if arena.fits {
        CountryWalk {
            lines: Outcome::Rows { len },
            bytes_len: arena.used - byte_start,
            required_bytes: arena.required - byte_start,
        }
    } else {
        CountryWalk {
            lines: Outcome::Truncated {
                len: 0,
                unread_rows: wanted,
            },
            bytes_len: 0,
            required_bytes: arena.required - byte_start,
        }
    })
}

/// The walk's string arena for one country's lines.
struct Arena<'a> {
    bytes: Buffer<'a, u8>,
    used: usize,
    required: usize,
    fits: bool,
}

impl Arena<'_> {
    /// Copies an exact std::string; once anything does not fit, only the
    /// requirement grows.
    fn copy(&mut self, memory: &impl Memory, address: usize) -> Result<Span, &'static str> {
        let descriptor = Descriptor::read(memory, address)?;
        let start = self.used;
        self.required = self
            .required
            .checked_add(descriptor.len)
            .ok_or("stability line bytes overflow")?;
        if self.fits
            && (matches!(&self.bytes, Buffer::Growing(_)) || self.bytes.ensure(self.required)?)
        {
            descriptor.copy_buffer(memory, &mut self.bytes, start)?;
            self.used += descriptor.len;
        } else {
            self.fits = false;
        }
        Ok(Span {
            start,
            len: descriptor.len,
        })
    }
}
fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("country stability address overflow")
}
fn pointer_from(bytes: &[u8]) -> Result<usize, &'static str> {
    usize::try_from(u64::from_le_bytes(bytes[..8].try_into().unwrap()))
        .ok()
        .filter(|p| *p != 0)
        .ok_or("country stability pointer null or invalid")
}
fn pointer(memory: &impl Memory, address: usize) -> Result<usize, &'static str> {
    pointer_from(&read::<[u8; 8]>(memory, address).ok_or("country stability pointer unreadable")?)
}
fn i64_at(bytes: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}
fn pointer_pages(
    memory: &impl Memory,
    array: Array,
    mut visit: impl FnMut(usize) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let mut bytes = [0; 64 * 8];
    let mut first = 0;
    while first < array.count {
        let count = (array.count - first).min(64);
        if !read_slice(
            memory,
            array.element(first, 8).ok_or("pointer page overflow")?,
            &mut bytes[..count as usize * 8],
        ) {
            return Err("country stability pointers unreadable");
        }
        for p in bytes[..count as usize * 8].chunks_exact(8) {
            visit(pointer_from(p)?)?;
        }
        first += count;
    }
    Ok(())
}
