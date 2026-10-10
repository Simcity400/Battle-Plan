use super::super::gui_windows::tests::{Fixture, VIEWPORT};
use super::super::gui_windows::{BG_FRAME, FRAME_BTN, GRIDS, PROGRESS};
use super::*;
use crate::map::native_gui::PartKind;
use crate::map::native_gui::lifecycle::Call;

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
    engine::executable_tables::initialize_for_tests()
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
            .is_some_and(|rect| !crate::combat::contains(rect, balance_point))
    );
    assert!(footprints.iter().any(|(hit, clickable)| *clickable
        && hit.supported
        && crate::combat::contains(hit.rect, balance_point)));
    assert!(footprints.iter().any(|(hit, clickable)| !*clickable
        && hit.supported
        && crate::combat::contains(hit.rect, balance_point)));
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
            .any(|(hit, _)| hit.supported && crate::combat::contains(hit.rect, balance_point))
    );
    assert!(!footprints.iter().any(|(hit, clickable)| *clickable
        && hit.supported
        && crate::combat::contains(hit.rect, balance_point)));
}

fn sea_plate(local: [i32; 2], count: usize, screen: [i32; 2]) -> land::AtSeaPlate {
    land::AtSeaPlate {
        member: 0,
        ordinal: 0,
        local,
        column: Arc::new(land::ColumnPlacement {
            hovered: false,
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
    engine::executable_tables::initialize_for_tests()
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
    engine::executable_tables::initialize_for_tests()
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
    engine::executable_tables::initialize_for_tests()
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
    /// plate colour the hour lacks, every other write made.
    fn ports(&mut self, slot: SlotId, items: &[Update]) {
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
        fill_ports(
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
    engine::executable_tables::initialize_for_tests()
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
    engine::executable_tables::initialize_for_tests()
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
    engine::executable_tables::initialize_for_tests()
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
    engine::executable_tables::initialize_for_tests()
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
    engine::executable_tables::initialize_for_tests()
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
    engine::executable_tables::initialize_for_tests()
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
    engine::executable_tables::initialize_for_tests()
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
    engine::executable_tables::initialize_for_tests()
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
    engine::executable_tables::initialize_for_tests()
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
    engine::executable_tables::initialize_for_tests()
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
    engine::executable_tables::initialize_for_tests()
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
    engine::executable_tables::initialize_for_tests()
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
        let hull = gui_windows::tests::part(&layout, "type").map(|part| match &part.kind {
            native_gui::PartKind::Sprite { sprite, .. } => sprite.clone(),
            other => panic!("type draws {other:?}"),
        });
        (
            self.windows.items_withheld.why(&key).map(str::to_owned),
            gui_windows::tests::part(&layout, "low_supply")
                .map(|_| gui_windows::tests::sprite_frame(&layout, "low_supply")),
            hull,
        )
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

/// Supply writes require full representative intel. A retained lower-intel
/// update leaves the prior supply drawable in place; a later plate write
/// repairs the withheld state.
#[test]
fn a_refused_update_makes_its_known_writes_and_waits_only_for_its_unknown_part() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::map::counter_inputs::NavalSupply;
    let mut shipped = Synthetic::new();
    let country = CounterSettings {
        color_mode: CounterColorMode::Country,
        ..CounterSettings::default()
    };
    let own = |supply, byte| {
        let mut item = gui_windows::tests::port_flotilla();
        item.owner = 0;
        item.logical = Some(0);
        item.supply = supply;
        item.item.as_mut().unwrap().byte = byte;
        if byte < 250 {
            item.bars = naval::NavalBars::Hidden;
        }
        item
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

/// A hull the recording cannot decide (a member's `ShipCounterInputs` unrecorded)
/// establishes none of the retaining returns. A port item or at-sea counter is
/// withheld and counted under its cause ([`gui_windows::TYPE_SPRITE`]), while its
/// window keeps its drawable, until selection writes the sprite again. An invalid
/// definition is a proven no-op and does not replace it.
#[test]
fn an_undecidable_hull_withholds_the_window_until_a_hull_is_written() {
    engine::executable_tables::initialize_for_tests()
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
    let native = shipped.native.clone();
    let navy = native.navy();
    let at_sea = |hull| with_hull(gui_windows::tests::at_sea_flotilla(false), hull);
    let withheld = |shipped: &mut Synthetic, flotilla: &naval::Flotilla| {
        let (update, _) =
            gui_windows::tests::production(navy, flotilla, settings, &shipped.fixture);
        counter_window(
            &mut shipped.windows,
            navy,
            flotilla.source,
            &sea_plate([0; 2], 1, [0; 2]),
            false,
            &update,
            &shipped.fixture,
        )
        .unwrap();
        shipped.windows.counters[&flotilla.source]
            .withheld
            .why(&())
            .map(str::to_owned)
    };
    assert_eq!(withheld(&mut shipped, &at_sea(destroyer())), None);
    assert!(withheld(&mut shipped, &at_sea(HullIcon::Ambiguous)).is_some());
    assert_eq!(withheld(&mut shipped, &at_sea(destroyer())), None);
}

/// Window graphics time and frame step are separate inputs. Fixed-step
/// presentation supplies both explicitly; a clock jump advances only the
/// clock-dependent state reached by the lifecycle.
#[test]
fn the_windows_run_on_the_presentation_clock_they_are_given() {
    engine::executable_tables::initialize_for_tests()
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
            grid,
            &view.fixture,
            VIEWPORT,
            &mut report,
        )
        .unwrap();
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
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use gui_windows::{CombatProgress, CombatState, Engaged, Rotation};
    let mut view = View::new();
    let owner = (4, 9);
    let slot = view.combat(owner);
    let native = view.native.clone();
    let window = native.naval_combat();
    let decided = |view: &View, engaged: bool| {
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
        // As the preparation adds them: a small icon's grids are cleared and not filled.
        let mut update = gui_windows::combat_calls(window, &state, &view.fixture);
        update.wrote.extend([GRIDS, PLACE]);
        update
    };
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
    let engaged = decided(&view, true);
    assert_eq!(step(&mut view, engaged), None);
    let cause = "icon omitted: convoy variant undecided: the NavalCombatShips table is \
                 truncated (3 rows unread)";
    let omitted = CombatIcon::omitted(owner, Some(([0.0; 3], 0)), true, cause).update;
    view.update(slot, omitted, None);
    view.tick(0.0);
    assert!(view.combat_items(slot, 9).0.is_none());
    let unengaged = decided(&view, false);
    assert!(!unengaged.wrote.contains(&BG_FRAME));
    assert_eq!(step(&mut view, unengaged).as_deref(), Some(cause));
    assert!(view.combat_items(slot, 9).0.is_none());
    let engaged = decided(&view, true);
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
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::map::counter_inputs::NavalSupply;
    let mut shipped = Synthetic::new();
    let settings = CounterSettings::default();
    let tile = |supply, byte| {
        let mut item = gui_windows::tests::port_flotilla();
        item.supply = supply;
        item.item.as_mut().unwrap().byte = byte;
        if byte < 250 {
            item.bars = naval::NavalBars::Hidden;
        }
        item
    };
    let (why, supply, _) = shipped.item(&tile(NavalSupply::Write(100_000), 255), settings);
    assert_eq!((why, supply), (None, None));
    // The icon's update is omitted: the tile is withheld with it.
    let cause = "icon omitted: convoy variant undecided: the NavalCombatShips table is \
                 truncated (3 rows unread)";
    shipped.omit(cause);
    let why = shipped.windows.items_withheld.why(&shipped.key).unwrap();
    assert!(why.contains(cause), "{why}");
    // Rebuilt at partial intel: supply is not written, so it stays unknown.
    let (why, supply, _) = shipped.item(&tile(NavalSupply::Retain, 200), settings);
    assert!(why.is_some());
    assert_eq!(supply, None);
    // Full intel writes it: drawn at the very-low frame.
    let (why, supply, _) = shipped.item(&tile(NavalSupply::Write(12_000), 255), settings);
    assert_eq!((why, supply), (None, Some(2)));
}

/// A received-supply ratio the recording lacks decides neither of
/// the `low_supply` writes. The item keeps what its window
/// showed and is withheld, counted by why ([`gui_windows::SUPPLY_UNRECORDED`]); a retained
/// (partial-intel) update keeps it withheld, and only a written ratio repairs it.
#[test]
fn an_unrecorded_supply_ratio_is_repaired_only_by_a_written_one() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::map::counter_inputs::NavalSupply;
    let mut shipped = Synthetic::new();
    let settings = CounterSettings::default();
    let tile = |supply, byte| {
        let mut item = gui_windows::tests::port_flotilla();
        item.supply = supply;
        item.item.as_mut().unwrap().byte = byte;
        if byte < 250 {
            item.bars = naval::NavalBars::Hidden;
        }
        item
    };
    let withheld = format!(
        "unit_counter_navy is not drawn: {}",
        gui_windows::SUPPLY_UNRECORDED
    );
    let (why, supply, _) = shipped.item(&tile(NavalSupply::Write(12_000), 255), settings);
    assert_eq!((why, supply), (None, Some(2)));
    // No hide is made: the glyph stands, withheld.
    let (why, supply, _) = shipped.item(&tile(NavalSupply::Unavailable, 255), settings);
    assert_eq!((why.as_deref(), supply), (Some(withheld.as_str()), Some(2)));
    let (why, _, _) = shipped.item(&tile(NavalSupply::Retain, 200), settings);
    assert_eq!(why.as_deref(), Some(withheld.as_str()));
    let (why, supply, _) = shipped.item(&tile(NavalSupply::Write(100_000), 255), settings);
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
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::map::counter_inputs::NavalSupply;
    let mut shipped = Synthetic::new();
    let settings = CounterSettings::default();
    let tile = |supply, byte| {
        let mut item = gui_windows::tests::port_flotilla();
        item.supply = supply;
        item.item.as_mut().unwrap().byte = byte;
        if byte < 250 {
            item.bars = naval::NavalBars::Hidden;
        }
        item
    };
    let (why, _, _) = shipped.item_at(0, &tile(NavalSupply::Write(100_000), 255), settings);
    assert_eq!(why, None);
    let cause = "icon omitted: convoy variant undecided: the NavalCombatShips table is \
                 truncated (3 rows unread)";
    shipped.omit(cause);
    // Two tiles at partial intel: the second is first built now.
    let partial = tile(NavalSupply::Retain, 200);
    let (first, _, _) = shipped.item_at(0, &partial, settings);
    let (second, _, _) = shipped.item_at(1, &partial, settings);
    assert!(first.is_some());
    let second = second.unwrap();
    assert!(second.contains(cause), "{second}");
    // Full intel writes its supply: drawn.
    let (second, supply, _) = shipped.item_at(1, &tile(NavalSupply::Write(12_000), 255), settings);
    assert_eq!((second, supply), (None, Some(2)));
}

/// Carrier sprite selection resolves its requested name through the registry.
/// Unavailable selection retains the old drawable and its named refusal.
#[test]
fn only_a_defined_carrier_sprite_repairs_the_carrier_drawable() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut shipped = Synthetic::new();
    let settings = CounterSettings::default();
    let carrier = |carrier: std::collections::BTreeSet<Option<(&'static str, u32)>>| {
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
    engine::executable_tables::initialize_for_tests()
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
    engine::executable_tables::initialize_for_tests()
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
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use counter_order::HullIcon;
    let mut shipped = Synthetic::new();
    let settings = CounterSettings::default();
    let native = shipped.native.clone();
    let navy = native.navy();
    let at_sea = |hull: &[u8]| {
        let mut flotilla = gui_windows::tests::at_sea_flotilla(false);
        flotilla.cells.as_mut().unwrap().hull = HullIcon::Resolved(Some(hull.to_vec()));
        flotilla
    };
    let update = |shipped: &mut Synthetic, flotilla: &naval::Flotilla| {
        let (update, _) =
            gui_windows::tests::production(navy, flotilla, settings, &shipped.fixture);
        counter_window(
            &mut shipped.windows,
            navy,
            flotilla.source,
            &sea_plate([0; 2], 1, [0; 2]),
            false,
            &update,
            &shipped.fixture,
        )
        .unwrap();
        shipped.windows.counters[&flotilla.source]
            .withheld
            .why(&())
            .map(str::to_owned)
    };
    let destroyer = at_sea(b"destroyer");
    assert_eq!(update(&mut shipped, &destroyer), None);
    skipped_counter(
        &mut shipped.windows,
        destroyer.source,
        "its task force is not placed: its route is not recorded",
    );
    let counter = &shipped.windows.counters[&destroyer.source];
    let why = counter.withheld.why(&()).unwrap().to_owned();
    assert!(why.contains("its route is not recorded"), "{why}");
    assert!(update(&mut shipped, &at_sea(b"cruiser")).is_some());
    assert_eq!(update(&mut shipped, &destroyer), None);
}

/// A counter at sea whose source an incomplete TaskForces census lacks may still stand and
/// be updated by the game: it is kept, withheld until its next update writes the parts that
/// update may keep, while a complete census that lacks its source destroys it.
#[test]
fn a_counter_an_incomplete_census_lacks_is_kept_unknown() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut shipped = Synthetic::new();
    let native = shipped.native.clone();
    let navy = native.navy();
    let flotilla = gui_windows::tests::at_sea_flotilla(false);
    let (update, _) = gui_windows::tests::production(
        navy,
        &flotilla,
        CounterSettings::default(),
        &shipped.fixture,
    );
    counter_window(
        &mut shipped.windows,
        navy,
        flotilla.source,
        &sea_plate([0; 2], 1, [0; 2]),
        false,
        &update,
        &shipped.fixture,
    )
    .unwrap();
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
    engine::executable_tables::initialize_for_tests()
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
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use gui_windows::{CombatProgress, CombatState, Engaged, Rotation};
    let mut view = View::new();
    let native = view.native.clone();
    let window = native.naval_combat();
    let rotate = |view: &View, engaged: bool| {
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
        let mut update = gui_windows::combat_calls(window, &state, &view.fixture);
        update.wrote.extend([GRIDS, PLACE]);
        update
    };
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
        let update = rotate(view, engaged);
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
    engine::executable_tables::initialize_for_tests()
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

/// Presentation-dirty state can retain an off-screen icon until the rotating
/// province update phase reaches and clears it.
#[test]
fn a_dirty_province_keeps_its_icon_off_screen() {
    let inside = |point: [i32; 2]| point[0] >= 0;
    assert_eq!(
        admission(false, [5, 5], inside),
        (Admission::Inside([5, 5]), false)
    );
    assert_eq!(
        admission(true, [5, 5], inside),
        (Admission::Inside([5, 5]), false)
    );
    assert_eq!(
        admission(false, [-50, 5], inside),
        (Admission::Outside, false)
    );
    assert_eq!(
        admission(true, [-50, 5], inside),
        (Admission::Inside([-50, 5]), true)
    );
    // Truncate toward zero; use 0x80000000 out of range or for a NaN.
    assert_eq!(truncate_int(-2.7), -2);
    assert_eq!(truncate_int(f32::NAN), i32::MIN);
    assert_eq!(truncate_int(3.0e9), i32::MIN);
}

/// An unengaged rotating icon keeps its pool slot's `bg_btn` frame; the game
/// sets it only while engaged. The pool's frame wins when established for this
/// owner. Otherwise the engaged index's carried frame establishes it and is
/// counted as carried; any other answer uses the binding's initial frame and
/// is counted as not from history.
#[test]
fn the_pools_frame_wins_and_the_index_establishes_one_it_lacks() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::combat::icon_frames::{CARRIED, Kept, NAVAL_NO_HISTORY};
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
    engine::executable_tables::initialize_for_tests()
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
    engine::executable_tables::initialize_for_tests()
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
    assert_eq!(
        schedule.update(3),
        Some(crate::map::province_schedule::Update::OffScreen)
    );
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
    engine::executable_tables::initialize_for_tests()
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
    for _ in 0..30 {
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
    engine::executable_tables::initialize_for_tests()
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
    engine::executable_tables::initialize_for_tests()
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
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut view = View::with_gui(&GUI.replacen(
        r#"name = "GFX_mapicon_tiny_unit_counter"
        size = { width = 10 height = 10 }"#,
        r#"name = "GFX_mapicon_tiny_unit_counter"
        size = { width = 10 height = 10 } fade_type = linear fade_time = 100"#,
        1,
    ));
    let combat = view.combat((1, 9));
    unknown_items(&mut view.windows, IconSlot::Combat(combat), "a test update");
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
    engine::executable_tables::initialize_for_tests()
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
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut shipped = Synthetic::new();
    let settings = CounterSettings::default();
    let native = shipped.native.clone();
    let navy = native.navy();
    let update = |shipped: &mut Synthetic, flotilla: &naval::Flotilla| {
        let (update, _) =
            gui_windows::tests::production(navy, flotilla, settings, &shipped.fixture);
        counter_window(
            &mut shipped.windows,
            navy,
            flotilla.source,
            &sea_plate([0; 2], 1, [0; 2]),
            false,
            &update,
            &shipped.fixture,
        )
        .unwrap();
        shipped.windows.counters[&flotilla.source]
            .withheld
            .why(&())
            .map(str::to_owned)
    };
    let with = |ideology: Result<Option<i32>, String>, coarse: bool| naval::Flotilla {
        ideology,
        ..gui_windows::tests::at_sea_flotilla(coarse)
    };
    let undecided = with(Err("CountryCivilWars is not recorded".into()), false);
    let why = update(&mut shipped, &undecided).unwrap();
    assert!(why.contains("counter_ideology is undecided"), "{why}");
    // Err -> Ok(None): drawn.
    assert_eq!(update(&mut shipped, &with(Ok(None), false)), None);
    // Err -> coarse: drawn.
    assert!(update(&mut shipped, &undecided).is_some());
    assert_eq!(update(&mut shipped, &with(Ok(None), true)), None);
    // A decided overlay shows it and is known too.
    assert!(update(&mut shipped, &undecided).is_some());
    assert_eq!(update(&mut shipped, &with(Ok(Some(2)), false)), None);
}

/// A definite hide clears an earlier refusal for the part it hides: unknown bars
/// followed by a coarse or hidden-bars update; an undecided hull followed by a
/// coarse update; or a mission item's unknown bars followed by hidden bars.
#[test]
fn a_definite_hide_repairs_an_uncertain_part() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use counter_order::HullIcon;
    let mut shipped = Synthetic::new();
    let settings = CounterSettings::default();
    let native = shipped.native.clone();
    let navy = native.navy();
    let update = |shipped: &mut Synthetic, flotilla: &naval::Flotilla| {
        let (update, _) =
            gui_windows::tests::production(navy, flotilla, settings, &shipped.fixture);
        counter_window(
            &mut shipped.windows,
            navy,
            flotilla.source,
            &sea_plate([0; 2], 1, [0; 2]),
            false,
            &update,
            &shipped.fixture,
        )
        .unwrap();
        shipped.windows.counters[&flotilla.source]
            .withheld
            .why(&())
            .map(str::to_owned)
    };
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
    assert!(update(&mut shipped, &unknown_bars).is_some());
    assert_eq!(update(&mut shipped, &at_sea(true)), None);
    assert!(update(&mut shipped, &unknown_bars).is_some());
    assert_eq!(update(&mut shipped, &hidden_bars), None);
    assert!(update(&mut shipped, &undecided_hull).is_some());
    assert_eq!(update(&mut shipped, &at_sea(true)), None);
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
    engine::executable_tables::initialize_for_tests()
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
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use counter_order::HullIcon;
    let mut shipped = Synthetic::new();
    let settings = CounterSettings::default();
    let native = shipped.native.clone();
    let navy = native.navy();
    let update = |shipped: &mut Synthetic, flotilla: &naval::Flotilla| {
        let (update, _) =
            gui_windows::tests::production(navy, flotilla, settings, &shipped.fixture);
        counter_window(
            &mut shipped.windows,
            navy,
            flotilla.source,
            &sea_plate([0; 2], 1, [0; 2]),
            false,
            &update,
            &shipped.fixture,
        )
        .unwrap();
        shipped.windows.counters[&flotilla.source]
            .withheld
            .why(&())
            .map(str::to_owned)
    };
    let hull = |hull| {
        let mut flotilla = gui_windows::tests::at_sea_flotilla(false);
        flotilla.cells.as_mut().unwrap().hull = hull;
        flotilla
    };
    assert!(update(&mut shipped, &hull(HullIcon::Ambiguous)).is_some());
    assert_eq!(
        update(&mut shipped, &gui_windows::tests::at_sea_flotilla(true)),
        None
    );
    // A hull whose sprite the content does not define: the selection keeps the drawable.
    let why = update(
        &mut shipped,
        &hull(HullIcon::Resolved(Some(b"battleship".to_vec()))),
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
    assert!(update(&mut shipped, &hull(HullIcon::Resolved(None))).is_some());
    // A defined hull replaces it.
    assert_eq!(
        update(
            &mut shipped,
            &hull(HullIcon::Resolved(Some(b"destroyer".to_vec())))
        ),
        None
    );
}
