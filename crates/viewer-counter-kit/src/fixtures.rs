use replay_frame::frame::DecodedFrame;
use viewer_native_gui::native_windows::MAPICONS_GUI;

use crate::{
    AbilityGrid, ActionContent, Child, ColumnLayout, CounterArt, CounterDefines, CounterDisplay,
    CounterInputs, CounterMap, CounterSettings, GunCounter, LAND_WINDOW, LandCounter, Layouts,
    TextChild, counter_portraits, layouts::land_counter,
};
use content_reader_layers::ResolvedLayers;
use content_reader_gui::{flags::FlagSet, gui::native};
use content_reader_map::map::MapData;
use content_reader_gfx::gfx::SpriteSet;
use replay_data::{
    Hour,
    rows::{
        AirBaseRegistrationRow, AirBaseRow, AirWingRow, CountryLinkRow, LocalPlayerRow,
        ProvinceRow, TableId, TableRows,
    },
};
use replay_frame::frame::{Metrics, TableView};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use viewer_foundation::{
    mode_visibility::Visibility,
    perspective::{Choice, Country, Vision},
    templates,
};
use viewer_gpu::{
    camera::{Camera, MapDefines, MapSize},
    province_map::HeightField,
    province_schedule::Shared,
};
use viewer_native_gui::{
    lazy_icons::{LazyTextures, flag_set},
    native_windows::{NativeWindows, WindowCache},
    sprites::SpriteSheet,
    text,
};

pub fn counter_assets_fixture(
    shared: Arc<ResolvedLayers>,
    registry: native::resources::Registry,
    guis: native::Guis,
    layouts: Layouts,
    sheets: BTreeMap<String, SpriteSheet>,
) -> (CounterArt, text::Fonts) {
    use viewer_native_gui::{native_resources::GuiResources, sprites::Interface};
    let registry = Arc::new(registry);
    let sprites = SpriteSet::default();
    let decode = Default::default();
    let lazy = LazyTextures::new();
    let flags = flag_set(&shared, FlagSet::Small, [1, 1], |_| {}, &lazy).unwrap();
    let (portraits, _) = counter_portraits::load(&shared, &sprites, decode, &lazy).unwrap();
    let fonts = text::Fonts::load(
        &shared,
        &Interface {
            registry: Arc::clone(&registry),
            sprites,
            decode,
        },
        "english",
        &Default::default(),
        &mut Vec::new(),
        &mut Vec::new(),
    )
    .unwrap();
    let assets = CounterArt {
        air_sortie_defines: Err("unused in this fixture".into()),
        layouts,
        windows: NativeWindows::of(guis).unwrap(),
        gui: GuiResources::new(registry, b"english", [100, 100], decode),
        gui_assets: Default::default(),
        sheets,
        flags: flags.clone(),
        medium_flags: flags.clone(),
        large_flags: flags,
        types: templates::TypeAtlas::test_icons(&[]),
        portraits,
        window_cache: Arc::new(WindowCache::new(shared)),
        recorded_sheets: Default::default(),
        ability_sprites: Arc::new(Default::default()),
        land_action: None,
        gun_action: None,
        warnings: Vec::new(),
    };
    (assets, fonts)
}

pub fn layout_gui(bytes: &[u8]) -> native::Guis {
    native::Guis::new(vec![(MAPICONS_GUI.to_owned(), native::read(bytes))])
}

pub fn layouts() -> Layouts {
    let sprite = |name: &str, at: [f32; 2]| Child {
        sprite: name.into(),
        at,
        scale: 1.0,
        frame: 0,
        rotation: 0.0,
        centered: false,
        order: 0,
    };
    let text = |at: [f32; 2], max_width: f32| TextChild {
        font: Some("Main_14".into()),
        order: 0,
        at,
        max_width,
        align: text::Align::Centre,
        truncate: true,
    };
    Layouts {
        land: Ok(LandCounter {
            size: [76.0, 24.0],
            plate: sprite("GFX_onmap_unit_counter", [0.0, 0.0]),
            selected: sprite("", [0.0, 0.0]),
            overlay: sprite("GFX_onmap_unit_counter_overlay", [3.0, 3.0]),
            type_icon: sprite("GFX_tiled_window_transparent", [3.0, 3.0]),
            bar_org: Child {
                rotation: std::f32::consts::FRAC_PI_2,
                ..sprite("GFX_mapicon_unit_bar_org", [14.0, 8.0])
            },
            bar_str: Child {
                rotation: std::f32::consts::FRAC_PI_2,
                ..sprite("GFX_mapicon_unit_bar_str", [14.0, 12.0])
            },
            supply: Child {
                scale: 0.75,
                ..sprite("GFX_unitcounter_supply_status_strip", [24.0, -4.0])
            },
            attrition: sprite("GFX_unit_attrition", [-4.0, -1.0]),
            digin: sprite("GFX_onmap_digin_icon", [56.0, 8.0]),
            training: sprite("GFX_onmap_training_icon", [54.0, 8.0]),
            action: sprite("GFX_fixture_land_actions", [54.0, 7.0]),
            no_orders: sprite("GFX_mapicon_unit_no_orders", [-10.0, 8.0]),
            defending_area: sprite("GFX_mapicon_unit_area_defense", [-13.0, 6.0]),
            invalid_orders: sprite("GFX_mapicon_unit_invalid_orders", [-10.0, 8.0]),
            flag: sprite("GFX_mapicon_unit_flag_stripe", [5.0, 15.0]),
            large_flag: Child {
                scale: 0.69,
                ..sprite("GFX_mapicon_unit_large_flag_stripe", [4.0, 5.0])
            },
            count: text([36.0, 6.0], 20.0),
            hq_badge: sprite("GFX_hq_badge_bg", [-20.0, 0.0]),
            hq_offset: [-20.0, 0.0],
            portrait: Child {
                scale: 0.1,
                ..sprite("GFX_portrait_GER_oswald_lutz", [36.0, 3.0])
            },
            group_marker: sprite("GFX_UV_group_marker", [55.0, 2.0]),
            leader_enroute: sprite("GFX_leader_en_route", [-20.0, 0.0]),
        }),
        columns: Ok(ColumnLayout {
            stack_size: [58.0, 27.0],
            zoomed_in: [0.0, 20.0],
        }),
        gun: Ok(GunCounter {
            size: [58.0, 24.0],
            plate: sprite("GFX_onmap_unit_counter", [0.0, 0.0]),
            selected: sprite("", [0.0, 0.0]),
            group_marker: sprite("GFX_UV_group_marker", [55.0, 2.0]),
            overlay: sprite("GFX_onmap_unit_counter_overlay", [3.0, 3.0]),
            no_orders: sprite("GFX_mapicon_unit_no_orders", [-10.0, 8.0]),
            type_icon: sprite("GFX_tiled_window_transparent", [3.0, 3.0]),
            count: text([36.0, 6.0], 20.0),
            bar_str: Child {
                rotation: std::f32::consts::FRAC_PI_2,
                ..sprite("GFX_mapicon_unit_bar_str", [14.0, 12.0])
            },
            flag: sprite("GFX_mapicon_unit_flag_stripe", [5.0, 15.0]),
            large_flag: Child {
                scale: 0.69,
                ..sprite("GFX_mapicon_unit_large_flag_stripe", [4.0, 5.0])
            },
            supply: Child {
                scale: 0.75,
                ..sprite("GFX_unitcounter_supply_status_strip", [24.0, -4.0])
            },
            action: sprite("GFX_onmap_action_icon", [54.0, 7.0]),
        }),
        abilities: Some(AbilityGrid {
            at: [55.0, -9.0],
            slot: [25.0, 25.0],
            order: 3,
            item_size: [20.0, 20.0],
            icon_at: [0.0, 0.0],
            scale: 0.8,
            from_centre: true,
            centred: false,
        }),
        missing: Vec::new(),
        unsupported: Vec::new(),
    }
}

pub fn sheet(name: &str, frames: u32, size: [u32; 2]) -> SpriteSheet {
    SpriteSheet {
        name: name.into(),
        texture: 0,
        frames,
        size,
    }
}

pub fn action_gui(centered: bool) -> native::Guis {
    layout_gui(
        format!(
            r#"guiTypes={{
        containerWindowType={{name=unit_counter_navy}}
        containerWindowType={{name=naval_combat_mapicon}}
        containerWindowType={{name=unit_counter size={{width=81 height=29}}
            iconType={{name=other spriteType=GFX_other}}
            buttonType={{name=count_bg spriteType=GFX_synthetic_plate}}
            iconType={{name=onmap_action_icon spriteType=GFX_synthetic_actions
                position={{x=57 y=9}} scale=0.5 frame=12 centerPosition={}}}
            iconType={{name=counter_overlay spriteType=GFX_synthetic_overlay
                position={{x=3 y=4}}}}
        }}
    }}"#,
            if centered { "yes" } else { "no" }
        )
        .as_bytes(),
    )
}

pub fn wings_frame(flags: TableView) -> (DecodedFrame, Vision) {
    const HOUR: Hour = 60_832_633;
    let mut views = std::array::from_fn(|_| TableView::Unavailable {
        reason: "not needed for the wings box".into(),
    });
    views[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
            id: 100,
            canonical_slot: 0,
        }]));
    views[TableId::LocalPlayer.index()] =
        TableView::Complete(TableRows::LocalPlayer(vec![LocalPlayerRow {
            country: 100,
            ..Default::default()
        }]));
    views[TableId::Provinces.index()] =
        TableView::Complete(TableRows::Provinces(vec![ProvinceRow {
            id: 9,
            controller: 100,
            ..Default::default()
        }]));
    views[TableId::AirBases.index()] = TableView::Complete(TableRows::AirBases(vec![AirBaseRow {
        id: 0xB0,
        state: Some(7),
        capacity: 10,
        ..Default::default()
    }]));
    views[TableId::AirBaseRegistrations.index()] =
        TableView::Complete(TableRows::AirBaseRegistrations(vec![
            AirBaseRegistrationRow {
                base: 0xB0,
                slot: 0,
                capacity_raw: 100_000,
                wings: vec![0xC1],
                carrier: Default::default(),
                transport_planes: Default::default(),
                fuel_demand_raw: Default::default(),
                fuel_allocated_raw: Default::default(),
            },
        ]));
    views[TableId::AirWings.index()] = TableView::Complete(TableRows::AirWings(vec![AirWingRow {
        id: 0xC1,
        air_base: 0xB0,
        planes: 5,
        ..Default::default()
    }]));
    views[TableId::StateFlags.index()] = flags;
    let frame = DecodedFrame {
        branch: 1,
        hour: HOUR,
        tables: views.map(Into::into),
        metrics: Metrics::default(),
    };
    let links = BTreeMap::from([(100, 0)]);
    let tags = BTreeMap::from([(0, "GER".to_owned())]);
    let vision = Vision::read(
        &frame,
        &links,
        &tags,
        &Choice {
            country: Country::Recorded,
            fog: true,
        },
        Err("not needed for the air base box"),
    )
    .unwrap();
    (frame, vision)
}

pub fn counter_art_fixture(guis: native::Guis) -> (CounterArt, text::Fonts) {
    let mut layouts = layouts();
    layouts.land = land_counter(&guis, &mut Vec::new()).map_err(|error| error.to_string());
    let sheets: BTreeMap<_, _> = [
        ("GFX_synthetic_plate", 1, [81, 29], 1),
        ("GFX_synthetic_actions", 12, [144, 13], 2),
        ("GFX_synthetic_overlay", 1, [8, 6], 3),
    ]
    .into_iter()
    .map(|(name, frames, size, texture)| {
        (
            name.to_owned(),
            SpriteSheet {
                texture,
                ..sheet(name, frames, size)
            },
        )
    })
    .collect();
    let shared = Arc::new(ResolvedLayers::from_folders(&[]).unwrap());
    let registry = native::resources::Registry::from_files(vec![("synthetic.gfx".into(),
        native::sprites::read_sprites(b"spriteTypes={spriteType={name=GFX_synthetic_plate} spriteType={name=GFX_synthetic_actions} spriteType={name=GFX_synthetic_overlay}}"))]);
    let (mut assets, fonts) = counter_assets_fixture(shared, registry, guis, layouts, sheets);
    let child = &assets.layouts.land.as_ref().unwrap().action;
    assets.land_action = Some(ActionContent::load(
        child,
        LAND_WINDOW,
        assets.sheets.get(&child.sprite),
    ));
    (assets, fonts)
}

pub fn counter_map_fixture() -> CounterMap {
    CounterMap {
        naval_mission_allegiance_group_cutoff: Err("not needed".into()),
        recorded_labels: None,
        terrain: Default::default(),
        wing_types: Ok(Arc::new(Default::default())),
        port_mission_defines: viewer_unit_reads::port_mission_defines::Defines::read(
            &content_reader_defines::defines::Defines::new().unwrap(),
        ),
        anchors: Arc::new(Default::default()),
        default_anchors: Arc::new(Default::default()),
        site_icons: Default::default(),
        air_base_first_provinces: Default::default(),
        ports: Default::default(),
        kinds: Arc::new(Default::default()),
        bounds: Default::default(),
        routes: Default::default(),
        region_of: Default::default(),
        region_provinces: Default::default(),
        region_icons: Default::default(),
        region_names: Default::default(),
        ground_crew_defines: Err("unused in fixture".into()),
        air_log_timeout: Err("unused in fixture".into()),
        missing_standstill: Vec::new(),
        map: MapSize::TEST,
        aggression: Arc::new(Vec::new()),
        fog: Arc::new(Err("not needed".into())),
        defines: CounterDefines::FIXTURE,
        supply_grace: Err("not needed".into()),
        dig_in_factor: Err("unused in fixture".into()),
        attrition: Err("unused in fixture".into()),
        ability_comms_scaling: Err("unused in fixture".into()),
        ability_no_hq_scaling: Err("unused in fixture".into()),
        comms_max_distance: Err("not needed".into()),
        subunit_tokens: Err("not needed".into()),
        ideologies: Default::default(),
        stack_scales: Default::default(),
    }
}

pub fn with_inputs<R>(
    content: &CounterMap,
    assets: &CounterArt,
    fonts: &text::Fonts,
    run: impl FnOnce(CounterInputs<'_>) -> R,
) -> R {
    let map = MapData {
        width: 4,
        height: 4,
        province_ids: vec![0; 16],
        definitions: Default::default(),
        country_colors: Default::default(),
    };
    let localisation = Default::default();
    let schedule = Shared::default();
    let movement_history = Default::default();
    let inputs = CounterInputs {
        content,
        equipment: Err("unreached variant content"),
        assets,
        windows: &assets.windows,
        fonts,
        localisation: &localisation,
        recorded_language: "english",
        map: &map,
        hour: 0,
        camera: Camera {
            center: [2.0, 2.0],
            height: 100.0,
            map: MapSize::TEST,
            defines: MapDefines::test(),
            pose: viewer_gpu::camera::Pose::Map,
        },
        viewport: [400, 400],
        ui_scale: 1.0,
        movement: None,
        heights: HeightField {
            width: 4,
            height: 4,
            values: &[0; 16],
        },
        settings: CounterSettings::default(),
        display: &CounterDisplay::default(),
        selected: &BTreeSet::new(),
        selected_air: &[],
        selected_air_bases: &[],
        selected_fleets: &[],
        hovered: None,
        pressed: None,
        plate_hovered: None,
        schedule: &schedule,
        moved: &viewer_unit_reads::counter_move::NO_MOVED_HOURS,
        movement_history: &movement_history,
        visibility: Visibility::POLITICAL,
        art: 0,
    };
    run(inputs)
}

pub fn with_counter_inputs_fixture<R>(
    guis: native::Guis,
    configure: impl FnOnce(&mut CounterArt),
    configure_content: impl FnOnce(&mut CounterMap),
    run: impl FnOnce(CounterInputs<'_>) -> R,
) -> R {
    let (mut art, fonts) = counter_art_fixture(guis);
    let mut content = counter_map_fixture();
    configure(&mut art);
    configure_content(&mut content);
    with_inputs(&content, &art, &fonts, run)
}
