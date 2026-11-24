use super::NAVAL_COMBAT_WINDOW;
use viewer_unit_reads::counter_order;
use viewer_counter_kit::gui_windows::{BG_FRAME, FRAME_BTN, GRIDS, PROGRESS};

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
    let mut bases = [written(&held), written(&held)];
    adopt_countries(bases.iter_mut(), Some(&other));
    assert!(bases.iter().all(|base| Arc::ptr_eq(base.countries.as_ref().unwrap(), &held)));
    assert!(!bases[0].same(7, None, 0, false, Ok(&[]), 0, Some(&other)));
    adopt_countries(bases.iter_mut(), Some(&same));
    assert!(bases.iter().all(|base| Arc::ptr_eq(base.countries.as_ref().unwrap(), &same)));
    assert!(bases[1].same(7, None, 0, false, Ok(&[]), 0, Some(&same)));
}

#[test]
fn a_base_stands_once_written_twice_alike_and_keeps_counting_what_its_grid_left_out() {
    content_reader_script::test_game::enter();
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
    assert_eq!(base_due(None, false, false), BaseDue::Write { settled: false });
    assert_eq!(base_due(Some(&written(false, 0)), true, true), BaseDue::Write { settled: true });
    assert_eq!(base_due(Some(&written(true, 2)), true, true), BaseDue::Stands { overflow: 2 });
    assert_eq!(base_due(Some(&written(true, 0)), false, false), BaseDue::Write { settled: false });
    assert_eq!(base_due(Some(&written(true, 0)), true, false), BaseDue::Write { settled: false });

    let gui = GUI.replace(
        "max_slots_horizontal = 1 } } }",
        "max_slots_horizontal = 1 add_horizontal = no max_slots_vertical = 4 } } }",
    );
    let mut view = View::with_gui(&gui);
    let base = view.base(1);
    let hulls: Vec<_> = (0..5).map(|frame| view.hull(frame)).collect();
    assert_eq!(view.ports(base, &hulls), 1);
}

#[test]
fn a_paused_map_paints_a_replaced_item_sprite_s_frame_before_it_stops() {
    content_reader_script::test_game::enter();
    let mut view = View::new();
    let base = view.base(1);
    let plain = [view.hull(2).unwrap()];
    assert!(view.paint_until_settled(base, &plain) > 1);
    let mut writes = viewer_counter_kit::gui_windows::Writes::new(view.native.navy());
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

#[test]
fn an_undecided_roster_never_lets_its_base_stand() {
    content_reader_script::test_game::enter();
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

#[test]
fn a_base_that_keeps_failing_lets_a_static_map_stop_painting() {
    content_reader_script::test_game::enter();
    let mut view = View::new();
    let base = view.base(1);
    let gone = view.base(2);
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

#[test]
fn a_base_failing_after_it_made_a_fading_item_lets_a_static_map_stop_painting() {
    content_reader_script::test_game::enter();
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

#[test]
fn an_emptied_roster_with_a_looping_item_left_behind_lets_a_static_map_stop_painting() {
    content_reader_script::test_game::enter();
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

#[test]
fn an_allocation_mark_after_an_undecided_mark_gets_its_recovery_frames() {
    content_reader_script::test_game::enter();
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

#[test]
fn a_looping_item_a_full_grid_left_out_lets_a_static_map_stop_painting() {
    content_reader_script::test_game::enter();
    let gui = GUI.replace(
        "max_slots_horizontal = 1 } } }",
        "max_slots_horizontal = 1 add_horizontal = no max_slots_vertical = 4 } } }",
    );
    let mut view = View::with_gui(&gui);
    view.fixture.animated.insert(b"GFX_tiny".to_vec(), 0.125);
    let base = view.base(1);
    let mut items: Vec<_> = (0..4).map(|frame| view.hull(frame).unwrap()).collect();
    let mut writes = viewer_counter_kit::gui_windows::Writes::new(view.native.navy());
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

#[test]
fn a_base_leaving_an_overflowing_roster_reads_what_that_roster_put_in_its_grid() {
    content_reader_script::test_game::enter();
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
        let next: Result<&[viewer_counter_kit::gui_windows::Update], _> = if undecided { Err(&why) } else { Ok(&[]) };
        let (stood, ..) = view.paint_bases(&[(base, next)]);
        assert_eq!(stood, [false], "undecided {undecided}: written again");
    }
}

#[test]
fn a_withheld_looping_ship_lets_a_static_map_stop_painting() {
    content_reader_script::test_game::enter();
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

#[test]
fn a_naval_icon_kept_off_screen_asks_only_while_its_province_will_be_cleaned() {
    for (off_screen, cleans, asks) in [(false, false, false), (false, true, false), (true, true, true), (true, false, false)] {
        assert_eq!(keeps_asking(off_screen, cleans), asks, "off screen {off_screen}, cleans {cleans}");
    }
}
use super::{
    Admission, Arc, BTreeMap, BaseDue, BaseInputs, BaseWritten, COMBAT_KEPT,
    COUNTER_SOURCE_UNRECORDED, CombatIcon, CombatOwner, CombatPass, CounterColorMode, CounterReport,
    CounterSettings, Countries, Cow, Forces, GROWTH_FRAME, GridFill, IconSlot, Instance, ItemKey,
    ItemPool, Marked, NAVAL_BASE_WINDOW, NativeWindows, PLACE, Pool, Registered, ResultPass,
    Schedule, SlotId, TRANSITION, Tick, Windows, adopt_countries, allocate, base_due, base_items,
    combat_owners, combat_parts, combat_resolves, counter_window, fill_grids_in, fill_ports, frame,
    gui_windows, held_presentation, keeps_asking, kept_frame, native, native_gui, naval,
    naval_markers, planned_update, port_parts, result_empty, result_owners, settle_counters,
    skipped_counter, tick, transition_unknown, undecided_results, unknown_items, update_combat,
    update_item, update_or_withhold, visible_layout_kept,
};
use viewer_counter_kit::counter_groups::Level;
use std::collections::BTreeSet;
use viewer_runtime::geometry::contains;
use viewer_native_gui::icon_windows::{GUI_DELTA_CLAMP, visible_layout};
use viewer_native_gui::native_gui::PartKind;
use viewer_native_gui::native_gui::lifecycle::Call;
use viewer_gpu::province_schedule::Update as ScheduleUpdate;
#[cfg(test)]
use viewer_native_gui::test_support::part as test_support_part;
use viewer_native_gui::test_support::{Fixture, VIEWPORT};

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
    content_reader_script::test_game::enter();
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
        viewer_counter_kit::gui_windows::background_footprint(window, &layout)
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
    assert_eq!(footprints.len(), 3);
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

fn sea_plate(local: [i32; 2], count: usize, screen: [i32; 2]) -> viewer_counter_kit::AtSeaPlate {
    viewer_counter_kit::AtSeaPlate {
        member: 0,
        ordinal: 0,
        local,
        column: Arc::new(viewer_counter_kit::ColumnPlacement {
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

struct View {
    native: NativeWindows,
    windows: Windows,
    fixture: Fixture,
    time: f32,
}

type Shown = (u32, [f32; 2]);

type Update = Result<viewer_counter_kit::gui_windows::Update, Cow<'static, str>>;

impl View {
    fn new() -> Self {
        Self::with_gui(GUI)
    }

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

    fn base(&mut self, province: u32) -> SlotId {
        let base = self.native.naval_base().unwrap();
        let pool = self.windows.bases.as_mut().unwrap();
        let slot = pool.request(province).unwrap();
        pool.update(base, |_, _| Admission::Inside([100, 100]))
            .unwrap();
        slot
    }

    fn combat(&mut self, owner: CombatOwner) -> SlotId {
        let window = self.native.naval_combat();
        let pool = self.windows.combats.as_mut().unwrap();
        let slot = pool.request(owner).unwrap();
        pool.update(window, |_, _| Admission::Inside([300, 100]))
            .unwrap();
        slot
    }

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

    fn ports(&mut self, slot: SlotId, items: &[Update]) -> usize {
        let base = self.native.naval_base().unwrap();
        let items: Vec<viewer_counter_kit::gui_windows::Update> = items
            .iter()
            .map(|item| match item {
                Ok(update) => update.clone(),
                Err(why) => viewer_counter_kit::gui_windows::Update {
                    unknown: vec![(viewer_counter_kit::gui_windows::PLATE, why.to_string())],
                    ..viewer_counter_kit::gui_windows::Update::default()
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

    fn update(&mut self, slot: SlotId, update: viewer_counter_kit::gui_windows::Update, fills: Option<&[GridFill]>) {
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

    fn paint(&mut self, slot: SlotId, items: &[viewer_counter_kit::gui_windows::Update]) -> bool {
        self.paint_bases(&[(slot, Ok(items))]).1
    }

    fn paint_bases(
        &mut self,
        bases: &[(SlotId, Result<&[viewer_counter_kit::gui_windows::Update], &Cow<'static, str>>)],
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

    fn paint_until_settled(&mut self, slot: SlotId, items: &[viewer_counter_kit::gui_windows::Update]) -> usize {
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

    fn hull(&self, frame: i32) -> Update {
        let mut writes = viewer_counter_kit::gui_windows::Writes::new(self.native.navy());
        writes.frame(native::Kind::Icon, "type", frame);
        writes.wrote(viewer_counter_kit::gui_windows::PLATE);
        Ok(writes.update())
    }

    fn full_hull(&self, frame: i32) -> Update {
        let mut update = self.hull(frame)?;
        update.wrote.extend(viewer_counter_kit::gui_windows::MISSION_ITEM_KEPT);
        Ok(update)
    }

    fn engaged_rotation(&self, engaged: bool) -> viewer_counter_kit::gui_windows::Update {
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

    fn convoy(&self, frame: i32) -> Update {
        let mut writes = viewer_counter_kit::gui_windows::Writes::new(self.native.tiny().unwrap());
        writes.frame(native::Kind::Icon, "icon", frame);
        Ok(writes.update())
    }

    fn shown(&self, keys: &[ItemKey]) -> Vec<bool> {
        keys.iter()
            .map(|key| self.windows.items[key].instance.drawn())
            .collect()
    }
}

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

fn rotation() -> viewer_counter_kit::gui_windows::Update {
    viewer_counter_kit::gui_windows::Update {
        wrote: vec![BG_FRAME, PROGRESS, GRIDS, FRAME_BTN, PLACE],
        ..viewer_counter_kit::gui_windows::Update::default()
    }
}

fn hq_only() -> viewer_counter_kit::gui_windows::Update {
    viewer_counter_kit::gui_windows::Update {
        wrote: vec![FRAME_BTN],
        ..viewer_counter_kit::gui_windows::Update::default()
    }
}

fn refused(why: &str) -> viewer_counter_kit::gui_windows::Update {
    viewer_counter_kit::gui_windows::Update {
        wrote: vec![BG_FRAME, GRIDS, FRAME_BTN, PLACE],
        unknown: vec![(
            PROGRESS,
            format!("icon omitted: {NAVAL_COMBAT_WINDOW} does not update: {why}"),
        )],
        ..viewer_counter_kit::gui_windows::Update::default()
    }
}

fn gap(why: &'static str) -> Update {
    Err(Cow::Borrowed(why))
}

#[test]
fn a_combat_and_a_base_of_one_slot_number_keep_their_own_grids() {
    content_reader_script::test_game::enter();
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
    for _ in 0..5 {
        let fills = view.fills(vec![view.hull(1)], Vec::new());
        view.update(combat, rotation(), Some(&fills));
        view.tick(0.05);
    }
    assert_eq!(view.shown(&ports), [false, false]);
    assert_eq!(view.shown(&tiles), [true]);
}

#[test]
fn a_refused_port_item_keeps_its_slot_and_draws_nothing() {
    content_reader_script::test_game::enter();
    let mut view = View::new();
    let base = view.base(1);
    let step = |view: &mut View, items: &[Update]| {
        view.ports(base, items);
        view.tick(0.0);
        view.port_items(base)
    };
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
    assert_eq!(first[0], (1, valid[1].1));
    let items = [view.hull(2), gap("a missing operand")];
    let (missing, report) = step(&mut view, &items);
    assert_eq!(
        report.navy_counter_gaps["unit_counter_navy is not drawn: a missing operand"],
        1
    );
    assert_eq!(missing, [valid[0]]);
    let items = [view.hull(2), view.hull(1)];
    let (again, report) = step(&mut view, &items);
    assert!(report.navy_counter_gaps.is_empty(), "{report:?}");
    assert_eq!(again, [valid[0], (0, valid[1].1)]);
}

#[test]
fn refused_combat_tiles_and_convoy_additions_keep_their_slots_and_draw_nothing() {
    content_reader_script::test_game::enter();
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

#[test]
fn a_refused_combat_update_withholds_the_icon_until_a_rotation() {
    content_reader_script::test_game::enter();
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

#[test]
fn an_erased_icon_draws_nothing_while_its_hide_fade_runs() {
    content_reader_script::test_game::enter();
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
    view.tick(0.01);
    assert!(view.windows.combats.as_ref().unwrap().active());
    assert!(view.windows.bases.as_ref().unwrap().active());
    assert_eq!(view.shown(&ports), [true, true]);
    assert_eq!(view.shown(&tiles), [true]);
    assert_eq!(visible(&view), (0, 0));
    for _ in 0..5 {
        view.tick(0.05);
    }
    assert_eq!(view.shown(&ports), [false, false]);
    assert_eq!(view.shown(&tiles), [false]);
    assert_eq!(visible(&view), (0, 0));
}

#[test]
fn a_hanging_frame_animation_is_counted_where_its_item_draws() {
    content_reader_script::test_game::enter();
    let mut view = View::new();
    view.fixture.animated.insert(b"GFX_hull".to_vec(), 0.01);
    let base = view.base(1);
    view.ports(base, &[Ok(viewer_counter_kit::gui_windows::Update::default())]);
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

#[test]
fn items_appended_while_the_grid_read_is_refused_draw_nothing_until_it_is_read() {
    content_reader_script::test_game::enter();
    let mut view = View::new();
    let base = view.base(1);
    let items = [view.hull(2), view.hull(3)];
    view.ports(base, &items);
    view.tick(0.0);
    assert_eq!(frames(&view.port_items(base).0), [1, 2]);
    view.fixture = Fixture::new(&[("GFX_hull", 3, [60, 10])], &[]);
    view.fixture.session_sprites = true;
    view.ports(base, &items);
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

#[test]
fn a_call_that_does_not_apply_withholds_the_window_until_it_is_built_again() {
    content_reader_script::test_game::enter();
    let mut view = View::new();
    let combat = view.combat((4, 9));
    let broken = || viewer_counter_kit::gui_windows::Update::calls(vec![("not_an_element".to_owned(), Call::Show)]);
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
    let tile_grid = View::grid_path(view.native.naval_combat(), "attacker_grid");
    let tiles = view.windows.grids[&(IconSlot::Combat(combat), tile_grid)].clone();
    assert!(view.windows.items_withheld.why(&tiles[0]).is_some());
    assert!(view.windows.items_withheld.why(&tiles[1]).is_none());
}

#[test]
fn a_refused_base_window_counts_its_port_items() {
    content_reader_script::test_game::enter();
    let mut view = View::new();
    let base = view.base(1);
    let items = [view.hull(2), view.hull(3)];
    view.ports(base, &items);
    view.tick(0.0);
    view.fixture = Fixture::new(&[("GFX_hull", 3, [60, 10])], &[]);
    view.fixture.session_sprites = true;
    let (items, report) = view.port_items(base);
    assert!(items.is_empty());
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
#[test]
fn an_omitted_icon_is_withheld_until_a_decided_rotation() {
    content_reader_script::test_game::enter();
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
    let valid = pass(decided(&view, 2));
    present(&mut view, &valid);
    let (drawn, report) = view.combat_items(slot, 9);
    assert_eq!(frames(&drawn.unwrap()[0]), [1]);
    assert_eq!(report.unavailable_naval_combats, 0);
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
    let unplaced = CombatPass {
        icons: Vec::new(),
        ..pass(decided(&view, 2))
    };
    present(&mut view, &unplaced);
    assert!(view.combat_items(slot, 9).0.is_none());
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

#[test]
fn an_undecided_hq_frame_withholds_the_icon_until_it_is_decided() {
    content_reader_script::test_game::enter();
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

struct Synthetic {
    native: NativeWindows,
    fixture: Fixture,
    windows: Windows,
    key: ItemKey,
}

impl Synthetic {
    fn new() -> Self {
        let native = gui_windows::test_support::windows();
        let fixture = gui_windows::test_support::navy_fixture();
        let mut windows = Windows::default();
        windows.build(&native, VIEWPORT, &fixture).unwrap();
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

    fn item(
        &mut self,
        flotilla: &viewer_counter_kit::flotilla::Flotilla,
        settings: CounterSettings,
    ) -> (Option<String>, Option<u32>, Option<Vec<u8>>) {
        self.item_at(0, flotilla, settings)
    }

    fn item_at(
        &mut self,
        index: usize,
        flotilla: &viewer_counter_kit::flotilla::Flotilla,
        settings: CounterSettings,
    ) -> (Option<String>, Option<u32>, Option<Vec<u8>>) {
        let key = ItemKey { index, ..self.key };
        let navy = self.native.navy();
        let (update, _) = gui_windows::test_support::production(navy, flotilla, settings, &self.fixture);
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
                .map(|_| gui_windows::test_support::sprite_frame(&layout, "low_supply")),
            hull,
        )
    }

    fn counter(&mut self, flotilla: &viewer_counter_kit::flotilla::Flotilla, settings: CounterSettings) -> Option<String> {
        let navy = self.native.navy();
        let (update, _) = gui_windows::test_support::production(navy, flotilla, settings, &self.fixture);
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
    supply: viewer_unit_reads::counter_inputs::NavalSupply,
    byte: u16,
) -> viewer_counter_kit::flotilla::Flotilla {
    let mut item = gui_windows::test_support::port_flotilla();
    item.supply = supply;
    item.item.as_mut().unwrap().byte = byte;
    if byte < 250 {
        item.bars = viewer_counter_kit::flotilla::NavalBars::Hidden;
    }
    item
}

#[test]
fn a_refused_update_makes_its_known_writes_and_waits_only_for_its_unknown_part() {
    content_reader_script::test_game::enter();
    use viewer_unit_reads::counter_inputs::NavalSupply;
    let mut shipped = Synthetic::new();
    let country = CounterSettings {
        color_mode: CounterColorMode::Country,
        ..CounterSettings::default()
    };
    let own = |supply, byte| viewer_counter_kit::flotilla::Flotilla {
        owner: 0,
        logical: Some(0),
        ..supply_flotilla(supply, byte)
    };
    let (why, supply, _) = shipped.item(&own(NavalSupply::Write(100_000), 255), country);
    assert_eq!((why, supply), (None, None));
    let mut refused = own(NavalSupply::Write(12_000), 255);
    refused.logical = Some(5);
    let (why, supply, _) = shipped.item(&refused, country);
    let why = why.unwrap();
    assert!(why.starts_with("unit_counter_navy is not drawn: "), "{why}");
    assert_eq!(supply, Some(2));
    let (why, supply, _) = shipped.item(&own(NavalSupply::Retain, 200), country);
    assert_eq!((why, supply), (None, Some(2)));
    let (why, supply, _) = shipped.item(&own(NavalSupply::Write(100_000), 255), country);
    assert_eq!((why, supply), (None, None));
}

#[test]
fn partial_intel_supply_retains_fresh_or_last_written_visibility() {
    content_reader_script::test_game::enter();
    use viewer_unit_reads::counter_inputs::NavalSupply;
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

#[test]
fn an_undecidable_hull_withholds_the_window_until_a_hull_is_written() {
    content_reader_script::test_game::enter();
    use counter_order::HullIcon;
    let mut shipped = Synthetic::new();
    let settings = CounterSettings::default();
    let with_hull = |mut flotilla: viewer_counter_kit::flotilla::Flotilla, hull: HullIcon| {
        flotilla.cells.as_mut().unwrap().hull = hull;
        flotilla
    };
    let destroyer = || HullIcon::Resolved(Some(b"destroyer".to_vec()));
    let sprite = b"GFX_unit_destroyer_icon_medium_white".to_vec();
    let port = gui_windows::test_support::port_flotilla;
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
    assert_eq!(hull, Some(sprite.clone()));
    let (why, _, _) = shipped.item(&with_hull(port(), HullIcon::Resolved(None)), settings);
    assert!(why.is_some());
    let (why, _, hull) = shipped.item(&with_hull(port(), destroyer()), settings);
    assert_eq!((why, hull), (None, Some(sprite)));
    let at_sea = |hull| with_hull(gui_windows::test_support::at_sea_flotilla(false), hull);
    assert_eq!(shipped.counter(&at_sea(destroyer()), settings), None);
    assert!(
        shipped
            .counter(&at_sea(HullIcon::Ambiguous), settings)
            .is_some()
    );
    assert_eq!(shipped.counter(&at_sea(destroyer()), settings), None);
}

#[test]
fn the_windows_run_on_the_presentation_clock_they_are_given() {
    content_reader_script::test_game::enter();
    let mut view = View::new();
    view.fixture.animated.insert(b"GFX_hull".to_vec(), 0.0625);
    let base = view.base(1);
    view.ports(base, &[Ok(viewer_counter_kit::gui_windows::Update::default())]);
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
#[test]
fn only_an_engaged_rotation_repairs_the_background_frame() {
    content_reader_script::test_game::enter();
    let mut view = View::new();
    let owner = (4, 9);
    let slot = view.combat(owner);
    let step = |view: &mut View, update: viewer_counter_kit::gui_windows::Update| {
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

#[test]
fn an_omitted_icon_leaves_its_items_unknown_until_they_are_written() {
    content_reader_script::test_game::enter();
    use viewer_unit_reads::counter_inputs::NavalSupply;
    let mut shipped = Synthetic::new();
    let settings = CounterSettings::default();
    let (why, supply, _) =
        shipped.item(&supply_flotilla(NavalSupply::Write(100_000), 255), settings);
    assert_eq!((why, supply), (None, None));
    let cause = "icon omitted: convoy variant undecided: the NavalCombatShips table is \
                 truncated (3 rows unread)";
    shipped.omit(cause);
    let why = shipped.windows.items_withheld.why(&shipped.key).unwrap();
    assert!(why.contains(cause), "{why}");
    let (why, supply, _) = shipped.item(&supply_flotilla(NavalSupply::Retain, 200), settings);
    assert!(why.is_some());
    assert_eq!(supply, None);
    let (why, supply, _) =
        shipped.item(&supply_flotilla(NavalSupply::Write(12_000), 255), settings);
    assert_eq!((why, supply), (None, Some(2)));
}

#[test]
fn an_unrecorded_supply_ratio_is_repaired_only_by_a_written_one() {
    content_reader_script::test_game::enter();
    use viewer_unit_reads::counter_inputs::NavalSupply;
    let mut shipped = Synthetic::new();
    let settings = CounterSettings::default();
    let withheld = format!(
        "unit_counter_navy is not drawn: {}",
        gui_windows::SUPPLY_UNRECORDED
    );
    let (why, supply, _) =
        shipped.item(&supply_flotilla(NavalSupply::Write(12_000), 255), settings);
    assert_eq!((why, supply), (None, Some(2)));
    let (why, supply, _) = shipped.item(&supply_flotilla(NavalSupply::Unavailable, 255), settings);
    assert_eq!((why.as_deref(), supply), (Some(withheld.as_str()), Some(2)));
    let (why, _, _) = shipped.item(&supply_flotilla(NavalSupply::Retain, 200), settings);
    assert_eq!(why.as_deref(), Some(withheld.as_str()));
    let (why, supply, _) =
        shipped.item(&supply_flotilla(NavalSupply::Write(100_000), 255), settings);
    assert_eq!((why, supply), (None, None));
}

#[test]
fn an_item_first_built_after_undecided_grids_waits_for_its_kept_parts() {
    content_reader_script::test_game::enter();
    use viewer_unit_reads::counter_inputs::NavalSupply;
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
    let partial = supply_flotilla(NavalSupply::Retain, 200);
    let (first, _, _) = shipped.item_at(0, &partial, settings);
    let (second, _, _) = shipped.item_at(1, &partial, settings);
    assert!(first.is_some());
    let second = second.unwrap();
    assert!(second.contains(cause), "{second}");
    let (second, supply, _) = shipped.item_at(
        1,
        &supply_flotilla(NavalSupply::Write(12_000), 255),
        settings,
    );
    assert_eq!((second, supply), (None, Some(2)));
}

#[test]
fn only_a_defined_carrier_sprite_repairs_the_carrier_drawable() {
    content_reader_script::test_game::enter();
    let mut shipped = Synthetic::new();
    let settings = CounterSettings::default();
    let carrier = |carrier: BTreeSet<Option<(&'static str, u32)>>| {
        let mut item = gui_windows::test_support::port_flotilla();
        item.cells.as_mut().unwrap().carrier = carrier;
        item
    };
    let defined = || [Some(("GFX_mapicon_air_base", 1))].into();
    let undefined = || [Some(("GFX_mapicon_undefined_base", 1))].into();
    let (why, _, _) = shipped.item(&carrier(defined()), settings);
    assert_eq!(why, None);
    let (why, _, _) = shipped.item(
        &carrier([None, Some(("GFX_mapicon_air_base", 2))].into()),
        settings,
    );
    let unknown = why.unwrap();
    assert!(unknown.contains("air_base_icon is undecided"), "{unknown}");
    let (why, _, _) = shipped.item(&carrier(undefined()), settings);
    assert_eq!(why.as_deref(), Some(unknown.as_str()));
    let (why, _, _) = shipped.item(&carrier([None].into()), settings);
    assert_eq!(why, None);
    let (why, _, _) = shipped.item(&carrier(undefined()), settings);
    assert_eq!(why.as_deref(), Some(unknown.as_str()));
    let (why, _, _) = shipped.item(&carrier(defined()), settings);
    assert_eq!(why, None);
    let (why, _, _) = shipped.item(&carrier(undefined()), settings);
    assert_eq!(why, None);
}

#[test]
fn an_omitted_icon_leaves_its_items_carrier_drawable_unknown() {
    content_reader_script::test_game::enter();
    let mut shipped = Synthetic::new();
    let settings = CounterSettings::default();
    let carrier = |sprite: Option<&'static str>| {
        let mut item = gui_windows::test_support::port_flotilla();
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

#[test]
fn an_undecided_port_roster_leaves_its_items_unknown_until_they_are_written() {
    content_reader_script::test_game::enter();
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
        let items = [full(&view, 2), full(&view, 3)];
        view.ports(base, &items);
        view.tick(0.0);
        let (drawn, report) = view.port_items(base);
        assert_eq!(frames(&drawn), [1, 2]);
        assert!(report.navy_counter_gaps.is_empty(), "{report:?}");
    }
}

#[test]
fn a_skipped_counter_at_sea_waits_for_its_kept_parts() {
    content_reader_script::test_game::enter();
    use counter_order::HullIcon;
    let mut shipped = Synthetic::new();
    let settings = CounterSettings::default();
    let at_sea = |hull: &[u8]| {
        let mut flotilla = gui_windows::test_support::at_sea_flotilla(false);
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

#[test]
fn a_counter_an_incomplete_census_lacks_is_kept_unknown() {
    content_reader_script::test_game::enter();
    let mut shipped = Synthetic::new();
    let flotilla = gui_windows::test_support::at_sea_flotilla(false);
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
#[test]
fn a_combat_an_incomplete_census_lacks_keeps_its_icon_omitted() {
    content_reader_script::test_game::enter();
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

#[test]
fn a_slot_allocated_after_a_divergence_is_unknown() {
    content_reader_script::test_game::enter();
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
    assert_eq!(request(&mut view, (1, 1)), before);
    assert_eq!(withheld(&mut view, before, false), None);
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

#[test]
fn a_window_whose_tick_fails_has_unknown_transitions() {
    content_reader_script::test_game::enter();
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
    let pool = view.windows.combats.as_ref().unwrap();
    assert_eq!(
        pool.instance(combat)
            .unwrap()
            .longest_transition(native.naval_combat()),
        Ok(0.1)
    );
}

#[test]
fn the_pools_frame_wins_and_the_index_establishes_one_it_lacks() {
    content_reader_script::test_game::enter();
    use viewer_engagements::icon_frames::{CARRIED, Kept, NAVAL_NO_HISTORY};
    let view = View::with_gui(&GUI.replacen(
        r#"buttonType = { name = "bg_btn" quadTextureSprite = "GFX_bg" }"#,
        r#"buttonType = { name = "bg_btn" quadTextureSprite = "GFX_bg" frame = 2 }"#,
        1,
    ));
    let window = view.native.naval_combat();
    let owner = (4, 44);
    let base = viewer_counter_kit::gui_windows::Update::default();
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
        viewer_counter_kit::gui_windows::apply(&mut instance, window, &kept.calls, &fixture).unwrap();
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
    let kept = kept_frame(false, owner, Kept::Frame(2), window, base, &mut report);
    viewer_counter_kit::gui_windows::apply(&mut instance, window, &kept.calls, &fixture).unwrap();
    let layout = instance.layout(window, &fixture, None).unwrap().unwrap();
    let background = layout
        .parts
        .iter()
        .find(|part| part.path.ends_with("bg_btn"))
        .unwrap();
    assert_eq!(shown(background).0, 2);
}

#[test]
fn a_skipped_counter_at_sea_has_unknown_transitions() {
    content_reader_script::test_game::enter();
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
        &viewer_counter_kit::gui_windows::Update::default(),
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
    let why = windows.counters[&7].withheld.why(&()).unwrap();
    assert!(why.contains("a test"), "{why}");
}

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

#[test]
fn an_off_screen_province_keeps_its_icon_until_its_update() {
    content_reader_script::test_game::enter();
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
    schedule.advance([3], |_| true, false);
    assert!(owners(&mut view, &mut pool, &schedule, Some(&pass)).is_empty());
    let slot = pool.slot_of(&(3, 1)).unwrap();
    schedule.advance([3], |_| false, false);
    assert!(owners(&mut view, &mut pool, &schedule, None).is_empty());
    assert_eq!(pool.slot_of(&(3, 1)), Some(slot));
    schedule.advance([3], |_| false, false);
    assert_eq!(schedule.update(3), Some(ScheduleUpdate::OffScreen));
    let erased = owners(&mut view, &mut pool, &schedule, Some(&pass));
    assert_eq!(erased.len(), 1);
    assert_eq!(pool.slot_of(&(3, 1)), None);
    for _ in 0..100 {
        schedule.advance([3], |_| false, false);
        assert!(owners(&mut view, &mut pool, &schedule, Some(&pass)).is_empty());
        assert_eq!(pool.slot_of(&(3, 1)), None);
    }
}

#[test]
fn an_off_screen_stretch_leaves_the_round_robin_as_the_game_has_it() {
    content_reader_script::test_game::enter();
    let mut view = View::new();
    let window = view.native.clone();
    let window = window.naval_combat();
    let mut schedule = Schedule::default();
    let mut pool = view.windows.combats.take().unwrap();
    let mut report = CounterReport::default();
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
    let mut game = View::new().windows.combats.take().unwrap();
    game.request((5, 1)).unwrap();
    let expected = game.request((9, 3));
    assert_eq!(pool.slot_of(&(9, 3)), expected);
}

#[test]
fn a_lower_right_centred_scaled_result_button_is_placed_as_the_game_places_it() {
    content_reader_script::test_game::enter();
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
        viewer_counter_kit::gui_windows::background_footprint(window, &layout).unwrap()
    };
    let authored = footprint(&mut pool);
    let mut undeclared = BTreeMap::new();
    let update = gui_windows::result_calls(
        window,
        &fixture,
        Ok((viewer_counter_kit::gui_windows::NAVAL_RESULT_SPRITES[3], 1)),
        &mut undeclared,
    );
    assert!(undeclared.is_empty());
    let instance = pool.get(slot).unwrap();
    viewer_counter_kit::gui_windows::apply(instance, window, &update.calls, &fixture).unwrap();
    let built = footprint(&mut pool);
    assert_eq!(authored, [5.0, 10.0, 20.0, 16.0]);
    assert_eq!(built, [-10.0, -11.0, 24.0, 20.0]);
    assert!(update.wrote.contains(&gui_windows::RESULT_SPRITE));
}

#[test]
fn a_grown_item_updated_every_frame_is_known_again() {
    content_reader_script::test_game::enter();
    let mut view = View::with_gui(&GUI.replacen(
        r#"name = "unit_counter_navy" size = { width = 20 height = 10 }"#,
        r#"name = "unit_counter_navy" size = { width = 20 height = 10 }
        fade_type = linear fade_time = 100"#,
        1,
    ));
    let base = view.base(1);
    view.undecided_ports(base, "a test roster");
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

#[test]
fn a_late_convoy_addition_inherits_the_unknown_transition() {
    content_reader_script::test_game::enter();
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

#[test]
fn a_drop_down_only_transition_bounds_the_wait() {
    content_reader_script::test_game::enter();
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

#[test]
fn a_definitely_hidden_ideology_repairs_an_undecided_one() {
    content_reader_script::test_game::enter();
    let mut shipped = Synthetic::new();
    let settings = CounterSettings::default();
    let with = |ideology: Result<Option<i32>, String>, coarse: bool| viewer_counter_kit::flotilla::Flotilla {
        ideology,
        ..gui_windows::test_support::at_sea_flotilla(coarse)
    };
    let undecided = with(Err("CountryCivilWars is not recorded".into()), false);
    let why = shipped.counter(&undecided, settings).unwrap();
    assert!(why.contains("counter_ideology is undecided"), "{why}");
    assert_eq!(shipped.counter(&with(Ok(None), false), settings), None);
    assert!(shipped.counter(&undecided, settings).is_some());
    assert_eq!(shipped.counter(&with(Ok(None), true), settings), None);
    assert!(shipped.counter(&undecided, settings).is_some());
    assert_eq!(shipped.counter(&with(Ok(Some(2)), false), settings), None);
}

#[test]
fn a_definite_hide_repairs_an_uncertain_part() {
    content_reader_script::test_game::enter();
    use counter_order::HullIcon;
    let mut shipped = Synthetic::new();
    let settings = CounterSettings::default();
    let at_sea = |coarse| gui_windows::test_support::at_sea_flotilla(coarse);
    let unknown_bars = viewer_counter_kit::flotilla::Flotilla {
        bars: viewer_counter_kit::flotilla::NavalBars::Unknown("a member's ratios are not recorded".into()),
        ..at_sea(false)
    };
    let hidden_bars = viewer_counter_kit::flotilla::Flotilla {
        bars: viewer_counter_kit::flotilla::NavalBars::Hidden,
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
    let item = |bars| viewer_counter_kit::flotilla::Flotilla {
        bars,
        ..gui_windows::test_support::port_flotilla()
    };
    let (why, _, _) = shipped.item(
        &item(viewer_counter_kit::flotilla::NavalBars::Unknown("unrecorded".into())),
        settings,
    );
    assert!(why.is_some());
    let (why, _, _) = shipped.item(&item(viewer_counter_kit::flotilla::NavalBars::Hidden), settings);
    assert_eq!(why, None);
}

#[test]
fn a_result_presentation_counts_every_held_reference() {
    let convoy = naval_markers::ResultTables {
        provinces: BTreeMap::from([(10, vec![1])]),
        facts: BTreeMap::from([(1, naval_markers::convoy_result_for_test())]),
        undecided: None,
    };
    assert_eq!(
        held_presentation(&convoy, &[1, 2]),
        Ok((viewer_counter_kit::gui_windows::NAVAL_RESULT_SPRITES[2], 1))
    );
    assert_eq!(
        held_presentation(&convoy, &[1]),
        Ok((viewer_counter_kit::gui_windows::NAVAL_RESULT_SPRITES[3], 1))
    );
    let incomplete = naval_markers::ResultTables {
        undecided: Some("incomplete"),
        ..convoy
    };
    assert_eq!(held_presentation(&incomplete, &[1, 2]), Err("incomplete"));
}

#[test]
fn an_incomplete_result_census_keeps_its_icons_undecided() {
    content_reader_script::test_game::enter();
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

#[test]
fn a_failed_hull_selection_restores_the_remembered_unknown_sprite() {
    content_reader_script::test_game::enter();
    use counter_order::HullIcon;
    let mut shipped = Synthetic::new();
    let settings = CounterSettings::default();
    let hull = |hull| {
        let mut flotilla = gui_windows::test_support::at_sea_flotilla(false);
        flotilla.cells.as_mut().unwrap().hull = hull;
        flotilla
    };
    assert!(
        shipped
            .counter(&hull(HullIcon::Ambiguous), settings)
            .is_some()
    );
    assert_eq!(
        shipped.counter(&gui_windows::test_support::at_sea_flotilla(true), settings),
        None
    );
    let why = shipped.counter(
        &hull(HullIcon::Resolved(Some(b"battleship".to_vec()))),
        settings,
    );
    let why = why.expect("the unknown drawable is shown again");
    assert!(why.contains(gui_windows::HULL_AMBIGUOUS), "{why}");
    let diagnostics = CounterReport {
        navy_counter_gaps: BTreeMap::from([(why.into(), 1)]),
        ..CounterReport::default()
    };
    assert!(
        diagnostics
            .warnings()
            .any(|line| line.contains(gui_windows::HULL_AMBIGUOUS))
    );
    assert!(
        shipped
            .counter(&hull(HullIcon::Resolved(None)), settings)
            .is_some()
    );
    assert_eq!(
        shipped.counter(
            &hull(HullIcon::Resolved(Some(b"destroyer".to_vec()))),
            settings
        ),
        None
    );
}

#[test]
fn regional_grids_keep_three_item_pools_separate_and_clear_together() {
    content_reader_script::test_game::enter();
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
        Ok(viewer_counter_kit::gui_windows::Update {
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
