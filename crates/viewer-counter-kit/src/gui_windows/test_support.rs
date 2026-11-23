use super::{NavyArt, NavyContext, NavyState, Update, navy_calls};
use crate::{CounterDefines, CounterSettings, Countries, UiColors, type_tint};
use content_reader_core::gui::native;
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use viewer_foundation::allegiance::Allegiance;
use viewer_native_gui::native_gui::PartKind;
use viewer_native_gui::native_windows::NativeWindows;
use viewer_native_gui::native_gui::{Layout, Texture};
use viewer_native_gui::test_support::{
    Fixture, SYNTHETIC_NAVAL_GUI, SYNTHETIC_NAVY_GUI, VIEWPORT, part,
};
use viewer_runtime::unit_handle::Unit;
use viewer_unit_reads::{counter_inputs, counter_order};

pub fn windows() -> NativeWindows {
    NativeWindows::read(
        format!("{SYNTHETIC_NAVY_GUI}\n{SYNTHETIC_NAVAL_GUI}").as_bytes(),
        VIEWPORT,
    )
    .unwrap()
}

pub fn sprite_frame(layout: &Layout, name: &str) -> u32 {
    match part(layout, name).unwrap().kind {
        PartKind::Sprite { uv, offsets, .. } => (offsets[0] / uv[2]).round() as u32,
        _ => panic!("{name} draws no sprite frame"),
    }
}

pub fn navy_fixture() -> Fixture {
    Fixture::new(
        &[
            ("GFX_fixture_navy_plate", 3, [246, 45]),
            ("GFX_fixture_group", 1, [7, 36]),
            ("GFX_fixture_overlay", 1, [72, 34]),
            ("GFX_fixture_ideology", 4, [288, 34]),
            ("GFX_fixture_capital", 1, [9, 11]),
            ("GFX_fixture_mission", 8, [128, 16]),
            ("GFX_fixture_org", 1, [3, 17]),
            ("GFX_fixture_strength", 1, [3, 17]),
            ("GFX_fixture_supply", 4, [80, 16]),
            ("GFX_fixture_pride", 1, [16, 13]),
            ("GFX_mapicon_air_base", 11, [352, 32]),
            ("GFX_fixture_no_orders", 1, [12, 12]),
            ("GFX_fixture_invalid_orders", 1, [12, 12]),
            ("GFX_general_portrait", 1, [20, 20]),
            ("GFX_fixture_hull_placeholder", 1, [30, 12]),
            ("GFX_fixture_flag", 1, [10, 7]),
            ("GFX_fixture_large_flag", 1, [10, 7]),
            ("GFX_unit_destroyer_icon_medium_white", 2, [68, 15]),
            ("GFX_unit_submarine_icon_medium_white", 2, [68, 15]),
            ("GFX_unit_no_intel_icon_medium_white", 2, [68, 15]),
            ("GFX_fixture_selected", 1, [82, 45]),
        ],
        &[],
    )
}

struct Art;

impl NavyArt for Art {
    fn flag(&self, slot: u32, medium: bool) -> Option<Texture> {
        Some(Texture {
            texture: 1_000 + slot as usize + if medium { 500 } else { 0 },
            uv: [0.0, 0.0, 0.5, 0.5],
            size: [10, 7],
        })
    }
}

pub(super) fn countries() -> Countries {
    Countries {
        index: None,
        tags: BTreeMap::from([(0, "AAA".to_owned()), (5, "BBB".to_owned())]),
        flag_tags: BTreeMap::new(),
        sides: BTreeMap::from([(0, Allegiance::Ours), (5, Allegiance::Enemy)]),
        groups: BTreeMap::new(),
        ui_colors: UiColors {
            colors: BTreeMap::from([(0, Some([0.1, 0.2, 0.3, 1.0]))]),
            inheritance: viewer_foundation::color_inheritance::Inheritance::default(),
            gap: "CountryAppearance has no row for it".into(),
            unavailable: None,
        },
    }
}

pub fn at_sea_flotilla(coarse: bool) -> crate::flotilla::Flotilla {
    crate::flotilla::Flotilla {
        source: 7,
        members: Arc::from([7]),
        owner: 0,
        logical: Some(0),
        type_tint: Some(type_tint::Side::Friend),
        side: Ok(Allegiance::Ours),
        flag_owner: Some(if coarse { 0 } else { 5 }),
        port: false,
        coarse,
        ships: Some(12),
        hidden: false,
        visible_member: true,
        cells: Some(crate::flotilla::NavalCells {
            capitals: Some(2),
            hull: counter_order::HullIcon::Resolved(Some(b"destroyer".to_vec())),
            carrier: [Some(("GFX_mapicon_air_base", 1))].into(),
        }),
        bars: crate::flotilla::NavalBars::Fill(16.0 / 17.0, 16.0 / 17.0),
        supply: counter_inputs::NavalSupply::Write(12_000),
        status: counter_inputs::NavalStatus {
            no_orders: Some(false),
            invalid_orders: Some(true),
            pride: Some(false),
            active: Some(false),
        },
        mission: None,
        marker: Ok(Some([1.0, 0.0, 0.0, 1.0])),
        moving: false,
        item: None,
        port_tooltip: None,
        ideology: Ok(Some(2)),
    }
}

pub fn port_flotilla() -> crate::flotilla::Flotilla {
    crate::flotilla::Flotilla {
        port: true,
        mission: Some(2),
        flag_owner: Some(5),
        item: Some(crate::flotilla::ItemIntel {
            byte: 255,
            first: (7 << 32) | 51,
            ships: Some(12),
        }),
        ..at_sea_flotilla(false)
    }
}

pub type Production = (Update, BTreeMap<Cow<'static, str>, usize>);

pub fn production(
    window: &native::Element,
    flotilla: &crate::flotilla::Flotilla,
    settings: CounterSettings,
    fixture: &Fixture,
) -> Production {
    production_selected(window, flotilla, settings, fixture, &BTreeSet::new())
}

pub(super) fn production_selected(
    window: &native::Element,
    flotilla: &crate::flotilla::Flotilla,
    settings: CounterSettings,
    fixture: &Fixture,
    selected: &BTreeSet<Unit>,
) -> Production {
    let countries = countries();
    let art = Art;
    let ctx = NavyContext {
        settings,
        selected,
        defines: &CounterDefines::VANILLA,
        art: &art,
        gui: fixture,
    };
    let mut report = BTreeMap::new();
    let state = NavyState {
        flotilla,
        countries: &countries,
    };
    let calls = navy_calls(&ctx, window, &state, &mut report);
    (calls, report)
}
