use engine_native::{ccharacter, cunit_leader};
use engine_native_memory::{Memory, read, walk::Buffer};
use engine_sites::Rva;

pub const ROLE_VTABLE_RVA: Rva = Rva(0x02a3_a8a8);
pub const ROLE_VISIBLE: usize = 0x48;
pub const CHARACTER_TEMPLATE_ROLE: usize = 0x230;

#[expect(
    clippy::type_complexity,
    reason = "Retain visibility status and the independently optional filename span and source line."
)]
pub fn visibility(
    memory: &impl Memory,
    base: usize,
    leader: usize,
    character: usize,
    names: &mut Buffer<'_, u8>,
    used: &mut usize,
) -> Result<(u8, Option<(usize, usize, u32)>), &'static str> {
    let at = |p: usize, n: usize| p.checked_add(n).ok_or("unit leader visibility address overflow");
    let word = |p| {
        read::<u64>(memory, p)
            .and_then(|n| usize::try_from(n).ok())
            .ok_or("unit leader visibility pointer unreadable")
    };
    let mut role = word(at(leader, cunit_leader::VISIBILITY_CHOICE)?)?;
    if role == 0 {
        let linked = read::<u8>(memory, at(leader, cunit_leader::TEMPLATE_VISIBILITY)?)
            .ok_or("unit leader template link unreadable")?;
        if linked != 0 {
            let template = word(at(character, ccharacter::TEMPLATE)?)?;
            if template != 0 {
                role = word(at(template, CHARACTER_TEMPLATE_ROLE)?)?;
            }
        }
    }
    if role == 0 {
        return Ok((0, None));
    }
    let vtable = engine_sites::at(base, ROLE_VTABLE_RVA).ok_or("unit leader role identity mismatch")?;
    if read::<u64>(memory, role) != Some(vtable as u64) {
        return Err("unit leader role identity mismatch");
    }
    crate::cscientist::visible_origin(memory, base, at(role, ROLE_VISIBLE)?, names, used)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    const LEADER: usize = 0x100;
    const CHARACTER: usize = 0x2000;
    const TEMPLATE: usize = 0x3000;
    const ROLE: usize = 0x4000;

    fn memory() -> Vec<u8> {
        let mut bytes = vec![0; 0x5000];
        put(&mut bytes, ROLE, ROLE_VTABLE_RVA.0 as u64);
        put(&mut bytes, ROLE + ROLE_VISIBLE, crate::cscientist::VISIBLE_VTABLE_RVA.0 as u64);
        bytes
    }

    fn put(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn read_from(bytes: &[u8]) -> Result<(u8, Option<(usize, usize, u32)>), &'static str> {
        let memory = SliceMemory { base: 0, bytes };
        let mut names = Vec::new();
        let mut used = 0;
        visibility(&memory, 0, LEADER, CHARACTER, &mut Buffer::Growing(&mut names), &mut used)
    }

    #[test]
    fn a_leader_with_no_role_choice_and_no_template_role_is_shown_as_the_game_shows_it() {
        let mut bytes = memory();
        assert_eq!(read_from(&bytes), Ok((0, None)), "no choice and no template link");
        bytes[LEADER + cunit_leader::TEMPLATE_VISIBILITY] = 1;
        put(&mut bytes, CHARACTER + ccharacter::TEMPLATE, TEMPLATE as u64);
        assert_eq!(read_from(&bytes), Ok((0, None)), "a linked template that has no unit leader role, as generate_character makes");
    }

    #[test]
    fn the_leader_s_own_role_wins_and_else_its_character_template_s_role_is_read() {
        let mut bytes = memory();
        put(&mut bytes, LEADER + cunit_leader::VISIBILITY_CHOICE, ROLE as u64);
        assert_eq!(read_from(&bytes), Ok((2, None)), "a role whose visible is empty is always shown");
        let mut linked = memory();
        linked[LEADER + cunit_leader::TEMPLATE_VISIBILITY] = 1;
        put(&mut linked, CHARACTER + ccharacter::TEMPLATE, TEMPLATE as u64);
        put(&mut linked, TEMPLATE + CHARACTER_TEMPLATE_ROLE, ROLE as u64);
        assert_eq!(read_from(&linked), Ok((2, None)));
        put(&mut linked, ROLE, 0x1234);
        assert_eq!(read_from(&linked), Err("unit leader role identity mismatch"));
    }
}
