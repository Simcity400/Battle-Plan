use super::*;
use engine_native::ccountry::synthetic::World;
use std::cell::Cell;

struct Fixture {
    world: World,
    lexer: [u8; 64],
    database: usize,
    agencies: Vec<usize>,
    entries: usize,
    definitions: [usize; 3],
    branches: [usize; 2],
    denied: Option<(usize, usize)>,
    changed_lexer: bool,
    witness_reads: Cell<u32>,
}

impl Memory for Fixture {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if self.denied.is_some_and(|(start, len)| {
            address < start + len && start < address.saturating_add(destination.len())
        }) {
            return false;
        }
        if address == self.world.module + DATABASE_RVA.0 {
            destination.copy_from_slice(&(self.database as u64).to_le_bytes());
            return true;
        }
        if let Some(offset) = address.checked_sub(self.world.module + cpdx_lexer::INITIALIZED_RVA.0)
            && offset + destination.len() <= self.lexer.len()
        {
            destination.copy_from_slice(&self.lexer[offset..offset + destination.len()]);
            if destination.len() == 64 {
                let reads = self.witness_reads.get();
                self.witness_reads.set(reads + 1);
                if self.changed_lexer && reads != 0 {
                    destination[4..8].copy_from_slice(&62_i32.to_le_bytes());
                }
            }
            return true;
        }
        self.world.memory().copy(address, destination)
    }
}

impl Fixture {
    fn new(countries: u32) -> Self {
        let mut world = World::new(0x4000_0000, countries, 0x1000);
        let module = world.module;
        let dictionary = world.alloc(64 * cpdx_lexer::TOKEN_NAME_STRIDE);
        for token in 0..64 {
            world.u64(dictionary + token * 32 + 24, 15);
        }
        for (token, name) in [
            (7, "mod_upgrade"),
            (8, "second_upgrade"),
            (11, "branch_alpha"),
            (12, "branch_beta"),
        ] {
            world.put(dictionary + token * 32, name.as_bytes());
            world.u64(dictionary + token * 32 + 16, name.len() as u64);
        }
        let mut lexer = [0; 64];
        lexer[0] = 1;
        let mut put = |rva, bytes: &[u8]| {
            lexer[rva - cpdx_lexer::INITIALIZED_RVA.0
                ..rva - cpdx_lexer::INITIALIZED_RVA.0 + bytes.len()]
                .copy_from_slice(bytes)
        };
        put(cpdx_lexer::MAX_TOKEN_ID_RVA.0, &63_i32.to_le_bytes());
        put(cpdx_lexer::STATIC_MAX_TOKEN_ID_RVA.0, &1_i32.to_le_bytes());
        put(cpdx_lexer::DYNAMIC_TOKENS_RVA.0, &0x2000_u64.to_le_bytes());
        put(cpdx_lexer::DYNAMIC_CAPACITY_RVA.0, &62_i32.to_le_bytes());
        put(cpdx_lexer::DYNAMIC_COUNT_RVA.0, &62_i32.to_le_bytes());
        put(
            cpdx_lexer::TOKEN_NAMES_RVA.0,
            &(dictionary as u64).to_le_bytes(),
        );
        put(cpdx_lexer::TOKEN_ALLOCATION_RVA.0, &64_i32.to_le_bytes());
        put(cpdx_lexer::TOKEN_CAPACITY_RVA.0, &64_i32.to_le_bytes());
        let database = world.alloc(0x100);
        world.u64(database, (module + DATABASE_VTABLE_RVA.0) as u64);
        let branches = [world.alloc(0x60), world.alloc(0x60)];
        let branch_vector = world.alloc(3 * 8);
        world.array(database + DATABASE_BRANCHES, branch_vector, 3);
        for (index, branch) in branches.iter().copied().enumerate() {
            world.u64(branch_vector + (index + 1) * 8, branch as u64);
            world.u64(branch, (module + BRANCH_VTABLE_RVA.0) as u64);
            world.put(branch + VALID, &[0x7f]);
            world.u32(branch + TOKEN, 11 + index as u32);
        }
        let definitions = [world.alloc(0x270), world.alloc(0x270), world.alloc(0x270)];
        for (index, definition) in definitions.iter().copied().enumerate() {
            world.u64(definition, (module + DEFINITION_VTABLE_RVA.0) as u64);
            world.put(definition + VALID, &[0x7f]);
            world.u32(definition + TOKEN, if index == 1 { 8 } else { 7 });
            world.u64(
                definition + DEFINITION_BRANCH,
                branches[usize::from(index == 2)] as u64,
            );
        }
        let first_members = world.alloc(2 * 8);
        world.u64(first_members, definitions[0] as u64);
        world.u64(first_members + 8, definitions[1] as u64);
        world.array(branches[0] + BRANCH_UPGRADES, first_members, 2);
        let second_members = world.alloc(8);
        world.u64(second_members, definitions[2] as u64);
        world.array(branches[1] + BRANCH_UPGRADES, second_members, 1);
        let globals = world.alloc(3 * 8);
        world.u64(globals + 8, definitions[0] as u64);
        world.u64(globals + 16, definitions[1] as u64);
        world.array(database + DATABASE_UPGRADES, globals, 3);
        let entries = world.alloc(3 * ENTRY_STRIDE);
        for (index, (definition, level)) in [
            (definitions[0], -7_i32),
            (definitions[2], i32::MAX),
            (definitions[0], 0),
        ]
        .into_iter()
        .enumerate()
        {
            world.u64(entries + index * ENTRY_STRIDE, definition as u64);
            world.put(entries + index * ENTRY_STRIDE + 8, &level.to_le_bytes());
            world.u32(entries + index * ENTRY_STRIDE + 12, 0xdead_beef);
        }
        let mut agencies = Vec::new();
        for country in 1..=countries {
            let agency = world.alloc(cintelligence_agency::DEFENSE + 8);
            world.u64(
                world.object(country) + ccountry::INTELLIGENCE_AGENCY,
                agency as u64,
            );
            world.u64(agency, (module + cintelligence_agency::VTABLE_RVA.0) as u64);
            world.u64(
                agency + cintelligence_agency::OWNER_COUNTRY,
                world.object(country) as u64,
            );
            world.u64(agency + cintelligence_agency::NAME + 24, 15);
            world.put(
                agency + cintelligence_agency::CREATED,
                &[(country % 2) as u8],
            );
            world.array(agency + ACQUIRED, entries, 3);
            agencies.push(agency);
        }
        Self {
            world,
            lexer,
            database,
            agencies,
            entries,
            definitions,
            branches,
            denied: None,
            changed_lexer: false,
            witness_reads: Cell::new(0),
        }
    }
    fn capture(
        &self,
        rows: &mut Vec<Row>,
        catalog: &mut Catalog,
    ) -> cintelligence_agency::UpgradeWalk {
        let captured = cintelligence_agency::walk_with_upgrades(
            self,
            self.world.module,
            self.world.game_state(),
            &mut Vec::new(),
            &mut Vec::new(),
            rows,
            catalog,
        );
        match captured.upgrades.outcome {
            Outcome::Rows { len } => {
                assert_eq!(len, captured.upgrades.required_rows);
            }
            Outcome::Truncated { len, unread_rows } => {
                assert_eq!(len + unread_rows as usize, captured.upgrades.required_rows);
            }
            Outcome::Unavailable { .. } => {}
        }
        captured
    }
}

#[test]
fn worker_grows_first_capture_and_later_catalog_at_unchanged_countries() {
    const COUNTRIES: usize = 4;
    let mut fixture = Fixture::new(COUNTRIES as u32);
    let mut agencies = Vec::new();
    let mut names = Vec::new();
    let mut rows = Vec::new();
    let mut catalog = Catalog::new(0);
    let first = cintelligence_agency::walk_with_upgrades(
        &fixture,
        fixture.world.module,
        fixture.world.game_state(),
        &mut agencies,
        &mut names,
        &mut rows,
        &mut catalog,
    );
    assert_eq!(first.agencies.outcome, Outcome::Rows { len: COUNTRIES });
    assert_eq!(first.upgrades.outcome, Outcome::Rows { len: 3 * COUNTRIES });
    assert_eq!(first.upgrades.refused_rows, 0);
    assert_eq!(catalog.required_capacity(), 3);
    assert!(catalog.capacity() >= 3);
    let first_row_capacity = rows.len();

    let members = fixture.world.alloc((COUNTRIES + 2) * 8);
    fixture.world.u64(members, fixture.definitions[0] as u64);
    fixture
        .world
        .u64(members + 8, fixture.definitions[1] as u64);
    let mut definitions = Vec::new();
    for index in 0..COUNTRIES {
        let definition = fixture.world.alloc(0x270);
        fixture.world.u64(
            definition,
            (fixture.world.module + DEFINITION_VTABLE_RVA.0) as u64,
        );
        fixture.world.put(definition + VALID, &[1]);
        fixture.world.u32(definition + TOKEN, 7);
        fixture
            .world
            .u64(definition + DEFINITION_BRANCH, fixture.branches[0] as u64);
        fixture
            .world
            .u64(members + (index + 2) * 8, definition as u64);
        definitions.push(definition);
    }
    fixture.world.array(
        fixture.branches[0] + BRANCH_UPGRADES,
        members,
        COUNTRIES as u32 + 2,
    );
    for (index, &agency) in fixture.agencies.iter().enumerate() {
        let acquired = fixture.world.alloc(10 * ENTRY_STRIDE);
        for ordinal in 0..10 {
            let definition = if ordinal == 9 {
                definitions[index]
            } else {
                fixture.definitions[ordinal % 3]
            };
            fixture
                .world
                .u64(acquired + ordinal * ENTRY_STRIDE, definition as u64);
            fixture.world.put(
                acquired + ordinal * ENTRY_STRIDE + 8,
                &(-(ordinal as i32)).to_le_bytes(),
            );
        }
        fixture.world.array(agency + ACQUIRED, acquired, 10);
    }
    assert!(first_row_capacity < 10 * COUNTRIES);
    assert_eq!(
        native_demand(&fixture, fixture.world.module, fixture.world.game_state()),
        Ok(Demand {
            rows: 10 * COUNTRIES,
            catalog: COUNTRIES + 3
        })
    );
    let later = cintelligence_agency::walk_with_upgrades(
        &fixture,
        fixture.world.module,
        fixture.world.game_state(),
        &mut agencies,
        &mut names,
        &mut rows,
        &mut catalog,
    );
    assert_eq!(later.agencies.outcome, Outcome::Rows { len: COUNTRIES });
    assert_eq!(
        later.upgrades.outcome,
        Outcome::Rows {
            len: 10 * COUNTRIES
        }
    );
    assert_eq!(later.upgrades.refused_rows, 0);
    assert_eq!(later.upgrades.omitted_rows, 0);
    assert_eq!(catalog.required_capacity(), COUNTRIES + 3);
    assert!(catalog.capacity() >= COUNTRIES + 3);
    for (index, country_rows) in rows[..10 * COUNTRIES].chunks_exact(10).enumerate() {
        assert!(
            country_rows
                .iter()
                .all(|row| row.country as usize == index + 1)
        );
        assert_eq!(country_rows[9].ordinal, 9);
        assert_eq!(country_rows[9].level, -9);
        assert_eq!(
            country_rows[9].definition_pointer,
            definitions[index] as u64
        );
        assert!(
            country_rows
                .iter()
                .all(|row| row.identity_status == AUTHENTICATED)
        );
    }
}

#[test]
fn agency_upgrade_levels_every_country_preserves_branch_duplicates_raw_levels_and_order() {
    const COUNTRIES: usize = 4;
    let fixture = Fixture::new(COUNTRIES as u32);
    let demand = native_demand(&fixture, fixture.world.module, fixture.world.game_state()).unwrap();
    assert_eq!(
        demand,
        Demand {
            rows: 3 * COUNTRIES,
            catalog: 3
        }
    );
    let mut rows = vec![Row::default(); demand.rows];
    let mut catalog = Catalog::new(demand.catalog);
    let captured = fixture.capture(&mut rows, &mut catalog);
    assert_eq!(
        captured.upgrades.outcome,
        Outcome::Rows { len: 3 * COUNTRIES }
    );
    assert_eq!(captured.upgrades.required_rows, 3 * COUNTRIES);
    assert_eq!(captured.upgrades.refused_rows, 0);
    assert_eq!(catalog.required_capacity(), 3);
    for country in 1..=COUNTRIES as u32 {
        let sample = &rows[(country as usize - 1) * 3..country as usize * 3];
        assert_eq!(
            sample
                .iter()
                .map(|r| (r.country, r.ordinal, r.level))
                .collect::<Vec<_>>(),
            vec![(country, 0, -7), (country, 1, i32::MAX), (country, 2, 0)]
        );
        assert_eq!(sample[0].upgrade_token, sample[1].upgrade_token);
        assert_eq!(
            (sample[0].branch_token, sample[1].branch_token),
            (Some(11), Some(12))
        );
        assert_eq!(sample[0].definition_pointer, sample[2].definition_pointer);
        assert_eq!(sample[1].branch_pointer, fixture.branches[1] as u64);
    }
}

#[test]
fn agency_upgrade_levels_short_storage_grows_to_the_exact_demand_without_clamping_raw_rows() {
    let fixture = Fixture::new(2);
    let mut rows = vec![Row::default(); 4];
    let mut catalog = Catalog::new(2);
    let captured = fixture.capture(&mut rows, &mut catalog);
    assert_eq!(captured.upgrades.required_rows, 6);
    assert_eq!(catalog.required_capacity(), 3);
    assert_eq!(captured.upgrades.outcome, Outcome::Rows { len: 6 });
    assert_eq!(captured.upgrades.omitted_rows, 0);
    assert_eq!(captured.upgrades.refused_rows, 0);
    assert_eq!(
        captured.upgrades.refusal_counts[CATALOG_CAPACITY as usize],
        0
    );
    assert_eq!(
        rows[..6].iter().map(|r| r.level).collect::<Vec<_>>(),
        [-7, i32::MAX, 0, -7, i32::MAX, 0]
    );
}

#[test]
fn agency_upgrade_levels_null_and_unregistered_entries_are_retained_named_and_counted() {
    let mut fixture = Fixture::new(1);
    fixture.world.u64(fixture.entries, 0);
    fixture
        .world
        .u64(fixture.entries + ENTRY_STRIDE, 0xfeed_cafe);
    let mut rows = vec![Row::default(); 3];
    let captured = fixture.capture(&mut rows, &mut Catalog::new(3));
    assert_eq!(captured.upgrades.outcome, Outcome::Rows { len: 3 });
    assert_eq!(captured.upgrades.required_rows, 3);
    assert_eq!(captured.upgrades.refused_rows, 2);
    assert_eq!(captured.upgrades.omitted_rows, 0);
    assert_eq!(
        captured.upgrades.refusal_counts[NULL_DEFINITION as usize],
        1
    );
    assert_eq!(
        captured.upgrades.refusal_counts[DEFINITION_NOT_REGISTERED as usize],
        1
    );
    assert_eq!((rows[0].level, rows[1].level), (-7, i32::MAX));
    assert_eq!(rows[1].definition_pointer, 0xfeed_cafe);
    assert_eq!(rows[1].upgrade_token, None);
    assert_eq!(captured.upgrades.first_refused.unwrap().ordinal, 0);
    assert!(status_name(rows[1].identity_status).contains("not registered"));
    assert_eq!(rows[2].identity_status, AUTHENTICATED);
}

#[test]
fn agency_upgrade_levels_refuses_definition_forms_owner_and_lexical_identity_without_following_unknowns()
 {
    for status in [
        DEFINITION_VTABLE_MISMATCH,
        DEFINITION_INVALID,
        DEFINITION_UNREADABLE,
        BRANCH_OWNER_MISMATCH,
        UPGRADE_TOKEN_UNAVAILABLE,
        BRANCH_TOKEN_UNAVAILABLE,
    ] {
        let mut fixture = Fixture::new(1);
        let definition = fixture.definitions[0];
        match status {
            DEFINITION_VTABLE_MISMATCH => fixture.world.u64(definition, 0xdead_beef),
            DEFINITION_INVALID => fixture.world.put(definition + VALID, &[0]),
            DEFINITION_UNREADABLE => fixture.denied = Some((definition + DEFINITION_BRANCH, 1)),
            BRANCH_OWNER_MISMATCH => fixture
                .world
                .u64(definition + DEFINITION_BRANCH, 0xdead_beef),
            UPGRADE_TOKEN_UNAVAILABLE => fixture.world.u32(definition + TOKEN, 100_000),
            BRANCH_TOKEN_UNAVAILABLE => fixture.world.u32(fixture.branches[0] + TOKEN, 100_000),
            _ => unreachable!(),
        }
        let mut rows = vec![Row::default(); 3];
        let captured = fixture.capture(&mut rows, &mut Catalog::new(3));
        assert_eq!(
            captured.upgrades.outcome,
            Outcome::Rows { len: 3 },
            "{status}"
        );
        assert_eq!(rows[0].identity_status, status, "{}", status_name(status));
        assert_eq!(rows[2].identity_status, status);
        assert_eq!(rows[0].definition_pointer, definition as u64);
        assert_eq!(rows[0].level, -7);
        assert_eq!(rows[1].identity_status, AUTHENTICATED);
        if status == BRANCH_OWNER_MISMATCH {
            assert_eq!(rows[0].branch_pointer, 0xdead_beef);
        }
    }
}

#[test]
fn agency_upgrade_levels_reset_empty_created_and_uncreated_skip_the_database_and_do_not_reuse_catalog()
 {
    let mut fixture = Fixture::new(2);
    let mut catalog = Catalog::new(3);
    let mut rows = vec![Row::default(); 6];
    assert_eq!(
        fixture.capture(&mut rows, &mut catalog).upgrades.outcome,
        Outcome::Rows { len: 6 }
    );
    for agency in &fixture.agencies {
        fixture.world.array(*agency + ACQUIRED, fixture.entries, 0);
    }
    fixture.denied = Some((fixture.world.module + DATABASE_RVA.0, 8));
    assert_eq!(
        native_demand(&fixture, fixture.world.module, fixture.world.game_state()).unwrap(),
        Demand::default()
    );
    let captured = fixture.capture(&mut rows, &mut catalog);
    assert_eq!(captured.upgrades.outcome, Outcome::Rows { len: 0 });
    assert_eq!(captured.upgrades.required_rows, 0);
    assert_eq!(catalog.required_capacity(), 0);
    assert_eq!(fixture.witness_reads.get(), 2);
}

#[test]
fn agency_upgrade_levels_bad_owner_and_short_acquired_reads_are_unavailable() {
    let mut fixture = Fixture::new(1);
    fixture.world.u64(
        fixture.agencies[0] + cintelligence_agency::OWNER_COUNTRY,
        fixture.world.object(1) as u64 + 8,
    );
    let mut rows = vec![Row::default(); 3];
    let captured = fixture.capture(&mut rows, &mut Catalog::new(3));
    assert_eq!(
        captured.agencies.outcome,
        Outcome::Unavailable {
            reason: "agency owner pointer mismatch"
        }
    );
    assert_eq!(captured.upgrades.outcome, captured.agencies.outcome);
    fixture.world.u64(
        fixture.agencies[0] + cintelligence_agency::OWNER_COUNTRY,
        fixture.world.object(1) as u64,
    );
    fixture.denied = Some((fixture.entries + ENTRY_STRIDE + 15, 1));
    let captured = fixture.capture(&mut rows, &mut Catalog::new(3));
    assert_eq!(captured.upgrades.required_rows, 3);
    assert_eq!(
        captured.upgrades.outcome,
        Outcome::Unavailable {
            reason: "agency upgrade acquired entry unreadable"
        }
    );
}

#[test]
fn agency_upgrade_levels_changed_dictionary_preserves_raw_entries_and_withholds_identity() {
    let mut fixture = Fixture::new(1);
    fixture.changed_lexer = true;
    let mut rows = vec![Row::default(); 3];
    let captured = fixture.capture(&mut rows, &mut Catalog::new(3));
    assert_eq!(captured.upgrades.outcome, Outcome::Rows { len: 3 });
    assert_eq!(captured.upgrades.refusal_counts[LEXER_CHANGED as usize], 3);
    assert!(
        rows.iter()
            .all(|r| r.upgrade_token.is_none() && r.branch_token.is_none())
    );
    assert_eq!(rows[0].level, -7);
    assert_eq!(rows[1].definition_pointer, fixture.definitions[2] as u64);
}

#[test]
fn agency_upgrade_levels_database_forms_and_duplicate_memberships_have_named_counts() {
    for status in [
        DATABASE_UNAVAILABLE,
        DATABASE_UNSUPPORTED,
        BRANCH_UNAVAILABLE,
        DUPLICATE_MEMBERSHIP,
    ] {
        let mut fixture = Fixture::new(1);
        match status {
            DATABASE_UNAVAILABLE => {
                fixture.denied = Some((fixture.world.module + DATABASE_RVA.0, 8))
            }
            DATABASE_UNSUPPORTED => fixture.world.u64(fixture.database, 0xdead_beef),
            BRANCH_UNAVAILABLE => fixture.world.u64(fixture.branches[0], 0xdead_beef),
            DUPLICATE_MEMBERSHIP => {
                let members =
                    read::<u64>(&fixture, fixture.branches[0] + BRANCH_UPGRADES).unwrap() as usize;
                fixture
                    .world
                    .u64(members + 8, fixture.definitions[0] as u64);
                let globals =
                    read::<u64>(&fixture, fixture.database + DATABASE_UPGRADES).unwrap() as usize;
                fixture
                    .world
                    .u64(globals + 16, fixture.definitions[0] as u64);
            }
            _ => unreachable!(),
        }
        let mut rows = vec![Row::default(); 3];
        let captured = fixture.capture(&mut rows, &mut Catalog::new(3));
        let refused = if status == DUPLICATE_MEMBERSHIP { 2 } else { 3 };
        assert_eq!(captured.upgrades.outcome, Outcome::Rows { len: 3 });
        assert_eq!(captured.upgrades.refusal_counts[status as usize], refused);
        assert_eq!(rows[0].identity_status, status);
        assert_eq!(rows[0].level, -7);
        assert_eq!(rows[0].definition_pointer, fixture.definitions[0] as u64);
        assert_ne!(
            status_name(status),
            "unknown agency upgrade identity status"
        );
    }
}
