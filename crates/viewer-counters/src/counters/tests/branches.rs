use super::with_counter_inputs_and_content;

use super::{
    Allegiance, Arc, BTreeSet, BaseView, CounterColorMode, CounterInputs, DecodedFrame, HourTables,
    Layouts, MissionMemo, PoolView, STANDSTILL, Shared, Vision, countries, hour_tables, land_unit,
    land_windows, layout_gui, naval_windows, prepare_cached, type_tint,
};

fn synthetic_hour(content: &super::CounterContent, inputs: &CounterInputs<'_>, frame: &DecodedFrame) -> HourTables {
    let moves = viewer_unit_reads::movement_reads::test_support::unnamed_assets();
    hour_tables(
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
    .unwrap()
}

fn transcript(branched: bool) -> Vec<String> {
    let guis = layout_gui(br#"guiTypes={
        containerWindowType={name=unit_mapicon size={width=81 height=29}
            gridBoxType={name=units_box size={width=81 height=290}
                slotsize={width=81 height=1} max_slots_horizontal=1 format=UPPER_LEFT}
        }
        positionType={name=unit_counter_zoomed_in_offset position={x=0 y=0}}
        containerWindowType={name=unit_counter_navy size={width=20 height=10}}
        containerWindowType={name=naval_headquarter_mapicon size={width=81 height=29}
            buttonType={name=bg_btn spriteType=GFX_synthetic_plate}}
        containerWindowType={name=naval_combat_mapicon size={width=20 height=10}}
        containerWindowType={name=unit_counter size={width=81 height=29}
            buttonType={name=count_bg spriteType=GFX_synthetic_plate}
            iconType={name=onmap_action_icon spriteType=GFX_synthetic_actions position={x=57 y=9} frame=12}
            iconType={name=counter_overlay spriteType=GFX_synthetic_overlay position={x=3 y=4}}
        }
    }"#);
    with_counter_inputs_and_content(
        guis,
        |assets| {
            assets.layouts = Layouts::read(&assets.windows);
        },
        |content| {
            let points = [
                (1, [2., 9.5, 2.]),
                (2, [20., 9.5, 2.]),
                (3, [100., 9.5, 100.]),
            ];
            content.default_anchors = Arc::new(points.into());
            content.bounds = Arc::new(
                points
                    .into_iter()
                    .map(|(id, [x, _, z])| (id, [x - 1., z - 1., x + 1., z + 1.]))
                    .collect(),
            );
            content.region_of = Arc::new([(1, 1), (2, 1), (3, 2)].into());
        },
        |inputs, content| {
            use replay_data::rows::{TableId, TableRows};
            use replay_frame::frame::TableView;
            let frame = DecodedFrame {
                branch: 1,
                hour: 11,
                metrics: Default::default(),
                tables: TableId::ALL
                    .map(|table| TableView::Complete(TableRows::empty(table)).into()),
            };
            let mut tables = synthetic_hour(content, &inputs, &frame);
            tables.divisions = [
                (
                    (1, STANDSTILL),
                    vec![
                        land_unit(10, 1, 10, 10, true),
                        land_unit(11, 1, 10, 10, true),
                    ],
                ),
                ((2, STANDSTILL), vec![land_unit(12, 1, 10, 10, true)]),
                ((3, STANDSTILL), vec![land_unit(13, 1, 20, 20, true)]),
            ]
            .into();
            tables.base_icons.headquarters = vec![(1, [2., 9.5, 2.]), (3, [100., 9.5, 100.])];
            tables.countries = Arc::new(countries([(1, Allegiance::Ours)]));
            let schedule = Shared::default();
            let empty = BTreeSet::new();
            let mut inputs = CounterInputs {
                selected: &empty,
                schedule: &schedule,
                ..inputs
            };
            inputs.settings.color_mode = CounterColorMode::Allegiance;
            let mut prepared = prepare_cached(&inputs, &tables).unwrap();
            let mut windows = naval_windows::Windows::default();
            let mut pings = PoolView::default();
            let mut bases = BaseView::default();
            let mut combats = land_windows::View::default();
            let groups = inputs.content.defines.groups;
            let low = groups.province_distance - 10.0;
            let heights = [
                low,
                groups.province_distance - 0.001,
                groups.province_distance,
                groups.state_distance - 0.001,
                groups.state_distance,
                groups.strategic_distance - 0.001,
                groups.strategic_distance,
                low,
                low,
            ];
            let mut frames = Vec::new();
            for (at, height) in heights.into_iter().enumerate() {
                inputs.camera.height = height;
                inputs.camera.center = if at == 4 {
                    [100., 100.]
                } else {
                    [2. + at as f32 * 0.125, 2.]
                };
                let time = at as f32 / 240.;
                if branched {
                    prepared
                        .draw_branched(
                            &inputs,
                            time,
                            viewer_runtime::profiling::FrameOwners::new(),
                            &mut windows,
                            &mut pings,
                            &mut bases,
                            &mut combats,
                        )
                        .unwrap();
                } else {
                    prepared
                        .draw(
                            &inputs,
                            time,
                            &mut windows,
                            &mut pings,
                            &mut bases,
                            &mut combats,
                        )
                        .unwrap();
                }
                let (batch, report) = prepared.drawn();
                frames.push(format!(
                    "pings {:?}\nbases {:?}\ncombats {:?}\ncounters {:?}\nnaval combats {:?}\nreport {:?}",
                    prepared.fixed.pings.quads,
                    prepared.fixed.bases.quads,
                    prepared.fixed.combats.quads,
                    batch.quads,
                    prepared.drawn_combats().quads,
                    report,
                ));
            }
            frames
        },
    )
}

#[test]
fn branched_counters_draw_every_frame_as_the_plain_draw() {
    content_reader_script::test_game::enter();
    let serial = transcript(false);
    let branched = transcript(true);
    assert_eq!(serial.len(), branched.len());
    for (frame, (serial, branched)) in serial.iter().zip(&branched).enumerate() {
        assert_eq!(serial, branched, "frame {frame}");
    }
    assert!(
        serial.iter().any(|frame| !frame.contains("\ncounters []")),
        "the fixture must draw land counters, not only base icons"
    );
}
