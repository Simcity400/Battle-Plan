use super::CounterFrame;
use viewer_counter_kit::fixtures::{action_gui, counter_art_fixture, counter_assets_fixture, counter_map_fixture, layout_gui, layouts, sheet, wings_frame, with_inputs};
use viewer_counter_kit::layouts;
mod branches;
use viewer_engagements::engaged::Engaged;
use viewer_engagements::engagement_history::LAND;
use viewer_engagements::engagement_history::Provenance;
use viewer_engagements::engagement_history::combats;
use viewer_engagements::engagement_progress::undefended;
use viewer_engagements::icon_frames::CARRIED;
use viewer_gpu::atlas_pages;
use viewer_gpu::camera::Camera;
use viewer_gpu::camera::MapDefines;
use viewer_gpu::camera::Placement;
use viewer_native_gui::lazy_icons::FlagReader;
use viewer_native_gui::native_gui::FALLBACK_SPRITE;
use viewer_native_gui::native_gui::evaluate_size;
use viewer_native_gui::native_resources::GuiResources;
use viewer_native_gui::native_windows::NativeWindows;
use viewer_foundation::perspective::Choice;
use viewer_foundation::perspective::Country;
use viewer_foundation::perspective::SharingUnknown;
use viewer_gpu::province_admission::edge_boxes;
use viewer_gpu::province_map::HeightField;
use viewer_gpu::province_schedule::Shared;
#[cfg(test)]
use viewer_native_gui::test_support::Fixture;
use viewer_foundation::visibility::FogContent;
use viewer_foundation::visibility::Relations;
use viewer_foundation::visibility::test_defines;
use dashboard_text::table_access::TokenKeys;
#[test]
fn a_stack_slot_the_file_leaves_out_takes_the_provinces_centroid_default() {
    use viewer_counter_kit::seeded_only;
    use viewer_gpu::stack_anchors::default_anchor;
    use viewer_runtime::label_reads::VICTORY_POINT;
    use viewer_unit_reads::counter_anchors::{STACK_KINDS, stack_anchor};
    let values = [20u8; 16];
    let heights = HeightField {
        width: 4,
        height: 4,
        values: &values,
    };
    let seeded = default_anchor([2.75, 1.5], &heights);
    assert_eq!(seeded, [2.0, heights.native_height(2.0, 1.0), 1.0]);
    let anchors = [
        ((4424, 0), [4414.0, 10.0, 678.0]),
        ((4424, DEFENDING), [4416.5, 10.25, 678.0]),
        ((7, VICTORY_POINT), [7.0, 7.0, 7.0]),
        ((7, DEFENDING), [7.5, 7.5, 7.5]),
    ]
    .into_iter()
    .collect();
    let defaults = [(4424, seeded), (7, seeded), (9, seeded)]
        .into_iter()
        .collect();
    let at = |province, kind| stack_anchor(&anchors, &defaults, province, kind);
    assert_eq!(at(4424, 0), Some([4414.0, 10.0, 678.0]));
    assert_eq!(at(4424, 19), Some(seeded));
    assert_eq!(at(4424, VICTORY_POINT), Some([4416.5, 10.25, 678.0]));
    assert!(!seeded_only(&anchors, 4424, VICTORY_POINT));
    assert_eq!(at(7, VICTORY_POINT), Some([7.0, 7.0, 7.0]));
    assert_eq!(at(7, 9), Some(seeded));
    assert_eq!(at(9, VICTORY_POINT), Some(seeded));
    assert!(seeded_only(&anchors, 9, VICTORY_POINT));
    assert_eq!(at(4424, STACK_KINDS), None);
    assert_eq!(at(4425, 19), None);
    let (labels, fallbacks) = viewer_runtime::label_reads::label_anchors(
        [0, 4424, 7, 9, 4425].into_iter(),
        |province| at(province, VICTORY_POINT),
        |province| {
            defaults.contains_key(&province) && seeded_only(&anchors, province, VICTORY_POINT)
        },
    );
    assert_eq!(
        labels,
        [
            (7, [7.0, 7.0, 7.0]),
            (9, seeded),
            (4424, [4416.5, 10.25, 678.0])
        ]
        .into_iter()
        .collect()
    );
    assert_eq!(fallbacks, [9]);
}

use content_reader_layers::ResolvedLayers;
use content_reader_gui::gui::native;
use content_reader_map::map::ProvinceKind;
use crate::counters::geography;
use crate::counters::layered::Layered;
use crate::counters::preparation::Preparation;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::io;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use super::{
    Carry, CounterAssets, CounterContent, HourTables, MissionMemo, PreparedCounters, hour_tables,
    hour_tables_profiled, peace_conference_hides_units, port_slot_names, prepare_cached,
    prepare_retained,
};
use viewer_counter_kit::{
    ActionContent, AirGridHover, Child, CounterColorMode, CounterDefines, CounterDisplay,
    CounterInputs, CounterReport, Countries, GUN_WINDOW, JoinRules, Kinds, LAND_WINDOW, Layouts,
    Movements, UiColors, assign_union, blend_flag_overlay, counter_plate_color, draw_bar,
    draw_child, draw_flag, land_windows, type_tint, unit_counters_shown, unit_flag_owner, visible,
    zoom_lift,
};
use viewer_counter_kit::combat_markers::{
    LandCombatDefines, LandMarker, LandMarkers, combat_midpoint, combat_tint,
};
use viewer_counter_kit::counter_groups::Level;
use viewer_counter_kit::flotilla::NavalBars;
use viewer_counter_kit::layouts::{
    ability_grid, child, gun_counter, land_counter, text_child, window, window_size,
};
use viewer_counters_land::land;
use viewer_counters_land::land::{
    AbilityIcon, AbilityInputs, CounterTables, GunInputs, GunUnit, LandUnit, PreparedLand,
    TemplateIcon, TemplateIconMemo, TemplateIcons, first_fit, glyph_disclosure, hq_portraits,
    land_join, update_abilities,
};
use viewer_counters_land::members::{LandStacks, Member};
use viewer_counters_naval::{naval, naval_windows};
use viewer_counters_naval::base_windows::View as BaseView;
use viewer_counters_naval::bases::{
    AirBaseCount, BaseContext, BaseIcons, BuildingIconContent, NavalBaseIcon, air_base_frame,
    air_base_hidden, controlled_provinces, merge, naval_base_frame, province_at, radar_states,
    site_frame, site_icon_states, strategic_locations, wings_per_site,
};
use viewer_counters_naval::naval::{NavalContext, NavalUnit, ShipInputs, mean_bars, naval_can_join};
use viewer_counters_naval::naval_markers::{NavalMarkerTables, Viewer};
use viewer_engagements::engagement_history::{CombatFlags, History, LAND_ICON};
use viewer_foundation::{allegiance, building_facts, templates};
use viewer_foundation::allegiance::Allegiance;
use viewer_foundation::mode_visibility::{PriorityIcon, PriorityTable, UnitKinds, Visibility};
use viewer_foundation::perspective::{Sight, Vision};
use viewer_foundation::templates::NO_INTEL_SPRITE;
use viewer_foundation::unit_rules::{DEFENDING, STANDSTILL};
use viewer_gpu::camera::MapSize;
use viewer_gpu::icon_tracks::IconFamily;
use viewer_gpu::province_admission::Admission;
use viewer_native_gui::sprites;
use viewer_native_gui::counter_hovers::land::{CounterControl, CounterHover};
use viewer_native_gui::icon_windows::{PoolView, admitted};
use viewer_native_gui::lazy_icons::{LazyFlags, LazyTextures};
use viewer_native_gui::native_windows::MAPICONS_GUI;
use viewer_native_gui::sprites::{Batch, SpriteSheet, layer};
use viewer_runtime::unit_handle::Unit;
use viewer_unit_reads::{
    counter_action, counter_inputs, counter_move, counter_order, counter_status,
};
use viewer_unit_reads::counter_anchors::stack_anchor;
use viewer_unit_reads::counter_inputs::NativeInputs;

#[test]
fn foreign_division_icons_undeclared_selection_draws_no_type_quad_and_is_counted_by_name() {
    content_reader_script::test_game::enter();
    let guis = layout_gui(
        br#"guiTypes={
        containerWindowType={name=unit_counter_navy}
        containerWindowType={name=naval_combat_mapicon}
        containerWindowType={name=unit_counter size={width=81 height=29}
            buttonType={name=count_bg spriteType=GFX_synthetic_plate}
            iconType={name=type spriteType=GFX_synthetic_actions
                position={x=3 y=4} scale=0.5 frame=9}
        }}"#,
    );
    let mut hidden = land_unit(7, 2, 10, 10, true);
    hidden.sight = Sight::Partial;
    let members = [hidden, land_unit(3, 2, 10, 10, true)];
    let (batch, report) = draw_land_action_fixture(guis, &members, false);
    assert!(!batch.quads.is_empty());
    assert!(batch.quads.iter().all(|quad| quad.texture == 1));
    assert_eq!(report.land_counters, 1);
    assert_eq!(report.no_intel_counters, 1);
    assert_eq!(report.land_counter_gaps.len(), 1);
    let (refusal, count) = report.land_counter_gaps.first_key_value().unwrap();
    assert!(refusal.contains(NO_INTEL_SPRITE));
    assert!(refusal.contains("previous drawable, which the recording does not hold"));
    assert_eq!(*count, 1);
}

#[test]
fn off_grid_supply_threshold_uses_exact_ratio_and_withholds_ambiguous_historical_glyph() {
    content_reader_script::test_game::enter();
    let guis = layout_gui(
        br#"guiTypes={
        containerWindowType={name=unit_counter_navy}
        containerWindowType={name=naval_combat_mapicon}
        containerWindowType={name=unit_counter size={width=81 height=29}
            buttonType={name=count_bg spriteType=GFX_synthetic_plate}
            iconType={name=low_supply spriteType=GFX_synthetic_actions position={x=5 y=6}}
        }}"#,
    );
    let off_grid = counter_status::SupplyThresholds {
        very_low: 20_500,
        low: 35_500,
        show: 50_500,
    };
    let draw_row = |row: replay_data::rows::DivisionStatusRow,
                    thresholds: counter_status::SupplyThresholds,
                    coarse: bool| {
        let mut frame = viewer_foundation::test_support::test_frame(1);
        viewer_foundation::test_support::test_put(
            &mut frame,
            TableRows::DivisionStatus(vec![row]),
        );
        let mut panel = replay_data::rows::DivisionPanelStatsRow {
            division: 1,
            ..Default::default()
        };
        panel.panel_stats[0] = Some(1_000_000);
        panel.panel_stats[1] = Some(1_000_000);
        viewer_foundation::test_support::test_put(
            &mut frame,
            TableRows::DivisionPanelStats(vec![panel]),
        );
        viewer_foundation::test_support::test_put(
            &mut frame,
            TableRows::DivisionMovement(vec![replay_data::rows::DivisionMovementRow {
                division: 1,
                naval_transfer: 0,
                ..Default::default()
            }]),
        );
        let statuses = land::DivisionStatuses::read(&frame).unwrap();
        let status = statuses.get(1);
        let mut unit = land_unit(1, 1, 10, 10, true);
        unit.bars = statuses.recorded(1);
        unit.status_recorded = status.recorded;
        unit.status = counter_status::Member {
            supply_ratio: status.supply_ratio,
            naval_transfer: status.naval_transfer,
            entrenchment: status.entrenchment,
            entrenchment_cap: status.entrenchment_cap,
            attrition: status.attrition,
            exercising: status.exercising,
            grouped: false,
            own: true,
            has_order: status.has_order,
            invalid_orders: status.invalid_orders,
        };
        with_counter_inputs_fixture(
            guis.clone(),
            |_| {},
            |content| {
                content.defines.supply = thresholds;
            },
            |inputs| {
                let countries = Arc::new(Countries::default());
                let templates = Arc::new(TemplateIcons::default());
                let vision = Arc::new(Vision::sharing(1, []));
                let abilities = AbilityInputs::default();
                let tables = CounterTables {
                    type_tints: &Default::default(),
                    countries: &countries,
                    icons: &templates,
                    status: &Default::default(),
                    movement: &Default::default(),
                    actions: &Default::default(),
                    vision: &vision,
                    order: None,
                    native: &Default::default(),
                    orders: &Default::default(),
                    guns: &Default::default(),
                    abilities: &abilities,
                    task_forces: &Default::default(),
                    hq_providers: &Default::default(),
                    portraits: &Default::default(),
                    icon_inputs: &Default::default(),
                };
                let members = [&unit];
                let stack = land::land_stack(
                    &members,
                    &tables,
                    &Default::default(),
                    false,
                    &|_| false,
                    coarse,
                );
                let style = land::stack_style(&inputs, &stack);
                let mut prepared = PreparedLand {
                    stacks: vec![stack],
                    styles: vec![style],
                    sprites: vec![None],
                    plans: vec![None],
                    flotillas: Vec::new(),
                    positions: Vec::new(),
                    units: Vec::new(),
                    layout: counter_move::DrawnLayout::new(inputs.hour),
                    icons: Vec::new(),
                    source_kinds: Vec::new(),
                    groups: Vec::new(),
                    items: Vec::new(),
                    hovered: Vec::new(),
                    levels: Vec::new(),
                    extents: Default::default(),
                    countries,
                    templates,
                    vision,
                    order: Vec::new(),
                    pointer: None,
                    exits: Vec::new(),
                    exits_running: Default::default(),
                    time: 0.0,
                    pointer_running: Default::default(),
                    pointer_refused: Default::default(),
                    pointer_drawn: Default::default(),
                };
                let mut report = CounterReport::default();
                let batch = prepared
                    .land_plate(
                        &inputs,
                        &mut report,
                        &mut Vec::new(),
                        0,
                        [100.0, 200.0],
                        [81.0, 29.0],
                        inputs
                            .content
                            .defines
                            .stack_priority(inputs.camera.height, PriorityTable::Default),
                        &[false, true].map(|gun| super::land::PlateStyle::read(&inputs, gun)),
                    )
                    .unwrap()
                    .unwrap()
                    .batch
                    .clone();
                let supply_uvs: Vec<_> = batch
                    .quads
                    .iter()
                    .filter(|quad| quad.texture == 2)
                    .map(|quad| quad.uv)
                    .collect();
                (supply_uvs, report.land_counter_gaps)
            },
        )
    };
    let supply_sheet = sheet("GFX_synthetic_actions", 12, [144, 13]);
    let exact_above = replay_data::rows::DivisionStatusRow {
        division: 1,
        supply_ratio: None,
        supply_ratio_exact: replay_data::rows::Recorded::Value(Some(35_500)),
        ..Default::default()
    };
    let (uvs, gaps) = draw_row(exact_above, off_grid, false);
    assert_eq!(uvs, [supply_sheet.frame_uv(0)], "exact 35.5% is not below 35.5% low threshold");
    assert!(gaps.is_empty(), "{gaps:?}");
    let exact_below = replay_data::rows::DivisionStatusRow {
        division: 1,
        supply_ratio: None,
        supply_ratio_exact: replay_data::rows::Recorded::Value(Some(35_499)),
        ..Default::default()
    };
    let (uvs, gaps) = draw_row(exact_below, off_grid, false);
    assert_eq!(uvs, [supply_sheet.frame_uv(1)], "exact 35.499% is below 35.5% low threshold");
    assert!(gaps.is_empty(), "{gaps:?}");
    let historical_ambiguous = replay_data::rows::DivisionStatusRow {
        division: 1,
        supply_ratio: Some(35_000),
        supply_ratio_exact: replay_data::rows::Recorded::Value(None),
        ..Default::default()
    };
    for coarse in [false, true] {
        let (uvs, gaps) = draw_row(historical_ambiguous.clone(), off_grid, coarse);
        assert!(uvs.is_empty(), "ambiguous historical supply must withhold glyph (coarse={coarse})");
        assert!(
            gaps.keys().any(|why| why.contains("low_supply")),
            "coarse={coarse}: {gaps:?}"
        );
    }
    let (uvs, gaps) = draw_row(historical_ambiguous, CounterDefines::VANILLA.supply, false);
    assert_eq!(uvs, [supply_sheet.frame_uv(0)], "vanilla thresholds stay decided for historical rows");
    assert!(gaps.is_empty(), "{gaps:?}");
}

#[test]
fn supply_glyph_and_gaps_work_without_bar_maxima_and_withhold_missing_movement() {
    content_reader_script::test_game::enter();
    let guis = layout_gui(
        br#"guiTypes={
        containerWindowType={name=unit_counter_navy}
        containerWindowType={name=naval_combat_mapicon}
        containerWindowType={name=unit_counter size={width=81 height=29}
            buttonType={name=count_bg spriteType=GFX_synthetic_plate}
            iconType={name=low_supply spriteType=GFX_synthetic_actions position={x=5 y=6}}
        }}"#,
    );
    let off_grid = counter_status::SupplyThresholds {
        very_low: 20_500,
        low: 35_500,
        show: 50_500,
    };
    let draw_case = |row: replay_data::rows::DivisionStatusRow,
                     with_maxima: bool,
                     with_movement: bool,
                     coarse: bool| {
        let mut frame = viewer_foundation::test_support::test_frame(1);
        viewer_foundation::test_support::test_put(
            &mut frame,
            TableRows::DivisionStatus(vec![row]),
        );
        if with_maxima {
            let mut panel = replay_data::rows::DivisionPanelStatsRow {
                division: 1,
                ..Default::default()
            };
            panel.panel_stats[0] = Some(1_000_000);
            panel.panel_stats[1] = Some(1_000_000);
            viewer_foundation::test_support::test_put(
                &mut frame,
                TableRows::DivisionPanelStats(vec![panel]),
            );
        }
        if with_movement {
            viewer_foundation::test_support::test_put(
                &mut frame,
                TableRows::DivisionMovement(vec![replay_data::rows::DivisionMovementRow {
                    division: 1,
                    naval_transfer: 0,
                    ..Default::default()
                }]),
            );
        }
        let statuses = land::DivisionStatuses::read(&frame).unwrap();
        let status = statuses.get(1);
        let mut unit = land_unit(1, 1, 10, 10, true);
        unit.bars = statuses.recorded(1);
        unit.status_recorded = status.recorded;
        unit.status = counter_status::Member {
            supply_ratio: status.supply_ratio,
            naval_transfer: status.naval_transfer,
            entrenchment: status.entrenchment,
            entrenchment_cap: status.entrenchment_cap,
            attrition: status.attrition,
            exercising: status.exercising,
            grouped: false,
            own: true,
            has_order: status.has_order,
            invalid_orders: status.invalid_orders,
        };
        with_counter_inputs_fixture(
            guis.clone(),
            |_| {},
            |content| {
                content.defines.supply = off_grid;
            },
            |inputs| {
                let countries = Arc::new(Countries::default());
                let templates = Arc::new(TemplateIcons::default());
                let vision = Arc::new(Vision::sharing(1, []));
                let abilities = AbilityInputs::default();
                let tables = CounterTables {
                    type_tints: &Default::default(),
                    countries: &countries,
                    icons: &templates,
                    status: &Default::default(),
                    movement: &Default::default(),
                    actions: &Default::default(),
                    vision: &vision,
                    order: None,
                    native: &Default::default(),
                    orders: &Default::default(),
                    guns: &Default::default(),
                    abilities: &abilities,
                    task_forces: &Default::default(),
                    hq_providers: &Default::default(),
                    portraits: &Default::default(),
                    icon_inputs: &Default::default(),
                };
                let members = [&unit];
                let stack = land::land_stack(
                    &members,
                    &tables,
                    &Default::default(),
                    false,
                    &|_| false,
                    coarse,
                );
                let style = land::stack_style(&inputs, &stack);
                let mut prepared = PreparedLand {
                    stacks: vec![stack],
                    styles: vec![style],
                    sprites: vec![None],
                    plans: vec![None],
                    flotillas: Vec::new(),
                    positions: Vec::new(),
                    units: Vec::new(),
                    layout: counter_move::DrawnLayout::new(inputs.hour),
                    icons: Vec::new(),
                    source_kinds: Vec::new(),
                    groups: Vec::new(),
                    items: Vec::new(),
                    hovered: Vec::new(),
                    levels: Vec::new(),
                    extents: Default::default(),
                    countries,
                    templates,
                    vision,
                    order: Vec::new(),
                    pointer: None,
                    exits: Vec::new(),
                    exits_running: Default::default(),
                    time: 0.0,
                    pointer_running: Default::default(),
                    pointer_refused: Default::default(),
                    pointer_drawn: Default::default(),
                };
                let mut report = CounterReport::default();
                let batch = prepared
                    .land_plate(
                        &inputs,
                        &mut report,
                        &mut Vec::new(),
                        0,
                        [100.0, 200.0],
                        [81.0, 29.0],
                        inputs
                            .content
                            .defines
                            .stack_priority(inputs.camera.height, PriorityTable::Default),
                        &[false, true].map(|gun| super::land::PlateStyle::read(&inputs, gun)),
                    )
                    .unwrap()
                    .unwrap()
                    .batch
                    .clone();
                let supply_uvs: Vec<_> = batch
                    .quads
                    .iter()
                    .filter(|quad| quad.texture == 2)
                    .map(|quad| quad.uv)
                    .collect();
                (supply_uvs, report.land_counter_gaps)
            },
        )
    };
    let supply_sheet = sheet("GFX_synthetic_actions", 12, [144, 13]);
    let exact_below = replay_data::rows::DivisionStatusRow {
        division: 1,
        supply_ratio: None,
        supply_ratio_exact: replay_data::rows::Recorded::Value(Some(35_499)),
        ..Default::default()
    };
    let historical_ambiguous = replay_data::rows::DivisionStatusRow {
        division: 1,
        supply_ratio: Some(35_000),
        supply_ratio_exact: replay_data::rows::Recorded::Value(None),
        ..Default::default()
    };
    let exact_full = replay_data::rows::DivisionStatusRow {
        division: 1,
        supply_ratio: None,
        supply_ratio_exact: replay_data::rows::Recorded::Value(Some(80_000)),
        ..Default::default()
    };
    for coarse in [false, true] {
        let (uvs, gaps) = draw_case(exact_below.clone(), false, true, coarse);
        assert_eq!(
            uvs,
            [supply_sheet.frame_uv(1)],
            "missing maxima must not suppress exact supply crate (coarse={coarse})"
        );
        assert!(gaps.is_empty(), "coarse={coarse}: {gaps:?}");
        let (uvs, gaps) = draw_case(historical_ambiguous.clone(), false, true, coarse);
        assert!(uvs.is_empty(), "coarse={coarse}");
        assert!(
            gaps.keys().any(|why| why.contains("low_supply")),
            "missing maxima must not drop ambiguous supply gap (coarse={coarse}): {gaps:?}"
        );
        let (uvs, gaps) = draw_case(exact_below.clone(), true, false, coarse);
        assert!(
            uvs.is_empty(),
            "missing DivisionMovement must withhold low supply crate (coarse={coarse})"
        );
        assert!(
            gaps.keys().any(|why| why.contains("DivisionMovement")),
            "missing DivisionMovement must report named gap (coarse={coarse}): {gaps:?}"
        );
        let (uvs, gaps) = draw_case(exact_full.clone(), true, false, coarse);
        assert!(uvs.is_empty(), "full supply agrees with transferring fallback (coarse={coarse})");
        assert!(gaps.is_empty(), "full supply without movement needs no gap (coarse={coarse}): {gaps:?}");
    }

    let draw_stack = |statuses_rows: Vec<replay_data::rows::DivisionStatusRow>,
                      movements_rows: Vec<replay_data::rows::DivisionMovementRow>,
                      order: &[u64]| {
        let mut frame = viewer_foundation::test_support::test_frame(1);
        viewer_foundation::test_support::test_put(
            &mut frame,
            TableRows::DivisionStatus(statuses_rows),
        );
        viewer_foundation::test_support::test_put(
            &mut frame,
            TableRows::DivisionMovement(movements_rows),
        );
        let statuses = land::DivisionStatuses::read(&frame).unwrap();
        let units: Vec<_> = order
            .iter()
            .map(|&id| {
                let status = statuses.get(id);
                let mut unit = land_unit(id, 1, 10, 10, true);
                unit.bars = statuses.recorded(id);
                unit.status_recorded = status.recorded;
                unit.status = counter_status::Member {
                    supply_ratio: status.supply_ratio,
                    naval_transfer: status.naval_transfer,
                    entrenchment: status.entrenchment,
                    entrenchment_cap: status.entrenchment_cap,
                    attrition: status.attrition,
                    exercising: status.exercising,
                    grouped: false,
                    own: true,
                    has_order: status.has_order,
                    invalid_orders: status.invalid_orders,
                };
                unit
            })
            .collect();
        with_counter_inputs_fixture(
            guis.clone(),
            |_| {},
            |content| {
                content.defines.supply = off_grid;
            },
            |inputs| {
                let countries = Arc::new(Countries::default());
                let templates = Arc::new(TemplateIcons::default());
                let vision = Arc::new(Vision::sharing(1, []));
                let abilities = AbilityInputs::default();
                let tables = CounterTables {
                    type_tints: &Default::default(),
                    countries: &countries,
                    icons: &templates,
                    status: &Default::default(),
                    movement: &Default::default(),
                    actions: &Default::default(),
                    vision: &vision,
                    order: None,
                    native: &Default::default(),
                    orders: &Default::default(),
                    guns: &Default::default(),
                    abilities: &abilities,
                    task_forces: &Default::default(),
                    hq_providers: &Default::default(),
                    portraits: &Default::default(),
                    icon_inputs: &Default::default(),
                };
                let refs: Vec<&LandUnit<'_>> = units.iter().collect();
                let stack = land::land_stack(
                    &refs,
                    &tables,
                    &Default::default(),
                    false,
                    &|_| false,
                    false,
                );
                let style = land::stack_style(&inputs, &stack);
                let mut prepared = PreparedLand {
                    stacks: vec![stack],
                    styles: vec![style],
                    sprites: vec![None],
                    plans: vec![None],
                    flotillas: Vec::new(),
                    positions: Vec::new(),
                    units: Vec::new(),
                    layout: counter_move::DrawnLayout::new(inputs.hour),
                    icons: Vec::new(),
                    source_kinds: Vec::new(),
                    groups: Vec::new(),
                    items: Vec::new(),
                    hovered: Vec::new(),
                    levels: Vec::new(),
                    extents: Default::default(),
                    countries,
                    templates,
                    vision,
                    order: Vec::new(),
                    pointer: None,
                    exits: Vec::new(),
                    exits_running: Default::default(),
                    time: 0.0,
                    pointer_running: Default::default(),
                    pointer_refused: Default::default(),
                    pointer_drawn: Default::default(),
                };
                let mut report = CounterReport::default();
                let batch = prepared
                    .land_plate(
                        &inputs,
                        &mut report,
                        &mut Vec::new(),
                        0,
                        [100.0, 200.0],
                        [81.0, 29.0],
                        inputs
                            .content
                            .defines
                            .stack_priority(inputs.camera.height, PriorityTable::Default),
                        &[false, true].map(|gun| super::land::PlateStyle::read(&inputs, gun)),
                    )
                    .unwrap()
                    .unwrap()
                    .batch
                    .clone();
                let supply_uvs: Vec<_> = batch
                    .quads
                    .iter()
                    .filter(|quad| quad.texture == 2)
                    .map(|quad| quad.uv)
                    .collect();
                (supply_uvs, report.land_counter_gaps)
            },
        )
    };

    let low_div2 = replay_data::rows::DivisionStatusRow {
        division: 2,
        supply_ratio: None,
        supply_ratio_exact: replay_data::rows::Recorded::Value(Some(35_499)),
        ..Default::default()
    };
    let on_land_1 = replay_data::rows::DivisionMovementRow {
        division: 1,
        naval_transfer: 0,
        ..Default::default()
    };
    let on_land_2 = replay_data::rows::DivisionMovementRow {
        division: 2,
        naval_transfer: 0,
        ..Default::default()
    };
    let at_sea_1 = replay_data::rows::DivisionMovementRow {
        division: 1,
        naval_transfer: 1,
        ..Default::default()
    };

    let (uvs, gaps) = draw_stack(vec![low_div2.clone()], vec![on_land_1.clone(), on_land_2.clone()], &[1, 2]);
    assert_eq!(uvs, [supply_sheet.frame_uv(1)], "missing status first must not suppress last member's supply crate");
    assert!(gaps.is_empty(), "{gaps:?}");

    let (uvs, gaps) = draw_stack(vec![low_div2.clone()], vec![on_land_1, on_land_2.clone()], &[2, 1]);
    assert!(uvs.is_empty(), "missing status last on land must withhold supply crate");
    assert!(
        gaps.keys().any(|why| why.contains("DivisionStatus")),
        "missing status last on land must report named supply gap: {gaps:?}"
    );

    let (uvs, gaps) = draw_stack(vec![low_div2], vec![at_sea_1, on_land_2], &[2, 1]);
    assert_eq!(uvs, [supply_sheet.frame_uv(1)], "missing status last at sea is skipped in favour of land member");
    assert!(gaps.is_empty(), "{gaps:?}");
}

#[test]
fn dump_census_excludes_combat_window_quads() {
    content_reader_script::test_game::enter();
    let shared = Arc::new(ResolvedLayers::from_folders(&[]).unwrap());
    let registry = native::resources::Registry::load(&shared).unwrap();
    let (assets, fonts) = counter_assets_fixture(
        shared,
        registry,
        layout_gui(
            br#"guiTypes = {
            containerWindowType = { name = unit_counter_navy }
            containerWindowType = { name = naval_combat_mapicon }
        }"#,
        ),
        layouts(),
        [(
            "GFX_onmap_unit_counter".into(),
            sheet("GFX_onmap_unit_counter", 1, [10, 10]),
        )]
        .into(),
    );
    let mut map = Batch::default();
    map.scaled(
        assets.sheet("GFX_onmap_unit_counter").unwrap(),
        0,
        [10., 10.],
        [10., 10.],
        [1.; 4],
    );
    let expected = assets.census(&map, &fonts, 1.);
    assert_eq!(expected["plates"].as_array().unwrap().len(), 1);
    let mut extra = Batch::default();
    extra.scaled(
        &sheet("synthetic_combat_window", 1, [5, 5]),
        0,
        [20., 20.],
        [5., 5.],
        [1.; 4],
    );
    let mut sprites = Batch::default();
    assert_eq!(
        viewer_native_gui::sprites::dump_sprites(&map, &extra, &mut sprites)
            .quads
            .len(),
        2
    );
    assert_eq!(assets.census(&map, &fonts, 1.), expected);
}
use replay_data::{
    Hour, rows::AirBaseRegistrationRow, rows::AirWingRow, rows::CountryLinkRow,
    rows::LocalPlayerRow, rows::ProvinceRow, rows::StateFlagRow, rows::TableId, rows::TableRows,
};
use replay_frame::{frame::Metrics, frame::TableView};

#[test]
fn window_size_applies_native_bounds_and_names_unsupported_constraints() {
    content_reader_script::test_game::enter();
    let guis = layout_gui(b"guiTypes={containerWindowType={name=test_icon size={width=20 height=30 min={width=40} max={height=15}}}}");
    assert_eq!(
        window_size(window(&guis, "test_icon").unwrap(), "test_icon").unwrap(),
        [40.0, 15.0]
    );
    let guis = layout_gui(b"guiTypes={containerWindowType={name=test_icon size={width=20 height=30 min={width=100%}}}}");
    let error = window_size(window(&guis, "test_icon").unwrap(), "test_icon")
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("test_icon") && error.contains("percentage"),
        "{error}"
    );
}

#[test]
fn port_slots_are_is_port_types_and_their_redirects() {
    let definitions = content_reader_key_names::buildings::parse(
        b"buildings={Port={is_port=yes spawn_point=Harbour} depot={}} spawn_points={Harbour={}}",
        &content_reader_script::key_names::KeyNames::default(),
    )
    .unwrap();
    assert_eq!(
        port_slot_names(&definitions),
        BTreeSet::from(["Harbour", "Port"])
    );
}

#[test]
fn a_template_with_unreadable_model_inputs_keeps_its_counter_icon() {
    let icon = |model_unreadable| {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 1,
            tables: std::array::from_fn(|_| TableView::Unavailable {
                reason: "not needed".into(),
            })
            .map(Into::into),
            metrics: Metrics::default(),
        };
        *frame.tables[TableId::DivisionTemplateIcons.index()] =
            TableView::Complete(TableRows::DivisionTemplateIcons(vec![
                replay_data::rows::DivisionTemplateIconRow {
                    template: 50,
                    priority_unit: 7,
                    priority_unit_name: b"infantry".to_vec(),
                    custom_icon: -1,
                    model_unreadable,
                    ..Default::default()
                },
            ]));
        let icons = TemplateIcons::read(&frame).unwrap();
        (icons.by_template[&50].clone(), icons.grouping[&50].clone())
    };
    assert_eq!(icon(1), icon(0));
}

#[test]
fn only_the_viewing_countrys_peace_conference_hides_unit_sources() {
    let country = |canonical_slot, peace_conference_participant| replay_data::rows::CountryRow {
        canonical_slot,
        peace_conference_participant,
        ..Default::default()
    };
    let countries = [country(1, 0), country(2, 1)];
    assert!(peace_conference_hides_units(&countries, 2));
    assert!(!peace_conference_hides_units(&countries, 1));
    assert!(!peace_conference_hides_units(&countries, 3));
    assert!(peace_conference_hides_units(&[country(1, 255)], 1));
}

#[test]
fn a_kept_report_assigned_from_another_equals_its_clone() {
    let mut source = CounterReport {
        land_counters: 3,
        unavailable_railway_gun_locations: 2,
        moving: 1,
        base_hits: vec![viewer_native_gui::counter_hovers::unit_hits::Blocker {
            rect: [3.0, 4.0, 5.0, 6.0],
            layer: layer(-3, 1, 0),
        }],
        land_hovers: vec![CounterHover {
            control: viewer_native_gui::counter_hovers::land::CounterControl::Normal,
            authored: None,
            rect: [1.0, 2.0, 3.0, 4.0],
            remaining: Some(5),
            ours: true,
            coarse: false,
            hidden: false,
            railway_gun: false,
            members: vec![1].into(),
            timing: vec![].into(),
        }],
        grouping: Some(Level::Province),
        ..Default::default()
    };
    source.missing_flags.insert("AAA".into());
    source
        .counter_flag_gaps
        .insert("known capture refusal".into(), 2);
    source.faction_ping_limits.undecided = Some("undecided".into());
    source.air_grid_hits.insert(
        0,
        AirGridHover {
            family: IconFamily::Bases,
            members: Some(Arc::new(vec![7, 8])),
            text: Arc::default(),
        },
    );
    let mut kept = CounterReport {
        naval_counters: 7,
        land_hovers: vec![source.land_hovers[0].clone(); 4],
        ..Default::default()
    };
    kept.missing_flags.insert("BBB".into());
    kept.counter_flag_gaps
        .insert("older capture refusal".into(), 1);
    kept.faction_ping_limits.undecided = Some("older".into());
    kept.assign(&source);
    assert_eq!(kept, source);
    kept.assign(&CounterReport::default());
    assert!(kept.base_hits.is_empty());
}

#[test]
fn native_grouping_refusals_survive_report_copy_and_clear_next_frame() {
    let why = "country slot 5: CountryCounterInputs.co_belligerents not recorded";
    let mut source = CounterReport::default();
    source.grouping_gaps.insert(why.into(), 3);
    let mut kept = CounterReport::default();
    kept.assign(&source);
    assert_eq!(kept.grouping_gaps.get(why), Some(&3));
    assert!(kept.warnings().any(|line| {
        line.contains("Counter grouping: 3 comparisons unknown")
            && line.contains("groups kept separate")
            && line.contains(why)
    }));
    kept.assign(&CounterReport::default());
    assert!(kept.grouping_gaps.is_empty());
    assert!(
        !kept
            .warnings()
            .any(|line| line.contains("Counter grouping:"))
    );
}

#[test]
fn the_flag_union_is_rebuilt_only_when_it_changes() {
    let text = |tags: &[&str]| tags.iter().map(|tag| tag.to_string()).collect::<Vec<_>>();
    let base: BTreeSet<String> = text(&["AAA"]).into_iter().collect();
    let extra = text(&["CCC", "BBB", "CCC"]);
    let mut target = BTreeSet::new();
    assign_union(&mut target, &base, extra.iter());
    assert_eq!(target, text(&["AAA", "BBB", "CCC"]).into_iter().collect());
    let kept = target.first().expect("filled above").as_ptr();
    assign_union(&mut target, &base, extra.iter());
    assert_eq!(target.first().expect("still filled").as_ptr(), kept);
    assign_union(&mut target, &base, extra[..1].iter());
    assert_eq!(target, text(&["AAA", "CCC"]).into_iter().collect());
    assign_union(&mut target, &BTreeSet::new(), [].iter());
    assert!(target.is_empty());
}

#[test]
fn the_air_base_icon_follows_the_state_s_base_not_a_type_name() {
    use replay_data::{rows::AirBaseRow, rows::BuildingRow, rows::LexicalTokenRow};
    let (mut frame, _) = wings_frame(TableView::Complete(TableRows::StateFlags(Vec::new())));
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
        [
            (201, &b"mod_airfield"[..]),
            (202, b"rocket_site"),
            (203, b"air_base"),
        ]
        .map(|(token, literal)| LexicalTokenRow {
            token,
            literal: literal.to_vec(),
        })
        .into(),
    ));
    *frame.tables[TableId::StateBuildings.index()] =
        TableView::Complete(TableRows::StateBuildings(vec![
            BuildingRow {
                location: 7,
                building_type: 201,
                level: 2,
                ..Default::default()
            },
            BuildingRow {
                location: 9,
                building_type: 202,
                level: 1,
                ..Default::default()
            },
            BuildingRow {
                location: 8,
                building_type: 203,
                level: 1,
                ..Default::default()
            },
        ]));
    let bases = |rows: Vec<AirBaseRow>, missing: Option<u32>| match missing {
        None => TableView::Complete(TableRows::AirBases(rows)),
        Some(missing) => TableView::Truncated {
            rows: TableRows::AirBases(rows),
            missing,
        },
    };
    let ordinary = AirBaseRow {
        id: 0xB0,
        state: Some(7),
        kind: 0,
        ..Default::default()
    };
    let site = AirBaseRow {
        id: 0xB1,
        state: Some(9),
        kind: 1,
        ..Default::default()
    };
    let carrier = AirBaseRow {
        id: 0xB2,
        state: None,
        kind: 0,
        ..Default::default()
    };
    *frame.tables[TableId::AirBases.index()] =
        bases(vec![ordinary.clone(), site.clone(), carrier], None);
    let expected = [BTreeSet::from([7]), BTreeSet::from([9]), BTreeSet::new()];
    assert_eq!(site_icon_states(&frame).unwrap(), (expected.clone(), true));
    *frame.tables[TableId::AirBases.index()] = bases(vec![ordinary, site], Some(1));
    assert_eq!(site_icon_states(&frame).unwrap(), (expected, false));
}

#[test]
fn radar_icons_follow_the_viewer_s_search_and_the_state_s_controller() {
    use replay_data::rows::{
        CountryAppearanceRow, FactionMemberRow, RadarCoverageRow, Recorded, StateRow,
    };
    let (mut frame, _) = wings_frame(TableView::Complete(TableRows::StateFlags(Vec::new())));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (0..4)
            .map(|slot| CountryLinkRow {
                id: 100 + slot,
                canonical_slot: slot,
            })
            .collect::<Vec<_>>(),
    ));
    let group = |country, ordinal, state| RadarCoverageRow {
        country,
        ordinal,
        state: Some(state),
        provinces: Vec::new(),
        radius: None,
    };
    *frame.tables[TableId::RadarCoverage.index()] =
        TableView::Complete(TableRows::RadarCoverage(vec![
            group(100, 0, 9),
            group(101, 0, 7),
            group(101, 1, 10),
            group(102, 0, 8),
            group(103, 0, 11),
        ]));
    let state = |id, controller| StateRow {
        id,
        controller,
        ..Default::default()
    };
    *frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![
        state(7, 101),
        state(8, 102),
        state(9, 100),
        state(10, 102),
        state(11, 103),
    ]));
    let member = |country, ordinal| FactionMemberRow {
        country,
        faction: 1,
        ordinal,
    };
    let members = |rows: Vec<FactionMemberRow>, missing: Option<u32>| match missing {
        None => TableView::Complete(TableRows::FactionMembers(rows)),
        Some(missing) => TableView::Truncated {
            rows: TableRows::FactionMembers(rows),
            missing,
        },
    };
    let appearance = |overlord: i32, subjects: Vec<i32>| {
        TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
            country: 100,
            subjects: Recorded::Value(subjects),
            overlord,
            ..Default::default()
        }]))
    };
    *frame.tables[TableId::FactionMembers.index()] =
        members(vec![member(100, 0), member(101, 1)], None);
    *frame.tables[TableId::CountryAppearance.index()] = appearance(0, Vec::new());
    assert_eq!(
        radar_states(&frame, 0).unwrap().shown,
        BTreeSet::from([7, 9])
    );
    *frame.tables[TableId::CountryAppearance.index()] = appearance(102, vec![103]);
    assert_eq!(
        radar_states(&frame, 0).unwrap().shown,
        BTreeSet::from([7, 8, 9, 11])
    );
    *frame.tables[TableId::CountryAppearance.index()] = appearance(0, Vec::new());
    *frame.tables[TableId::FactionMembers.index()] = members(vec![member(100, 0)], Some(1));
    let radar = radar_states(&frame, 0).unwrap();
    assert_eq!(radar.shown, BTreeSet::from([9]));
    assert_eq!(
        radar.notes,
        [
            "3 radar host states are not drawn: the viewer's relations are not all recorded (FactionMembers)"
        ]
    );
}

#[test]
fn a_base_holding_only_empty_wings_is_not_occupied() {
    let (mut frame, vision) = wings_frame(TableView::Complete(TableRows::StateFlags(Vec::new())));
    let wings = |planes: i32| {
        TableRows::AirWings(vec![AirWingRow {
            id: 0xC1,
            air_base: 0xB0,
            planes,
            ..Default::default()
        }])
    };
    let occupied = |frame: &DecodedFrame| {
        wings_per_site(frame, &vision).unwrap()[&(7, building_facts::AIR_BASE)].occupied
    };
    assert_eq!(occupied(&frame), Some(true));
    *frame.tables[TableId::AirWings.index()] = TableView::Complete(wings(0));
    assert_eq!(occupied(&frame), Some(false));
    let count = wings_per_site(&frame, &vision).unwrap()[&(7, building_facts::AIR_BASE)];
    assert_eq!(site_frame(&count), 1);
    *frame.tables[TableId::AirWings.index()] = TableView::Truncated {
        rows: TableRows::AirWings(Vec::new()),
        missing: 1,
    };
    assert_eq!(occupied(&frame), None);
}

#[test]
fn base_icons_need_a_controlled_province() {
    let (mut frame, _) = wings_frame(TableView::Complete(TableRows::StateFlags(Vec::new())));
    let province = |id, controller| ProvinceRow {
        id,
        controller,
        ..Default::default()
    };
    *frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(vec![
        province(9, 100),
        province(10, 0),
    ]));
    let controlled = controlled_provinces(&frame).unwrap();
    assert_eq!(
        (controlled.get(&9), controlled.get(&10), controlled.get(&11)),
        (Some(&true), Some(&false), None)
    );
}

#[test]
fn strategic_locations_list_the_province_s_tokens_then_its_state_s() {
    use replay_data::{rows::Recorded, rows::StateCoreRow};
    let (mut frame, _) = wings_frame(TableView::Complete(TableRows::StateFlags(Vec::new())));
    let pair = |high: u64, low: u64| high << 32 | low;
    let province = |id, state| ProvinceRow {
        id,
        state,
        ..Default::default()
    };
    *frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(vec![
        province(31, 4),
        province(32, 4),
        province(33, 4),
        province(40, 5),
    ]));
    *frame.tables[TableId::StateCores.index()] = TableView::Complete(TableRows::StateCores(vec![
        StateCoreRow {
            state: 4,
            strategic_locations: Recorded::Value(vec![
                pair(701, 31),
                pair(702, 33),
                pair(703, 40),
                pair(704, 99),
            ]),
            province_strategic_locations: Recorded::Value(vec![
                pair(31, 801),
                pair(31, 802),
                pair(32, 803),
            ]),
            ..Default::default()
        },
        StateCoreRow {
            state: 5,
            ..Default::default()
        },
        StateCoreRow {
            state: 6,
            strategic_locations: Recorded::NotRecorded,
            province_strategic_locations: Recorded::NotRecorded,
            ..Default::default()
        },
    ]));
    let (locations, notes) = strategic_locations(&frame).unwrap();
    assert_eq!(
        locations,
        BTreeMap::from([(31, vec![801, 802, 701]), (32, vec![803]), (33, vec![702]),])
    );
    assert_eq!(
        notes,
        [
            "1 states' strategic locations are not recorded, so their icons are not drawn",
            "1 state strategic locations anchor on a province whose state is not recorded (Provinces), so they are not drawn"
        ]
    );
}

#[test]
fn each_base_icon_reads_its_own_state_base() {
    use replay_data::rows::AirBaseRow;
    let (mut frame, vision) = wings_frame(TableView::Complete(TableRows::StateFlags(Vec::new())));
    *frame.tables[TableId::AirBases.index()] = TableView::Complete(TableRows::AirBases(vec![
        AirBaseRow {
            id: 0xB0,
            state: Some(7),
            capacity: 10,
            ..Default::default()
        },
        AirBaseRow {
            id: 0xB1,
            state: Some(7),
            capacity: 4,
            kind: 1,
            ..Default::default()
        },
    ]));
    let sites = wings_per_site(&frame, &vision).unwrap();
    let air = sites[&(7, building_facts::AIR_BASE)];
    let rocket = sites[&(7, building_facts::ROCKET_SITE)];
    assert_eq!((air.capacity, air.planes), (10, 5));
    assert_eq!((rocket.capacity, rocket.planes), (4, 0));
    assert!(air.shown && !rocket.shown);
    assert_eq!(rocket.occupied, Some(false));
    assert_eq!(site_frame(&rocket), 3);
    assert_eq!(
        site_frame(&AirBaseCount {
            shown: true,
            occupied: Some(true),
            ..rocket
        }),
        2
    );
    assert_eq!(
        site_frame(&AirBaseCount {
            shown: true,
            ..rocket
        }),
        1
    );
    assert_eq!(
        site_frame(&AirBaseCount {
            shown: true,
            occupied: None,
            planes: 1,
            ..rocket
        }),
        2
    );
}

#[test]
fn truncated_state_flags_hide_every_box_and_fall_back_the_badge() {
    let (frame, vision) = wings_frame(TableView::Truncated {
        rows: TableRows::StateFlags(vec![StateFlagRow {
            state: 8,
            demilitarized: 0,
            ..Default::default()
        }]),
        missing: 1,
    });
    let states = wings_per_site(&frame, &vision).unwrap();
    let count = states
        .get(&(7, building_facts::AIR_BASE))
        .copied()
        .unwrap_or_default();
    assert!(!count.shown, "a missing state could be demilitarized");
    assert_eq!(count.stationed, Some(true));
    assert_eq!(count.occupied, Some(true));
    assert_eq!(count.planes, 5);
    assert_eq!(
        air_base_frame(&count, None),
        1,
        "wholly undecided detail falls back to the recorded planes"
    );
    assert_eq!(
        air_base_frame(&count, Some(false)),
        0,
        "decided-no-detail stays plain despite the planes"
    );
}

#[test]
fn unavailable_state_flags_do_not_invent_a_non_demilitarized_base() {
    let (frame, vision) = wings_frame(TableView::Unavailable {
        reason: "state flags capture failed".into(),
    });
    let states = wings_per_site(&frame, &vision).unwrap();
    let count = states
        .get(&(7, building_facts::AIR_BASE))
        .copied()
        .unwrap_or_default();
    assert!(!count.shown);
    assert_eq!(count.stationed, Some(true));
}

#[test]
fn recorded_states_read_their_own_flag_row() {
    let (mut frame, vision) = wings_frame(TableView::Truncated {
        rows: TableRows::StateFlags(vec![StateFlagRow {
            state: 7,
            demilitarized: 0,
            ..Default::default()
        }]),
        missing: 1,
    });
    let TableView::Complete(TableRows::AirBaseRegistrations(registrations)) =
        &mut *frame.tables[TableId::AirBaseRegistrations.index()]
    else {
        panic!("fixture holds complete registrations");
    };
    registrations[0].wings.clear();
    let states = wings_per_site(&frame, &vision).unwrap();
    let count = states
        .get(&(7, building_facts::AIR_BASE))
        .copied()
        .unwrap_or_default();
    assert!(count.shown, "state 7 is recorded non-demilitarised");
    assert_eq!(count.stationed, Some(false));
    assert_eq!(
        air_base_frame(&count, Some(true)),
        0,
        "stationed-false beats decided-true detail"
    );
}

#[test]
fn complete_state_flags_show_an_accessed_base() {
    let (frame, vision) = wings_frame(TableView::Complete(TableRows::StateFlags(vec![
        StateFlagRow {
            state: 7,
            demilitarized: 0,
            ..Default::default()
        },
    ])));
    let states = wings_per_site(&frame, &vision).unwrap();
    let count = states
        .get(&(7, building_facts::AIR_BASE))
        .copied()
        .unwrap_or_default();
    assert!(count.shown);
    assert_eq!(count.stationed, Some(true));
    assert_eq!(count.occupied, Some(true));
    assert_eq!(air_base_frame(&count, Some(true)), 1);
}

#[test]
fn another_countrys_wings_take_the_flag_frame() {
    let (mut frame, vision) = wings_frame(TableView::Complete(TableRows::StateFlags(vec![
        StateFlagRow {
            state: 7,
            demilitarized: 0,
            ..Default::default()
        },
    ])));
    let TableView::Complete(TableRows::AirBaseRegistrations(registrations)) =
        &mut *frame.tables[TableId::AirBaseRegistrations.index()]
    else {
        panic!("fixture holds complete registrations");
    };
    registrations[0].wings.clear();
    registrations.push(AirBaseRegistrationRow {
        base: 0xB0,
        slot: 2,
        capacity_raw: 100_000,
        wings: vec![0xC2],
        carrier: Default::default(),
        transport_planes: Default::default(),
        fuel_demand_raw: Default::default(),
        fuel_allocated_raw: Default::default(),
    });
    let TableView::Complete(TableRows::AirWings(wings)) =
        &mut *frame.tables[TableId::AirWings.index()]
    else {
        panic!("fixture holds complete wings");
    };
    wings[0].id = 0xC2;
    let states = wings_per_site(&frame, &vision).unwrap();
    let count = states
        .get(&(7, building_facts::AIR_BASE))
        .copied()
        .unwrap_or_default();
    assert!(count.shown, "our access row is still recorded");
    assert_eq!(count.stationed, Some(false));
    assert_eq!(count.foreign, Some(true));
    assert_eq!(count.planes, 5);
    assert_eq!(
        air_base_frame(&count, Some(true)),
        3,
        "another country's wings, none of ours: the flag with no badge"
    );
    let stationed = AirBaseCount {
        stationed: Some(true),
        ..count
    };
    assert_eq!(air_base_frame(&stationed, Some(true)), 4);
}

#[test]
fn a_container_without_capacity_gives_no_access() {
    let (mut frame, vision) = wings_frame(TableView::Complete(TableRows::StateFlags(vec![
        StateFlagRow {
            state: 7,
            demilitarized: 0,
            ..Default::default()
        },
    ])));
    assert!(wings_per_site(&frame, &vision).unwrap()[&(7, building_facts::AIR_BASE)].shown);
    let TableView::Complete(TableRows::AirBaseRegistrations(registrations)) =
        &mut *frame.tables[TableId::AirBaseRegistrations.index()]
    else {
        panic!("fixture holds complete registrations");
    };
    registrations[0].capacity_raw = 0;
    let count = wings_per_site(&frame, &vision).unwrap()[&(7, building_facts::AIR_BASE)];
    assert!(!count.shown);
    assert_eq!(
        count.stationed,
        Some(true),
        "the container still holds wings"
    );
}

#[test]
fn state_merge_ors_stationed_and_occupied() {
    assert_eq!(merge(Some(false), Some(true)), Some(true));
    assert_eq!(merge(Some(false), Some(false)), Some(false));
    assert_eq!(merge(None, Some(true)), Some(true));
    assert_eq!(merge(None, None), None);
}

#[test]
fn truncated_registrations_keep_surviving_rows_decided() {
    let (mut frame, vision) = wings_frame(TableView::Complete(TableRows::StateFlags(vec![
        StateFlagRow {
            state: 7,
            demilitarized: 0,
            ..Default::default()
        },
    ])));
    let TableView::Complete(TableRows::AirBaseRegistrations(registrations)) =
        &mut *frame.tables[TableId::AirBaseRegistrations.index()]
    else {
        panic!("fixture holds complete registrations");
    };
    registrations[0].wings.clear();
    registrations.push(AirBaseRegistrationRow {
        base: 0xB0,
        slot: 2,
        wings: vec![0xC1],
        capacity_raw: 100_000,
        carrier: Default::default(),
        transport_planes: Default::default(),
        fuel_demand_raw: Default::default(),
        fuel_allocated_raw: Default::default(),
    });
    let rows = match std::mem::replace(
        &mut *frame.tables[TableId::AirBaseRegistrations.index()],
        TableView::Unavailable {
            reason: "moved out".into(),
        },
    ) {
        TableView::Complete(rows) => rows,
        _ => panic!("fixture holds complete registrations"),
    };
    *frame.tables[TableId::AirBaseRegistrations.index()] =
        TableView::Truncated { rows, missing: 3 };
    let states = wings_per_site(&frame, &vision).unwrap();
    let count = states
        .get(&(7, building_facts::AIR_BASE))
        .copied()
        .unwrap_or_default();
    assert!(count.shown, "our access row survived the truncation");
    assert_eq!(
        count.stationed,
        Some(false),
        "our surviving row carries no wings: decided, not unknown"
    );
    assert_eq!(
        count.occupied,
        Some(true),
        "a surviving foreign wing decides occupancy true"
    );
    assert_eq!(
        air_base_frame(&count, Some(false)),
        0,
        "stationed-false beats the detail branch"
    );
}

fn native_gui(source: impl AsRef<str>) -> native::Gui {
    native::read_at(source.as_ref().as_bytes(), [1920, 1080])
}

use viewer_native_gui::test_support::SYNTHETIC_NAVAL_GUI;

use viewer_native_gui::test_support::SYNTHETIC_NAVY_GUI;

#[test]
fn counter_display_hides_kinds_and_owners_it_filters() {
    let all = CounterDisplay::default();
    assert!(all.land_plate(false, 7) && all.land_plate(true, 7) && all.naval_plate(7));
    let players = CounterDisplay {
        owners: Some(Arc::new(BTreeSet::from([1, 2]))),
        ..CounterDisplay::default()
    };
    assert!(players.land_plate(false, 2) && players.naval_plate(1));
    assert!(!players.land_plate(false, 7) && !players.land_plate(true, 7));
    assert!(!players.naval_plate(7));
    let guns = CounterDisplay {
        railway_guns: false,
        ..CounterDisplay::default()
    };
    assert!(guns.land_plate(false, 7) && !guns.land_plate(true, 7));
    let nobody = CounterDisplay {
        owners: Some(Arc::default()),
        ..CounterDisplay::default()
    };
    assert!(!nobody.land_plate(false, 0) && !nobody.naval_plate(0));
    assert!(players.port_item(Some(1)) && !players.port_item(Some(7)));
    assert!(players.port_item(None) && !nobody.port_item(Some(0)));
    let ports = CounterDisplay {
        ports: false,
        ..CounterDisplay::default()
    };
    assert!(!ports.port_item(Some(1)) && !ports.port_item(None) && ports.naval_plate(1));
}

#[test]
fn a_member_without_recorded_maxima_withholds_the_stack_bars() {
    let countries = Arc::new(Countries::default());
    let templates = Arc::new(TemplateIcons::default());
    let vision = Arc::new(Vision::sharing(1, []));
    let abilities = AbilityInputs::default();
    let tables = CounterTables {
        type_tints: &Default::default(),
        countries: &countries,
        icons: &templates,
        status: &Default::default(),
        movement: &Default::default(),
        actions: &Default::default(),
        vision: &vision,
        order: None,
        native: &Default::default(),
        orders: &Default::default(),
        guns: &Default::default(),
        abilities: &abilities,
        task_forces: &Default::default(),
        hq_providers: &Default::default(),
        portraits: &Default::default(),
        icon_inputs: &Default::default(),
    };
    let stack = |members: &[&LandUnit<'_>]| {
        land::land_stack(
            members,
            &tables,
            &Default::default(),
            false,
            &|_| false,
            false,
        )
    };
    let recorded = land_unit(1, 1, 10, 10, true);
    let mut unrecorded = land_unit(2, 1, 10, 10, true);
    unrecorded.bars = false;
    assert!(stack(&[&recorded]).bars);
    assert!(!stack(&[&recorded, &unrecorded]).bars);
    assert!(!stack(&[&unrecorded, &recorded]).bars);
}

#[test]
fn a_member_without_recorded_status_withholds_the_stack_glyphs() {
    content_reader_script::test_game::enter();
    with_counter_inputs_fixture(
        action_gui(false),
        |_| {},
        |_| {},
        |inputs| {
            let countries = Arc::new(Countries::default());
            let templates = Arc::new(TemplateIcons::default());
            let vision = Arc::new(Vision::sharing(1, []));
            let abilities = AbilityInputs::default();
            let tables = CounterTables {
                type_tints: &Default::default(),
                countries: &countries,
                icons: &templates,
                status: &Default::default(),
                movement: &Default::default(),
                actions: &Default::default(),
                vision: &vision,
                order: None,
                native: &Default::default(),
                orders: &Default::default(),
                guns: &Default::default(),
                abilities: &abilities,
                task_forces: &Default::default(),
                hq_providers: &Default::default(),
                portraits: &Default::default(),
                icon_inputs: &Default::default(),
            };
            let glyphs = |members: &[&LandUnit<'_>]| {
                let stack = land::land_stack(
                    members,
                    &tables,
                    &Default::default(),
                    false,
                    &|_| false,
                    false,
                );
                land::stack_style(&inputs, &stack).glyphs
            };
            let mut recorded = land_unit(1, 1, 10, 10, true);
            recorded.status.attrition = true;
            let mut unrecorded = land_unit(2, 1, 10, 10, true);
            unrecorded.bars = false;
            unrecorded.status_recorded = false;
            unrecorded.status.supply_ratio = counter_status::SupplyRatio::Missing;
            assert!(glyphs(&[&recorded]).attrition);
            assert_eq!(
                glyphs(&[&recorded, &unrecorded]),
                counter_status::Glyphs {
                    supply: counter_status::SupplyFrame::Uncertain,
                    ..Default::default()
                }
            );
            let mut witnessed = unrecorded.clone();
            witnessed.status.exercising = Some(true);
            witnessed.status.attrition = true;
            assert_eq!(
                glyphs(&[&unrecorded, &witnessed]),
                counter_status::Glyphs {
                    supply: counter_status::SupplyFrame::Uncertain,
                    training: Some(true),
                    ..Default::default()
                }
            );
            let mut idle = unrecorded.clone();
            idle.status.exercising = Some(false);
            assert_eq!(glyphs(&[&idle]).training, Some(false));
            assert_eq!(glyphs(&[&idle, &unrecorded]).training, None);
        },
    );
}

fn draw_land_action_fixture(
    guis: native::Guis,
    members: &[LandUnit<'_>],
    coarse: bool,
) -> (Batch, CounterReport) {
    draw_land_action_fixture_with(guis, members, coarse, &AbilityInputs::default(), |_| {})
}

fn draw_land_action_fixture_with(
    guis: native::Guis,
    members: &[LandUnit<'_>],
    coarse: bool,
    abilities: &AbilityInputs,
    configure: impl FnOnce(&mut CounterAssets),
) -> (Batch, CounterReport) {
    with_counter_inputs_fixture(
        guis,
        configure,
        |_| {},
        |inputs| {
            let countries = Arc::new(Countries::default());
            let templates = Arc::new(TemplateIcons::default());
            let vision = Arc::new(Vision::sharing(1, []));
            let tables = CounterTables {
                type_tints: &Default::default(),
                countries: &countries,
                icons: &templates,
                status: &Default::default(),
                movement: &Default::default(),
                actions: &Default::default(),
                vision: &vision,
                order: None,
                native: &Default::default(),
                orders: &Default::default(),
                guns: &Default::default(),
                abilities,
                task_forces: &Default::default(),
                hq_providers: &Default::default(),
                portraits: &Default::default(),
                icon_inputs: &Default::default(),
            };
            let members: Vec<_> = members.iter().collect();
            let stack = land::land_stack(
                &members,
                &tables,
                &Default::default(),
                false,
                &|_| false,
                coarse,
            );
            let style = land::stack_style(&inputs, &stack);
            let mut land = PreparedLand {
                stacks: vec![stack],
                styles: vec![style],
                sprites: vec![None],
                plans: vec![None],
                flotillas: Vec::new(),
                positions: Vec::new(),
                units: Vec::new(),
                layout: counter_move::DrawnLayout::new(inputs.hour),
                icons: Vec::new(),
                source_kinds: Vec::new(),
                groups: Vec::new(),
                items: Vec::new(),
                hovered: Vec::new(),
                levels: Vec::new(),
                extents: Default::default(),
                countries,
                templates,
                vision,
                order: Vec::new(),
                pointer: None,
                exits: Vec::new(),
                exits_running: Default::default(),
                time: 0.0,
                pointer_running: Default::default(),
                pointer_refused: Default::default(),
                pointer_drawn: Default::default(),
            };
            let mut report = CounterReport::default();
            let batch = land
                .land_plate(
                    &inputs,
                    &mut report,
                    &mut Vec::new(),
                    0,
                    [100.0, 200.0],
                    [81.0, 29.0],
                    inputs
                        .content
                        .defines
                        .stack_priority(inputs.camera.height, PriorityTable::Default),
                    &[false, true].map(|gun| super::land::PlateStyle::read(&inputs, gun)),
                )
                .unwrap()
                .unwrap()
                .batch
                .clone();
            (batch, report)
        },
    )
}

pub(super) fn with_counter_inputs_fixture<R>(
    guis: native::Guis,
    configure: impl FnOnce(&mut CounterAssets),
    configure_content: impl FnOnce(&mut CounterContent),
    run: impl FnOnce(CounterInputs<'_>) -> R,
) -> R {
    with_counter_inputs_and_content(guis, configure, configure_content, |inputs, _| run(inputs))
}

pub(super) fn with_counter_inputs_and_content<R>(
    guis: native::Guis,
    configure: impl FnOnce(&mut CounterAssets),
    configure_content: impl FnOnce(&mut CounterContent),
    run: impl FnOnce(CounterInputs<'_>, &CounterContent) -> R,
) -> R {
    let (art, fonts) = counter_art_fixture(guis);
    let mut assets = CounterAssets(art);
    configure(&mut assets);
    let mut content = CounterContent {
        mission_content: Err("not needed".into()),
        building_icons: Default::default(),
        geography: geography::Memo::default(),
        template_icons: TemplateIconMemo::default(),
        volunteer_air_defines: Err("unused in fixture".into()),
        common: counter_map_fixture(),
    };
    configure_content(&mut content);
    with_inputs(&content, &assets, &fonts, |inputs| run(inputs, &content))
}

#[test]
fn stationary_counter_hovers_keep_hidden_members_at_both_zoom_levels() {
    content_reader_script::test_game::enter();
    let mut hidden = land_unit(7, 1, 10, 10, true);
    hidden.sight = Sight::Partial;
    let members = [hidden, land_unit(3, 1, 10, 10, true)];
    for coarse in [false, true] {
        let report = draw_land_action_fixture(action_gui(false), &members, coarse).1;
        let hover = &report.land_hovers[0];
        assert!(!report.land_hovers.is_empty());
        assert_eq!(&*hover.members, &[7, 3]);
        assert_eq!(hover.coarse, coarse);
        assert!(hover.hidden);
        assert_eq!(hover.timing.len(), hover.members.len());
        assert_eq!(hover.remaining, None);
    }
}

#[test]
fn counter_action_current_hour_ladder_and_struck_ladder_draw_through_the_plate() {
    content_reader_script::test_game::enter();
    let strip = sheet("GFX_synthetic_actions", 12, [144, 13]);
    for (modifier, code) in [(0, 6), (100_000, 11)] {
        let movement = replay_data::rows::DivisionMovementRow {
            path_total: 1,
            strategic_redeployment: 1,
            strategic_action: replay_data::rows::Recorded::Value(
                replay_data::rows::StrategicAction::Available(
                    replay_data::rows::StrategicRedeploymentInputs {
                        country_modifier: 0,
                        state_modifier: Some(0),
                        friendly_state_modifier: Some(modifier),
                    },
                ),
            ),
            ..Default::default()
        };
        let mut unit = land_unit(1, 1, 10, 10, true);
        unit.action = counter_action::action_code(&movement, None, Err("not reached"), None);
        assert_eq!(unit.action, Ok(code));
        let (batch, report) = draw_land_action_fixture(action_gui(false), &[unit], false);
        let quad = batch.quads.iter().find(|quad| quad.texture == 2).unwrap();
        assert_eq!(quad.uv, strip.frame_uv(u32::from(code)));
        assert_eq!(quad.rect, [157.0, 209.0, 6.0, 6.5]);
        assert_eq!(quad.color, [1.0; 4]);
        assert!(report.land_action_gaps.is_empty());
        let hit = report
            .land_hovers
            .iter()
            .rev()
            .find(|hit| hit.contains([160.0, 212.0]))
            .unwrap();
        assert_eq!(hit.control, CounterControl::Action);
        assert_eq!(hit.rect, quad.rect);
    }
    let mut unit = land_unit(1, 1, 10, 10, true);
    unit.action = counter_action::action_code(
        &replay_data::rows::DivisionMovementRow {
            path_total: 1,
            strategic_redeployment: 1,
            ..Default::default()
        },
        None,
        Err("not reached"),
        None,
    );
    let (batch, report) = draw_land_action_fixture(action_gui(false), &[unit], false);
    assert!(batch.quads.iter().all(|quad| quad.texture != 2));
    assert_eq!(
        report.land_action_gaps[counter_action::CURRENT_RAIL_MODIFIERS],
        1
    );
}

fn strategic_action_frame(
    rows: Vec<replay_data::rows::DivisionMovementRow>,
    valid_layout: bool,
) -> DecodedFrame {
    use engine_executable::{build::FIXED_MODIFIER_GAP, executable_tables};
    use replay_data::{
        rows::DivisionRow, rows::LexicalTokenRow, rows::ModifierDefinitionRow, rows::TableId,
        rows::TableRows,
    };
    static TABLES: std::sync::OnceLock<executable_tables::Tables> = std::sync::OnceLock::new();
    let tables = TABLES.get_or_init(|| {
        synthetic_game::Executable::empty()
            .modifiers((0..FIXED_MODIFIER_GAP).map(|index| format!("synthetic_modifier_{index}")))
            .pinned()
            .build()
    });
    executable_tables::enter(Some(*tables));
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 0,
        metrics: Default::default(),
        tables: TableId::ALL.map(|table| TableView::Complete(TableRows::empty(table)).into()),
    };
    let divisions = rows
        .iter()
        .map(|row| DivisionRow {
            id: row.division,
            roster_owner: 1,
            logical_owner: 1,
            ..Default::default()
        })
        .collect();
    *frame.tables[TableId::Divisions.index()] =
        TableView::Complete(TableRows::Divisions(divisions));
    *frame.tables[TableId::DivisionMovement.index()] =
        TableView::Complete(TableRows::DivisionMovement(rows));
    let keys: Vec<_> = engine_executable::modifier_keys::all()
        .unwrap()
        .filter(|key| {
            replay_derive::fixed_layout::STRATEGIC_ACTION
                .keys
                .contains(&key.index)
        })
        .collect();
    *frame.tables[TableId::ModifierDefinitions.index()] =
        TableView::Complete(TableRows::ModifierDefinitions(
            keys.iter()
                .map(|key| ModifierDefinitionRow {
                    index: key.index,
                    token: if valid_layout { key.index + 1 } else { 0 },
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
        keys.iter()
            .map(|key| LexicalTokenRow {
                token: key.index + 1,
                literal: key.name.as_bytes().to_vec(),
            })
            .collect(),
    ));
    frame
}

#[test]
fn counter_action_layout_and_query_refusals_keep_other_movement_and_glyphs() {
    use replay_data::{
        rows::DivisionMovementRow, rows::StrategicAction, rows::StrategicActionFailure,
        rows::StrategicRedeploymentInputs,
    };
    let moving = |division, status| DivisionMovementRow {
        division,
        path_total: 1,
        path: vec![7],
        strategic_redeployment: 1,
        strategic_action: status,
        ..Default::default()
    };
    let rows = vec![
        moving(
            1,
            replay_data::rows::Recorded::Value(StrategicAction::Available(
                StrategicRedeploymentInputs::default(),
            )),
        ),
        moving(
            2,
            replay_data::rows::Recorded::Value(StrategicAction::Available(
                StrategicRedeploymentInputs {
                    country_modifier: 1,
                    ..Default::default()
                },
            )),
        ),
        moving(
            3,
            replay_data::rows::Recorded::Value(StrategicAction::Withheld(
                StrategicActionFailure::CountryModifier,
            )),
        ),
        DivisionMovementRow {
            division: 4,
            path_total: 1,
            path: vec![7],
            target: 7,
            ..Default::default()
        },
    ];
    let links = BTreeMap::from([(1, 1)]);
    for valid_layout in [true, false] {
        let frame = strategic_action_frame(rows.clone(), valid_layout);
        let hostility = counter_action::DestinationHostility::read(&frame, &links).unwrap();
        let actions = counter_action::Actions::read(&frame, &hostility, Some(1)).unwrap();
        content_reader_script::test_game::enter();
        for (division, code) in [(1, 6), (2, 11), (4, 3)] {
            let mut unit = land_unit(1, 1, 10, 10, true);
            unit.action = actions.get(division);
            let (batch, report) = draw_land_action_fixture(action_gui(false), &[unit], false);
            assert!(!batch.quads.is_empty());
            if division == 4 || valid_layout {
                assert!(report.land_action_gaps.is_empty());
                let quad = batch.quads.iter().find(|q| q.texture == 2).unwrap();
                assert_eq!(
                    quad.uv,
                    sheet("GFX_synthetic_actions", 12, [144, 13]).frame_uv(code)
                );
            } else {
                assert!(batch.quads.iter().all(|q| q.texture != 2));
                assert_eq!(report.land_action_gaps[counter_action::STRATEGIC_LAYOUT], 1);
            }
        }
        let mut unit = land_unit(1, 1, 10, 10, true);
        unit.action = actions.get(3);
        let (batch, report) = draw_land_action_fixture(action_gui(false), &[unit], false);
        assert!(!batch.quads.is_empty());
        assert!(batch.quads.iter().all(|q| q.texture != 2));
        assert_eq!(
            report.land_action_gaps[StrategicActionFailure::CountryModifier.reason()],
            1
        );
        let mut unit = land_unit(1, 1, 10, 10, true);
        unit.action = counter_action::aggregate_actions(actions.get(1), [(true, actions.get(4))]);
        assert_eq!(unit.action, Ok(0));
        let (batch, report) = draw_land_action_fixture(action_gui(false), &[unit], false);
        assert!(report.land_action_gaps.is_empty());
        assert_eq!(
            batch.quads.iter().find(|q| q.texture == 2).unwrap().uv,
            sheet("GFX_synthetic_actions", 12, [144, 13]).frame_uv(0)
        );
    }
}

#[test]
fn counter_action_centered_child_refuses_only_the_action_glyph() {
    content_reader_script::test_game::enter();
    let mut unit = land_unit(1, 1, 10, 10, true);
    unit.action = Ok(8);
    let (regular, regular_report) =
        draw_land_action_fixture(action_gui(false), &[unit.clone()], false);
    let (centered, centered_report) = draw_land_action_fixture(action_gui(true), &[unit], false);
    assert!(regular_report.land_action_gaps.is_empty());
    assert_eq!(centered_report.land_counters, 1);
    assert_eq!(centered_report.land_action_gaps.values().sum::<usize>(), 1);
    assert!(
        centered_report
            .land_action_gaps
            .keys()
            .next()
            .unwrap()
            .contains("centerPosition")
    );
    assert_eq!(
        regular
            .quads
            .iter()
            .map(|quad| quad.texture)
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(
        centered
            .quads
            .iter()
            .map(|quad| quad.texture)
            .collect::<Vec<_>>(),
        [1, 3]
    );
    for texture in [1, 3] {
        let before = regular
            .quads
            .iter()
            .find(|quad| quad.texture == texture)
            .unwrap();
        let after = centered
            .quads
            .iter()
            .find(|quad| quad.texture == texture)
            .unwrap();
        assert_eq!(before.rect, after.rect);
        assert_eq!(before.uv, after.uv);
        assert_eq!(before.color, after.color);
    }
}

#[test]
fn counter_action_hidden_children_and_named_refusals_survive_report_copy() {
    let layouts = layouts();
    let child = &layouts.land.as_ref().unwrap().action;
    let missing = Err(counter_action::CURRENT_RAIL_MODIFIERS);
    let content = ActionContent::load(child, LAND_WINDOW, None);
    for (own, coarse) in [(false, false), (true, true)] {
        assert!(content.frame(missing, own, coarse).unwrap().is_none());
    }
    assert!(content.frame(Ok(1), true, false).unwrap().is_none());
    let cause = content.frame(missing, true, false).unwrap_err();
    assert_eq!(cause, counter_action::CURRENT_RAIL_MODIFIERS);
    let mut report = CounterReport::default();
    *report
        .land_action_gaps
        .entry(cause.to_owned().into())
        .or_default() += 2;
    let mut copied = CounterReport::default();
    copied.assign(&report);
    assert_eq!(copied.land_action_gaps, report.land_action_gaps);
    assert!(copied.warnings().any(|line| line
        == "Land counter actions: 2 not drawn: current strategic-redeployment modifier operands."));
    copied.assign(&CounterReport::default());
    assert!(copied.land_action_gaps.is_empty());

    let cause = content.frame(Ok(8), true, false).unwrap_err();
    assert!(cause.contains(&child.sprite));
    let short = sheet(&child.sprite, 8, [96, 13]);
    let content = ActionContent::load(child, LAND_WINDOW, Some(&short));
    let cause = content.frame(Ok(8), true, false).unwrap_err();
    assert!(cause.contains(&child.sprite) && cause.contains("frame 9"));
    let rotated = Child {
        rotation: 0.5,
        ..child.clone()
    };
    let content = ActionContent::load(&rotated, LAND_WINDOW, Some(&short));
    let cause = content.frame(Ok(8), true, false).unwrap_err();
    assert!(cause.contains(&child.sprite) && cause.contains("rotation"));
    let centered = Child {
        centered: true,
        ..child.clone()
    };
    let content = ActionContent::load(&centered, LAND_WINDOW, Some(&short));
    let cause = content.frame(Ok(8), true, false).unwrap_err();
    assert!(cause.contains(&child.sprite) && cause.contains("centerPosition"));
}

#[test]
fn a_column_nearer_the_camera_sorts_to_a_smaller_depth_and_draws_on_top() {
    let camera = Camera {
        center: [3009.986, 1567.9869],
        height: 310.13113,
        map: MapSize::TEST,
        defines: MapDefines::test(),
        pose: viewer_gpu::camera::Pose::Map,
    };
    let matrix = camera.view_projection([2048, 1152]);
    let nearer = Camera::map_icon_depth(&matrix, [3092.0, 10.0, 1590.0]);
    let farther = Camera::map_icon_depth(&matrix, [3092.0, 10.0, 1605.0]);
    assert!(nearer < farther, "{nearer} {farther}");
    let behind = Camera::map_icon_depth(&matrix, [3009.986, 400.0, 1567.9869]);
    assert!(behind.is_finite());
    assert!(behind > farther);
}

fn countries(sides: impl IntoIterator<Item = (u32, Allegiance)>) -> Countries {
    Countries {
        sides: sides.into_iter().collect(),
        ..Default::default()
    }
}

fn rules(coarse: bool) -> JoinRules {
    JoinRules {
        coarse,
        color_mode: CounterColorMode::Allegiance,
        nato: false,
    }
}

fn land_fit<'a, 'm>(
    units: impl IntoIterator<Item = &'a LandUnit<'m>>,
    templates: &TemplateIcons,
    countries: &Countries,
    rules: JoinRules,
    undecided: &mut BTreeMap<u64, SharingUnknown>,
) -> Vec<Vec<&'a LandUnit<'m>>> {
    first_fit(units, |rep, unit| {
        land_join(unit, rep, templates, countries, rules, undecided)
    })
}

#[test]
fn land_items_share_by_type_and_allegiance_but_hq_and_own_orders_split() {
    let countries = countries([
        (1, Allegiance::Allied),
        (2, Allegiance::Allied),
        (3, Allegiance::Ours),
    ]);
    let templates = TemplateIcons {
        grouping: [
            (10, (b"infantry".to_vec(), -1, 0)),
            (20, (b"infantry".to_vec(), -1, 0)),
            (30, (b"infantry".to_vec(), 13, 0)),
        ]
        .into(),
        ..Default::default()
    };
    let first = land_unit(1, 1, 10, 10, true);
    let mut unit = land_unit(2, 1, 20, 20, true);
    unit.sight = Sight::Partial;
    assert!(
        unit.can_join(&first, &templates, &countries, rules(false))
            .unwrap()
    );
    unit.owner = 2;
    assert!(
        !unit
            .can_join(&first, &templates, &countries, rules(false))
            .unwrap()
    );
    unit.owner = 1;
    unit.template = 30;
    unit.alternate_template = 30;
    assert!(
        !unit
            .can_join(&first, &templates, &countries, rules(false))
            .unwrap()
    );
    let nato = JoinRules {
        nato: true,
        ..rules(false)
    };
    assert!(unit.can_join(&first, &templates, &countries, nato).unwrap());
    let mut keeps = templates.clone();
    keeps.grouping.insert(30, (b"infantry".to_vec(), 13, 1));
    assert!(!unit.can_join(&first, &keeps, &countries, nato).unwrap());
    unit.template = 10;
    unit.alternate_template = 10;
    unit.hq = true;
    assert!(
        !unit
            .can_join(&first, &templates, &countries, rules(false))
            .unwrap()
    );
    assert!(
        !first
            .can_join(&unit, &templates, &countries, rules(false))
            .unwrap()
    );
    unit.hq = false;
    let mut own = first.clone();
    own.owner = 3;
    unit.owner = 3;
    assert!(
        unit.can_join(&own, &templates, &countries, rules(false))
            .unwrap()
    );
    unit.orders_group = 44;
    assert!(
        !unit
            .can_join(&own, &templates, &countries, rules(false))
            .unwrap()
    );
    own.orders_group = 40;
    unit.orders_group = 41;
    own.group_parent = Some(7);
    unit.group_parent = Some(7);
    assert!(
        !unit
            .can_join(&own, &templates, &countries, rules(false))
            .unwrap()
    );
    own.parent_branch = Some(true);
    unit.parent_branch = Some(true);
    assert!(
        unit.can_join(&own, &templates, &countries, rules(false))
            .unwrap()
    );
    unit.owner = 2;
    own.owner = 2;
    assert!(
        unit.can_join(&own, &templates, &countries, rules(false))
            .unwrap()
    );
    unit.template = 30;
    unit.alternate_template = 30;
    unit.hq = true;
    unit.owner = 1;
    own.owner = 2;
    assert!(
        unit.can_join(&own, &templates, &countries, rules(true))
            .unwrap()
    );
    own.owner = 3;
    assert!(
        !unit
            .can_join(&own, &templates, &countries, rules(true))
            .unwrap()
    );
}

fn land_unit(
    id: u64,
    owner: u32,
    template: u64,
    alternate: u64,
    shared: bool,
) -> LandUnit<'static> {
    LandUnit {
        ordinal: 0,
        alternate_template: alternate,
        type_tint: Some(type_tint::Side::Neutral),
        shared_vision: Ok(shared),
        defending_area: Some(false),
        separate: false,
        id,
        owner,
        logical: Some(owner),
        flag_owner: owner,
        template,
        orders_group: 0,
        hq: false,
        sight: Sight::Full,
        hp: counter_status::FIXED,
        org: counter_status::FIXED,
        bars: true,
        status_recorded: true,
        status: counter_status::Member::default(),
        movement: None,
        action: Ok(1),
        group_parent: None,
        parent_branch: None,
        withdrawing: None,
        commander: Some(None),
        gun: None,
    }
}

#[test]
fn unit_flag_owner_keeps_the_coarse_roster_and_resolves_detailed_references() {
    for (case, roster, expeditionary, resolved, detailed) in [
        ("division", 7, 5_000, 5_000, 5_000),
        ("null reference", 7, 0, 0, 7),
        ("negative reference", 7, u32::MAX, u32::MAX, 7),
        ("signed boundary", 7, 0x8000_0000, 0x8000_0000, 7),
        ("railway gun alias", 11, 20, 5_000, 5_000),
        ("railway gun null reference", 11, 0, 5_000, 11),
        ("railway gun negative reference", 11, u32::MAX, 5_000, 11),
    ] {
        let resolve = |id| (id == expeditionary).then_some(resolved);
        assert_eq!(
            unit_flag_owner(roster, expeditionary, true, &resolve),
            roster,
            "{case}: coarse"
        );
        assert_eq!(
            unit_flag_owner(roster, expeditionary, false, &resolve),
            detailed,
            "{case}: detailed"
        );
    }
}

fn gun_unit(id: u64, owner: u32, name: &str, join_action: u8, group: u64) -> LandUnit<'static> {
    LandUnit {
        orders_group: group,
        gun: Some(GunUnit {
            timing: viewer_native_gui::counter_hovers::land::Timing {
                next: None,
                kind: None,
            },
            name: name.into(),
            location_warning: Some(false),
            join_action,
            action: Ok(join_action),
            position: None,
        }),
        ..land_unit(id, owner, 0, 0, true)
    }
}

#[test]
fn railway_gun_counters_keep_their_hover_at_both_zoom_levels() {
    content_reader_script::test_game::enter();
    let members = [
        gun_unit(5, 1, "synthetic_gun", 1, 0),
        gun_unit(9, 1, "synthetic_gun", 1, 0),
    ];
    for coarse in [false, true] {
        let report = draw_land_action_fixture(action_gui(false), &members, coarse).1;
        assert!(!report.land_hovers.is_empty());
        assert_eq!(report.railway_guns, 1);
        let hover = &report.land_hovers[0];
        assert!(hover.railway_gun);
        assert_eq!(&*hover.members, &[5, 9]);
        assert_eq!(hover.rect, [100.0, 200.0, 81.0, 29.0]);
        assert_eq!(hover.coarse, coarse);
        assert_eq!(hover.timing.len(), hover.members.len());
    }
}

#[test]
fn railway_guns_join_only_guns_of_one_definition_and_own_orders() {
    let countries = countries([(1, Allegiance::Allied), (3, Allegiance::Ours)]);
    let templates = TemplateIcons::default();
    let division = land_unit(1, 1, 10, 10, true);
    let gun = gun_unit(2, 1, "railway_gun", 1, 0);
    assert!(
        !gun.can_join(&division, &templates, &countries, rules(false))
            .unwrap()
    );
    assert!(
        !division
            .can_join(&gun, &templates, &countries, rules(false))
            .unwrap()
    );
    assert!(
        !gun.can_join(&division, &templates, &countries, rules(true))
            .unwrap()
    );
    let twin = gun_unit(3, 1, "railway_gun", 12, 9);
    assert!(
        twin.can_join(&gun, &templates, &countries, rules(false))
            .unwrap()
    );
    let other = gun_unit(4, 1, "super_heavy_railway_gun", 1, 0);
    assert!(
        !other
            .can_join(&gun, &templates, &countries, rules(false))
            .unwrap()
    );
    assert!(
        other
            .can_join(&gun, &templates, &countries, rules(true))
            .unwrap()
    );
    let own = gun_unit(5, 3, "railway_gun", 1, 7);
    let busy = gun_unit(6, 3, "railway_gun", 12, 7);
    assert!(
        !busy
            .can_join(&own, &templates, &countries, rules(false))
            .unwrap()
    );
    let idle = gun_unit(7, 3, "railway_gun", 1, 7);
    assert!(
        idle.can_join(&own, &templates, &countries, rules(false))
            .unwrap()
    );
    let elsewhere = gun_unit(8, 3, "railway_gun", 1, 8);
    assert!(
        !elsewhere
            .can_join(&own, &templates, &countries, rules(false))
            .unwrap()
    );
    let first_fitted = land_fit(
        [&own, &busy, &idle, &gun],
        &templates,
        &countries,
        rules(false),
        &mut BTreeMap::new(),
    );
    let plates: Vec<Vec<u64>> = first_fitted
        .iter()
        .map(|item| item.iter().map(|unit| unit.id).collect())
        .collect();
    assert_eq!(plates, vec![vec![5, 7], vec![6], vec![2]]);
}

fn ability_frame(rows: TableView) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 10,
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "not needed".into(),
            }
            .into()
        }),
        metrics: Metrics::default(),
    };
    *frame.tables[TableId::ActiveAbilities.index()] = rows;
    frame
}

fn ability(
    country: u32,
    ordinal: u32,
    caster: Option<u64>,
    phase: u8,
    token: &[u8],
) -> replay_data::rows::ActiveAbilityRow {
    replay_data::rows::ActiveAbilityRow {
        country,
        ordinal,
        caster,
        phase,
        token: token.to_vec(),
        ..Default::default()
    }
}

#[test]
fn the_ability_grid_takes_vector_order_and_the_last_visible_commander() {
    let rows = TableView::Complete(TableRows::ActiveAbilities(vec![
        ability(101, 2, Some(11), 0, b"third"),
        ability(101, 0, Some(11), 0, b"first"),
        ability(101, 1, Some(22), 0, b"other"),
        ability(101, 3, Some(11), 1, b"cooling_down"),
        ability(101, 4, None, 0, b"no_caster"),
        ability(102, 0, Some(11), 0, b"another_country"),
    ]));
    let links = BTreeMap::from([(101, 7), (102, 1)]);
    let abilities = AbilityInputs::read(&ability_frame(rows), &links).unwrap();
    assert_eq!(
        abilities.of(7, 11),
        Some(
            &[
                AbilityIcon {
                    country: 101,
                    ordinal: 0,
                    token: b"first".to_vec()
                },
                AbilityIcon {
                    country: 101,
                    ordinal: 2,
                    token: b"third".to_vec()
                },
            ][..]
        )
    );
    assert_eq!(abilities.of(7, 33), Some(&[][..]));
    let mut tokens = Some(Vec::new());
    update_abilities(&mut tokens, Sight::Full, Some(Some(11)), 7, &abilities);
    assert_eq!(tokens.as_ref().map(Vec::len), Some(2));
    update_abilities(&mut tokens, Sight::Partial, Some(Some(22)), 7, &abilities);
    update_abilities(&mut tokens, Sight::Full, Some(None), 7, &abilities);
    assert_eq!(tokens.as_ref().map(Vec::len), Some(2));
    update_abilities(&mut tokens, Sight::Full, Some(Some(33)), 7, &abilities);
    assert_eq!(tokens, Some(Vec::new()));
    update_abilities(&mut tokens, Sight::Full, None, 7, &abilities);
    assert_eq!(tokens, None);

    let truncated = TableView::Truncated {
        rows: TableRows::ActiveAbilities(vec![ability(101, 0, Some(11), 0, b"first")]),
        missing: 1,
    };
    let abilities = AbilityInputs::read(&ability_frame(truncated), &links).unwrap();
    assert_eq!(abilities.of(7, 11), None);
    let unknown = TableView::Complete(TableRows::ActiveAbilities(vec![ability(
        999,
        0,
        Some(11),
        0,
        b"first",
    )]));
    let abilities = AbilityInputs::read(&ability_frame(unknown), &links).unwrap();
    assert_eq!(abilities.of(7, 11), None);
}

#[test]
fn a_moving_guns_destination_is_enemy_only_when_its_controller_is_at_war() {
    use replay_data::{
        rows::FactionMemberRow, rows::RailwayGunCounterInputRow, rows::SubjectRow,
        rows::WarRelationRow,
    };
    let gun = |id: u64, last: u32| RailwayGunCounterInputRow {
        gun: id,
        logical_owner: 100,
        ordinary_path: vec![9, last],
        ..Default::default()
    };
    let mut frame = ability_frame(TableView::Unavailable {
        reason: "not needed".into(),
    });
    *frame.tables[TableId::RailwayGunCounterInputs.index()] =
        TableView::Complete(TableRows::RailwayGunCounterInputs(vec![
            gun(1, 10),
            gun(2, 11),
            gun(3, 12),
            gun(4, 13),
        ]));
    *frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(
        [(10, 200), (11, 300), (12, 0), (13, 101)]
            .map(|(id, controller)| ProvinceRow {
                id,
                controller,
                ..Default::default()
            })
            .to_vec(),
    ));
    *frame.tables[TableId::WarRelations.index()] =
        TableView::Complete(TableRows::WarRelations(vec![WarRelationRow {
            first_country: 100,
            second_country: 200,
            ..Default::default()
        }]));
    *frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(Vec::<FactionMemberRow>::new()));
    *frame.tables[TableId::Subjects.index()] =
        TableView::Complete(TableRows::Subjects(Vec::<SubjectRow>::new()));
    let links = BTreeMap::from([(100, 1), (101, 1), (200, 2), (300, 3)]);
    use replay_data::{
        rows::CountryCounterInputRow, rows::CountryLinkRow, rows::CountryRow, rows::DirectedWars,
        rows::Recorded, rows::WarCell,
    };
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        links
            .iter()
            .map(|(&id, &canonical_slot)| CountryLinkRow { id, canonical_slot })
            .collect(),
    ));
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        [(100, 1), (200, 2), (300, 3)]
            .map(|(id, canonical_slot)| CountryRow {
                id,
                canonical_slot,
                ..Default::default()
            })
            .to_vec(),
    ));
    *frame.tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(TableRows::CountryCounterInputs(
            [(200, 2, true), (300, 3, false)]
                .map(|(country, receiver_slot, active)| CountryCounterInputRow {
                    country,
                    directed_wars: Recorded::Value(Some(DirectedWars {
                        sample_hour: replay_data::rows::PacketHour::Stamped(frame.hour),
                        receiver_raw_id: country as i32,
                        receiver_slot,
                        matrix_count: 4,
                        cells: vec![WarCell {
                            target_slot: 1,
                            active,
                        }],
                        ..Default::default()
                    })),
                    ..Default::default()
                })
                .to_vec(),
        ));
    let inputs = gun_inputs(&frame, &links, Ok(&subunit_keys()));
    assert_eq!(
        inputs.destination_enemy,
        BTreeMap::from([
            (1, Ok(true)),
            (2, Ok(false)),
            (3, Ok(false)),
            (4, Ok(false))
        ])
    );
    *frame.tables[TableId::CountryCounterInputs.index()] = TableView::Unavailable {
        reason: "directed observations incomplete".into(),
    };
    let inputs = gun_inputs(&frame, &links, Ok(&subunit_keys()));
    assert_eq!(
        inputs.destination_enemy,
        BTreeMap::from([
            (1, Ok(true)),
            (2, Ok(false)),
            (3, Ok(false)),
            (4, Ok(false))
        ])
    );
    *frame.tables[TableId::WarRelations.index()] =
        TableView::Complete(TableRows::WarRelations(Vec::<WarRelationRow>::new()));
    let inputs = gun_inputs(&frame, &links, Ok(&subunit_keys()));
    assert_eq!(inputs.destination_enemy[&1], Ok(false));
    *frame.tables[TableId::WarRelations.index()] = TableView::Unavailable {
        reason: "war relations incomplete".into(),
    };
    let inputs = gun_inputs(&frame, &links, Ok(&subunit_keys()));
    assert_eq!(
        inputs.destination_enemy[&1],
        Err("action destination directed-war operands unavailable")
    );
    assert_eq!(inputs.destination_enemy[&3], Ok(false));
}

fn subunit_keys() -> TokenKeys {
    TokenKeys::new(&["railway_gun".to_owned()])
}

fn gun_inputs(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    subunits: Result<&TokenKeys, &str>,
) -> GunInputs {
    let hostility = counter_action::DestinationHostility::read(frame, links).unwrap();
    GunInputs::read(frame, &hostility, subunits).unwrap()
}

#[test]
fn gun_sub_unit_tokens_name_their_authored_key_under_the_lexers_fold() {
    use replay_data::{rows::LexicalTokenRow, rows::RailwayGunCounterInputRow};
    let gun = |gun, subunit_token| RailwayGunCounterInputRow {
        gun,
        subunit_token,
        province: 9,
        ..Default::default()
    };
    let mut frame = ability_frame(TableView::Unavailable {
        reason: "not needed".into(),
    });
    *frame.tables[TableId::RailwayGunCounterInputs.index()] =
        TableView::Complete(TableRows::RailwayGunCounterInputs(vec![
            gun(5, 3),
            gun(6, 4),
            gun(7, 8),
        ]));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 3,
                literal: b"Railway_Gun".to_vec(),
            },
            LexicalTokenRow {
                token: 4,
                literal: b"armored_train".to_vec(),
            },
        ]));
    let inputs = gun_inputs(&frame, &BTreeMap::new(), Ok(&subunit_keys()));
    assert_eq!(
        inputs.names,
        BTreeMap::from([
            (3, Ok("railway_gun".to_owned())),
            (4, Err("sub-unit armored_train is not authored".to_owned())),
            (8, Err("token 8 is not in lexical tokens".to_owned())),
        ])
    );
    assert_eq!(
        inputs.refused,
        BTreeSet::from([
            "railway gun sub-unit: sub-unit armored_train is not authored".to_owned(),
            "railway gun sub-unit: token 8 is not in lexical tokens".to_owned(),
        ])
    );
    let rows = inputs.rows.as_ref().expect("the census is complete");
    let name_of = |gun: &RailwayGunCounterInputRow| inputs.names[&gun.subunit_token].as_deref().ok();
    assert_eq!(name_of(&rows[0]), Some("railway_gun"));
    assert_eq!(name_of(&rows[1]), None);
    let countries = countries([(1, Allegiance::Allied)]);
    let templates = TemplateIcons::default();
    let folded = gun_unit(1, 1, name_of(&rows[0]).unwrap(), 1, 0);
    let authored = gun_unit(2, 1, "railway_gun", 1, 0);
    assert!(
        folded
            .can_join(&authored, &templates, &countries, rules(false))
            .unwrap()
    );
    let other = gun_unit(3, 1, "Railway_Gun", 1, 0);
    assert!(
        !other
            .can_join(&folded, &templates, &countries, rules(false))
            .unwrap(),
        "the join compares the authored key, not the token's spelling"
    );
    for (nato, frame) in [
        (false, templates::PICTORIAL_FRAME),
        (true, templates::NATO_FRAME),
    ] {
        let icon = templates::type_icon(&folded.gun.as_ref().unwrap().name, -1, nato, &|_| true);
        assert_eq!(icon.sprite, "GFX_unit_railway_gun_icon_medium_white");
        assert_eq!(icon.frame, frame);
    }
    let unloaded = gun_inputs(&frame, &BTreeMap::new(), Err("no units"));
    assert!(unloaded.names.values().all(Result::is_err));
    assert!(
        unloaded
            .refused
            .contains("railway gun sub-unit: sub-units unavailable: no units")
    );
    let ambiguous = TokenKeys::new(&BTreeSet::from([
        "railway_gun".to_owned(),
        "RAILWAY_GUN".to_owned(),
    ]));
    let refused = gun_inputs(&frame, &BTreeMap::new(), Ok(&ambiguous));
    assert!(refused.names.values().all(Result::is_err));
    assert!(refused.refused.contains(
        "railway gun sub-unit: token Railway_Gun names several content keys: RAILWAY_GUN, railway_gun"
    ));
}

#[test]
fn an_assigned_report_copies_and_clears_its_refused_tokens() {
    let refusing = CounterReport {
        refused_tokens: BTreeSet::from([
            "air_base folds to several tokens: 3, 4 and perhaps more".to_owned(),
            "railway gun sub-unit: token 8 is not in lexical tokens".to_owned(),
        ]),
        ..Default::default()
    };
    let mut kept = CounterReport::default();
    kept.assign(&refusing);
    assert_eq!(kept.refused_tokens, refusing.refused_tokens);
    assert_eq!(
        kept.warnings().collect::<Vec<_>>(),
        [
            "Counters: refused token: air_base folds to several tokens: 3, 4 and perhaps more.",
            "Counters: refused token: railway gun sub-unit: token 8 is not in lexical tokens.",
        ]
    );
    kept.assign(&CounterReport::default());
    assert!(kept.refused_tokens.is_empty());
    assert_eq!(kept.warnings().count(), 0);
}

#[test]
fn base_icon_refusals_reach_the_diagnostics_by_reason() {
    let mut refusing = CounterReport {
        refused_tokens: BTreeSet::from(["building naval_hq folds to several tokens".into()]),
        unresolved_facility_sprites: BTreeMap::from([(
            "GFX_mapicon_facility_specialization_air".into(),
            2,
        )]),
        naval_base_hits: BTreeMap::from([(0, 9)]),
        ..Default::default()
    };
    refusing
        .unknown_bases
        .naval_bases
        .insert("the base's recorded country list is unreadable".into(), 3);
    refusing.unknown_bases.facilities.insert(
        "facility building yard is declared more than once, differently".into(),
        1,
    );
    let mut kept = CounterReport::default();
    kept.assign(&refusing);
    assert_eq!(kept, refusing);
    assert_eq!(kept.unknown_bases.totals(), [3, 0, 1]);
    assert_eq!(
        kept.warnings().collect::<Vec<_>>(),
        [
            "Counters: refused token: building naval_hq folds to several tokens.",
            "Counters: 3 naval base icons not drawn: the base's recorded country list is unreadable.",
            "Counters: 1 special-facility icons not drawn: facility building yard is declared more than once, differently.",
            "Counters: 2 special-facility icons keep their drawable: the content declares no sprite GFX_mapicon_facility_specialization_air.",
        ]
    );
    kept.assign(&CounterReport::default());
    assert_eq!(kept.warnings().count(), 0);
}

#[test]
fn task_force_refusals_reach_the_diagnostics_by_reason_and_id() {
    let mut refusing = CounterReport::default();
    refusing.refuse_task_forces("task-force motion row absent", [(9_u64 << 32) | 51]);
    refusing.refuse_task_forces(
        "the native port sort's fleet-handle subtraction overflows",
        [(4_u64 << 32) | 51, (2_u64 << 32) | 51],
    );
    refusing
        .unplaced_task_force_routes
        .entry("the content movement map is not loaded")
        .or_default()
        .insert((7_u64 << 32) | 51);
    let mut kept = CounterReport::default();
    kept.assign(&refusing);
    assert_eq!(kept, refusing);
    assert_eq!(
        kept.warnings().collect::<Vec<_>>(),
        [
            "Counters: 1 task forces not drawn: task-force motion row absent; task forces 9.",
            "Counters: 2 task forces not drawn: the native port sort's fleet-handle subtraction overflows; task forces 2, 4.",
            "Counters: 1 moving task forces stand on their province: the content movement map is not loaded; task forces 7.",
        ]
    );
    kept.assign(&CounterReport::default());
    assert_eq!(kept.warnings().count(), 0);
}

#[test]
fn gun_inputs_need_the_whole_census_and_keep_rail_status_per_province() {
    use replay_data::{
        rows::LexicalTokenRow, rows::RailCooldownRow, rows::RailEdgeRow,
        rows::RailwayGunCounterInputRow, rows::RailwaySelectionInputs, rows::Recorded,
    };
    let gun = RailwayGunCounterInputRow {
        gun: 5,
        subunit_token: 3,
        province: 9,
        ..Default::default()
    };
    let mut frame = ability_frame(TableView::Unavailable {
        reason: "not needed".into(),
    });
    *frame.tables[TableId::RailwayGunCounterInputs.index()] = TableView::Truncated {
        rows: TableRows::RailwayGunCounterInputs(vec![gun.clone()]),
        missing: 2,
    };
    let inputs = gun_inputs(&frame, &BTreeMap::new(), Ok(&subunit_keys()));
    assert!(inputs.rows.is_none());
    assert_eq!(inputs.recorded, 1);

    *frame.tables[TableId::RailwayGunCounterInputs.index()] =
        TableView::Complete(TableRows::RailwayGunCounterInputs(vec![gun]));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 3,
                literal: b"railway_gun".to_vec(),
            },
            LexicalTokenRow {
                token: 4,
                literal: b"unrelated".to_vec(),
            },
        ]));
    *frame.tables[TableId::RailEdges.index()] = TableView::Complete(TableRows::RailEdges(vec![
        RailEdgeRow {
            from: 9,
            to: 10,
            level: 1,
        },
        RailEdgeRow {
            from: 9,
            to: 11,
            level: 1,
        },
    ]));
    let inputs = gun_inputs(&frame, &BTreeMap::new(), Ok(&subunit_keys()));
    assert_eq!(inputs.rows.as_ref().map(Vec::len), Some(1));
    assert_eq!(
        inputs.names,
        BTreeMap::from([(3, Ok("railway_gun".to_owned()))])
    );
    assert_eq!(inputs.rail, None);
    *frame.tables[TableId::RailCooldowns.index()] =
        TableView::Complete(TableRows::RailCooldowns(vec![RailCooldownRow {
            province: 9,
            cooldown_ticks: 0,
            selection: Some(RailwaySelectionInputs {
                levels: None,
                pair_count: Some(-1),
                building_level: None,
            })
            .into(),
            selection_hour: Some(frame.hour).into(),
            ..Default::default()
        }]));
    let inputs = gun_inputs(&frame, &BTreeMap::new(), Ok(&subunit_keys()));
    assert_eq!(
        inputs.rail,
        Some(BTreeMap::from([(
            9,
            viewer_unit_reads::railway_gun_counter::RailStatus {
                neighbor_pairs: -1,
                cooldown_ticks: 0
            }
        )]))
    );
    *frame.tables[TableId::RailEdges.index()] = TableView::Unavailable {
        reason: "sorted pairs unreadable".into(),
    };
    assert_eq!(
        gun_inputs(&frame, &BTreeMap::new(), Ok(&subunit_keys())).rail,
        inputs.rail
    );
    for selection in [
        Recorded::NotRecorded,
        None.into(),
        Some(RailwaySelectionInputs::default()).into(),
    ] {
        *frame.tables[TableId::RailCooldowns.index()] =
            TableView::Complete(TableRows::RailCooldowns(vec![RailCooldownRow {
                province: 9,
                selection,
                selection_hour: Some(frame.hour).into(),
                ..Default::default()
            }]));
        assert_eq!(
            gun_inputs(&frame, &BTreeMap::new(), Ok(&subunit_keys())).rail,
            Some(BTreeMap::new())
        );
    }
    for selection_hour in [
        Recorded::NotRecorded,
        None.into(),
        Some(frame.hour - 1).into(),
    ] {
        *frame.tables[TableId::RailCooldowns.index()] =
            TableView::Complete(TableRows::RailCooldowns(vec![RailCooldownRow {
                province: 9,
                selection: Some(RailwaySelectionInputs {
                    pair_count: Some(0),
                    ..Default::default()
                })
                .into(),
                selection_hour,
                ..Default::default()
            }]));
        assert_eq!(
            gun_inputs(&frame, &BTreeMap::new(), Ok(&subunit_keys())).rail,
            Some(BTreeMap::new())
        );
    }
}

fn naval_context<'a>(
    vision: &'a Vision,
    native: &'a NativeInputs,
    missions: BTreeMap<u64, u32>,
) -> NavalContext<'a> {
    NavalContext {
        type_tints: Box::leak(Box::default()),
        vision,
        native,
        ship_inputs: None,
        aggression: &[],
        ship_task_forces: Arc::new(BTreeMap::new()),
        ships_complete: true,
        ships: Arc::new(BTreeMap::new()),
        country_ids: Box::leak(Box::default()),
        ideology: &|_| Ok(None),
        missions: Arc::new(missions),
        forces: &[],
        mission_skips: Default::default(),
        forces_complete: true,
    }
}

#[test]
fn sea_items_join_by_allegiance_and_own_fleets_stay_apart() {
    use replay_data::rows::TaskForceRow;
    let countries = countries([
        (0, Allegiance::Ours),
        (1, Allegiance::Allied),
        (2, Allegiance::Allied),
        (3, Allegiance::Enemy),
    ]);
    let vision = Vision::default();
    let native = NativeInputs::default();
    let context = naval_context(&vision, &native, BTreeMap::new());
    let force = |id: u64, fleet: u64| TaskForceRow {
        id,
        fleet,
        mission: 1,
        ..Default::default()
    };
    fn unit(force: &TaskForceRow, slot: u32) -> NavalUnit<'_> {
        NavalUnit {
            force,
            ordinal: 0,
            slot,
            logical: Some(slot),
            flag_owner: slot,
            sight: Sight::Full,
            byte: 255,
            ships: Some(1),
            position: None,
            strength: counter_status::FIXED,
        }
    }
    let (fa, fb, fc) = (force(10, 0), force(11, 0), force(12, 0));
    let join = |a: &NavalUnit<'_>, b: &NavalUnit<'_>| {
        naval_can_join(a, b, rules(false), &countries, &context)
    };
    assert!(join(&unit(&fa, 1), &unit(&fb, 2)));
    assert!(!join(&unit(&fa, 1), &unit(&fc, 3)));
    let (own_a, own_b) = (force(20, 5), force(21, 6));
    assert!(!join(&unit(&own_a, 0), &unit(&own_b, 0)));
    assert!(join(&unit(&own_a, 0), &unit(&force(22, 5), 0)));
    for coarse in [false, true] {
        let country = JoinRules {
            coarse,
            color_mode: CounterColorMode::Country,
            nato: false,
        };
        let join = |a: &NavalUnit<'_>, b: &NavalUnit<'_>| {
            naval_can_join(a, b, country, &countries, &context)
        };
        assert!(!join(&unit(&fa, 1), &unit(&fb, 2)));
        assert!(join(&unit(&fa, 1), &unit(&fb, 1)));
    }
}

#[test]
fn reserve_is_unknown_without_the_recorded_roster() {
    let vision = Vision::default();
    let native = NativeInputs::default();
    let context = naval_context(&vision, &native, BTreeMap::new());
    assert_eq!(context.reserve(10), None);
    assert_eq!(context.reserve(0), Some(false));
}

#[test]
fn naval_bars_fill_or_hide() {
    assert_eq!(
        mean_bars([Some((50_000, 100_000))].into_iter()),
        NavalBars::Fill(1.0, 0.5)
    );
    assert_eq!(
        mean_bars([None].into_iter()),
        NavalBars::Unknown(viewer_counter_kit::flotilla::BARS_UNRECORDED.to_owned())
    );
    assert_eq!(mean_bars(std::iter::empty()), NavalBars::Hidden);
    assert_eq!(
        mean_bars([Some((counter_inputs::UNDEFINED, counter_inputs::UNDEFINED))].into_iter()),
        NavalBars::Fill(
            counter_inputs::bar_fill(counter_inputs::UNDEFINED),
            counter_inputs::bar_fill(counter_inputs::UNDEFINED),
        )
    );
}

#[test]
fn a_recorded_column_order_decides_the_merge_and_the_plate_order() {
    let countries = countries([
        (1, Allegiance::Allied),
        (2, Allegiance::Allied),
        (3, Allegiance::Ours),
    ]);
    let templates = TemplateIcons {
        grouping: [
            (10, (b"infantry".to_vec(), -1, 0)),
            (20, (b"infantry".to_vec(), 5, 0)),
            (21, (b"infantry".to_vec(), -1, 0)),
        ]
        .into(),
        priorities: [(10, 7), (20, 7), (21, 7)].into(),
        ..Default::default()
    };
    let rom_a = land_unit(0x10, 1, 10, 10, true);
    let rom_b = land_unit(0x11, 1, 10, 10, true);
    let hun = land_unit(0x20, 2, 20, 21, true);

    let hidden: Vec<LandUnit> = [&rom_a, &hun]
        .into_iter()
        .map(|unit| LandUnit {
            shared_vision: Ok(false),
            alternate_template: 21,
            ..unit.clone()
        })
        .collect();
    let merged = land_fit(
        &hidden,
        &templates,
        &countries,
        rules(false),
        &mut BTreeMap::new(),
    );
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].len(), 2);

    let cold = land_fit(
        [&rom_a, &rom_b, &hun],
        &templates,
        &countries,
        rules(false),
        &mut BTreeMap::new(),
    );
    let cold: Vec<(u32, usize)> = cold
        .iter()
        .map(|item| (item[0].owner, item.len()))
        .collect();
    assert_eq!(cold, vec![(1, 2), (2, 1)]);
    let coarse = land_fit(
        [&rom_a, &rom_b, &hun],
        &templates,
        &countries,
        rules(true),
        &mut BTreeMap::new(),
    );
    assert_eq!(coarse.len(), 1);
    assert_eq!(coarse[0].len(), 3);
    let by_country = JoinRules {
        coarse: true,
        color_mode: CounterColorMode::Country,
        nato: false,
    };
    let split = land_fit(
        [&rom_a, &rom_b, &hun],
        &templates,
        &countries,
        by_country,
        &mut BTreeMap::new(),
    );
    let split: Vec<(u32, usize)> = split
        .iter()
        .map(|item| (item[0].owner, item.len()))
        .collect();
    assert_eq!(split, vec![(1, 2), (2, 1)]);

    let order =
        counter_order::RecordedOrder::from_rows(&[replay_data::rows::ProvinceUnitOrderRow {
            province: 500,
            mixed: vec![0x20, 0x10, 0x11],
            naval: Vec::new(),
        }]);
    let mut admitted: Vec<LandUnit> = [&rom_a, &rom_b, &hun]
        .into_iter()
        .map(|unit| LandUnit {
            ordinal: order.mixed(unit.id, 500).unwrap(),
            ..unit.clone()
        })
        .collect();
    admitted.sort_by_key(|unit| unit.ordinal);
    let recorded = land_fit(
        &admitted,
        &templates,
        &countries,
        rules(false),
        &mut BTreeMap::new(),
    );
    let plates: Vec<(u32, usize)> = recorded
        .iter()
        .map(|item| (item[0].flag_owner, item.len()))
        .collect();
    assert_eq!(plates, vec![(2, 1), (1, 2)]);
}

#[test]
fn lead_type_and_supply_follow_different_ends_of_the_admitted_members() {
    let countries = countries([(1, Allegiance::Allied)]);
    let declared = |_: &str| true;
    let plain_a = templates::type_icon("type_a", -1, false, &declared);
    let plain_b = templates::type_icon("type_b", -1, false, &declared);
    let custom = templates::type_icon("type_a", 7, false, &declared);
    let template = |name: &str, custom_icon| TemplateIcon {
        name: name.to_owned(),
        custom_icon,
        custom_icon_nato: 0,
    };
    let icons = TemplateIcons {
        by_template: [
            (10, template("type_a", -1)),
            (20, template("type_b", -1)),
            (30, template("type_a", 7)),
        ]
        .into(),
        ..Default::default()
    };
    let mut a = land_unit(1, 1, 10, 20, true);
    a.status.supply_ratio = counter_status::SupplyRatio::exact(25_000);
    let mut b = land_unit(2, 1, 20, 10, true);
    b.status.supply_ratio = counter_status::SupplyRatio::exact(75_000);
    let mut sea = land_unit(3, 1, 30, 20, true);
    sea.status.naval_transfer = Some(true);
    let mut partial = land_unit(4, 1, 30, 20, true);
    partial.sight = Sight::Partial;
    for (order, icon, supply) in [
        ([&a, &b, &sea, &partial], &plain_b, counter_status::SupplyRatio::exact(75_000)),
        ([&b, &a, &sea, &partial], &plain_a, counter_status::SupplyRatio::exact(25_000)),
    ] {
        let fitted = land_fit(order, &icons, &countries, rules(true), &mut BTreeMap::new());
        assert_eq!(fitted.len(), 1);
        let rep = fitted[0][0];
        assert_eq!(
            icons.icon(
                rep.template,
                rep.alternate_template,
                rep.shared_vision.unwrap(),
                false,
                &declared
            ),
            Some(icon.clone())
        );
        let visible = fitted[0]
            .iter()
            .filter(|u| u.sight == Sight::Full)
            .map(|u| u.status)
            .collect::<Vec<_>>();
        assert_eq!(counter_status::stack_supply(&visible), supply);
    }
    let icon = |template, alternate, shared, nato| {
        icons.icon(template, alternate, shared, nato, &declared)
    };
    assert_eq!(icon(30, 20, true, false), Some(custom));
    assert_eq!(icon(30, 20, false, false), Some(plain_b.clone()));
    assert_eq!(icon(10, 20, true, false), Some(plain_b.clone()));
    let nato_b = templates::type_icon("type_b", -1, true, &declared);
    assert_eq!(nato_b.frame, templates::NATO_FRAME);
    assert_eq!(icon(30, 20, true, true), Some(nato_b));
    assert_eq!(
        icons.icon(30, 20, true, false, &|name| name != "GFX_div_templ_7_small"),
        Some(plain_b)
    );
}

#[test]
fn mixed_plate_lead_follows_source_order_not_majority_or_strength() {
    let countries = countries([(1, Allegiance::Allied), (2, Allegiance::Allied)]);
    let templates = TemplateIcons::default();
    let mut first = land_unit(30, 2, 10, 10, true);
    first.sight = Sight::Partial;
    first.hp = 1;
    let majority = [
        land_unit(10, 1, 20, 20, true),
        land_unit(20, 1, 20, 20, true),
    ];
    let sources = [vec![&first], majority.iter().collect()];
    let a = land_fit(
        sources.iter().flatten().copied(),
        &templates,
        &countries,
        rules(true),
        &mut BTreeMap::new(),
    );
    let b = land_fit(
        sources.iter().rev().flatten().copied(),
        &templates,
        &countries,
        rules(true),
        &mut BTreeMap::new(),
    );
    assert_eq!((a.len(), b.len()), (1, 1));
    assert_eq!((a[0][0].owner, b[0][0].owner), (2, 1));
    let members =
        |plate: &[&LandUnit<'_>]| plate.iter().map(|unit| unit.id).collect::<BTreeSet<_>>();
    assert_eq!(members(&a[0]), members(&b[0]));
}

#[test]
fn a_port_counter_sums_capitals_over_every_member_and_selects_its_hull() {
    use replay_data::{rows::ShipCounterInputs, rows::ShipModelRow};
    let model = |ship, ordinal, flags, variant, name: &str| ShipModelRow {
        ship,
        variant,
        fallback_sprite: name.as_bytes().to_vec(),
        formation_flags: flags,
        counter: Some(ShipCounterInputs {
            roster_ordinal: ordinal,
            definition_valid: 1,
            ..Default::default()
        }),
        ..Default::default()
    };
    let models = [
        model(1, 0, 1 << 8, 101, "destroyer"),
        model(2, 1, counter_order::CAPITAL_FLAG, 102, "battleship"),
        model(3, 0, 1 << 7, 103, "light_cruiser"),
        model(4, 1, counter_order::CAPITAL_FLAG, 102, "battleship"),
        model(5, 2, 1 << 8, 101, "destroyer"),
    ];
    fn hull(model: &ShipModelRow, strength: i64) -> counter_order::Hull<'_> {
        counter_order::Hull { strength, model }
    }
    let inputs = ShipInputs {
        tables: super::naval::ShipTables::from_rosters(
            [
                (10, vec![hull(&models[0], 50), hull(&models[1], 90)]),
                (
                    11,
                    vec![
                        hull(&models[2], 60),
                        hull(&models[3], 90),
                        hull(&models[4], 50),
                    ],
                ),
            ]
            .into(),
        ),
        equipment: Err("unreached equipment content"),
    };
    let cells = inputs.cells(&[10, 11], true);
    assert_eq!(cells.capitals, Some(2));
    assert_eq!(
        cells.hull,
        counter_order::HullIcon::Resolved(Some(b"battleship".to_vec()))
    );
    assert_eq!(cells.carrier, BTreeSet::from([None]));
    assert_eq!(inputs.cells(&[10, 99], false).capitals, Some(1));
    let mut legacy = models[1].clone();
    legacy.counter = None;
    let inputs = ShipInputs {
        tables: super::naval::ShipTables::from_rosters([(10, vec![hull(&legacy, 90)])].into()),
        equipment: Err("unreached equipment content"),
    };
    let cells = inputs.cells(&[10], false);
    assert_eq!(cells.capitals, None);
    assert_eq!(cells.hull, counter_order::HullIcon::Unrecorded);
}

#[test]
fn only_provinces_the_camera_admits_offer_unit_sources() {
    let camera = Placement {
        center: [3000.0, 1000.0],
        height: 50.0,
        pose: viewer_gpu::camera::Pose::Map,
    }
    .on(MapSize::TEST, MapDefines::test());
    let bounds = edge_boxes();
    let admission = Admission::new(camera, [1024, 768]);
    let sources: Vec<u32> = (0..=7)
        .filter(|&province| admission.admits_province(&bounds, province))
        .collect();
    assert_eq!(sources, [1, 2, 5, 6]);
}

fn land_markers(attackers: Vec<u64>, flag: Engaged, sides: TableView) -> LandMarkers {
    use replay_data::{rows::DivisionMovementRow, rows::DivisionRow, rows::LandCombatRow};
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 5,
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "not captured".into(),
            }
            .into()
        }),
        metrics: Default::default(),
    };
    *frame.tables[TableId::LandCombats.index()] =
        TableView::Complete(TableRows::LandCombats(vec![LandCombatRow {
            id: 62,
            attacker_classes: vec![0; attackers.len()],
            attackers,
            defenders: vec![20],
            ..Default::default()
        }]));
    *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(
        [(1, 11), (2, 22), (3, 33), (6, 66)]
            .map(|(id, province)| DivisionRow {
                id,
                province,
                ..Default::default()
            })
            .to_vec(),
    ));
    *frame.tables[TableId::DivisionMovement.index()] =
        TableView::Complete(TableRows::DivisionMovement(
            [(1, 0), (2, 1), (3, 1), (4, 0), (5, 1), (6, 1)]
                .map(|(division, map_entity)| DivisionMovementRow {
                    division,
                    map_entity,
                    ..Default::default()
                })
                .to_vec(),
        ));
    *frame.tables[TableId::LandCombatSides.index()] = sides;
    let route = |stack_kind| viewer_unit_reads::counter_move::DivisionRoute {
        stack_kind,
        kind_resolved: true,
        ..Default::default()
    };
    let movement = Movements {
        by_division: Arc::new(BTreeMap::from([(2, route(9)), (3, route(0)), (6, Default::default())])),
        ..Default::default()
    };
    LandMarkers::read(
        &frame,
        &Vision::sharing(1, []),
        &BTreeMap::from([(1, 1), (5, 5)]),
        &(BTreeMap::from([(62, flag)]), Provenance::Indexed),
        &BTreeMap::new(),
        &movement,
        &LandCombatDefines::VANILLA,
    )
    .unwrap()
}

#[test]
fn the_combat_wedge_points_from_the_first_attacker_with_a_map_entity() {
    use viewer_engagements::engaged::Engaged;
    let unseen = || {
        TableView::Complete(TableRows::LandCombatSides(vec![
            replay_data::rows::LandCombatSideRow {
                combat: 62,
                attacker_primary: vec![1],
                defender_primary: vec![5],
                ..Default::default()
            },
        ]))
    };
    let placed = |attackers: Vec<u64>| {
        let markers = land_markers(attackers, Engaged::No, unseen());
        assert!(markers.gaps.is_empty(), "{:?}", markers.gaps);
        markers.markers[0].attacker
    };
    assert_eq!(placed(vec![1, 2, 3]), Some((22, 9)));
    assert_eq!(placed(vec![1]), None);
    assert_eq!(placed(vec![4, 2]), Some((22, 9)));
    let refused = |attackers: Vec<u64>| {
        let markers = land_markers(attackers, Engaged::No, unseen());
        assert!(markers.markers.is_empty());
        markers.gaps
    };
    assert_eq!(
        refused(vec![7, 2]),
        BTreeMap::from([(
            "icon omitted: an attacker's map entity is unrecorded (it has no DivisionMovement row), so the icon's first mapped attacker is unknown".to_owned(),
            BTreeSet::from([62])
        )])
    );
    assert_eq!(placed(vec![4, 3, 8]), Some((33, 0)));
    assert_eq!(
        refused(vec![1, 6, 2]),
        BTreeMap::from([(
            "icon omitted: the first mapped attacker's stack kind is unknown (content cannot place its route)".to_owned(),
            BTreeSet::from([62])
        )])
    );
    assert_eq!(
        refused(vec![1, 5]),
        BTreeMap::from([(
            "icon omitted: the first mapped attacker's province is unrecorded (it has no Divisions row)".to_owned(),
            BTreeSet::from([62])
        )])
    );
}

#[test]
fn an_icon_needs_its_side_row_and_counts_initial_frame_without_history() {
    use viewer_engagements::{engaged::Engaged, icon_frames::{Kept, LAND_NO_HISTORY}};
    for flag in [Engaged::No, Engaged::Yes] {
        for (sides, how) in [
            (
                TableView::Unavailable {
                    reason: "sides read failed".into(),
                },
                "the LandCombatSides table is unavailable (sides read failed), so the combat is unknown",
            ),
            (
                TableView::Truncated {
                    rows: TableRows::empty(TableId::LandCombatSides),
                    missing: 1,
                },
                "the truncated LandCombatSides table did not reach the combat",
            ),
        ] {
            let markers = land_markers(vec![1, 2], flag, sides);
            assert!(markers.markers.is_empty());
            assert_eq!(
                markers.gaps,
                BTreeMap::from([(
                    format!(
                        "icon omitted (it exists only while both sides' country lists are non-empty): {how}"
                    ),
                    BTreeSet::from([62])
                )])
            );
        }
        let empty = TableView::Complete(TableRows::LandCombatSides(vec![
            replay_data::rows::LandCombatSideRow {
                combat: 62,
                defender_primary: vec![5],
                ..Default::default()
            },
        ]));
        let markers = land_markers(vec![1, 2], flag, empty);
        assert!(markers.markers.is_empty() && markers.gaps.is_empty());
    }
    let live = TableView::Complete(TableRows::LandCombatSides(vec![
        replay_data::rows::LandCombatSideRow {
            combat: 62,
            attacker_primary: vec![1],
            defender_primary: vec![5],
            ..Default::default()
        },
    ]));
    let unseen = land_markers(vec![1, 2], Engaged::No, live);
    let [marker] = &unseen.markers[..] else {
        panic!("one marker");
    };
    assert!(!marker.seen);
    assert_eq!(marker.attacker, Some((22, 9)));
    assert_eq!(marker.tint, Some([1.0; 4]));
    assert_eq!(marker.progress, None);
    assert_eq!(marker.frame, None);
    assert_eq!(
        unseen.limits,
        BTreeMap::from([(
            Kept::Undecided.limitation(LAND_NO_HISTORY),
            BTreeSet::from([62])
        )])
    );
}

#[test]
fn naval_progress_turns_by_the_first_attacker_the_walk_decides() {
    use replay_data::{rows::FactionMemberRow, rows::WarRelationRow};
    use viewer_foundation::perspective::{Choice, Country};
    let links = BTreeMap::from([(100, 0), (101, 1), (102, 2), (103, 3)]);
    let tags = BTreeMap::from([
        (0, "GER".to_owned()),
        (1, "SOV".into()),
        (2, "ITA".into()),
        (3, "ROM".into()),
    ]);
    let owner = |raw: u32| links.get(&raw).copied();
    let read = |factions: TableView| {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 5,
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "not captured".into(),
                }
                .into()
            }),
            metrics: Default::default(),
        };
        *frame.tables[TableId::FactionMembers.index()] = factions;
        *frame.tables[TableId::WarRelations.index()] =
            TableView::Complete(TableRows::WarRelations(vec![WarRelationRow {
                first_country: 100,
                second_country: 101,
                ..Default::default()
            }]));
        *frame.tables[TableId::Subjects.index()] =
            TableView::Complete(TableRows::empty(TableId::Subjects));
        let choice = Choice {
            country: Country::Tag("GER".into()),
            fog: true,
        };
        Vision::read(&frame, &links, &tags, &choice, Err("no fog content")).unwrap()
    };
    let allies = [100, 102]
        .map(|country| FactionMemberRow {
            country,
            faction: 4,
            ordinal: 0,
        })
        .to_vec();
    let whole = read(TableView::Complete(TableRows::FactionMembers(
        allies.clone(),
    )));
    let partial = read(TableView::Truncated {
        rows: TableRows::FactionMembers(allies),
        missing: 1,
    });
    let turned = |vision: &Vision, attackers: Vec<u32>| {
        vision
            .turns_naval_progress(attackers.into_iter().map(owner))
            .map_err(|cause| cause.describe())
    };
    let decided = Ok;
    assert_eq!(turned(&whole, vec![101, 102]), decided(false));
    assert_eq!(turned(&whole, vec![102, 101]), decided(true));
    assert_eq!(turned(&whole, vec![103, 102]), decided(true));
    assert_eq!(turned(&whole, vec![103]), decided(false));
    assert_eq!(turned(&partial, vec![100, 101]), decided(true));
    assert_eq!(
        turned(&partial, vec![101, 100]),
        Err("the viewer's relations are not whole this hour (FactionMembers)".to_owned())
    );
}

#[test]
fn undecided_land_markers_reach_the_diagnostics_and_survive_assign() {
    let mut source = CounterReport::default();
    source.combat_gaps.insert(
        ("Land", "icon omitted: engaged flag unknown (no refresh seen since the branch joined a running game or skipped hours, or an input the test reads is unrecorded)".into()),
        [7, 3, 5].into(),
    );
    source.combat_gaps.insert(
        (
            "Naval",
            "icon omitted: the content places no Standstill unit-stack anchor for sea province 9"
                .into(),
        ),
        [4].into(),
    );
    source.combat_limits.insert(
        ("Land", Provenance::Pending.limitation().unwrap().into()),
        [7, 3, 5].into(),
    );
    source
        .combat_limits
        .insert(("Naval", CARRIED.into()), [4, 8].into());
    let mut kept = CounterReport::default();
    kept.assign(&source);
    assert_eq!(
        kept.combat_gap_lines().collect::<Vec<_>>(),
        [
            "Land combat icons: 3 undecided: icon omitted: engaged flag unknown (no refresh seen since the branch joined a running game or skipped hours, or an input the test reads is unrecorded); combats 3, 5, 7".to_owned(),
            "Naval combat icons: 1 undecided: icon omitted: the content places no Standstill unit-stack anchor for sea province 9; combats 4".to_owned(),
            format!(
                "Land combats: 3 {}; combats 3, 5, 7",
                Provenance::Pending.limitation().unwrap()
            ),
            format!(
                "Naval combats: 2 {}; combats 4, 8",
                CARRIED
            ),
        ]
    );
    kept.assign(&CounterReport::default());
    assert_eq!(kept.combat_gap_lines().count(), 0);
}

#[test]
fn a_load_rebuilt_engaged_flag_is_a_named_limitation() {
    use replay_data::{rows::LandCombatRow, rows::LandCombatSideRow};
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 5,
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "not captured".into(),
            }
            .into()
        }),
        metrics: Default::default(),
    };
    *frame.tables[TableId::LandCombats.index()] = TableView::Complete(TableRows::LandCombats(
        [61, 62]
            .map(|id| LandCombatRow {
                id,
                attacker_classes: vec![0],
                attackers: vec![id],
                defenders: vec![id + 100],
                ..Default::default()
            })
            .to_vec(),
    ));
    *frame.tables[TableId::LandCombatSides.index()] =
        TableView::Complete(TableRows::LandCombatSides(
            [61, 62]
                .map(|combat| LandCombatSideRow {
                    combat,
                    attacker_primary: vec![1],
                    defender_primary: vec![5],
                    ..Default::default()
                })
                .to_vec(),
        ));
    let links = BTreeMap::from([(1, 1), (5, 5)]);
    let relations = Relations::factions_for_tests(&[]);
    let engaged = combats(&frame, LAND, 1, &links, Ok(&relations), None).unwrap();
    let markers = LandMarkers::read(
        &frame,
        &Vision::sharing(1, []),
        &links,
        &engaged,
        &BTreeMap::new(),
        &Movements::default(),
        &LandCombatDefines::VANILLA,
    )
    .unwrap();
    assert_eq!(
        markers.limits,
        BTreeMap::from([(
            Provenance::Unindexed.limitation().unwrap().to_owned(),
            BTreeSet::from([61, 62])
        )])
    );
    let indexed = (engaged.0, Provenance::Indexed);
    let markers = LandMarkers::read(
        &frame,
        &Vision::sharing(1, []),
        &links,
        &indexed,
        &BTreeMap::new(),
        &Movements::default(),
        &LandCombatDefines::VANILLA,
    )
    .unwrap();
    assert!(markers.limits.is_empty());
}

#[test]
fn counter_parts_never_take_unknown_vision_sharing_as_no() {
    use replay_data::rows::FactionMemberRow;
    use viewer_foundation::perspective::{Choice, Country};
    let links = BTreeMap::from([(100, 0), (101, 1), (102, 2)]);
    let tags = BTreeMap::from([(0, "GER".to_owned()), (1, "SOV".into()), (2, "ITA".into())]);
    let allies = [100, 102].map(|country| FactionMemberRow {
        country,
        faction: 4,
        ordinal: 0,
    });
    let read = |factions: TableView, fog: bool| {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 5,
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "not captured".into(),
                }
                .into()
            }),
            metrics: Default::default(),
        };
        *frame.tables[TableId::FactionMembers.index()] = factions;
        for table in [TableId::WarRelations, TableId::Subjects] {
            *frame.tables[table.index()] = TableView::Complete(TableRows::empty(table));
        }
        let choice = Choice {
            country: Country::Tag("GER".into()),
            fog,
        };
        Vision::read(&frame, &links, &tags, &choice, Err("no fog content")).unwrap()
    };
    let missing = SharingUnknown::Relations("FactionMembers");
    let countries = countries([
        (0, Allegiance::Ours),
        (1, Allegiance::Other),
        (2, Allegiance::Other),
    ]);
    let templates = TemplateIcons {
        by_template: [(
            10,
            TemplateIcon {
                name: "infantry".into(),
                custom_icon: 3,
                custom_icon_nato: 0,
            },
        )]
        .into(),
        grouping: [(10, (b"infantry".to_vec(), -1, 0))].into(),
        ..Default::default()
    };
    let glyph_disclosure = |unit: &LandUnit<'_>, templates: &TemplateIcons| {
        glyph_disclosure(unit, templates, false, &|_| true)
    };
    let rep = land_unit(1, 1, 10, 10, false);
    for fog in [true, false] {
        let whole = read(
            TableView::Complete(TableRows::FactionMembers(allies.to_vec())),
            fog,
        );
        let lacking = [
            TableView::Unavailable {
                reason: "faction read failed".into(),
            },
            TableView::Truncated {
                rows: TableRows::FactionMembers(allies.to_vec()),
                missing: 1,
            },
        ]
        .map(|factions| read(factions, fog));
        let mut ita = land_unit(2, 2, 10, 10, false);
        ita.shared_vision = whole.shares_vision_proven(2);
        assert_eq!(
            ita.can_join(&rep, &templates, &countries, rules(false)),
            Ok(false)
        );
        assert_eq!(glyph_disclosure(&ita, &templates), Ok(true));
        for vision in &lacking {
            let mut ita = land_unit(2, 2, 10, 10, false);
            ita.shared_vision = vision.shares_vision_proven(2);
            assert_eq!(ita.shared_vision, Err(missing));
            assert_eq!(
                ita.can_join(&rep, &templates, &countries, rules(false)),
                Err(missing)
            );
            let mut undecided = BTreeMap::new();
            let fitted = land_fit(
                [&rep, &ita],
                &templates,
                &countries,
                rules(false),
                &mut undecided,
            );
            assert_eq!(fitted.len(), 2, "an undecided join is not taken");
            assert_eq!(undecided, BTreeMap::from([(2, missing)]));
            assert_eq!(glyph_disclosure(&ita, &templates), Err(missing));
            let mut own = land_unit(3, 0, 10, 10, false);
            own.shared_vision = vision.shares_vision_proven(0);
            assert_eq!(glyph_disclosure(&own, &templates), Ok(true));
        }
    }
}

#[test]
fn a_defending_side_needs_whole_relations_to_prove_a_negative() {
    use replay_data::{
        rows::DivisionRow, rows::FactionMemberRow, rows::LandCombatRow, rows::LandCombatSideRow,
    };
    use viewer_foundation::perspective::{Choice, Country};
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 5,
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "not captured".into(),
            }
            .into()
        }),
        metrics: Default::default(),
    };
    let mut set = |table: TableId, view: TableView| *frame.tables[table.index()] = view;
    set(
        TableId::LandCombats,
        TableView::Complete(TableRows::LandCombats(vec![LandCombatRow {
            id: 62,
            attacker_classes: vec![0],
            attackers: vec![10],
            defenders: vec![20],
            ..Default::default()
        }])),
    );
    set(
        TableId::Divisions,
        TableView::Complete(TableRows::Divisions(
            [(10, 1), (20, 3)]
                .map(|(id, organisation)| DivisionRow {
                    id,
                    organisation,
                    ..Default::default()
                })
                .to_vec(),
        )),
    );
    set(
        TableId::DivisionMovement,
        TableView::Complete(TableRows::DivisionMovement(vec![
            replay_data::rows::DivisionMovementRow {
                division: 10,
                ..Default::default()
            },
        ])),
    );
    for table in [TableId::WarRelations, TableId::Subjects] {
        set(table, TableView::Complete(TableRows::empty(table)));
    }
    let allies = [1, 2].map(|country| FactionMemberRow {
        country,
        faction: 9,
        ordinal: 0,
    });
    let links = BTreeMap::from([(1, 1), (2, 2), (3, 3)]);
    let tags = BTreeMap::from([(1, "AAA".to_owned()), (2, "BBB".into()), (3, "CCC".into())]);
    let engaged = (BTreeMap::from([(62, Engaged::Yes)]), Provenance::Indexed);
    let read = |frame: &DecodedFrame, fog: bool| {
        let choice = Choice {
            country: Country::Tag("AAA".into()),
            fog,
        };
        let vision = Vision::read(frame, &links, &tags, &choice, Err("no fog content")).unwrap();
        LandMarkers::read(
            frame,
            &vision,
            &links,
            &engaged,
            &BTreeMap::new(),
            &Movements::default(),
            &LandCombatDefines::VANILLA,
        )
        .unwrap()
    };
    let with_defenders = |frame: &mut DecodedFrame, primary: Vec<u32>, secondary: Vec<u32>| {
        *frame.tables[TableId::LandCombatSides.index()] =
            TableView::Complete(TableRows::LandCombatSides(vec![LandCombatSideRow {
                combat: 62,
                attacker_primary: vec![3],
                defender_primary: primary,
                defender_secondary: secondary,
                ..Default::default()
            }]));
    };
    with_defenders(&mut frame, vec![2], vec![]);
    for fog in [true, false] {
        *frame.tables[TableId::FactionMembers.index()] =
            TableView::Complete(TableRows::FactionMembers(allies.to_vec()));
        let whole = read(&frame, fog);
        assert_eq!(whole.markers[0].progress, Some(7_500_000), "fog {fog}");
        assert!(whole.limits.is_empty(), "fog {fog}");
        for missing in [
            TableView::Unavailable {
                reason: "faction read failed".into(),
            },
            TableView::Truncated {
                rows: TableRows::FactionMembers(allies.to_vec()),
                missing: 1,
            },
        ] {
            *frame.tables[TableId::FactionMembers.index()] = missing;
            let unknown = read(&frame, fog);
            assert!(unknown.markers.is_empty(), "fog {fog}");
            assert_eq!(
                unknown.gaps,
                BTreeMap::from([(
                    format!(
                        "icon omitted: {}",
                        undefended(SharingUnknown::Relations("FactionMembers"),)
                    ),
                    BTreeSet::from([62])
                )]),
                "fog {fog}"
            );
        }
    }
    with_defenders(&mut frame, vec![2], vec![1]);
    for fog in [true, false] {
        assert_eq!(
            read(&frame, fog).markers[0].progress,
            Some(7_500_000),
            "fog {fog}"
        );
    }
}

#[test]
fn a_seen_combat_takes_the_winning_losing_or_balanced_colour_from_its_side() {
    let colors = LandCombatDefines::VANILLA.colors;
    assert_eq!(combat_tint([111_000, 100_000], false, &colors), colors[0]);
    assert_eq!(combat_tint([111_000, 100_000], true, &colors), colors[1]);
    assert_eq!(combat_tint([110_000, 100_000], false, &colors), colors[2]);
}

#[test]
fn hq_portraits_require_runtime_leader_data_even_with_an_authored_character() {
    use replay_data::{
        rows::CharacterPortrait, rows::CharacterPortraitRow, rows::CharacterRow,
        rows::LeaderPortraitRow, rows::OrdersGroupRow, rows::UnitLeaderRow,
    };
    const LEADER: u64 = (5 << 32) | 11;
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 10,
        tables: std::array::from_fn(|_| TableView::Unavailable {
            reason: "not captured".into(),
        })
        .map(Into::into),
        metrics: Metrics::default(),
    };
    *frame.tables[TableId::OrdersGroups.index()] =
        TableView::Complete(TableRows::OrdersGroups(vec![OrdersGroupRow {
            handle: 7,
            leader: LEADER,
            hq_provider: 99,
            ..Default::default()
        }]));
    *frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(vec![UnitLeaderRow {
            id: 5,
            reference_type: 11,
            character_id: 9,
            category: 0,
            ..Default::default()
        }]));
    *frame.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(vec![CharacterRow {
            id: 9,
            token: b"GER_authored_character".to_vec(),
            ..Default::default()
        }]));
    *frame.tables[TableId::CharacterPortraits.index()] =
        TableView::Complete(TableRows::CharacterPortraits(vec![CharacterPortraitRow {
            character: 9,
            portraits: vec![CharacterPortrait {
                category: 1,
                size: counter_order::HQ_PORTRAIT_SIZE,
                is_set: 1,
                kind: 0,
                identity: b"GFX_runtime_override".to_vec(),
            }],
        }]));
    let leader = LeaderPortraitRow {
        leader: LEADER,
        character: (9 << 32) | 73,
        ..Default::default()
    };
    let native = NativeInputs::default();
    for missing in [
        TableView::Unavailable {
            reason: "leader read failed".into(),
        },
        TableView::Truncated {
            rows: TableRows::LeaderPortraits(vec![leader.clone()]),
            missing: 1,
        },
    ] {
        *frame.tables[TableId::LeaderPortraits.index()] = missing;
        assert!(hq_portraits(&frame, &native).unwrap().is_empty());
    }

    *frame.tables[TableId::LeaderPortraits.index()] =
        TableView::Complete(TableRows::LeaderPortraits(vec![leader.clone()]));
    assert_eq!(
        hq_portraits(&frame, &native).unwrap().get(&99),
        Some(&counter_order::Portrait::Sprite(
            b"GFX_runtime_override".to_vec()
        ))
    );

    *frame.tables[TableId::CharacterPortraits.index()] = TableView::Unavailable {
        reason: "character holder read failed".into(),
    };
    assert_eq!(
        hq_portraits(&frame, &native).unwrap().get(&99),
        Some(&counter_order::Portrait::Unrecorded("character census"))
    );
    *frame.tables[TableId::LeaderPortraits.index()] =
        TableView::Complete(TableRows::LeaderPortraits(vec![LeaderPortraitRow {
            character: 0,
            portrait_c0: b"gfx/runtime_leader.dds".to_vec(),
            ..leader
        }]));
    assert_eq!(
        hq_portraits(&frame, &native).unwrap().get(&99),
        Some(&counter_order::Portrait::FallbackIfUnresolved(Box::new(
            counter_order::Portrait::File(b"gfx/runtime_leader.dds".to_vec())
        )))
    );
}

#[test]
fn a_combat_icon_sits_between_both_anchors_across_the_seam() {
    const W: f32 = 5632.0;
    let (midpoint, angle) = combat_midpoint(W, [3104.32, 9.7, 1606.0], [3086.12, 10.6, 1610.0]);
    assert!((midpoint[0] - 3095.22).abs() < 1e-3);
    assert!((midpoint[2] - 1608.0).abs() < 1e-3);
    assert!((angle + 1.787).abs() < 1e-3, "{angle}");
    let (midpoint, _) = combat_midpoint(W, [10.0, 0.0, 0.0], [W - 10.0, 0.0, 0.0]);
    assert!(midpoint[0].abs() < 1e-3);
    let (same, angle) = combat_midpoint(W, [5.0, 1.0, 2.0], [5.0, 1.0, 2.0]);
    assert_eq!(same, [5.0, 1.0, 2.0]);
    assert!((angle.abs() - std::f32::consts::PI).abs() < 1e-6);
}

#[test]
fn counter_cutoffs_preserve_visibility_and_limit_zoom_lift() {
    let layouts = layouts();
    let reference = 310.178;
    let defines = CounterDefines::VANILLA;
    assert!(visible(reference, defines.icons_cutoff));
    assert!(visible(reference, defines.air_base_cutoff));
    assert!(visible(reference, defines.naval_base_cutoff));
    assert!(visible(reference, defines.land_combat_cutoff));
    assert!(!visible(1000.0, defines.icons_cutoff));
    assert!(visible(1000.0, defines.naval_combat_cutoff));
    assert!(!visible(1600.0, defines.land_combat_cutoff));
    assert!(!visible(defines.icons_cutoff, defines.icons_cutoff));
    let units = defines.units_cutoff;
    assert_eq!(
        zoom_lift(layouts.columns.as_ref().unwrap(), units, reference),
        0.0
    );
    assert_eq!(
        zoom_lift(layouts.columns.as_ref().unwrap(), units, units),
        0.0
    );
    assert_eq!(
        zoom_lift(layouts.columns.as_ref().unwrap(), units, 60.0),
        24.0
    );
}

#[test]
fn counter_defines_read_each_define_through_its_native_getter() {
    let defines = content_reader_defines::defines::Defines::new().unwrap();
    let others = PriorityTable::ALL[1..]
        .iter()
        .enumerate()
        .flat_map(|(table, prefix)| {
            PriorityIcon::ALL
                .iter()
                .enumerate()
                .map(move |(icon, suffix)| {
                    format!(
                        "{}_PRIORITY_{} = {},
",
                        prefix.prefix(),
                        suffix.suffix(),
                        40 + 12 * table + icon
                    )
                })
        })
        .collect::<String>();
    let text = r"NDefines = {
            NGraphics = {
                UNITS_ICONS_DISTANCE_CUTOFF = 12.345674,
                UNITS_DISTANCE_CUTOFF = 120,
                LAND_UNIT_MOVEMENT_SPEED = 12,
                NAVAL_UNIT_MOVEMENT_SPEED = 6.5,
                AIRBASE_ICON_DISTANCE_CUTOFF = 900,
                NAVALBASE_ICON_DISTANCE_CUTOFF = 900,
                NAVAL_MISSION_ICONS_DISTANCE_CUTOFF = 1500,
                FACILITY_DISTANCE_CUTOFF = 900,
                LAND_COMBAT_DISTANCE_CUTOFF = 1500,
                NAVAL_COMBAT_DISTANCE_CUTOFF = 1500,
                RADAR_ICON_DISTANCE_CUTOFF = 1100,
                MAP_ICONS_COARSE_COUNTRY_GROUPING_DISTANCE = 500.9,
                MAP_ICONS_COARSE_COUNTRY_GROUPING_DISTANCE_STRATEGIC = 350.4,
                MAP_ICONS_GROUP_CAM_DISTANCE = 90,
                MAP_ICONS_STATE_GROUP_CAM_DISTANCE = 180,
                MAP_ICONS_STRATEGIC_GROUP_CAM_DISTANCE = 350,
                MAP_ICONS_STATE_HUGE = 100,
                MAP_ICONS_STRATEGIC_AREA_HUGE = 220,
                MAPICON_GROUP_PASSES = 20,
                SUPPLY_UNIT_COUNTER_VERY_LOW_THRESHOLD = 0.2,
                SUPPLY_UNIT_COUNTER_LOW_THRESHOLD = 0.35,
                SUPPLY_UNIT_COUNTER_SHOW_THRESHOLD = 0.5,
                COUNTER_MODE_ALLEGIANCE_OURS = { 1, 1, 1, 1 },
                COUNTER_MODE_ALLEGIANCE_ALLIED = { 1, 1, 1, 1 },
                COUNTER_MODE_ALLEGIANCE_ENEMY = { 1, 1, 1, 1 },
                COUNTER_MODE_ALLEGIANCE_OTHER = { 1, 1, 1, 1 },
                FRIEND_COLOR = { 0.2, 0.8, 0.3 },
                ENEMY_COLOR = { 0.9, 0.1, 0.4 },
                NEUTRAL_COLOR = { 0.8, 0.6, 0.4 },
                LAND_COMBAT_WINNING_COLOR = { 0, 1, 0, 1 },
                LAND_COMBAT_LOSING_COLOR = { 1, 0, 0, 1 },
                LAND_COMBAT_BALANCED_COLOR = { 1, 1, 0, 1 },
            },
            NMilitary = {
                BASE_CHANCE_TO_AVOID_HIT = 90,
                CHANCE_TO_AVOID_HIT_AT_NO_DEF = 60,
                LAND_COMBAT_ORG_DICE_SIZE = 4,
            },
            NMapIcons = {
                DEFAULT_PRIORITY_UNITS_STACK = 10.7,
                DEFAULT_PRIORITY_UNITS_STACK_GROUP = 267,
                DEFAULT_PRIORITY_AIR_BASE = -3.2,
                DEFAULT_PRIORITY_NAVAL_BASE = 3,
                DEFAULT_PRIORITY_NAVAL_HEADQUARTER = 4,
                DEFAULT_PRIORITY_FACILITY = 5,
                DEFAULT_PRIORITY_LAND_COMBAT = 20,
                DEFAULT_PRIORITY_NAVAL_COMBAT = 532,
                DEFAULT_PRIORITY_NAVAL_COMBAT_RESULTS = 19,
                DEFAULT_PRIORITY_NAVAL_MISSION = 13,
                DEFAULT_PRIORITY_AIR_MISSION = 14.8,
                DEFAULT_PRIORITY_VICTORY_POINTS = 5,
                DEFAULT_PRIORITY_CAPITAL = 6,
                DEFAULT_PRIORITY_ROCKET_SITE = 7,
                DEFAULT_PRIORITY_GUN_EMPLACEMENT = 8,
                DEFAULT_PRIORITY_STRATEGIC_LOCATION = 9,
                DEFAULT_PRIORITY_RESOURCE = 10,
                DEFAULT_PRIORITY_ADJACENCY_RULE = 11,
                OTHER_TABLES
                HOME_BASE_MAP_ICON_SCALE_DEFAULT = 1.25,
                HOME_BASE_MAP_ICON_SCALE_HIGHLIGHTED = 1.5,
            },
            NNavy = {
                INTEL_LEVEL_LOW_HALF_RANGE_PERCENTAGE = 10,
                INTEL_LEVEL_LOW_HALF_RANGE_MIN_SHIPS = 3,
                INTEL_LEVEL_LOW_HALF_RANGE_MIN_CAPITALS = 1,
                INTEL_LEVEL_MEDIUM_HALF_RANGE_PERCENTAGE = 5,
                INTEL_LEVEL_MEDIUM_HALF_RANGE_MIN_SHIPS = 1,
                INTEL_LEVEL_MEDIUM_HALF_RANGE_MIN_CAPITALS = 1,
            },
        }"
    .replace("OTHER_TABLES", &others);
    defines.load("mod.lua", text.as_bytes()).unwrap();
    let read = CounterDefines::read(&defines).unwrap();
    assert_eq!(read.type_colors, type_tint::Colors::TEST);
    assert_eq!(read.icons_cutoff, 12.34567);
    assert_eq!(read.movement_speeds, [12.0, 6.5]);
    assert_eq!(read.coarse_distance, [500.0, 350.0]);
    assert!(!read.coarse(500.0, false));
    assert!(read.coarse(500.01, false));
    assert!(!read.coarse(400.0, false));
    assert!(read.coarse(400.0, true));
    assert!(!read.coarse(350.0, true));
    let default = |icon| read.priorities.get(PriorityTable::Default, icon);
    assert_eq!(default(PriorityIcon::UnitsStack), 10);
    assert_eq!(default(PriorityIcon::UnitsStackGroup), 11);
    assert_eq!(default(PriorityIcon::AirBase), -3);
    assert_eq!(default(PriorityIcon::AirMission), 14);
    assert_eq!(
        (
            default(PriorityIcon::NavalHeadquarter),
            default(PriorityIcon::Facility)
        ),
        (4, 5)
    );
    assert_eq!(read.naval_base_scale, 1.25);
    assert_eq!(read.naval_base_highlighted_scale, 1.5);
    assert_eq!(default(PriorityIcon::NavalCombatResults), 19);
    assert_eq!(default(PriorityIcon::NavalCombat), 20);
    assert_eq!(
        (
            default(PriorityIcon::RocketSite),
            default(PriorityIcon::GunEmplacement),
            default(PriorityIcon::StrategicLocation)
        ),
        (7, 8, 9)
    );
    assert_eq!(default(PriorityIcon::AdjacencyRule), 11);
    assert_eq!(default(PriorityIcon::Resource), 10);
    assert_eq!(
        read.priorities
            .get(PriorityTable::StrategicNavy, PriorityIcon::NavalBase),
        (40 + 12 + 4) as i8
    );
    assert_eq!(
        read.priorities
            .get(PriorityTable::Ideology, PriorityIcon::Capital),
        (40 + 12 * 11 + 12) as i8
    );
    assert_eq!(
        read.priorities
            .get(PriorityTable::StrategicAir, PriorityIcon::AirMission),
        51
    );
    assert_eq!(read.stack_priority(89.9, PriorityTable::Default), 10);
    assert_eq!(read.stack_priority(90.0, PriorityTable::Default), 11);
    assert_eq!(read.stack_priority(89.9, PriorityTable::StrategicAir), 40);
}

#[test]
fn children_take_their_declaration_order_and_their_font() {
    content_reader_script::test_game::enter();
    let document = layout_gui(
        br#"guiTypes = { containerWindowType = { name = "unit_counter"
            buttonType = { name = "count_bg" quadTextureSprite = "GFX_a" position = { x = 0 y = 0 } }
            iconType = { name = "flag" spriteType = "GFX_b" position = { x = 1 y = 1 } }
            instantTextboxType = { name = "count_txt" font = "modded_12" maxWidth = 20
                position = { x = 2 y = 2 } }
        } }"#,
    );
    let window = window(&document, LAND_WINDOW).unwrap();
    let mut missing = Vec::new();
    let plate = child(
        window,
        native::Kind::Button,
        "count_bg",
        LAND_WINDOW,
        &mut missing,
    )
    .unwrap();
    let flag = child(
        window,
        native::Kind::Icon,
        "flag",
        LAND_WINDOW,
        &mut missing,
    )
    .unwrap();
    let count = text_child(window, "count_txt", LAND_WINDOW, &mut missing).unwrap();
    assert!(plate.order < flag.order && flag.order < count.order);
    assert_eq!(count.font.as_deref(), Some("modded_12"));
    let absent = child(
        window,
        native::Kind::Icon,
        "hq_badge",
        LAND_WINDOW,
        &mut missing,
    )
    .unwrap();
    assert_eq!(absent.order, u32::MAX);
    assert_eq!(missing, vec![format!("{LAND_WINDOW}.hq_badge")]);
}

#[test]
fn counter_button_sprite_aliases_and_frames_match_native_button_drawing() {
    content_reader_script::test_game::enter();
    use viewer_native_gui::native_gui::{PartKind, lifecycle::Instance};
    let plate_sheet = sheet("GFX_plate", 3, [60, 10]);
    let resources = Fixture::new(&[("GFX_plate", 3, [60, 10])], &[]);
    for alias in ["quadTextureSprite", "spriteType"] {
        for (frame, index) in [("", 0), ("frame=-2", 0), ("frame=2", 1)] {
            let guis = layout_gui(
                format!(
                    "guiTypes={{containerWindowType={{name=unit_counter size={{width=80 height=30}}
                    buttonType={{name=count_bg {alias}=GFX_plate position={{x=2 y=3}}
                        orientation=UPPER_RIGHT scale=0.5 {frame}}}}}}}"
                )
                .as_bytes(),
            );
            let root = window(&guis, LAND_WINDOW).unwrap();
            let plate = child(
                root,
                native::Kind::Button,
                "count_bg",
                LAND_WINDOW,
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(plate.frame_index(0), index);
            if frame.is_empty() {
                assert_eq!(plate.frame_index(2), 2, "{alias}: absent frame keeps state");
            }
            let mut batch = Batch::default();
            draw_child(&mut batch, &plate_sheet, &plate, [0.0; 2], index, [1.0; 4]);
            let mut instance = Instance::construct(root, [800, 600], &resources).unwrap();
            instance.gui_show(root, root).unwrap();
            let native = instance.layout(root, &resources, None).unwrap().unwrap();
            let PartKind::Sprite {
                geometry,
                uv,
                offsets,
                ..
            } = &native.parts[0].kind
            else {
                panic!("button draws no sprite");
            };
            assert_eq!(batch.quads[0].rect, geometry.rect, "{alias} {frame}");
            let native_uv = [
                uv[0] + offsets[0],
                uv[1] + offsets[1],
                uv[2] + offsets[0],
                uv[3] + offsets[1],
            ];
            assert_eq!(batch.quads[0].uv, native_uv, "{alias} {frame}");
        }
    }
}

#[test]
fn land_plates_take_the_counter_colour_mode_and_their_gaps_reach_the_warnings() {
    let tints = allegiance::Tints::VANILLA;
    let countries = Countries {
        ui_colors: UiColors {
            colors: BTreeMap::from([(3, Some([0.5, 0.25, 0.125, 1.0])), (6, None)]),
            inheritance: viewer_foundation::color_inheritance::Inheritance {
                inherited: BTreeMap::from([(4, 3)]),
                ..Default::default()
            },
            gap: "CountryAppearance has no row for it".into(),
            unavailable: None,
        },
        ..Countries::default()
    };
    let color =
        |mode, logical| counter_plate_color(mode, &tints, &countries, Allegiance::Enemy, logical);
    let (allegiance, country) = (CounterColorMode::Allegiance, CounterColorMode::Country);
    assert_eq!(color(allegiance, Some(3)), Ok(tints.enemy));
    assert_eq!(color(allegiance, None), Ok(tints.enemy));
    assert_eq!(color(country, Some(3)), Ok([0.5, 0.25, 0.125, 1.0]));
    assert_eq!(color(country, Some(4)), Ok([0.5, 0.25, 0.125, 1.0]));
    assert!(color(country, None).unwrap_err().contains("logical owner"));
    assert!(color(country, Some(9)).is_err());
    let non_finite = color(country, Some(6)).unwrap_err();
    assert!(non_finite.contains("non-finite"), "{non_finite}");
    assert!(non_finite.contains("slot 6"), "{non_finite}");
    let undecided = Countries {
        ui_colors: UiColors {
            colors: BTreeMap::from([
                (3, Some([0.5, 0.25, 0.125, 1.0])),
                (5, Some([1.0; 4])),
            ]),
            inheritance: viewer_foundation::color_inheritance::Inheritance {
                undecided: BTreeMap::from([(5, "Subjects is truncated".to_owned())]),
                ..Default::default()
            },
            gap: String::new(),
            unavailable: None,
        },
        ..Countries::default()
    };
    let why =
        counter_plate_color(country, &tints, &undecided, Allegiance::Enemy, Some(5)).unwrap_err();
    assert!(why.contains("Subjects is truncated"), "{why}");
    let report = CounterReport {
        land_counter_gaps: BTreeMap::from([("no colour".into(), 2)]),
        ..Default::default()
    };
    assert!(
        report
            .warnings()
            .any(|line| line == "Land counters: 2 not drawn: no colour.")
    );
    let mut kept = CounterReport::default();
    kept.assign(&report);
    assert_eq!(kept.land_counter_gaps, report.land_counter_gaps);
}

#[test]
fn active_ability_center_position_is_a_native_boolean() {
    content_reader_script::test_game::enter();
    for (fields, expected) in [
        ("", false),
        ("centerPosition = YES", false),
        ("centerPosition = \"yes\"", true),
        ("centerPosition = yes centerPosition = YES", true),
        ("centerPosition = yes centerPosition = no", false),
        ("centerPosition = no centerPosition = Yes", false),
    ] {
        let source = format!(
            r#"guiTypes = {{
            containerWindowType = {{ name = "unit_counter"
                gridBoxType = {{ name = "active_abilities" slotsize = {{ width = 25 height = 25 }}
                    position = {{ x = 0 y = 0 }}
                    max_slots_vertical = 1 format = "UPPER_LEFT" }} }}
            containerWindowType = {{ name = "active_ability_item" size = {{ width = 20 height = 20 }}
                iconType = {{ name = "active_ability" orientation = center {fields} }} }}
        }}"#
        );
        let document = layout_gui(source.as_bytes());
        let land = window(&document, LAND_WINDOW).unwrap();
        let grid = ability_grid(&document, land, &mut Vec::new(), &mut Vec::new())
            .unwrap()
            .unwrap();
        assert_eq!(grid.centred, expected, "{fields}");
    }
}

#[test]
fn the_gun_window_and_ability_grid_are_read_or_refused_by_name() {
    content_reader_script::test_game::enter();
    let document = layout_gui(
        br#"guiTypes = {
            containerWindowType = { name = "unit_counter"
                buttonType = { name = "count_bg" quadTextureSprite = "GFX_a" position = { x = 0 y = 0 } }
                gridBoxType = { name = "active_abilities" position = { x = 55 y = -9 }
                    slotsize = { width = 25 height = 25 } max_slots_vertical = 1
                    format = "UPPER_LEFT" } }
            containerWindowType = { name = "active_ability_item" size = { width = 20 height = 20 }
                iconType = { name = "active_ability" spriteType = "GFX_b" orientation = center
                    scale = 0.8 } }
            containerWindowType = { name = "unit_counter_railway_gun" size = { width = 58 height = 24 }
                iconType = { name = "onmap_action_icon" spriteType = "GFX_c" position = { x = 54 y = 7 } } }
        }"#,
    );
    let land = window(&document, LAND_WINDOW).unwrap();
    let (mut missing, mut unsupported) = (Vec::new(), Vec::new());
    let grid = ability_grid(&document, land, &mut missing, &mut unsupported)
        .unwrap()
        .unwrap();
    assert_eq!(
        (grid.at, grid.slot, grid.item_size, grid.scale, grid.order),
        ([55.0, -9.0], [25.0, 25.0], [20.0, 20.0], 0.8, 1)
    );
    assert_eq!((grid.from_centre, grid.centred), (true, false));
    let gun = gun_counter(&document, &mut missing).unwrap().unwrap();
    assert_eq!((gun.size, gun.action.at), ([58.0, 24.0], [54.0, 7.0]));
    assert!(missing.contains(&format!("{GUN_WINDOW}.bar_str")));
    assert!(unsupported.is_empty());

    for (rows, orientation) in [(2, "center"), (1, "LOWER_RIGHT")] {
        let source = format!(
            r#"guiTypes = {{
            containerWindowType = {{ name = "unit_counter"
                gridBoxType = {{ name = "active_abilities" position = {{ x = 0 y = 0 }}
                    slotsize = {{ width = 25 height = 25 }} max_slots_vertical = {rows}
                    format = "UPPER_LEFT" }} }}
            containerWindowType = {{ name = "active_ability_item" size = {{ width = 20 height = 20 }}
                iconType = {{ name = "active_ability" spriteType = "GFX_b" orientation = {orientation} }} }}
        }}"#
        );
        let document = layout_gui(source.as_bytes());
        let land = window(&document, LAND_WINDOW).unwrap();
        let (mut missing, mut unsupported) = (Vec::new(), Vec::new());
        assert!(
            ability_grid(&document, land, &mut missing, &mut unsupported)
                .unwrap()
                .is_none(),
            "{rows} rows, {orientation}"
        );
        assert_eq!(unsupported.len(), 1, "{rows} rows, {orientation}");
        assert!(gun_counter(&document, &mut missing).unwrap().is_none());
        assert_eq!(missing, vec![GUN_WINDOW.to_owned()]);
    }
}

#[test]
fn the_layouts_name_every_sprite_the_pass_has_to_load_once_each() {
    content_reader_script::test_game::enter();
    let layouts = layouts();
    let names = layouts.sprite_names();
    assert!(names.contains(&"GFX_onmap_action_icon"));
    assert!(names.contains(&"GFX_onmap_unit_counter"));
    assert!(!names.contains(&"GFX_onmap_naval_unit_counter"));
    let walked = NativeWindows::read(
        format!(
            "{SYNTHETIC_NAVY_GUI}
{SYNTHETIC_NAVAL_GUI}"
        )
        .as_bytes(),
        [1920, 1080],
    )
    .unwrap()
    .assets(&Default::default())
    .sprites;
    for name in [
        b"GFX_fixture_navy_plate".as_slice(),
        b"GFX_fixture_combat",
        b"GFX_fixture_tiny_plate",
        FALLBACK_SPRITE,
    ] {
        assert!(walked.contains(name), "{}", String::from_utf8_lossy(name));
    }
    assert!(!names.contains(&"GFX_mapicon_unit_flag_stripe"));
    assert_eq!(
        names
            .iter()
            .filter(|n| **n == "GFX_mapicon_unit_bar_org")
            .count(),
        1
    );
}

#[test]
fn a_size_block_evaluates_as_the_native_evaluator_does() {
    content_reader_script::test_game::enter();
    let evaluate = |size: &str, parent: [i32; 2], position: [i32; 2]| {
        let gui = native_gui(format!(
            r#"guiTypes = {{ containerWindowType = {{ name = "w"
                gridBoxType = {{ name = "g" size = {{ {size} }} }} }} }}"#
        ));
        let grid = gui
            .template("w")
            .unwrap()
            .find(native::Kind::Grid, "g")
            .unwrap();
        assert!(grid.refusals().is_empty(), "{:?}", grid.refusals());
        evaluate_size(&grid.grid().unwrap().size, Some(parent), position, "size").ok()
    };
    assert_eq!(
        evaluate("width = -5 height = 100%%", [33, 43], [30, 30]),
        Some([-2, 13])
    );
    assert_eq!(
        evaluate("width = -10% height = 10%", [33, 43], [0, 0]),
        Some([30, 4])
    );
    let aspect = "width = 100 height = 50 preserve_aspect_ratio = yes";
    assert_eq!(
        evaluate(&format!("{aspect} max = {{ width = 60 }}"), [0; 2], [0; 2]),
        Some([60, 30])
    );
    assert_eq!(
        evaluate(
            &format!("{aspect} min = {{ height = 50 }} max = {{ width = 60 }}"),
            [0; 2],
            [0; 2]
        ),
        Some([60, 50])
    );
    assert_eq!(
        evaluate(&format!("{aspect} min = {{ height = 80 }}"), [0; 2], [0; 2]),
        Some([160, 80])
    );
    assert_eq!(
        evaluate(&format!("{aspect} max = {{ height = 10 }}"), [0; 2], [0; 2]),
        Some([20, 10])
    );
    assert_eq!(
        evaluate(
            &format!("{aspect} min = {{ width = 100 }} max = {{ height = 10 }}"),
            [0; 2],
            [0; 2]
        ),
        Some([100, 10])
    );
    assert_eq!(
        evaluate(&format!("{aspect} min = {{ width = 200 }}"), [0; 2], [0; 2]),
        Some([200, 100])
    );
    assert_eq!(evaluate(aspect, [0; 2], [0; 2]), Some([100, 50]));
}

#[test]
fn a_mission_item_shows_its_hull_and_pride_only_from_the_first_members_150() {
    use replay_data::rows::TaskForceRow;
    let vision = Vision::default();
    let native = NativeInputs::default();
    let context = naval_context(&vision, &native, BTreeMap::new());
    let force = TaskForceRow {
        id: (7 << 32) | 1,
        roster_owner: 2,
        ..Default::default()
    };
    for (byte, detailed) in [(99, false), (100, false), (149, false), (150, true)] {
        let unit = NavalUnit {
            force: &force,
            ordinal: 0,
            slot: 2,
            logical: Some(2),
            flag_owner: 2,
            sight: Sight::Partial,
            byte,
            ships: Some(12),
            position: None,
            strength: 0,
        };
        let mut tile = context.flotilla(&[&unit], true, false);
        tile.status.pride = Some(true);
        assert_eq!(tile.no_intel_type(), !detailed, "{byte}");
        assert_eq!(tile.shows_pride(), Some(detailed), "{byte}");
        let item = tile.item.unwrap();
        let count = viewer_unit_reads::naval_count_approx::format_count(
            12,
            item.first,
            item.byte,
            false,
            12,
            &viewer_unit_reads::naval_count_approx::Defines::VANILLA,
        );
        assert_eq!(count == "?", byte < 100, "{byte}: {count}");
    }
    let unit = NavalUnit {
        force: &force,
        ordinal: 0,
        slot: 2,
        logical: Some(2),
        flag_owner: 2,
        sight: Sight::Partial,
        byte: 200,
        ships: Some(12),
        position: None,
        strength: 0,
    };
    let mut sea = context.flotilla(&[&unit], false, false);
    sea.status.pride = Some(true);
    assert!(sea.no_intel_type());
    assert_eq!(sea.shows_pride(), Some(true));
    sea.status.pride = None;
    assert_eq!(sea.shows_pride(), None);
}

fn flag_fixture<const N: usize>(names: [&str; N]) -> Arc<LazyFlags> {
    let textures = LazyTextures::new();
    textures.set_background(false);
    let index = names
        .map(|key| (key.to_owned(), format!("gfx/flags/small/{key}.tga")))
        .into();
    let pages = atlas_pages::CellPages::plan([10, 7], 8192, "flags").unwrap();
    let read: FlagReader = Arc::new(|_: &str| {
        Some(content_reader_image::Rgba8Image {
            width: 10,
            height: 7,
            pixels: vec![9; 280],
        })
    });
    LazyFlags::new(index, pages, read, &textures)
}

#[test]
fn a_counter_flag_takes_its_group_flag_and_falls_back_to_its_tag() {
    let flags = flag_fixture(["GER_fascism", "ROM"]);
    let countries = Countries {
        tags: BTreeMap::from([(1, "GER".into()), (2, "ROM".into()), (3, "XXX".into())]),
        flag_tags: [
            (1, (Ok(String::new()), Ok("GER".into()))),
            (2, (Ok(String::new()), Ok("ROM".into()))),
            (3, (Ok(String::new()), Ok("XXX".into()))),
        ]
        .into(),
        groups: BTreeMap::from([
            (1, Ok(Some("fascism".into()))),
            (2, Ok(Some("fascism".into()))),
            (3, Ok(None)),
        ]),
        ..Default::default()
    };
    let mut batch = Batch::default();
    draw_flag(
        &flags,
        &mut batch,
        &layouts().land.unwrap().flag,
        [0.0; 2],
        &countries,
        1,
    );
    assert_eq!(batch.quads.len(), 1);
    let (_, germany) = countries.flag_now(&flags, 1).unwrap();
    let (_, romania) = countries.flag_now(&flags, 2).unwrap();
    assert_ne!(germany, romania);
    assert_eq!(countries.flag_now(&flags, 3), None);
    assert_eq!(flags.len(), 2);
}

#[test]
fn window_flags_resolve_recorded_tags_through_the_small_atlas() {
    use viewer_native_gui::native_flags;
    let flags = flag_fixture(["GER"]);
    let countries = Countries {
        tags: BTreeMap::from([(1, "GER".into()), (2, "XXX".into())]),
        flag_tags: [
            (1, (Ok(String::new()), Ok("GER".into()))),
            (2, (Ok(String::new()), Ok("XXX".into()))),
        ]
        .into(),
        groups: [(1, Ok(None)), (2, Ok(None))].into(),
        ..Countries::default()
    };
    let source = viewer_counter_kit::gui_windows::WindowFlags {
        countries: &countries,
        small: &flags,
        large: &flags,
    };
    let drawn = native_flags::text_flag(&source, "GER").unwrap().unwrap();
    let (page, _) = countries.flag_now(&flags, 1).unwrap();
    assert_eq!((drawn.texture, drawn.size), (page, [10, 7]));
    assert_eq!(native_flags::text_flag(&source, "XXX").unwrap(), None);
    assert_eq!(native_flags::text_flag(&source, "GE").unwrap(), None);
    assert!(
        native_flags::text_flag(&source, "ENG")
            .unwrap_err()
            .contains("ENG")
    );
    assert!(
        native_flags::shield_flag(&source, true, 1)
            .unwrap()
            .is_some()
    );
    assert!(
        native_flags::shield_flag(&source, false, 1)
            .unwrap()
            .is_some()
    );
    let mut unknown = countries.clone();
    unknown.flag_tags.get_mut(&1).unwrap().1 = Err("original_tag_reference not recorded".into());
    let source = viewer_counter_kit::gui_windows::WindowFlags {
        countries: &unknown,
        small: &flags,
        large: &flags,
    };
    assert!(
        native_flags::text_flag(&source, "GER")
            .unwrap_err()
            .contains("original_tag_reference not recorded")
    );
}

#[test]
fn a_counter_flag_follows_the_cosmetic_and_exact_original_tag() {
    let flags = flag_fixture(["GER_fascism", "GER_NEW", "D01", "Z01_communism", "Z01"]);
    let countries = Countries {
        tags: [
            (1, "D01".to_owned()),
            (2, "D02".to_owned()),
            (3, "Z01".to_owned()),
        ]
        .into(),
        flag_tags: [
            (1, (Ok(String::new()), Ok("GER".to_owned()))),
            (2, (Ok("GER_NEW".to_owned()), Ok("GER".to_owned()))),
            (3, (Ok("Z01_ssr".to_owned()), Ok("Z01".to_owned()))),
        ]
        .into(),
        groups: [
            (1, Ok(Some("fascism".to_owned()))),
            (2, Ok(Some("fascism".to_owned()))),
            (3, Ok(Some("communism".to_owned()))),
        ]
        .into(),
        ..Default::default()
    };
    assert_eq!(
        countries.flag_now(&flags, 1),
        flags.acquire(&["GER_fascism"])
    );
    assert_eq!(countries.flag_now(&flags, 2), flags.acquire(&["GER_NEW"]));
    assert_ne!(countries.flag_now(&flags, 1), flags.acquire(&["D01"]));
    assert_eq!(
        countries.flag_now(&flags, 3),
        flags.acquire(&["Z01_communism"])
    );
    assert_ne!(countries.flag_now(&flags, 3), flags.acquire(&["Z01"]));
    for reason in [
        "CountryAppearance.original_tag capture refused",
        "CountryAppearance.original_tag not recorded",
    ] {
        let mut unknown = countries.clone();
        unknown.flag_tags.get_mut(&1).unwrap().1 = Err(reason.into());
        unknown.flag_tags.get_mut(&2).unwrap().1 = Err(reason.into());
        assert_eq!(unknown.flag_now(&flags, 1), None);
        assert!(unknown.flag_refusal(1).unwrap().contains(reason));
        assert_eq!(unknown.flag_now(&flags, 2), flags.acquire(&["GER_NEW"]));
    }
    let mut unknown = countries.clone();
    unknown.flag_tags.get_mut(&1).unwrap().0 = Err("country cosmetic tag is not UTF-8".into());
    assert_eq!(unknown.flag_now(&flags, 1), None);
    assert!(
        unknown
            .flag_refusal(1)
            .unwrap()
            .contains("cosmetic tag is not UTF-8")
    );
    let mut unknown = countries.clone();
    unknown
        .groups
        .insert(2, Err("ruling group is not UTF-8".into()));
    assert_eq!(unknown.flag_now(&flags, 2), None);
    assert!(
        unknown
            .flag_refusal(2)
            .unwrap()
            .contains("ruling group is not UTF-8")
    );
    unknown.groups.insert(2, Ok(None));
    assert_eq!(unknown.flag_now(&flags, 2), flags.acquire(&["GER_NEW"]));
}

#[test]
fn an_icon_wholly_outside_the_viewport_and_its_pad_is_dropped() {
    use viewer_native_gui::{anchored::PAD, icon_windows::on_screen};
    let viewport = [2048.0, 1152.0];
    assert!(on_screen([0.0, 0.0], [76.0, 24.0], viewport));
    assert!(on_screen([-70.0, -20.0], [76.0, 24.0], viewport));
    assert!(!on_screen([-80.0, 10.0], [76.0, 24.0], viewport));
    assert!(!on_screen([10.0, -30.0], [76.0, 24.0], viewport));
    assert!(!on_screen([2049.0, 10.0], [76.0, 24.0], viewport));
    assert!(!on_screen([10.0, 1153.0], [76.0, 24.0], viewport));
    assert!(admitted([-80.0, 10.0], [76.0, 24.0], viewport));
    assert!(admitted([10.0, 1153.0], [76.0, 24.0], viewport));
    assert!(!admitted([-77.0 - PAD, 10.0], [76.0, 24.0], viewport));
    assert!(!admitted([10.0, 1153.0 + PAD], [76.0, 24.0], viewport));
}

#[test]
fn air_base_icons_hide_only_on_an_answered_no() {
    use content_reader_map::map::MapData;
    let map = MapData {
        width: 4,
        height: 3,
        province_ids: vec![0, 0, 0, 0, 0, 9, 0, 0, 0, 0, 0, 0],
        definitions: BTreeMap::new(),
        country_colors: Default::default(),
    };
    fn vision(byte: Option<u8>) -> Vision {
        const HOUR: Hour = 60_832_633;
        let mut views = std::array::from_fn(|_| TableView::Unavailable {
            reason: "not needed for the hide decision".into(),
        });
        views[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 100,
                canonical_slot: 0,
            },
            CountryLinkRow {
                id: 101,
                canonical_slot: 1,
            },
        ]));
        views[TableId::LocalPlayer.index()] =
            TableView::Complete(TableRows::LocalPlayer(vec![LocalPlayerRow {
                country: 100,
                ..Default::default()
            }]));
        views[TableId::Provinces.index()] =
            TableView::Complete(TableRows::Provinces(vec![ProvinceRow {
                id: 9,
                controller: 101,
                ..Default::default()
            }]));
        views[TableId::CountryIntel.index()] =
            TableView::Complete(TableRows::CountryIntel(Vec::new()));
        views[TableId::RelationLinks.index()] =
            TableView::Complete(TableRows::RelationLinks(Vec::new()));
        views[TableId::CountryVisibility.index()] =
            TableView::Complete(TableRows::CountryVisibility(Vec::new()));
        views[TableId::SimulationFeatures.index()] =
            TableView::Complete(TableRows::SimulationFeatures(vec![
                replay_data::rows::SimulationFeatureRow {
                    dlc_mask: 0x20,
                    human_ai: 0,
                    weather_suspend: replay_data::rows::Recorded::Value(None),
                    ..Default::default()
                },
            ]));
        views[TableId::WarRelations.index()] =
            TableView::Complete(TableRows::WarRelations(Vec::new()));
        views[TableId::FactionMembers.index()] =
            TableView::Complete(TableRows::FactionMembers(Vec::new()));
        views[TableId::Subjects.index()] = TableView::Complete(TableRows::Subjects(Vec::new()));
        views[TableId::UnitIntel.index()] = TableView::Complete(TableRows::UnitIntel(Vec::new()));
        let frame = DecodedFrame {
            branch: 1,
            hour: HOUR,
            tables: views.map(Into::into),
            metrics: Metrics::default(),
        };
        let content = FogContent {
            topology: Default::default(),
            defines: test_defines(),
        };
        let mut vision = Vision::read(
            &frame,
            &BTreeMap::from([(100, 0), (101, 1)]),
            &BTreeMap::from([(0, "GER".to_owned()), (1, "SOV".to_owned())]),
            &Choice {
                country: Country::Recorded,
                fog: true,
            },
            Ok(&content),
        )
        .unwrap();
        vision.set_visibility(byte.map(|byte| (9, byte)));
        vision
    }
    let fogged = vision(Some(49));
    assert!(air_base_hidden(&map, [1.0, 0.0, 1.0], &fogged));
    assert!(!air_base_hidden(&map, [1.0, 0.0, 1.0], &vision(Some(50))));
    assert!(
        !air_base_hidden(&map, [1.0, 0.0, 1.0], &vision(None)),
        "no byte is not a no: the icon stays"
    );
    assert!(
        !air_base_hidden(&map, [2.0, 0.0, 2.0], &fogged),
        "an unclaimed pixel keeps the icon"
    );
    assert!(
        !air_base_hidden(&map, [9.0, 0.0, 9.0], &fogged),
        "off the map keeps the icon"
    );
}

#[test]
fn province_lookup_truncates_and_rejects_what_is_not_a_province() {
    use content_reader_map::map::MapData;
    use std::collections::BTreeMap;
    let map = MapData {
        width: 4,
        height: 3,
        province_ids: vec![11, 12, 0, 14, 21, 22, 23, 24, 31, 32, 33, 34],
        definitions: BTreeMap::new(),
        country_colors: Default::default(),
    };
    assert_eq!(province_at(&map, [0.0, 0.0, 0.0]), Some(31));
    assert_eq!(province_at(&map, [3.9, 0.0, 2.9]), Some(14));
    assert_eq!(province_at(&map, [0.9, 0.0, 0.9]), Some(31));
    assert_eq!(province_at(&map, [2.0, 0.0, 2.0]), None);
    assert_eq!(province_at(&map, [-0.1, 0.0, 1.0]), None);
    assert_eq!(province_at(&map, [4.0, 0.0, 1.0]), None);
    assert_eq!(province_at(&map, [1.0, 0.0, -0.1]), None);
    assert_eq!(province_at(&map, [1.0, 0.0, 3.0]), None);
    assert_eq!(province_at(&map, [f32::NAN, 0.0, 1.0]), None);
    assert_eq!(province_at(&map, [1.0, 0.0, f32::INFINITY]), None);
}

#[test]
fn the_air_base_badge_prefers_recorded_wings_over_the_fallback_reading() {
    let count =
        |shown: bool, stationed: Option<bool>, occupied: Option<bool>, planes: i64| AirBaseCount {
            shown,
            stationed,
            occupied,
            planes,
            ..Default::default()
        };
    assert_eq!(
        air_base_frame(&count(true, Some(true), None, 0), Some(false)),
        1
    );
    assert_eq!(
        air_base_frame(&count(true, Some(false), None, 5), Some(true)),
        0
    );
    assert_eq!(
        air_base_frame(&count(false, None, Some(true), 5), Some(true)),
        1
    );
    assert_eq!(
        air_base_frame(&count(false, None, Some(true), 5), Some(false)),
        0
    );
    assert_eq!(
        air_base_frame(&count(false, None, Some(false), 0), Some(true)),
        0
    );
    assert_eq!(air_base_frame(&count(false, None, None, 5), Some(true)), 1);
    assert_eq!(air_base_frame(&count(false, None, None, 0), Some(true)), 0);
    assert_eq!(air_base_frame(&count(true, None, None, 5), None), 0);
    assert_eq!(air_base_frame(&count(false, None, Some(true), 0), None), 0);
    assert_eq!(air_base_frame(&count(true, None, None, 5), Some(true)), 0);
    assert_eq!(
        air_base_frame(&count(true, None, Some(true), 5), Some(true)),
        0
    );
    assert_eq!(air_base_frame(&count(false, None, None, 5), Some(false)), 0);
    assert_eq!(
        air_base_frame(&count(false, None, Some(true), 5), Some(false)),
        0
    );
}

#[test]
fn counter_child_hovers_follow_presented_order_and_transparent_siblings() {
    content_reader_script::test_game::enter();
    let gui = |transparent: &str, action_transparent: &str| {
        layout_gui(
            format!(
                r#"guiTypes={{
        containerWindowType={{name=unit_counter_navy}}
        containerWindowType={{name=naval_combat_mapicon}}
        containerWindowType={{name=unit_counter size={{width=81 height=29}}
            pdx_tooltip=ROOT
            buttonType={{name=count_bg spriteType=GFX_synthetic_plate pdx_tooltip=PLATE}}
            iconType={{name=onmap_action_icon spriteType=GFX_synthetic_actions
                position={{x=57 y=9}} scale=0.5 alwaystransparent={action_transparent}
                pdx_tooltip=ACTION pdx_tooltip_delayed=ACTION_BODY}}
            iconType={{name=counter_overlay spriteType=GFX_synthetic_overlay
                position={{x=57 y=9}} alwaystransparent={transparent}
                pdx_tooltip=OVERLAY pdx_tooltip_delayed=OVERLAY_BODY}}
        }}
    }}"#
            )
            .as_bytes(),
        )
    };
    let mut unit = land_unit(1, 1, 10, 10, true);
    unit.action = Ok(5);
    for (transparent, action_transparent, expected, name, kind) in [
        (
            "no",
            "no",
            CounterControl::Normal,
            "counter_overlay",
            native::Kind::Icon,
        ),
        (
            "yes",
            "no",
            CounterControl::Action,
            "onmap_action_icon",
            native::Kind::Icon,
        ),
        (
            "yes",
            "yes",
            CounterControl::Normal,
            "count_bg",
            native::Kind::Button,
        ),
    ] {
        let gui = gui(transparent, action_transparent);
        let root = gui.template(LAND_WINDOW).unwrap();
        let authored = viewer_native_gui::counter_hovers::gui_authored::Authored::read(
            root.find(kind, name).unwrap(),
            false,
        );
        let root_authored =
            viewer_native_gui::counter_hovers::gui_authored::Authored::read(root, false);
        let (_, report) = draw_land_action_fixture(gui, std::slice::from_ref(&unit), false);
        let hit = report
            .land_hovers
            .iter()
            .rev()
            .find(|hit| hit.contains([160.0, 212.0]))
            .unwrap();
        assert_eq!(hit.control, expected);
        assert_eq!(hit.authored, authored);
        assert_eq!(report.land_hovers[0].authored, root_authored);
    }
    unit.action = Ok(1);
    let (_, report) = draw_land_action_fixture(gui("yes", "no"), &[unit], false);
    assert!(
        report
            .land_hovers
            .iter()
            .all(|hit| hit.control != CounterControl::Action)
    );
}

#[test]
fn counter_sprite_aliases_keep_the_selected_sprites_hit_flags() {
    content_reader_script::test_game::enter();
    let gui = layout_gui(br#"guiTypes={
        containerWindowType={name=unit_counter_navy}
        containerWindowType={name=naval_combat_mapicon}
        containerWindowType={name=unit_counter size={width=81 height=29}
            buttonType={name=count_bg spriteType=GFX_synthetic_plate}
            iconType={name=onmap_action_icon spriteType=GFX_synthetic_actions position={x=57 y=9} scale=0.5}
            iconType={name=counter_overlay spriteType=GFX_synthetic_overlay position={x=57 y=9}}
        }
    }"#);
    let mut unit = land_unit(1, 1, 10, 10, true);
    unit.action = Ok(5);
    let (_, report) =
        draw_land_action_fixture_with(gui, &[unit], false, &AbilityInputs::default(), |assets| {
            assets
                .sheets
                .get_mut("GFX_synthetic_overlay")
                .unwrap()
                .texture = 2;
            let registry = native::resources::Registry::from_files(vec![(
                "synthetic.gfx".into(),
                native::sprites::read_sprites(
                    br#"spriteTypes={
                spriteType={name=GFX_synthetic_plate}
                spriteType={name=GFX_synthetic_actions}
                spriteType={name=GFX_synthetic_overlay alwaystransparent=yes}
            }"#,
                ),
            )]);
            assets.gui = GuiResources::new(registry, b"english", [100, 100], Default::default());
        });
    let hit = report
        .land_hovers
        .iter()
        .rev()
        .find(|hit| hit.contains([160.0, 212.0]))
        .unwrap();
    assert_eq!(hit.control, CounterControl::Action);
}

#[test]
fn active_ability_hovers_keep_individual_bound_records_and_transparency() {
    content_reader_script::test_game::enter();
    let abilities = AbilityInputs::read(
        &ability_frame(TableView::Complete(TableRows::ActiveAbilities(vec![
            ability(7, 13, Some(11), 0, b"first"),
            ability(7, 29, Some(11), 0, b"second"),
        ]))),
        &BTreeMap::from([(7, 1)]),
    )
    .unwrap();
    let mut unit = land_unit(1, 1, 10, 10, true);
    unit.commander = Some(Some(11));
    for transparent in ["no", "yes"] {
        let gui = layout_gui(format!(r#"guiTypes={{
            containerWindowType={{name=unit_counter_navy}}
            containerWindowType={{name=naval_combat_mapicon}}
            containerWindowType={{name=unit_counter size={{width=81 height=29}} clipping=no
                buttonType={{name=count_bg spriteType=GFX_synthetic_plate}}
                gridBoxType={{name=active_abilities position={{x=0 y=-50}}
                    slotsize={{width=25 height=25}} max_slots_vertical=1 format=UPPER_LEFT}}
            }}
            containerWindowType={{name=active_ability_item size={{width=20 height=20}}
                iconType={{name=active_ability orientation=center scale=0.8 alwaystransparent={transparent}}}
            }}
        }}"#).as_bytes());
        let (_, report) = draw_land_action_fixture_with(
            gui,
            std::slice::from_ref(&unit),
            false,
            &abilities,
            |assets| {
                let root = assets.windows.guis.template(LAND_WINDOW).unwrap();
                assets.layouts.abilities = layouts::ability_grid(
                    &assets.windows.guis,
                    root,
                    &mut Vec::new(),
                    &mut Vec::new(),
                )
                .unwrap();
                let mut ability_sprites =
                    Arc::unwrap_or_clone(std::mem::take(&mut assets.ability_sprites));
                for token in [b"first".to_vec(), b"second".to_vec()] {
                    ability_sprites
                        .by_token
                        .insert(token, "GFX_synthetic_ability".into());
                }
                assets.ability_sprites = Arc::new(ability_sprites);
                assets.sheets.insert(
                    "GFX_synthetic_ability".into(),
                    SpriteSheet {
                        name: "GFX_synthetic_ability".into(),
                        texture: 4,
                        frames: 1,
                        size: [32, 32],
                    },
                );
                let registry = native::resources::Registry::from_files(vec![("synthetic.gfx".into(),
                native::sprites::read_sprites(b"spriteTypes={spriteType={name=GFX_synthetic_plate} spriteType={name=GFX_synthetic_ability}}"))]);
                assets.gui =
                    GuiResources::new(registry, b"english", [100, 100], Default::default());
            },
        );
        let hovers: Vec<_> = report
            .land_hovers
            .iter()
            .filter(|hit| matches!(hit.control, CounterControl::Ability { .. }))
            .collect();
        if transparent == "yes" {
            assert!(hovers.is_empty());
        } else {
            assert_eq!(hovers.len(), 2);
            assert_eq!(
                hovers[0].control,
                CounterControl::Ability {
                    country: 7,
                    ordinal: 13
                }
            );
            assert_eq!(
                hovers[1].control,
                CounterControl::Ability {
                    country: 7,
                    ordinal: 29
                }
            );
            assert!(hovers[0].contains([116.0, 166.0]));
            assert!(!hovers[1].contains([116.0, 166.0]));
            assert!(hovers[1].contains([155.0, 166.0]));
        }
    }
}

#[test]
fn the_resource_and_raid_modes_draw_no_unit_counters() {
    let political = Visibility::POLITICAL;
    let hidden = Visibility {
        unit_counters: false,
        ..Visibility::POLITICAL
    };
    assert!(unit_counters_shown(
        &political,
        visible(100.0, 900.0),
        false
    ));
    assert!(!unit_counters_shown(
        &political,
        visible(900.0, 900.0),
        false
    ));
    assert!(!unit_counters_shown(
        &political,
        visible(100.0, 900.0),
        true
    ));
    for height in [0.0, 100.0, 899.0] {
        assert!(!unit_counters_shown(&hidden, visible(height, 900.0), false));
    }
}

#[test]
fn memoized_template_icons_equal_a_fresh_read_hour_by_hour() {
    let icon = |template, name: &str, custom_icon| replay_data::rows::DivisionTemplateIconRow {
        template,
        priority_unit: 7,
        priority_unit_name: name.as_bytes().to_vec(),
        custom_icon,
        ..Default::default()
    };
    let frame = |hour, view: TableView| {
        let mut frame = DecodedFrame {
            branch: 1,
            hour,
            tables: std::array::from_fn(|_| TableView::Unavailable {
                reason: "not needed".into(),
            })
            .map(Into::into),
            metrics: Metrics::default(),
        };
        *frame.tables[TableId::DivisionTemplateIcons.index()] = view;
        frame
    };
    let complete = |rows| TableView::Complete(TableRows::DivisionTemplateIcons(rows));
    let at = |frame: &DecodedFrame, hour| DecodedFrame {
        hour,
        ..frame.clone()
    };
    let first = frame(1, complete(vec![icon(50, "infantry", -1), icon(60, "", 3)]));
    let other = frame(8, TableView::Complete(TableRows::Countries(Vec::new())));
    let hours = [
        at(&first, 1),
        at(&first, 2),
        frame(3, complete(vec![icon(50, "infantry", -1), icon(60, "", 3)])),
        frame(4, complete(vec![icon(50, "armor", -1), icon(60, "", 3)])),
        frame(
            5,
            complete(vec![icon(50, "infantry", -1), icon(50, "armor", 2)]),
        ),
        frame(
            6,
            TableView::Truncated {
                rows: TableRows::DivisionTemplateIcons(vec![icon(70, "cavalry", -1)]),
                missing: 3,
            },
        ),
        frame(
            7,
            TableView::Unavailable {
                reason: "not read this hour".into(),
            },
        ),
        at(&other, 8),
        at(&other, 9),
    ];
    let read = |icons: &io::Result<Arc<TemplateIcons>>| match icons {
        Ok(icons) => Ok((
            icons.by_template.clone(),
            icons.grouping.clone(),
            icons.priorities.clone(),
        )),
        Err(error) => Err(error.to_string()),
    };
    let memo = TemplateIconMemo::default();
    let kept: Vec<_> = hours
        .iter()
        .map(|frame| {
            let fresh = TemplateIcons::read(frame).map(Arc::new);
            let kept = memo.read(frame);
            assert_eq!(read(&kept), read(&fresh), "hour {}", frame.hour);
            kept
        })
        .collect();
    let value = |hour: usize| kept[hour - 1].as_ref().unwrap();
    assert!(Arc::ptr_eq(value(1), value(2)));
    assert!(!Arc::ptr_eq(value(1), value(3)));
    assert_eq!(read(&kept[0]), read(&kept[2]));
    assert_eq!(value(4).by_template[&50].name, "armor");
    assert_eq!(
        (
            value(5).by_template[&50].name.as_str(),
            value(5).by_template[&50].custom_icon
        ),
        ("armor", 2)
    );
    assert_eq!(value(6).priorities.keys().collect::<Vec<_>>(), [&70]);
    assert!(value(7).by_template.is_empty());
    for hour in [8, 9] {
        let Err(refusal) = &kept[hour - 1] else {
            panic!("hour {hour} is not refused");
        };
        assert_eq!(refusal.to_string(), "division template icons holds rows of another kind", "hour {hour}");
    }
}

mod capture_scope;
mod retained;

#[test]
fn a_view_shows_an_icon_family_below_its_own_cutoff_or_at_the_height_a_zoom_is_sending_it_to() {
    use super::Shown;
    let defines = &CounterDefines::VANILLA;
    let political = &Visibility::POLITICAL;
    assert_eq!(Shown::at(Some(110.0), None, political, defines), Shown::EVERY);
    assert_eq!(
        Shown::at(Some(2844.0), None, political, defines),
        Shown { units: false, bases: false, land_combats: false, naval_missions: false },
        "above every icon cutoff nothing of the hour's icon families can be drawn"
    );
    assert_eq!(Shown::at(Some(2844.0), Some(110.0), political, defines), Shown::EVERY);
    assert_eq!(
        Shown::at(Some(1200.0), None, political, defines),
        Shown { units: false, bases: false, land_combats: true, naval_missions: true }
    );
    assert_eq!(Shown::at(None, None, political, defines), Shown::EVERY);
}

#[test]
fn the_dominance_sources_are_made_where_no_naval_mission_icon_draws() {
    use super::{Need, Shown};
    content_reader_script::test_game::enter();
    with_counter_inputs_and_content(action_gui(false), |_| {}, |_| {}, |inputs, content| {
        let frame = DecodedFrame {
            branch: 1,
            hour: 1,
            metrics: Default::default(),
            tables: TableId::ALL.map(|table| TableView::Complete(TableRows::empty(table)).into()),
        };
        let moves = viewer_unit_reads::movement_reads::test_support::unnamed_assets();
        for naval_missions in [false, true] {
            let need = Need { air_missions: false, by_key: true, shown: Shown { naval_missions, ..Shown::EVERY } };
            let tables = hour_tables_profiled(
                &frame,
                inputs.map,
                Arc::new(Vision::sharing(1, [])),
                content,
                inputs.assets,
                inputs.equipment,
                inputs.localisation,
                &Default::default(),
                None,
                None,
                None,
                &type_tint::Memo::new(type_tint::TABLES),
                &MissionMemo::new(),
                moves.movement_reads(),
                Err("unreached synthetic naval topology".into()),
                Kinds::default(),
                need,
                &Carry::default(),
                None,
                &mut viewer_runtime::profiling::BuilderSpans::new(false),
            )
            .unwrap();
            assert_eq!((*tables.naval_dominance_sources()).as_ref().map(BTreeMap::len), Ok(0), "naval mission icons drawn: {naval_missions}");
        }
    });
}

#[test]
fn unknown_relations_keep_distinct_foreign_owners_apart_in_land_and_naval_grouping() {
    let countries = countries([(0, Allegiance::Ours)]);
    let templates = TemplateIcons {
        grouping: [(10, (b"infantry".to_vec(), -1, 0))].into(),
        ..Default::default()
    };
    let foreign_a = land_unit(1, 1, 10, 10, false);
    let foreign_a2 = land_unit(2, 1, 10, 10, false);
    let foreign_b = land_unit(3, 2, 10, 10, false);
    assert_eq!(
        foreign_a2.can_join(&foreign_a, &templates, &countries, rules(true)),
        Ok(true)
    );
    assert_eq!(
        foreign_b.can_join(&foreign_a, &templates, &countries, rules(true)),
        Ok(false)
    );
    assert_eq!(
        foreign_b.can_join(&foreign_a, &templates, &countries, rules(false)),
        Ok(false)
    );

    let vision = Vision::default();
    let native = NativeInputs::default();
    let context = naval_context(&vision, &native, BTreeMap::new());
    let force = |id| replay_data::rows::TaskForceRow {
        id,
        mission: 1,
        ..Default::default()
    };
    let (fa, fa2, fb) = (force(10), force(11), force(12));
    fn unit(force: &replay_data::rows::TaskForceRow, slot: u32) -> NavalUnit<'_> {
        NavalUnit {
            force,
            ordinal: 0,
            slot,
            logical: Some(slot),
            flag_owner: slot,
            sight: Sight::Full,
            byte: 255,
            ships: Some(1),
            position: None,
            strength: counter_status::FIXED,
        }
    }
    assert!(naval_can_join(
        &unit(&fa, 1),
        &unit(&fa2, 1),
        rules(true),
        &countries,
        &context
    ));
    assert!(!naval_can_join(
        &unit(&fa, 1),
        &unit(&fb, 2),
        rules(true),
        &countries,
        &context
    ));
    assert!(!naval_can_join(
        &unit(&fa, 1),
        &unit(&fb, 2),
        rules(false),
        &countries,
        &context
    ));
}

#[test]
fn land_stack_refuses_non_finite_group_marker_and_preserves_valid_ones() {
    let countries = Arc::new(Countries::default());
    let templates = Arc::new(TemplateIcons::default());
    let vision = Arc::new(Vision::sharing(1, []));
    let abilities = AbilityInputs::default();
    let mut orders = counter_inputs::OrdersIndex::default();
    orders.groups.insert(
        10,
        counter_inputs::GroupFacts {
            parent: 20,
            army_group: false,
        },
    );
    orders.colors.insert(10, None);
    orders.colors.insert(20, Some([0.2, 0.6, 0.8, 1.0]));
    let tables = CounterTables {
        type_tints: &Default::default(),
        countries: &countries,
        icons: &templates,
        status: &Default::default(),
        movement: &Default::default(),
        actions: &Default::default(),
        vision: &vision,
        order: None,
        native: &Default::default(),
        orders: &orders,
        guns: &Default::default(),
        abilities: &abilities,
        task_forces: &Default::default(),
        hq_providers: &Default::default(),
        portraits: &Default::default(),
        icon_inputs: &Default::default(),
    };
    let mut bad_own = land_unit(1, 1, 10, 10, true);
    bad_own.orders_group = 10;
    bad_own.parent_branch = Some(false);
    let stack = land::land_stack(&[&bad_own], &tables, &Default::default(), false, &|_| false, false);
    let err = stack.marker.expect_err("non-finite land group colour must be refused");
    assert!(err.contains("non-finite") && err.contains("10"), "{err}");

    let mut good_parent = bad_own.clone();
    good_parent.parent_branch = Some(true);
    let stack =
        land::land_stack(&[&good_parent], &tables, &Default::default(), false, &|_| false, false);
    assert_eq!(stack.marker, Ok(Some([0.2, 0.6, 0.8, 1.0])));

    let mut foreign = bad_own;
    foreign.owner = 2;
    let stack = land::land_stack(&[&foreign], &tables, &Default::default(), false, &|_| false, false);
    assert_eq!(stack.marker, Ok(None));
}

#[test]
fn counters_laid_out_across_a_seek_back_never_mix_their_tables_hour_with_another() {
    use replay_data::rows::{TaskForceCounterInputRow, TaskForceRow};
    use viewer_counters_naval::naval::PortTables;
    content_reader_script::test_game::enter();
    let guis = native::Guis::new(vec![(
        MAPICONS_GUI.into(),
        native::read(
            br#"guiTypes={
        containerWindowType={name=unit_counter_navy size={width=81 height=29}
            buttonType={name=count_bg spriteType=GFX_synthetic_plate}
            iconType={name=counter_overlay spriteType=GFX_synthetic_overlay}
            iconType={name=mission_type spriteType=GFX_synthetic_actions}}
        containerWindowType={name=naval_combat_mapicon}
        containerWindowType={name=naval_base_mapicon size={width=81 height=29}}
        containerWindowType={name=unit_counter size={width=81 height=29}
            buttonType={name=count_bg spriteType=GFX_synthetic_plate}
            iconType={name=onmap_action_icon spriteType=GFX_synthetic_actions}
            iconType={name=counter_overlay spriteType=GFX_synthetic_overlay}}
    }"#,
        ),
    )]);
    with_counter_inputs_and_content(
        guis,
        |_| {},
        |content| content.aggression = Arc::new(vec![1]),
        |mut inputs, content| {
            inputs.visibility.political_mode = false;
            let frame = |hour: i32, forces: &[u64]| {
                let mut frame = DecodedFrame {
                    branch: 1,
                    hour,
                    metrics: Default::default(),
                    tables: TableId::ALL
                        .map(|table| TableView::Complete(TableRows::empty(table)).into()),
                };
                *frame.tables[TableId::TaskForces.index()] = TableView::Complete(TableRows::TaskForces(
                    forces
                        .iter()
                        .map(|&id| TaskForceRow { id, province: 7, roster_owner: 10, logical_owner: 10, ..Default::default() })
                        .collect(),
                ));
                *frame.tables[TableId::TaskForceCounterInputs.index()] = TableView::Complete(TableRows::TaskForceCounterInputs(
                    forces.iter().map(|&task_force| TaskForceCounterInputRow { task_force, ..Default::default() }).collect(),
                ));
                frame
            };
            let tables = |frame: &DecodedFrame, rosters: BTreeMap<u32, Vec<(u64, u32)>>| {
                let moves = viewer_unit_reads::movement_reads::test_support::unnamed_assets();
                let mut tables = hour_tables(
                    frame,
                    inputs.map,
                    Arc::new(Vision::sharing(1, [])),
                    content,
                    inputs.assets,
                    inputs.equipment,
                    inputs.localisation,
                    &Default::default(),
                    None,
                    None,
                    None,
                    &type_tint::Memo::new(type_tint::TABLES),
                    &MissionMemo::new(),
                    moves.movement_reads(),
                    Err("unreached synthetic naval topology".into()),
                )
                .unwrap();
                tables.links = [(10, 1)].into();
                tables.countries = Arc::new(Countries { sides: [(1, Allegiance::Ours)].into(), ..Default::default() });
                tables.ports = PortTables::with_rosters(rosters);
                tables
            };
            let members = |prepared: &PreparedCounters| -> BTreeMap<u32, Vec<Vec<u64>>> {
                let pass = prepared.fixed.naval.bases.as_ref().expect("the naval base windows are planned");
                let ports = pass.ports.as_ref().expect("the port items are decided").as_ref().expect("and filled");
                ports
                    .iter()
                    .map(|(&province, items)| {
                        let items = items.as_ref().expect("the port's roster is decided");
                        (province, items.iter().map(|item| item.selectable.as_ref().map_or_else(Vec::new, |(_, members)| members.to_vec())).collect())
                    })
                    .collect()
            };
            let late = frame(60_800_559, &[1, 2, 3]);
            let early = frame(60_798_213, &[3]);
            let late_tables = tables(&late, [(7, vec![(2, 0), (3, 1)]), (8, vec![(1, 0)])].into());
            let early_tables = tables(&early, [(8, vec![(3, 0)])].into());
            let mut memo = Preparation::default();
            for resources in [0, 1] {
                let played = prepare_retained(&inputs, &late_tables, resources, &mut memo).unwrap();
                assert_eq!(members(&played), [(7, vec![vec![2, 3]]), (8, vec![vec![1]])].into());
            }
            let arrived = prepare_retained(&inputs, &early_tables, 1, &mut memo).unwrap();
            assert_eq!(members(&arrived), [(8, vec![vec![3]])].into());
            let fresh = prepare_cached(&inputs, &early_tables).unwrap();
            assert_eq!(members(&arrived), members(&fresh));
            assert_eq!(arrived.fixed.report, fresh.fixed.report);
        },
    );
}

