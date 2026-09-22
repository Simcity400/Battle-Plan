use super::*;
use std::cell::{Cell, RefCell};

const GAME: usize = 0x1000;
const COUNTRY: usize = 0x3000;
const STATUS: usize = 0x5000;
const TECH: usize = 0x6000;
const TEMPLATE: usize = 0x7000;
const METADATA: usize = 0x10000;

struct Fixture {
    bytes: Vec<u8>,
    reads: RefCell<Vec<(usize, usize)>>,
    calls: Cell<usize>,
    fault: Option<usize>,
}
impl Fixture {
    fn new() -> Self {
        let mut f = Self {
            bytes: vec![0; 0x40000],
            reads: RefCell::new(vec![]),
            calls: Cell::new(0),
            fault: None,
        };
        f.array(GAME + crate::cgame_state::COUNTRIES, 0x2000, 3);
        f.array(GAME + crate::cgame_state::COUNTRY_LINKS, 0x2100, 3);
        for id in [1_u32, 2] {
            f.put(0x2000 + id as usize * 8, (COUNTRY as u64).to_le_bytes());
            f.put(0x2100 + id as usize * 4, 1_u32.to_le_bytes());
        }
        f.put(COUNTRY, (ccountry::VTABLE_RVA as u64).to_le_bytes());
        f.put(COUNTRY + 8, 1_u32.to_le_bytes());
        f.put(
            COUNTRY + ccountry::TECHNOLOGY_STATUS,
            (STATUS as u64).to_le_bytes(),
        );
        f.put(
            STATUS,
            (crate::ctechnology_status::VTABLE_RVA as u64).to_le_bytes(),
        );
        f.put(
            STATUS + crate::ctechnology_status::COUNTRY,
            (COUNTRY as u64).to_le_bytes(),
        );
        f
    }
    fn put<const N: usize>(&mut self, at: usize, bytes: [u8; N]) {
        self.bytes[at..at + N].copy_from_slice(&bytes);
    }
    fn array(&mut self, at: usize, data: usize, count: u32) {
        self.put(at, (data as u64).to_le_bytes());
        self.put(at + 8, count.to_le_bytes());
        self.put(at + 12, count.to_le_bytes());
    }
    fn text(&mut self, at: usize, text: &[u8]) {
        assert!(text.len() <= 15);
        self.bytes[at..at + text.len()].copy_from_slice(text);
        self.put(at + 16, (text.len() as u64).to_le_bytes());
        self.put(at + 24, 15_u64.to_le_bytes());
    }
    fn modifier(&mut self, at: usize, storage: usize, key: u32, raw: i64) {
        self.array(at, storage, 1);
        self.put(storage, key.to_le_bytes());
        self.put(storage + 8, raw.to_le_bytes());
    }
    fn field(&self, at: usize, out: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((at, out.len()));
        if self
            .fault
            .is_some_and(|f| at <= f && f < at.saturating_add(out.len()))
        {
            if !out.is_empty() {
                out[0] = 255;
            }
            return false;
        }
        if at == crate::modifier_definition::TABLE_RVA && out.len() == 8 {
            out.copy_from_slice(&(METADATA as u64).to_le_bytes());
            return true;
        }
        if let Some(bytes) = self.bytes.get(at..at.saturating_add(out.len())) {
            out.copy_from_slice(bytes);
            true
        } else {
            false
        }
    }
    fn input(&self) -> research_inputs::Row {
        research_inputs::Row {
            technology_pointer: TECH,
            template_pointer: TEMPLATE,
            status_pointer: STATUS,
            template_year: 1938,
            template_index: 1,
            template_max_level: 1,
            slot: crate::cresearch_slot::Row {
                country: 1,
                index: 0,
                technology_handle: 7,
                technology_present: 1,
                ..Default::default()
            },
            ..Default::default()
        }
    }
    fn capture(&self, inputs: &[research_inputs::Row], rows: &mut [Row]) -> Outcome {
        let countries = ccountry::Reader::new(self, 0, GAME).unwrap();
        walk_country(
            self,
            0,
            &countries,
            ccountry::Country {
                pointer: COUNTRY,
                id: 1,
                canonical_slot: 1,
            },
            Ok(Context {
                minimum: 10_000,
                license: 20_000,
                sharing_maximum: 50_000,
                ahead_factor: 200_000,
            }),
            inputs,
            rows,
        )
    }
}
impl Memory for Fixture {
    fn copy(&self, at: usize, out: &mut [u8]) -> bool {
        self.calls.set(self.calls.get() + 1);
        self.field(at, out)
    }
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        output: &mut [&mut [u8]; N],
    ) -> usize {
        self.calls.set(self.calls.get() + 1);
        for (i, (&address, out)) in addresses.iter().zip(output).enumerate() {
            if !self.field(address, out) {
                return i;
            }
        }
        N
    }
}

#[test]
fn active_operands_are_exact_and_idle_avoids_country_dependencies() {
    let mut f = Fixture::new();
    f.put(TECH + 0x190, (-25_001_i64).to_le_bytes());
    f.put(TECH + 0x1a0, 12_345_i64.to_le_bytes());
    f.put(TECH + 0x1e8, [2]);
    f.put(TEMPLATE + 0x538, 100_003_i64.to_le_bytes());
    let mut rows = [Row::default(); 64];
    let result = f.capture(&[f.input()], &mut rows);
    assert_eq!(result, Outcome::Rows { len: 12 });
    let value = |kind| {
        rows[..result.len()]
            .iter()
            .find(|r| r.kind == kind)
            .unwrap()
            .raw
    };
    assert_eq!(value(Kind::CategorySpeed), -25_001);
    assert_eq!(value(Kind::DesignTeam), 12_345);
    assert_eq!(value(Kind::Experience), 100_003);
    assert_eq!(value(Kind::License), 0);
    assert_eq!(value(Kind::Scientist), 0);
    assert!(
        !f.reads
            .borrow()
            .iter()
            .any(|(at, _)| *at == COUNTRY + ccountry::PRODUCTION_STATUS || *at == COUNTRY + 0xfa8)
    );
    let mut idle = f.input();
    idle.slot.technology_present = 0;
    f.reads.borrow_mut().clear();
    assert_eq!(f.capture(&[idle], &mut rows), Outcome::Rows { len: 0 });
    assert_eq!(f.reads.borrow().len(), 2); // Shared country census headers only.
}

#[test]
fn unbonused_slots_add_one_guard_each_and_share_country_reads() {
    let f = Fixture::new();
    let mut rows = [Row::default(); 64];
    assert!(matches!(
        f.capture(&[f.input()], &mut rows),
        Outcome::Rows { .. }
    ));
    assert_eq!(f.calls.get(), 6); // Includes two shared country-census headers.
    f.calls.set(0);
    let mut second = f.input();
    second.slot.index = 1;
    assert!(matches!(
        f.capture(&[f.input(), second], &mut rows),
        Outcome::Rows { .. }
    ));
    assert_eq!(f.calls.get(), 7);
}

#[test]
fn cyclic_country_source_trees_fail_without_unbounded_recursion() {
    let mut f = Fixture::new();
    f.array(COUNTRY + ccountry::MODIFIER_ACCUMULATOR + 0x18, 0x8000, 1);
    f.put(
        0x8000,
        (COUNTRY as u64 + ccountry::MODIFIER_ACCUMULATOR as u64 - 0x10).to_le_bytes(),
    );
    assert_eq!(
        f.capture(&[f.input()], &mut [Row::default(); 32]),
        Outcome::Unavailable {
            reason: "research modifier tree limit or cycle"
        }
    );
}

#[test]
fn country_tree_is_shared_and_preserves_negative_sources_and_clamps() {
    let mut f = Fixture::new();
    f.modifier(
        COUNTRY + ccountry::MODIFIER_ACCUMULATOR,
        0x8000,
        0xcd,
        -12_000,
    );
    f.array(COUNTRY + ccountry::MODIFIER_ACCUMULATOR + 0x18, 0x8100, 1);
    f.put(0x8100, 0x8200_u64.to_le_bytes());
    f.modifier(0x8210, 0x8300, 0xcd, -12_000);
    f.text(0x8258, b"slow_idea");
    let mut second = f.input();
    second.slot.index = 1;
    let mut rows = [Row::default(); 64];
    let result = f.capture(&[f.input(), second], &mut rows);
    assert!(matches!(result, Outcome::Rows { .. }), "{result:?}");
    let sources: Vec<_> = rows[..result.len()]
        .iter()
        .filter(|r| r.kind == Kind::CountrySource)
        .collect();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].index, u32::MAX);
    assert_eq!(sources[0].raw, -12_000);
    assert_eq!(sources[0].name.as_bytes(), b"slow_idea");
    f.put(
        METADATA + 0xcd * crate::modifier_definition::STRIDE + crate::modifier_definition::FLAGS,
        crate::modifier_definition::CLAMP_MAXIMUM.to_le_bytes(),
    );
    let result = f.capture(&[f.input()], &mut rows);
    let value = rows[..result.len()]
        .iter()
        .find(|r| r.kind == Kind::CountrySpeed)
        .unwrap();
    assert_eq!((value.raw, value.auxiliary), (-12_000, 0));
    let metadata = rows[..result.len()]
        .iter()
        .find(|r| r.kind == Kind::ModifierMetadata && r.source_id == value.ordinal)
        .unwrap();
    assert_eq!(
        metadata.raw,
        i64::from(crate::modifier_definition::CLAMP_MAXIMUM)
    );
}

#[test]
fn every_short_read_refuses_a_complete_breakdown_and_capacity_reports_demand() {
    let mut f = Fixture::new();
    let mut rows = [Row::default(); 64];
    let input = f.input();
    let result = f.capture(&[input], &mut rows);
    let reads = f.reads.borrow().clone();
    for (at, _) in reads.into_iter().skip(2) {
        f.fault = Some(at);
        assert!(
            matches!(f.capture(&[input], &mut rows), Outcome::Unavailable { .. }),
            "{at:x}"
        );
    }
    f.fault = None;
    assert_eq!(
        f.capture(&[input], &mut rows[..1]),
        Outcome::Truncated {
            len: 1,
            unread_rows: result.len() as u32 - 1
        }
    );
}

#[test]
fn selected_idea_and_script_sources_retain_names_without_reading_catalogues() {
    let mut f = Fixture::new();
    f.array(TECH + 0x110, 0x8000, 1);
    f.put(0x8000, 0x8100_u64.to_le_bytes());
    f.put(0x8138, (-15_000_i64).to_le_bytes());
    f.put(0x8140, 0x8200_u64.to_le_bytes());
    f.put(0x8200, (crate::cidea::VTABLE_RVA as u64).to_le_bytes());
    f.text(0x8218, b"idea_penalty");
    f.array(TECH + 0x128, 0x8400, 1);
    f.put(0x8400, 0x8500_u64.to_le_bytes());
    f.text(0x8408, b"script_bonus");
    f.put(0x8530, 25_000_i64.to_le_bytes());
    let mut rows = [Row::default(); 64];
    let result = f.capture(&[f.input()], &mut rows);
    assert!(matches!(result, Outcome::Rows { .. }), "{result:?}");
    assert!(
        rows[..result.len()]
            .iter()
            .any(|r| r.kind == Kind::IdeaSource
                && r.raw == -15_000
                && r.name.as_bytes() == b"idea_penalty")
    );
    assert!(
        rows[..result.len()]
            .iter()
            .any(|r| r.kind == Kind::ScriptSource
                && r.raw == 25_000
                && r.name.as_bytes() == b"script_bonus")
    );
}

#[test]
fn license_and_scientist_use_only_matching_equipment_and_category() {
    let mut f = Fixture::new();
    f.array(TECH + 0x68, 0x8000, 1);
    f.put(0x8000, 0x8100_u64.to_le_bytes());
    f.put(
        COUNTRY + ccountry::PRODUCTION_STATUS,
        0x9000_u64.to_le_bytes(),
    );
    f.array(0x91b0, 0x9200, 1);
    f.put(0x9200, 0x9300_u64.to_le_bytes());
    f.put(0x96f0, 0x8100_u64.to_le_bytes());
    f.array(TECH + 0x140, 0x9800, 1);
    f.put(0x9800, 17_u32.to_le_bytes());
    f.put(COUNTRY + 0xfa8, 0xa000_u64.to_le_bytes());
    f.put(0xa088, 0xa100_u64.to_le_bytes());
    f.put(
        0xa000,
        (crate::nproject_cprogram_status::VTABLE_RVA as u64).to_le_bytes(),
    );
    f.put(
        0xa008,
        (crate::nproject_cprogram_status::SECONDARY_VTABLE_RVA as u64).to_le_bytes(),
    );
    f.put(0xa010, 1_u32.to_le_bytes());
    f.put(0xa104, [1]);
    f.put(0xa108, 17_u32.to_le_bytes());
    f.put(0xa110, 8_765_i64.to_le_bytes());
    let mut rows = [Row::default(); 64];
    let result = f.capture(&[f.input()], &mut rows);
    assert!(matches!(result, Outcome::Rows { .. }), "{result:?}");
    assert!(
        rows[..result.len()]
            .iter()
            .any(|r| r.kind == Kind::License && r.raw == 20_000)
    );
    assert!(
        rows[..result.len()]
            .iter()
            .any(|r| r.kind == Kind::ScientistCategory && r.raw == 8_765)
    );
    f.put(0xa108, 18_u32.to_le_bytes());
    let result = f.capture(&[f.input()], &mut rows);
    assert!(
        rows[..result.len()]
            .iter()
            .any(|r| r.kind == Kind::ScientistCategory && r.raw == 0)
    );
}

#[test]
fn sharing_resolves_aliases_and_uses_max_level_with_predecessor_gate() {
    let mut f = Fixture::new();
    f.array(STATUS + 0x118, 0x8000, 1);
    f.put(0x8000, 0x8100_u64.to_le_bytes());
    f.put(0x8100, (sharing::FACTION_VTABLE as u64).to_le_bytes());
    f.put(0x8108, 4_u32.to_le_bytes());
    f.put(0x8110, 10_000_i64.to_le_bytes());
    f.array(0x8118, 0x8200, 1);
    f.put(0x8200, 2_u32.to_le_bytes());
    f.array(0x8130, 0x8300, 1);
    f.put(0x8300, 5_000_i64.to_le_bytes());
    f.array(
        STATUS + crate::ctechnology_status::TECHNOLOGIES_DATA,
        0x8400,
        1,
    );
    f.put(0x8400, (TECH as u64).to_le_bytes());
    f.put(TECH, (crate::ctechnology::VTABLE_RVA as u64).to_le_bytes());
    f.put(TECH + 8, 7_u32.to_le_bytes());
    f.put(TECH + 0x158, (STATUS as u64).to_le_bytes());
    f.put(TECH + 0x160, (TEMPLATE as u64).to_le_bytes());
    f.put(TECH + 0x174, 1_i32.to_le_bytes());
    let mut rows = [Row::default(); 64];
    let result = f.capture(&[f.input()], &mut rows);
    assert!(matches!(result, Outcome::Rows { .. }), "{result:?}");
    let row = rows[..result.len()]
        .iter()
        .find(|r| r.kind == Kind::Sharing)
        .unwrap();
    assert_eq!(
        (row.raw, row.auxiliary, row.count, row.source_id),
        (10_000, 5_000, 1, 4)
    );
    f.array(TECH + 0xc8, 0x8500, 1);
    f.put(0x8500, 0x8600_u64.to_le_bytes());
    f.put(0x8770, 6_u32.to_le_bytes());
    let result = f.capture(&[f.input()], &mut rows);
    assert!(
        rows[..result.len()]
            .iter()
            .any(|r| r.kind == Kind::SharingPredecessor && r.raw == 6)
    );
    assert!(
        rows[..result.len()]
            .iter()
            .any(|r| r.kind == Kind::SharingMember
                && r.raw == 1
                && r.auxiliary == 1
                && r.source_id == 2)
    );
    f.put(TEMPLATE + 0x409, [1]);
    f.fault = Some(TEMPLATE + 0x2c0);
    let result = f.capture(&[f.input()], &mut rows);
    assert!(
        rows[..result.len()]
            .iter()
            .any(|r| r.kind == Kind::SharingDoctrine && r.raw == 1)
    );
    assert!(matches!(result, Outcome::Rows { .. }));
    assert!(!rows[..result.len()].iter().any(|r| r.kind == Kind::Sharing));
}

#[test]
fn scientist_efficiency_sources_preserve_character_identity_and_cached_total() {
    let mut f = Fixture::new();
    f.array(TECH + 0x140, 0x9800, 1);
    f.put(0x9800, 17_u32.to_le_bytes());
    f.put(COUNTRY + ccountry::PROGRAM_STATUS, 0xa000_u64.to_le_bytes());
    f.put(
        0xa000,
        (crate::nproject_cprogram_status::VTABLE_RVA as u64).to_le_bytes(),
    );
    f.put(
        0xa008,
        (crate::nproject_cprogram_status::SECONDARY_VTABLE_RVA as u64).to_le_bytes(),
    );
    f.put(0xa010, 2_u32.to_le_bytes());
    f.put(0xa088, 0xa100_u64.to_le_bytes());
    f.put(0xa104, [1]);
    f.put(0xa108, 17_u32.to_le_bytes());
    f.put(0xa110, 8_765_i64.to_le_bytes());
    f.array(0xa118, 0xa200, 1);
    f.put(0xa200, 0xb000_u64.to_le_bytes());
    f.put(0xb000, (crate::cscientist::VTABLE_RVA as u64).to_le_bytes());
    f.put(0xb008, 0xc000_u64.to_le_bytes());
    f.modifier(0xb058, 0xb200, 0x27a, 50_000);
    f.put(0xc000, (crate::ccharacter::VTABLE_RVA as u64).to_le_bytes());
    f.put(0xc008, crate::ccharacter::REFERENCE_TYPE.to_le_bytes());
    f.put(0xc00c, 91_u32.to_le_bytes());
    f.put(0xc0c0, 0xb000_u64.to_le_bytes());
    f.text(0xc048, b"scientist_name");
    let mut rows = [Row::default(); 64];
    let result = f.capture(&[f.input()], &mut rows);
    assert!(matches!(result, Outcome::Rows { .. }), "{result:?}");
    let source = rows[..result.len()]
        .iter()
        .find(|r| r.kind == Kind::ScientistSource)
        .unwrap();
    assert_eq!(
        (source.raw, source.source_id, source.count),
        (50_000, 91, 17)
    );
    assert_eq!(source.name.as_bytes(), b"scientist_name");
    assert_eq!(
        rows[..result.len()]
            .iter()
            .find(|r| r.kind == Kind::ScientistCategory)
            .unwrap()
            .raw,
        8_765
    );
    f.put(0xc0c0, 0xb100_u64.to_le_bytes());
    assert!(matches!(
        f.capture(&[f.input()], &mut rows),
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn scientist_hash_collisions_keep_each_raw_category_without_summing() {
    let mut f = Fixture::new();
    f.array(TECH + 0x140, 0x9800, 2);
    f.put(0x9800, 17_u32.to_le_bytes());
    f.put(0x9804, 18_u32.to_le_bytes());
    f.put(COUNTRY + ccountry::PROGRAM_STATUS, 0xa000_u64.to_le_bytes());
    f.put(
        0xa000,
        (crate::nproject_cprogram_status::VTABLE_RVA as u64).to_le_bytes(),
    );
    f.put(
        0xa008,
        (crate::nproject_cprogram_status::SECONDARY_VTABLE_RVA as u64).to_le_bytes(),
    );
    f.put(0xa010, 1_u32.to_le_bytes());
    f.put(0xa088, 0xa100_u64.to_le_bytes());
    f.put(0xa098, [1]);
    for (offset, category, distance) in [(0_usize, 17_u32, 1_u8), (48, 18, 2)] {
        f.put(0xa104 + offset, [distance]);
        f.put(0xa108 + offset, category.to_le_bytes());
        f.put(0xa110 + offset, i64::MAX.to_le_bytes());
    }
    let mut rows = [Row::default(); 64];
    let result = f.capture(&[f.input()], &mut rows);
    assert!(matches!(result, Outcome::Rows { .. }), "{result:?}");
    let categories: Vec<_> = rows[..result.len()]
        .iter()
        .filter(|r| r.kind == Kind::ScientistCategory)
        .map(|r| (r.source_id, r.raw))
        .collect();
    assert_eq!(categories, [(17, i64::MAX), (18, i64::MAX)]);
    f.fault = Some(0xa130);
    assert!(matches!(
        f.capture(&[f.input()], &mut rows),
        Outcome::Unavailable { .. }
    ));
}
