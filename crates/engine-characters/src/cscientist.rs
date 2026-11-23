
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x029f_b128);
pub const TRAIT_DATA: usize = 0x30;
pub const TRAIT_CAPACITY: usize = 0x38;
pub const TRAIT_COUNT: usize = 0x3c;
pub const SKILLS: usize = 0x110;
pub const SKILL_COUNT: usize = 0x11c;
pub const TRAIT_VTABLE_RVA: Rva = Rva(0x0279_9590);
pub const TRAIT_TOKEN: usize = 0x08;
pub const VISIBILITY_TEMPLATE: usize = 0x128;
pub const TEMPLATE_VTABLE_RVA: Rva = Rva(0x027e_3720);
pub const TEMPLATE_VISIBLE: usize = 0x60;
pub const VISIBLE_VTABLE_RVA: Rva = Rva(0x0279_64c8);
const VISIBLE_CHILDREN: usize = 0x08;
const VISIBLE_SOURCE: usize = 0x2c;
const VISIBLE_HAS_SOURCE: usize = 0x34;
const SOURCE_FILES_RVA: Rva = Rva(0x0369_19e0);

#[allow(
    clippy::type_complexity,
    reason = "Retain visibility status and the independently optional filename span and source line."
)]
pub fn visibility(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    scientist: usize,
    names: &mut engine_native_memory::walk::Buffer<'_, u8>,
    used: &mut usize,
) -> Result<(u8, Option<(usize, usize, u32)>), &'static str> {
    use engine_native_memory::{cpdx_array::Array, read};
    let at = |p: usize, n: usize| {
        p.checked_add(n)
            .ok_or("scientist visibility address overflow")
    };
    let word = |p| {
        read::<u64>(memory, p)
            .and_then(|n| usize::try_from(n).ok())
            .ok_or("scientist visibility pointer unreadable")
    };
    let template = word(at(scientist, VISIBILITY_TEMPLATE)?)?;
    if template == 0 {
        return Ok((0, None));
    }
    let template_vtable = engine_sites::at(base, TEMPLATE_VTABLE_RVA)
        .ok_or("scientist retained template identity mismatch")?;
    if read::<u64>(memory, template) != Some(template_vtable as u64) {
        return Err("scientist retained template identity mismatch");
    }
    let visible = at(template, TEMPLATE_VISIBLE)?;
    let visible_vtable = engine_sites::at(base, VISIBLE_VTABLE_RVA)
        .ok_or("scientist visible conjunction identity mismatch")?;
    if read::<u64>(memory, visible) != Some(visible_vtable as u64) {
        return Err("scientist visible conjunction identity mismatch");
    }
    let children = Array::read(memory, at(visible, VISIBLE_CHILDREN)?, 8)
        .ok_or("scientist visible children invalid")?;
    if children.count == 0 {
        return Ok((2, None));
    }
    if read::<u8>(memory, at(visible, VISIBLE_HAS_SOURCE)?) != Some(1) {
        return Ok((1, None));
    }
    let source = read::<[u8; 8]>(memory, at(visible, VISIBLE_SOURCE)?)
        .ok_or("scientist visibility origin unreadable")?;
    let source = [
        u32::from_le_bytes(source[..4].try_into().unwrap()),
        u32::from_le_bytes(source[4..].try_into().unwrap()),
    ];
    let pool = word(
        engine_sites::at(base, SOURCE_FILES_RVA).ok_or("scientist visibility address overflow")?,
    )?;
    let size = read::<i32>(memory, pool).ok_or("source filename pool size unreadable")?;
    if size < 0 || source[0] >= size as u32 {
        return Err("source filename index invalid");
    }
    let sentinel = word(at(pool, 8)?)?;
    let blocks =
        read::<u64>(memory, at(pool, 16)?).ok_or("source filename block count unreadable")?;
    let first = word(sentinel)?;
    let first_data = word(at(first, 16)?)?;
    let first_capacity = read::<i32>(memory, first_data)
        .filter(|n| *n > 0)
        .ok_or("source filename first capacity invalid")?;
    let mut node = word(at(sentinel, 8)?)?;
    let mut string = None;
    for _ in 0..blocks {
        if node == sentinel || node == 0 {
            break;
        }
        let data = word(at(node, 16)?)?;
        let header = read::<[u8; 8]>(memory, data).ok_or("source filename block unreadable")?;
        let header = [
            i32::from_le_bytes(header[..4].try_into().unwrap()),
            i32::from_le_bytes(header[4..].try_into().unwrap()),
        ];
        let start = header[0]
            .checked_sub(first_capacity)
            .filter(|n| *n >= 0)
            .ok_or("source filename block capacity invalid")?;
        if source[0] >= start as u32 {
            let index = source[0] - start as u32;
            if header[1] < 0 || header[1] > header[0] || index >= header[1] as u32 {
                return Err("source filename block index invalid");
            }
            string = Some(at(
                data,
                8usize
                    .checked_add(
                        (index as usize)
                            .checked_mul(32)
                            .ok_or("source filename offset overflow")?,
                    )
                    .ok_or("source filename offset overflow")?,
            )?);
            break;
        }
        node = word(at(node, 8)?)?;
    }
    let string = engine_native_memory::msvc_string::Descriptor::read(
        memory,
        string.ok_or("source filename block absent")?,
    )?;
    let end = used
        .checked_add(string.len)
        .ok_or("scientist visibility filename overflow")?;
    if !names.ensure(end)? {
        return Err("scientist visibility filename arena insufficient");
    }
    string.copy(memory, &mut names[*used..end])?;
    let origin = (*used, string.len, source[1]);
    *used = end;
    Ok((1, Some(origin)))
}
pub const ACTIVE: usize = 0x130;
pub const INJURY_DAYS: usize = 0x134;
pub const SIZE: usize = 0x138;

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::{Memory, memory::SliceMemory, walk::Buffer};
    struct Fixture(Vec<u8>);
    impl Memory for Fixture {
        fn copy(&self, at: usize, out: &mut [u8]) -> bool {
            if at == SOURCE_FILES_RVA.0 && out.len() == 8 {
                out.copy_from_slice(&0xe00u64.to_le_bytes());
                return true;
            }
            SliceMemory {
                base: 0,
                bytes: &self.0,
            }
            .copy(at, out)
        }
    }
    fn put64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn put32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn fixture() -> Fixture {
        let mut bytes = vec![0; 0x2000];
        put64(&mut bytes, 0x200 + VISIBILITY_TEMPLATE, 0x400);
        put64(&mut bytes, 0x400, TEMPLATE_VTABLE_RVA.0 as u64);
        let visible = 0x400 + TEMPLATE_VISIBLE;
        put64(&mut bytes, visible, VISIBLE_VTABLE_RVA.0 as u64);
        put64(&mut bytes, visible + VISIBLE_CHILDREN, 0x900);
        put32(&mut bytes, visible + VISIBLE_CHILDREN + 8, 1);
        put32(&mut bytes, visible + VISIBLE_CHILDREN + 12, 1);
        put32(&mut bytes, visible + VISIBLE_SOURCE, 0);
        put32(&mut bytes, visible + VISIBLE_SOURCE + 4, 17);
        bytes[visible + VISIBLE_HAS_SOURCE] = 1;
        put32(&mut bytes, 0xe00, 1);
        put64(&mut bytes, 0xe08, 0x1000);
        put64(&mut bytes, 0xe10, 1);
        put64(&mut bytes, 0x1000, 0x1020);
        put64(&mut bytes, 0x1008, 0x1020);
        put64(&mut bytes, 0x1020, 0x1000);
        put64(&mut bytes, 0x1028, 0x1000);
        put64(&mut bytes, 0x1030, 0x1100);
        put32(&mut bytes, 0x1100, 4);
        put32(&mut bytes, 0x1104, 1);
        bytes[0x1108..0x1116].copy_from_slice(b"events/mod.txt");
        put64(&mut bytes, 0x1118, 14);
        put64(&mut bytes, 0x1120, 15);
        Fixture(bytes)
    }
    #[test]
    fn retained_role_origin_is_authenticated_and_empty_visibility_needs_no_origin() {
        let mut memory = fixture();
        let mut names = Vec::new();
        let mut used = 0;
        let observed = visibility(
            &memory,
            0,
            0x200,
            &mut Buffer::Growing(&mut names),
            &mut used,
        )
        .unwrap();
        assert_eq!(observed, (1, Some((0, 14, 17))));
        assert_eq!(&names[..used], b"events/mod.txt");
        put32(
            &mut memory.0,
            0x400 + TEMPLATE_VISIBLE + VISIBLE_CHILDREN + 12,
            0,
        );
        memory.0[0x400 + TEMPLATE_VISIBLE + VISIBLE_HAS_SOURCE] = 0;
        assert_eq!(
            visibility(&memory, 0, 0x200, &mut Buffer::Count, &mut 0).unwrap(),
            (2, None)
        );
        put64(&mut memory.0, 0x400, 0);
        assert!(visibility(&memory, 0, 0x200, &mut Buffer::Count, &mut 0).is_err());
    }
}
