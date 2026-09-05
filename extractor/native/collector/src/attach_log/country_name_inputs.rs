use super::*;
use compact_contract::country_name_inputs::{self as wire, EffectiveNameTag, NameInputRefusal as Refusal, NameToken};

fn pointer<M: DecodeMemory + ?Sized>(mem: &M, at: usize) -> Result<usize, Refusal> {
    let value = mem.u64(at).ok_or(Refusal::Unreadable)? as usize;
    if !is_map_control_heap_pointer(value) { return Err(Refusal::InvalidIdentity); }
    Ok(value)
}

fn token<M: DecodeMemory + ?Sized>(mem: &M, at: usize) -> Result<MsvcStringSnapshot, Refusal> {
    read_msvc_string_snapshot_once_in(mem, at, wire::MAX_NAME_TOKEN_BYTES)
        .ok_or(Refusal::Unreadable)
}

fn utf8(snapshot: &MsvcStringSnapshot) -> Result<String, Refusal> {
    String::from_utf8(snapshot.bytes.clone()).map_err(|_| Refusal::InvalidUtf8)
}

fn leader_once<M: DecodeMemory + ?Sized>(mem: &M, country: usize, vtable: usize) -> Result<(Vec<u64>, NameToken), Refusal> {
    let module = vtable.checked_sub(COUNTRY_VTABLE_RVA).ok_or(Refusal::InvalidIdentity)?;
    let country_vtable = mem.u64(country).ok_or(Refusal::Unreadable)?;
    let country_id = mem.u32(country + OBJ_TAG_DB_ID).ok_or(Refusal::Unreadable)?;
    let status = pointer(mem, country + 0xf90)?;
    let status_vtable = mem.u64(status).ok_or(Refusal::Unreadable)?;
    let party = pointer(mem, status + POLITICAL_STATUS_RULING_PARTY)?;
    let party_vtable = mem.u64(party).ok_or(Refusal::Unreadable)?;
    if country_vtable as usize != vtable || status_vtable as usize != module + POLITICAL_STATUS_VTABLE_RVA
        || party_vtable as usize != module + POLITICAL_PARTY_VTABLE_RVA {
        return Err(Refusal::InvalidIdentity);
    }
    let data = mem.u64(party + POLITICAL_PARTY_LEADER_VEC).ok_or(Refusal::Unreadable)?;
    let capacity = mem.u32(party + POLITICAL_PARTY_LEADER_VEC + 8).ok_or(Refusal::Unreadable)?;
    let count = mem.u32(party + POLITICAL_PARTY_LEADER_VEC + 12).ok_or(Refusal::Unreadable)?;
    if count > capacity || count > i32::MAX as u32 { return Err(Refusal::InvalidVector); }
    let mut custody = vec![country_vtable, country_id as u64, status as u64, status_vtable, party as u64, party_vtable, data, capacity as u64, count as u64];
    if count == 0 { return Ok((custody, NameToken::Absent)); }
    if !is_map_control_heap_pointer(data as usize) { return Err(Refusal::InvalidVector); }
    let leader = pointer(mem, data as usize)?;
    let leader_vtable = mem.u64(leader).ok_or(Refusal::Unreadable)?;
    if leader_vtable as usize != module + COUNTRY_LEADER_VTABLE_RVA { return Err(Refusal::InvalidIdentity); }
    let ideology = pointer(mem, leader + 0x140)?;
    let text = token(mem, ideology + 0x10)?;
    custody.extend([leader as u64, leader_vtable, ideology as u64, text.size as u64, text.cap as u64, text.chars_addr as u64]);
    Ok((custody, NameToken::Present(utf8(&text)?)))
}

fn stable_leader<M: DecodeMemory + ?Sized>(mem: &M, country: usize, vtable: usize, between: impl FnOnce()) -> Result<NameToken, Refusal> {
    let first = leader_once(mem, country, vtable)?;
    between();
    let second = leader_once(mem, country, vtable)?;
    if first != second { return Err(Refusal::Changed); }
    Ok(first.1)
}

pub(super) fn current_leader(country: usize, vtable: usize) -> NameToken {
    let read = || -> Result<NameToken, Refusal> {
        let module = vtable.checked_sub(COUNTRY_VTABLE_RVA).ok_or(Refusal::InvalidIdentity)?;
        let hour = read_research_game_hour_raw(module).ok_or(Refusal::Unreadable)?;
        let result = stable_leader(&LIVE_MEMORY, country, vtable, || {})?;
        if read_research_game_hour_raw(module) != Some(hour) { return Err(Refusal::ClockChanged); }
        Ok(result)
    };
    read().unwrap_or_else(NameToken::Unavailable)
}

#[derive(PartialEq, Eq)]
struct IdentityRead {
    custody: Vec<u64>,
    cosmetic: NameToken,
    base_tag: String,
    effective: EffectiveNameTag,
}

fn tag_once<M: DecodeMemory + ?Sized>(mem: &M, game: usize, database: usize, id: i32, index_count: u32) -> Result<(Vec<u64>, String), Refusal> {
    let header = if id == 0 { database } else { game + 0x358 };
    let tags = pointer(mem, header)?;
    let capacity = mem.u32(header + 8).ok_or(Refusal::Unreadable)?;
    let count = mem.u32(header + 12).ok_or(Refusal::Unreadable)?;
    if count > capacity || count > i32::MAX as u32 || id < 0 || id as u32 >= count
        || (id != 0 && count != index_count) { return Err(Refusal::OutOfBounds); }
    let tag = token(mem, tags + id as usize * COUNTRY_TAG_TABLE_ENTRY_STRIDE)?;
    Ok((vec![tags as u64, capacity as u64, count as u64, tag.size as u64, tag.cap as u64, tag.chars_addr as u64], utf8(&tag)?))
}

fn identity_once<M: DecodeMemory + ?Sized>(mem: &M, state: &CollectorState, country: usize, expected_id: u32) -> Result<IdentityRead, Refusal> {
    let vtable = state.country_identity.country_vtable;
    let module = vtable.checked_sub(COUNTRY_VTABLE_RVA).ok_or(Refusal::InvalidIdentity)?;
    let game = pointer(mem, module + compact_contract::build_profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA)?;
    let database = pointer(mem, module + COUNTRY_DATABASE_OFFSET)?;
    let object_vtable = mem.u64(country).ok_or(Refusal::Unreadable)?;
    let base = mem.u32(country + OBJ_TAG_DB_ID).ok_or(Refusal::Unreadable)? as i32;
    if game != state.date_ptr as usize || database != state.country_database_ptr as usize
        || object_vtable as usize != vtable || base as u32 != expected_id || base < 0 {
        return Err(Refusal::InvalidIdentity);
    }
    let override_id = mem.u32(country + 0x130c).ok_or(Refusal::Unreadable)? as i32;
    let boundary = mem.u32(database + 0x88).ok_or(Refusal::Unreadable)? as i32;
    let indices = pointer(mem, game + 0x340)?;
    let index_capacity = mem.u32(game + 0x348).ok_or(Refusal::Unreadable)?;
    let index_count = mem.u32(game + 0x34c).ok_or(Refusal::Unreadable)?;
    if index_count > index_capacity || index_count > i32::MAX as u32 || base as u32 >= index_count {
        return Err(Refusal::InvalidVector);
    }
    let mapped = if base == 0 { 0 } else { mem.u32(indices + base as usize * 4).ok_or(Refusal::Unreadable)? as i32 };
    let (source, selected_id) = wire::select_name_tag(base, override_id, mapped, boundary);
    let (mut tag_custody, tag) = tag_once(mem, game, database, selected_id, index_count)?;
    let base_tag = if selected_id == base { tag.clone() } else {
        let (base_custody, base_tag) = tag_once(mem, game, database, base, index_count)?;
        tag_custody.extend(base_custody);
        base_tag
    };
    let cosmetic = token(mem, country + OBJ_COSMETIC_TAG)?;
    let cosmetic_value = if cosmetic.bytes.is_empty() { NameToken::Absent } else { NameToken::Present(utf8(&cosmetic)?) };
    let mut custody = vec![game as u64, database as u64, object_vtable, base as u32 as u64, override_id as u32 as u64,
            boundary as u32 as u64, indices as u64, index_capacity as u64, index_count as u64, mapped as u32 as u64,
            cosmetic.size as u64, cosmetic.cap as u64, cosmetic.chars_addr as u64];
    custody.extend(tag_custody);
    Ok(IdentityRead {
        custody,
        cosmetic: cosmetic_value,
        base_tag,
        effective: EffectiveNameTag { source, selected_id, tag },
    })
}

pub(super) fn identity(state: &CollectorState, country: usize, id: u32, expected_tag: &str) -> (NameToken, Result<EffectiveNameTag, Refusal>) {
    let read = || -> Result<IdentityRead, Refusal> {
        let module = state.country_identity.country_vtable.checked_sub(COUNTRY_VTABLE_RVA).ok_or(Refusal::InvalidIdentity)?;
        let hour = read_research_game_hour_raw(module).ok_or(Refusal::Unreadable)?;
        let first = identity_once(&LIVE_MEMORY, state, country, id)?;
        let second = identity_once(&LIVE_MEMORY, state, country, id)?;
        if first != second || first.base_tag != expected_tag { return Err(Refusal::Changed); }
        if read_research_game_hour_raw(module) != Some(hour) { return Err(Refusal::ClockChanged); }
        Ok(first)
    };
    match read() {
        Ok(value) => (value.cosmetic, Ok(value.effective)),
        Err(reason) => (NameToken::Unavailable(reason), Err(reason)),
    }
}

fn culture_once<M: DecodeMemory + ?Sized>(mem: &M, country: usize, vtable: usize, id: u32) -> Result<MsvcStringSnapshot, Refusal> {
    if mem.u64(country).ok_or(Refusal::Unreadable)? != vtable as u64
        || mem.u32(country + OBJ_TAG_DB_ID).ok_or(Refusal::Unreadable)? != id
    {
        return Err(Refusal::InvalidIdentity);
    }
    token(mem, country + 0x1028)
}

fn stable_culture<M: DecodeMemory + ?Sized>(mem: &M, country: usize, vtable: usize, id: u32,
    mut hour: impl FnMut() -> Option<i32>, between: impl FnOnce()) -> NameToken {
    let read = || -> Result<String, Refusal> {
        let before = hour().ok_or(Refusal::Unreadable)?;
        let first = culture_once(mem, country, vtable, id)?;
        between();
        let second = culture_once(mem, country, vtable, id)?;
        if first != second { return Err(Refusal::Changed); }
        if hour() != Some(before) { return Err(Refusal::ClockChanged); }
        utf8(&first)
    };
    match read() {
        Ok(value) => NameToken::Present(value),
        Err(reason) => NameToken::Unavailable(reason),
    }
}

pub(super) fn graphical_culture(state: &CollectorState, country: usize, id: u32) -> NameToken {
    let vtable = state.country_identity.country_vtable;
    let Some(module) = vtable.checked_sub(COUNTRY_VTABLE_RVA) else {
        return NameToken::Unavailable(Refusal::InvalidIdentity);
    };
    if pointer(&LIVE_MEMORY, module + compact_contract::build_profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA)
        != Ok(state.date_ptr as usize) {
        return NameToken::Unavailable(Refusal::InvalidIdentity);
    }
    stable_culture(&LIVE_MEMORY, country, vtable, id, || read_research_game_hour_raw(module), || {})
}

pub(super) fn token_json(token: &NameToken) -> serde_json::Value {
    match token {
        NameToken::Unavailable(reason) => serde_json::json!({"status":"unavailable", "reason":reason.name()}),
        NameToken::Absent => serde_json::json!({"status":"absent"}),
        NameToken::Present(value) => serde_json::json!({"status":"measured", "value":value}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    #[derive(Default)]
    struct Memory(RefCell<BTreeMap<usize,u8>>);
    impl Memory {
        fn put(&self, at: usize, bytes: &[u8]) { self.0.borrow_mut().extend(bytes.iter().enumerate().map(|(i,b)| (at+i,*b))); }
        fn u64(&self, at: usize, value: u64) { self.put(at, &value.to_le_bytes()); }
        fn u32(&self, at: usize, value: u32) { self.put(at, &value.to_le_bytes()); }
    }
    impl DecodeMemory for Memory {
        fn scalar<const N: usize>(&self, at: usize) -> Option<[u8;N]> { self.bytes(at,N)?.try_into().ok() }
        fn bytes_with_reason(&self, at: usize, len: usize) -> Result<Vec<u8>,GuardedReadRefusal> {
            (at..at+len).map(|key| self.0.borrow().get(&key).copied().ok_or(GuardedReadRefusal::OutsideConfirmedRanges)).collect()
        }
        fn fill(&self, at: usize, buf: &mut [u8]) -> bool {
            let Some(bytes) = self.bytes(at,buf.len()) else { return false; }; buf.copy_from_slice(&bytes); true
        }
    }

    #[test]
    fn culture_is_independent_of_cosmetic_and_names_and_keeps_exact_empty_and_loss() {
        let mem = Memory::default();
        let country = HEAP_LO + 0x10000;
        let vtable = 0x140000000 + COUNTRY_VTABLE_RVA;
        mem.u64(country, vtable as u64);
        mem.u32(country + OBJ_TAG_DB_ID, 47);
        let at = country + 0x1028;
        mem.put(at, b"gfx\0"); mem.u64(at + 0x10, 3); mem.u64(at + 0x18, 15);
        let sample = || stable_culture(&mem, country, vtable, 47, || Some(17), || {});
        assert_eq!(sample(), NameToken::Present("gfx".into()));
        mem.put(country + OBJ_COSMETIC_TAG, b"different");
        assert_eq!(sample(), NameToken::Present("gfx".into()));
        mem.put(at, b"\0"); mem.u64(at + 0x10, 0);
        assert_eq!(sample(), NameToken::Present(String::new()));
        mem.put(at, b"gfx\0"); mem.u64(at + 0x10, 3);
        assert_eq!(stable_culture(&mem, country, vtable, 47, || Some(17), || mem.put(at, b"new")), NameToken::Unavailable(Refusal::Changed));
        let mut tick = 17;
        assert_eq!(stable_culture(&mem, country, vtable, 47, || { tick += 1; Some(tick) }, || {}), NameToken::Unavailable(Refusal::ClockChanged));
        mem.u32(country + OBJ_TAG_DB_ID, 48);
        assert_eq!(sample(), NameToken::Unavailable(Refusal::InvalidIdentity));
        mem.u32(country + OBJ_TAG_DB_ID, 47);
        mem.0.borrow_mut().remove(&at);
        assert_eq!(sample(), NameToken::Unavailable(Refusal::Unreadable));
        mem.put(at, &[255]);
        assert_eq!(sample(), NameToken::Unavailable(Refusal::InvalidUtf8));
    }

    #[test]
    fn selected_tag_requires_exact_table_bounds_and_preserves_empty_bytes() {
        let mem = Memory::default();
        let game = HEAP_LO + 0x10000;
        let database = game + 0x10000;
        let tags = database + 0x10000;
        mem.u64(game+0x358, tags as u64); mem.u32(game+0x360, 3); mem.u32(game+0x364, 3);
        let at = tags + COUNTRY_TAG_TABLE_ENTRY_STRIDE;
        mem.put(at, b"GER\0"); mem.u64(at+0x10, 3); mem.u64(at+0x18, 15);
        assert_eq!(tag_once(&mem,game,database,1,3).unwrap().1, "GER");
        assert_eq!(tag_once(&mem,game,database,1,2), Err(Refusal::OutOfBounds));
        assert_eq!(tag_once(&mem,game,database,3,3), Err(Refusal::OutOfBounds));
        mem.put(at, b"\0"); mem.u64(at+0x10, 0);
        assert_eq!(tag_once(&mem,game,database,1,3).unwrap().1, "");
        mem.u64(database, tags as u64); mem.u32(database+8, 1); mem.u32(database+12, 1);
        mem.put(tags, b"---\0"); mem.u64(tags+0x10, 3); mem.u64(tags+0x18, 15);
        assert_eq!(tag_once(&mem,game,database,0,3).unwrap().1, "---");
    }

    #[test]
    fn leader_presence_empty_subtype_and_pointer_mutation_are_distinct() {
        let mem=Memory::default();
        let country=HEAP_LO+0x10000; let status=country+0x10000; let party=status+0x10000;
        let vector=party+0x10000; let leader=vector+0x10000; let ideology=leader+0x10000;
        let module=0x140000000usize; let vtable=module+COUNTRY_VTABLE_RVA;
        mem.u64(country,vtable as u64); mem.u32(country+8,47);
        mem.u64(country+0xf90,status as u64); mem.u64(status,(module+POLITICAL_STATUS_VTABLE_RVA) as u64);
        mem.u64(status+POLITICAL_STATUS_RULING_PARTY,party as u64); mem.u64(party,(module+POLITICAL_PARTY_VTABLE_RVA) as u64);
        mem.u64(party+0x70,vector as u64); mem.u32(party+0x78,2); mem.u32(party+0x7c,0);
        assert_eq!(stable_leader(&mem,country,vtable,||{}),Ok(NameToken::Absent));
        mem.u32(party+0x7c,2); mem.u64(vector,leader as u64);
        mem.u64(leader,(module+COUNTRY_LEADER_VTABLE_RVA) as u64); mem.u64(leader+0x140,ideology as u64);
        mem.u64(ideology+0x20,0); mem.u64(ideology+0x28,15); mem.put(ideology+0x10,&[0]);
        assert_eq!(stable_leader(&mem,country,vtable,||{}),Ok(NameToken::Present(String::new())));
        mem.u64(ideology+0x20,6); mem.put(ideology+0x10,b"nazism\0");
        assert_eq!(stable_leader(&mem,country,vtable,||{}),Ok(NameToken::Present("nazism".into())));
        assert_eq!(stable_leader(&mem,country,vtable,||mem.u32(party+0x7c,0)),Err(Refusal::Changed));
        mem.0.borrow_mut().remove(&(party+0x7c));
        assert_eq!(stable_leader(&mem,country,vtable,||{}),Err(Refusal::Unreadable));
    }
}
