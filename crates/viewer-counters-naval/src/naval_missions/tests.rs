use super::MISSION_WINDOW;
use super::{
    BTreeMap, BTreeSet, COUNTED_CONTAINERS, CounterDefines, CounterReport, DecodedFrame,
    ENGAGEMENT_UNKNOWN, Instance, LaidIcon, MAPICONS_GUI, MISSION_ENTRY_WINDOW, MISSION_SPRITES,
    MapIconPool, MissionEntry, MissionIcons, MissionPass, MissionView, MissionWindows,
    NAVAL_UI_MODE, NativeInputs, P2_ACTIVE, P3_ESCORT, P3_ESCORT_ALERT, P3_RAIDING, PRIDE,
    PRIDE_SHIPS_INCOMPLETE, PRIDE_SHIP_UNRECORDED, Reached, Schedule, SlotId, Tick,
    UNPROJECTED_PLACE, Vision, content_calls, efficiency_text, mission_assets, mission_pass, native,
    native_gui, naval_windows, reach, record, tables, undecided, undefined, update,
};
use replay_data::{
    rows::CountryCounterInputRow, rows::NavalMissionLedgerRow, rows::ShipRow, rows::TableId,
    rows::TableRows, rows::TaskForceCounterInputRow, rows::TaskForceMissionRow, rows::TaskForceRow,
};
use replay_frame::{frame::Metrics, frame::TableView};
use viewer_native_gui::native_effects::Effect;
use viewer_native_gui::native_effects::synthetic;
use viewer_native_gui::native_text::Font;
use viewer_native_gui::native_text::monospace;
use viewer_native_gui::native_windows::NativeWindows;
#[cfg(test)]
use viewer_native_gui::test_support::SYNTHETIC_NAVAL_GUI;
#[cfg(test)]
use viewer_native_gui::test_support::SYNTHETIC_NAVY_GUI;

#[test]
fn mission_icons_are_drawn_strictly_below_the_contents_cutoff() {
    let mut defines = CounterDefines::FIXTURE;
    let cutoff = defines.naval_mission_cutoff;
    assert!(mission_pass(cutoff - 0.5, &defines));
    assert!(!mission_pass(cutoff, &defines));
    assert!(!mission_pass(cutoff + 0.5, &defines));
    defines.naval_mission_cutoff = 400.0;
    assert!(mission_pass(399.5, &defines));
    assert!(!mission_pass(400.0, &defines));
    assert!(!mission_pass(cutoff - 0.5, &defines));
}

const SYNTHETIC_MISSION_GUI: &str = r##"guiTypes = {
	containerWindowType = {
		name = "naval_mission_mapicon_entry"
		position = { x = 0 y = 0 }
		size = { width = 30 height = 48 }
		orientation = upper_left

		buttonType = {
			name = "button"
			quadTextureSprite = "GFX_synthetic_mission_button"
		}
		instantTextboxType = {
			name = "count"
			position = { x = 5 y = 23 }
			font = "synthetic_font"
			maxWidth = 24
			maxHeight = 14
			format = center
			text = "0"
		}
		iconType = {
			name = "efficiency_background"
			position = { x = 1 y = 36 }
			spriteType = "GFX_synthetic_efficiency_background"
		}
		instantTextboxType = {
			name = "efficiency"
			position = { x = 3 y = 34 }
			font = "synthetic_font"
			maxWidth = 24
			maxHeight = 14
			format = center
			text = "100%"
		}
		iconType = {
			name = "pride"
			position = { x = -3 y = -4 }
			spriteType = "GFX_synthetic_pride"
			scale = 0.6
		}
		iconType = {
			name = "alert"
			position = { x = 2 y = 25 }
			spriteType = "GFX_synthetic_alert"
		}
	}
	containerWindowType = {
		name = "naval_mission_mapicon"
		position = { x = 0 y = 0 }
		containerWindowType = {
			name = "content"
			position = { x = 0 y = 0 }
			OverlappingElementsBoxType = {
				name = "missions_list"
				position = { x = 0 y = 0 }
				size = { x = 241 y = 50 }
				format = left
				spacing = 2.0
			}
			gridBoxType = {
				name = "unit_grid"
				position = { x = -45 y = 70 }
				size = { width = 80 height = 70 }
				slotsize = { width = 22 height = 18 }
			}
			gridBoxType = {
				name = "enemy_unit_grid"
				position = { x = 12 y = 72 }
				size = { width = 80 height = 70 }
				slotsize = { width = 20 height = 17 }
			}
			gridBoxType = {
				name = "enemy_convoy_grid"
				position = { x = 14 y = 74 }
				size = { width = 80 height = 70 }
				slotsize = { width = 21 height = 12 }
			}
			# synthetic status controls
			containerWindowType = {
				name = "mine_status_container"
				position = { x = 2 y = 52 }
				size = { width = 20 height = 26 }
				iconType = { name = "marker" spriteType = "GFX_synthetic_mines" }
			}
			containerWindowType = {
				name = "ongoing_combat_container"
				position = { x = 5 y = 54 }
				size = { width = 21 height = 27 }
				iconType = { name = "marker" spriteType = "GFX_synthetic_ongoing_combat" }
			}
			containerWindowType = {
				name = "combat_results_container"
				position = { x = 8 y = 56 }
				size = { width = 22 height = 28 }
				iconType = { name = "marker" spriteType = "GFX_synthetic_combat_results" }
			}
			containerWindowType = {
				name = "accidents_container"
				position = { x = 11 y = 58 }
				size = { width = 23 height = 29 }
				iconType = { name = "marker" spriteType = "GFX_synthetic_accidents" }
			}
		}
	}
}"##;

const STRIKE: u64 = (2973 << 32) | 51;
const INVASION: u64 = (9001 << 32) | 51;
const PATROL_FORCE: u64 = (6689 << 32) | 51;
const PRIDE_SHIP: u64 = (77 << 32) | 52;

fn frame(pride: Option<u64>, ships: bool) -> DecodedFrame {
    let mut views = std::array::from_fn(|_| TableView::Unavailable {
        reason: "not needed".into(),
    });
    let mut set = |rows: TableRows| {
        let index = rows.table().index();
        views[index] = TableView::Complete(rows);
    };
    let force = |id, mission| TaskForceRow {
        id,
        mission,
        roster_owner: 100,
        ..Default::default()
    };
    set(TableRows::TaskForces(vec![
        force(STRIKE, 2),
        force(INVASION, 9),
        force(PATROL_FORCE, 1),
    ]));
    let mission = |task_force, current_region, regions: &[u32]| TaskForceMissionRow {
        task_force,
        owner: 100,
        current_region,
        regions: regions.to_vec(),
    };
    set(TableRows::TaskForceMissions(vec![
        mission(STRIKE, 84, &[84, 97]),
        mission(INVASION, 0, &[84]),
        mission(PATROL_FORCE, 79, &[76, 79]),
    ]));
    set(TableRows::NavalMissionLedgers(
        [
            (84, vec![STRIKE, INVASION]),
            (97, vec![STRIKE]),
            (76, vec![PATROL_FORCE]),
            (79, vec![PATROL_FORCE]),
        ]
        .into_iter()
        .map(
            |(region, task_forces)| replay_data::rows::NavalMissionLedgerRow {
                owner: 100,
                region,
                task_forces,
                ..Default::default()
            },
        )
        .collect(),
    ));
    set(TableRows::TaskForceCounterInputs(
        [(STRIKE, 0), (INVASION, 0), (PATROL_FORCE, 2)]
            .map(|(task_force, engagement_rule)| TaskForceCounterInputRow {
                task_force,
                engagement_rule,
                ..Default::default()
            })
            .into(),
    ));
    if let Some(ship) = pride {
        set(TableRows::CountryCounterInputs(vec![
            CountryCounterInputRow {
                forward_visibility: replay_data::rows::Recorded::NotRecorded,
                country: 100,
                pride_of_the_fleet: ship,
                owned_state_count: 1,
                ..Default::default()
            },
        ]));
    }
    if ships {
        set(TableRows::Ships(vec![ShipRow {
            id: PRIDE_SHIP,
            task_force: PATROL_FORCE,
            ..Default::default()
        }]));
    }
    DecodedFrame {
        branch: 1,
        hour: 0,
        tables: views.map(Into::into),
        metrics: Metrics::default(),
    }
}

fn icons(frame: &DecodedFrame) -> MissionIcons {
    let native = NativeInputs::read(frame).unwrap();
    MissionIcons::read(
        frame,
        &Vision::default(),
        &BTreeMap::from([(100, 0)]),
        &BTreeMap::from([(0, 100)]),
        &native,
        &frame
            .complete::<tables::TaskForces>()
            .unwrap()
            .iter()
            .map(|row| (row.id, row))
            .collect(),
    )
    .unwrap()
}

fn entry(icons: &MissionIcons, region: u32, kind: u32) -> &MissionEntry {
    icons.regions[&region]
        .iter()
        .find(|entry| entry.kind == kind)
        .unwrap()
}

#[test]
fn f2_missing_engagement_row_is_named_unknown() {
    let mut frame = frame(Some(0), true);
    *frame.tables[TableId::TaskForceCounterInputs.index()] =
        TableView::Complete(TableRows::TaskForceCounterInputs(vec![
            TaskForceCounterInputRow {
                task_force: INVASION,
                engagement_rule: 2,
                ..Default::default()
            },
        ]));
    let icons = icons(&frame);
    assert_eq!(entry(&icons, 84, 2).alert, Err(ENGAGEMENT_UNKNOWN));
    assert_eq!(entry(&icons, 84, 9).alert, Err(P2_ACTIVE));
}

#[test]
fn pride_order_uses_last_counted_registration_across_countries_and_repeats() {
    let mut frame = frame(Some(PRIDE_SHIP), true);
    let other_ship = PRIDE_SHIP + (1 << 32);
    *frame.tables[TableId::TaskForces.index()] = TableView::Complete(TableRows::TaskForces(vec![
        TaskForceRow {
            id: STRIKE,
            mission: 2,
            roster_owner: 100,
            ..Default::default()
        },
        TaskForceRow {
            id: PATROL_FORCE,
            mission: 1,
            roster_owner: 200,
            ..Default::default()
        },
    ]));
    *frame.tables[TableId::Ships.index()] = TableView::Complete(TableRows::Ships(vec![
        ShipRow {
            id: PRIDE_SHIP,
            task_force: STRIKE,
            ..Default::default()
        },
        ShipRow {
            id: other_ship,
            task_force: PATROL_FORCE,
            ..Default::default()
        },
    ]));
    *frame.tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(TableRows::CountryCounterInputs(vec![
            CountryCounterInputRow {
                forward_visibility: replay_data::rows::Recorded::NotRecorded,
                country: 100,
                pride_of_the_fleet: PRIDE_SHIP,
                ..Default::default()
            },
            CountryCounterInputRow {
                forward_visibility: replay_data::rows::Recorded::NotRecorded,
                country: 200,
                pride_of_the_fleet: other_ship,
                ..Default::default()
            },
        ]));
    for (order, chosen, patrol_count) in [
        (vec![STRIKE, PATROL_FORCE], 1, 1),
        (vec![PATROL_FORCE, STRIKE], 2, 1),
        (vec![PATROL_FORCE, STRIKE, PATROL_FORCE], 1, 2),
    ] {
        *frame.tables[TableId::NavalMissionLedgers.index()] =
            TableView::Complete(TableRows::NavalMissionLedgers(vec![
                NavalMissionLedgerRow {
                    owner: 100,
                    region: 84,
                    task_forces: order,
                    ..Default::default()
                },
            ]));
        let icons = MissionIcons::read(
            &frame,
            &Vision::default(),
            &[(100, 0), (200, 1)].into(),
            &[(0, 100), (1, 200)].into(),
            &NativeInputs::read(&frame).unwrap(),
            &frame
                .complete::<tables::TaskForces>()
                .unwrap()
                .iter()
                .map(|row| (row.id, row))
                .collect(),
        )
        .unwrap();
        assert_eq!(entry(&icons, 84, 1).pride, Ok(chosen == 1));
        assert_eq!(entry(&icons, 84, 2).pride, Ok(chosen == 2));
        assert_eq!(entry(&icons, 84, 1).total, patrol_count);
    }
}

#[test]
fn complete_hours_decide_what_their_inputs_decide_and_report_the_rest() {
    let complete = icons(&frame(Some(PRIDE_SHIP), true));
    assert_eq!(complete.unavailable, None);
    let strike = entry(&complete, 84, 2);
    let invasion = entry(&complete, 84, 9);
    assert_eq!((strike.alert, invasion.alert), (Ok(true), Ok(true)));
    assert_eq!(
        (strike.active, invasion.active),
        (Err(P2_ACTIVE), Err(P2_ACTIVE))
    );
    let patrol = entry(&complete, 79, 1);
    assert_eq!(
        (patrol.active, patrol.alert, patrol.pride),
        (Ok(Some(1)), Ok(false), Ok(true))
    );
    assert_eq!(entry(&complete, 76, 1).alert, Ok(true));
    assert_eq!((strike.pride, invasion.pride), (Ok(false), Ok(false)));
    let strike_sprite = MISSION_SPRITES[1].1;
    let invasion_sprite = MISSION_SPRITES[7].1;
    let hidden: Vec<String> = [(strike, strike_sprite), (invasion, invasion_sprite)]
        .into_iter()
        .flat_map(|(entry, sprite)| {
            undecided(
                entry.kind,
                sprite,
                &record(entry.kind, std::slice::from_ref(entry)),
            )
        })
        .collect();
    assert_eq!(
        hidden,
        [
            format!("its count for mission kind 2 ({strike_sprite}): {P2_ACTIVE}"),
            format!("its count for mission kind 9 ({invasion_sprite}): {P2_ACTIVE}"),
        ]
    );
    for (pride, ships, why) in [
        (None, true, PRIDE_SHIP_UNRECORDED),
        (Some(PRIDE_SHIP), false, PRIDE_SHIPS_INCOMPLETE),
    ] {
        let unknown = icons(&frame(pride, ships));
        let patrol = entry(&unknown, 79, 1);
        assert_eq!(patrol.pride, Err(why), "{pride:?} {ships}");
        assert!(
            undecided(
                patrol.kind,
                MISSION_SPRITES[0].1,
                &record(patrol.kind, std::slice::from_ref(patrol))
            )
            .is_empty()
        );
    }
    let report = CounterReport {
        undecided_naval_mission_children: hidden.iter().map(|child| (child.clone(), 3)).collect(),
        refused_tokens: BTreeSet::from(["a token".to_owned()]),
        ..Default::default()
    };
    let mut shown = CounterReport::default();
    shown.assign(&report);
    assert_eq!(
        shown.warnings().collect::<Vec<_>>(),
        [
            "Counters: refused token: a token.".to_owned(),
            format!(
                "3 x a naval mission entry hides its count for mission kind 2 ({strike_sprite}): {P2_ACTIVE}."
            ),
            format!(
                "3 x a naval mission entry hides its count for mission kind 9 ({invasion_sprite}): {P2_ACTIVE}."
            ),
        ]
    );
    shown.assign(&CounterReport::default());
    assert_eq!(shown.warnings().count(), 0);
}

fn windows_of(source: &str) -> NativeWindows {
    NativeWindows::read(
        format!(
            "{}
{}
{source}",
            SYNTHETIC_NAVY_GUI, SYNTHETIC_NAVAL_GUI
        )
        .as_bytes(),
        VIEWPORT,
    )
    .unwrap()
}

fn synthetic_with(from: &str, to: &str) -> String {
    let source = SYNTHETIC_MISSION_GUI.replacen(from, to, 1);
    assert_ne!(source, SYNTHETIC_MISSION_GUI, "{from}");
    source
}

struct Res {
    font: Font,
    localisation: content_reader_localisation::localisation::Localisation,
    text_colors: native::fonts::TextColors,
    undefined: Vec<&'static str>,
    vertical: Vec<&'static str>,
    progress: Vec<&'static str>,
    refused: Vec<&'static str>,
    animated: Vec<(&'static str, f32)>,
}

impl Res {
    fn new() -> Self {
        let red: native::fonts::TextColors = [(b'R', [1.0, 0.0, 0.0, 1.0])].into();
        let mut font = monospace("synthetic_font", 7, 14);
        font.facts.text_colors = red.clone();
        Self {
            font,
            localisation: [].into_iter().collect(),
            text_colors: red,
            undefined: Vec::new(),
            vertical: Vec::new(),
            progress: Vec::new(),
            refused: Vec::new(),
            animated: Vec::new(),
        }
    }

    fn without(mut self, sprite: &'static str) -> Self {
        self.undefined.push(sprite);
        self
    }

    fn refusing(mut self, sprite: &'static str) -> Self {
        self.refused.push(sprite);
        self
    }

    fn defined(&self) -> impl Fn(&str) -> bool + '_ {
        |sprite| !self.undefined.contains(&sprite)
    }
}

impl native_gui::Resources for Res {
    fn sprite(&self, name: &[u8]) -> Option<native_gui::SpriteInfo> {
        let name = std::str::from_utf8(name).ok()?;
        if self.undefined.contains(&name) {
            return None;
        }
        let (kind, effect) = if self.progress.contains(&name) {
            (
                native_gui::SpriteKind::Progress {
                    size: [30, 24],
                    steps: 100,
                    colors: [[1.0; 4]; 2],
                    horizontal: true,
                    textured: true,
                },
                synthetic::progress(),
            )
        } else if let Some(&(_, seconds_per_frame)) =
            self.animated.iter().find(|(animated, _)| *animated == name)
        {
            (
                native_gui::SpriteKind::FrameAnimated {
                    seconds_per_frame,
                    looping: true,
                    play_on_show: true,
                    pause: 0.0,
                },
                synthetic::buttonstate(),
            )
        } else {
            (native_gui::SpriteKind::Plain, synthetic::buttonstate())
        };
        Some(native_gui::SpriteInfo {
            texture: [60, 24],
            frames: 2,
            vertical: self.vertical.contains(&name),
            masked: false,
            animated: false,
            kind,
            anchor: [0.0; 2],
            effect: Ok(effect),
            hit: native_gui::Hit::Frame,
            missing: None,
            refusals: if self.refused.contains(&name) {
                vec![(native_gui::RefusalKind::Reader, "a refused read".to_owned())]
            } else {
                Vec::new()
            },
        })
    }
    fn font(&self, _: &[u8]) -> Option<&Font> {
        Some(&self.font)
    }
    fn localisation(&self) -> &content_reader_localisation::localisation::Localisation {
        &self.localisation
    }
    fn text_colors(&self) -> &native::fonts::TextColors {
        &self.text_colors
    }
    fn text_effect(&self) -> Result<Effect, String> {
        Ok(synthetic::text())
    }
    fn effect(&self, file: &str) -> Result<Effect, String> {
        Err(format!("custom effect {file}"))
    }
    fn shield_flag(
        &self,
        _: bool,
        _: i32,
    ) -> Result<Option<(native_gui::Texture, [f32; 4])>, String> {
        Ok(None)
    }
    fn text_flag(&self, _: &str) -> Result<Option<native_gui::Texture>, String> {
        Ok(None)
    }
    fn gui_type(&self, _: &[u8]) -> Option<&native::Element> {
        None
    }
}

const VIEWPORT: [i32; 2] = [1920, 1080];

fn fixture_entry(kind: u32, active: Result<Option<u32>, &'static str>) -> MissionEntry {
    MissionEntry {
        kind,
        total: 2,
        active,
        pride: Ok(true),
        engagement: Ok(false),
        alert: Ok(false),
        efficiency: match kind {
            3 => Some(Err(P3_RAIDING)),
            4 => Some(Err(P3_ESCORT)),
            _ => None,
        },
    }
}

fn kinds(count: usize) -> Vec<MissionEntry> {
    MISSION_SPRITES
        .iter()
        .take(count)
        .map(|(kind, _)| fixture_entry(*kind, Ok(None)))
        .collect()
}

#[test]
fn mission_click_footprints_follow_the_composed_entry_buttons() {
    content_reader_script::test_game::enter();
    let res = Res::new();
    let mut harness = Harness::new(SYNTHETIC_MISSION_GUI);
    let entries = kinds(2);
    harness.frame(&res, &[(7, Some(&entries), [100, 200])], 0.0);
    let (origin, icon) = harness.laid(&res, &entries).remove(0);
    assert_eq!(
        icon.hits
            .iter()
            .filter(|(_, control, _)| *control
                == super::super::naval_mission_inputs::hover::Control::Button)
            .map(|(kind, _, _)| *kind)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    for (kind, control, rect) in &icon.hits {
        if *control != super::super::naval_mission_inputs::hover::Control::Button {
            continue;
        }
        let sprite = MISSION_SPRITES.iter().find(|(id, _)| id == kind).unwrap().1;
        let button = icon
            .items
            .iter()
            .flatten()
            .find(|part| {
                part.path.ends_with(".button")
                    && matches!(&part.kind,
                native_gui::PartKind::Sprite { sprite: found, .. } if name(found) == sprite)
            })
            .unwrap();
        let native_gui::PartKind::Sprite { geometry, .. } = &button.kind else {
            panic!()
        };
        let [x, y, w, h] = geometry.rect;
        assert_eq!(*rect, [x + origin[0] as f32, y + origin[1] as f32, w, h]);
    }
}

#[test]
fn f3_fading_strike_and_invasion_keep_last_unknown_count_until_rewritten() {
    content_reader_script::test_game::enter();
    let source = synthetic_with(
        "name = \"naval_mission_mapicon_entry\"",
        "name = \"naval_mission_mapicon_entry\" fade_type = linear fade_time = 1000",
    );
    let res = Res::new();
    for kind in [2, 9] {
        let mut harness = Harness::new(&source);
        let unknown = [fixture_entry(kind, Err(P2_ACTIVE))];
        harness.frame(&res, &[(1, Some(&unknown), [0, 0])], 0.0);
        harness.frame(&res, &[(1, Some(&unknown), [0, 0])], 2.0);
        let patrol = [fixture_entry(1, Ok(Some(1)))];
        harness.frame(&res, &[(1, Some(&patrol), [0, 0])], 0.01);
        let drawn = drawn_buttons(&mut harness, &patrol);
        assert!(
            !drawn.contains(
                &MISSION_SPRITES
                    .iter()
                    .find(|(id, _)| *id == kind)
                    .unwrap()
                    .1
                    .to_owned()
            )
        );
        assert!(
            harness
                .report
                .naval_mission_gaps
                .keys()
                .any(|reason| reason.contains("last listed count is unknown")),
            "{:?}",
            harness.report.naval_mission_gaps
        );
        let known = [fixture_entry(kind, Ok(Some(1)))];
        harness.frame(&res, &[(1, Some(&known), [0, 0])], 0.01);
        assert!(
            drawn_buttons(&mut harness, &known).contains(
                &MISSION_SPRITES
                    .iter()
                    .find(|(id, _)| *id == kind)
                    .unwrap()
                    .1
                    .to_owned()
            )
        );
    }
}

type Owner<'e> = (u32, Option<&'e [MissionEntry]>, [i32; 2]);

struct Harness {
    native: NativeWindows,
    windows: naval_windows::Windows,
    report: CounterReport,
    time: f32,
}

impl Harness {
    fn new(source: &str) -> Self {
        let native = windows_of(source);
        let mut windows = naval_windows::Windows::default();
        {
            let missions = MissionWindows::find(&native).unwrap();
            windows
                .missions
                .build(&native, &missions, VIEWPORT, &Res::new())
                .unwrap();
        }
        Self {
            native,
            windows,
            report: CounterReport::default(),
            time: 0.0,
        }
    }

    fn tick(&mut self, dt: f32) -> Tick {
        self.time += dt;
        Tick {
            time: self.time,
            dt,
        }
    }

    fn frame(&mut self, res: &Res, owners: &[Owner<'_>], dt: f32) {
        let tick = self.tick(dt);
        let windows = MissionWindows::find(&self.native).unwrap();
        let reached: Vec<Reached<'_>> = owners
            .iter()
            .map(|&(region, entries, point)| Reached {
                region,
                entries,
                point: Ok(point),
            })
            .collect();
        self.windows
            .missions
            .frame(&windows, res, &reached, VIEWPORT, &mut self.report)
            .unwrap();
        self.tick_windows(res, tick);
    }

    fn tick_windows(&mut self, res: &Res, tick: Tick) {
        let windows = MissionWindows::find(&self.native).unwrap();
        naval_windows::tick(
            &mut self.windows,
            &self.native,
            Some(&windows),
            res,
            tick,
            &mut self.report,
        );
        assert!(
            self.report.naval_window_notes.is_empty(),
            "{:?}",
            self.report.naval_window_notes
        );
    }

    fn laid(&mut self, res: &Res, entries: &[MissionEntry]) -> Vec<([i32; 2], LaidIcon)> {
        let windows = MissionWindows::find(&self.native).unwrap();
        self.windows
            .missions
            .laid(
                &windows,
                res,
                &|_| entries.to_vec(),
                &res.defined(),
                &mut self.report,
            )
            .into_iter()
            .filter_map(|(origin, _, laid)| Some((origin, laid.unwrap()?)))
            .collect()
    }

    fn entry_layouts(&self, res: &Res) -> Vec<native_gui::Layout> {
        let windows = MissionWindows::find(&self.native).unwrap();
        let icon = self.windows.missions.icons.values().next().unwrap();
        icon.listed
            .iter()
            .map(|&index| {
                icon.entries[index]
                    .layout(windows.entry, res, None)
                    .unwrap()
                    .unwrap()
            })
            .collect()
    }
}

fn laid_with(source: &str, entries: &[MissionEntry], res: &Res) -> LaidIcon {
    let mut harness = Harness::new(source);
    harness.frame(res, &[(1, Some(entries), [0, 0])], 0.0);
    harness.laid(res, entries).remove(0).1
}

fn all_parts(icon: &LaidIcon) -> Vec<&native_gui::Part> {
    icon.layout
        .parts
        .iter()
        .chain(icon.items.iter().flatten())
        .collect()
}

fn name(sprite: &[u8]) -> String {
    String::from_utf8_lossy(sprite).into_owned()
}

fn buttons(
    source: &str,
    entries: &[MissionEntry],
) -> Vec<(String, [f32; 2], Option<native_gui::Clip>)> {
    let icon = laid_with(source, entries, &Res::new());
    icon.items
        .iter()
        .flatten()
        .filter(|part| part.path == format!("{MISSION_ENTRY_WINDOW}.button"))
        .map(|part| match &part.kind {
            native_gui::PartKind::Sprite {
                sprite, geometry, ..
            } => (
                name(sprite),
                [geometry.rect[0], geometry.rect[1]],
                part.clip,
            ),
            other => panic!("{other:?}"),
        })
        .collect()
}

fn places(source: &str, count: usize) -> Vec<[i32; 2]> {
    let entries = kinds(count);
    let mut drawn = buttons(source, &entries);
    drawn.sort_by_key(|(sprite, _, _)| MISSION_SPRITES.iter().position(|(_, name)| name == sprite));
    drawn
        .into_iter()
        .map(|(_, at, _)| at.map(|value| value as i32))
        .collect()
}

fn laid_entry_with(
    source: &str,
    entry: &MissionEntry,
    res: &Res,
    report: &mut CounterReport,
) -> native_gui::Layout {
    let mut harness = Harness::new(source);
    let entries = std::slice::from_ref(entry);
    harness.frame(res, &[(1, Some(entries), [0, 0])], 0.0);
    harness.laid(res, entries);
    *report = std::mem::take(&mut harness.report);
    harness.entry_layouts(res).remove(0)
}

fn laid_entry(entry: &MissionEntry, res: &Res, report: &mut CounterReport) -> native_gui::Layout {
    laid_entry_with(SYNTHETIC_MISSION_GUI, entry, res, report)
}

fn part<'l>(layout: &'l native_gui::Layout, name: &str) -> Option<&'l native_gui::Part> {
    layout
        .parts
        .iter()
        .find(|part| part.path.rsplit('.').next() == Some(name))
}

fn sprite_frame(part: &native_gui::Part) -> u32 {
    match part.kind {
        native_gui::PartKind::Sprite { uv, offsets, .. } => (offsets[0] / uv[2]).round() as u32,
        ref other => panic!("{other:?}"),
    }
}

#[test]
fn counts_colour_and_undecided_children() {
    content_reader_script::test_game::enter();
    let res = Res::new();
    let mut harness = Harness::new(SYNTHETIC_MISSION_GUI);
    let entries = [fixture_entry(1, Ok(Some(0)))];
    harness.frame(&res, &[(1, Some(&entries), [0, 0])], 0.0);
    harness.laid(&res, &entries);
    let layout = harness.entry_layouts(&res).remove(0);
    let native_gui::PartKind::Text { glyphs, .. } = &part(&layout, "count").unwrap().kind else {
        panic!("the count is no text");
    };
    let text: String = glyphs.iter().map(|glyph| glyph.ch).collect();
    assert_eq!(text, "0/2");
    assert!(
        glyphs
            .iter()
            .all(|glyph| glyph.color == [1.0, 0.0, 0.0, 1.0])
    );
    let windows = MissionWindows::find(&harness.native).unwrap();
    let count = windows.entry.find(native::Kind::TextBox, "count").unwrap();
    let icon = harness.windows.missions.icons.values_mut().next().unwrap();
    assert_eq!(
        icon.entries[0]
            .control(windows.entry, count)
            .unwrap()
            .text
            .as_deref(),
        Some("\u{11}R0/2\u{11}!")
    );
    let mut report = CounterReport::default();
    let undecided = MissionEntry {
        engagement: Ok(false),
        alert: Err(P2_ACTIVE),
        ..fixture_entry(2, Err(P2_ACTIVE))
    };
    let layout = laid_entry(&undecided, &Res::new(), &mut report);
    assert!(part(&layout, "count").is_none());
    assert!(part(&layout, "alert").is_none());
    let sprite = MISSION_SPRITES[1].1;
    assert_eq!(
        report.undecided_naval_mission_children,
        BTreeMap::from([
            (
                format!("its alert for mission kind 2 ({sprite}): {P2_ACTIVE}"),
                1
            ),
            (
                format!("its count for mission kind 2 ({sprite}): {P2_ACTIVE}"),
                1
            ),
        ])
    );
}

#[test]
fn an_undefined_kind_sprite_keeps_the_authored_one() {
    content_reader_script::test_game::enter();
    let mut report = CounterReport::default();
    let res = Res::new().without("GFX_mapicon_naval_invasion_support");
    let layout = laid_entry(&fixture_entry(9, Ok(None)), &res, &mut report);
    assert!(matches!(
        &part(&layout, "button").unwrap().kind,
        native_gui::PartKind::Sprite { sprite, .. } if sprite == b"GFX_synthetic_mission_button"
    ));
    assert_eq!(report.authored_naval_mission_buttons, 1);
    assert!(
        report
            .undefined_naval_mission_sprites
            .contains("GFX_mapicon_naval_invasion_support")
    );
    assert!(
        report
            .naval_mission_warnings()
            .iter()
            .any(|line| line.contains("draw their button's authored sprite"))
    );
}

#[test]
fn native_reader_keys_keep_the_entrys_later_children() {
    content_reader_script::test_game::enter();
    let source = synthetic_with(
        "iconType = {
			name = \"pride\"",
        "iconType = { name = \"extra\" spriteType = \"GFX_x\" 3dposition = { 1 2 3 }              context_aware_tooltip = { a = b } }
		iconType = {
			name = \"pride\"",
    );
    let layout = laid_entry_with(
        &source,
        &fixture_entry(1, Ok(Some(2))),
        &Res::new(),
        &mut CounterReport::default(),
    );
    assert!(part(&layout, "extra").is_some());
    assert!(part(&layout, "pride").is_some());
}

#[test]
fn inline_icons_in_mission_texts_are_acquired() {
    content_reader_script::test_game::enter();
    let source = synthetic_with("text = \"100%\"", "text = \"EFFICIENCY_KEY\"");
    let windows = windows_of(&source);
    let localisation: content_reader_localisation::localisation::Localisation =
        [("EFFICIENCY_KEY".to_owned(), "£mission_icon 5".to_owned())]
            .into_iter()
            .collect();
    let registry = native::resources::Registry::default();
    let assets = mission_assets(&windows, &registry, &localisation);
    assert!(assets.sprites.contains(b"GFX_mission_icon".as_slice()));
}

#[test]
fn modded_forms_lay_out_rather_than_being_refused() {
    content_reader_script::test_game::enter();
    let right = synthetic_with("format = left", "format = right");
    assert_eq!(places(&right, 2).len(), 2);
    let clipping = synthetic_with(
        "orientation = upper_left\n\n\t\tbuttonType",
        "orientation = upper_left clipping = yes\n\n\t\tbuttonType",
    );
    assert_eq!(places(&clipping, 2).len(), 2);
    let extra = synthetic_with(
        "iconType = {\n\t\t\tname = \"alert\"",
        "containerWindowType = { name = \"wrapper\" iconType = { name = \"badge\" spriteType = \"GFX_x\" } }\n\t\ticonType = {\n\t\t\tname = \"alert\"",
    );
    let layout = laid_entry_with(
        &extra,
        &fixture_entry(1, Ok(Some(2))),
        &Res::new(),
        &mut CounterReport::default(),
    );
    assert!(layout.parts.iter().any(|part| part.path.ends_with("badge")));
}

fn entry_with(extra: &str) -> String {
    let buttons = "orientation = upper_left\n\n\t\tbuttonType";
    synthetic_with(
        buttons,
        &format!("orientation = upper_left {extra}\n\n\t\tbuttonType"),
    )
}

fn drawn_buttons(harness: &mut Harness, entries: &[MissionEntry]) -> Vec<String> {
    let laid = harness.laid(&Res::new(), entries);
    laid.iter()
        .flat_map(|(_, icon)| icon.items.iter().flatten())
        .filter(|part| part.path == format!("{MISSION_ENTRY_WINDOW}.button"))
        .map(|part| match &part.kind {
            native_gui::PartKind::Sprite { sprite, .. } => name(sprite),
            other => panic!("{other:?}"),
        })
        .collect()
}

fn gaps(harness: &mut Harness, what: &str) -> usize {
    std::mem::take(&mut harness.report)
        .naval_mission_gaps
        .iter()
        .filter(|(reason, _)| reason.contains(what))
        .map(|(_, count)| count)
        .sum()
}

#[test]
fn an_undecided_pride_leaves_its_entry_unknown_until_a_decided_write() {
    content_reader_script::test_game::enter();
    let res = Res::new();
    let mut harness = Harness::new(SYNTHETIC_MISSION_GUI);
    let decided = [fixture_entry(1, Ok(Some(2)))];
    let undecided = [MissionEntry {
        pride: Err(PRIDE_SHIPS_INCOMPLETE),
        ..fixture_entry(1, Ok(Some(2)))
    }];
    harness.frame(&res, &[(1, Some(&decided), [0, 0])], 0.0);
    assert_eq!(drawn_buttons(&mut harness, &decided).len(), 1);
    assert_eq!(gaps(&mut harness, PRIDE_SHIPS_INCOMPLETE), 0);
    for _ in 0..2 {
        harness.frame(&res, &[(1, Some(&undecided), [0, 0])], 0.1);
        assert!(drawn_buttons(&mut harness, &undecided).is_empty());
        assert_eq!(gaps(&mut harness, PRIDE_SHIPS_INCOMPLETE), 1);
    }
    harness.frame(&res, &[(1, Some(&decided), [0, 0])], 0.1);
    assert_eq!(drawn_buttons(&mut harness, &decided).len(), 1);
    assert_eq!(gaps(&mut harness, PRIDE), 0);
}

#[test]
fn a_slot_allocated_after_a_divergence_leaves_what_an_update_may_keep_unknown() {
    content_reader_script::test_game::enter();
    const MOVED: &str = "its pool slot may not be the game's";
    let res = Res::new();
    let mut harness = Harness::new(&entry_with(
        "hide_animation_type = linear hide_animation_time = 1000              hide_position = { x = 0 y = 50 }",
    ));
    let (none, one, two) = ([], kinds(1), kinds(2));
    let slots = MapIconPool::Mission.initial() as u32;
    let full: Vec<Owner<'_>> = (1..=slots)
        .map(|region| (region, Some(&two[..]), [0, 0]))
        .collect();
    harness.frame(&res, &full, 0.0);
    assert_eq!(harness.windows.missions.diverged, None);
    let first = harness.windows.missions.pool.as_ref().unwrap().slot_of(&1);
    let mut more = full.clone();
    more.push((slots + 1, Some(&two[..]), [0, 0]));
    harness.frame(&res, &more, 0.01);
    assert_eq!(
        harness.windows.missions.diverged.as_deref(),
        Some(naval_windows::GROWTH_FRAME)
    );
    std::mem::take(&mut harness.report);
    let mut next = full.clone();
    next[0].1 = None;
    next.push((slots + 1, Some(&one[..]), [0, 0]));
    harness.frame(&res, &next, 0.01);
    let pool = harness.windows.missions.pool.as_ref().unwrap();
    assert_eq!(pool.slot_of(&(slots + 1)), first);
    let laid = harness.laid(&res, &two);
    assert_eq!(laid.iter().filter(|(_, icon)| icon.entries == 0).count(), 1);
    assert_eq!(gaps(&mut harness, MOVED), 2);
    next.push((slots + 2, Some(&none[..]), [0, 0]));
    harness.frame(&res, &next, 0.01);
    harness.laid(&res, &two);
    let icon_moved = format!("{MISSION_WINDOW} is not drawn: {MOVED}");
    assert_eq!(gaps(&mut harness, &icon_moved), 1);
}

#[test]
fn a_frame_that_stops_withholds_every_icon_until_rebuilt() {
    content_reader_script::test_game::enter();
    const STOPPED: &str = "its frame does not update";
    let res = Res::new();
    let two = kinds(2);
    let both = [(1, Some(&two[..]), [0, 0]), (2, Some(&two[..]), [0, 0])];
    let mut harness = Harness::new(SYNTHETIC_MISSION_GUI);
    harness.frame(&res, &both, 0.0);
    assert_eq!(drawn_buttons(&mut harness, &two).len(), 4);
    let content = "		containerWindowType = {
			name = \"content\"";
    let other = windows_of(&synthetic_with(
        content,
        &format!(
            "iconType = {{ name = \"extra\" spriteType = \"GFX_x\" }}
{content}"
        ),
    ));
    let foreign = MissionWindows::find(&other).unwrap();
    let native = harness.native.clone();
    let own = MissionWindows::find(&native).unwrap();
    let windows = MissionWindows {
        root: foreign.root,
        ..own
    };
    let reached = [
        Reached {
            region: 1,
            entries: None,
            point: Ok([0, 0]),
        },
        Reached {
            region: 2,
            entries: Some(&two),
            point: Ok([0, 0]),
        },
    ];
    let tick = harness.tick(0.01);
    assert!(
        harness
            .windows
            .missions
            .frame(&windows, &res, &reached, VIEWPORT, &mut harness.report,)
            .is_err()
    );
    harness.tick_windows(&res, tick);
    for _ in 0..2 {
        harness.frame(&res, &both, 0.01);
        assert!(drawn_buttons(&mut harness, &two).is_empty());
        assert_eq!(gaps(&mut harness, STOPPED), 2);
    }
}

#[test]
fn a_structural_refusal_withholds_the_icon_and_its_entries_until_rebuilt() {
    content_reader_script::test_game::enter();
    const REFUSED: &str = "its update does not apply";
    let res = Res::new();
    let two = kinds(2);
    let mut harness = Harness::new(SYNTHETIC_MISSION_GUI);
    harness.frame(&res, &[(1, Some(&two), [0, 0])], 0.0);
    assert_eq!(drawn_buttons(&mut harness, &two).len(), 2);
    assert_eq!(gaps(&mut harness, REFUSED), 0);
    let other = windows_of(SYNTHETIC_MISSION_GUI);
    let foreign = MissionWindows::find(&other).unwrap();
    let tick = harness.tick(0.01);
    let own = MissionWindows::find(&harness.native).unwrap();
    let windows = MissionWindows {
        list: foreign.list,
        ..own
    };
    let reached = [Reached {
        region: 1,
        entries: Some(&two),
        point: Ok([0, 0]),
    }];
    harness
        .windows
        .missions
        .frame(&windows, &res, &reached, VIEWPORT, &mut harness.report)
        .unwrap();
    harness.tick_windows(&res, tick);
    let pool = harness.windows.missions.pool.as_ref().unwrap();
    let icon = pool
        .visible()
        .next()
        .expect("the icon stays in the visible list");
    assert!(icon.withheld.unwrap().contains(REFUSED));
    assert!(drawn_buttons(&mut harness, &two).is_empty());
    assert_eq!(gaps(&mut harness, REFUSED), 1);
    for _ in 0..3 {
        harness.frame(&res, &[(1, Some(&two), [0, 0])], 0.01);
    }
    assert!(drawn_buttons(&mut harness, &two).is_empty());
    assert_eq!(gaps(&mut harness, REFUSED), 1);
    let lost_tick = harness.tick(0.01);
    let own = MissionWindows::find(&harness.native).unwrap();
    let unprojected = [Reached {
        region: 1,
        entries: Some(&two),
        point: Err(UNPROJECTED_PLACE),
    }];
    harness
        .windows
        .missions
        .frame(&own, &res, &unprojected, VIEWPORT, &mut harness.report)
        .unwrap();
    harness.tick_windows(&res, lost_tick);
    harness.frame(&res, &[(1, Some(&two), [0, 0])], 0.01);
    let pool = harness.windows.missions.pool.as_ref().unwrap();
    let icon = pool.visible().next().expect("the icon stays live");
    assert!(
        icon.withheld.unwrap().contains(REFUSED),
        "structural refusal reason must survive projection loss/recovery, got {:?}",
        icon.withheld
    );
    assert!(drawn_buttons(&mut harness, &two).is_empty());
    assert_eq!(gaps(&mut harness, REFUSED), 1);
    harness.native = windows_of(SYNTHETIC_MISSION_GUI);
    {
        let windows = MissionWindows::find(&harness.native).unwrap();
        harness
            .windows
            .missions
            .build(&harness.native, &windows, VIEWPORT, &res)
            .unwrap();
    }
    harness.frame(&res, &[(1, Some(&two), [0, 0])], 0.0);
    assert_eq!(drawn_buttons(&mut harness, &two).len(), 2);
    assert_eq!(gaps(&mut harness, REFUSED), 0);
}

#[test]
fn a_list_placement_refusal_withholds_the_entries_until_they_are_placed_again() {
    content_reader_script::test_game::enter();
    const UNPLACED: &str = "its place in";
    let root = "name = \"naval_mission_mapicon\"";
    let source = entry_with("fade_type = linear fade_time = 1000").replacen(
        root,
        &format!("{root} iconType = {{ name = \"mark\" spriteType = \"GFX_mark\" }}"),
        1,
    );
    let res = Res::new();
    let refusing = Res::new().refusing("GFX_mark");
    let (one, three) = (kinds(1), kinds(3));
    let mut harness = Harness::new(&source);
    harness.frame(&res, &[(1, Some(&kinds(2)), [0, 0])], 0.0);
    harness.frame(&res, &[(1, Some(&three), [0, 0])], 0.01);
    assert_eq!(drawn_buttons(&mut harness, &three).len(), 3);
    assert_eq!(gaps(&mut harness, UNPLACED), 0);
    harness.frame(&refusing, &[(1, Some(&three), [0, 0])], 2.0);
    assert!(drawn_buttons(&mut harness, &three).is_empty());
    assert_eq!(gaps(&mut harness, UNPLACED), 3);
    harness.frame(&res, &[(1, Some(&one), [0, 0])], 0.01);
    assert_eq!(drawn_buttons(&mut harness, &one), [MISSION_SPRITES[0].1]);
    assert_eq!(gaps(&mut harness, UNPLACED), 2);
    harness.frame(&res, &[(1, Some(&three), [0, 0])], 0.01);
    assert_eq!(drawn_buttons(&mut harness, &three).len(), 3);
    assert_eq!(gaps(&mut harness, UNPLACED), 0);
}

#[test]
fn unlisted_records_take_the_code_writes_over_no_registration() {
    content_reader_script::test_game::enter();
    let none: [MissionEntry; 0] = [];
    let red_zero = "\u{11}R0%\u{11}!";
    assert_eq!(record(1, &none).alert, Ok(true));
    assert_eq!(record(2, &none).alert, Ok(false));
    assert_eq!(record(9, &none).alert, Ok(false));
    let raiding = record(3, &none);
    assert_eq!(raiding.alert, Ok(true));
    assert_eq!(raiding.efficiency, Some(Ok(red_zero.to_owned())));
    let escort = record(4, &none);
    assert_eq!(
        (escort.alert, escort.efficiency.clone()),
        (Err(P3_ESCORT_ALERT), Some(Err(P3_ESCORT)))
    );
    assert_eq!(undecided(4, MISSION_SPRITES[3].1, &escort).len(), 2);
    assert_eq!(record(5, &none).efficiency, None);
    assert_eq!(efficiency_text(80_000), "\u{11}Y80%\u{11}!");
    assert_eq!(efficiency_text(150_000), "100%");
    let res = Res::new();
    let mut harness = Harness::new(SYNTHETIC_MISSION_GUI);
    let three = kinds(3);
    harness.frame(&res, &[(1, Some(&three), [0, 0])], 0.0);
    let one = kinds(1);
    harness.frame(&res, &[(1, Some(&one), [0, 0])], 0.0);
    let windows = MissionWindows::find(&harness.native).unwrap();
    let efficiency = windows
        .entry
        .find(native::Kind::TextBox, "efficiency")
        .unwrap();
    let icon = harness.windows.missions.icons.values_mut().next().unwrap();
    assert_eq!(icon.listed, [0]);
    let raiding = &mut icon.entries[2];
    assert_eq!(
        raiding
            .control(windows.entry, efficiency)
            .unwrap()
            .text
            .as_deref(),
        Some(red_zero)
    );
}

#[test]
fn crossing_the_cutoff_erases_and_shows_again() {
    content_reader_script::test_game::enter();
    let res = Res::new();
    let two = kinds(2);
    let mut harness = Harness::new(SYNTHETIC_MISSION_GUI);
    harness.frame(&res, &[(4, Some(&two), [10, 20])], 0.0);
    let (_, first, _) = harness
        .windows
        .missions
        .pool
        .as_ref()
        .unwrap()
        .live()
        .next()
        .unwrap();
    harness.frame(&res, &[(4, None, [10, 20])], 0.1);
    assert_eq!(
        harness
            .windows
            .missions
            .pool
            .as_ref()
            .unwrap()
            .live()
            .count(),
        0
    );
    assert!(harness.laid(&res, &two).is_empty());
    let icon = &harness.windows.missions.icons[&first];
    assert_eq!(icon.listed, [0, 1]);
    assert!(icon.entries.iter().all(|entry| !entry.drawn()));
    harness.frame(&res, &[(4, Some(&two), [10, 20])], 0.0);
    let pool = harness.windows.missions.pool.as_ref().unwrap();
    let (_, again, window) = pool.live().next().unwrap();
    assert_ne!(again, first);
    assert!(window.drawn());
    assert_eq!(window.position(), [10, 20]);
    assert_eq!(drawn_buttons(&mut harness, &two).len(), 2);
    let source = synthetic_with(
        "name = \"naval_mission_mapicon\"",
        "name = \"naval_mission_mapicon\" fade_type = linear fade_time = 1000",
    );
    let mut harness = Harness::new(&source);
    harness.frame(&res, &[(4, Some(&two), [10, 20])], 0.0);
    harness.frame(&res, &[(4, Some(&two), [10, 20])], 0.5);
    let opacity = |harness: &Harness| {
        let pool = harness.windows.missions.pool.as_ref().unwrap();
        pool.live().next().map(|(_, _, window)| window.opacity())
    };
    assert!(opacity(&harness).unwrap() > 0.4);
    harness.frame(&res, &[(4, Some(&two), [10, 20])], 2.0);
    harness.frame(&res, &[(4, None, [10, 20])], 0.1);
    assert_eq!(opacity(&harness), None);
    assert!(harness.windows.take_active());
    assert!(harness.laid(&res, &two).is_empty());
    harness.frame(&res, &[(4, Some(&two), [10, 20])], 0.0);
    assert!(opacity(&harness).unwrap() < 0.01);
}

#[test]
fn a_region_out_of_view_is_reached_until_its_provinces_are_clean() {
    content_reader_script::test_game::enter();
    let provinces = |region: u32| -> &'static [u32] {
        match region {
            4 => &[140, 141],
            5 => &[150],
            _ => &[],
        }
    };
    let all = [140, 141, 150];
    let mut schedule = Schedule::default();
    schedule.advance(all, |province| province == 140, true);
    let candidates = || BTreeSet::from([4, 5]);
    assert_eq!(
        reach(candidates(), provinces, &schedule),
        [(4, true), (5, false)]
    );
    schedule.advance(all, |province| province == 140, false);
    assert_eq!(reach(candidates(), provinces, &schedule), [(4, true)]);
    schedule.advance(all, |province| province == 140 || province == 150, false);
    assert_eq!(
        reach(candidates(), provinces, &schedule),
        [(4, true), (5, true)]
    );
    let mut left = BTreeMap::new();
    for frame in 1..=48 {
        schedule.advance(all, |_| false, false);
        let reached = reach(candidates(), provinces, &schedule);
        assert!(reached.iter().all(|(_, in_view)| !in_view), "{reached:?}");
        for region in [4, 5] {
            if !reached.iter().any(|(reached, _)| *reached == region) {
                left.entry(region).or_insert(frame);
            }
        }
    }
    assert_eq!(left, BTreeMap::from([(4, 38), (5, 48)]));
    assert!(!schedule.listed(140) && !schedule.listed(141) && !schedule.listed(150));

    let res = Res::new();
    let mut harness = Harness::new(SYNTHETIC_MISSION_GUI);
    let one = kinds(1);
    harness.frame(&res, &[(4, Some(&one), [0, 0])], 0.0);
    harness.frame(&res, &[(4, Some(&one), [0, 0])], 0.1);
    let live = |harness: &Harness| {
        harness
            .windows
            .missions
            .pool
            .as_ref()
            .unwrap()
            .live()
            .count()
    };
    assert_eq!(live(&harness), 1);
    harness.frame(&res, &[], 0.1);
    assert_eq!(live(&harness), 0);
    assert!(
        harness.report.naval_mission_gaps.is_empty(),
        "{:?}",
        harness.report
    );
}

#[test]
fn a_fixed_window_entry_is_refused_by_name() {
    content_reader_script::test_game::enter();
    let source = SYNTHETIC_MISSION_GUI.replacen(
        "containerWindowType = {\n\t\tname = \"naval_mission_mapicon_entry\"",
        "windowType = {\n\t\tname = \"naval_mission_mapicon_entry\"",
        1,
    );
    assert_ne!(source, SYNTHETIC_MISSION_GUI);
    let native = windows_of(&source);
    let Err(refusal) = MissionWindows::find(&native).map(|_| ()) else {
        panic!("a windowType entry is refused");
    };
    assert_eq!(refusal.kind, native_gui::RefusalKind::TypeConfusion);
    assert!(
        refusal
            .detail
            .contains("makes its entries of the windowType"),
        "{}",
        refusal.detail
    );
    assert!(!refusal.detail.contains("  "), "{}", refusal.detail);
    assert!(
        refusal
            .detail
            .contains("the icon's construction reads it as a container window"),
        "{}",
        refusal.detail
    );
}

#[test]
fn political_mode_keeps_the_empty_grids_and_hides_the_counted_containers() {
    content_reader_script::test_game::enter();
    let modded = synthetic_with(
        "name = \"unit_grid\"",
        "name = \"unit_grid\" background = { name = \"bg\" spriteType = \"GFX_grid_bg\" }",
    );
    let icon = laid_with(&modded, &kinds(1), &Res::new());
    let drawn: Vec<_> = icon
        .layout
        .parts
        .iter()
        .map(|part| part.path.as_str())
        .collect();
    assert!(
        drawn.iter().any(|path| path.contains("unit_grid")),
        "{drawn:?}"
    );
    for (name, _) in COUNTED_CONTAINERS {
        assert!(!drawn.iter().any(|path| path.contains(name)), "{name}");
    }
    for name in ["unit_grid", "enemy_unit_grid", "enemy_convoy_grid"] {
        assert!(
            icon.layout
                .holders
                .iter()
                .any(|holder| holder.path.ends_with(name)),
            "{name}"
        );
    }
}

#[test]
fn naval_mode_accepts_prepared_grids_and_independent_badge_counts() {
    content_reader_script::test_game::enter();
    let res = Res::new();
    let mut harness = Harness::new(SYNTHETIC_MISSION_GUI);
    harness.windows.missions.mode = NAVAL_UI_MODE;
    harness
        .windows
        .missions
        .badge_counts
        .insert(1, [Ok(2), Err("result countries unavailable".into())]);
    let entries = kinds(1);
    harness.frame(&res, &[(1, Some(&entries), [0, 0])], 0.0);
    let laid = harness.laid(&res, &entries).remove(0).1;
    let paths: Vec<_> = laid
        .layout
        .parts
        .iter()
        .map(|part| part.path.as_str())
        .collect();
    assert!(
        paths
            .iter()
            .any(|path| path.contains("ongoing_combat_container")),
        "{paths:?}"
    );
    assert!(
        !paths
            .iter()
            .any(|path| path.contains("combat_results_container"))
    );
    assert_eq!(
        harness
            .report
            .naval_mission_gaps
            .get("combat_results_container: result countries unavailable"),
        Some(&1)
    );
    harness
        .windows
        .missions
        .badge_counts
        .insert(1, [Ok(0), Ok(0)]);
    harness.frame(&res, &[(1, Some(&entries), [0, 0])], 0.0);
    assert!(
        !harness.laid(&res, &entries)[0]
            .1
            .layout
            .parts
            .iter()
            .any(|part| part.path.contains("ongoing_combat_container"))
    );
    harness
        .windows
        .missions
        .badge_counts
        .insert(1, [Ok(3), Ok(0)]);
    harness.frame(&res, &[(1, Some(&entries), [0, 0])], 0.0);
    assert!(
        harness.laid(&res, &entries)[0]
            .1
            .layout
            .parts
            .iter()
            .any(|part| part.path.contains("ongoing_combat_container")
                && part.path.ends_with("marker"))
    );
    for mode in [0, 2, 5] {
        let windows = MissionWindows::find(&harness.native).unwrap();
        let mut root = Instance::construct(windows.root, VIEWPORT, &res).unwrap();
        assert!(content_calls(&windows, mode, &mut root).is_ok());
    }
}

#[test]
fn gaps_and_unanchored_icons_are_named_and_counted() {
    content_reader_script::test_game::enter();
    let windows = windows_of("");
    let Err(reason) = MissionWindows::find(&windows).map(|_| ()) else {
        panic!("no mission windows declared");
    };
    let reason = reason.to_string();
    assert!(
        reason.contains("no .gui file declares the containerWindowType naval_mission_mapicon"),
        "{reason}"
    );
    let mut report = CounterReport {
        unanchored_naval_missions: 2,
        authored_naval_mission_buttons: 1,
        ..Default::default()
    };
    undefined(&mut report, "GFX_mapicon_naval_invasion_support");
    report
        .naval_mission_limits
        .insert("the census is incomplete");
    report.naval_mission_gaps.insert(
        format!("naval mission entries are not drawn: {reason}").into(),
        3,
    );
    let warnings = report.naval_mission_warnings();
    assert_eq!(warnings.len(), 4, "{warnings:?}");
    assert!(warnings[0].contains("the census is incomplete"));
    assert!(
        warnings[1].starts_with("3 x naval mission entries are not drawn: ")
            && warnings[1].contains("declares the containerWindowType naval_mission_mapicon")
    );
    assert!(warnings[2].starts_with("2 naval mission icons"));
    assert!(
        warnings[3].contains("1 naval mission entries")
            && warnings[3].contains("GFX_mapicon_naval_invasion_support")
    );
}

#[test]
fn slot_history_survives_unknown_census_and_unplaced_regions_becoming_drawable() {
    content_reader_script::test_game::enter();
    let gui_text = format!(
        "guiTypes={{containerWindowType={{name=unit_counter size={{width=81 height=29}} \
         buttonType={{name=count_bg spriteType=GFX_synthetic_plate}} \
         iconType={{name=onmap_action_icon spriteType=GFX_synthetic_actions}} \
         iconType={{name=counter_overlay spriteType=GFX_synthetic_overlay}}}}}}\n\
         {SYNTHETIC_NAVY_GUI}\n{SYNTHETIC_NAVAL_GUI}\n{SYNTHETIC_MISSION_GUI}"
    );
    let guis = native::Guis::new(vec![(
        MAPICONS_GUI.into(),
        native::read(gui_text.as_bytes()),
    )]);
    viewer_counter_kit::fixtures::with_counter_inputs_fixture(
        guis,
        |_| {},
        |content| {
            content.region_provinces.insert(4, vec![140]);
            content.region_provinces.insert(5, vec![150]);
            content.region_provinces.insert(6, vec![160]);
            content.region_icons.insert(4, [0.0, 0.0, 0.0]);
            content.region_icons.insert(5, [0.0, 200.0, 0.0]);
        },
        |mut inputs| {
            let mut schedule = Schedule::default();
            schedule.advance([140, 150, 160], |_| true, true);
            let res = Res::new();
            let resources = viewer_counter_kit::gui_windows::gui_assets(&inputs, None);
            let mut view = MissionView::default();
            let one = kinds(1);

            let known = MissionPass {
                regions: Some(BTreeMap::from([(4, one.clone())])),
                undecided: None,
                ..Default::default()
            };
            let mut report = CounterReport::default();
            let windows = update(&inputs, &known, &mut view, &resources, &schedule, &mut report)
                .expect("mission windows exist");
            let slot4 = view
                .pool
                .as_ref()
                .unwrap()
                .slot_of(&4)
                .expect("region 4 admitted");
            assert_eq!(
                view.laid(&windows, &res, &|_| one.clone(), &res.defined(), &mut report)
                    .len(),
                1
            );

            let unknown = MissionPass {
                regions: Some(BTreeMap::new()),
                undecided: Some("NavalMissionLedgers is incomplete"),
                ..Default::default()
            };
            let mut report = CounterReport::default();
            let windows = update(&inputs, &unknown, &mut view, &resources, &schedule, &mut report)
                .expect("mission windows exist");
            assert_eq!(
                view.pool.as_ref().unwrap().slot_of(&4),
                Some(slot4),
                "unknown census must not erase or advance retained slot"
            );
            assert!(
                view.laid(&windows, &res, &|_| Vec::new(), &res.defined(), &mut report)
                    .is_empty(),
                "unknown census must not draw stale mission icons"
            );
            assert!(
                report
                    .naval_mission_gaps
                    .keys()
                    .any(|why| why.contains("NavalMissionLedgers is incomplete")),
                "{:?}",
                report.naval_mission_gaps
            );

            let mut report = CounterReport::default();
            let windows = update(&inputs, &known, &mut view, &resources, &schedule, &mut report)
                .expect("mission windows exist");
            assert_eq!(
                view.pool.as_ref().unwrap().slot_of(&4),
                Some(slot4),
                "decided frame after unknown census keeps the same slot"
            );
            assert_eq!(
                view.laid(&windows, &res, &|_| one.clone(), &res.defined(), &mut report)
                    .len(),
                1
            );
            assert!(report.naval_mission_gaps.is_empty(), "{:?}", report.naval_mission_gaps);

            let mut view2 = MissionView::default();
            inputs.camera.height = 50.0;
            assert!(
                inputs.screen([0.0, 200.0, 0.0]).is_none(),
                "region 5 starts behind the camera"
            );
            let reg5_and_6 = MissionPass {
                regions: Some(BTreeMap::from([(5, one.clone()), (6, one.clone())])),
                undecided: None,
                ..Default::default()
            };
            let mut report = CounterReport::default();
            let windows = update(&inputs, &reg5_and_6, &mut view2, &resources, &schedule, &mut report)
                .expect("mission windows exist");
            let slot5 = view2
                .pool
                .as_ref()
                .unwrap()
                .slot_of(&5)
                .expect("unprojectable registered region still admits its logical slot");
            let slot6 = view2
                .pool
                .as_ref()
                .unwrap()
                .slot_of(&6)
                .expect("unanchored registered region still admits its logical slot");
            assert_ne!(slot5, slot6);
            assert!(
                view2.updated_grids.contains(&(5, slot5)),
                "unprojectable slot still queues retained grid updates"
            );
            assert!(
                view2.updated_grids.contains(&(6, slot6)),
                "unanchored slot still queues retained grid updates"
            );
            assert_eq!(view2.diverged, None);
            assert!(
                view2
                    .laid(&windows, &res, &|_| one.clone(), &res.defined(), &mut report)
                    .is_empty(),
                "unplaced regions cannot draw geometry"
            );
            assert!(
                report
                    .naval_mission_gaps
                    .keys()
                    .any(|why| why.contains("icon position")),
                "{:?}",
                report.naval_mission_gaps
            );

            inputs.camera.height = 400.0;
            assert!(
                inputs.screen([0.0, 200.0, 0.0]).is_some(),
                "region 5 is now projectable"
            );
            let reg5 = MissionPass {
                regions: Some(BTreeMap::from([(5, one.clone())])),
                undecided: None,
                ..Default::default()
            };
            let mut report = CounterReport::default();
            let windows = update(&inputs, &reg5, &mut view2, &resources, &schedule, &mut report)
                .expect("mission windows exist");
            assert_eq!(
                view2.pool.as_ref().unwrap().slot_of(&5),
                Some(slot5),
                "becoming drawable keeps the same logical slot"
            );
            assert_eq!(view2.diverged, None);
            assert_eq!(
                view2
                    .laid(&windows, &res, &|_| one.clone(), &res.defined(), &mut report)
                    .len(),
                1
            );
            assert!(report.naval_mission_gaps.is_empty(), "{:?}", report.naval_mission_gaps);
        },
    );
}
