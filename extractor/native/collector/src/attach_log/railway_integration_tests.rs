use super::*;
use compact_contract::country_roster::{Cause, Kind, Observation};
use std::cell::RefCell;

const BASE: usize = 0x140000000;
const GAME: usize = HEAP_LO + 0x200000;
const COUNTRY: usize = HEAP_LO + 0x210000;
const TABLE: usize = HEAP_LO + 0x220000;
const VECTOR: usize = HEAP_LO + 0x230000;
const REGISTRY: usize = HEAP_LO + 0x240000;
const SLOTS: usize = HEAP_LO + 0x250000;
const BODY: usize = HEAP_LO + 0x260000;
const HOUR: i32 = 60_759_359;

#[derive(Default)]
struct Memory {
    bytes: RefCell<BTreeMap<usize, u8>>,
    calls: RefCell<Vec<(usize, usize)>>,
    visits: RefCell<BTreeMap<(usize, usize), usize>>,
    mutations: RefCell<BTreeMap<(usize, usize, usize), Vec<u8>>>,
    failures: RefCell<BTreeSet<(usize, usize, usize)>>,
}
impl Memory {
    fn put(&self, address: usize, bytes: &[u8]) {
        let mut memory = self.bytes.borrow_mut();
        for (offset, byte) in bytes.iter().enumerate() { memory.insert(address + offset, *byte); }
    }
    fn word(&self, address: usize, value: u32) { self.put(address, &value.to_le_bytes()); }
    fn pointer(&self, address: usize, value: usize) { self.put(address, &(value as u64).to_le_bytes()); }
    fn new(ids: &[u32]) -> Self {
        for pointer in [GAME, COUNTRY, TABLE, VECTOR, REGISTRY, SLOTS, BODY] {
            assert!(is_heap_pointer_aligned(pointer, 8));
        }
        let memory = Self::default();
        memory.pointer(GAME + CGAMESTATE_COUNTRY_TABLE, TABLE);
        memory.word(GAME + CGAMESTATE_COUNTRY_COUNT, 1);
        memory.word(GAME + GS_CURRENT_DATE_OFFSET, HOUR as u32);
        memory.pointer(TABLE, COUNTRY);
        memory.pointer(COUNTRY, BASE + COUNTRY_VTABLE_RVA);
        memory.word(COUNTRY + OBJ_TAG_DB_ID, 7);
        memory.put(COUNTRY + COUNTRY_DIVISION_WRAPPER_DATA, &[0; 16]);
        memory.pointer(COUNTRY + 0x2a8, if ids.is_empty() { 0 } else { VECTOR });
        memory.word(COUNTRY + 0x2b0, ids.len() as u32);
        memory.word(COUNTRY + 0x2b4, ids.len() as u32);
        memory.pointer(BASE + 0x03438e60 + 51 * 8, REGISTRY);
        memory.pointer(REGISTRY + 8, SLOTS);
        memory.word(REGISTRY + 0x14, 0);
        memory.put(REGISTRY + 0x18, &[ids.len() as u8]);
        for (ordinal, id) in ids.iter().enumerate() {
            let body = BODY + ordinal * 0x1000;
            memory.pointer(VECTOR + ordinal * 8, body);
            memory.pointer(body, BASE + 0x0295b600);
            memory.word(body + 8, 13);
            memory.pointer(body + 0x10, BASE + 0x0295b7d8);
            memory.word(body + 0x18, 51);
            memory.word(body + 0x1c, *id);
            memory.word(body + 0x1d8, 7);
            let slot = SLOTS + ordinal * 24;
            memory.put(slot + 4, &[(ordinal + 1) as u8]);
            memory.word(slot + 8, 51);
            memory.word(slot + 0xc, *id);
            memory.pointer(slot + 0x10, body + 0x10);
        }
        memory
    }
    fn state(&self) -> CollectorState {
        let mut state = super::tests::empty_collector_state();
        state.base = BASE; state.daily_match = true; state.long_string_match = true;
        state.date_ptr = GAME as u64;
        state
    }
    fn mutate(&self, address: usize, width: usize, visit: usize, value: Vec<u8>) {
        assert_eq!(width, value.len());
        self.mutations.borrow_mut().insert((address, width, visit), value);
    }
}
impl d81_snapshot_memory::Domain81Memory for Memory {
    fn read_into(&self, address: u64, out: &mut [u8]) -> bool {
        let address = address as usize;
        self.calls.borrow_mut().push((address, out.len()));
        let visit = {
            let mut visits = self.visits.borrow_mut();
            let value = visits.entry((address, out.len())).or_default(); *value += 1; *value
        };
        if self.failures.borrow().contains(&(address, out.len(), visit)) { return false; }
        let changed = self.mutations.borrow_mut().remove(&(address, out.len(), visit));
        if let Some(bytes) = changed { self.put(address, &bytes); }
        let memory = self.bytes.borrow();
        for (offset, byte) in out.iter_mut().enumerate() {
            let Some(value) = memory.get(&(address + offset)) else { return false; };
            *byte = *value;
        }
        true
    }
}
fn discover(memory: &Memory, budget: &country_roster_readset::Budget, enabled: bool) -> CompactDivisionRootCache {
    let state = memory.state();
    let lanes = FrameLaneTracker::new(Instant::now(), Instant::now(), BASE, GAME + GS_CURRENT_DATE_OFFSET);
    with_d81_memory_override(memory, || {
        let _guard = ReaderClockBracketGuard::new(&lanes, COMPACT_DOMAIN_DIVISIONS);
        discover_compact_division_roots_from_countries_budgeted(&state, budget, enabled).unwrap()
    })
}
fn payload(memory: &Memory, delta: bool) -> Vec<u8> {
    let roots = discover(memory, &country_roster_readset::Budget::default(), true);
    assert!(roots.railway_roster.as_ref().unwrap().countries[0].complete());
    compact_divisions_data_payload(&[], 0, if delta { COMPACT_STATUS_DELTA_DATA } else { 0 },
        0, 0, roots.country_roster.as_ref(), roots.railway_roster.as_ref(), &country_roster_readset::Budget::default()).unwrap()
}

#[test]
fn scheduled_railway_discovery_emits_original_order_and_measured_empty_in_real_d2r8() {
    for ids in [vec![9, 2], vec![2, 9], vec![]] {
        let memory = Memory::new(&ids);
        let bytes = payload(&memory, false);
        let parsed = compact_contract::division_routes::DivisionRoutes::parse(&bytes).unwrap();
        let observed = Observation::parse(parsed.railway_roster_bytes(), Kind::Railway).unwrap();
        assert_eq!(observed.countries[0].owner_tag_db_id, Some(7));
        assert!(observed.countries[0].complete());
        assert_eq!(observed.countries[0].slots.iter().map(|slot| slot.reference.unwrap().id).collect::<Vec<_>>(), ids);
        assert_eq!(memory.calls.borrow().iter().filter(|(a,n)| (*a,*n)==(COUNTRY,8)).count(), 2);
        if ids.is_empty() {
            assert!(!memory.calls.borrow().iter().any(|(a,_)| *a >= BODY || (*a >= REGISTRY && *a < REGISTRY + 0x20)));
        }
    }
    let memory = Memory::new(&[9]);
    let roots = discover(&memory, &country_roster_readset::Budget::default(), false);
    assert!(roots.railway_roster.is_none());
    assert!(!memory.calls.borrow().iter().any(|(a,_)| (*a >= COUNTRY+0x2a8 && *a < COUNTRY+0x2b8) || *a==VECTOR));
}

#[test]
fn outer_country_class_identity_cell_and_table_failures_withdraw_locally_good_railways() {
    for (address, width, visit, replacement) in [
        (COUNTRY,8,2,(BASE+COUNTRY_VTABLE_RVA+8).to_le_bytes().to_vec()),
        (COUNTRY+OBJ_TAG_DB_ID,4,2,8u32.to_le_bytes().to_vec()),
        (TABLE,8,2,(COUNTRY+0x1000).to_le_bytes().to_vec()),
        (TABLE,8,3,(COUNTRY+0x1000).to_le_bytes().to_vec()),
        (GAME+CGAMESTATE_COUNTRY_TABLE,8,2,(TABLE+0x1000).to_le_bytes().to_vec()),
        (GAME+CGAMESTATE_COUNTRY_COUNT,4,2,2u32.to_le_bytes().to_vec()),
    ] {
        let memory = Memory::new(&[9,2]); memory.mutate(address,width,visit,replacement);
        let roots = discover(&memory, &country_roster_readset::Budget::default(), true);
        let railway = roots.railway_roster.unwrap();
        assert!(railway.failure.is_some() || railway.countries[0].failure.is_some(), "{address:x}/{visit}");
        assert!(!railway.countries[0].complete());
        assert_eq!(memory.calls.borrow().iter().filter(|(a,n)|(*a,*n)==(COUNTRY,8)).count(),2);
        assert!(!memory.calls.borrow().iter().any(|(a,_)| *a==COUNTRY+0x1000 || *a==TABLE+0x1000));
    }
}

#[test]
fn class_close_survives_local_read_error_owner_close_error_and_exhausted_budget() {
    for mode in 0..3 {
        let memory = Memory::new(&[9]);
        let budget = if mode==2 { country_roster_readset::Budget::test_limits(16, 16*1024*1024) }
            else { country_roster_readset::Budget::default() };
        if mode==0 { memory.failures.borrow_mut().insert((VECTOR,8,1)); }
        if mode==1 { memory.failures.borrow_mut().insert((COUNTRY+OBJ_TAG_DB_ID,4,2)); }
        let roots = discover(&memory,&budget,true);
        assert!(!roots.railway_roster.unwrap().countries[0].complete());
        assert_eq!(memory.calls.borrow().iter().filter(|(a,n)|(*a,*n)==(COUNTRY,8)).count(),2,"mode {mode}");
        if mode==2 { assert!(budget.verification_bytes()>16); }
    }
}

#[test]
fn caller_country_collection_retention_refuses_before_class_or_railway_reads() {
    let memory = Memory::new(&[9]);
    let bytes = 8 + 2*std::mem::size_of::<compact_contract::country_roster::Country>();
    let roots = discover(&memory,&country_roster_readset::Budget::test_limits(10*1024*1024,bytes-1),true);
    assert_eq!(roots.railway_roster.unwrap().failure,Some(Cause::BoundExceeded));
    assert!(!memory.calls.borrow().iter().any(|(a,_)| *a==COUNTRY || *a==VECTOR));
}

#[test]
fn actual_first_reader_cache_tick_refusal_replaces_railway_d20_publication() {
    let memory=Memory::new(&[9]); let state=memory.state();
    let lanes=FrameLaneTracker::new(Instant::now(),Instant::now(),BASE,GAME+GS_CURRENT_DATE_OFFSET);
    let mut cache=CompactFrameDivisionRootsCache::new(&state,true);cache.capture_railways=true;
    with_d81_memory_override(&memory, || {
        assert!(lanes.read_clock().is_none(), "an absent singleton cannot authenticate the clock");
        memory.pointer(BASE + compact_contract::build_profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA, GAME);
        { let _guard=ReaderClockBracketGuard::new(&lanes,26); assert!(cache.get().railway_roster.as_ref().unwrap().countries[0].complete());
            memory.word(GAME+GS_CURRENT_DATE_OFFSET,(HOUR+1) as u32); }
        memory.word(GAME+GS_CURRENT_DATE_OFFSET,HOUR as u32);
        { let _guard=ReaderClockBracketGuard::new(&lanes,20); assert!(cache.get().railway_roster.is_some()); }
    });
    let brackets=lanes.take_clock_brackets();
    let hours=brackets.iter().map(|bracket| (bracket.tag,
        bracket.opening.map(|clock| clock.hour), bracket.closing.map(|clock| clock.hour)))
        .collect::<Vec<_>>();
    assert_eq!(hours, vec![(26,Some(HOUR),Some(HOUR+1)),(20,Some(HOUR),Some(HOUR))]);
    assert_eq!(cache.acquisition.dependencies(), vec![(20,Some(26)),(26,Some(26))]);
    let roots=cache.observed();
    let bytes=compact_divisions_data_payload(&[],0,0,0,0,roots.country_roster.as_ref(),roots.railway_roster.as_ref(), &country_roster_readset::Budget::default()).unwrap();
    let mut records=Vec::new();append_compact_record(&mut records,1,HOUR,20,COMPACT_RECORD_TYPE_DOMAIN_DATA,0,&bytes);
    let (_,refused)=replace_rejected_domain_records_with_root_dependencies(&mut records,1,HOUR,
        &brackets,&mut BTreeMap::new(),HourLabelMismatchPolicy::Refuse,&cache.acquisition.dependencies()).unwrap();
    assert!(refused.contains(&20));
    let header=compact_contract::parse_compact_record_header(&records).unwrap();
    assert_eq!(header.record_type,compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP);
    let gap=compact_contract::domain_gap::DomainGapRecordV1::decode(&records[compact_contract::COMPACT_RECORD_HEADER_LEN..header.record_len()]).unwrap();
    assert_eq!(gap.reason,compact_contract::domain_gap::DomainGapReason::Torn);
}

#[test]
fn actual_railway_order_only_delta_and_unknown_recover_through_framed_fold() {
    let first=payload(&Memory::new(&[9,2]),false);
    let reordered=payload(&Memory::new(&[2,9]),true);
    assert_eq!(u32::from_le_bytes(reordered[8..12].try_into().unwrap()),0);
    let mut bad=reordered.clone();bad.pop();
    let recovered=payload(&Memory::new(&[9,2]),false);
    let mut stream=Vec::new();let mut sequence=1;
    for (offset,payload) in [first,reordered,bad,recovered].iter().enumerate() {
        let hour=HOUR+offset as i32;
        append_compact_record(&mut stream,sequence,hour,20,COMPACT_RECORD_TYPE_DOMAIN_DATA,0,payload);sequence+=1;
        append_compact_record(&mut stream,sequence,hour,compact_contract::COMPACT_DOMAIN_FRAME,COMPACT_RECORD_TYPE_FRAME_END,0,
            &[0;compact_contract::record_semantics::COMPACT_FRAME_END_PAYLOAD_LEN]);sequence+=1;
    }
    let mut fold=recorder_host::world_fold::WorldFold::new("railway_order_fixture");fold.ingest_stream(&stream);
    let snapshots=fold.finish();assert_eq!(snapshots.len(),4);
    let values=snapshots.iter().map(|(_,_,bytes)|serde_json::from_slice::<serde_json::Value>(bytes).unwrap()).collect::<Vec<_>>();
    let ids=|value:&serde_json::Value|value["country_semantic_rosters"]["railways"]["countries"][0]["slots"].as_array().unwrap()
        .iter().map(|slot|slot["reference"]["id"].as_u64().unwrap()).collect::<Vec<_>>();
    assert_eq!(ids(&values[0]),[9,2]);assert_eq!(ids(&values[1]),[2,9]);
    assert!(values[2]["country_semantic_rosters"]["railways"].is_null());assert_eq!(ids(&values[3]),[9,2]);
    assert_eq!(values[3]["country_semantic_rosters"]["schema"],2);
}
