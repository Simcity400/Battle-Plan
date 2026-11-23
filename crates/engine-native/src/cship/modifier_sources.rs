use crate::{Memory, capture_text::{Owned, Value}, cnaval_combatant::field, cpdx_array::Array};
use engine_sites::Rva;

pub fn ptr(memory: &impl Memory, p: usize, o: usize) -> Result<usize, &'static str> {
    field::<u64>(memory, p, o).map(|n| n as usize)
}
pub fn add(p: usize, o: usize) -> Result<usize, &'static str> {
    p.checked_add(o)
        .filter(|_| p != 0)
        .ok_or("naval weapon pointer invalid")
}
pub fn text<const TERMINAL: bool>(
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
pub fn site(base: usize, key: Rva) -> Result<usize, &'static str> {
    engine_sites::at(base, key).ok_or("naval weapon source site withheld")
}
pub fn auth(memory: &impl Memory, base: usize, p: usize, rva: Rva) -> Result<(), &'static str> {
    if p == 0 || field::<u64>(memory, p, 0)? != site(base, rva)? as u64 {
        return Err("naval weapon source identity mismatch");
    }
    Ok(())
}
pub fn array(memory: &impl Memory, p: usize, o: usize, stride: usize) -> Result<Array, &'static str> {
    let header = field(memory, p, o)?;
    let a = Array::from_header(header, stride).ok_or("naval weapon array invalid")?;
    Ok(a)
}
pub fn element(a: Array, i: u32, stride: usize) -> Result<usize, &'static str> {
    a.element(i, stride)
        .ok_or("naval weapon array element invalid")
}

pub fn terrain_class(
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

pub fn terrain_with_policy<const TERMINAL: bool>(
    memory: &impl Memory,
    base: usize,
    definition: usize,
) -> Result<usize, &'static str> {
    terrain_from_template_with_policy::<TERMINAL>(memory, base, ptr(memory, definition, 0x30)?)
}

pub fn terrain_kept(
    memory: &impl Memory,
    base: usize,
    definition: usize,
    ocean: &mut Option<usize>,
) -> Result<usize, &'static str> {
    terrain_from_template_kept(memory, base, ptr(memory, definition, 0x30)?, ocean)
}

pub fn terrain_from_template_kept(
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

pub fn terrain_from_template_with_policy<const TERMINAL: bool>(
    memory: &impl Memory,
    base: usize,
    template: usize,
) -> Result<usize, &'static str> {
    match template_terrain(memory, base, template)? {
        Some(terrain) => Ok(terrain),
        None => ocean_terrain::<TERMINAL>(memory, base),
    }
}

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

fn ocean_terrain<const TERMINAL: bool>(
    memory: &impl Memory,
    base: usize,
) -> Result<usize, &'static str> {
    let key: [u8; 5] = field(memory, site(base, Rva(0x029f_5b58))?, 0)?;
    let root = ptr(
        memory,
        site(base, crate::cterrain_database::SINGLETON_RVA)?,
        0,
    )?;
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

pub fn selected_class(
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

pub type DoctrineKeys = crate::recent::Recent<u32, [u32; 2], 64>;

pub fn doctrine_keys_kept<const TERMINAL: bool>(
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

pub fn doctrine_keys<const TERMINAL: bool>(
    memory: &impl Memory,
    base: usize,
    selector: u32,
) -> Result<[u32; 2], &'static str> {
    use crate::cpdx_lexer as lexer;
    if field::<u32>(memory, site(base, lexer::WRITER_FLAG_RVA)?, 0)? != 0 {
        return Err("naval doctrine lexer writer active");
    }
    let witness: [u8; 64] = field(memory, lexer::witness_address(base)?, 0)?;
    let _ = lexer::PreparedWitness::from_header(witness)?;
    let maximum = field::<u32>(memory, site(base, lexer::MAX_TOKEN_ID_RVA)?, 0)?;
    let table = ptr(memory, site(base, lexer::TOKEN_NAMES_RVA)?, 0)?;
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
        ptr(memory, site(base, Rva(0x0343_9158))?, 0)?,
        ptr(memory, site(base, Rva(0x0343_9150))?, 0)?,
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
    let head = ptr(memory, site(base, Rva(0x030f_eba8))?, 0)?;
    let mut node = ptr(memory, head, 8)?;
    let mut previous = head;
    let mut result = [0; 2];
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
