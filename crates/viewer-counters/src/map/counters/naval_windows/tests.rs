use super::super::gui_windows::{BG_FRAME, FRAME_BTN, GRIDS, PROGRESS};

/// A base written through one hour's countries takes the next hour's table when its flags are the
/// same, so the frames after find it the same by pointer; other flags leave it as written.
#[test]
fn a_written_base_takes_a_new_countries_table_only_with_the_same_flags() {
    let written = |countries: &Arc<Countries>| BaseWritten {
        province: 7,
        glyph: None,
        scale: 0,
        strategic: false,
        items: Ok(Vec::new()),
        art: 0,
        countries: Some(Arc::clone(countries)),
        settled: true,
        overflow: 0,
    };
    let tagged = |tag: &str| Countries { tags: [(1, tag.to_owned())].into(), ..Countries::default() };
    let held = Arc::new(tagged("GER"));
    let same = Arc::new(tagged("GER"));
    let other = Arc::new(tagged("ITA"));
    let mut bases = vec![written(&held), written(&held)];
    adopt_countries(bases.iter_mut(), Some(&other));
    assert!(bases.iter().all(|base| Arc::ptr_eq(base.countries.as_ref().unwrap(), &held)));
    assert!(!bases[0].same(7, None, 0, false, Ok(&[]), 0, Some(&other)));
    adopt_countries(bases.iter_mut(), Some(&same));
    assert!(bases.iter().all(|base| Arc::ptr_eq(base.countries.as_ref().unwrap(), &same)));
    assert!(bases[1].same(7, None, 0, false, Ok(&[]), 0, Some(&same)));
}

/// A base stands only once written twice in a row with the same inputs, the second time quiet,
/// and a standing base still counts the port items its grid has no slot for. The first write of
/// a no-intel port item drops the frame written before its sprite, which the second sets; the
/// items of a fading base take its opacity before the tick that ends the fade, which only the
/// next write gives them.
#[test]
fn a_base_stands_once_written_twice_alike_and_keeps_counting_what_its_grid_left_out() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let written = |settled, overflow| BaseWritten {
        province: 7,
        glyph: None,
        scale: 0,
        strategic: false,
        items: Ok(Vec::new()),
        art: 0,
        countries: None,
        settled,
        overflow,
    };
    // Written once, then given the same on a quiet window: written again, and only then standing.
    assert_eq!(base_due(None, false, false), BaseDue::Write { settled: false });
    assert_eq!(base_due(Some(&written(false, 0)), true, true), BaseDue::Write { settled: true });
    assert_eq!(base_due(Some(&written(true, 2)), true, true), BaseDue::Stands { overflow: 2 });
    // A fading window is not quiet, and other inputs are written: neither settles.
    assert_eq!(base_due(Some(&written(true, 0)), false, false), BaseDue::Write { settled: false });
    assert_eq!(base_due(Some(&written(true, 0)), true, false), BaseDue::Write { settled: false });

    // A port grid of four slots takes four items: the fifth is left out, every write.
    let gui = GUI.replace(
        "max_slots_horizontal = 1 } } }",
        "max_slots_horizontal = 1 add_horizontal = no max_slots_vertical = 4 } } }",
    );
    let mut view = View::with_gui(&gui);
    let base = view.base(1);
    let hulls: Vec<_> = (0..5).map(|frame| view.hull(frame)).collect();
    assert_eq!(view.ports(base, &hulls), 1);
}

/// The frames a paused map paints: each one the base's update and the view's tick, the next only
/// while the view asks for it. A port item switched to a sprite that drops the frame written
/// before it shows that frame once the view stops asking.
#[test]
fn a_paused_map_paints_a_replaced_item_sprite_s_frame_before_it_stops() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut view = View::new();
    let base = view.base(1);
    let plain = [view.hull(2).unwrap()];
    assert!(view.paint_until_settled(base, &plain) > 1);
    // The frame written first, then the sprite: a replacement takes its own drawable's frame.
    let mut writes = gui_windows::Writes::new(view.native.navy());
    writes.frame(native::Kind::Icon, "type", 3);
    writes.control(native::Kind::Icon, "type", |bound| {
        bound.sprite = Some(b"GFX_tiny".to_vec());
        bound.setter = native_gui::SpriteSetter::Icon;
    });
    let replaced = [writes.update()];
    let once = {
        let mut once = View::new();
        let base = once.base(1);
        once.paint_until_settled(base, &plain);
        once.paint(base, &replaced);
        once.port_items(base).0
    };
    assert_eq!(once.iter().map(|(frame, _)| *frame).collect::<Vec<_>>(), [0], "one write leaves the new drawable's frame");
    assert!(view.paint_until_settled(base, &replaced) >= 2, "the view asks for the frame that writes it again");
    assert_eq!(view.port_items(base).0.iter().map(|(frame, _)| *frame).collect::<Vec<_>>(), [2]);
}

/// A base fading in is written while it fades; the view keeps asking until a write made after
/// the fade has ended gives its port items the grid's whole opacity.
#[test]
fn a_paused_map_paints_port_items_opaque_once_their_base_has_faded_in() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut view = View::new();
    let base = view.base(1);
    let hulls = [view.hull(1).unwrap(), view.hull(2).unwrap()];
    let painted = view.paint_until_settled(base, &hulls);
    assert!(painted > 1, "the fade takes more than a frame: {painted}");
    let window = view.windows.bases.as_ref().unwrap().instance(base).unwrap();
    assert_eq!(window.opacity(), 1.0);
    for index in 0..2 {
        let key = ItemKey { icon: IconSlot::Base(base), pool: ItemPool::Port, index };
        assert_eq!(view.windows.items[&key].instance.opacity(), 1.0, "item {index}");
    }
}

/// A port item whose own show movement outlasts its base's fade stands where a grid finalized
/// every frame puts it, frame by frame: its base is written while any item under it moves. One
/// withheld while a roster was undecided is written through its movement too, and first drawn
/// where that grid puts it. One withheld by a part no frame can learn, not by its transition, asks
/// for no frame while it moves.
#[test]
fn a_port_item_moving_after_its_base_settles_stays_where_the_grid_puts_it() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let gui = GUI.replace(
        r#"containerWindowType = { name = "unit_counter_navy" size = { width = 20 height = 10 }"#,
        r#"containerWindowType = { name = "unit_counter_navy" size = { width = 20 height = 10 }
        show_position = { x = 40 y = 0 } show_animation_type = linear show_animation_time = 1000"#,
    );
    assert_ne!(gui, GUI);
    let (mut kept, mut every) = (View::with_gui(&gui), View::with_gui(&gui));
    let (base, reference) = (kept.base(1), every.base(1));
    let hulls = [kept.hull(1)];
    let written = [hulls[0].clone().unwrap()];
    let key = |slot| ItemKey { icon: IconSlot::Base(slot), pool: ItemPool::Port, index: 0 };
    for frame in 0..120 {
        kept.paint(base, &written);
        every.ports(reference, &hulls);
        every.tick(1.0 / 60.0);
        assert_eq!(
            kept.windows.items[&key(base)].instance.position(),
            every.windows.items[&key(reference)].instance.position(),
            "frame {frame}"
        );
    }

    let (mut kept, mut every) = (View::with_gui(&gui), View::with_gui(&gui));
    let (base, reference) = (kept.base(1), every.base(1));
    let hulls = [kept.full_hull(1)];
    let written = [hulls[0].clone().unwrap()];
    let why: Cow<'static, str> = "the roster is not recorded".into();
    let mut frame = 0;
    loop {
        let roster: Result<&[gui_windows::Update], _> = if (1..20).contains(&frame) { Err(&why) } else { Ok(&written) };
        kept.paint_bases(&[(base, roster)]);
        every.ports(reference, &hulls);
        every.tick(1.0 / 60.0);
        let ship = &kept.windows.items[&key(base)];
        if frame >= 20 && ship.instance.drawn() && kept.windows.items_withheld.why(&key(base)).is_none() {
            assert_eq!(
                ship.instance.position(),
                every.windows.items[&key(reference)].instance.position(),
                "first drawn at frame {frame}"
            );
            break;
        }
        frame += 1;
        assert!(frame < 600, "the ship is drawn again");
    }

    let mut view = View::with_gui(&gui);
    let base = view.base(1);
    let mut flagless = view.full_hull(1).unwrap();
    flagless.unknown.push((gui_windows::FLAG, "the Ships census is incomplete".into()));
    let roster = [flagless];
    let mut painted = 0;
    while view.paint(base, &roster) {
        painted += 1;
        assert!(painted < 600, "a static map stops asking for frames");
    }
    let ship = &view.windows.items[&key(base)];
    assert!(view.windows.items_withheld.why(&key(base)).is_some(), "the flag stays unknown");
    assert!(ship.instance.transitioning(), "frames stop while the ship still moves ({painted} painted)");
}


/// A base whose roster turns undecided keeps the port items it held, and stands no frame while
/// its roster is undecided, here while one of those items still moves: its items' parts and
/// transitions stay unknown until a decided roster writes them.
#[test]
fn an_undecided_roster_never_lets_its_base_stand() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let gui = GUI.replace(
        r#"containerWindowType = { name = "unit_counter_navy" size = { width = 20 height = 10 }"#,
        r#"containerWindowType = { name = "unit_counter_navy" size = { width = 20 height = 10 }
        show_position = { x = 40 y = 0 } show_animation_type = linear show_animation_time = 1000"#,
    );
    let mut view = View::with_gui(&gui);
    let base = view.base(1);
    let hulls = [view.hull(1).unwrap()];
    view.paint(base, &hulls);
    let why: Cow<'static, str> = "the roster is not recorded".into();
    for frame in 0..40 {
        let (stood, ..) = view.paint_bases(&[(base, Err(&why))]);
        assert_eq!(stood, [false], "frame {frame}");
    }
    let key = ItemKey { icon: IconSlot::Base(base), pool: ItemPool::Port, index: 0 };
    assert!(view.windows.items[&key].instance.active(), "the retained item still moves");
}

/// A base whose update fails the same way every frame is withheld alone, under why, every
/// frame, and owes the view no frame; the other base keeps what it wrote and stands, so a static
/// map stops asking for frames.
#[test]
fn a_base_that_keeps_failing_lets_a_static_map_stop_painting() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut view = View::new();
    let base = view.base(1);
    let gone = view.base(2);
    // The second base's window is let go of: its update finds none, every frame.
    let window = view.native.naval_base().unwrap();
    view.windows.bases.as_mut().unwrap().erase(window, &2).unwrap();
    let hulls = [view.hull(1).unwrap()];
    let mut painted = 0;
    loop {
        painted += 1;
        let (_, asks, report) = view.paint_bases(&[(base, Ok(&hulls)), (gone, Ok(&hulls))]);
        assert_eq!(
            report.navy_counter_gaps.get("port items are not placed: an updated icon has no window"),
            Some(&1),
            "frame {painted}"
        );
        if !asks {
            break;
        }
        assert!(painted < 60, "a static map stops asking for frames");
    }
    let (stood, ..) = view.paint_bases(&[(base, Ok(&hulls)), (gone, Ok(&hulls))]);
    assert_eq!(stood, [true, false], "the working base stands");
}

/// A base that fails every frame after it made a port item with a second-long fade keeps that
/// item withheld while the failure lasts, and a static map stops asking for frames: the item's
/// wait starts only once its updates resume.
#[test]
fn a_base_failing_after_it_made_a_fading_item_lets_a_static_map_stop_painting() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let gui = GUI.replace(
        r#"containerWindowType = { name = "unit_counter_navy" size = { width = 20 height = 10 }"#,
        r#"containerWindowType = { name = "unit_counter_navy" size = { width = 20 height = 10 }
        fade_type = linear fade_time = 1000"#,
    );
    assert_ne!(gui, GUI);
    let mut view = View::with_gui(&gui);
    let base = view.base(1);
    let hulls = [view.hull(1).unwrap()];
    for _ in 0..5 {
        view.paint(base, &hulls);
    }
    // The base's window is let go of: its update finds none, every frame.
    let window = view.native.naval_base().unwrap();
    view.windows.bases.as_mut().unwrap().erase(window, &1).unwrap();
    let item = ItemKey { icon: IconSlot::Base(base), pool: ItemPool::Port, index: 0 };
    for painted in 1.. {
        let (_, asks, report) = view.paint_bases(&[(base, Ok(&hulls))]);
        assert_eq!(
            report.navy_counter_gaps.get("port items are not placed: an updated icon has no window"),
            Some(&1),
            "frame {painted}: the failure is named every frame"
        );
        if !asks {
            break;
        }
        assert!(painted < 200, "a static map stops asking for frames");
    }
    assert!(view.windows.items_withheld.why(&item).is_some(), "the item stays withheld");
}

/// A base whose decided roster empties keeps no check on the items it no longer writes: one left
/// behind with a looping sprite neither keeps the base written nor keeps a static map painting.
#[test]
fn an_emptied_roster_with_a_looping_item_left_behind_lets_a_static_map_stop_painting() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut view = View::new();
    view.fixture.animated.insert(b"GFX_hull".to_vec(), 0.125);
    let base = view.base(1);
    let hulls = [view.hull(1).unwrap()];
    for _ in 0..10 {
        view.paint(base, &hulls);
    }
    let item = ItemKey { icon: IconSlot::Base(base), pool: ItemPool::Port, index: 0 };
    assert!(view.windows.items[&item].instance.active(), "the item's sprite loops");
    let painted = view.paint_until_settled(base, &[]);
    assert!(painted < 60, "a static map stops asking for frames: {painted}");
    let (stood, ..) = view.paint_bases(&[(base, Ok(&[]))]);
    assert_eq!(stood, [true], "the emptied base stands");
}

/// A slot an update the view cannot decide marked at one frame, and that an allocation marks at
/// the next, waits out its transition on frames the view asks for: the allocation's mark is the
/// frame's own event, which no later frame repeats. Its window's only transition is a hidden
/// child's, so it is idle all the while.
#[test]
fn an_allocation_mark_after_an_undecided_mark_gets_its_recovery_frames() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let gui = GUI.replace(
        r#"containerWindowType = { name = "naval_base_mapicon" size = { width = 32 height = 32 }
        fade_type = linear fade_time = 100"#,
        r#"containerWindowType = { name = "naval_base_mapicon" size = { width = 32 height = 32 }
        containerWindowType = { name = "later" size = { width = 4 height = 4 } hide = yes
            fade_type = linear fade_time = 250 }"#,
    );
    assert_ne!(gui, GUI);
    let mut view = View::with_gui(&gui);
    let base = view.base(1);
    let hulls = [view.hull(1).unwrap()];
    view.paint_until_settled(base, &hulls);
    let pool = view.windows.bases.as_ref().unwrap();
    let window = pool.instance(base).unwrap();
    assert!(window.longest_transition(view.native.naval_base().unwrap()).unwrap() > 0.0);
    assert!(!window.transitioning());
    let icon = Registered::Icon(IconSlot::Base(base));
    transition_unknown(&mut view.windows, icon, "an undecided update", Marked::EveryFrame);
    view.paint(base, &hulls);
    transition_unknown(&mut view.windows, icon, "a new owner in a reused slot", Marked::Once);
    view.paint_until_settled(base, &hulls);
    assert_eq!(view.windows.bases.as_ref().unwrap().withheld(base), None, "known again");
}

/// A full port grid leaves out an item whose update shows a looping sprite: the item draws
/// nowhere, so neither its base nor a static map keeps asking for frames over it.
#[test]
fn a_looping_item_a_full_grid_left_out_lets_a_static_map_stop_painting() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let gui = GUI.replace(
        "max_slots_horizontal = 1 } } }",
        "max_slots_horizontal = 1 add_horizontal = no max_slots_vertical = 4 } } }",
    );
    let mut view = View::with_gui(&gui);
    view.fixture.animated.insert(b"GFX_tiny".to_vec(), 0.125);
    let base = view.base(1);
    let mut items: Vec<_> = (0..4).map(|frame| view.hull(frame).unwrap()).collect();
    let mut writes = gui_windows::Writes::new(view.native.navy());
    writes.show(native::Kind::Icon, "type");
    writes.control(native::Kind::Icon, "type", |bound| {
        bound.sprite = Some(b"GFX_tiny".to_vec());
        bound.setter = native_gui::SpriteSetter::Icon;
    });
    items.push(writes.update());
    let painted = view.paint_until_settled(base, &items);
    let left_out = ItemKey { icon: IconSlot::Base(base), pool: ItemPool::Port, index: 4 };
    assert!(view.windows.items[&left_out].instance.active(), "the left-out item's sprite loops");
    assert!(painted < 60, "a static map stops asking for frames: {painted}");
    let (stood, _, report) = view.paint_bases(&[(base, Ok(&items))]);
    assert_eq!(stood, [true]);
    assert_eq!(report.navy_counter_gaps.get(naval::PORT_GRID_FULL), Some(&1), "the left-out item is still counted");
}

/// A base that stood over a roster its grid could not hold whole is written again when the roster
/// empties or turns undecided: what its grid held is that roster's own, its first items up to
/// those it left out.
#[test]
fn a_base_leaving_an_overflowing_roster_reads_what_that_roster_put_in_its_grid() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let gui = GUI.replace(
        "max_slots_horizontal = 1 } } }",
        "max_slots_horizontal = 1 add_horizontal = no max_slots_vertical = 4 } } }",
    );
    let why: Cow<'static, str> = "the roster is not recorded".into();
    for undecided in [false, true] {
        let mut view = View::with_gui(&gui);
        let base = view.base(1);
        let ships: Vec<_> = (0..5).map(|frame| view.hull(frame % 3).unwrap()).collect();
        view.paint_until_settled(base, &ships);
        let (stood, ..) = view.paint_bases(&[(base, Ok(&ships))]);
        assert_eq!(stood, [true], "five ships over four slots stand");
        let next: Result<&[gui_windows::Update], _> = if undecided { Err(&why) } else { Ok(&[]) };
        let (stood, ..) = view.paint_bases(&[(base, next)]);
        assert_eq!(stood, [false], "undecided {undecided}: written again");
    }
}

/// A port roster turns undecided while a ship its grid holds shows a looping sprite: the ship's
/// draw is withheld, so it neither keeps its base written nor keeps a static map painting.
#[test]
fn a_withheld_looping_ship_lets_a_static_map_stop_painting() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut view = View::new();
    view.fixture.animated.insert(b"GFX_hull".to_vec(), 0.125);
    let base = view.base(1);
    let hulls = [view.hull(1).unwrap()];
    for _ in 0..10 {
        view.paint(base, &hulls);
    }
    let ship = ItemKey { icon: IconSlot::Base(base), pool: ItemPool::Port, index: 0 };
    assert!(view.windows.items[&ship].instance.active(), "the ship's sprite loops");
    let why: Cow<'static, str> = "the roster is not recorded".into();
    let mut painted = 0;
    loop {
        painted += 1;
        let (_, asks, _) = view.paint_bases(&[(base, Err(&why))]);
        if !asks {
            break;
        }
        assert!(painted < 60, "a static map stops asking for frames");
    }
    assert!(view.windows.items_withheld.why(&ship).is_some(), "the ship's draw is withheld");
}

/// The naval windows' own admissions (combats, bases, results), not the pools': an icon kept off
/// screen asks for frames only while its province is dirty out of view; an icon admitted on screen
/// never asks by admission.
#[test]
fn a_naval_icon_kept_off_screen_asks_only_while_its_province_will_be_cleaned() {
    for (off_screen, cleans, asks) in [(false, false, false), (false, true, false), (true, true, true), (true, false, false)] {
        assert_eq!(keeps_asking(off_screen, cleans), asks, "off screen {off_screen}, cleans {cleans}");
    }
}
use super::*;
use std::collections::BTreeSet;
use viewer_foundation::map::gui_resources::contains;
use viewer_foundation::map::icon_windows::{GUI_DELTA_CLAMP, visible_layout};
use viewer_foundation::map::native_gui::PartKind;
use viewer_foundation::map::native_gui::lifecycle::Call;
use viewer_foundation::map::province_schedule::Update as ScheduleUpdate;
#[cfg(test)]
use viewer_foundation::map::test_support::part as test_support_part;
use viewer_foundation::map::test_support::{Fixture, VIEWPORT};

/// The four naval map-icon windows in a mod's own form: a three-frame hull on the item, a
/// three-frame icon on the convoy addition, a combat icon with a tile grid and an addition
/// grid, both icon windows fading.
const GUI: &str = r#"guiTypes = {
    containerWindowType = { name = "unit_counter_navy" size = { width = 20 height = 10 }
        iconType = { name = "type" spriteType = "GFX_hull" } }
    containerWindowType = { name = "GFX_mapicon_tiny_unit_counter"
        size = { width = 10 height = 10 }
        iconType = { name = "icon" spriteType = "GFX_tiny" } }
    containerWindowType = { name = "naval_combat_mapicon" size = { width = 40 height = 60 }
        fade_type = linear fade_time = 100
        buttonType = { name = "bg_btn" quadTextureSprite = "GFX_bg" }
        gridboxtype = { name = "attacker_grid" position = { x = 0 y = 0 }
            size = { width = 20 height = 40 } slotsize = { width = 20 height = 10 }
            format = "UPPER_LEFT" max_slots_horizontal = 1 }
        gridboxtype = { name = "attacker_adds" position = { x = 20 y = 0 }
            size = { width = 10 height = 40 } slotsize = { width = 10 height = 10 }
            format = "UPPER_LEFT" max_slots_horizontal = 1 } }
    containerWindowType = { name = "naval_base_mapicon" size = { width = 32 height = 32 }
        fade_type = linear fade_time = 100
        buttonType = { name = "bg_btn" quadTextureSprite = "GFX_bg" }
        buttonType = { name = "bg_strat_btn" quadTextureSprite = "GFX_bg" hide = yes }
        instantTextBoxType = { name = "txt_capacity" hide = yes }
        gridboxtype = { name = "box_ships" position = { x = 0 y = 32 }
            size = { width = 20 height = 40 } slotsize = { width = 20 height = 10 }
            format = "UPPER_LEFT" max_slots_horizontal = 1 } } }"#;

/// The sprites [`GUI`] draws.
fn fixture() -> Fixture {
    Fixture::new(
        &[
            ("GFX_hull", 3, [60, 10]),
            ("GFX_tiny", 3, [30, 10]),
            ("GFX_bg", 1, [40, 40]),
        ],
        &[],
    )
}

#[test]
fn naval_marker_hovers_cover_progress_and_visible_hq_frame_is_clickable() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let gui = GUI.replacen(
        r#"buttonType = { name = "bg_btn" quadTextureSprite = "GFX_bg" }"#,
        r#"buttonType = { name = "bg_btn" quadTextureSprite = "GFX_bg" }
        buttonType = { name = "frame_btn" position = { x = 50 y = 5 }
            quadTextureSprite = "GFX_frame" }
        iconType = { name = "progress" position = { x = 60 y = 10 }
            spriteType = "GFX_progress" }
        iconType = { name = "progress_small" position = { x = 60 y = 30 }
            spriteType = "GFX_progress" hide = yes }"#,
        1,
    );
    let native = NativeWindows::read(gui.as_bytes(), VIEWPORT).unwrap();
    let fixture = Fixture::new(
        &[
            ("GFX_bg", 1, [40, 40]),
            ("GFX_frame", 1, [50, 20]),
            ("GFX_progress", 1, [30, 10]),
        ],
        &[],
    );
    let mut windows = Windows::default();
    windows.build(&native, VIEWPORT, &fixture).unwrap();
    let window = native.naval_combat();
    let mut pool = windows.combats.take().unwrap();
    let slot = pool.request((3, 19)).unwrap();
    pool.update(window, |_, _| Admission::Inside([300, 100]))
        .unwrap();
    let instance = pool.get(slot).unwrap();
    instance
        .tick(window, &fixture, Tick { time: 1.0, dt: 1.0 })
        .unwrap();
    let layout = instance
        .layout_with_regions(window, &fixture, None, true)
        .unwrap()
        .unwrap();
    let footprints =
        naval_markers::combat_footprints(window, &layout, [0, 0], &fixture, &mut BTreeMap::new());
    let balance_point = [65., 15.];
    assert!(
        naval_markers::background_footprint(window, &layout)
            .is_some_and(|rect| !contains(rect, balance_point))
    );
    assert!(
        footprints.iter().any(|(hit, clickable)| *clickable
            && hit.supported
            && contains(hit.rect, balance_point))
    );
    assert!(
        footprints.iter().any(|(hit, clickable)| !*clickable
            && hit.supported
            && contains(hit.rect, balance_point))
    );
    assert_eq!(footprints.len(), 3); // Hidden small bar has no observer hit.
    let frame = window.find(native::Kind::Button, "frame_btn").unwrap();
    instance.hide(window, frame).unwrap();
    let layout = instance
        .layout_with_regions(window, &fixture, None, true)
        .unwrap()
        .unwrap();
    let footprints =
        naval_markers::combat_footprints(window, &layout, [0, 0], &fixture, &mut BTreeMap::new());
    assert!(
        footprints
            .iter()
            .any(|(hit, _)| hit.supported && contains(hit.rect, balance_point))
    );
    assert!(
        !footprints.iter().any(|(hit, clickable)| *clickable
            && hit.supported
            && contains(hit.rect, balance_point))
    );
}

fn sea_plate(local: [i32; 2], count: usize, screen: [i32; 2]) -> land::AtSeaPlate {
    land::AtSeaPlate {
        member: 0,
        ordinal: 0,
        local,
        column: Arc::new(land::ColumnPlacement {
            hovered: false,
            priority: 10,
            anchor: None,
            identity: (Level::None, 0, 1, 2, 0),
            count,
            screen,
            grid: [0; 2],
            holder: native_gui::Holder {
                path: "units_box".into(),
                child: None,
                content: [20, 10],
                origin: [0.0; 2],
                scale: 1.0,
                hit_scales: None,
                logical: [0; 2],
                clip: None,
                opacity: 1.0,
                before: 0,
                position: None,
            },
        }),
    }
}

#[test]
fn at_sea_movement_keeps_the_viewport_parent_and_local_position() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let gui = GUI.replacen(
        r#"name = "unit_counter_navy" size = { width = 20 height = 10 }"#,
        r#"name = "unit_counter_navy" size = { width = 20 height = 10 }
            moveable = yes show_animation_type = linear show_animation_time = 1000
            show_position = { x = 50% y = 25% }
            hide_animation_type = linear hide_animation_time = 1000
            hide_position = { x = 75% y = 50% }"#,
        1,
    );
    let native = NativeWindows::read(gui.as_bytes(), VIEWPORT).unwrap();
    let fixture = fixture();
    let navy = native.navy();
    let mut windows = Windows::default();
    windows.build(&native, VIEWPORT, &fixture).unwrap();
    let update = gui_windows::Update::default();
    let mut plate = sea_plate([20, 10], 1, [100, 200]);
    let column = Arc::make_mut(&mut plate.column);
    column.holder.origin[1] = -31.0;
    column.holder.logical[1] = -31;
    column.grid = [2, 3];
    counter_window(&mut windows, navy, 7, &plate, false, &update, &fixture).unwrap();
    windows
        .counters
        .get_mut(&7)
        .unwrap()
        .instance
        .tick(navy, &fixture, Tick { time: 0.5, dt: 0.5 })
        .unwrap();
    assert_eq!(windows.counters[&7].instance.position(), [490, 140]);
    windows
        .columns
        .insert(plate.column.identity, plate.column.count);
    // Camera translation moves only the graphical parent, including BlackICE's grid offset.
    Arc::make_mut(&mut plate.column).screen = [300, 400];
    counter_window(&mut windows, navy, 7, &plate, true, &update, &fixture).unwrap();
    let instance = &windows.counters[&7].instance;
    assert_eq!(instance.position(), [490, 140]);
    let layout = instance.layout(navy, &fixture, None).unwrap().unwrap();
    let place = [0, 1].map(|axis| plate.column.grid[axis] + instance.position()[axis]);
    assert_eq!(
        shown(&plate.column.holder.place(place, &layout)[0]).1,
        [492.0, 112.0]
    );
    // A trailing item changes the used count and re-places even an unchanged slot.
    Arc::make_mut(&mut plate.column).count = 2;
    counter_window(&mut windows, navy, 7, &plate, true, &update, &fixture).unwrap();
    let instance = &mut windows.counters.get_mut(&7).unwrap().instance;
    assert_eq!(instance.position(), [20, 10]);
    instance.hide(navy, navy).unwrap();
    instance
        .tick(navy, &fixture, Tick { time: 1.0, dt: 0.5 })
        .unwrap();
    assert_eq!(instance.position(), [730, 275]);
    assert!(windows.counters[&7].withheld.why(&()).is_none());
}

#[test]
fn at_sea_movement_follows_hidden_column_and_visible_append_order() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let gui = GUI.replacen(
        r#"name = "unit_counter_navy" size = { width = 20 height = 10 }"#,
        r#"name = "unit_counter_navy" size = { width = 20 height = 10 }
            moveable = yes show_animation_type = linear show_animation_time = 1000
            show_position = { x = 100 y = 60 }"#,
        1,
    );
    let native = NativeWindows::read(gui.as_bytes(), VIEWPORT).unwrap();
    let fixture = fixture();
    let navy = native.navy();
    let mut windows = Windows::default();
    windows.build(&native, VIEWPORT, &fixture).unwrap();
    let plate = sea_plate([20, 10], 1, [0; 2]);
    for (source, visible, expected) in [(7, false, [60, 35]), (8, true, [50, 30])] {
        counter_window(
            &mut windows,
            navy,
            source,
            &plate,
            visible,
            &gui_windows::Update::default(),
            &fixture,
        )
        .unwrap();
        let instance = &mut windows.counters.get_mut(&source).unwrap().instance;
        instance
            .tick(navy, &fixture, Tick { time: 0.5, dt: 0.5 })
            .unwrap();
        assert_eq!(instance.position(), expected);
    }
}

#[test]
fn at_sea_movement_places_a_returning_item_from_the_columns_previous_count() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut view = View::new();
    let native = view.native.clone();
    let navy = native.navy();
    let update = gui_windows::Update::default();
    let mut plate = sea_plate([0, 20], 2, [0; 2]);
    counter_window(
        &mut view.windows,
        navy,
        7,
        &plate,
        false,
        &update,
        &view.fixture,
    )
    .unwrap();
    assert_eq!(view.windows.counters[&7].instance.position(), [0, 20]);
    // Another plate keeps the column alive while this item is detached.
    view.windows.columns.insert(plate.column.identity, 1);
    view.windows.held_counters.clear();
    plate.local = [0, 40];
    counter_window(
        &mut view.windows,
        navy,
        7,
        &plate,
        true,
        &update,
        &view.fixture,
    )
    .unwrap();
    assert_eq!(view.windows.counters[&7].instance.position(), [0, 40]);
}

/// One map view's windows over [`GUI`].
struct View {
    native: NativeWindows,
    windows: Windows,
    fixture: Fixture,
    /// The view's graphics clock.
    time: f32,
}

/// What one drawn item shows: its zero-based frame and its top-left.
type Shown = (u32, [f32; 2]);

/// One grid item's update, or the operand it lacks.
type Update = Result<gui_windows::Update, Cow<'static, str>>;

impl View {
    fn new() -> Self {
        Self::with_gui(GUI)
    }

    /// A view over `gui`, [`GUI`] with a template changed.
    fn with_gui(gui: &str) -> Self {
        let native = NativeWindows::read(gui.as_bytes(), VIEWPORT).unwrap();
        let mut windows = Windows::default();
        let fixture = fixture();
        windows.build(&native, VIEWPORT, &fixture).unwrap();
        Self {
            native,
            windows,
            fixture,
            time: 0.0,
        }
    }

    fn base_window(&self) -> &native::Element {
        self.native.naval_base().unwrap()
    }

    fn grid_path(window: &native::Element, name: &str) -> String {
        window.find(native::Kind::Grid, name).unwrap().path.clone()
    }

    /// `province`'s base icon requested and passed inside the viewport: its slot.
    fn base(&mut self, province: u32) -> SlotId {
        let base = self.native.naval_base().unwrap();
        let pool = self.windows.bases.as_mut().unwrap();
        let slot = pool.request(province).unwrap();
        pool.update(base, |_, _| Admission::Inside([100, 100]))
            .unwrap();
        slot
    }

    /// `owner`'s combat icon requested and passed inside the viewport: its slot.
    fn combat(&mut self, owner: CombatOwner) -> SlotId {
        let window = self.native.naval_combat();
        let pool = self.windows.combats.as_mut().unwrap();
        let slot = pool.request(owner).unwrap();
        pool.update(window, |_, _| Admission::Inside([300, 100]))
            .unwrap();
        slot
    }

    /// The base in `slot` updated with an undecided port roster, by `why`.
    fn undecided_ports(&mut self, slot: SlotId, why: &str) {
        let base = self.native.naval_base().unwrap();
        let mut pool = self.windows.bases.take().unwrap();
        let instance = pool.get(slot).unwrap();
        base_items(
            &mut self.windows,
            instance,
            base,
            slot,
            Err(&Cow::Owned(why.to_owned())),
            &self.fixture,
            &mut CounterReport::default(),
        )
        .unwrap();
        self.windows.bases = Some(pool);
    }

    /// The base in `slot` updated with this hour's port `items`: a gap is an item whose
    /// plate colour the hour lacks, every other write made. How many its grid left out.
    fn ports(&mut self, slot: SlotId, items: &[Update]) -> usize {
        let base = self.native.naval_base().unwrap();
        let items: Vec<gui_windows::Update> = items
            .iter()
            .map(|item| match item {
                Ok(update) => update.clone(),
                Err(why) => gui_windows::Update {
                    unknown: vec![(gui_windows::PLATE, why.to_string())],
                    ..gui_windows::Update::default()
                },
            })
            .collect();
        let mut pool = self.windows.bases.take().unwrap();
        let instance = pool.get(slot).unwrap();
        let mut report = CounterReport::default();
        let left_out = fill_ports(
            &mut self.windows,
            instance,
            base,
            slot,
            &items,
            &self.fixture,
            &mut report,
        )
        .unwrap();
        self.windows.bases = Some(pool);
        assert_eq!(report.navy_counter_gaps.get(naval::PORT_GRID_FULL).copied().unwrap_or(0), left_out);
        left_out
    }

    /// The combat icon in `slot` updated with `update`, and its grids refilled with `fills`
    /// when it rotates.
    fn update(&mut self, slot: SlotId, update: gui_windows::Update, fills: Option<&[GridFill]>) {
        let window = self.native.naval_combat();
        let mut pool = self.windows.combats.take().unwrap();
        let mut report = CounterReport::default();
        update_combat(
            &mut self.windows,
            &mut pool,
            window,
            slot,
            &update,
            fills,
            &self.fixture,
            &mut report,
        )
        .unwrap();
        self.windows.combats = Some(pool);
    }

    /// One painted frame: the base in `slot` updated with port `items` as a presented frame updates
    /// an admitted base ([`update_base`]), then the view's tick. Whether the view asks for the
    /// next frame.
    fn paint(&mut self, slot: SlotId, items: &[gui_windows::Update]) -> bool {
        self.paint_bases(&[(slot, Ok(items))]).1
    }

    /// One painted frame of `bases`, each with its port roster, updated as a presented frame
    /// updates its admitted bases ([`update_or_withhold`]), then the view's tick: which stood,
    /// whether the view asks for the next frame, and what the updates counted.
    fn paint_bases(
        &mut self,
        bases: &[(SlotId, Result<&[gui_windows::Update], &Cow<'static, str>>)],
    ) -> (Vec<bool>, bool, CounterReport) {
        let mut pool = self.windows.bases.take().unwrap();
        let mut report = CounterReport::default();
        let stood = bases
            .iter()
            .map(|&(slot, items)| {
                update_or_withhold(
                    &mut self.windows,
                    &mut pool,
                    self.native.naval_base().unwrap(),
                    slot,
                    BaseInputs { province: 1, glyph: None, scale: 0.0, strategic: false, items, art: 0, countries: None },
                    &self.fixture,
                    &mut report,
                )
            })
            .collect();
        self.windows.bases = Some(pool);
        self.tick(1.0 / 60.0);
        (stood, self.windows.visible, report)
    }

    /// Frames painted while the view asks for them, at most 600: how many.
    fn paint_until_settled(&mut self, slot: SlotId, items: &[gui_windows::Update]) -> usize {
        for painted in 1..=600 {
            if !self.paint(slot, items) {
                return painted;
            }
        }
        panic!("the view never stopped asking for frames");
    }

    fn tick(&mut self, dt: f32) {
        let mut report = CounterReport::default();
        self.time += dt;
        let time = self.time;
        tick(
            &mut self.windows,
            &self.native,
            None,
            &self.fixture,
            Tick { time, dt },
            &mut report,
        );
        assert!(report.naval_window_notes.is_empty(), "{report:?}");
    }

    /// The port items the base in `slot` of the visible list draws, and what the draw
    /// counted.
    fn port_items(&self, slot: SlotId) -> (Vec<Shown>, CounterReport) {
        let mut report = CounterReport::default();
        let pool = self.windows.bases.as_ref().unwrap();
        let base = pool.visible().find(|base| base.slot == slot).unwrap();
        let grid = self
            .base_window()
            .find(native::Kind::Grid, "box_ships")
            .unwrap();
        let shown = port_parts(
            &self.windows,
            &base,
            self.base_window(),
            None,
            grid,
            &self.fixture,
            VIEWPORT,
            &mut report,
        )
        .map(|(holder, items)| {
            items
                .iter()
                .flat_map(|item| holder.place(item.place, &item.layout))
                .map(|part| shown(&part))
                .collect()
        })
        .unwrap_or_default();
        (shown, report)
    }

    /// What the combat icon in `slot` of the visible list, showing `combat`, draws in each of
    /// its grids (`None` when it draws nothing), and what the draw counted.
    fn combat_items(&self, slot: SlotId, combat: u64) -> (Option<Vec<Vec<Shown>>>, CounterReport) {
        let mut report = CounterReport::default();
        let pool = self.windows.combats.as_ref().unwrap();
        let icon = pool.visible().find(|icon| icon.slot == slot).unwrap();
        assert_eq!(icon.owner.1, combat);
        let drawn = combat_parts(
            &self.windows,
            &icon,
            false,
            self.native.naval_combat(),
            &self.fixture,
            VIEWPORT,
            &mut report,
        )
        .map(|(_, items)| {
            items
                .iter()
                .map(|parts| parts.iter().map(shown).collect())
                .collect()
        });
        (drawn, report)
    }

    /// The combat tiles and convoy additions of one rotation: `tiles` on the attacker's
    /// grid, `adds` on its addition grid.
    fn fills(&self, tiles: Vec<Update>, adds: Vec<Update>) -> Vec<GridFill> {
        let window = self.native.naval_combat();
        vec![
            GridFill {
                grid: Self::grid_path(window, "attacker_grid"),
                side: Some(0),
                items: tiles,
            },
            GridFill {
                grid: Self::grid_path(window, "attacker_adds"),
                side: None,
                items: adds,
            },
        ]
    }

    /// A naval item's update: its hull on one-based `frame`, its plate's colour written as
    /// every update writes it.
    fn hull(&self, frame: i32) -> Update {
        let mut writes = gui_windows::Writes::new(self.native.navy());
        writes.frame(native::Kind::Icon, "type", frame);
        writes.wrote(gui_windows::PLATE);
        Ok(writes.update())
    }

    /// A naval item's full-intel update: [`Self::hull`] with its supply and hull sprite
    /// written ([`gui_windows::MISSION_ITEM_KEPT`]).
    fn full_hull(&self, frame: i32) -> Update {
        let mut update = self.hull(frame)?;
        update.wrote.extend(gui_windows::MISSION_ITEM_KEPT);
        Ok(update)
    }

    /// The production rotation with an engaged or retained background frame.
    fn engaged_rotation(&self, engaged: bool) -> gui_windows::Update {
        use gui_windows::{CombatProgress, CombatState, Engaged, Rotation};
        let state = CombatState {
            rotation: Some(Rotation {
                engaged: engaged.then_some(Engaged {
                    variant: 0,
                    progress: Ok(CombatProgress {
                        small: true,
                        progress: 0,
                        flip: false,
                    }),
                }),
            }),
            hq: Ok(false),
        };
        let mut update =
            gui_windows::combat_calls(self.native.naval_combat(), &state, &self.fixture);
        update.wrote.extend([GRIDS, PLACE]);
        update
    }

    /// A convoy addition's update: its icon on one-based `frame`.
    fn convoy(&self, frame: i32) -> Update {
        let mut writes = gui_windows::Writes::new(self.native.tiny().unwrap());
        writes.frame(native::Kind::Icon, "icon", frame);
        Ok(writes.update())
    }

    /// Whether each item of `keys` is shown.
    fn shown(&self, keys: &[ItemKey]) -> Vec<bool> {
        keys.iter()
            .map(|key| self.windows.items[key].instance.drawn())
            .collect()
    }
}

/// A drawn sprite part's zero-based frame and top-left.
fn shown(part: &native_gui::Part) -> Shown {
    match &part.kind {
        PartKind::Sprite {
            uv,
            offsets,
            geometry,
            ..
        } => (
            (offsets[0] / uv[2]).round() as u32,
            [geometry.rect[0], geometry.rect[1]],
        ),
        other => panic!("{} draws {other:?}", part.path),
    }
}

fn frames(shown: &[Shown]) -> Vec<u32> {
    shown.iter().map(|(frame, _)| *frame).collect()
}

/// A rotation the combat window takes whole: it writes the rotation, the HQ frame and the
/// icon's place.
fn rotation() -> gui_windows::Update {
    gui_windows::Update {
        wrote: vec![BG_FRAME, PROGRESS, GRIDS, FRAME_BTN, PLACE],
        ..gui_windows::Update::default()
    }
}

/// An update without a rotation (the game writing only `frame_btn`: an icon the hour does
/// not place again, or one whose province is not its combat's).
fn hq_only() -> gui_windows::Update {
    gui_windows::Update {
        wrote: vec![FRAME_BTN],
        ..gui_windows::Update::default()
    }
}

/// A rotation of the combat window whose flipped bar is not measured, by `why`
/// ([`gui_windows::combat_calls`]).
fn refused(why: &str) -> gui_windows::Update {
    gui_windows::Update {
        wrote: vec![BG_FRAME, GRIDS, FRAME_BTN, PLACE],
        unknown: vec![(
            PROGRESS,
            format!("icon omitted: {NAVAL_COMBAT_WINDOW} does not update: {why}"),
        )],
        ..gui_windows::Update::default()
    }
}

fn gap(why: &'static str) -> Update {
    Err(Cow::Borrowed(why))
}

/// Erased naval bases retain fade-out lifecycle state and hide their port
/// items when that fade ends.
#[test]
fn a_combat_and_a_base_of_one_slot_number_keep_their_own_grids() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut view = View::new();
    let base = view.base(1);
    let combat = view.combat((1, 9));
    assert_eq!(base, combat, "both pools' first slot");
    view.ports(base, &[view.hull(2), view.hull(3)]);
    let fills = view.fills(vec![view.hull(1)], Vec::new());
    view.update(combat, rotation(), Some(&fills));
    for _ in 0..5 {
        view.tick(0.05);
    }
    let box_ships = View::grid_path(view.base_window(), "box_ships");
    let tile_grid = View::grid_path(view.native.naval_combat(), "attacker_grid");
    let ports = view.windows.grids[&(IconSlot::Base(base), box_ships)].clone();
    let tiles = view.windows.grids[&(IconSlot::Combat(combat), tile_grid)].clone();
    assert_eq!((ports.len(), tiles.len()), (2, 1));
    assert_eq!(frames(&view.port_items(base).0), [1, 2]);
    assert_eq!(frames(&view.combat_items(combat, 9).0.unwrap()[0]), [0]);
    // The base erased while opaque: its fade begins and nothing is hidden yet.
    let window = view.native.naval_base().unwrap();
    let held = view
        .windows
        .bases
        .as_mut()
        .unwrap()
        .erase(window, &1)
        .unwrap();
    assert!(held.is_empty());
    assert_eq!(view.shown(&ports), [true, true]);
    // The combat icon of the same number goes on rotating through the fade.
    for _ in 0..5 {
        let fills = view.fills(vec![view.hull(1)], Vec::new());
        view.update(combat, rotation(), Some(&fills));
        view.tick(0.05);
    }
    assert_eq!(view.shown(&ports), [false, false]);
    assert_eq!(view.shown(&tiles), [true]);
}

/// A refused pooled update retains the window but withholds its draw from
/// the first affected frame, with a named reason.
#[test]
fn a_refused_port_item_keeps_its_slot_and_draws_nothing() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut view = View::new();
    let base = view.base(1);
    let step = |view: &mut View, items: &[Update]| {
        view.ports(base, items);
        view.tick(0.0);
        view.port_items(base)
    };
    // A first-frame failure: the refused item is constructed and holds slot 0.
    let items = [gap("a first gap"), view.hull(2)];
    let (first, report) = step(&mut view, &items);
    assert_eq!(
        report.navy_counter_gaps["unit_counter_navy is not drawn: a first gap"],
        1
    );
    assert_eq!(first.len(), 1);
    let items = [view.hull(2), view.hull(3)];
    let (valid, report) = step(&mut view, &items);
    assert!(report.navy_counter_gaps.is_empty(), "{report:?}");
    assert_eq!(frames(&valid), [1, 2]);
    // The first frame drew the second item at its own slot.
    assert_eq!(first[0], (1, valid[1].1));
    // Missing input: the second item draws nothing; the first stays where it was.
    let items = [view.hull(2), gap("a missing operand")];
    let (missing, report) = step(&mut view, &items);
    assert_eq!(
        report.navy_counter_gaps["unit_counter_navy is not drawn: a missing operand"],
        1
    );
    assert_eq!(missing, [valid[0]]);
    // Valid again: the second item shows this update's hull, not its last shown one.
    let items = [view.hull(2), view.hull(1)];
    let (again, report) = step(&mut view, &items);
    assert!(report.navy_counter_gaps.is_empty(), "{report:?}");
    assert_eq!(again, [valid[0], (0, valid[1].1)]);
}

/// Combat tiles and convoy additions go the same way: a refused one keeps its slot and draws
/// nothing, counted under why, from its first frame, and draws again what a later update
/// wrote.
#[test]
fn refused_combat_tiles_and_convoy_additions_keep_their_slots_and_draw_nothing() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut view = View::new();
    let combat = view.combat((4, 9));
    let step = |view: &mut View, tiles: Vec<Update>, adds: Vec<Update>| {
        let fills = view.fills(tiles, adds);
        view.update(combat, rotation(), Some(&fills));
        view.tick(0.0);
        let (drawn, report) = view.combat_items(combat, 9);
        (drawn.unwrap(), report)
    };
    let tiles = vec![gap("a tile gap"), view.hull(2)];
    let (first, report) = step(&mut view, tiles, vec![gap("a convoy gap")]);
    assert_eq!(report.navy_counter_gaps["a tile gap"], 1);
    assert_eq!(report.naval_grid_gaps["a convoy gap"], 1);
    assert_eq!(first[0].len(), 1);
    assert!(first[1].is_empty());
    let (tiles, adds) = (vec![view.hull(2), view.hull(3)], vec![view.convoy(2)]);
    let (valid, report) = step(&mut view, tiles, adds);
    assert!(report.navy_counter_gaps.is_empty(), "{report:?}");
    assert!(report.naval_grid_gaps.is_empty(), "{report:?}");
    assert_eq!(frames(&valid[0]), [1, 2]);
    assert_eq!(frames(&valid[1]), [1]);
    assert_eq!(first[0][0], (1, valid[0][1].1));
    let tiles = vec![view.hull(2), gap("a missing operand")];
    let (missing, report) = step(&mut view, tiles, vec![gap("a missing tint")]);
    assert_eq!(report.navy_counter_gaps["a missing operand"], 1);
    assert_eq!(report.naval_grid_gaps["a missing tint"], 1);
    assert_eq!(missing, [vec![valid[0][0]], Vec::new()]);
    let (tiles, adds) = (vec![view.hull(2), view.hull(1)], vec![view.convoy(3)]);
    let (again, _) = step(&mut view, tiles, adds);
    assert_eq!(
        again,
        [
            vec![valid[0][0], (0, valid[0][1].1)],
            vec![(2, valid[1][0].1)]
        ]
    );
}

/// A combat icon whose update is refused draws nothing, counted with its combat, from its
/// first frame; an update without a rotation keeps the unknown the refused one left, and the
/// next rotation, which writes both bars again, draws it.
#[test]
fn a_refused_combat_update_withholds_the_icon_until_a_rotation() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut view = View::new();
    let combat = view.combat((4, 9));
    view.update(combat, refused("a flipped bar is not measured"), Some(&[]));
    view.tick(0.0);
    let (drawn, report) = view.combat_items(combat, 9);
    assert!(drawn.is_none());
    assert_eq!(report.unavailable_naval_combats, 1);
    assert_eq!(
        report.combat_gap_lines().collect::<Vec<_>>(),
        [
            "Naval combat icons: 1 undecided: icon omitted: naval_combat_mapicon does not \
          update: a flipped bar is not measured; combats 9"
        ]
    );
    view.update(combat, hq_only(), None);
    view.tick(0.0);
    assert!(view.combat_items(combat, 9).0.is_none());
    view.update(combat, rotation(), Some(&[]));
    view.tick(0.0);
    let (drawn, report) = view.combat_items(combat, 9);
    assert!(drawn.is_some());
    assert_eq!(report.unavailable_naval_combats, 0);
}

/// Only visible live pool icons attach for painting. Erased icons may still
/// fade and tick their retained items without drawing.
#[test]
fn an_erased_icon_draws_nothing_while_its_hide_fade_runs() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut view = View::new();
    let base = view.base(1);
    let combat = view.combat((1, 9));
    let items = [view.hull(2), view.hull(3)];
    view.ports(base, &items);
    let fills = view.fills(vec![view.hull(1)], Vec::new());
    view.update(combat, rotation(), Some(&fills));
    for _ in 0..5 {
        view.tick(0.05);
    }
    let visible = |view: &View| {
        (
            view.windows.combats.as_ref().unwrap().visible().count(),
            view.windows.bases.as_ref().unwrap().visible().count(),
        )
    };
    assert_eq!(visible(&view), (1, 1));
    let box_ships = View::grid_path(view.base_window(), "box_ships");
    let tile_grid = View::grid_path(view.native.naval_combat(), "attacker_grid");
    let ports = view.windows.grids[&(IconSlot::Base(base), box_ships)].clone();
    let tiles = view.windows.grids[&(IconSlot::Combat(combat), tile_grid)].clone();
    let combat_window = view.native.naval_combat();
    let base_window = view.native.naval_base().unwrap();
    let pools = &mut view.windows;
    let held = pools
        .combats
        .as_mut()
        .unwrap()
        .erase(combat_window, &(1, 9))
        .unwrap();
    assert!(held.is_empty());
    let held = pools
        .bases
        .as_mut()
        .unwrap()
        .erase(base_window, &1)
        .unwrap();
    assert!(held.is_empty());
    // Mid-fade: both windows still fade out and their items keep their visible bits, but
    // no icon is in the visible list, so nothing of either is drawn.
    view.tick(0.01);
    assert!(view.windows.combats.as_ref().unwrap().active());
    assert!(view.windows.bases.as_ref().unwrap().active());
    assert_eq!(view.shown(&ports), [true, true]);
    assert_eq!(view.shown(&tiles), [true]);
    assert_eq!(visible(&view), (0, 0));
    // The fades end: each window's Hide reaches its grids' items.
    for _ in 0..5 {
        view.tick(0.05);
    }
    assert_eq!(view.shown(&ports), [false, false]);
    assert_eq!(view.shown(&tiles), [false]);
    assert_eq!(visible(&view), (0, 0));
}

/// Advance every shown frame-animated sprite before GUI transitions. When an
/// erased base's fade ends, the port item its `box_ships` hides has already had
/// that frame's clock. Showing it again retains the clock, so it reaches the
/// same point as an unbroken clock. Ticking the whole instance in pool/hides/items
/// order would hide the item before advancing its clock and lose the frame.
#[test]
fn an_item_hidden_by_its_icons_ended_fade_keeps_that_frames_clock() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut view = View::new();
    // Three frames of 0.125 s: the loop is 0.375 s.
    view.fixture.animated.insert(b"GFX_hull".to_vec(), 0.125);
    let base = view.base(1);
    view.ports(base, &[Ok(gui_windows::Update::default())]);
    for _ in 0..4 {
        view.tick(0.0625);
    }
    let box_ships = View::grid_path(view.base_window(), "box_ships");
    let ports = view.windows.grids[&(IconSlot::Base(base), box_ships)].clone();
    // 0.25 s in: the third frame.
    assert_eq!(frames(&view.port_items(base).0), [2]);
    // Erased while opaque: its 0.1 s fade ends inside the next frame, which hides the item.
    let window = view.native.naval_base().unwrap();
    let held = view
        .windows
        .bases
        .as_mut()
        .unwrap()
        .erase(window, &1)
        .unwrap();
    assert!(held.is_empty());
    view.tick(0.1875);
    assert_eq!(view.shown(&ports), [false]);
    // A 0.4375-second clock wraps to 0.0625 seconds, selecting the first frame.
    let navy = view.native.navy().clone();
    let item = view.windows.items.get_mut(&ports[0]).unwrap();
    item.instance.gui_show(&navy, &navy).unwrap();
    view.tick(0.0);
    assert_eq!(view.shown(&ports), [true]);
    let item = &view.windows.items[&ports[0]];
    let layout = item
        .instance
        .layout(&navy, &view.fixture, None)
        .unwrap()
        .unwrap();
    assert_eq!(
        frames(&layout.parts.iter().map(shown).collect::<Vec<_>>()),
        [0]
    );
}

/// A clock whose period subtraction cannot change its value would hang.
/// Refuse the affected item by name while keeping its pooled history.
#[test]
fn a_hanging_frame_animation_is_counted_where_its_item_draws() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut view = View::new();
    view.fixture.animated.insert(b"GFX_hull".to_vec(), 0.01);
    let base = view.base(1);
    view.ports(base, &[Ok(gui_windows::Update::default())]);
    view.tick(0.0);
    assert_eq!(view.port_items(base).0.len(), 1);
    view.tick(1.0e9);
    let (items, report) = view.port_items(base);
    assert!(items.is_empty());
    let lines: Vec<String> = report.warnings().collect();
    assert!(
        lines.iter().any(
            |line| line.starts_with("Naval counters: 1 not drawn: ") && line.contains("(hang)")
        ),
        "{lines:?}"
    );
}

/// Grid updates read every item they append and place. Unknown item geometry
/// refuses the dependent update.
#[test]
fn items_appended_while_the_grid_read_is_refused_draw_nothing_until_it_is_read() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut view = View::new();
    let base = view.base(1);
    let items = [view.hull(2), view.hull(3)];
    view.ports(base, &items);
    view.tick(0.0);
    assert_eq!(frames(&view.port_items(base).0), [1, 2]);
    // The update's read is refused (a sprite only the recording client's session may have).
    view.fixture = Fixture::new(&[("GFX_hull", 3, [60, 10])], &[]);
    view.fixture.session_sprites = true;
    view.ports(base, &items);
    // The draw lays the window out again.
    view.fixture = fixture();
    view.tick(0.0);
    let (drawn, report) = view.port_items(base);
    assert!(drawn.is_empty());
    let unread: Vec<_> = report
        .navy_counter_gaps
        .iter()
        .filter(|(why, _)| why.contains("the grid it is appended to is not read"))
        .map(|(_, count)| *count)
        .collect();
    assert_eq!(unread, [2], "{report:?}");
    view.ports(base, &items);
    view.tick(0.0);
    let (drawn, report) = view.port_items(base);
    assert_eq!(frames(&drawn), [1, 2]);
    assert!(report.navy_counter_gaps.is_empty(), "{report:?}");
}

/// A call that does not apply is structural, the same on every update; what followed it is
/// never known to be written again, so the window stays refused though later updates succeed,
/// an item's and a combat icon's alike (a rotation included).
#[test]
fn a_call_that_does_not_apply_withholds_the_window_until_it_is_built_again() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut view = View::new();
    let combat = view.combat((4, 9));
    let broken = || gui_windows::Update::calls(vec![("not_an_element".to_owned(), Call::Show)]);
    let fills = view.fills(vec![Ok(broken()), view.hull(2)], Vec::new());
    view.update(combat, broken(), Some(&fills));
    view.tick(0.0);
    let (drawn, report) = view.combat_items(combat, 9);
    assert!(drawn.is_none());
    assert_eq!(report.unavailable_naval_combats, 1);
    let fills = view.fills(vec![view.hull(2), view.hull(3)], Vec::new());
    view.update(combat, rotation(), Some(&fills));
    view.tick(0.0);
    assert!(view.combat_items(combat, 9).0.is_none());
    // The tile whose update did not apply stays refused beside its neighbour.
    let tile_grid = View::grid_path(view.native.naval_combat(), "attacker_grid");
    let tiles = view.windows.grids[&(IconSlot::Combat(combat), tile_grid)].clone();
    assert!(view.windows.items_withheld.why(&tiles[0]).is_some());
    assert!(view.windows.items_withheld.why(&tiles[1]).is_none());
}

/// A base whose window the module refuses places none of its port items: counted once per
/// item it holds, on the diagnostics lines the window, the dump and the map details print
/// ([`CounterReport::warnings`]).
#[test]
fn a_refused_base_window_counts_its_port_items() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut view = View::new();
    let base = view.base(1);
    let items = [view.hull(2), view.hull(3)];
    view.ports(base, &items);
    view.tick(0.0);
    // The base's background names a sprite the content does not define and a text box
    // does: the lookup depends on the recording client's session.
    view.fixture = Fixture::new(&[("GFX_hull", 3, [60, 10])], &[]);
    view.fixture.session_sprites = true;
    let (items, report) = view.port_items(base);
    assert!(items.is_empty());
    // One line per refusal (the sprite, then the fallback texture), each counting both items.
    let lines: Vec<String> = report.warnings().collect();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(
        lines.iter().all(|line| line.starts_with(
            "Naval counters: 2 not drawn: port items are not placed: naval_base_mapicon is \
             not drawn: naval_base_mapicon.bg_btn (missing_input): "
        )),
        "{lines:?}"
    );
}
/// An icon the hour places but cannot decide (here an engaged rotating icon whose
/// `NavalCombatShips` census is not whole) keeps its identity in the plan
/// ([`CombatIcon::omitted`]), so the manager's update of its window is that omitted update,
/// never the retention of an icon the hour no longer places: the game
/// hides and flags both bars and clears the grids before it reads whether the combat is
/// engaged, so the old frame, progress and grids are not the game's. The window keeps its
/// slot and draws nothing, counted once (the preparation counted it with its cause), until a
/// decided rotation writes it again; an HQ-only update in between does not.
#[test]
fn an_omitted_icon_is_withheld_until_a_decided_rotation() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut view = View::new();
    let owner = (4, 9);
    let slot = view.combat(owner);
    let native = view.native.clone();
    let window = native.naval_combat();
    let pass = |icon: CombatIcon| CombatPass {
        icons: vec![icon],
        gated: BTreeSet::new(),
        live: BTreeSet::from([9]),
        live_undecided: None,
        hq: Ok(BTreeMap::from([(9, Ok(true))])),
        size: [40, 60],
    };
    let decided = |view: &View, frame: i32| CombatIcon {
        owner,
        place: Some(([0.0; 3], 0)),
        update: rotation(),
        rotates: true,
        grids: view.fills(vec![view.hull(frame)], Vec::new()),
        frame: Some(naval_markers::FrameSource::Engaged),
        omitted: false,
    };
    let present = |view: &mut View, pass: &CombatPass| {
        let (update, fills) = planned_update(pass, owner, window, &view.fixture);
        view.update(slot, update.into_owned(), fills);
        view.tick(0.0);
    };
    // Valid: the rotation draws its tile.
    let valid = pass(decided(&view, 2));
    present(&mut view, &valid);
    let (drawn, report) = view.combat_items(slot, 9);
    assert_eq!(frames(&drawn.unwrap()[0]), [1]);
    assert_eq!(report.unavailable_naval_combats, 0);
    // The member census goes missing: the plan still names the icon, with why.
    let cause = "icon omitted: convoy variant undecided: the NavalCombatShips table is \
                 truncated (3 rows unread)";
    let missing = pass(CombatIcon::omitted(owner, Some(([0.0; 3], 0)), true, cause));
    present(&mut view, &missing);
    assert_eq!(
        view.windows.combats.as_ref().unwrap().withheld(slot),
        Some(cause)
    );
    let (drawn, report) = view.combat_items(slot, 9);
    assert!(drawn.is_none());
    assert_eq!(
        report.combat_gap_lines().collect::<Vec<_>>(),
        [format!(
            "Naval combat icons: 1 undecided: {cause}; combats 9"
        )]
    );
    // The preparation counted it already: where it would draw, nothing is counted again.
    let pool = view.windows.combats.as_ref().unwrap();
    let icon = pool.visible().find(|icon| icon.slot == slot).unwrap();
    let mut report = CounterReport::default();
    assert!(
        combat_parts(
            &view.windows,
            &icon,
            true,
            window,
            &view.fixture,
            VIEWPORT,
            &mut report
        )
        .is_none()
    );
    assert_eq!(report.unavailable_naval_combats, 0);
    // The hour stops placing it: its HQ frame alone is written, the rotation stays unknown.
    let unplaced = CombatPass {
        icons: Vec::new(),
        ..pass(decided(&view, 2))
    };
    present(&mut view, &unplaced);
    assert!(view.combat_items(slot, 9).0.is_none());
    // Valid again: the decided rotation draws, but its reused tile, which the omitted hour
    // may have updated, waits for writes of the parts a mission item's update may
    // keep: a tile update that makes none of them leaves it withheld.
    let valid = pass(decided(&view, 3));
    present(&mut view, &valid);
    let (drawn, report) = view.combat_items(slot, 9);
    assert!(drawn.unwrap()[0].is_empty());
    assert_eq!(report.unavailable_naval_combats, 0);
    let mut full = pass(decided(&view, 3));
    full.icons[0].grids[0].items = vec![view.full_hull(3)];
    present(&mut view, &full);
    assert_eq!(frames(&view.combat_items(slot, 9).0.unwrap()[0]), [2]);
}

/// The HQ frame: the game writes `frame_btn` on every update of a live combat's
/// icon, so an HQ frame the hour cannot decide, for an icon it places or one it no longer
/// places, leaves [`gui_windows::FRAME_BTN`] unknown rather than keeping the old frame.
#[test]
fn an_undecided_hq_frame_withholds_the_icon_until_it_is_decided() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut view = View::new();
    let owner = (4, 9);
    let slot = view.combat(owner);
    let native = view.native.clone();
    let window = native.naval_combat();
    let unplaced = |hq| CombatPass {
        icons: Vec::new(),
        gated: BTreeSet::new(),
        live: BTreeSet::from([9]),
        live_undecided: None,
        hq,
        size: [40, 60],
    };
    let present = |view: &mut View, pass: &CombatPass| {
        let (update, _) = planned_update(pass, owner, window, &view.fixture);
        view.update(slot, update.into_owned(), None);
        view.tick(0.0);
        view.combat_items(slot, 9)
    };
    let (drawn, _) = present(&mut view, &unplaced(Ok(BTreeMap::from([(9, Ok(false))]))));
    assert!(drawn.is_some());
    let census = "the Ships table is truncated (2 rows unread)";
    let (drawn, report) = present(&mut view, &unplaced(Err(census.to_owned())));
    assert!(drawn.is_none());
    assert_eq!(
        report.combat_gap_lines().collect::<Vec<_>>(),
        [format!(
            "Naval combat icons: 1 undecided: icon omitted: HQ frame undecided: {census}; \
             combats 9"
        )]
    );
    let (drawn, _) = present(&mut view, &unplaced(Ok(BTreeMap::new())));
    assert!(drawn.is_none());
    let (drawn, _) = present(&mut view, &unplaced(Ok(BTreeMap::from([(9, Ok(true))]))));
    assert!(drawn.is_some());
}

/// The shipped naval windows and a mission item's window (a combat tile), built as its icon
/// updates it.
struct Synthetic {
    native: NativeWindows,
    fixture: Fixture,
    windows: Windows,
    key: ItemKey,
}

impl Synthetic {
    fn new() -> Self {
        let native = gui_windows::tests::windows();
        let fixture = gui_windows::tests::navy_fixture();
        let mut windows = Windows::default();
        windows.build(&native, VIEWPORT, &fixture).unwrap();
        // A combat tile: the same item window as a port item, updated the same way.
        let slot = windows.combats.as_mut().unwrap().request((1, 1)).unwrap();
        let key = ItemKey {
            icon: IconSlot::Combat(slot),
            pool: ItemPool::Tile(0),
            index: 0,
        };
        Self {
            native,
            fixture,
            windows,
            key,
        }
    }

    /// The item updated with the production calls for `flotilla` under `settings`, shown
    /// as its grid shows it and ticked: why it is withheld, and the zero-based frame its
    /// `low_supply` and the sprite its `type` draw, when they draw.
    fn item(
        &mut self,
        flotilla: &naval::Flotilla,
        settings: CounterSettings,
    ) -> (Option<String>, Option<u32>, Option<Vec<u8>>) {
        self.item_at(0, flotilla, settings)
    }

    /// [`Self::item`] for the item at `index` of the same pool.
    fn item_at(
        &mut self,
        index: usize,
        flotilla: &naval::Flotilla,
        settings: CounterSettings,
    ) -> (Option<String>, Option<u32>, Option<Vec<u8>>) {
        let key = ItemKey { index, ..self.key };
        let navy = self.native.navy();
        let (update, _) = gui_windows::tests::production(navy, flotilla, settings, &self.fixture);
        update_item(
            &mut self.windows,
            key,
            navy,
            false,
            Ok(&update),
            &self.fixture,
        )
        .unwrap();
        let item = self.windows.items.get_mut(&key).unwrap();
        item.instance.gui_show(navy, navy).unwrap();
        item.instance
            .tick(navy, &self.fixture, Tick { time: 0.0, dt: 0.0 })
            .unwrap();
        let layout = item
            .instance
            .layout(navy, &self.fixture, None)
            .unwrap()
            .unwrap();
        let hull = test_support_part(&layout, "type").map(|part| match &part.kind {
            native_gui::PartKind::Sprite { sprite, .. } => sprite.clone(),
            other => panic!("type draws {other:?}"),
        });
        (
            self.windows.items_withheld.why(&key).map(str::to_owned),
            test_support_part(&layout, "low_supply")
                .map(|_| gui_windows::tests::sprite_frame(&layout, "low_supply")),
            hull,
        )
    }

    /// Update the at-sea counter with the production calls and return its refusal.
    fn counter(&mut self, flotilla: &naval::Flotilla, settings: CounterSettings) -> Option<String> {
        let navy = self.native.navy();
        let (update, _) = gui_windows::tests::production(navy, flotilla, settings, &self.fixture);
        counter_window(
            &mut self.windows,
            navy,
            flotilla.source,
            &sea_plate([0; 2], 1, [0; 2]),
            false,
            &update,
            &self.fixture,
        )
        .unwrap();
        self.windows.counters[&flotilla.source]
            .withheld
            .why(&())
            .map(str::to_owned)
    }

    /// The item's combat icon updated as omitted for `cause`, its grids undecided
    /// ([`CombatIcon::omitted`], [`unknown_items`]).
    fn omit(&mut self, cause: &str) {
        let IconSlot::Combat(slot) = self.key.icon else {
            unreachable!("the synthetic item is a combat tile");
        };
        let omitted = CombatIcon::omitted((1, 1), Some(([0.0; 3], 0)), true, cause).update;
        let native = self.native.clone();
        let mut pool = self.windows.combats.take().unwrap();
        update_combat(
            &mut self.windows,
            &mut pool,
            native.naval_combat(),
            slot,
            &omitted,
            None,
            &self.fixture,
            &mut CounterReport::default(),
        )
        .unwrap();
        self.windows.combats = Some(pool);
    }
}

fn supply_flotilla(
    supply: viewer_foundation::map::counter_inputs::NavalSupply,
    byte: u16,
) -> naval::Flotilla {
    let mut item = gui_windows::tests::port_flotilla();
    item.supply = supply;
    item.item.as_mut().unwrap().byte = byte;
    if byte < 250 {
        item.bars = naval::NavalBars::Hidden;
    }
    item
}

/// Supply writes require full representative intel. A retained lower-intel
/// update leaves the prior supply drawable in place; a later plate write
/// repairs the withheld state.
#[test]
fn a_refused_update_makes_its_known_writes_and_waits_only_for_its_unknown_part() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use viewer_foundation::map::counter_inputs::NavalSupply;
    let mut shipped = Synthetic::new();
    let country = CounterSettings {
        color_mode: CounterColorMode::Country,
        ..CounterSettings::default()
    };
    let own = |supply, byte| naval::Flotilla {
        owner: 0,
        logical: Some(0),
        ..supply_flotilla(supply, byte)
    };
    // Full intel, supply above the show threshold: hidden.
    let (why, supply, _) = shipped.item(&own(NavalSupply::Write(100_000), 255), country);
    assert_eq!((why, supply), (None, None));
    // Full intel, very low supply, the plate's colour unknown.
    let mut refused = own(NavalSupply::Write(12_000), 255);
    refused.logical = Some(5);
    let (why, supply, _) = shipped.item(&refused, country);
    let why = why.unwrap();
    assert!(why.starts_with("unit_counter_navy is not drawn: "), "{why}");
    assert_eq!(supply, Some(2));
    // Partial intel, the colour known: supply is not written; the glyph the refused update
    // wrote stands and the item draws.
    let (why, supply, _) = shipped.item(&own(NavalSupply::Retain, 200), country);
    assert_eq!((why, supply), (None, Some(2)));
    // Full intel again.
    let (why, supply, _) = shipped.item(&own(NavalSupply::Write(100_000), 255), country);
    assert_eq!((why, supply), (None, None));
}

/// A partial-intel update keeps the authored supply glyph on a fresh pooled item,
/// but keeps it hidden after a full-supply update of that same item.
#[test]
fn partial_intel_supply_retains_fresh_or_last_written_visibility() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use viewer_foundation::map::counter_inputs::NavalSupply;
    let mut shipped = Synthetic::new();
    let settings = CounterSettings::default();
    let partial = supply_flotilla(NavalSupply::Retain, 50);
    let (why, supply, _) = shipped.item(&partial, settings);
    assert_eq!((why, supply), (None, Some(0)));
    let full = supply_flotilla(NavalSupply::Write(100_000), 255);
    let (why, supply, _) = shipped.item(&full, settings);
    assert_eq!((why, supply), (None, None));
    let (why, supply, _) = shipped.item(&partial, settings);
    assert_eq!((why, supply), (None, None));
}

/// A hull the recording cannot decide (a member's `ShipCounterInputs` unrecorded)
/// establishes none of the retaining returns. A port item or at-sea counter is
/// withheld and counted under its cause ([`gui_windows::TYPE_SPRITE`]), while its
/// window keeps its drawable, until selection writes the sprite again. An invalid
/// definition is a proven no-op and does not replace it.
#[test]
fn an_undecidable_hull_withholds_the_window_until_a_hull_is_written() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use counter_order::HullIcon;
    let mut shipped = Synthetic::new();
    let settings = CounterSettings::default();
    let with_hull = |mut flotilla: naval::Flotilla, hull: HullIcon| {
        flotilla.cells.as_mut().unwrap().hull = hull;
        flotilla
    };
    let destroyer = || HullIcon::Resolved(Some(b"destroyer".to_vec()));
    let sprite = b"GFX_unit_destroyer_icon_medium_white".to_vec();
    let port = gui_windows::tests::port_flotilla;
    let (why, _, hull) = shipped.item(&with_hull(port(), destroyer()), settings);
    assert_eq!((why, hull), (None, Some(sprite.clone())));
    let (why, _, hull) = shipped.item(&with_hull(port(), HullIcon::Unrecorded), settings);
    assert_eq!(
        why.as_deref(),
        Some(
            format!(
                "unit_counter_navy is not drawn: {}",
                gui_windows::HULL_UNRECORDED
            )
            .as_str()
        )
    );
    // The window keeps its drawable internally.
    assert_eq!(hull, Some(sprite.clone()));
    let (why, _, _) = shipped.item(&with_hull(port(), HullIcon::Resolved(None)), settings);
    assert!(why.is_some());
    let (why, _, hull) = shipped.item(&with_hull(port(), destroyer()), settings);
    assert_eq!((why, hull), (None, Some(sprite)));
    // A counter at sea: withheld where it would draw, then drawn.
    let at_sea = |hull| with_hull(gui_windows::tests::at_sea_flotilla(false), hull);
    assert_eq!(shipped.counter(&at_sea(destroyer()), settings), None);
    assert!(
        shipped
            .counter(&at_sea(HullIcon::Ambiguous), settings)
            .is_some()
    );
    assert_eq!(shipped.counter(&at_sea(destroyer()), settings), None);
}

/// Window graphics time and frame step are separate inputs. Fixed-step
/// presentation supplies both explicitly; a clock jump advances only the
/// clock-dependent state reached by the lifecycle.
#[test]
fn the_windows_run_on_the_presentation_clock_they_are_given() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut view = View::new();
    view.fixture.animated.insert(b"GFX_hull".to_vec(), 0.0625);
    let base = view.base(1);
    view.ports(base, &[Ok(gui_windows::Update::default())]);
    let native = view.native.clone();
    let window = native.naval_base().unwrap();
    let grid = window.find(native::Kind::Grid, "box_ships").unwrap();
    let mut seen = Vec::new();
    for capture in 0..5 {
        let time = 10.0 + 0.0625 * capture as f32;
        let frame = frame(&mut view.windows.last, time);
        let mut report = CounterReport::default();
        tick(
            &mut view.windows,
            &view.native,
            None,
            &view.fixture,
            frame,
            &mut report,
        );
        let pool = view.windows.bases.as_ref().unwrap();
        let visible = pool.visible().find(|icon| icon.slot == base).unwrap();
        let (holder, items) = port_parts(
            &view.windows,
            &visible,
            window,
            None,
            grid,
            &view.fixture,
            VIEWPORT,
            &mut report,
        )
        .unwrap();
        let drawn = visible_layout(
            &visible,
            window,
            NAVAL_BASE_WINDOW,
            &view.fixture,
            &mut report.icon_gui_notes,
        )
        .unwrap();
        let kept = visible_layout_kept(&visible, window, NAVAL_BASE_WINDOW, &view.fixture, &mut report.icon_gui_notes, 1)
            .unwrap();
        assert_eq!(kept.layout.as_drawn(), drawn.layout.clone().as_drawn(), "capture {capture}: the kept layout draws as a fresh one");
        let (reused_holder, reused_items) = port_parts(
            &view.windows,
            &visible,
            window,
            Some(&drawn.layout),
            grid,
            &view.fixture,
            VIEWPORT,
            &mut report,
        )
        .unwrap();
        assert_eq!(reused_holder, holder);
        assert_eq!(reused_items.len(), items.len());
        for (reused, fresh) in reused_items.iter().zip(&items) {
            assert_eq!(reused.key, fresh.key);
            assert_eq!(reused.place, fresh.place);
            assert_eq!(reused.layout, fresh.layout);
            assert_eq!(reused.selectable, fresh.selectable);
        }
        assert!(report.navy_counter_gaps.is_empty());
        assert!(report.icon_gui_notes.is_empty());
        let parts = holder.place(items[0].place, &items[0].layout);
        let hull = shown(&parts[0]).0;
        assert!(parts.iter().all(|part| part.shading.time == time));
        seen.push(hull);
    }
    assert_eq!(seen, [0, 1, 2, 0, 1]);
    assert_eq!(
        frame(&mut view.windows.last, 20.0),
        Tick {
            time: 20.0,
            dt: GUI_DELTA_CLAMP
        }
    );
    assert_eq!(
        frame(&mut view.windows.last, 5.0),
        Tick { time: 5.0, dt: 0.0 }
    );
}
/// Unengaged combat updates retain the prior background frame. A later
/// engaged rotation writes and repairs it.
#[test]
fn only_an_engaged_rotation_repairs_the_background_frame() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut view = View::new();
    let owner = (4, 9);
    let slot = view.combat(owner);
    let step = |view: &mut View, update: gui_windows::Update| {
        view.update(slot, update, Some(&[]));
        view.tick(0.0);
        view.windows
            .combats
            .as_ref()
            .unwrap()
            .withheld(slot)
            .map(str::to_owned)
    };
    let engaged = view.engaged_rotation(true);
    assert_eq!(step(&mut view, engaged), None);
    let cause = "icon omitted: convoy variant undecided: the NavalCombatShips table is \
                 truncated (3 rows unread)";
    let omitted = CombatIcon::omitted(owner, Some(([0.0; 3], 0)), true, cause).update;
    view.update(slot, omitted, None);
    view.tick(0.0);
    assert!(view.combat_items(slot, 9).0.is_none());
    let unengaged = view.engaged_rotation(false);
    assert!(!unengaged.wrote.contains(&BG_FRAME));
    assert_eq!(step(&mut view, unengaged).as_deref(), Some(cause));
    assert!(view.combat_items(slot, 9).0.is_none());
    let engaged = view.engaged_rotation(true);
    assert_eq!(step(&mut view, engaged), None);
    assert!(view.combat_items(slot, 9).0.is_some());
}

/// An omitted rotating icon leaves its grids' pooled items unknown too: the game
/// reuses them and may have updated them in the
/// omitted hour, and the game writes `low_supply` only at full
/// first-member intel. A tile that hid `low_supply` stays withheld when its icon's grids are
/// rebuilt with a partial-intel update (which writes every other part), and draws once a
/// full-intel update writes its supply.
#[test]
fn an_omitted_icon_leaves_its_items_unknown_until_they_are_written() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use viewer_foundation::map::counter_inputs::NavalSupply;
    let mut shipped = Synthetic::new();
    let settings = CounterSettings::default();
    let (why, supply, _) =
        shipped.item(&supply_flotilla(NavalSupply::Write(100_000), 255), settings);
    assert_eq!((why, supply), (None, None));
    // The icon's update is omitted: the tile is withheld with it.
    let cause = "icon omitted: convoy variant undecided: the NavalCombatShips table is \
                 truncated (3 rows unread)";
    shipped.omit(cause);
    let why = shipped.windows.items_withheld.why(&shipped.key).unwrap();
    assert!(why.contains(cause), "{why}");
    // Rebuilt at partial intel: supply is not written, so it stays unknown.
    let (why, supply, _) = shipped.item(&supply_flotilla(NavalSupply::Retain, 200), settings);
    assert!(why.is_some());
    assert_eq!(supply, None);
    // Full intel writes it: drawn at the very-low frame.
    let (why, supply, _) =
        shipped.item(&supply_flotilla(NavalSupply::Write(12_000), 255), settings);
    assert_eq!((why, supply), (None, Some(2)));
}

/// A received-supply ratio the recording lacks decides neither of
/// the `low_supply` writes. The item keeps what its window
/// showed and is withheld, counted by why ([`gui_windows::SUPPLY_UNRECORDED`]); a retained
/// (partial-intel) update keeps it withheld, and only a written ratio repairs it.
#[test]
fn an_unrecorded_supply_ratio_is_repaired_only_by_a_written_one() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use viewer_foundation::map::counter_inputs::NavalSupply;
    let mut shipped = Synthetic::new();
    let settings = CounterSettings::default();
    let withheld = format!(
        "unit_counter_navy is not drawn: {}",
        gui_windows::SUPPLY_UNRECORDED
    );
    let (why, supply, _) =
        shipped.item(&supply_flotilla(NavalSupply::Write(12_000), 255), settings);
    assert_eq!((why, supply), (None, Some(2)));
    // No hide is made: the glyph stands, withheld.
    let (why, supply, _) = shipped.item(&supply_flotilla(NavalSupply::Unavailable, 255), settings);
    assert_eq!((why.as_deref(), supply), (Some(withheld.as_str()), Some(2)));
    let (why, _, _) = shipped.item(&supply_flotilla(NavalSupply::Retain, 200), settings);
    assert_eq!(why.as_deref(), Some(withheld.as_str()));
    let (why, supply, _) =
        shipped.item(&supply_flotilla(NavalSupply::Write(100_000), 255), settings);
    assert_eq!((why, supply), (None, None));
}

/// An update that leaves a combat icon's grids undecided may have built more pooled
/// items (the game allocates the items a wrapper lacks and
/// reuses them afterwards), which the game may have given a
/// low-supply glyph it keeps below full intel. So an item the view first builds after such an
/// update is withheld like an existing one: tile two, built at partial intel after the
/// omitted update, waits for a full-intel write of its supply.
#[test]
fn an_item_first_built_after_undecided_grids_waits_for_its_kept_parts() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use viewer_foundation::map::counter_inputs::NavalSupply;
    let mut shipped = Synthetic::new();
    let settings = CounterSettings::default();
    let (why, _, _) = shipped.item_at(
        0,
        &supply_flotilla(NavalSupply::Write(100_000), 255),
        settings,
    );
    assert_eq!(why, None);
    let cause = "icon omitted: convoy variant undecided: the NavalCombatShips table is \
                 truncated (3 rows unread)";
    shipped.omit(cause);
    // Two tiles at partial intel: the second is first built now.
    let partial = supply_flotilla(NavalSupply::Retain, 200);
    let (first, _, _) = shipped.item_at(0, &partial, settings);
    let (second, _, _) = shipped.item_at(1, &partial, settings);
    assert!(first.is_some());
    let second = second.unwrap();
    assert!(second.contains(cause), "{second}");
    // Full intel writes its supply: drawn.
    let (second, supply, _) = shipped.item_at(
        1,
        &supply_flotilla(NavalSupply::Write(12_000), 255),
        settings,
    );
    assert_eq!((second, supply), (None, Some(2)));
}

/// Carrier sprite selection resolves its requested name through the registry.
/// Unavailable selection retains the old drawable and its named refusal.
#[test]
fn only_a_defined_carrier_sprite_repairs_the_carrier_drawable() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut shipped = Synthetic::new();
    let settings = CounterSettings::default();
    let carrier = |carrier: BTreeSet<Option<(&'static str, u32)>>| {
        let mut item = gui_windows::tests::port_flotilla();
        item.cells.as_mut().unwrap().carrier = carrier;
        item
    };
    let defined = || [Some(("GFX_mapicon_air_base", 1))].into();
    let undefined = || [Some(("GFX_mapicon_undefined_base", 1))].into();
    let (why, _, _) = shipped.item(&carrier(defined()), settings);
    assert_eq!(why, None);
    // Undecided: two states possible.
    let (why, _, _) = shipped.item(
        &carrier([None, Some(("GFX_mapicon_air_base", 2))].into()),
        settings,
    );
    let unknown = why.unwrap();
    assert!(unknown.contains("air_base_icon is undecided"), "{unknown}");
    // Shown with a sprite the registry lacks: the unknown drawable is kept and seen.
    let (why, _, _) = shipped.item(&carrier(undefined()), settings);
    assert_eq!(why.as_deref(), Some(unknown.as_str()));
    // Hidden: nothing unknown is seen.
    let (why, _, _) = shipped.item(&carrier([None].into()), settings);
    assert_eq!(why, None);
    // Shown again keeping the drawable: withheld again.
    let (why, _, _) = shipped.item(&carrier(undefined()), settings);
    assert_eq!(why.as_deref(), Some(unknown.as_str()));
    // A defined sprite makes it known.
    let (why, _, _) = shipped.item(&carrier(defined()), settings);
    assert_eq!(why, None);
    let (why, _, _) = shipped.item(&carrier(undefined()), settings);
    assert_eq!(why, None);
}

/// The carrier drawable is one of the parts an omitted icon's pooled items may
/// keep ([`gui_windows::MISSION_ITEM_KEPT`]). After the omission, the existing item and one
/// first built then stay withheld while a Show keeps a sprite the registry lacks, though
/// every other part is written; hidden they draw, and only a defined sprite repairs them.
#[test]
fn an_omitted_icon_leaves_its_items_carrier_drawable_unknown() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut shipped = Synthetic::new();
    let settings = CounterSettings::default();
    let carrier = |sprite: Option<&'static str>| {
        let mut item = gui_windows::tests::port_flotilla();
        item.cells.as_mut().unwrap().carrier = [sprite.map(|name| (name, 1))].into();
        item
    };
    let defined = carrier(Some("GFX_mapicon_air_base"));
    let undefined = carrier(Some("GFX_mapicon_undefined_base"));
    let (why, _, _) = shipped.item_at(0, &defined, settings);
    assert_eq!(why, None);
    let cause = "icon omitted: convoy variant undecided: the NavalCombatShips table is \
                 truncated (3 rows unread)";
    shipped.omit(cause);
    for index in [0, 1] {
        let (why, _, _) = shipped.item_at(index, &undefined, settings);
        let why = why.unwrap();
        assert!(why.contains(cause), "{index}: {why}");
        let (why, _, _) = shipped.item_at(index, &carrier(None), settings);
        assert_eq!(why, None, "{index}");
        let (why, _, _) = shipped.item_at(index, &undefined, settings);
        assert!(why.is_some(), "{index}");
        let (why, _, _) = shipped.item_at(index, &defined, settings);
        assert_eq!(why, None, "{index}");
    }
}

/// An unavailable port roster retains existing pooled membership and withholds
/// the affected items until a later update repairs their unknown parts.
#[test]
fn an_undecided_port_roster_leaves_its_items_unknown_until_they_are_written() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut view = View::new();
    let base = view.base(1);
    let partial = |view: &View, frame| view.hull(frame);
    let full = |view: &View, frame| view.full_hull(frame);
    for why in [
        "port items are not reproduced: task force 7's route row is not recorded",
        "port items are not reproduced: the port sort refuses the roster",
    ] {
        let items = [full(&view, 2)];
        view.ports(base, &items);
        view.tick(0.0);
        let (drawn, report) = view.port_items(base);
        assert_eq!(frames(&drawn), [1]);
        assert!(report.navy_counter_gaps.is_empty(), "{report:?}");
        view.undecided_ports(base, why);
        view.tick(0.0);
        let (drawn, report) = view.port_items(base);
        assert!(drawn.is_empty());
        assert!(
            report
                .navy_counter_gaps
                .keys()
                .all(|line| line.contains(why)),
            "{report:?}"
        );
        // Partial intel: the reused item and one first built now stay withheld.
        let items = [partial(&view, 2), partial(&view, 3)];
        view.ports(base, &items);
        view.tick(0.0);
        let (drawn, report) = view.port_items(base);
        assert!(drawn.is_empty(), "{why}");
        assert_eq!(
            report.navy_counter_gaps.values().sum::<usize>(),
            2,
            "{report:?}"
        );
        // Full intel writes them.
        let items = [full(&view, 2), full(&view, 3)];
        view.ports(base, &items);
        view.tick(0.0);
        let (drawn, report) = view.port_items(base);
        assert_eq!(frames(&drawn), [1, 2]);
        assert!(report.navy_counter_gaps.is_empty(), "{report:?}");
    }
}

/// S4: a naval counter at sea whose update the view skips though the game makes it (its task
/// force refused by the unit column, or its window's update stopped) is withheld until its
/// next update and until writes of the parts that update may keep: a later update whose hull
/// the content does not define leaves
/// it withheld, and a defined hull draws it.
#[test]
fn a_skipped_counter_at_sea_waits_for_its_kept_parts() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use counter_order::HullIcon;
    let mut shipped = Synthetic::new();
    let settings = CounterSettings::default();
    let at_sea = |hull: &[u8]| {
        let mut flotilla = gui_windows::tests::at_sea_flotilla(false);
        flotilla.cells.as_mut().unwrap().hull = HullIcon::Resolved(Some(hull.to_vec()));
        flotilla
    };
    let destroyer = at_sea(b"destroyer");
    assert_eq!(shipped.counter(&destroyer, settings), None);
    skipped_counter(
        &mut shipped.windows,
        destroyer.source,
        "its task force is not placed: its route is not recorded",
    );
    let counter = &shipped.windows.counters[&destroyer.source];
    let why = counter.withheld.why(&()).unwrap().to_owned();
    assert!(why.contains("its route is not recorded"), "{why}");
    assert!(shipped.counter(&at_sea(b"cruiser"), settings).is_some());
    assert_eq!(shipped.counter(&destroyer, settings), None);
}

/// A counter at sea whose source an incomplete TaskForces census lacks may still stand and
/// be updated by the game: it is kept, withheld until its next update writes the parts that
/// update may keep, while a complete census that lacks its source destroys it.
#[test]
fn a_counter_an_incomplete_census_lacks_is_kept_unknown() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut shipped = Synthetic::new();
    let flotilla = gui_windows::tests::at_sea_flotilla(false);
    let _ = shipped.counter(&flotilla, CounterSettings::default());
    let counter = |shipped: &Synthetic| {
        shipped
            .windows
            .counters
            .get(&flotilla.source)
            .map(|counter| counter.withheld.why(&()).map(str::to_owned))
    };
    assert_eq!(counter(&shipped), Some(None));
    let incomplete = Forces {
        recorded: BTreeSet::new(),
        complete: false,
    };
    settle_counters(&mut shipped.windows, &incomplete);
    let why = counter(&shipped).flatten().unwrap();
    assert!(why.contains(COUNTER_SOURCE_UNRECORDED), "{why}");
    let complete = Forces {
        complete: true,
        ..incomplete
    };
    settle_counters(&mut shipped.windows, &complete);
    assert_eq!(counter(&shipped), None);
}
/// Under an incomplete `NavalCombats` census, an absent combat may still resolve.
/// Keep its icon with an omitted update ([`CombatIcon::omitted`]), leaving bars,
/// grids, frames and place unknown and marking its pooled items. Under a complete
/// census, the absent combat is empty.
#[test]
fn a_combat_an_incomplete_census_lacks_keeps_its_icon_omitted() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut view = View::new();
    let owner = (4, 9);
    let slot = view.combat(owner);
    let native = view.native.clone();
    let window = native.naval_combat();
    let fills = view.fills(vec![view.full_hull(2)], Vec::new());
    view.update(slot, rotation(), Some(&fills));
    view.tick(0.0);
    assert!(view.combat_items(slot, 9).0.is_some());
    let mut pass = CombatPass {
        icons: Vec::new(),
        gated: BTreeSet::new(),
        live: BTreeSet::new(),
        live_undecided: None,
        hq: Ok(BTreeMap::new()),
        size: [40, 60],
    };
    assert_eq!(combat_resolves(&pass, 9), Some(false));
    let cause = "icon omitted: whether its combat resolves is undecided: the NavalCombats \
                 table is truncated (1 rows unread)";
    pass.live_undecided = Some(cause.into());
    assert_eq!(combat_resolves(&pass, 9), None);
    let (update, fills) = planned_update(&pass, owner, window, &view.fixture);
    assert!(fills.is_none());
    let unknown: Vec<&str> = update.unknown.iter().map(|(part, _)| *part).collect();
    for part in [BG_FRAME, PROGRESS, GRIDS, FRAME_BTN, PLACE] {
        assert!(unknown.contains(&part), "{part}: {unknown:?}");
    }
    view.update(slot, update.into_owned(), None);
    view.tick(0.0);
    assert!(view.combat_items(slot, 9).0.is_none());
    let tile_grid = View::grid_path(native.naval_combat(), "attacker_grid");
    let tiles = view.windows.grids[&(IconSlot::Combat(slot), tile_grid)].clone();
    let why = view.windows.items_withheld.why(&tiles[0]).unwrap();
    assert!(why.contains(cause), "{why}");
}

/// S5, allocations after a decision the view cannot reproduce: once the view's requests or
/// erasures may have left the game's (an icon with no decided place never requested, an
/// unanchored base, a pass erased because it cannot be planned, a growth's completion frame),
/// The round robin may give a new owner another slot, with another
/// history, so a slot newly allocated afterwards is withheld until writes of the parts an
/// update may keep (here `bg_btn`'s frame, written only engaged), and its items are unknown;
/// an owner that keeps its slot, and any allocation before the divergence, are not.
#[test]
fn a_slot_allocated_after_a_divergence_is_unknown() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut view = View::new();
    let native = view.native.clone();
    let window = native.naval_combat();
    let request = |view: &mut View, owner: CombatOwner| {
        let mut pool = view.windows.combats.take().unwrap();
        let diverged = view.windows.diverged_combats.clone();
        let slot = allocate(
            &mut view.windows,
            &mut pool,
            owner,
            IconSlot::Combat,
            &COMBAT_KEPT,
            diverged,
        )
        .unwrap();
        pool.update(window, |_, _| Admission::Inside([300, 100]))
            .unwrap();
        view.windows.combats = Some(pool);
        slot
    };
    let withheld = |view: &mut View, slot: SlotId, engaged: bool| {
        let update = view.engaged_rotation(engaged);
        view.update(slot, update, Some(&[]));
        view.tick(0.0);
        view.windows
            .combats
            .as_ref()
            .unwrap()
            .withheld(slot)
            .map(str::to_owned)
    };
    let before = request(&mut view, (1, 1));
    assert_eq!(withheld(&mut view, before, false), None);
    view.windows.diverged_combats = Some(GROWTH_FRAME.into());
    // Kept by its owner: unaffected.
    assert_eq!(request(&mut view, (1, 1)), before);
    assert_eq!(withheld(&mut view, before, false), None);
    // A new owner: unknown until an engaged rotation writes its frame, and (F4) until its
    // window has been out of a transition for the template's longest (its 0.1 s fade): the
    // slot's fade and movement are another owner's history.
    let after = request(&mut view, (2, 2));
    let why = withheld(&mut view, after, false).unwrap();
    assert!(why.contains(GROWTH_FRAME), "{why}");
    let why = withheld(&mut view, after, true).unwrap();
    assert!(why.contains(TRANSITION), "{why}");
    let standing = |view: &View| {
        view.windows
            .combats
            .as_ref()
            .unwrap()
            .withheld(after)
            .map(str::to_owned)
    };
    // Its own fade in runs first, then the wait: still unknown after 0.1 s.
    view.tick(0.05);
    view.tick(0.05);
    assert!(standing(&view).is_some());
    assert!(view.windows.take_active());
    for _ in 0..4 {
        view.tick(0.05);
    }
    assert_eq!(standing(&view), None);
    assert!(view.windows.settling.is_empty());
}

/// F4: a window whose tick fails may stand mid-transition where the game's does not: its
/// fade and movement are unknown until the view's window has been out of a transition for
/// the template's longest; a template with no transition is left alone.
#[test]
fn a_window_whose_tick_fails_has_unknown_transitions() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut view = View::new();
    let combat = view.combat((1, 9));
    view.update(combat, rotation(), Some(&[]));
    for _ in 0..5 {
        view.tick(0.05);
    }
    let native = view.native.clone();
    let registered = Registered::Icon(IconSlot::Combat(combat));
    transition_unknown(
        &mut view.windows,
        registered,
        "its window does not tick: a test failure",
        Marked::EveryFrame,
    );
    view.tick(0.0);
    let why = view.windows.combats.as_ref().unwrap().withheld(combat);
    assert!(why.is_some_and(|why| why.contains(TRANSITION)), "{why:?}");
    view.tick(0.05);
    assert!(
        view.windows
            .combats
            .as_ref()
            .unwrap()
            .withheld(combat)
            .is_some()
    );
    view.tick(0.06);
    assert_eq!(
        view.windows.combats.as_ref().unwrap().withheld(combat),
        None
    );
    // The bound is the constructed window's registered graph: the combat window's 0.1 s fade.
    let pool = view.windows.combats.as_ref().unwrap();
    assert_eq!(
        pool.instance(combat)
            .unwrap()
            .longest_transition(native.naval_combat()),
        Ok(0.1)
    );
}

/// An unengaged rotating icon keeps its pool slot's `bg_btn` frame; the game
/// sets it only while engaged. The pool's frame wins when established for this
/// owner. Otherwise the engaged index's carried frame establishes it and is
/// counted as carried; any other answer uses the binding's initial frame and
/// is counted as not from history.
#[test]
fn the_pools_frame_wins_and_the_index_establishes_one_it_lacks() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use viewer_foundation::icon_frames::{CARRIED, Kept, NAVAL_NO_HISTORY};
    let view = View::with_gui(&GUI.replacen(
        r#"buttonType = { name = "bg_btn" quadTextureSprite = "GFX_bg" }"#,
        r#"buttonType = { name = "bg_btn" quadTextureSprite = "GFX_bg" frame = 2 }"#,
        1,
    ));
    let window = view.native.naval_combat();
    let owner = (4, 44);
    let base = gui_windows::Update::default();
    // Established by the view: nothing written, nothing counted.
    let mut report = CounterReport::default();
    let kept = kept_frame(
        true,
        owner,
        Kept::Frame(2),
        window,
        base.clone(),
        &mut report,
    );
    assert_eq!(kept, base);
    assert!(report.combat_limits.is_empty());
    // Not established: the carried frame is written, one-based, and counted.
    let kept = kept_frame(
        false,
        owner,
        Kept::Frame(2),
        window,
        base.clone(),
        &mut report,
    );
    assert_eq!(kept.wrote, [gui_windows::BG_FRAME]);
    assert!(
        kept.calls
            .iter()
            .any(|(path, call)| path.ends_with("bg_btn") && *call == Call::SetFrame(3)),
        "{:?}",
        kept.calls
    );
    assert_eq!(
        report.combat_limits.get(&("Naval", CARRIED.to_owned())),
        Some(&BTreeSet::from([44]))
    );
    // The binding overwrites the authored frame 2 with 1.
    let fixture = Fixture::new(&[("GFX_bg", 3, [120, 40])], &[]);
    let mut instance = Instance::construct(window, VIEWPORT, &fixture).unwrap();
    instance.gui_show(window, window).unwrap();
    instance
        .tick(window, &fixture, Tick { time: 1.0, dt: 1.0 })
        .unwrap();
    let mut report = CounterReport::default();
    for unknown in [
        Kept::Fresh,
        Kept::Unobserved,
        Kept::Undecided,
        Kept::Unindexed,
    ] {
        let kept = kept_frame(false, owner, unknown, window, base.clone(), &mut report);
        assert_eq!(kept.wrote, [BG_FRAME]);
        assert!(kept.unknown.is_empty());
        assert!(
            kept.calls
                .iter()
                .any(|(_, call)| *call == Call::SetFrame(1))
        );
        gui_windows::apply(&mut instance, window, &kept.calls, &fixture).unwrap();
        let layout = instance.layout(window, &fixture, None).unwrap().unwrap();
        let background = layout
            .parts
            .iter()
            .find(|part| part.path.ends_with("bg_btn"))
            .expect("unseen background drawn");
        assert_eq!(shown(background).0, 0);
        assert_eq!(
            report
                .combat_limits
                .get(&("Naval", unknown.limitation(NAVAL_NO_HISTORY))),
            Some(&BTreeSet::from([44]))
        );
    }
    assert_eq!(report.combat_limits.len(), 4);
    // A later index result takes over from the initial fallback.
    let kept = kept_frame(false, owner, Kept::Frame(2), window, base, &mut report);
    gui_windows::apply(&mut instance, window, &kept.calls, &fixture).unwrap();
    let layout = instance.layout(window, &fixture, None).unwrap().unwrap();
    let background = layout
        .parts
        .iter()
        .find(|part| part.path.ends_with("bg_btn"))
        .unwrap();
    assert_eq!(shown(background).0, 2);
}

/// The lead's F4 sibling: a counter at sea whose update the view skipped may have missed that
/// update's Show or Hide, so its window's fade and movement are unknown as well as its
/// update: [`TRANSITION`] stands until the view's window has been out of a transition for its
/// template's longest, and lifting it leaves the update's own refusal standing.
#[test]
fn a_skipped_counter_at_sea_has_unknown_transitions() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    // `unit_counter_navy` with a 0.1 s fade.
    let gui = GUI.replacen(
        r#"name = "unit_counter_navy" size = { width = 20 height = 10 }"#,
        r#"name = "unit_counter_navy" size = { width = 20 height = 10 }
        fade_type = linear fade_time = 100"#,
        1,
    );
    let native = NativeWindows::read(gui.as_bytes(), VIEWPORT).unwrap();
    let fixture = fixture();
    let mut windows = Windows::default();
    windows.build(&native, VIEWPORT, &fixture).unwrap();
    let navy = native.navy();
    counter_window(
        &mut windows,
        navy,
        7,
        &sea_plate([0; 2], 1, [0; 2]),
        false,
        &gui_windows::Update::default(),
        &fixture,
    )
    .unwrap();
    let mut time = 0.0;
    let mut tick_by = |windows: &mut Windows, dt: f32| {
        time += dt;
        let mut report = CounterReport::default();
        tick(
            windows,
            &native,
            None,
            &fixture,
            Tick { time, dt },
            &mut report,
        );
    };
    for _ in 0..4 {
        tick_by(&mut windows, 0.05);
    }
    assert_eq!(windows.counters[&7].withheld.why(&()), None);
    skipped_counter(&mut windows, 7, "its task force is not placed: a test");
    assert!(windows.settling.contains_key(&Registered::Counter(7)));
    tick_by(&mut windows, 0.0);
    let transition = |windows: &Windows| {
        windows.counters[&7]
            .withheld
            .why(&())
            .is_some_and(|why| why.contains(TRANSITION))
    };
    assert!(transition(&windows));
    tick_by(&mut windows, 0.05);
    assert!(windows.settling.contains_key(&Registered::Counter(7)));
    tick_by(&mut windows, 0.06);
    assert!(windows.settling.is_empty());
    // The skipped update itself is still unknown until an update writes it again.
    let why = windows.counters[&7].withheld.why(&()).unwrap();
    assert!(why.contains("a test"), "{why}");
}

/// A combat pass placing `owners`' icons, each decided.
fn combat_pass(owners: &[CombatOwner]) -> CombatPass {
    CombatPass {
        icons: owners
            .iter()
            .map(|&owner| CombatIcon::omitted(owner, Some(([0.0; 3], 0)), true, "test"))
            .collect(),
        gated: BTreeSet::new(),
        live: owners.iter().map(|(_, combat)| *combat).collect(),
        live_undecided: None,
        hq: Ok(BTreeMap::new()),
        size: [40, 60],
    }
}

/// The owners run only in the frame's province updates : an icon whose
/// province is off screen and not at its phase keeps its slot, whether or not the pass
/// still places it; the province's update out of view, at its phase, erases it (the game
/// clears the pass flag); a clean province off screen is not updated at all.
#[test]
fn an_off_screen_province_keeps_its_icon_until_its_update() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut view = View::new();
    let window = view.native.clone();
    let window = window.naval_combat();
    let mut schedule = Schedule::default();
    let mut pool = view.windows.combats.take().unwrap();
    let mut report = CounterReport::default();
    let mut owners = |view: &mut View, pool: &mut Pool<CombatOwner>, schedule: &Schedule, pass| {
        combat_owners(&mut view.windows, pool, window, pass, schedule, &mut report).unwrap()
    };
    let pass = combat_pass(&[(3, 1)]);
    // Frame 1 (phase 1): province 3 in view requests its icon.
    schedule.advance([3], |_| true, false);
    assert!(owners(&mut view, &mut pool, &schedule, Some(&pass)).is_empty());
    let slot = pool.slot_of(&(3, 1)).unwrap();
    // Frame 2 (phase 2): off screen and dirty, not at its phase: kept, even with the pass off.
    schedule.advance([3], |_| false, false);
    assert!(owners(&mut view, &mut pool, &schedule, None).is_empty());
    assert_eq!(pool.slot_of(&(3, 1)), Some(slot));
    // Frame 3 (phase 3): its update out of view erases it.
    schedule.advance([3], |_| false, false);
    assert_eq!(schedule.update(3), Some(ScheduleUpdate::OffScreen));
    let erased = owners(&mut view, &mut pool, &schedule, Some(&pass));
    assert_eq!(erased.len(), 1);
    assert_eq!(pool.slot_of(&(3, 1)), None);
    // Now clean: a hundred frames on, at its phase again, it is not in the list, and the
    // pass's icon of it is never requested.
    for _ in 0..100 {
        schedule.advance([3], |_| false, false);
        assert!(owners(&mut view, &mut pool, &schedule, Some(&pass)).is_empty());
        assert_eq!(pool.slot_of(&(3, 1)), None);
    }
}

/// The round robin moves only for the requests of provinces the
/// frame updates: across a stretch where a planned icon's province stays off screen and
/// clean, the next owner in view takes the slot after the last one taken, as the game's.
#[test]
fn an_off_screen_stretch_leaves_the_round_robin_as_the_game_has_it() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut view = View::new();
    let window = view.native.clone();
    let window = window.naval_combat();
    let mut schedule = Schedule::default();
    let mut pool = view.windows.combats.take().unwrap();
    let mut report = CounterReport::default();
    // Province 5's icon in view; 7 was dirty, clears at its phase and then stays off screen.
    let first = combat_pass(&[(5, 1)]);
    schedule.advance([5, 7], |id| id == 5, false);
    combat_owners(
        &mut view.windows,
        &mut pool,
        window,
        Some(&first),
        &schedule,
        &mut report,
    )
    .unwrap();
    assert!(pool.slot_of(&(5, 1)).is_some());
    let both = combat_pass(&[(5, 1), (7, 2)]);
    for _ in 0..7 {
        schedule.advance([5, 7], |id| id == 5, false);
        combat_owners(
            &mut view.windows,
            &mut pool,
            window,
            Some(&both),
            &schedule,
            &mut report,
        )
        .unwrap();
        assert_eq!(pool.slot_of(&(7, 2)), None);
    }
    // Province 9 comes into view: the next slot of the round robin, untouched by 7.
    let next = combat_pass(&[(5, 1), (7, 2), (9, 3)]);
    schedule.advance([5, 7, 9], |id| id != 7, false);
    combat_owners(
        &mut view.windows,
        &mut pool,
        window,
        Some(&next),
        &schedule,
        &mut report,
    )
    .unwrap();
    // The pool: 5's request, then 9's, and nothing between.
    let mut game = View::new().windows.combats.take().unwrap();
    game.request((5, 1)).unwrap();
    let expected = game.request((9, 3));
    assert_eq!(pool.slot_of(&(9, 3)), expected);
}

/// The retained result window is laid out and drawn through the GUI module. A
/// lower-right anchored, centred and scaled `bg_btn` uses the authored button's
/// placement; replacing its drawable applies the setter's placement rules.
/// The hit target is the drawn quad.
#[test]
fn a_lower_right_centred_scaled_result_button_is_placed_as_the_game_places_it() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let gui = format!(
        "{}\n{}",
        GUI.trim_end().trim_end_matches('}'),
        r#"containerWindowType = { name = "naval_combat_results_mapicon"
            size = { width = 19 height = 24 }
            buttonType = { name = "bg_btn" quadTextureSprite = "GFX_naval_combat_result"
                position = { x = -4 y = -6 } orientation = lower_right centerPosition = yes
                scale = 2 } } }"#
    );
    let native = NativeWindows::read(gui.as_bytes(), VIEWPORT).unwrap();
    let fixture = Fixture::new(
        &[
            ("GFX_naval_combat_result", 3, [30, 8]),
            ("GFX_naval_combat_result_convoys", 3, [36, 10]),
        ],
        &[],
    );
    let mut windows = Windows::default();
    windows.build(&native, VIEWPORT, &fixture).unwrap();
    let window = native.naval_result().unwrap();
    let mut pool = windows.results.take().unwrap();
    let slot = pool.request(10).unwrap();
    pool.update(window, |_, _| Admission::Inside([100, 200]))
        .unwrap();
    let footprint = |pool: &mut Pool<u32>| {
        let instance = pool.get(slot).unwrap();
        let layout = instance.layout(window, &fixture, None).unwrap().unwrap();
        naval_markers::background_footprint(window, &layout).unwrap()
    };
    let authored = footprint(&mut pool);
    let mut undeclared = BTreeMap::new();
    let update = gui_windows::result_calls(
        window,
        &fixture,
        Ok((naval_markers::NAVAL_RESULT_SPRITES[3], 1)),
        &mut undeclared,
    );
    assert!(undeclared.is_empty());
    let instance = pool.get(slot).unwrap();
    gui_windows::apply(instance, window, &update.calls, &fixture).unwrap();
    let built = footprint(&mut pool);
    // Built: a 10 x 8 frame at scale 2, 20 x 16; `lower_right` puts the position (-4, -6)
    // at (15, 18) of the 19 x 24 window, and `centerPosition` centres the scaled extent on it.
    assert_eq!(authored, [5.0, 10.0, 20.0, 16.0]);
    // After the convoy literal (a 12 x 10 frame): the raw position with no orientation
    // offset, centred on the unscaled size (-4 - 6, -6 - 5), then drawn at scale 2.
    assert_eq!(built, [-10.0, -11.0, 24.0, 20.0]);
    assert!(update.wrote.contains(&gui_windows::RESULT_SPRITE));
}

/// F4 class, finding 1: an item first built after its icon's update went unreproduced
/// inherits the unknown transition once, at construction; its later updates, made every
/// frame, do not restart the wait, so it is known again once its window has been out of a
/// transition for the longest.
#[test]
fn a_grown_item_updated_every_frame_is_known_again() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut view = View::with_gui(&GUI.replacen(
        r#"name = "unit_counter_navy" size = { width = 20 height = 10 }"#,
        r#"name = "unit_counter_navy" size = { width = 20 height = 10 }
        fade_type = linear fade_time = 100"#,
        1,
    ));
    let base = view.base(1);
    view.undecided_ports(base, "a test roster");
    // Every write of the item, its retained parts included: only the transition can wait.
    let items = [view.full_hull(2)];
    view.ports(base, &items);
    let box_ships = View::grid_path(view.base_window(), "box_ships");
    let key = view.windows.grids[&(IconSlot::Base(base), box_ships)][0];
    assert!(view.windows.settling.contains_key(&Registered::Item(key)));
    for _ in 0..10 {
        view.ports(base, &items);
        view.tick(0.05);
    }
    assert!(!view.windows.settling.contains_key(&Registered::Item(key)));
    assert_eq!(view.windows.items_withheld.why(&key), None);
}

/// An unavailable port roster retains existing pooled membership and withholds
/// the affected items until a later update repairs their unknown parts.
#[test]
fn a_late_convoy_addition_inherits_the_unknown_transition() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut view = View::with_gui(&GUI.replacen(
        r#"name = "GFX_mapicon_tiny_unit_counter"
        size = { width = 10 height = 10 }"#,
        r#"name = "GFX_mapicon_tiny_unit_counter"
        size = { width = 10 height = 10 } fade_type = linear fade_time = 100"#,
        1,
    ));
    let combat = view.combat((1, 9));
    unknown_items(&mut view.windows, IconSlot::Combat(combat), "a test update", Marked::EveryFrame);
    let fills = view.fills(Vec::new(), vec![view.convoy(1)]);
    view.update(combat, rotation(), Some(&fills));
    let key = *view
        .windows
        .items
        .keys()
        .find(|key| key.icon == IconSlot::Combat(combat))
        .unwrap();
    assert!(view.windows.items[&key].tiny);
    assert!(view.windows.settling.contains_key(&Registered::Item(key)));
    assert!(view.windows.items[&key].unknown.is_empty());
    view.tick(0.0);
    let why = view.windows.items_withheld.why(&key).unwrap();
    assert!(why.contains(TRANSITION), "{why}");
}

/// The registered transition graph includes a drop-down's expanded window,
/// even when no declared child window has a transition.
#[test]
fn a_drop_down_only_transition_bounds_the_wait() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let view = View::with_gui(&GUI.replacen(
        r#"iconType = { name = "type" spriteType = "GFX_hull" } }"#,
        r#"iconType = { name = "type" spriteType = "GFX_hull" }
        dropDownBoxType = { name = "dd" expandButton = { name = "b" }
            expandedWindow = { name = "x" fade_type = linear fade_time = 250 } } }"#,
        1,
    ));
    let navy = view.native.navy();
    let instance = Instance::construct(navy, VIEWPORT, &view.fixture).unwrap();
    assert_eq!(instance.longest_transition(navy), Ok(0.25));
}

/// Every mission update first hides and flags the ideology child.
#[test]
fn a_definitely_hidden_ideology_repairs_an_undecided_one() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut shipped = Synthetic::new();
    let settings = CounterSettings::default();
    let with = |ideology: Result<Option<i32>, String>, coarse: bool| naval::Flotilla {
        ideology,
        ..gui_windows::tests::at_sea_flotilla(coarse)
    };
    let undecided = with(Err("CountryCivilWars is not recorded".into()), false);
    let why = shipped.counter(&undecided, settings).unwrap();
    assert!(why.contains("counter_ideology is undecided"), "{why}");
    // Err -> Ok(None): drawn.
    assert_eq!(shipped.counter(&with(Ok(None), false), settings), None);
    // Err -> coarse: drawn.
    assert!(shipped.counter(&undecided, settings).is_some());
    assert_eq!(shipped.counter(&with(Ok(None), true), settings), None);
    // A decided overlay shows it and is known too.
    assert!(shipped.counter(&undecided, settings).is_some());
    assert_eq!(shipped.counter(&with(Ok(Some(2)), false), settings), None);
}

/// A definite hide clears an earlier refusal for the part it hides: unknown bars
/// followed by a coarse or hidden-bars update; an undecided hull followed by a
/// coarse update; or a mission item's unknown bars followed by hidden bars.
#[test]
fn a_definite_hide_repairs_an_uncertain_part() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use counter_order::HullIcon;
    let mut shipped = Synthetic::new();
    let settings = CounterSettings::default();
    let at_sea = |coarse| gui_windows::tests::at_sea_flotilla(coarse);
    let unknown_bars = naval::Flotilla {
        bars: naval::NavalBars::Unknown("a member's ratios are not recorded".into()),
        ..at_sea(false)
    };
    let hidden_bars = naval::Flotilla {
        bars: naval::NavalBars::Hidden,
        ..at_sea(false)
    };
    let mut undecided_hull = at_sea(false);
    undecided_hull.cells.as_mut().unwrap().hull = HullIcon::Ambiguous;
    assert!(shipped.counter(&unknown_bars, settings).is_some());
    assert_eq!(shipped.counter(&at_sea(true), settings), None);
    assert!(shipped.counter(&unknown_bars, settings).is_some());
    assert_eq!(shipped.counter(&hidden_bars, settings), None);
    assert!(shipped.counter(&undecided_hull, settings).is_some());
    assert_eq!(shipped.counter(&at_sea(true), settings), None);
    // A mission item: unknown bars at full intel, then hidden below it.
    let item = |bars| naval::Flotilla {
        bars,
        ..gui_windows::tests::port_flotilla()
    };
    let (why, _, _) = shipped.item(
        &item(naval::NavalBars::Unknown("unrecorded".into())),
        settings,
    );
    assert!(why.is_some());
    let (why, _, _) = shipped.item(&item(naval::NavalBars::Hidden), settings);
    assert_eq!(why, None);
}

/// The convoy majority counts every held reference, including expired ones. A
/// reference absent from an incomplete census leaves presentation undecided.
#[test]
fn a_result_presentation_counts_every_held_reference() {
    let convoy = naval_markers::ResultTables {
        provinces: BTreeMap::from([(10, vec![1])]),
        facts: BTreeMap::from([(1, naval_markers::convoy_result_for_test())]),
        undecided: None,
    };
    // Held [1, 2], 2 expired: one convoy battle of two is no majority, a plain battle.
    assert_eq!(
        held_presentation(&convoy, &[1, 2]),
        Ok((naval_markers::NAVAL_RESULT_SPRITES[2], 1))
    );
    assert_eq!(
        held_presentation(&convoy, &[1]),
        Ok((naval_markers::NAVAL_RESULT_SPRITES[3], 1))
    );
    let incomplete = naval_markers::ResultTables {
        undecided: Some("incomplete"),
        ..convoy
    };
    assert_eq!(held_presentation(&incomplete, &[1, 2]), Err("incomplete"));
}

/// An incomplete result census cannot establish that a live icon is empty or
/// should be erased. Owner and manager decisions are undecided, the pool diverges,
/// and Show/Hide history is unknown. A later complete census recovers the icon
/// in its slot; a new owner allocated after divergence takes an unknown slot.
#[test]
fn an_incomplete_result_census_keeps_its_icons_undecided() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let gui = format!(
        "{}\n{}",
        GUI.trim_end().trim_end_matches('}'),
        r#"containerWindowType = { name = "naval_combat_results_mapicon"
            size = { width = 19 height = 24 } fade_type = linear fade_time = 100
            buttonType = { name = "bg_btn" quadTextureSprite = "GFX_bg" } } }"#
    );
    let mut view = View::with_gui(&gui);
    let native = view.native.clone();
    let window = native.naval_result().unwrap();
    let mut pool = view.windows.results.take().unwrap();
    let mut report = CounterReport::default();
    let pass = |undecided, provinces: BTreeMap<u32, Vec<u64>>| ResultPass {
        tables: Arc::new(naval_markers::ResultTables {
            facts: provinces
                .values()
                .flatten()
                .map(|&id| (id, naval_markers::convoy_result_for_test()))
                .collect(),
            provinces,
            undecided,
        }),
        raised: BTreeSet::new(),
        vectors_whole: true,
        size: [19, 24],
    };
    let mut schedule = Schedule::default();
    // Frame 1: a complete census shows province 10's result: requested.
    schedule.advance([10, 20], |_| true, false);
    let shown = pass(None, BTreeMap::from([(10, vec![1])]));
    let undecided = result_owners(
        &mut view.windows,
        &mut pool,
        window,
        Some(&shown),
        &schedule,
        &mut report,
    )
    .unwrap();
    assert!(undecided.is_empty());
    let slot = pool.slot_of(&10).unwrap();
    // Frame 2: an incomplete census with no row at all: not erased, undecided.
    schedule.advance([10, 20], |_| true, false);
    let empty = pass(
        Some("the retained naval results are incomplete"),
        BTreeMap::new(),
    );
    let undecided = result_owners(
        &mut view.windows,
        &mut pool,
        window,
        Some(&empty),
        &schedule,
        &mut report,
    )
    .unwrap();
    assert_eq!(pool.slot_of(&10), Some(slot));
    assert_eq!(undecided.len(), 1);
    assert_eq!(result_empty(&empty.tables, &[1]), None);
    undecided_results(&mut view.windows, &pool, &undecided, &mut report);
    assert!(view.windows.diverged_results.is_some());
    assert!(
        view.windows
            .settling
            .contains_key(&Registered::Icon(IconSlot::Result(slot)))
    );
    // Frame 3: the census is whole again: the icon keeps its slot and its results.
    schedule.advance([10, 20], |_| true, false);
    let undecided = result_owners(
        &mut view.windows,
        &mut pool,
        window,
        Some(&shown),
        &schedule,
        &mut report,
    )
    .unwrap();
    assert!(undecided.is_empty());
    assert_eq!(pool.slot_of(&10), Some(slot));
    assert_eq!(view.windows.held_results[&10], [1]);
    // A new owner after the divergence: its slot's history is unknown.
    let both = pass(None, BTreeMap::from([(10, vec![1]), (20, vec![2])]));
    schedule.advance([10, 20], |_| true, false);
    result_owners(
        &mut view.windows,
        &mut pool,
        window,
        Some(&both),
        &schedule,
        &mut report,
    )
    .unwrap();
    let other = pool.slot_of(&20).unwrap();
    assert!(
        view.windows
            .settling
            .contains_key(&Registered::Icon(IconSlot::Result(other)))
    );
}

/// An invalid hull or undefined sprite returns before writing, keeping the
/// drawable. After an unknown hull, a coarse update hides `type` and draws the
/// counter. A detailed update whose selection fails shows the remembered unknown
/// drawable, withholding the counter again and counting its cause. A successful
/// replacement repairs it.
#[test]
fn a_failed_hull_selection_restores_the_remembered_unknown_sprite() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use counter_order::HullIcon;
    let mut shipped = Synthetic::new();
    let settings = CounterSettings::default();
    let hull = |hull| {
        let mut flotilla = gui_windows::tests::at_sea_flotilla(false);
        flotilla.cells.as_mut().unwrap().hull = hull;
        flotilla
    };
    assert!(
        shipped
            .counter(&hull(HullIcon::Ambiguous), settings)
            .is_some()
    );
    assert_eq!(
        shipped.counter(&gui_windows::tests::at_sea_flotilla(true), settings),
        None
    );
    // A hull whose sprite the content does not define: the selection keeps the drawable.
    let why = shipped.counter(
        &hull(HullIcon::Resolved(Some(b"battleship".to_vec()))),
        settings,
    );
    let why = why.expect("the unknown drawable is shown again");
    assert!(why.contains(gui_windows::HULL_AMBIGUOUS), "{why}");
    let diagnostics = CounterReport {
        navy_counter_gaps: BTreeMap::from([(why.clone().into(), 1)]),
        ..CounterReport::default()
    };
    assert!(
        diagnostics
            .warnings()
            .any(|line| line.contains(gui_windows::HULL_AMBIGUOUS))
    );
    // An empty (invalid) hull keeps it too.
    assert!(
        shipped
            .counter(&hull(HullIcon::Resolved(None)), settings)
            .is_some()
    );
    // A defined hull replaces it.
    assert_eq!(
        shipped.counter(
            &hull(HullIcon::Resolved(Some(b"destroyer".to_vec()))),
            settings
        ),
        None
    );
}

/// Region grids reuse retained item windows without sharing slots with a
/// combat icon, and clearing one region removes every prior child attachment.
#[test]
fn regional_grids_keep_three_item_pools_separate_and_clear_together() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let native = NativeWindows::read(GUI.as_bytes(), VIEWPORT).unwrap();
    let fixture = fixture();
    let mut windows = Windows::default();
    windows.build(&native, VIEWPORT, &fixture).unwrap();
    let region_gui = native::read_at(
        br#"guiTypes={containerWindowType={name=region size={width=100 height=100}
        gridBoxType={name=unit_grid size={width=40 height=40} slotsize={width=20 height=10}}
        gridBoxType={name=enemy_unit_grid size={width=40 height=40} slotsize={width=20 height=10}}
        gridBoxType={name=enemy_convoy_grid size={width=40 height=40} slotsize={width=10 height=10}}
    }}"#,
        VIEWPORT,
    );
    let root = region_gui.template("region").unwrap();
    let mut instance = Instance::construct(root, VIEWPORT, &fixture).unwrap();
    instance.gui_show(root, root).unwrap();
    let slot = windows.combats.as_mut().unwrap().request((1, 99)).unwrap();
    let update = |path: String, frame: i32| {
        Ok(gui_windows::Update {
            calls: vec![(path, Call::SetFrame(frame))],
            ..Default::default()
        })
    };
    let hull = native
        .navy()
        .find(native::Kind::Icon, "type")
        .unwrap()
        .path
        .clone();
    let tiny = native
        .tiny()
        .unwrap()
        .find(native::Kind::Icon, "icon")
        .unwrap()
        .path
        .clone();
    let fills: Vec<_> = [
        ("unit_grid", Some(0), hull.clone(), 1),
        ("enemy_unit_grid", Some(1), hull, 2),
        ("enemy_convoy_grid", None, tiny, 3),
    ]
    .into_iter()
    .map(|(name, side, path, frame)| GridFill {
        grid: root.find(native::Kind::Grid, name).unwrap().path.clone(),
        side,
        items: vec![update(path, frame)],
    })
    .collect();
    let mut report = CounterReport::default();
    fill_grids_in(
        &fixture,
        &mut windows,
        &instance,
        root,
        IconSlot::Mission(slot),
        &fills,
        &mut report,
    )
    .unwrap();
    assert_eq!(
        windows
            .grids
            .iter()
            .filter(|((icon, _), _)| *icon == IconSlot::Mission(slot))
            .count(),
        3
    );
    let keys: Vec<_> = windows.items.keys().copied().collect();
    assert_eq!(keys.len(), 3);
    assert!(keys.iter().all(|key| key.icon == IconSlot::Mission(slot)));
    assert!(
        keys.iter()
            .map(|key| key.pool)
            .collect::<BTreeSet<_>>()
            .len()
            == 3
    );
    fill_grids_in(
        &fixture,
        &mut windows,
        &instance,
        root,
        IconSlot::Mission(slot),
        &[],
        &mut report,
    )
    .unwrap();
    assert!(
        !windows
            .grids
            .keys()
            .any(|(icon, _)| *icon == IconSlot::Mission(slot))
    );
    assert_eq!(
        windows.items.len(),
        3,
        "detached item windows remain available for reuse"
    );
}
