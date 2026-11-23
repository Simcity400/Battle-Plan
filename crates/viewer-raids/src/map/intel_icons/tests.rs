use super::values::*;
use super::*;
use crate::map::raids::test_support::{self, put};
use replay_data::{
    rows::AgencyBufferedOperation, rows::AgencyOperationBuffersRow, rows::OperationEquipment,
    rows::OperationEquipmentTotal, rows::OperationRequirement, rows::PoliticsRow,
    rows::LocalPlayerRow, rows::RaidCounterOperative, rows::RaidIntelInputRow, rows::StateRow,
};

fn client(frame: &mut replay_frame::frame::DecodedFrame, country: u32, observer_country: u32) {
    put::<tables::LocalPlayer>(frame, vec![LocalPlayerRow { country, observer_country }]);
}
use viewer_native_gui::{native_resources::Context, sprites::Batch, test_support::gui};

fn defines() -> IntelDefines {
    IntelDefines {
        missions: Err("synthetic mission defines absent".into()),
        cutoff: 90_000_000,
        offset: [12, 40],
        max_stack: 2,
        priorities: OperationPriorities {
            can_start: 1,
            completed: 4,
            in_progress: 3,
            prepared: 2,
            default: 5,
        },
        counter: Ok(CounterDefines {
            base: 0,
            operative_base: 0,
            stacking: 50_000,
            foreign_agent: 0,
            log_factor: 0,
            divisor: 100_000,
        }),
        activity: Ok(ActivityDefines {
            thresholds: vec![100, 200],
            colors: vec![
                [0.0, 1.0, 0.0, 1.0],
                [1.0, 1.0, 0.0, 1.0],
                [1.0, 0.0, 0.0, 1.0],
            ],
        }),
        root_out: Ok([10_000, 50_000]),
        control_trade_max: Ok(100_000),
        diplomatic_max: Ok(100_000),
        propaganda_max: Ok([-100_000, -100_000]),
    }
}

#[test]
fn the_cutoff_is_strict_and_divides_in_single_precision() {
    let mut d = defines();
    assert!(visible(899.999, &d));
    assert!(!visible(900.0, &d));
    d.cutoff = 1_074_218_815;
    assert!(visible(10742.187, &d));
    assert!(!visible(10742.1875, &d));
}

#[test]
fn containers_centre_round_half_away_and_take_the_offset() {
    assert_eq!(
        container_point([400.0, 300.0], [300, 64], [12, 40]),
        [262, 308]
    );
    assert_eq!(container_point([150.5, 32.5], [300, 64], [0, 0]), [1, 1]);
    assert_eq!(
        container_point([100.2, -10.6], [300, 64], [12, 40]),
        [-38, -3]
    );
}

#[test]
fn the_activity_level_advances_on_equality_and_is_magenta_past_the_colours() {
    let d = defines().activity.unwrap();
    assert_eq!(activity_level(99, &d), 0);
    assert_eq!(activity_level(100, &d), 1);
    assert_eq!(activity_level(200, &d), 2);
    assert_eq!(activity_color(2, &d), [1.0, 0.0, 0.0, 1.0]);
    let short = ActivityDefines {
        colors: vec![[0.0, 1.0, 0.0, 1.0]],
        ..d
    };
    assert_eq!(activity_color(activity_level(500, &short), &short), MAGENTA);
}

#[test]
fn the_shield_rating_takes_the_foreign_agent_factor_and_the_defense_terms() {
    let mut d = defines().counter.unwrap();
    d.base = 100_000;
    d.operative_base = 50_000;
    d.foreign_agent = 100_000;
    let row = RaidIntelInputRow {
        strategic_present: 1,
        operatives: vec![RaidCounterOperative {
            handle: 1,
            defense: 50_000,
            context: 0,
        }],
        ..Default::default()
    };
    let rating = counter_rating(&row, &d).unwrap();
    assert_eq!(rating, 300_000);
    assert_eq!(round_whole(agency_defense(rating, &d).unwrap()), 3);
    let log = CounterDefines {
        log_factor: 100_000,
        divisor: 0,
        ..d
    };
    assert_eq!(agency_defense(171_828, &log), Ok(99_999));
    assert_eq!(round_whole(150_000), 2);
    assert_eq!(round_whole(149_999), 1);
    assert_eq!(round_whole(-150_000), -2);
    assert_eq!(
        counter_rating(
            &RaidIntelInputRow {
                strategic_present: 0,
                ..row
            },
            &d
        ),
        Ok(0)
    );
}

#[test]
fn root_out_stacks_greatest_first_ties_by_reference_with_successive_weights() {
    assert_eq!(root_out_base(10_000, 20_000, 30_000, 0), 15_000);
    assert_eq!(root_out_base(10_000, -300_000, 0, 0), 0);
    let handle = |kind: u64, id: u64| (id << 32) | kind;
    let stacked = root_out_stacked(
        &[
            (handle(1, 5), 15_000),
            (handle(2, 5), 15_000),
            (handle(1, 5), 15_000),
            (handle(1, 6), 20_000),
        ],
        50_000,
    );
    assert_eq!(
        stacked,
        vec![
            (handle(1, 6), 20_000),
            (handle(2, 5), 7_500),
            (handle(1, 5), 3_750)
        ]
    );
}

#[test]
fn operation_states_phases_and_progress_follow_the_elapsed_days() {
    let epoch = replay_data::day_night::EPOCH;
    let mut row = OperationRow {
        start_hour: epoch + 24 * 10,
        duration_days: 10,
        phase_count: 5,
        ..Default::default()
    };
    let hour = epoch + 24 * 14 + 5;
    assert_eq!(
        operation_state(&row, hour),
        OperationState::Running {
            elapsed: 4,
            remaining: 6
        }
    );
    assert_eq!(current_phase(&row, hour), Some(2));
    assert_eq!(phase_progress(4, 10), 40);
    assert_eq!(current_phase(&row, epoch + 24 * 20), None);
    row.terminal = 1;
    assert_eq!(operation_state(&row, hour), OperationState::Finished);
    row.terminal = 0;
    row.duration_days = 0;
    assert_eq!(operation_state(&row, hour), OperationState::Preparing);
}

#[test]
fn the_preparation_value_keeps_the_native_order() {
    let requirement = |amount, archetype, uses_time| OperationRequirement {
        amount,
        days: 0,
        archetype,
        equipment: 0,
        uses_time,
    };
    let mut row = OperationRow {
        preparation_current: 25_000,
        preparation_required: 100_000,
        requirements: vec![requirement(100_000, 7, 0), requirement(0, 0, 1)],
        equipment: vec![OperationEquipment {
            variant: 1,
            archetype: 7,
            amount: 90_000,
        }],
        equipment_totals: vec![
            OperationEquipmentTotal {
                archetype: 8,
                amount: 90_000,
            },
            OperationEquipmentTotal {
                archetype: 7,
                amount: 50_000,
            },
        ],
        ..Default::default()
    };
    assert_eq!(preparation(&row), Ok(31_250));
    row.preparation_current = 100_000;
    assert_eq!(preparation(&row), Ok(50_000));
    row.requirements = vec![requirement(0, 7, 0)];
    assert_eq!(preparation(&row), Ok(ONE));
}

#[test]
fn cooldowns_count_whole_days_from_one_day_else_hours() {
    assert_eq!(cooldown_text(24, "d", "h"), "\u{11}Y1d\u{11}!");
    assert_eq!(cooldown_text(49, "d", "h"), "\u{11}Y2d\u{11}!");
    assert_eq!(cooldown_text(23, "d", "h"), "23h");
    assert_eq!(gain_frame(5), 1);
    assert_eq!(gain_frame(-5), 2);
    assert_eq!(gain_frame(0), 3);
}

fn kinds(state: &StateIcon) -> Vec<(EntryKind, EntryKey)> {
    state
        .entries
        .iter()
        .map(|entry| (entry.kind, entry.key.clone()))
        .collect()
}

#[test]
fn entries_group_by_state_sort_and_bound_the_operations() {
    let raids = test_support::assets();
    let mut frame = test_support::positive_frame();
    put::<tables::Politics>(
        &mut frame,
        vec![
            PoliticsRow {
                country: 10,
                capital: 10,
                ..Default::default()
            },
            PoliticsRow {
                country: 11,
                capital: 20,
                ..Default::default()
            },
        ],
    );
    let state = |id, controller| StateRow {
        id,
        owner: controller,
        controller,
        ..Default::default()
    };
    put::<tables::States>(&mut frame, vec![state(10, 10), state(20, 11)]);
    let mut provinces = frame.complete::<tables::Provinces>().unwrap().to_vec();
    provinces[1].victory_points = 5;
    put::<tables::Provinces>(&mut frame, provinces);
    put::<tables::RaidIntelInputs>(
        &mut frame,
        vec![RaidIntelInputRow {
            country: 10,
            strategic_present: 1,
            country_defense: 250_000,
            ..Default::default()
        }],
    );
    let hour = frame.hour;
    let operative = |id: u64, ordinal, assignment| OperativeMissionRow {
        map_selected: replay_data::rows::Recorded::Value(Some(0)),
        map_warning: replay_data::rows::Recorded::Value(Some(0)),
        roster_country: 10,
        id: (id << 32) | 9,
        ordinal,
        owner_country: 10,
        assignment,
        ..Default::default()
    };
    put::<tables::OperativeMissions>(
        &mut frame,
        vec![
            OperativeMissionRow {
                variant: Some(1),
                target_country: Some(11),
                target_state: Some(20),
                ..operative(5, 1, 3)
            },
            OperativeMissionRow {
                variant: Some(3),
                target_country: Some(10),
                ..operative(6, 0, 3)
            },
            OperativeMissionRow {
                pending_variant: Some(1),
                pending_state: Some(20),
                cooldown_end_hour: Some(hour + 30),
                ..operative(7, 2, 1)
            },
            OperativeMissionRow {
                roster_country: 11,
                owner_country: 11,
                variant: Some(1),
                target_state: Some(10),
                ..operative(8, 0, 3)
            },
            operative(9, 3, 2),
        ],
    );
    put::<tables::Operations>(
        &mut frame,
        (1..=3_u64)
            .map(|id| OperationRow {
                id,
                owner: 10,
                target: 11,
                target_province: Some(3),
                definition_token: 900 + id as u32,
                ..Default::default()
            })
            .collect(),
    );
    client(&mut frame, 10, 0);
    put::<tables::AgencyOperationBuffers>(
        &mut frame,
        vec![AgencyOperationBuffersRow {
            unready: (1..=3)
                .map(|operation| AgencyBufferedOperation {
                    operation,
                    state: Some(20),
                })
                .collect(),
            ..Default::default()
        }],
    );
    let assets = IntelAssets::test(defines(), Vec::new(), Vec::new(), 0);
    let icons = icons(
        &assets,
        &raids,
        &frame,
        2,
        &content_reader_localisation::localisation::Localisation::default(),
        "english",
    );
    let states: Vec<u32> = icons.states.iter().map(|state| state.state).collect();
    assert_eq!(states, [10, 20], "{:?}", icons.report);
    let [ger, eng] = &icons.states[..] else {
        unreachable!()
    };
    assert_eq!(ger.anchor, Anchor::Province(2));
    assert_eq!(eng.anchor, Anchor::Centre([3.0, 0.0, 0.0]));
    assert_eq!(
        kinds(ger),
        [
            (EntryKind::Shield, EntryKey::Shield(2)),
            (EntryKind::Operative, EntryKey::Operative(10, (6 << 32) | 9)),
        ]
    );
    assert_eq!(
        kinds(eng),
        [
            (EntryKind::Operative, EntryKey::Operative(10, (5 << 32) | 9)),
            (EntryKind::Operative, EntryKey::Operative(10, (7 << 32) | 9)),
            (EntryKind::Operation, EntryKey::Operation(1)),
            (EntryKind::Operation, EntryKey::Operation(2)),
            (EntryKind::More, EntryKey::More),
        ]
    );
    assert!(
        ger.entries[0]
            .writes
            .contains(&Write::Text("rating_txt", "3".into()))
    );
    let relocating = &eng.entries[1].writes;
    assert!(relocating.contains(&Write::Shown("leader_enroute", true)));
    assert!(relocating.contains(&Write::Shown("mission_icon", false)));
    assert!(relocating.contains(&Write::Text(
        "mission_progress_text",
        "\u{11}Y1DAY_UNIT_SHORT\u{11}!".into()
    )));
    let operation = &eng.entries[2].writes;
    assert!(operation.contains(&Write::Shown("operation_icon", false)));
    assert!(operation.contains(&Write::Frame("operation_equipment_overlay", 2)));
    assert!(operation.contains(&Write::Shown("phase_circular_progress_icon", false)));
    assert_eq!(*eng.entries[4].tooltip, Tooltip::default());
}

#[test]
fn a_shield_needs_its_country_to_control_its_capital_state() {
    let raids = test_support::assets();
    let mut frame = test_support::positive_frame();
    put::<tables::Politics>(
        &mut frame,
        vec![PoliticsRow {
            country: 10,
            capital: 10,
            ..Default::default()
        }],
    );
    let state = |controller| StateRow {
        id: 10,
        owner: 10,
        controller,
        ..Default::default()
    };
    let assets = IntelAssets::test(defines(), Vec::new(), Vec::new(), 0);
    let built = |frame: &replay_frame::frame::DecodedFrame| {
        icons(
            &assets,
            &raids,
            frame,
            2,
            &content_reader_localisation::localisation::Localisation::default(),
            "english",
        )
    };
    put::<tables::States>(&mut frame, vec![state(10)]);
    let held = built(&frame);
    assert_eq!(held.states.len(), 1, "{:?}", held.report);
    assert_eq!(held.states[0].state, 10);
    put::<tables::States>(&mut frame, vec![state(11)]);
    assert!(built(&frame).states.is_empty());
}

#[test]
fn an_unavailable_character_table_withholds_the_operative_portrait() {
    let raids = test_support::assets();
    let mut frame = test_support::positive_frame();
    put::<tables::Politics>(
        &mut frame,
        vec![PoliticsRow {
            country: 10,
            capital: 10,
            ..Default::default()
        }],
    );
    put::<tables::States>(
        &mut frame,
        vec![StateRow {
            id: 10,
            owner: 10,
            controller: 10,
            ..Default::default()
        }],
    );
    put::<tables::OperativeMissions>(
        &mut frame,
        vec![OperativeMissionRow {
            roster_country: 10,
            id: 9,
            owner_country: 10,
            assignment: 3,
            variant: Some(3),
            target_country: Some(10),
            portrait: b"GFX_synthetic_own_portrait".to_vec(),
            ..Default::default()
        }],
    );
    let assets = IntelAssets::test(defines(), Vec::new(), Vec::new(), 0);
    let portrait = |frame: &replay_frame::frame::DecodedFrame| {
        let icons = icons(
            &assets,
            &raids,
            frame,
            2,
            &content_reader_localisation::localisation::Localisation::default(),
            "english",
        );
        icons
            .states
            .iter()
            .flat_map(|state| &state.entries)
            .find(|entry| entry.kind == EntryKind::Operative)
            .expect("the operative is drawn")
            .writes
            .iter()
            .find(|write| {
                matches!(
                    write,
                    Write::Portrait("portrait_icn", _) | Write::Withheld("portrait_icn", _)
                )
            })
            .cloned()
            .unwrap()
    };
    *frame.tables[replay_data::rows::TableId::CharacterOperatives.index()] =
        replay_frame::frame::TableView::Unavailable {
            reason: "synthetic character refusal".into(),
        };
    assert!(matches!(
        portrait(&frame),
        Write::Withheld("portrait_icn", _)
    ));
    put::<tables::CharacterOperatives>(&mut frame, Vec::new());
    assert_eq!(
        portrait(&frame),
        Write::Portrait("portrait_icn", b"GFX_synthetic_own_portrait".to_vec())
    );
}

const GUI: &[u8] = br#"guiTypes = {
    containerWindowType = { name = intel_map_mode_map_icon size = { width = 300 height = 64 }
        OverlappingElementsBoxType = { name = entries position = { x = 0 y = 14 }
            size = { x = 300 y = 64 } format = center spacing = 1 }
    }
    containerWindowType = { name = operation_map_icon_entry size = { width = 68 height = 48 }
        iconType = { name = operation_icon spriteType = GFX_even }
        iconType = { name = clear spriteType = GFX_even position = { x = 30 y = 0 } alwaystransparent = yes }
    }
    containerWindowType = { name = intel_map_icon_more size = { width = 68 height = 48 }
        iconType = { name = more_icon spriteType = wide }
    }
} "#;

fn state(kinds: &[EntryKind]) -> Arc<IntelIcons> {
    let entries = kinds
        .iter()
        .enumerate()
        .map(|(index, &kind)| Entry {
            kind,
            key: EntryKey::Operation(index as u64),
            writes: Vec::new(),
            tooltip: Arc::new(Tooltip {
                title: index.to_string(),
                ..Tooltip::default()
            }),
        })
        .collect();
    Arc::new(IntelIcons {
        states: vec![StateIcon {
            state: 10,
            anchor: Anchor::Centre([0.0; 3]),
            entries,
        }],
        ..IntelIcons::default()
    })
}

fn drawn_into(
    view: &mut IntelView,
    windows: &viewer_native_gui::native_windows::NativeWindows,
    writes: Vec<Write>,
) -> Vec<IntelHit> {
    let mut resources = gui::fixture();
    resources.physical = Some([2048, 1152]);
    let context = Context {
        resources: &resources,
        windows,
        viewport: [2048, 1152],
        time: Some(0.0),
    };
    let assets = IntelAssets::test(defines(), Vec::new(), Vec::new(), 0);
    let mut icons = (*state(&[EntryKind::Operation])).clone();
    icons.states[0].entries[0].writes = writes;
    let prepared = PreparedIntel::test_placed(Arc::new(icons), &[[400.0, 300.0]]);
    let mut hits = Vec::new();
    prepared.draw(
        &assets,
        &context,
        30,
        view,
        &mut Batch::default(),
        &mut hits,
        &mut IntelReport::default(),
    );
    hits
}

#[test]
fn a_withheld_child_shows_again_when_a_later_write_sets_it() {
    content_reader_script::test_game::enter();
    let mut view = IntelView::default();
    let windows = gui::windows(GUI);
    let top = 308.0 + 14.0 + 5.0;
    let icon = [262.0 + 115.0 + 1.0, top];
    let withheld = drawn_into(
        &mut view,
        &windows,
        vec![
            Write::Withheld("operation_icon", "an input is absent".into()),
            Write::Frame("operation_icon", 1),
            Write::Enabled("operation_icon", true),
        ],
    );
    assert_eq!(title(&withheld, icon), None);
    let recovered = drawn_into(&mut view, &windows, vec![Write::Frame("operation_icon", 1)]);
    assert_eq!(title(&recovered, icon), Some("0".into()));
    let hidden = drawn_into(
        &mut view,
        &windows,
        vec![Write::Shown("operation_icon", false)],
    );
    assert_eq!(title(&hidden, icon), None);
    let framed = drawn_into(&mut view, &windows, vec![Write::Frame("operation_icon", 1)]);
    assert_eq!(title(&framed, icon), None);
}

fn drawn(kinds: &[EntryKind]) -> (Vec<IntelHit>, usize, IntelReport) {
    let mut resources = gui::fixture();
    resources.add_sprite("wide", [68, 48], 1);
    resources.physical = Some([2048, 1152]);
    let windows = gui::windows(GUI);
    let context = Context {
        resources: &resources,
        windows: &windows,
        viewport: [2048, 1152],
        time: Some(0.0),
    };
    let assets = IntelAssets::test(defines(), Vec::new(), Vec::new(), 0);
    let prepared = PreparedIntel::test_placed(state(kinds), &[[400.0, 300.0]]);
    let mut hits = Vec::new();
    let mut report = IntelReport::default();
    let count = prepared.draw(
        &assets,
        &context,
        30,
        &mut IntelView::default(),
        &mut Batch::default(),
        &mut hits,
        &mut report,
    );
    (hits, count, report)
}

fn title(hits: &[IntelHit], point: [f32; 2]) -> Option<String> {
    hit_at(hits, point).map(|hit| hit.tooltip.title.clone())
}

#[test]
fn a_free_eye_hits_the_nearest_shown_container_in_its_own_layout() {
    let hit = |layer: u64, title: &str| IntelHit {
        shapes: vec![
            viewer_native_gui::native_gui::HitGeometry::test_rectangle(
                [0, 0],
                [20, 20],
                [2048, 1152],
            ),
        ],
        layer,
        tooltip: Arc::new(Tooltip {
            title: title.into(),
            ..Tooltip::default()
        }),
    };
    let hits = [hit(2, "far"), hit(1, "near")];
    assert_eq!(title(&hits, [5.0, 5.0]).as_deref(), Some("far"));
    let seen = |hit: &IntelHit| match hit.tooltip.title.as_str() {
        "near" => Some(([5.0, 5.0], 1)),
        _ => Some(([50.0, 50.0], 0)),
    };
    assert_eq!(
        hit_seen(&hits, seen).map(|hit| hit.tooltip.title.as_str()),
        Some("near")
    );
    assert!(hit_seen(&hits, |_| None).is_none());
}

#[test]
fn entries_take_the_list_places_and_hover_by_their_own_children() {
    content_reader_script::test_game::enter();
    let (hits, count, report) = drawn(&[EntryKind::Operation; 4]);
    assert_eq!(count, 1, "{report:?}");
    assert_eq!(hits.len(), 4);
    let top = 308.0 + 14.0 + 5.0;
    for (index, x) in [12.0, 81.0, 150.0, 219.0].into_iter().enumerate() {
        assert_eq!(
            title(&hits, [262.0 + x + 5.0, top]),
            Some(index.to_string())
        );
    }
    assert_eq!(title(&hits, [262.0 + 12.0 + 25.0, top]), None);
    assert_eq!(title(&hits, [262.0 + 12.0 + 35.0, top]), None);
    let (hits, _, _) = drawn(&[EntryKind::Operation]);
    assert_eq!(title(&hits, [262.0 + 115.0 + 1.0, top]), Some("0".into()));
    assert_eq!(title(&hits, [262.0 + 114.0, top]), None);
}

#[test]
fn compressed_entries_overlap_and_the_later_one_wins() {
    content_reader_script::test_game::enter();
    let (hits, _, report) = drawn(&[EntryKind::More; 5]);
    assert_eq!(hits.len(), 5, "{report:?}");
    let top = 308.0 + 14.0 + 5.0;
    assert_eq!(title(&hits, [262.0 + 2.0, top]), Some("0".into()));
    assert_eq!(title(&hits, [262.0 + 60.0, top]), Some("1".into()));
    assert_eq!(title(&hits, [262.0 + 232.0 + 67.0, top]), Some("4".into()));
    assert_eq!(title(&hits, [262.0 + 232.0 + 68.0, top]), None);
}

#[test]
fn both_operative_frames_test_alpha_and_transparent_pixels_leave_lower_hits() {
    content_reader_script::test_game::enter();
    let mut resources = gui::fixture();
    resources.physical = Some([2048, 1152]);
    resources.add_sprite("frame", [8, 8], 1);
    resources.sprites.get_mut("frame").unwrap().hit = native_gui::Hit::Alpha;
    let mut mask = vec![1; 64];
    mask[1] = 0;
    resources.alpha.insert(
        "frame",
        native_gui::AlphaMask {
            width: 8,
            transparent: mask.into(),
        },
    );
    let windows = gui::windows(br#"guiTypes = {
        containerWindowType = { name = intel_map_mode_map_icon size = { width = 300 height = 64 }
            OverlappingElementsBoxType = { name = entries size = { x = 300 y = 64 } format = center }
        }
        containerWindowType = { name = operative_map_icon_entry size = { width = 68 height = 48 }
            iconType = { name = frame_icon_small spriteType = frame scale = 0.7 }
            buttonType = { name = frame_btn spriteType = frame position = { x = 20 y = 0 } }
        }
    }"#);
    let context = Context {
        resources: &resources,
        windows: &windows,
        viewport: [2048, 1152],
        time: Some(0.0),
    };
    let assets = IntelAssets::test(defines(), Vec::new(), Vec::new(), 0);
    let prepared = PreparedIntel::test_placed(state(&[EntryKind::Operative]), &[[400.0, 300.0]]);
    let mut hits = Vec::new();
    let mut report = IntelReport::default();
    prepared.draw(
        &assets,
        &context,
        30,
        &mut IntelView::default(),
        &mut Batch::default(),
        &mut hits,
        &mut report,
    );
    assert_eq!(hits.len(), 1, "{report:?}");
    assert_eq!(hits[0].shapes.len(), 2);
    for shape in &hits[0].shapes {
        let [left, top, _, _] = shape.bounds().unwrap();
        let origin = [left + 1.0, top + 1.0];
        assert!(hit_at(&hits, origin).is_none());
        assert!(hit_at(&hits, [origin[0] + 1.9, origin[1]]).is_some());
        assert!(hit_at(&hits, [origin[0] + 2.0, origin[1]]).is_none());
        let lower = IntelHit {
            shapes: vec![native_gui::HitGeometry::test_rectangle(
                [0, 0],
                [2048, 1152],
                [2048, 1152],
            )],
            layer: 0,
            tooltip: Arc::new(Tooltip {
                title: "lower".into(),
                ..Default::default()
            }),
        };
        let layered = [lower, hits[0].clone()];
        assert_eq!(title(&layered, origin), Some("lower".into()));
        assert_eq!(
            title(&layered, [origin[0] + 1.9, origin[1]]),
            Some("0".into())
        );
    }
}

fn buffered_operation_icons(
    operations: Vec<OperationRow>,
    buffer: Option<AgencyOperationBuffersRow>,
) -> IntelIcons {
    buffered_operation_icons_for(operations, buffer, (10, 0))
}

fn buffered_operation_icons_for(
    operations: Vec<OperationRow>,
    buffer: Option<AgencyOperationBuffersRow>,
    (country, observer): (u32, u32),
) -> IntelIcons {
    let raids = test_support::assets();
    let mut frame = test_support::positive_frame();
    client(&mut frame, country, observer);
    put::<tables::Operations>(&mut frame, operations);
    if let Some(buffer) = buffer {
        put::<tables::AgencyOperationBuffers>(&mut frame, vec![buffer]);
    }
    let mut limits = defines();
    limits.max_stack = 1;
    icons(
        &IntelAssets::test(limits, Vec::new(), Vec::new(), 0),
        &raids,
        &frame,
        2,
        &content_reader_localisation::localisation::Localisation::default(),
        "english",
    )
}

fn buffered_test_operation(id: u64) -> OperationRow {
    OperationRow {
        id,
        owner: 10,
        target: 11,
        target_province: Some(2),
        ..Default::default()
    }
}

#[test]
fn a_saved_running_operation_absent_from_the_ui_buffer_never_draws() {
    let icons = buffered_operation_icons(
        vec![OperationRow {
            duration_days: 100,
            ..buffered_test_operation(1)
        }],
        Some(Default::default()),
    );
    assert!(
        icons
            .states
            .iter()
            .flat_map(|state| &state.entries)
            .all(|entry| !matches!(entry.kind, EntryKind::Operation))
    );
}

#[test]
fn completed_bucket_admits_before_an_older_preparation_with_one_slot() {
    let buffered = |operation| AgencyBufferedOperation {
        operation,
        state: Some(10),
    };
    let icons = buffered_operation_icons(
        vec![
            buffered_test_operation(1),
            OperationRow {
                terminal: 1,
                ..buffered_test_operation(2)
            },
        ],
        Some(AgencyOperationBuffersRow {
            completed: vec![buffered(2)],
            unready: vec![buffered(1)],
            ..Default::default()
        }),
    );
    let admitted: Vec<_> = icons
        .states
        .iter()
        .flat_map(|state| &state.entries)
        .filter_map(|entry| match entry.key {
            EntryKey::Operation(id) => Some(id),
            _ => None,
        })
        .collect();
    assert_eq!(admitted, [2]);
    assert!(
        icons
            .states
            .iter()
            .flat_map(|state| &state.entries)
            .any(|entry| entry.kind == EntryKind::More)
    );
}

#[test]
fn the_client_buffer_draws_only_while_the_map_is_viewed_as_the_clients_country() {
    let buffer = || AgencyOperationBuffersRow {
        unready: vec![AgencyBufferedOperation { operation: 1, state: Some(10) }],
        ..Default::default()
    };
    let operations = |icons: &IntelIcons| icons.states.iter().flat_map(|state| &state.entries).filter(|entry| entry.key == EntryKey::Operation(1)).count();
    for client in [(10, 0), (0, 10)] {
        let shown = buffered_operation_icons_for(vec![buffered_test_operation(1)], Some(buffer()), client);
        assert_eq!(operations(&shown), 1, "{client:?}: {:?}", shown.report);
    }
    let shown = buffered_operation_icons_for(vec![buffered_test_operation(1)], Some(buffer()), (11, 0));
    assert_eq!(operations(&shown), 0);
    assert!(
        shown.report.reasons.keys().any(|reason| reason.contains("AgencyOperationBuffers holds only the recording client's buffer")),
        "{:?}",
        shown.report
    );
    let raids = test_support::assets();
    let mut frame = test_support::positive_frame();
    put::<tables::Operations>(&mut frame, vec![buffered_test_operation(1)]);
    put::<tables::AgencyOperationBuffers>(&mut frame, vec![buffer()]);
    let localisation = content_reader_localisation::localisation::Localisation::default();
    let shown = icons(&IntelAssets::test(defines(), Vec::new(), Vec::new(), 0), &raids, &frame, 2, &localisation, "english");
    assert_eq!(operations(&shown), 0);
    assert!(shown.report.reasons.keys().any(|reason| reason.contains("LocalPlayer holds 0 rows")), "{:?}", shown.report);
}

#[test]
fn unavailable_ui_buffer_withholds_saved_operations_and_names_the_refusal() {
    let icons = buffered_operation_icons(vec![buffered_test_operation(1)], None);
    assert!(
        icons
            .states
            .iter()
            .flat_map(|state| &state.entries)
            .all(|entry| !matches!(entry.kind, EntryKind::Operation))
    );
    assert!(
        icons
            .report
            .reasons
            .iter()
            .any(|(reason, count)| *count == 1
                && reason.contains("operation entries are withheld: AgencyOperationBuffers")),
        "{:?}",
        icons.report
    );
}

#[test]
fn running_duration_substitutes_the_lowercase_green_number_and_keeps_authored_units() {
    let localisation: content_reader_localisation::localisation::Localisation = [(
        "ABILITY_DUR_DAYS".to_owned(),
        "\u{11}Y$num|H$ cycles\u{11}!".to_owned(),
    )]
    .into_iter()
    .collect();
    let text = build::running_duration_text(&localisation, 140).unwrap();
    assert!(text.contains("\u{11}G140\u{11}!"), "{text:?}");
    assert!(text.starts_with("\u{11}Y"), "{text:?}");
    assert!(text.contains(" cycles"), "{text:?}");
}

#[test]
fn a_nonterminal_operation_at_zero_remaining_days_has_the_finished_entry_presentation() {
    let raids = test_support::assets();
    let mut frame = test_support::positive_frame();
    let start_hour = frame.hour - 24;
    put::<tables::Operations>(
        &mut frame,
        vec![OperationRow {
            start_hour,
            duration_days: 1,
            terminal: 0,
            detected: replay_data::rows::Recorded::Value(0),
            ..buffered_test_operation(1)
        }],
    );
    client(&mut frame, 10, 0);
    put::<tables::AgencyOperationBuffers>(
        &mut frame,
        vec![AgencyOperationBuffersRow {
            running: vec![AgencyBufferedOperation {
                operation: 1,
                state: Some(10),
            }],
            ..Default::default()
        }],
    );
    let localisation = [
        ("FINISHED".to_owned(), "settled".to_owned()),
        ("ABILITY_DUR_DAYS".to_owned(), "$num$ cycles".to_owned()),
    ]
    .into_iter()
    .collect();
    let built = icons(
        &IntelAssets::test(defines(), Vec::new(), Vec::new(), 0),
        &raids,
        &frame,
        2,
        &localisation,
        "english",
    );
    let entry = built
        .states
        .iter()
        .flat_map(|state| &state.entries)
        .find(|entry| entry.key == EntryKey::Operation(1))
        .unwrap();
    assert!(
        entry.writes.contains(&Write::Text(
            "operation_progress_text",
            "\u{11}Gsettled\u{11}!".into()
        )),
        "{:?}",
        entry.writes
    );
    assert!(
        entry
            .writes
            .contains(&Write::Shown("phase_circular_progress_icon", false))
    );
    assert!(
        entry
            .writes
            .contains(&Write::Shown("operation_completed_overlay", true))
    );
}
