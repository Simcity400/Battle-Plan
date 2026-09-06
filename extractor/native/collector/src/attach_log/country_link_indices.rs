use super::*;
use compact_contract::country_name_inputs::{CountryLinkIndices, NameInputRefusal as Refusal};

#[derive(Debug, PartialEq, Eq)]
struct Header { data: usize, capacity: u32, count: u32 }

pub(super) struct Pending {
    module: usize,
    game: usize,
    selected_hour: i32,
    header: Header,
    body: Vec<u8>,
}

fn header<M: DecodeMemory + ?Sized>(mem: &M, game: usize) -> Result<Header, Refusal> {
    let at = game.checked_add(0x340).ok_or(Refusal::OutOfBounds)?;
    let data = mem.u64(at).ok_or(Refusal::Unreadable)? as usize;
    let capacity = mem.u32(at.checked_add(8).ok_or(Refusal::OutOfBounds)?).ok_or(Refusal::Unreadable)?;
    let count = mem.u32(at.checked_add(12).ok_or(Refusal::OutOfBounds)?).ok_or(Refusal::Unreadable)?;
    Ok(Header { data, capacity, count })
}

fn body<M: DecodeMemory + ?Sized>(mem: &M, header: &Header, max_bytes: usize) -> Result<Vec<u8>, Refusal> {
    if header.count > i32::MAX as u32 || header.count > header.capacity { return Err(Refusal::InvalidVector); }
    let len = (header.count as usize).checked_mul(4).ok_or(Refusal::OutOfBounds)?;
    if len > max_bytes { return Err(Refusal::OutOfBounds); }
    if len != 0 && !is_map_control_heap_pointer(header.data) { return Err(Refusal::InvalidVector); }
    header.data.checked_add(len).ok_or(Refusal::OutOfBounds)?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(len).map_err(|_| Refusal::OutOfBounds)?;
    bytes.resize(len, 0);
    if len != 0 && !mem.fill(header.data, &mut bytes) { return Err(Refusal::Unreadable); }
    Ok(bytes)
}

pub(super) fn begin<M: DecodeMemory + ?Sized>(mem: &M, module: usize, game: usize,
    selected_hour: i32, hour: impl FnOnce() -> Option<i32>, max_bytes: usize) -> Result<Pending, Refusal> {
    let singleton = module.checked_add(compact_contract::build_profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA)
        .ok_or(Refusal::OutOfBounds)?;
    if !is_map_control_heap_pointer(game) || mem.u64(singleton).ok_or(Refusal::Unreadable)? as usize != game {
        return Err(Refusal::InvalidIdentity);
    }
    if hour().ok_or(Refusal::Unreadable)? != selected_hour { return Err(Refusal::ClockChanged); }
    let header = header(mem, game)?;
    let body = body(mem, &header, max_bytes)?;
    Ok(Pending { module, game, selected_hour, header, body })
}

impl Pending {
    pub(super) fn finish<M: DecodeMemory + ?Sized>(self, mem: &M, hour: impl FnOnce() -> Option<i32>)
        -> CountryLinkIndices {
        let read = || -> Result<Vec<u32>, Refusal> {
            let repeated = body(mem, &self.header, self.body.len())?;
            if repeated != self.body || header(mem, self.game)? != self.header { return Err(Refusal::Changed); }
            let singleton = self.module.checked_add(compact_contract::build_profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA)
                .ok_or(Refusal::OutOfBounds)?;
            if mem.u64(singleton).ok_or(Refusal::Unreadable)? as usize != self.game { return Err(Refusal::InvalidIdentity); }
            if hour().ok_or(Refusal::Unreadable)? != self.selected_hour { return Err(Refusal::ClockChanged); }
            let mut indices = Vec::new();
            indices.try_reserve_exact(self.header.count as usize).map_err(|_| Refusal::OutOfBounds)?;
            indices.extend(self.body.chunks_exact(4).map(|cell| u32::from_le_bytes(cell.try_into().unwrap())));
            Ok(indices)
        };
        match read() { Ok(indices) => CountryLinkIndices::Measured(indices), Err(reason) => CountryLinkIndices::Unavailable(reason) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    #[derive(Default)]
    struct Memory { bytes: RefCell<BTreeMap<usize, u8>>, fills: Cell<usize> }
    impl Memory {
        fn put(&self, at: usize, bytes: &[u8]) { self.bytes.borrow_mut().extend(bytes.iter().enumerate().map(|(i,b)| (at+i,*b))); }
        fn put32(&self, at: usize, value: u32) { self.put(at, &value.to_le_bytes()); }
        fn put64(&self, at: usize, value: usize) { self.put(at, &(value as u64).to_le_bytes()); }
    }
    impl DecodeMemory for Memory {
        fn scalar<const N: usize>(&self, at: usize) -> Option<[u8; N]> { self.bytes(at, N)?.try_into().ok() }
        fn bytes_with_reason(&self, at: usize, len: usize) -> Result<Vec<u8>, GuardedReadRefusal> {
            (at..at+len).map(|key| self.bytes.borrow().get(&key).copied().ok_or(GuardedReadRefusal::OutsideConfirmedRanges)).collect()
        }
        fn fill(&self, at: usize, buf: &mut [u8]) -> bool {
            self.fills.set(self.fills.get()+1);
            let Some(bytes) = self.bytes(at, buf.len()) else { return false; }; buf.copy_from_slice(&bytes); true
        }
    }
    const MODULE: usize = 0x140000000;
    fn fixture(values: &[u32]) -> (Memory, usize, usize) {
        let mem = Memory::default(); let game = HEAP_LO+0x10000; let data = HEAP_LO+0x20000;
        mem.put64(MODULE+compact_contract::build_profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA, game);
        mem.put64(game+0x340, data); mem.put32(game+0x348, values.len() as u32); mem.put32(game+0x34c, values.len() as u32);
        for (i,value) in values.iter().enumerate() { mem.put32(data+i*4,*value); }
        (mem,game,data)
    }
    #[test]
    fn complete_body_preserves_slot_zero_aliases_and_unrostered_raw_bits() {
        let values = vec![0, u32::MAX, 7, 7, 0x80000000];
        let (mem,game,_) = fixture(&values);
        let first = begin(&mem,MODULE,game,17,||Some(17),20).unwrap();
        assert_eq!(first.finish(&mem,||Some(17)),CountryLinkIndices::Measured(values));
        assert_eq!(mem.fills.get(),2);
    }
    #[test]
    fn empty_is_measured_without_dereferencing_root_and_unreadable_is_not_empty() {
        let (mem,game,_) = fixture(&[]); mem.put64(game+0x340,0);
        assert_eq!(begin(&mem,MODULE,game,17,||Some(17),0).unwrap().finish(&mem,||Some(17)),CountryLinkIndices::Measured(vec![]));
        assert_eq!(mem.fills.get(),0);
        mem.bytes.borrow_mut().remove(&(game+0x34c));
        assert!(matches!(begin(&mem,MODULE,game,17,||Some(17),0),Err(Refusal::Unreadable)));
    }
    #[test]
    fn full_reread_refuses_same_pointer_rewrite_and_each_header_change() {
        for change in 0..4 {
            let (mem,game,data) = fixture(&[1,2,3]);
            let first = begin(&mem,MODULE,game,17,||Some(17),12).unwrap();
            match change { 0=>mem.put32(data+4,9),1=>mem.put64(game+0x340,data+16),2=>mem.put32(game+0x348,4),_=>mem.put32(game+0x34c,2) }
            assert_eq!(first.finish(&mem,||Some(17)),CountryLinkIndices::Unavailable(Refusal::Changed));
        }
    }
    #[test]
    fn selected_hour_and_singleton_are_checked_at_both_ends() {
        let (mem,game,_) = fixture(&[1]);
        assert!(matches!(begin(&mem,MODULE,game,17,||Some(18),4),Err(Refusal::ClockChanged)));
        assert_eq!(mem.fills.get(),0);
        assert_eq!(begin(&mem,MODULE,game,17,||Some(17),4).unwrap().finish(&mem,||Some(18)),CountryLinkIndices::Unavailable(Refusal::ClockChanged));
        let first = begin(&mem,MODULE,game,17,||Some(17),4).unwrap();
        mem.put64(MODULE+compact_contract::build_profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA,game+0x1000);
        assert_eq!(first.finish(&mem,||Some(17)),CountryLinkIndices::Unavailable(Refusal::InvalidIdentity));
        assert!(matches!(begin(&mem,MODULE,game,17,||Some(17),4),Err(Refusal::InvalidIdentity)));
    }
    #[test]
    fn signed_count_capacity_and_allocation_limit_refuse_before_body_read() {
        let (mem,game,_) = fixture(&[1,2]);
        assert!(matches!(begin(&mem,MODULE,game,17,||Some(17),7),Err(Refusal::OutOfBounds)));
        assert_eq!(mem.fills.get(),0);
        for (count,capacity) in [(u32::MAX,u32::MAX),(3,2)] {
            mem.put32(game+0x34c,count);mem.put32(game+0x348,capacity);
            assert!(matches!(begin(&mem,MODULE,game,17,||Some(17),usize::MAX),Err(Refusal::InvalidVector)));
        }
        assert_eq!(mem.fills.get(),0);
    }
    #[test]
    fn second_body_unreadable_refuses_without_partial_vector() {
        let (mem,game,data) = fixture(&[1,2]);
        let first = begin(&mem,MODULE,game,17,||Some(17),8).unwrap();
        mem.bytes.borrow_mut().remove(&(data+7));
        assert_eq!(first.finish(&mem,||Some(17)),CountryLinkIndices::Unavailable(Refusal::Unreadable));
    }
}
