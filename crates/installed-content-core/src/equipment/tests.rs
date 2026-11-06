use super::*;

#[test]
fn designer_families_follow_recursive_archetypes_and_each_types_own_defaults() {
    let database = read(&[(
        "families.txt",
        "equipment_modules={m={add_equipment_type=cas}} equipments={root={is_archetype=yes type=armor} middle={is_archetype=yes archetype=root type=infantry} leaf={archetype=middle type=convoy default_modules={s=m}}}",
    )]);
    assert!(database.lost.is_none(), "{:?}", database.lost);
    assert_eq!(
        database.designer_families(database.named("leaf").unwrap()),
        Ok([true, true, true])
    );
    assert_eq!(
        database.designer_families(database.named("root").unwrap()),
        Ok([true, false, false])
    );
}

#[test]
fn designer_families_refuse_an_archetype_loop_the_parent_walk_misses() {
    let database = read(&[(
        "loop.txt",
        "equipments={c={is_archetype=yes} a={is_archetype=yes} b={is_archetype=yes archetype=a} a={is_archetype=yes archetype=b parent=c}}",
    )]);
    assert!(database.lost.is_none(), "{:?}", database.lost);
    let error = database
        .designer_families(database.named("a").unwrap())
        .unwrap_err();
    assert!(error.contains("equipment a reaches archetype"), "{error}");
}

#[test]
fn module_slot_rules_follow_skipped_duplicates_aliases_inheritance_and_generated_removal() {
    let database = read(&[(
        "slots.txt",
        "equipments={root={is_archetype=yes module_slots={main={required=yes allowed_module_categories={a b a}} alias=main}} child={archetype=root module_slots=inherit module_slots={main={required=no allowed_module_categories={ignored}}}}} duplicate_archetypes={no_slots={archetype=root module_slots=none}}",
    )]);
    assert!(database.lost.is_none(), "{:?}", database.lost);
    let root = database.named("root").unwrap();
    let child = database.named("child").unwrap();
    assert_eq!(root.module_slot_rules.len(), root.module_slots.len());
    assert!(root.module_slot_rules[0].required);
    assert_eq!(
        root.module_slot_rules[0].categories,
        [b"a".to_vec(), b"b".to_vec(), b"a".to_vec()]
    );
    assert_eq!(root.module_slot_rules[0], root.module_slot_rules[1]);
    assert_eq!(child.module_slot_rules, root.module_slot_rules);
    assert!(
        database
            .named("no_slots")
            .unwrap()
            .module_slot_rules
            .is_empty()
    );
}

#[test]
fn category_names_invert_the_table_and_refuse_unknown_bits() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("equipment category checks require BATTLE_PLAN_GAME_DIR");
    for &(name, bit) in engine_executable::executable_tables::get()
        .auxiliary
        .as_ref()
        .unwrap()
        .category_names
        .as_ref()
        .unwrap()
    {
        assert_eq!(category_mask(name), Some(bit));
        if bit != 0 {
            assert_eq!(category_names(bit).unwrap(), [name]);
        }
    }
    let mask = category_mask("carrier").unwrap() | category_mask("support_ship").unwrap();
    assert_eq!(category_names(mask).unwrap(), ["carrier", "support_ship"]);
    assert_eq!(category_names(mask | 1 << 63), Err(1 << 63));
    assert!(category_names(0).unwrap().is_empty());
}

#[test]
fn module_country_conditions_keep_final_limit_and_reset_on_redefinition() {
    let database = read(&[
        (
            "a.txt",
            "equipment_modules={mod_module={allowed={always=yes}} limit={always=no} other_module={} limit={always=yes}}",
        ),
        (
            "b.txt",
            "equipment_modules={MOD_MODULE={allowed={always=no}}}",
        ),
    ]);
    assert!(database.lost.is_none(), "{:?}", database.lost);
    let redefined = database.module_conditions("mod_module").unwrap();
    assert!(redefined.limit.is_none());
    assert_eq!(redefined.allowed[0].value.scalar_str(), Some("no"));
    let retained = database.module_conditions("other_module").unwrap();
    assert_eq!(
        retained.limit.as_ref().unwrap()[0].value.scalar_str(),
        Some("yes")
    );
    assert!(retained.allowed.is_empty());
}

/// `(path, text)` files read in order through the file loop, then
/// finalized.
fn read(files: &[(&str, &str)]) -> Database {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("equipment reader checks require BATTLE_PLAN_GAME_DIR");
    let mut database = Database::default();
    for (path, text) in files {
        database.read_file(text.as_bytes(), path, &KeyNames::default());
    }
    if database.lost.is_none() {
        database.finalize();
    }
    database
}

fn named<'a>(database: &'a Database, name: &str) -> &'a Equipment {
    &database.equipment[database.by_name[name.to_ascii_lowercase().as_bytes()]]
}

#[test]
fn supply_trucks_follow_boolean_redefinition_and_copy_rules() {
    let database = read(&[
        ("a.txt", "equipments = { trucks = { is_archetype = yes supply_truck = yes supply_truck = maybe } }
            duplicate_archetypes = { copies = { archetype = trucks } }"),
        ("b.txt", "equipments = { TRUCKS = { is_archetype = yes } ordinary = { supply_truck = yes supply_truck = no } }"),
    ]);
    assert!(database.lost.is_none(), "{:?}", database.lost);
    assert!(!named(&database, "trucks").supply_truck);
    assert!(named(&database, "copies").supply_truck);
    assert!(!named(&database, "ordinary").supply_truck);
}

#[test]
fn unreproduced_stat_fields_are_kept_silently_and_found_through_parents_and_archetypes() {
    let database = read(&[(
        "chassis.txt",
        "equipments = { root = { is_archetype = yes scaled_by_reliability = { max_strength = 0.5 } }
            leaf = { archetype = root module_multiply_stats_multiplier = { soft_attack = 0.1 }
                MODULE_MULTIPLY_STATS_MULTIPLIER = { soft_attack = 0.1 } }
            child = { parent = leaf }
            plain = { soft_attack = 1 } }",
    )]);
    assert!(database.lost.is_none(), "{:?}", database.lost);
    assert!(database.misreads.is_empty(), "{:?}", database.misreads);
    assert_eq!(
        named(&database, "leaf").unreproduced_stat_fields,
        ["module_multiply_stats_multiplier"]
    );
    let field = |name: &str| {
        database
            .unreproduced_stat_field(database.get(name).unwrap())
            .map(|(field, owner)| (field, String::from_utf8_lossy(owner).into_owned()))
    };
    assert_eq!(
        field("root"),
        Some(("scaled_by_reliability", "root".to_owned()))
    );
    assert_eq!(
        field("leaf"),
        Some(("module_multiply_stats_multiplier", "leaf".to_owned()))
    );
    assert_eq!(
        field("child"),
        Some(("module_multiply_stats_multiplier", "leaf".to_owned()))
    );
    assert_eq!(field("plain"), None);
}

fn stat(name: &str) -> usize {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("equipment stat checks require BATTLE_PLAN_GAME_DIR");
    stat_index(name.as_bytes()).unwrap()
}

/// Nested readers retain their own consumption and variable definitions.
/// An upgrade variable remains visible to later equipment, while reported
/// keys skip their blocks and register nothing at any depth.
#[test]
fn every_top_level_reader_consumes_its_own_block() {
    let database = read(&[(
        "a.txt",
        "upgrades = { up = { @u = 0.5 max_level = 3 cost = land lg_attack = 1 odd = { @no = 9 } } }
         equipment_modules = { limit = { always = yes } m = { add_equipment_type = convoy add_stats = { lg_attack = 1 } } }
         search_filters = { name = f values = { a b } }
         unknown = { @skipped = 7 }
         equipments = { c = { type = armor lg_attack = @u hg_attack = @skipped
            torpedo_attack = @no default_modules = { s = M } odd = { @deep = 3 } sub_attack = @deep } }",
    )]);
    assert!(database.lost.is_none(), "{:?}", database.lost);
    let c = named(&database, "c");
    assert_eq!(c.stats[stat("lg_attack")], 50_000);
    // Skipped blocks define nothing: the references stay text, which
    // does not scan.
    for skipped in ["hg_attack", "torpedo_attack", "sub_attack"] {
        assert_eq!(c.stats[stat(skipped)], 0, "{skipped}");
    }
    // Finalization includes the module's added equipment categories, so
    // armor equipment with that default module is convoy too.
    assert_eq!(c.module_category, 1);
    assert_eq!(database.category(c), 5);
}

/// Direct files load from `upgrades`, then `modules`, then root.
/// Upgrade-file archetypes register before root files; deeper directories
/// are not read.
#[cfg(feature = "test-support")]
#[test]
fn the_loader_reads_its_three_directories_in_order() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must select the supported installation for runtime table tests");
    use std::fs;
    let root = std::env::temp_dir().join(format!(
        "hoi4_installed_content_equipment_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    for (path, text) in [
        (
            "common/units/equipment/a.txt",
            "equipments = { child = { archetype = base type = convoy } }",
        ),
        (
            "common/units/equipment/upgrades/z.txt",
            "equipments = { base = { is_archetype = yes lg_attack = 2 } }",
        ),
        (
            "common/units/equipment/modules/m.txt",
            "equipment_modules = { gun = { add_equipment_type = convoy } }",
        ),
        (
            "common/units/equipment/nested/deep.txt",
            "equipments = { deep = { type = convoy } }",
        ),
    ] {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    assert_eq!(
        files(&layers),
        [
            "common/units/equipment/upgrades/z.txt",
            "common/units/equipment/modules/m.txt",
            "common/units/equipment/a.txt",
        ]
    );
    let database = Database::load(&layers).unwrap();
    assert!(database.misreads.is_empty(), "{:?}", database.misreads);
    assert_eq!(named(&database, "child").stats[stat("lg_attack")], 200_000);
    assert!(!database.by_name.contains_key(b"deep".as_slice()));
    fs::remove_dir_all(&root).unwrap();
}

/// Duplicate every type in the archetype's group under the definition name
/// plus its suffix after the common prefix. Clone the archetype first and
/// remap links to clones. `type` replaces categories; `for_each` applies
/// to all copies with wrapping, truncating multiplication. Optional
/// archetype-only copies, duplicate-name skips, and later references remain.
#[test]
fn duplicated_archetypes_generate_their_clones() {
    let database = read(&[
        (
            "a.txt",
            "equipments = { hull = { is_archetype = yes type = screen_ship lg_attack = 2 hardness = 0.3 }
               hull_1 = { archetype = hull parent = hull }
               hull_2 = { archetype = hull parent = hull_1 lg_attack = 3 }
               raider_2 = { } }",
        ),
        (
            "b.txt",
            "duplicate_archetypes = {
               raider = { archetype = HULL type = { convoy }
                 for_each = { lg_attack = { multiply = 1.5 } hardness = { set = 0.5 }
                   variant_name = { find_and_replace = { \"hull\" \"raider\" } } } }
               solo = { archetype = hull only_duplicate_archetype = yes }
               nothing = { archetype = missing } }
             equipments = { later = { archetype = raider parent = raider_1 } }",
        ),
    ]);
    assert!(database.lost.is_none(), "{:?}", database.lost);
    let raider = named(&database, "raider");
    assert!(raider.duplicate && raider.is_archetype);
    assert_eq!(
        raider.archetype,
        Some(database.by_name[b"raider".as_slice()])
    );
    assert_eq!(raider.category, 1);
    assert_eq!(raider.stats[stat("lg_attack")], 300_000);
    assert_eq!(raider.stats[stat("hardness")], 50_000);
    let raider_1 = named(&database, "raider_1");
    assert_eq!(
        raider_1.archetype,
        Some(database.by_name[b"raider".as_slice()])
    );
    assert_eq!(raider_1.parent, database.by_name[b"raider".as_slice()]);
    // hull_1 copied hull's 2 through `archetype`, times 1.5.
    assert_eq!(raider_1.stats[stat("lg_attack")], 300_000);
    // `raider_2` was defined already: skipped and logged.
    assert!(!named(&database, "raider_2").duplicate);
    assert!(
        database.misreads.iter().any(|note| note
            .contains("asked to duplicate hull_2 into raider_2, which is already defined")),
        "{:?}",
        database.misreads
    );
    assert!(
        database
            .misreads
            .iter()
            .any(|note| note
                .contains("asked to duplicate missing, which is not a known archetype")),
        "{:?}",
        database.misreads
    );
    let solo = named(&database, "solo");
    assert!(solo.is_archetype && solo.category == 0);
    assert!(!database.by_name.contains_key(b"solo_1".as_slice()));
    let later = named(&database, "later");
    assert_eq!(later.parent, database.by_name[b"raider_1".as_slice()]);
    assert_eq!(later.stats[stat("lg_attack")], 300_000);
    assert_eq!(database.category(later) & 1, 1);
}

/// Read one statement, report and skip any second statement and its block,
/// then resume after the entry's closing brace.
#[test]
fn a_for_each_entry_reads_one_statement_and_skips_the_rest() {
    let database = read(&[(
        "a.txt",
        "equipments = { hull = { is_archetype = yes lg_attack = 2 hardness = 0.3 } }
         duplicate_archetypes = { raider = { archetype = hull
           for_each = { lg_attack = { set = 1 add = 2 } HARDNESS = { Multiply = 2 } } } }",
    )]);
    assert!(database.lost.is_none(), "{:?}", database.lost);
    assert!(
        database
            .misreads
            .iter()
            .any(|note| note.starts_with("a.txt line 3:") && note.contains("more than one entry")),
        "{:?}",
        database.misreads
    );
    let raider = named(&database, "raider");
    assert_eq!(raider.stats[stat("lg_attack")], 100_000);
    // Keys and keywords fold: the later entry is read.
    assert_eq!(raider.stats[stat("hardness")], 60_000);
}

/// Unknown category tokens use unsigned decimal conversion. Repeated
/// modules reset and read from their defaults.
#[test]
fn category_masks_fall_back_to_the_unsigned_decimal() {
    assert_eq!(unsigned_decimal(b"12x"), 12);
    assert_eq!(unsigned_decimal(b"x"), 0);
    assert_eq!(unsigned_decimal(b"-1"), u64::MAX);
    assert_eq!(unsigned_decimal(b"99999999999999999999"), u64::MAX);
    let database = read(&[(
        "m.txt",
        "equipment_modules = { a = { add_equipment_type = { 1 armor } } b = { add_equipment_type = convoy } B = { } }
         equipments = { x = { default_modules = { s = A t = b } } }",
    )]);
    let x = named(&database, "x");
    assert_eq!(x.module_category, 1 | 4);
    // Unsigned decimal conversion skips whitespace before digits.
    let database = read(&[(
        "m.txt",
        "equipment_modules = { m = { add_equipment_type = \" 1\" } }
         equipments = { y = { default_modules = { s = m } } }",
    )]);
    assert_eq!(unsigned_decimal(b" \t1"), 1);
    assert_eq!(named(&database, "y").module_category, 1);
}

/// Compare common prefixes byte by byte and retain the remaining name
/// bytes, including a cut inside a UTF-8 character.
#[test]
fn duplication_cuts_names_as_bytes() {
    let database = read(&[(
        "a.txt",
        "equipments = { é = { is_archetype = yes lg_attack = 1 } ê = { archetype = é } }
         duplicate_archetypes = { d = { archetype = é } }",
    )]);
    assert!(database.lost.is_none(), "{:?}", database.lost);
    // é is C3 A9 and ê C3 AA: one common byte with the archetype.
    assert!(named(&database, "d").duplicate);
    let cut = database.by_name[&[b'd', 0xAA][..]];
    assert_eq!(database.equipment[cut].name, [b'd', 0xAA]);
    assert!(database.equipment[cut].duplicate);
}

/// Slots and modules use token identity. Rejected slot names share token 0
/// and retain the last default module; rejected module names select the
/// first registered token-0 module. Repeated slot tokens skip their block
/// and register nothing.
#[test]
fn slots_and_modules_are_tokens() {
    let database = read(&[(
        "a.txt",
        "equipment_modules = { 1st = { add_equipment_type = armor } 2nd = { add_equipment_type = convoy }
            a = { add_equipment_type = infantry } b = { add_equipment_type = cas } }
         equipments = {
           x = { default_modules = { 1 = a 2 = b } }
           y = { default_modules = { s = 99 } }
           z = { module_slots = { 1 = { } 2 = { @v = 0.5 } } lg_attack = @v } }",
    )]);
    assert!(database.lost.is_none(), "{:?}", database.lost);
    let x = named(&database, "x");
    assert_eq!(x.default_modules.len(), 1);
    assert_eq!(x.module_category, 1 << 14);
    // `99`, `1st` and `2nd` are all token 0; the first is `1st`.
    assert_eq!(named(&database, "y").module_category, 1 << 2);
    assert_eq!(named(&database, "z").stats[stat("lg_attack")], 0);
    assert_eq!(database.module_category("99"), Some(1 << 2));
    assert_eq!(database.module_category("A"), Some(1 << 5));
    assert_eq!(database.module_category("missing"), None);
}

/// Duplicate definitions group keys by token and resolve archetypes by
/// token text. Token-0 text is unavailable from content and stays refused.
#[test]
fn a_duplicate_of_token_zero_is_refused_by_name() {
    let base = "equipments = { hull = { is_archetype = yes } }";
    for (definitions, cause) in [
        (
            "1a = { archetype = hull } 2b = { }",
            "has a name that is empty",
        ),
        ("d = { archetype = \"9\" }", "names no archetype"),
        ("d = { type = convoy }", "names no archetype"),
    ] {
        let database = read(&[(
            "a.txt",
            &format!("{base} duplicate_archetypes = {{ {definitions} }}"),
        )]);
        let lost = database.lost.expect(definitions);
        assert!(lost.starts_with("a.txt: duplicate archetype "), "{lost}");
        assert!(lost.contains(cause), "{lost}");
    }
}

/// Pairs require two strings. Extra strings are reported and skipped
/// through the pair's closing brace; missing strings are reported.
/// Both recover in step so the following stat applies.
#[test]
fn find_and_replace_pairs_read_two_strings_and_recover_in_step() {
    let pair = |pair: &str| {
        read(&[(
            "a.txt",
            &format!(
                "equipments = {{ hull = {{ is_archetype = yes lg_attack = 2 }} }}
                 duplicate_archetypes = {{ d = {{ archetype = hull
                   for_each = {{ variant_name = {{ find_and_replace = {pair} }}
                     lg_attack = {{ set = 5 }} }} }} }}"
            ),
        )])
    };
    for form in [
        "{ a b }",
        "{ a b extra more }",
        "{ a b { x } extra }",
        "{ a }",
        "{ @v = 1 }",
    ] {
        let database = pair(form);
        assert!(database.lost.is_none(), "{form}: {:?}", database.lost);
        assert_eq!(
            named(&database, "d").stats[stat("lg_attack")],
            500_000,
            "{form}"
        );
    }
}

/// Parent and archetype string reads end at NUL. Line-feed text uses
/// the shared unreadable-string name before equipment lookup.
#[test]
fn links_read_their_names_as_read_string_stores_them() {
    let database = read(&[(
        "a.txt",
        "equipments = { base = { is_archetype = yes lg_attack = 3 }
           \"Unreadable String\" = { is_archetype = yes lg_attack = 7 }
           cut = { archetype = \"base\0junk\" }
           lined = { archetype = \"two\nlines\" } }",
    )]);
    assert!(database.lost.is_none(), "{:?}", database.lost);
    assert_eq!(named(&database, "cut").stats[stat("lg_attack")], 300_000);
    assert_eq!(named(&database, "lined").stats[stat("lg_attack")], 700_000);
}

/// One equipment file loaded through the real load path.
#[cfg(feature = "test-support")]
fn load_one(text: &str) -> Database {
    let (database, _) = fixture::load(&[("common/units/equipment/a.txt", text)]);
    assert!(database.lost.is_none(), "{:?}", database.lost);
    database
}

/// Copies from later files retain every source-group level, remapping
/// parent and archetype links. Family points to the copied source,
/// `type` replaces categories, and `module_slots = none` drops defaults.
/// Variant-name pairs are consumed; archetype-only copies omit levels,
/// and generated copies are not duplicated again.
#[cfg(feature = "test-support")]
#[test]
fn duplicate_archetypes_copy_every_level_of_their_source() {
    let (database, _) = fixture::load(&[
        (
            "common/units/equipment/modules/parts.txt",
            "equipment_modules = { spinner = { add_equipment_type = rocket } }",
        ),
        (
            "common/units/equipment/a_frames.txt",
            "equipments = {
                kite = { is_archetype = yes type = interceptor }
                kite_mk1 = { archetype = kite }
                kite_mk2 = { archetype = kite parent = kite_mk1 }
                kite_mk3 = { archetype = kite parent = kite_mk2 }
                cart = { is_archetype = yes type = motorized default_modules = { roof = spinner } }
                cart_mk1 = { archetype = cart }
                cart_mk2 = { archetype = cart parent = cart_mk1 }
            }",
        ),
        (
            "common/units/equipment/b_copies.txt",
            "duplicate_archetypes = {
                spotter = { archetype = kite type = scout_plane
                    variant_name = { spotter_mk2 = Lookout } }
                patrol = { archetype = kite type = maritime_patrol_plane }
                flamecart = { archetype = cart type = { motorized flame } module_slots = none }
                lonely = { archetype = cart only_duplicate_archetype = yes }
            }",
        ),
    ]);
    assert!(database.lost.is_none(), "{:?}", database.lost);
    // Every copy, in the order they are made, by definition: `patrol`
    // copies `kite`'s group only, not `spotter`'s copies of it.
    let copies: [(&str, &[&str]); 4] = [
        ("spotter", &["spotter_mk1", "spotter_mk2", "spotter_mk3"]),
        ("patrol", &["patrol_mk1", "patrol_mk2", "patrol_mk3"]),
        ("flamecart", &["flamecart_mk1", "flamecart_mk2"]),
        ("lonely", &[]),
    ];
    let made: Vec<&[u8]> = database
        .equipment
        .iter()
        .filter(|kind| kind.duplicate)
        .map(|kind| kind.name.as_slice())
        .collect();
    let expected: Vec<&[u8]> = copies
        .iter()
        .flat_map(|(archetype, levels)| std::iter::once(archetype).chain(levels.iter()))
        .map(|name| name.as_bytes())
        .collect();
    assert_eq!(made, expected);
    for (archetype, levels) in copies {
        for name in std::iter::once(archetype).chain(levels.iter().copied()) {
            assert_eq!(
                database.named(name).unwrap().archetype,
                database.get(archetype),
                "{name}"
            );
        }
    }
    let at = |name: &str| database.get(name).unwrap();
    for (copy, parent, family) in [
        ("spotter_mk1", "spotter_mk1", "kite_mk1"),
        ("spotter_mk2", "spotter_mk1", "kite_mk2"),
        ("spotter_mk3", "spotter_mk2", "kite_mk3"),
    ] {
        let copy = database.named(copy).unwrap();
        assert_eq!(copy.parent, at(parent));
        assert_eq!(copy.family, at(family));
    }
    let duplicate = database.named("spotter").unwrap();
    assert!(duplicate.is_archetype);
    assert_eq!(duplicate.family, at("kite"));
    let category = |name: &str| database.category(database.named(name).unwrap());
    let mask = |names: &[&str]| {
        names
            .iter()
            .map(|name| category_mask(name).unwrap())
            .fold(0, |mask, bit| mask | bit)
    };
    // The duplicate's type replaces the source's for every copy.
    assert_eq!(category("spotter_mk2"), mask(&["scout_plane"]));
    // module_slots = none drops the source's default modules.
    assert_eq!(category("cart"), mask(&["motorized", "rocket"]));
    assert_eq!(category("flamecart"), mask(&["motorized", "flame"]));
}

/// Inherited slots report and skip repeated block values; absent slots
/// read as new. The loaded source determines this choice, preserving
/// variable-definition consumption when the two paths differ.
#[test]
fn a_slot_block_after_inherit_is_skipped_when_the_source_has_it() {
    let database = read(&[(
        "a.txt",
        "equipments = {
            planes = { is_archetype = yes module_slots = { gun = { } } }
            planes_0 = { archetype = planes module_slots = inherit
                module_slots = { gun = { @v = 3 required = yes } bomb = { @w = 4 required = yes } }
                lg_attack = @v air_attack = @w }
        }",
    )]);
    assert!(database.lost.is_none(), "{:?}", database.lost);
    let plane = named(&database, "planes_0");
    let slots: Vec<_> = plane.module_slots.iter().map(Vec::as_slice).collect();
    assert_eq!(slots, [b"gun".as_slice(), b"bomb"]);
    // The skipped block defined no `@v`; the read one defined `@w`.
    assert_eq!(plane.stats[stat("lg_attack")], 0);
    assert_eq!(plane.stats[stat("air_attack")], 400_000);
    assert!(
        database
            .misreads
            .iter()
            .any(|note| note.contains("module slot gun is defined again")),
        "{:?}",
        database.misreads
    );
}

/// Model inputs read in order; tech-level addition defaults to one.
/// Default-module blocks accumulate with repeated slots replacing.
/// Slot blocks create slots, aliases copy existing ones, and `inherit`
/// copies a source slot; other values are reported. Whole-slot inheritance
/// uses parent, then archetype, with no slots on the null object. Copies
/// retain all model inputs and use a nonempty duplicate-definition sprite.
#[cfg(feature = "test-support")]
#[test]
fn model_inputs_are_read_in_order_and_copied_by_duplicates() {
    let database = load_one(
        "equipments = {
            tanks = { is_archetype = yes type = armor model = tank_mesh sprite = tank
                module_slots = { main = {} secondary = main } }
            tanks_0 = { archetype = tanks visual_level = 3 model = tier_mesh module_slots = inherit
                default_modules = { weapon = turret } default_modules = { weapon = gun other = empty } }
            tanks_1 = { archetype = tanks parent = tanks_0 visual_tech_level_addition = 2
                module_slots = { main = inherit extra = inherit } }
            early = { module_slots = inherit archetype = tanks }
            alias = { archetype = tanks module_slots = { main = {} extra = missing } }
        }
        duplicate_archetypes = {
            flame = { archetype = tanks type = { armor flame } sprite = flame_tank }
            amphibious = { archetype = tanks type = { armor amphibious } module_slots = none sprite = \"\" }
        }",
    );
    let kind = |name: &str| database.named(name).unwrap();
    let slots = |name: &str| -> Vec<String> {
        kind(name)
            .module_slots
            .iter()
            .map(|slot| String::from_utf8_lossy(slot).into_owned())
            .collect()
    };
    assert_eq!(kind("tanks").visual_tech_level_addition, 1);
    assert_eq!(kind("tanks_1").visual_tech_level_addition, 2);
    assert_eq!(
        (
            kind("tanks_0").visual_level,
            kind("tanks_0").model.as_slice()
        ),
        (3, b"tier_mesh".as_slice())
    );
    assert_eq!(
        kind("tanks_0").default_modules,
        [
            (b"weapon".to_vec(), b"gun".to_vec()),
            (b"other".to_vec(), b"empty".to_vec())
        ]
    );
    assert_eq!(slots("tanks"), ["main", "secondary"]);
    assert_eq!(slots("tanks_0"), ["main", "secondary"]);
    // Per slot: `main` is the parent's, `extra` is not.
    assert_eq!(slots("tanks_1"), ["main"]);
    // Before its archetype, `inherit` copies the null object's none.
    assert!(slots("early").is_empty());
    assert_eq!(slots("alias"), ["main"]);
    for reported in [
        "module slot extra inherits from tanks_0, which has no module slot of that name",
        "module slot extra copies undefined module slot missing",
    ] {
        assert!(
            database.misreads.iter().any(|note| note.contains(reported)),
            "{reported}: {:?}",
            database.misreads
        );
    }
    // The archetype's sprite reached tanks_0 when its `archetype` was
    // read.
    assert_eq!(kind("tanks_0").sprite, b"tank");
    let copy = kind("flame_0");
    assert_eq!(
        (copy.visual_level, copy.model.as_slice()),
        (3, b"tier_mesh".as_slice())
    );
    assert_eq!(slots("flame_0"), ["main", "secondary"]);
    assert_eq!(copy.sprite, b"flame_tank");
    assert_eq!(
        database.get("flame").unwrap() + 1,
        database.get("flame_0").unwrap()
    );
    // module_slots = none clears slots and default modules; an empty
    // sprite keeps the source's.
    assert!(slots("amphibious_0").is_empty());
    assert!(kind("amphibious_0").default_modules.is_empty());
    assert_eq!(kind("amphibious").sprite, b"tank");
}

/// Redefinition resets links, family, sprite, slots, modules, and tech-level
/// addition while retaining model, visual level, and stats. Finalization
/// sums parent additions with a minimum of one and counts parents.
#[cfg(feature = "test-support")]
#[test]
fn a_redefinition_is_reset_and_read_again() {
    let database = load_one(
        "equipments = {
            base = { is_archetype = yes visual_tech_level_addition = -5 }
            a = { is_archetype = yes sprite = x model = m visual_level = 3 lg_attack = 2
                visual_tech_level_addition = 4 family = base module_slots = { s = {} }
                variant_name = named derived_variant_name = derived }
            A = { parent = base }
            b = { parent = a visual_tech_level_addition = 2 }
        }",
    );
    let a = database.named("a").unwrap();
    assert_eq!(a.name, b"a");
    assert!(!a.is_archetype && a.archetype == database.get("base"));
    assert_eq!(a.family, database.get("a").unwrap());
    assert!(a.sprite.is_empty() && a.module_slots.is_empty());
    assert_eq!((a.model.as_slice(), a.visual_level), (b"m".as_slice(), 3));
    // The option-list name keys are retained like the model.
    assert_eq!(
        (a.variant_name.as_slice(), a.derived_variant_name.as_slice()),
        (b"named".as_slice(), b"derived".as_slice())
    );
    assert_eq!(a.stats[stat("lg_attack")], 200_000);
    assert_eq!(a.visual_tech_level_addition, 1);
    // b: 2 + 1 (a) - 5 (base) is below 1; two parents above it.
    let b = database.named("b").unwrap();
    assert_eq!((b.model_priority, b.depth), (1, 2));
    assert_eq!(database.named("a").unwrap().model_priority, 1);
}

#[cfg(feature = "test-support")]
#[test]
fn air_map_frames_keep_signed_scans_and_reset_instead_of_inheriting() {
    let database = load_one(
        "equipments = {
            planes = { is_archetype = yes air_map_icon_frame = 17 carrier_capable = yes }
            child = { parent = planes }
            signed = { air_map_icon_frame = -7 air_map_icon_frame = invalid }
            reset = { air_map_icon_frame = 22 carrier_capable = yes }
            reset = { parent = planes }
        }",
    );
    assert_eq!(named(&database, "planes").air_map_icon_frame, 17);
    assert_eq!(named(&database, "child").air_map_icon_frame, 0);
    assert_eq!(named(&database, "signed").air_map_icon_frame, -7);
    assert_eq!(named(&database, "reset").air_map_icon_frame, 0);
    assert!(named(&database, "planes").carrier_capable);
    assert!(!named(&database, "child").carrier_capable);
    assert!(!named(&database, "reset").carrier_capable);
}

/// Rereading a generated type clears its generated flag, allowing it to
/// participate in a later duplication pass.
#[test]
fn redefined_generated_equipment_can_be_duplicated_again() {
    let database = read(&[(
        "a.txt",
        "equipments = { hull = { is_archetype = yes } }
         duplicate_archetypes = { copy = { archetype = hull } }
         equipments = { copy = { is_archetype = yes lg_attack = 3 } }
         duplicate_archetypes = { final = { archetype = copy } }",
    )]);
    assert!(database.lost.is_none(), "{:?}", database.lost);
    let redefined = named(&database, "copy");
    assert!(redefined.is_archetype && !redefined.duplicate);
    let generated = named(&database, "final");
    assert!(generated.is_archetype && generated.duplicate);
    assert_eq!(generated.stats[stat("lg_attack")], 300_000);
}

/// Aliases append tokens and register names after the body, so only
/// later bodies resolve new aliases.
#[test]
fn equipment_aliases_resolve_links_and_duplicate_sources() {
    let database = read(&[(
        "a.txt",
        "equipments = {
            hull = { is_archetype = yes alias = HullAlias
                alias = { \"OtherAlias\" } parent = HullAlias }
            hull_1 = { archetype = HULLALIAS family = otheralias }
         }
         duplicate_archetypes = { copy = { archetype = HullAlias } }",
    )]);
    assert!(database.lost.is_none(), "{:?}", database.lost);
    let hull = database.get("hull").unwrap();
    assert_eq!(database.get("hullalias"), Some(hull));
    assert_eq!(database.get("OTHERALIAS"), Some(hull));
    assert_eq!(named(&database, "hull_1").archetype, Some(hull));
    assert_eq!(named(&database, "hull_1").family, hull);
    assert!(
        database
            .misreads
            .iter()
            .any(|note| note.contains("parent HullAlias is no equipment defined before it"))
    );
    assert!(named(&database, "copy").duplicate);
    assert!(named(&database, "copy_1").duplicate);
    // Clone aliases are cleared and still name the original hull.
    assert!(named(&database, "copy").aliases.is_empty());
}

/// Redefinitions use the ordinary name index and remove old aliases first.
#[test]
fn redefining_by_alias_keeps_the_object_and_replaces_its_aliases() {
    let database = read(&[(
        "a.txt",
        "equipments = {
            hull = { is_archetype = yes alias = { old retained } lg_attack = 2 }
            OLD = { is_archetype = yes alias = new lg_attack = 5 }
            hull_1 = { archetype = NEW }
         }",
    )]);
    assert!(database.lost.is_none(), "{:?}", database.lost);
    assert_eq!(database.equipment.len(), 2);
    assert!(database.get("old").is_none());
    assert!(database.get("retained").is_none());
    let hull = database.get("hull").unwrap();
    assert_eq!(database.get("new"), Some(hull));
    assert_eq!(database.equipment[hull].stats[stat("lg_attack")], 500_000);
    assert_eq!(named(&database, "hull_1").archetype, Some(hull));
}

/// Type registration overwrites existing name-index entries. Reload cleanup
/// erases aliases without testing which type the index currently names.
#[test]
fn equipment_alias_collisions_follow_index_mutations() {
    let database = read(&[(
        "a.txt",
        "equipments = {
            first = { alias = shared }
            second = { alias = { shared first } }
            first = { alias = latest }
         }",
    )]);
    assert!(database.lost.is_none(), "{:?}", database.lost);
    assert_eq!(database.equipment.len(), 2);
    assert_eq!(database.get("latest"), database.get("second"));
    assert!(database.get("shared").is_none());
    assert!(database.get("first").is_none());
    assert_eq!(database.equipment[0].name, b"first");
}

#[test]
fn an_equipment_alias_with_token_zero_refuses_the_database_by_name() {
    let database = read(&[("a.txt", "equipments = { hull = { alias = 7 } later = { } }")]);
    let lost = database.lost.as_deref().unwrap();
    assert!(
        lost.contains("equipment hull alias \"7\" registers token 0"),
        "{lost}"
    );
    assert!(database.get("later").is_none());
}

/// Authored categories accumulate. Own categories replace archetype
/// categories, and default modules add their categories. Copies retain
/// own categories and source defaults. Capital ships clear convoy,
/// submarine, and screen bits.
#[cfg(feature = "test-support")]
#[test]
fn category_masks_or_types_and_default_modules_then_normalize() {
    let (database, _) = fixture::load(&[
        (
            "common/units/equipment/modules/m.txt",
            "equipment_modules = { engine_1 = { add_equipment_type = { naval_bomber } } }",
        ),
        (
            "common/units/equipment/a.txt",
            "equipments = {
                large_plane_airframe = { is_archetype = yes type = strategic_bomber
                    default_modules = { engine_type_slot = engine_1 fixed_main_weapon_slot = empty } }
                large_plane_airframe_1 = { archetype = large_plane_airframe }
                large_plane_airframe_2 = { parent = large_plane_airframe_1 type = { fighter } }
                carrier_hull = { is_archetype = yes type = { capital_ship carrier } type = submarine }
            }
            duplicate_archetypes = {
                large_plane_cas_airframe = { archetype = large_plane_airframe type = cas }
            }",
        ),
    ]);
    let category = |name: &str| database.category(database.named(name).unwrap());
    assert_eq!(category("large_plane_airframe"), 0x2000 | 0x8000);
    assert_eq!(category("large_plane_airframe_2"), 0x400);
    assert_eq!(category("large_plane_airframe_1"), 0x2000);
    assert_eq!(category("large_plane_cas_airframe"), 0x4000 | 0x8000);
    assert_eq!(category("carrier_hull"), 0x40 | 0x40_0000);
    assert_eq!(category_mask("Air_Transport"), Some(0x10_0000));
}

/// Group declarations keep the first token, report repeats, read icon and
/// description, and accumulate equipment members. Unknown fields skip.
/// Expansion uses folded tokens and retains nongroup names once. Admission
/// checks type, archetype, and archetype family by folded name, otherwise
/// category keywords without decimal conversion. Cycles remain refused.
#[cfg(feature = "test-support")]
#[test]
fn groups_expand_and_admit_by_name_family_or_category() {
    let (database, groups) = fixture::load(&[
        (
            "common/units/equipment/modules/m.txt",
            "equipment_modules = { Gun_1 = { } }",
        ),
        (
            "common/units/equipment/a.txt",
            "equipments = {
                large_plane_airframe = { is_archetype = yes type = strategic_bomber }
                convoy = { is_archetype = yes type = convoy }
                hull = { is_archetype = yes type = screen_ship family = convoy }
            }
            duplicate_archetypes = { large_plane_cas_airframe = { archetype = large_plane_airframe type = cas } }",
        ),
        (
            "common/equipment_groups/g.txt",
            "bombers = { description = \"Bombers\" icon = x unknown = { a } equipment_type = { strategic_bomber NESTED } }
             plain = { equipment_type = { a } }
             nested = { equipment_type = { large_plane_airframe Strategic_Bomber } equipment_type = { 1024 } }
             Bombers = { equipment_type = { other } }
             loop_a = { equipment_type = { loop_b } }
             loop_b = { equipment_type = { loop_a } }",
        ),
    ]);
    assert!(groups.lost.is_none(), "{:?}", groups.lost);
    let members = groups.expand(&["bombers"]).unwrap();
    assert_eq!(
        members,
        [
            b"strategic_bomber".to_vec(),
            b"large_plane_airframe".to_vec(),
            b"1024".to_vec()
        ]
    );
    assert!(
        groups
            .misreads
            .iter()
            .any(|note| note.contains("equipment group Bombers is declared again")),
        "{:?}",
        groups.misreads
    );
    let at = |name: &str| database.get(name).unwrap();
    let admits = |members: &[&str], name: &str| {
        let members: Vec<Vec<u8>> = members.iter().map(|m| m.as_bytes().to_vec()).collect();
        database.group_admits(&members, at(name))
    };
    assert!(admits(&["CAS"], "large_plane_cas_airframe"));
    // Its archetype is itself, whose family is the type it copies.
    assert!(admits(
        &["Large_Plane_Airframe"],
        "large_plane_cas_airframe"
    ));
    assert!(!admits(&["fighter"], "large_plane_airframe"));
    // The decimal fighter bit is not a category keyword.
    assert!(!admits(&["1024"], "large_plane_airframe"));
    assert!(admits(&["convoy"], "hull"));
    let cycle = groups.expand(&["loop_a"]).unwrap_err();
    assert!(cycle.contains("loop_a -> loop_b -> loop_a"), "{cycle}");
    // Group display uses description, otherwise its name.
    assert_eq!(groups.group(b"BOMBERS").unwrap().unwrap().key(), b"Bombers");
    assert_eq!(groups.group(b"plain").unwrap().unwrap().key(), b"plain");
    assert!(groups.group(b"strategic_bomber").unwrap().is_none());
    // Module lookup returns the registered name by token.
    assert_eq!(database.module("GUN_1"), Some(b"Gun_1".as_slice()));
    assert_eq!(database.module("missing"), None);
}

/// Keywords fold: field keys, `inherit`, `none` and category
/// names match whatever their case; names keep theirs (the
/// first spelling, found without case by type-name lookup), and
/// so do the sprite and model texts.
#[cfg(feature = "test-support")]
#[test]
fn keywords_fold_and_names_keep_their_case() {
    let (database, groups) = fixture::load(&[
        (
            "common/units/equipment/a.txt",
            "Equipments = {
                Hull = { IS_ARCHETYPE = yes TYPE = { Screen_Ship } Sprite = Hull_Sprite MODEL = Hull_Mesh
                    Module_Slots = { Main = {} } Visual_Level = 2 Default_Modules = { Main = Gun } }
                hull_1 = { Archetype = HULL Module_Slots = INHERIT }
            }
            Duplicate_Archetypes = { Raider = { ARCHETYPE = hull Module_Slots = NONE Type = CONVOY } }",
        ),
        (
            "common/equipment_groups/g.txt",
            "Ships = { EQUIPMENT_TYPE = { HULL } }",
        ),
    ]);
    assert!(database.lost.is_none(), "{:?}", database.lost);
    let hull = database.named("HULL").unwrap();
    assert_eq!(hull.name, b"Hull");
    assert!(hull.is_archetype);
    assert_eq!(
        (hull.sprite.as_slice(), hull.model.as_slice()),
        (b"Hull_Sprite".as_slice(), b"Hull_Mesh".as_slice())
    );
    assert_eq!(hull.visual_level, 2);
    assert_eq!(hull.default_modules, [(b"Main".to_vec(), b"Gun".to_vec())]);
    assert_eq!(database.category(hull), 1 << 8);
    let copy = database.named("hull_1").unwrap();
    assert_eq!(copy.archetype, database.get("hull"));
    assert_eq!(copy.module_slots, [b"Main".to_vec()]);
    let raider = database.named("raider").unwrap();
    assert!(raider.module_slots.is_empty() && raider.default_modules.is_empty());
    assert_eq!(database.category(raider), 1);
    assert_eq!(groups.expand(&["ships"]).unwrap(), [b"HULL".to_vec()]);
    assert!(database.group_admits(&[b"HULL".to_vec()], database.get("hull_1").unwrap()));
}

#[test]
fn stat_metadata_covers_the_index_order() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("equipment metadata checks require BATTLE_PLAN_GAME_DIR");
    assert_eq!(stat_metadata().len(), STAT_COUNT);
    for (index, &(keyword, label, _)) in stat_metadata().iter().enumerate() {
        assert_eq!(stat_index(keyword.as_bytes()), Some(index), "{keyword}");
        assert!(!label.is_empty(), "{keyword}");
    }
    assert_eq!(
        stat_metadata()[4],
        ("soft_attack", "STAT_ARMY_SOFT_ATTACK", true)
    );
    assert_eq!(
        stat_metadata()[11],
        ("supply_consumption", "STAT_ARMY_SUPPLY_CONSUMPTION", false)
    );
    assert_eq!(
        stat_metadata()[77],
        ("build_cost_ic", "STAT_COMMON_BUILD_COST_IC", false)
    );
}

#[test]
fn saved_validation_requires_explicit_dependency_but_allows_required_default_modules() {
    let database = read(&[(
        "rules.txt",
        "equipment_modules={basic={category=turret} advanced={category=turret allowed_module_categories={gun_slot={heavy}}} light={category=light} heavy={category=heavy}} equipments={hull={is_archetype=yes module_slots={turret_slot={required=yes allowed_module_categories={turret}} gun_slot={required=yes allowed_module_categories={light}}} default_modules={turret_slot=basic gun_slot=light}}}",
    )]);
    let hull = database.named("hull").unwrap();
    let empty = [Assignment {
        slot: b"gun_slot",
        module: b"",
        active: false,
    }];
    assert!(
        database
            .validate_modules(hull, &empty)
            .unwrap()
            .failures
            .iter()
            .any(|failure| failure.contains("required slot gun_slot"))
    );
    assert!(database.permitted_role_mask(hull, &empty).is_ok());
    assert!(
        database
            .validate_modules(hull, &[])
            .unwrap()
            .failures
            .is_empty()
    );
    let gun = Assignment {
        slot: b"gun_slot",
        module: b"heavy",
        active: true,
    };
    let failed = database.validate_modules(hull, &[gun]).unwrap();
    assert!(failed.failures[0].contains("no installed dependency"));
    let rows = [
        Assignment {
            slot: b"turret_slot",
            module: b"advanced",
            active: true,
        },
        Assignment {
            slot: b"gun_slot",
            module: b"heavy",
            active: true,
        },
    ];
    assert!(
        database
            .validate_modules(hull, &rows)
            .unwrap()
            .failures
            .is_empty()
    );
    let inactive = [Assignment {
        slot: b"turret_slot",
        module: b"advanced",
        active: false,
    }];
    assert!(
        database
            .validate_modules(hull, &inactive)
            .unwrap()
            .failures
            .iter()
            .any(|failure| failure.contains("required slot turret_slot"))
    );
}

/// An alias that takes over its parent's name does not redirect the default
/// walk: it follows the stored links and ends at the original type.
#[test]
fn default_modules_follow_stored_links_when_an_alias_takes_over_the_parent_name() {
    let database = read(&[(
        "alias.txt",
        "equipment_modules={basic={category=turret}} equipments={root={is_archetype=yes module_slots={main={required=yes allowed_module_categories={turret}} extra={required=yes allowed_module_categories={turret}}} default_modules={extra=basic}} child={parent=root alias=root module_slots=inherit}}",
    )]);
    assert!(database.lost.is_none(), "{:?}", database.lost);
    let child = database.named("root").unwrap();
    assert_eq!(child.name, b"child");
    assert_eq!(
        database.validate_modules(child, &[]).unwrap().failures,
        ["required slot main is empty"]
    );
    // Registration order: root 0, child 1. A database built with a loop is refused.
    let mut looped = database.clone();
    looped.equipment[0].parent = 1;
    assert!(
        looped
            .validate_modules(&looped.equipment[1], &[])
            .unwrap_err()
            .contains("links back through its parents and archetypes")
    );
    let detached = database.equipment[0].clone();
    assert!(
        database
            .validate_modules(&detached, &[])
            .unwrap_err()
            .contains("not a type of this equipment database")
    );
}

#[test]
fn count_validation_uses_present_explicit_keys_with_child_override_and_native_comparison_mapping() {
    let database = read(&[(
        "counts.txt",
        "equipment_modules={a={category=c} b={category=d}} equipments={root={is_archetype=yes module_count_limit={category=c count<2} module_count_limit={category=missing count<0}} child={archetype=root module_count_limit={category=c count<3}} lower={archetype=root module_count_limit={category=c count=1}} equal={archetype=root module_count_limit={category=c count>2}}}",
    )]);
    let rows = [
        Assignment {
            slot: b"one",
            module: b"a",
            active: true,
        },
        Assignment {
            slot: b"two",
            module: b"a",
            active: false,
        },
    ];
    assert_eq!(
        database
            .validate_modules(database.named("root").unwrap(), &rows)
            .unwrap()
            .failures
            .len(),
        1
    );
    for name in ["child", "lower", "equal"] {
        assert!(
            database
                .validate_modules(database.named(name).unwrap(), &rows)
                .unwrap()
                .failures
                .is_empty(),
            "{name}"
        );
    }
    assert!(
        database
            .validate_modules(database.named("root").unwrap(), &[])
            .unwrap()
            .failures
            .is_empty()
    );
}
#[test]
fn role_masks_keep_allow_add_forbid_override_and_exact_category_restrictions_separate() {
    let mut database = read(&[(
        "roles.txt",
        "equipment_modules={a={category=gun allow_equipment_type=1152921504606846976 add_equipment_type=2305843009213693952 forbid_equipment_type=4611686018427387904 forbid_equipment_type_exact_match=1152921504606846976} b={category=turret forbid_equipment_type_exact_match_for_category={gun=2305843009213693952}}} equipments={root={is_archetype=yes module_slots={main={} other={}}}}",
    )]);
    database.equipment[0].category = 4_611_686_018_427_387_904;
    let rows = [
        Assignment {
            slot: b"main",
            module: b"a",
            active: true,
        },
        Assignment {
            slot: b"other",
            module: b"b",
            active: true,
        },
    ];
    let kind = database.named("root").unwrap();
    let first = 1_152_921_504_606_846_976;
    let second = 2_305_843_009_213_693_952;
    assert_eq!(
        database.permitted_role_mask(kind, &rows).unwrap(),
        first | second
    );
    assert!(!database.role_permitted(kind, &rows, first).unwrap());
    assert!(!database.role_permitted(kind, &rows, second).unwrap());
    assert!(
        database
            .role_permitted(kind, &rows, first | second)
            .unwrap()
    );
    database.equipment[0].type_override = b"main".to_vec();
    assert_eq!(
        database
            .permitted_role_mask(database.named("root").unwrap(), &rows)
            .unwrap(),
        second
    );
}
