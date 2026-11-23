use super::*;

use engine_native_memory::memory::SliceMemory;

const START: usize = 0x1000_0000;
const BASE: usize = START - cgame_state::SINGLETON_RVA.0;
const COUNTRY: [usize; 3] = [START + 0x1000, START + 0x2000, START + 0x3000];
const DIPLOMACY: [usize; 2] = [START + 0x5000, START + 0x6000];
const STATUS: usize = START + 0x8200;

fn put_u64(bytes: &mut [u8], address: usize, value: u64) {
    let at = address - START;
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut [u8], address: usize, value: u32) {
    let at = address - START;
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_array(bytes: &mut [u8], address: usize, data: usize, count: u32) {
    put_u64(bytes, address, data as u64);
    put_u32(bytes, address + 8, count);
    put_u32(bytes, address + 12, count);
}

fn world() -> Vec<u8> {
    let mut bytes = vec![0_u8; 0x10000];
    let state = START + 0x100;
    put_u64(&mut bytes, START, state as u64);
    put_array(&mut bytes, state + cgame_state::COUNTRIES, START + 0x800, 4);
    for (slot, country) in COUNTRY.iter().enumerate() {
        put_u64(&mut bytes, START + 0x800 + (slot + 1) * 8, *country as u64);
    }
    put_array(
        &mut bytes,
        state + cgame_state::COUNTRY_LINKS,
        START + 0x900,
        40,
    );
    for (raw, slot) in [(7, 1), (9, 2), (30, 2), (11, 3)] {
        put_u32(&mut bytes, START + 0x900 + raw * 4, slot);
    }
    for (country, id) in COUNTRY.into_iter().zip([7, 9, 11]) {
        put_u32(&mut bytes, country + ccountry::ID, id);
    }
    for (country, diplomacy) in COUNTRY.into_iter().zip(DIPLOMACY) {
        put_u64(
            &mut bytes,
            country + ccountry::DIPLOMACY_STATUS,
            diplomacy as u64,
        );
    }
    put_u64(
        &mut bytes,
        DIPLOMACY[0] + engine_native_parts::cdiplomacy_status_layout::COUNTRY,
        COUNTRY[0] as u64,
    );
    put_u64(
        &mut bytes,
        START + 0x7000 + crelation_status::DIPLOMACY_STATUS,
        DIPLOMACY[0] as u64,
    );
    put_u32(&mut bytes, START + 0x7400 + crelation::FIRST_COUNTRY, 7);
    put_u32(&mut bytes, START + 0x7400 + crelation::SECOND_COUNTRY, 30);
    put_u64(
        &mut bytes,
        START + 0x7800 + engine_native::ccountry::rosters::OWNER,
        COUNTRY[1] as u64,
    );
    put_u64(
        &mut bytes,
        START + 0x7a00 + engine_native_parts::cpolitical_status_layout::COUNTRY,
        COUNTRY[0] as u64,
    );
    put_u32(&mut bytes, START + 0x7c00 + cpolitical_party::TAG, 30);
    put_u32(&mut bytes, START + 0x7e00 + ccharacter::COUNTRY, 11);
    put_u64(
        &mut bytes,
        START + 0x8000 + SCIENTIST_CHARACTER,
        (START + 0x7e00) as u64,
    );
    put_u64(
        &mut bytes,
        STATUS + ctechnology_status::COUNTRY,
        COUNTRY[1] as u64,
    );
    put_u64(
        &mut bytes,
        START + 0x8400 + cresearch_slot::STATUS,
        STATUS as u64,
    );
    put_u64(
        &mut bytes,
        START + 0x8600 + ctechnology::STATUS,
        STATUS as u64,
    );
    put_u32(
        &mut bytes,
        START + 0x8800 + nproject_cprogram_status::OWNER,
        7,
    );
    put_u64(
        &mut bytes,
        START + 0x8a00 + cnational_focus_progress::COUNTRY,
        COUNTRY[2] as u64,
    );
    put_u32(&mut bytes, START + 0x8c00 + nproject_cproject::OWNER, 30);
    put_u32(
        &mut bytes,
        START + 0x8e00 + engine_native::cproduction_status::equipment_license::OWNER,
        30,
    );
    bytes
}

fn named(bytes: &[u8], receiver: Receiver, first: usize, second: usize) -> (bool, Vec<u32>) {
    let memory = SliceMemory { base: START, bytes };
    let mut ids = Vec::new();
    let named = countries(&memory, BASE, receiver, first, second, &mut |id| {
        ids.push(id)
    });
    (named, ids)
}

#[test]
fn sites_are_distinct_and_agree_with_their_layout_constants() {
    let mut rvas: Vec<Rva> = MUTATIONS.iter().map(|(site, _, _)| site.rva).collect();
    rvas.sort_unstable();
    rvas.dedup();
    assert_eq!(rvas.len(), MUTATIONS.len());
    let site = |name: &str| {
        MUTATIONS
            .iter()
            .find(|(site, _, _)| site.name == name)
            .unwrap()
    };
    assert_eq!(MUTATIONS[SYNC_RESEARCH_SLOTS].0.name, "sync_research_slots");
    assert_eq!(MUTATIONS[GROW_INTEL_NETWORK].0.name, "grow_intel_network");
    assert_eq!(MUTATIONS[GROW_INTEL_NETWORK].2, Tables::INTEL_NETWORKS);
    assert_eq!(
        MUTATIONS[PROJECT_REWARD_OPTION].0.name,
        "project_reward_option"
    );
    assert_eq!(
        MUTATIONS[PROJECT_DATABASE_RESYNC].0.name,
        "project_database_resync"
    );
    assert_eq!(site("add_stability").0.rva, ccountry::ADD_STABILITY_RVA);
    for name in [
        "register_license",
        "license_removal",
        "license_extension",
        "faction_set_leader",
    ] {
        assert_eq!(site(name).2, Tables::RESEARCH, "{name}");
    }
    assert_eq!(site("license_removal").1, Receiver::License);
    assert_eq!(site("license_extension").1, Receiver::License);
    assert_eq!(site("weekly_update").0.rva, ccountry::WEEKLY_UPDATE_RVA);
    assert_eq!(
        site("research_completed").0.rva,
        ctechnology_status::RESEARCH_COMPLETED_RVA
    );
    assert_eq!(
        site("recalculate_modifiers").2,
        Tables::BOTH.with(Tables::COUNTERS).with(Tables::VISIBILITY)
    );
    assert_eq!(
        site("replace_relation").2,
        Tables::STABILITY.with(Tables::COUNTERS)
    );
    assert_eq!(
        CREW_WRITERS
            .clone()
            .map(|index| MUTATIONS[index].0.name)
            .collect::<Vec<_>>(),
        [
            "refresh_wing_crews",
            "refresh_ship_crews",
            "remove_ship_crews"
        ]
    );
    for (index, (site, receiver, tables)) in MUTATIONS.iter().enumerate() {
        if CREW_WRITERS.contains(&index) {
            assert_eq!(*tables, Tables::NONE, "{}", site.name);
            assert_eq!(*receiver, Receiver::Every, "{}", site.name);
            continue;
        }
        assert!(
            tables.stability
                || tables.research
                || tables.focus
                || tables.intel_networks
                || tables.projects
                || tables.appearance
                || tables.counters
                || tables.visibility
                || tables.weather,
            "{}",
            site.name
        );
        assert!(!tables.technologies || tables.research, "{}", site.name);
    }
    for name in [
        "set_technology",
        "set_level",
        "claim_limited_use_bonuses",
        "release_limited_use_bonuses",
        "complete_research_slot",
        "reset_research",
        "research_completed",
        "rebuild_or_transfer_a",
        "rebuild_or_transfer_b",
        "research_console",
        "fresh_init",
        "reload_technologies",
    ] {
        assert!(site(name).2.technologies, "{name}");
    }
    for name in [
        "recalculate_modifiers",
        "set_design_team",
        "set_boosted_by_xp",
    ] {
        assert!(!site(name).2.technologies, "{name}");
    }
    assert_eq!(Receiver::SharedTechnology.own(), Receiver::Technology);
    assert_eq!(
        Receiver::SharedTechnologyStatus.own(),
        Receiver::TechnologyStatus
    );
    assert_eq!(Receiver::SharedResearchSlot.own(), Receiver::ResearchSlot);
    assert_eq!(
        Receiver::CompletedTechnology.own(),
        Receiver::TechnologyStatus
    );
    for name in [
        "set_level",
        "reset_research",
        "research_completed",
        "rebuild_or_transfer_b",
        "set_technology",
        "fresh_init",
    ] {
        assert!(
            matches!(
                site(name).1,
                Receiver::SharedTechnology
                    | Receiver::SharedTechnologyStatus
                    | Receiver::SharedResearchSlot
                    | Receiver::CompletedTechnology
            ),
            "{name}"
        );
    }
}

#[test]
fn appearance_and_counter_sites_reach_their_table() {
    let names = |range: std::ops::Range<usize>| -> Vec<&'static str> {
        MUTATIONS[range]
            .iter()
            .map(|(site, _, _)| site.name)
            .collect()
    };
    assert_eq!(
        names(APPEARANCE_SITES),
        [
            "set_cosmetic_tag",
            "copy_country_colors",
            "resolve_graphical_culture",
            "resolve_graphical_culture_2d",
            "rebuild_localized_name",
            "rebuild_subjects",
        ]
    );
    assert_eq!(
        names(COUNTER_SITES),
        [
            "add_army",
            "remove_army",
            "insert_unit_leader",
            "remove_unit_leader",
            "refresh_neighbors",
            "reload_mio_policies",
        ]
    );
    assert_eq!(COUNTER_SITES.end, FACTION_SHARING_BASE);
    assert_eq!(FACTION_SHARING_BASE + 1, WEATHER_WRITERS.start);
    for (index, (site, receiver, tables)) in MUTATIONS.iter().enumerate().skip(COUNTER_SITES.end) {
        let sent = matches!(index, ADD_SENT_EXPEDITIONARY | REMOVE_SENT_EXPEDITIONARY);
        assert!(!tables.appearance, "{}", site.name);
        assert_eq!(tables.counters, sent, "{}", site.name);
        if sent {
            assert_eq!(*tables, Tables::COUNTERS, "{}", site.name);
            assert_eq!(*receiver, Receiver::Country, "{}", site.name);
        }
    }
    for (site, _, tables) in &MUTATIONS[APPEARANCE_SITES] {
        assert_eq!(*tables, Tables::APPEARANCE, "{}", site.name);
    }
    for (site, _, tables) in &MUTATIONS[COUNTER_SITES] {
        assert_eq!(*tables, Tables::COUNTERS, "{}", site.name);
    }
    for (site, _, tables) in &MUTATIONS[..APPEARANCE_SITES.start] {
        assert!(!tables.appearance, "{}", site.name);
    }
    assert_eq!(MUTATIONS[APPEARANCE_SITES.start + 5].1, Receiver::Subjects);
    assert!(Receiver::Subjects.renames());
    assert!(!Receiver::StateOwner.renames());
    const { assert!(Tables::ALL.appearance && Tables::ALL.counters) };
    const { assert!(!Tables::ALL.intel_networks) };
}

#[test]
fn links_name_the_native_id_of_their_object() {
    let bytes = world();
    let memory = SliceMemory {
        base: START,
        bytes: &bytes,
    };
    assert_eq!(of_link(&memory, BASE, 7), Some(7));
    assert_eq!(of_link(&memory, BASE, 30), Some(9));
    assert_eq!(of_link(&memory, BASE, 31), None);
    assert_eq!(of_link(&memory, BASE, 0), None);
    assert_eq!(of_link(&memory, BASE, 400), None);
    assert_eq!(of_link(&memory, BASE + 8, 7), None, "no game state");
}

#[test]
fn receivers_name_their_countries() {
    let bytes = world();
    let cases = [
        (Receiver::Country, COUNTRY[0], 0, vec![7]),
        (Receiver::Interception, COUNTRY[0], 1, vec![7]),
        (Receiver::RelationStatus, START + 0x7000, 0, vec![7]),
        (Receiver::WarRelation, START + 0x7400, 0, vec![7, 9]),
        (Receiver::CountryCharacters, START + 0x7800, 0, vec![9]),
        (Receiver::PoliticalStatus, START + 0x7a00, 0, vec![7]),
        (Receiver::Party, START + 0x7c00, 0, vec![9]),
        (Receiver::Character, START + 0x7e00, 0, vec![11]),
        (Receiver::Scientist, START + 0x8000, 0, vec![11]),
        (Receiver::TechnologyStatus, STATUS, 0, vec![9]),
        (Receiver::ResearchSlot, START + 0x8400, 0, vec![9]),
        (Receiver::Technology, START + 0x8600, 0, vec![9]),
        (Receiver::ProgramStatus, START + 0x8800, 0, vec![7]),
        (Receiver::FocusProgress, START + 0x8a00, 0, vec![11]),
        (Receiver::Project, START + 0x8c00, 0, vec![9]),
        (Receiver::License, START + 0x8e00, 0, vec![9]),
    ];
    for (receiver, first, second, ids) in cases {
        assert_eq!(
            named(&bytes, receiver, first, second),
            (true, ids),
            "{receiver:?}"
        );
    }
    assert_eq!(named(&bytes, Receiver::Interception, 0, 2), (true, vec![]));
    assert_eq!(
        named(&bytes, Receiver::Every, COUNTRY[0], 0),
        (false, vec![])
    );
    for receiver in [
        Receiver::Country,
        Receiver::RelationStatus,
        Receiver::TechnologyStatus,
        Receiver::Technology,
        Receiver::Character,
        Receiver::FocusProgress,
        Receiver::Project,
        Receiver::License,
    ] {
        assert!(!named(&bytes, receiver, 0, 1).0, "{receiver:?}");
    }
    assert!(!named(&bytes, Receiver::TechnologyStatus, START + 0xf000, 0).0);
}

#[test]
fn diplomacy_state_and_tag_receivers_name_their_countries() {
    let mut bytes = world();
    let state = START + 0xb000;
    let tag = START + 0xb800;
    assert_eq!(
        named(&bytes, Receiver::DiplomacyStatus, DIPLOMACY[0], 0),
        (true, vec![7])
    );
    assert_eq!(
        named(&bytes, Receiver::Subjects, DIPLOMACY[0], 0),
        (true, vec![7])
    );
    put_u32(&mut bytes, DIPLOMACY[0] + engine_native_parts::cdiplomacy_status_layout::OVERLORD, 30);
    assert_eq!(
        named(&bytes, Receiver::Subjects, DIPLOMACY[0], 0),
        (true, vec![7, 9])
    );
    put_u32(&mut bytes, DIPLOMACY[0] + engine_native_parts::cdiplomacy_status_layout::OVERLORD, 31);
    assert_eq!(
        named(&bytes, Receiver::Subjects, DIPLOMACY[0], 0),
        (false, vec![7])
    );
    put_u32(
        &mut bytes,
        DIPLOMACY[0] + engine_native_parts::cdiplomacy_status_layout::OVERLORD,
        u32::MAX,
    );
    assert_eq!(
        named(&bytes, Receiver::Subjects, DIPLOMACY[0], 0),
        (true, vec![7])
    );
    put_u32(&mut bytes, state + cstate::OWNER, 11);
    put_u32(&mut bytes, state + cstate::CONTROLLER, 30);
    let edx = 0xdead_0000_0000_0007_usize;
    assert_eq!(
        named(&bytes, Receiver::StateOwner, state, edx),
        (true, vec![11, 7])
    );
    assert_eq!(
        named(&bytes, Receiver::StateController, state, edx),
        (true, vec![9, 7])
    );
    put_u32(&mut bytes, state + cstate::OWNER, 0);
    assert_eq!(
        named(&bytes, Receiver::StateOwner, state, 0),
        (true, vec![])
    );
    assert_eq!(
        named(&bytes, Receiver::StateOwner, state, 31),
        (false, vec![])
    );
    assert!(!named(&bytes, Receiver::StateOwner, 0, 7).0);
    put_u32(&mut bytes, tag, 30);
    assert_eq!(
        named(&bytes, Receiver::TagPointer, state, tag),
        (true, vec![9])
    );
    put_u32(&mut bytes, tag, 0);
    assert_eq!(
        named(&bytes, Receiver::TagPointer, state, tag),
        (true, vec![])
    );
    assert!(!named(&bytes, Receiver::TagPointer, state, 0).0);
    assert!(!named(&bytes, Receiver::DiplomacyStatus, 0, 0).0);
}

#[test]
fn a_war_endpoint_that_does_not_link_names_every_country() {
    let mut bytes = world();
    put_u32(&mut bytes, START + 0x7400 + crelation::SECOND_COUNTRY, 31);
    let (named, ids) = named(&bytes, Receiver::WarRelation, START + 0x7400, 0);
    assert!(!named);
    assert_eq!(ids, vec![7]);
}

#[test]
fn shared_levels_name_every_country_sharing_research() {
    let mut bytes = world();
    let sorted = |(named, mut ids): (bool, Vec<u32>)| {
        ids.sort_unstable();
        ids.dedup();
        (named, ids)
    };
    put_array(&mut bytes, STATUS + SHARING_ROSTER, 0, 0);
    assert_eq!(
        named(&bytes, Receiver::SharedTechnologyStatus, STATUS, 0),
        (true, vec![9])
    );
    put_array(&mut bytes, STATUS + SHARING_ROSTER, START + 0x9000, 1);
    put_u64(&mut bytes, START + 0x9000, (START + 0x9100) as u64);
    put_u64(
        &mut bytes,
        START + 0x9100,
        (BASE + ctechnology_sharing_group::VTABLE_RVA.0) as u64,
    );
    put_array(
        &mut bytes,
        START + 0x9100 + GROUP_MEMBERS,
        START + 0x9200,
        2,
    );
    put_u32(&mut bytes, START + 0x9200, 7);
    put_u32(&mut bytes, START + 0x9204, 11);
    assert_eq!(
        sorted(named(&bytes, Receiver::SharedTechnologyStatus, STATUS, 0)),
        (true, vec![7, 9, 11])
    );
    assert_eq!(
        sorted(named(&bytes, Receiver::SharedTechnology, START + 0x8600, 0)),
        (true, vec![7, 9, 11])
    );
    assert_eq!(
        sorted(named(
            &bytes,
            Receiver::SharedResearchSlot,
            START + 0x8400,
            0
        )),
        (true, vec![7, 9, 11])
    );
    assert_eq!(
        named(&bytes, Receiver::ResearchSlot, START + 0x8400, 0),
        (true, vec![9])
    );
    put_array(&mut bytes, START + 0x9100 + GROUP_MEMBERS, 0, 0);
    put_u64(
        &mut bytes,
        DIPLOMACY[1] + engine_native_parts::cdiplomacy_status_layout::FACTION,
        (START + 0x9400) as u64,
    );
    put_array(
        &mut bytes,
        START + 0x9400 + cfaction::MEMBERS,
        START + 0x9500,
        2,
    );
    put_u64(&mut bytes, START + 0x9500, COUNTRY[1] as u64);
    put_u64(&mut bytes, START + 0x9508, COUNTRY[2] as u64);
    assert_eq!(
        sorted(named(&bytes, Receiver::SharedTechnologyStatus, STATUS, 0)),
        (true, vec![9, 11])
    );
    put_array(
        &mut bytes,
        START + 0x9100 + GROUP_MEMBERS,
        START + 0x9200,
        1,
    );
    put_u32(&mut bytes, START + 0x9200, 31);
    assert!(!named(&bytes, Receiver::SharedTechnologyStatus, STATUS, 0).0);
    put_u32(&mut bytes, START + 0x9200, 7);
    put_u64(&mut bytes, START + 0x9100, 0x1234);
    assert!(!named(&bytes, Receiver::SharedTechnologyStatus, STATUS, 0).0);
}

#[test]
fn the_slot_roster_header_reads_from_the_status() {
    let mut bytes = world();
    put_array(&mut bytes, STATUS + SLOT_ROSTER, START + 0xa000, 3);
    let memory = SliceMemory {
        base: START,
        bytes: &bytes,
    };
    let header = slot_roster(&memory, STATUS).unwrap();
    assert_eq!(&header[..8], &((START + 0xa000) as u64).to_le_bytes());
    assert_eq!(&header[12..], &3_u32.to_le_bytes());
    assert_eq!(slot_roster(&memory, START + 0xfff8), None);
}

#[test]
fn the_leader_roster_counts_read_both_rosters() {
    let mut bytes = world();
    let characters = START + 0xa000;
    put_array(&mut bytes, characters + 0x70, START + 0xa400, 3);
    put_array(&mut bytes, characters + 0x88, START + 0xa500, 5);
    let memory = SliceMemory {
        base: START,
        bytes: &bytes,
    };
    let counts = unit_leader_counts(&memory, characters).unwrap();
    assert_eq!(&counts[..4], &3_u32.to_le_bytes());
    assert_eq!(&counts[4..8], &5_u32.to_le_bytes());
    assert_eq!(&counts[8..], &[0; 8]);
    assert_eq!(unit_leader_counts(&memory, START + 0xfff8), None);
}

#[test]
fn a_faction_sharing_base_names_its_faction_members() {
    let mut bytes = world();
    let group = START + 0x9800;
    put_array(&mut bytes, group + GROUP_MEMBERS, START + 0x9900, 1);
    put_u32(&mut bytes, START + 0x9900, 7);
    put_u64(
        &mut bytes,
        group + FACTION_SHARING_FACTION,
        (START + 0x9a00) as u64,
    );
    put_array(
        &mut bytes,
        START + 0x9a00 + cfaction::MEMBERS,
        START + 0x9b00,
        2,
    );
    put_u64(&mut bytes, START + 0x9b00, COUNTRY[1] as u64);
    put_u64(&mut bytes, START + 0x9b08, COUNTRY[2] as u64);
    let (all, mut ids) = named(&bytes, Receiver::FactionSharing, group, 0);
    ids.sort_unstable();
    assert_eq!((all, ids), (true, vec![7, 9, 11]));
    put_u64(&mut bytes, group + FACTION_SHARING_FACTION, 0);
    assert!(!named(&bytes, Receiver::FactionSharing, group, 0).0);
    assert_eq!(
        MUTATIONS[FACTION_SHARING_BASE].0.name,
        "faction_sharing_base"
    );
    assert_eq!(MUTATIONS[FACTION_SHARING_BASE].1, Receiver::FactionSharing);
    assert_eq!(MUTATIONS[FACTION_SHARING_BASE].2, Tables::RESEARCH);
    let memory = SliceMemory {
        base: START,
        bytes: &bytes,
    };
    let before = faction_sharing_base(&memory, group).unwrap();
    put_u64(&mut bytes, group + 0x10, 300_000);
    let memory = SliceMemory {
        base: START,
        bytes: &bytes,
    };
    assert_ne!(faction_sharing_base(&memory, group), Some(before));
}

fn partner_world() -> (Vec<u8>, [usize; 4]) {
    let mut bytes = world();
    put_array(&mut bytes, STATUS + SHARING_ROSTER, START + 0x9000, 1);
    put_u64(&mut bytes, START + 0x9000, (START + 0x9100) as u64);
    put_u64(
        &mut bytes,
        START + 0x9100,
        (BASE + ctechnology_sharing_group::VTABLE_RVA.0) as u64,
    );
    put_array(
        &mut bytes,
        START + 0x9100 + GROUP_MEMBERS,
        START + 0x9200,
        2,
    );
    put_u32(&mut bytes, START + 0x9200, 7);
    put_u32(&mut bytes, START + 0x9204, 11);
    let [t0, t1, t2, t3] = [0xa800, 0xa100, 0xa300, 0xa500].map(|at| START + at);
    put_array(
        &mut bytes,
        STATUS + ctechnology_status::TECHNOLOGIES_DATA,
        START + 0xa000,
        3,
    );
    for (index, technology) in [t1, t2, t3].into_iter().enumerate() {
        put_u64(&mut bytes, START + 0xa000 + index * 8, technology as u64);
    }
    for technology in [t0, t1, t2, t3] {
        put_u64(&mut bytes, technology + ctechnology::STATUS, STATUS as u64);
    }
    put_array(&mut bytes, t3 + TECHNOLOGY_PREDECESSORS, START + 0xa700, 1);
    put_u64(&mut bytes, START + 0xa700, t2 as u64);
    put_array(&mut bytes, t2 + TECHNOLOGY_DEPENDENTS, START + 0xa710, 1);
    put_u64(&mut bytes, START + 0xa710, t3 as u64);
    put_array(&mut bytes, t0 + TECHNOLOGY_DEPENDENTS, START + 0xa720, 1);
    put_u64(&mut bytes, START + 0xa720, t2 as u64);
    for (country, at, database) in [(COUNTRY[0], 0xb000, 3), (COUNTRY[2], 0xc000, 1)] {
        let status = START + at;
        put_u64(
            &mut bytes,
            country + ccountry::TECHNOLOGY_STATUS,
            status as u64,
        );
        put_array(
            &mut bytes,
            status + ctechnology_status::RESEARCH_SLOTS_DATA,
            status + 0x100,
            1,
        );
        put_u64(&mut bytes, status + 0x100, (status + 0x200) as u64);
        put_u64(
            &mut bytes,
            status + 0x200 + cresearch_slot::TECHNOLOGY,
            (status + 0x300) as u64,
        );
        put_u64(
            &mut bytes,
            status + 0x300 + ctechnology::TEMPLATE,
            (status + 0x400) as u64,
        );
        put_u32(
            &mut bytes,
            status + 0x400 + ctechnology_template::DATABASE_INDEX,
            database,
        );
    }
    (bytes, [t0, t1, t2, t3])
}

fn sorted((named, mut ids): (bool, Vec<u32>)) -> (bool, Vec<u32>) {
    ids.sort_unstable();
    (named, ids)
}

#[test]
fn a_technology_write_marks_only_the_partners_it_can_change() {
    let (mut bytes, [t0, t1, t2, t3]) = partner_world();
    let shared =
        |bytes: &[u8], technology| sorted(named(bytes, Receiver::SharedTechnology, technology, 0));
    assert_eq!(shared(&bytes, t2), (true, vec![7, 9]));
    assert_eq!(shared(&bytes, t0), (true, vec![7, 9]));
    assert_eq!(shared(&bytes, t3), (true, vec![7, 9]));
    assert_eq!(shared(&bytes, t1), (true, vec![9, 11]));
    assert_eq!(
        sorted(named(&bytes, Receiver::CompletedTechnology, STATUS, t1)),
        (true, vec![9, 11])
    );
    assert_eq!(
        sorted(named(
            &bytes,
            Receiver::SharedResearchSlot,
            START + 0x8400,
            t2
        )),
        (true, vec![7, 9])
    );
    put_u64(
        &mut bytes,
        START + 0x8400 + cresearch_slot::TECHNOLOGY,
        t1 as u64,
    );
    assert_eq!(
        sorted(named(
            &bytes,
            Receiver::SharedResearchSlot,
            START + 0x8400,
            t2
        )),
        (true, vec![7, 9, 11])
    );
    put_u64(&mut bytes, COUNTRY[2] + ccountry::TECHNOLOGY_STATUS, 0);
    assert_eq!(shared(&bytes, t2), (true, vec![7, 9, 11]));
}

#[test]
fn an_unreadable_research_slot_marks_every_partner() {
    let (mut bytes, [_, t1, _, _]) = partner_world();
    let shared = |bytes: &[u8]| sorted(named(bytes, Receiver::SharedTechnology, t1, 0));
    assert_eq!(shared(&bytes), (true, vec![9, 11]));
    put_u64(&mut bytes, START + 0xc100, (START + 0x2_0000) as u64);
    assert_eq!(shared(&bytes), (true, vec![7, 9, 11]));
    put_u64(&mut bytes, START + 0xc100, (START + 0xc200) as u64);
    put_u64(&mut bytes, START + 0xc200 + cresearch_slot::TECHNOLOGY, 0);
    assert_eq!(shared(&bytes), (true, vec![9]));
}

#[test]
fn weather_writers_reach_only_the_weather_count() {
    for index in WEATHER_WRITERS {
        let (site, receiver, tables) = &MUTATIONS[index];
        assert!(site.name.starts_with("weather_"), "{}", site.name);
        assert_eq!(*receiver, Receiver::Every);
        assert_eq!(*tables, Tables::WEATHER);
    }
    assert_eq!(WEATHER_WRITERS.end, RESEARCH_PROGRESS);
    const { assert!(!Tables::ALL.weather) };
}

#[test]
fn a_write_marks_the_partners_reading_its_listeners() {
    let (mut bytes, [_, _, t2, _]) = partner_world();
    let t4 = START + 0xa900;
    put_u64(&mut bytes, t4 + ctechnology::STATUS, STATUS as u64);
    put_array(&mut bytes, t4 + ctechnology::LISTENERS, START + 0xa730, 1);
    put_u64(
        &mut bytes,
        START + 0xa730,
        (t2 + ctechnology::LISTENER) as u64,
    );
    put_u64(
        &mut bytes,
        t2 + ctechnology::LISTENER,
        (BASE + ctechnology::LISTENER_VTABLE_RVA.0) as u64,
    );
    let shared =
        |bytes: &[u8], technology| sorted(named(bytes, Receiver::SharedTechnology, technology, 0));
    assert_eq!(shared(&bytes, t4), (true, vec![7, 9]));
    assert_eq!(
        sorted(named(&bytes, Receiver::CompletedTechnology, STATUS, t4)),
        (true, vec![7, 9])
    );
    put_u64(&mut bytes, t2 + ctechnology::LISTENER, 0x1234);
    assert_eq!(shared(&bytes, t4), (true, vec![7, 9, 11]));
    put_u64(
        &mut bytes,
        t2 + ctechnology::LISTENER,
        (BASE + ctechnology::LISTENER_VTABLE_RVA.0) as u64,
    );
    put_u64(
        &mut bytes,
        t4 + ctechnology::PENDING_LISTENERS,
        (START + 0xa740) as u64,
    );
    assert_eq!(shared(&bytes, t4), (true, vec![7, 9, 11]));
}

#[test]
fn research_progress_marks_its_technology_country() {
    let (site, receiver, tables) = &MUTATIONS[RESEARCH_PROGRESS];
    assert_eq!(site.name, "research_progress");
    assert_eq!(*receiver, Receiver::Technology);
    assert_eq!(*tables, Tables::TECHNOLOGY);
    assert_eq!(
        MUTATIONS.last().unwrap().0.name,
        "remove_sent_expeditionary"
    );
}

#[test]
fn visibility_reaches_only_the_country_accumulator_rebuild_and_reset_tables() {
    const { assert!(Tables::ALL.visibility && Tables::VISIBILITY.visibility) };
    for tables in [
        Tables::STABILITY,
        Tables::RESEARCH,
        Tables::BOTH,
        Tables::TECHNOLOGY,
        Tables::FOCUS,
        Tables::INTEL_NETWORKS,
        Tables::PROJECTS,
        Tables::APPEARANCE,
        Tables::COUNTERS,
        Tables::WEATHER,
    ] {
        assert!(!tables.visibility);
        assert!(tables.with(Tables::VISIBILITY).visibility);
    }
    let writers: Vec<_> = MUTATIONS
        .iter()
        .filter(|(_, _, tables)| tables.visibility)
        .map(|(site, _, _)| site.name)
        .collect();
    assert_eq!(writers, ["recalculate_modifiers"]);
}

#[test]
fn manpower_setters_name_only_the_embedded_owner_and_keep_crew_sites_separate() {
    let mut bytes = world();
    let embedded = START + 0x9000;
    put_u32(
        &mut bytes,
        embedded + engine_native_parts::ccountry_manpower::COUNTRY,
        30,
    );
    assert_eq!(
        named(&bytes, Receiver::Manpower, embedded, 0),
        (true, vec![9])
    );
    for (site, receiver, tables) in &MUTATIONS[SET_RECRUITMENT_TARGET..ADD_EXILE_LEGITIMACY] {
        assert_eq!(*receiver, Receiver::Manpower, "{}", site.name);
        assert_eq!(*tables, Tables::STABILITY);
    }
    for (_, _, tables) in &MUTATIONS[CREW_WRITERS] {
        assert_eq!(*tables, Tables::NONE);
    }
}
