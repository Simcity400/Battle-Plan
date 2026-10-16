use super::*;
use replay::{
    frame::{Metrics, TableView},
    rows::{
        CountryCounterInputRow, ShipRow, TableRows, TaskForceCounterInputRow, TaskForceMissionRow,
        TaskForceRow,
    },
    rows::{NavalMissionLedgerRow, TableId},
};
use viewer_foundation::map::native_effects::Effect;
use viewer_foundation::map::native_effects::synthetic;
use viewer_foundation::map::native_text::Font;
use viewer_foundation::map::native_text::monospace;
use viewer_foundation::map::native_windows::NativeWindows;
#[cfg(test)]
use viewer_foundation::map::test_support::SYNTHETIC_NAVAL_GUI;
#[cfg(test)]
use viewer_foundation::map::test_support::SYNTHETIC_NAVY_GUI;

#[test]
fn mission_icons_are_drawn_strictly_below_the_contents_cutoff() {
    let mut defines = CounterDefines::VANILLA;
    let cutoff = defines.naval_mission_cutoff;
    assert!(mission_pass(cutoff - 0.5, &defines));
    assert!(!mission_pass(cutoff, &defines));
    assert!(!mission_pass(cutoff + 0.5, &defines));
    // A mod's lower cutoff moves the boundary with it.
    defines.naval_mission_cutoff = 400.0;
    assert!(mission_pass(399.5, &defines));
    assert!(!mission_pass(400.0, &defines));
    assert!(!mission_pass(cutoff - 0.5, &defines));
}

// Independently authored parser/layout fixture. The lookup names are interface contracts;
// dimensions, positions, resources and sample text are synthetic, not game asset extracts.
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

/// A complete hour for country 100 (slot 0, the default viewer): a strike force and an
/// invasion support in region 84, both with engagement rule 0, and a patrol in 79 patrolling
/// there, whose task force holds the pride of the fleet. `pride` and `ships` choose what the
/// hour records of the pride operands.
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
            |(region, task_forces)| replay::rows::NavalMissionLedgerRow {
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
                forward_visibility: replay::rows::Recorded::NotRecorded,
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
    // A recorded nonzero rule leaves only the range predicate undecided.
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
                forward_visibility: replay::rows::Recorded::NotRecorded,
                country: 100,
                pride_of_the_fleet: PRIDE_SHIP,
                ..Default::default()
            },
            CountryCounterInputRow {
                forward_visibility: replay::rows::Recorded::NotRecorded,
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

/// Invasion support alerts from its registrations like kind 2, without range inputs.
/// Pride follows recorded operands and remains unknown when absent; complete hours
/// report any still-undecided children.
#[test]
fn complete_hours_decide_what_their_inputs_decide_and_report_the_rest() {
    let complete = icons(&frame(Some(PRIDE_SHIP), true));
    assert_eq!(complete.unavailable, None);
    let strike = entry(&complete, 84, 2);
    let invasion = entry(&complete, 84, 9);
    assert_eq!((strike.alert, invasion.alert), (Ok(true), Ok(true)));
    // Their active counts wait for the range test (P2).
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
    // Each strike or invasion entry hides its count, named by kind and input.
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
    // Without the country's pride reference, or without the ship census, the pride marker
    // is unknown, not absent, and names why; it is no hidden child.
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
    // The hidden children of an otherwise complete hour reach `CounterReport::warnings`,
    // which the warning area (and so the embedded map details) and the dump print, through
    // the report the app keeps across frames, by child, kind and input.
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

/// The mission windows of `source` as the draw finds them, beside the synthetic naval windows.
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

/// `SYNTHETIC_MISSION_GUI` with `from` replaced by `to` once.
fn synthetic_with(from: &str, to: &str) -> String {
    let source = SYNTHETIC_MISSION_GUI.replacen(from, to, 1);
    assert_ne!(source, SYNTHETIC_MISSION_GUI, "{from}");
    source
}

/// Every sprite a synthetic two-frame 30 x 24 strip drawn with the game's button effect, but those
/// named undefined (no sprite), those named as progress bars (30 x 24, 100 steps) and those
/// named refused (a reader refusal, which refuses every layout that shows them), and those
/// named animated frame-animated (looping, `play_on_show`, at their seconds per frame); one
/// monospace font, white with `R` red.
struct Res {
    font: Font,
    localisation: installed_content::localisation::Localisation,
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
    fn localisation(&self) -> &installed_content::localisation::Localisation {
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

/// `count` patrol, strike force, convoy raiding ... entries, by kind order.
fn kinds(count: usize) -> Vec<MissionEntry> {
    MISSION_SPRITES
        .iter()
        .take(count)
        .map(|(kind, _)| fixture_entry(*kind, Ok(None)))
        .collect()
}

#[test]
fn f3_fading_strike_and_invasion_keep_last_unknown_count_until_rewritten() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

/// One region's owner stage in a frame: its gate's entries (`None` when it fails) and its
/// point.
type Owner<'e> = (u32, Option<&'e [MissionEntry]>, [i32; 2]);

/// A map view presenting the mission windows of one source, frame by frame: the updates, then
/// the one ordered tick of the view's naval windows ([`naval_windows::tick`]), which here hold
/// only the mission windows.
struct Harness {
    native: NativeWindows,
    windows: naval_windows::Windows,
    report: CounterReport,
    /// The view's clock at the last frame.
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

    /// The next frame's tick, `dt` seconds on.
    fn tick(&mut self, dt: f32) -> Tick {
        self.time += dt;
        Tick {
            time: self.time,
            dt,
        }
    }

    /// One presented frame of `dt` seconds with `owners` reached in order.
    fn frame(&mut self, res: &Res, owners: &[Owner<'_>], dt: f32) {
        let tick = self.tick(dt);
        let windows = MissionWindows::find(&self.native).unwrap();
        let reached: Vec<Reached<'_>> = owners
            .iter()
            .map(|&(region, entries, point)| Reached {
                region,
                entries,
                point,
            })
            .collect();
        self.windows
            .missions
            .frame(&windows, res, &reached, VIEWPORT, &mut self.report)
            .unwrap();
        self.tick_windows(res, tick);
    }

    /// The view's one ordered tick of its naval windows, which must all tick.
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

    /// Every live icon laid out, its region showing `entries`.
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

    /// The listed entries' own layouts, in list order, of the one live icon.
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

/// `source`'s icon with `entries` after its first (still) frame.
fn laid_with(source: &str, entries: &[MissionEntry], res: &Res) -> LaidIcon {
    let mut harness = Harness::new(source);
    harness.frame(res, &[(1, Some(entries), [0, 0])], 0.0);
    harness.laid(res, entries).remove(0).1
}

/// Every part of the icon, its own and its entries', relative to the root.
fn all_parts(icon: &LaidIcon) -> Vec<&native_gui::Part> {
    icon.layout
        .parts
        .iter()
        .chain(icon.items.iter().flatten())
        .collect()
}

/// A sprite part's sprite name.
fn name(sprite: &[u8]) -> String {
    String::from_utf8_lossy(sprite).into_owned()
}

/// Each drawn entry button in paint order: its sprite, its top-left relative to the root
/// and its clip.
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

/// Each entry's place relative to the root, by entry index.
fn places(source: &str, count: usize) -> Vec<[i32; 2]> {
    let entries = kinds(count);
    let mut drawn = buttons(source, &entries);
    drawn.sort_by_key(|(sprite, _, _)| MISSION_SPRITES.iter().position(|(_, name)| name == sprite));
    drawn
        .into_iter()
        .map(|(_, at, _)| at.map(|value| value as i32))
        .collect()
}

/// Overlapping rows keep authored spacing when they fit and compress it otherwise.
/// Centering shifts the row. Content moves up by half its height; the list shifts
/// from its saved position by half the displayed entries' total width.
#[test]
fn entries_stand_where_the_code_moves_content_and_the_list() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let list = r#"position = { x = 0 y = 0 }
				size = { x = 241 y = 50 }"#;
    assert_eq!(
        places(SYNTHETIC_MISSION_GUI, 3),
        [[-45, -24], [-13, -24], [19, -24]]
    );
    let narrow = synthetic_with(list, "position = { x = 0 y = 0 } size = { x = 70 y = 50 }");
    assert_eq!(places(&narrow, 3), [[-45, -24], [-25, -24], [-5, -24]]);
    let offset = synthetic_with(
        list,
        "position = { x = 10 y = 5 } size = { x = 241 y = 50 }",
    );
    assert_eq!(places(&offset, 2), [[-20, -19], [12, -19]]);
    // Native centering biases the 62-pixel row by 0.5 before truncating:
    // trunc(241 / 2 - 62.5 / 2) = 89, then the updater shifts it left by 30.
    let centred = synthetic_with("format = left", "format = center");
    assert_eq!(places(&centred, 2), [[59, -24], [91, -24]]);
    assert!(places(SYNTHETIC_MISSION_GUI, 0).is_empty());
    // Unclipped: neither the synthetic root nor `content` clips.
    assert!(
        buttons(SYNTHETIC_MISSION_GUI, &kinds(3))
            .iter()
            .all(|(_, _, clip)| clip.is_none())
    );
}

/// `first_on_top` paints the first entry last, on top; where each stands is unchanged.
#[test]
fn first_on_top_reverses_the_paint_order_only() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let source = synthetic_with("spacing = 2.0", "spacing = 2.0 first_on_top = yes");
    let painted = buttons(&source, &kinds(3));
    let order: Vec<_> = painted
        .iter()
        .map(|(sprite, _, _)| sprite.as_str())
        .collect();
    assert_eq!(
        order,
        [
            MISSION_SPRITES[2].1,
            MISSION_SPRITES[1].1,
            MISSION_SPRITES[0].1
        ]
    );
    assert_eq!(painted[2].1, [-45.0, -24.0]);
}

/// Root margins inset the content coordinates used to place content and its entries.
#[test]
fn root_margins_reach_the_entries() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let source = synthetic_with(
        "name = \"naval_mission_mapicon\"",
        "name = \"naval_mission_mapicon\" size = { width = 200 height = 100 } \
         margin = { left = 10 top = 5 }",
    );
    assert_eq!(places(&source, 3), [[-35, -19], [-3, -19], [29, -19]]);
}

/// Root clipping reaches the entries its list holds.
#[test]
fn a_clipping_root_clips_the_entries() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let source = synthetic_with(
        "name = \"naval_mission_mapicon\"",
        "name = \"naval_mission_mapicon\" size = { width = 40 height = 40 } clipping = yes",
    );
    for (_, _, clip) in buttons(&source, &kinds(3)) {
        assert_eq!(clip, Some([0, 0, 40, 40]));
    }
}

/// Percentage-sized content evaluates within the root's content rectangle.
#[test]
fn a_percentage_content_lays_out_in_the_root() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let source = synthetic_with(
        "name = \"naval_mission_mapicon\"",
        "name = \"naval_mission_mapicon\" size = { width = 200 height = 100 }",
    )
    .replacen(
        "name = \"content\"",
        "name = \"content\" size = { width = 100% height = 100% }",
        1,
    );
    assert_eq!(places(&source, 3), [[-45, -24], [-13, -24], [19, -24]]);
}

/// A hidden `content` hides its entries only: the root's own artwork still draws.
#[test]
fn a_hidden_content_keeps_the_roots_artwork() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let source = synthetic_with(
        "name = \"naval_mission_mapicon\"",
        "name = \"naval_mission_mapicon\" \
         iconType = { name = \"root_art\" spriteType = \"GFX_root_art\" }",
    )
    .replacen("name = \"content\"", "name = \"content\" hide = yes", 1);
    let icon = laid_with(&source, &kinds(2), &Res::new());
    let drawn = all_parts(&icon);
    assert!(drawn.iter().any(|part| part.path.ends_with("root_art")));
    assert!(
        !drawn
            .iter()
            .any(|part| part.path.starts_with(MISSION_ENTRY_WINDOW))
    );
    assert_eq!(icon.entries, 0);
}

/// Icons without displayed registration kinds still draw their root with content
/// at zero and the list at its constructed position.
#[test]
fn an_icon_without_shown_kinds_draws_its_root() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let with_art = SYNTHETIC_MISSION_GUI.replacen(
        "OverlappingElementsBoxType = {",
        "iconType = { name = \"content_art\" spriteType = \"GFX_content_art\" position = { x = 3 y = 4 } }\n\t\t\tOverlappingElementsBoxType = {",
        1,
    );
    let icon = laid_with(&with_art, &[], &Res::new());
    let art = all_parts(&icon)
        .into_iter()
        .find(|part| part.path.ends_with("content_art"))
        .expect("content's artwork draws");
    let native_gui::PartKind::Sprite { geometry, .. } = &art.kind else {
        panic!("{art:?}");
    };
    assert_eq!(geometry.rect[..2], [3.0, 4.0]);
    // With one entry, `content` rises by half the entry's height.
    let icon = laid_with(&with_art, &kinds(1), &Res::new());
    let art = all_parts(&icon)
        .into_iter()
        .find(|part| part.path.ends_with("content_art"))
        .unwrap();
    let native_gui::PartKind::Sprite { geometry, .. } = &art.kind else {
        panic!("{art:?}");
    };
    assert_eq!(geometry.rect[..2], [3.0, 4.0 - 24.0]);
}

/// Lays the synthetic entry out for `entry` with `res`, as the icon's first frame lays it out,
/// counting into `report` what the icon's draw counts.
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

/// The zero-based sprite frame is its strip offset divided by one frame's extent.
fn sprite_frame(part: &native_gui::Part) -> u32 {
    match part.kind {
        native_gui::PartKind::Sprite { uv, offsets, .. } => (offsets[0] / uv[2]).round() as u32,
        ref other => panic!("{other:?}"),
    }
}

/// The synthetic entry draws the selected kind at one-based frame 2, centres its count
/// in the authored 24-pixel box at (5, 23), and shows pride while hiding alert and efficiency.
#[test]
fn the_synthetic_entry_draws_the_code_selected_children_at_authored_positions() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut report = CounterReport::default();
    let layout = laid_entry(&fixture_entry(2, Ok(None)), &Res::new(), &mut report);
    let names: Vec<_> = layout
        .parts
        .iter()
        .map(|part| part.path.rsplit('.').next().unwrap())
        .collect();
    assert_eq!(names, ["button", "count", "pride"]);
    let button = part(&layout, "button").unwrap();
    match &button.kind {
        native_gui::PartKind::Sprite {
            sprite, geometry, ..
        } => {
            assert_eq!(name(sprite), "GFX_mapicon_naval_mission_strike_force");
            assert_eq!(geometry.rect[..2], [0.0, 0.0]);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(sprite_frame(button), 1);
    match &part(&layout, "count").unwrap().kind {
        // Whole-pixel centering places the 7-pixel glyph at x 14.
        native_gui::PartKind::Text { glyphs, .. } => {
            assert_eq!(glyphs.len(), 1);
            assert_eq!((glyphs[0].ch, glyphs[0].at), ('2', [14.0, 23.0]));
        }
        other => panic!("{other:?}"),
    }
    assert!(part(&layout, "pride").is_some());
    assert_eq!(report.authored_naval_mission_buttons, 0);
}

/// A counted kind with none active shows `active/total` in the font's `R` colour, the whole
/// string bound with its reset; an undecided count, pride or alert is hidden and counted.
#[test]
fn counts_colour_and_undecided_children() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    // Mission display writes the whole string, including its colour reset.
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

/// Undefined runtime sprite names retain the button's authored drawable and count by name.
#[test]
fn an_undefined_kind_sprite_keeps_the_authored_one() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

/// Replaced sprites lose authored rotation; selecting the current sprite retains it.
#[test]
fn a_replaced_kind_sprite_drops_the_authored_rotation() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let source = synthetic_with(
        "quadTextureSprite = \"GFX_synthetic_mission_button\"",
        "quadTextureSprite = \"GFX_mapicon_naval_mission_patrol\" rotation = 0.5",
    );
    let rotation = |kind| {
        let layout = laid_entry_with(
            &source,
            &fixture_entry(kind, Ok(None)),
            &Res::new(),
            &mut CounterReport::default(),
        );
        match part(&layout, "button").unwrap().kind {
            native_gui::PartKind::Sprite { geometry, .. } => geometry.rotation,
            ref other => panic!("{other:?}"),
        }
    };
    assert_eq!(rotation(2), 0.0);
    assert_eq!(rotation(1), 0.5);
}

/// A mod's progress-bar button takes mission frame 2 as its value.
#[test]
fn a_progress_button_takes_frame_two_as_its_value() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut res = Res::new();
    res.progress.push("GFX_mapicon_naval_mission_strike_force");
    let layout = laid_entry(
        &fixture_entry(2, Ok(None)),
        &res,
        &mut CounterReport::default(),
    );
    match &part(&layout, "button").unwrap().kind {
        native_gui::PartKind::Progress { sprite, state, .. } => {
            assert_eq!(name(sprite), "GFX_mapicon_naval_mission_strike_force");
            assert_eq!(*state, 2.0 / 100.0);
        }
        other => panic!("{other:?}"),
    }
}

/// Vertical frame strips sample down the texture; button frame 2 selects the second row.
#[test]
fn a_vertical_kind_strip_draws_its_second_row() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut res = Res::new();
    res.vertical.push("GFX_mapicon_naval_mission_strike_force");
    let layout = laid_entry(
        &fixture_entry(2, Ok(None)),
        &res,
        &mut CounterReport::default(),
    );
    match part(&layout, "button").unwrap().kind {
        native_gui::PartKind::Sprite { offsets, .. } => {
            assert_eq!(offsets[0], 0.0);
            assert!(offsets[1] > 0.0, "{offsets:?}");
        }
        ref other => panic!("{other:?}"),
    }
}

/// Accepted keys preserve the icon and later children: the reader skips 3dposition
/// as a whole and retains context_aware_tooltip for the tooltip.
#[test]
fn native_reader_keys_keep_the_entrys_later_children() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

/// Mission entries support text in scaled containers.
#[test]
fn a_text_in_a_scaled_entry_container_draws() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let source = synthetic_with(
        "iconType = {
			name = \"pride\"",
        "containerWindowType = { name = \"scaled\" size = { width = 40 height = 20 }              scale = 0.5 instantTextboxType = { name = \"note\" font = \"synthetic_font\"              text = \"7\" maxWidth = 20 maxHeight = 14 } }
		iconType = {
			name = \"pride\"",
    );
    let layout = laid_entry_with(
        &source,
        &fixture_entry(1, Ok(Some(2))),
        &Res::new(),
        &mut CounterReport::default(),
    );
    match &part(&layout, "note").expect("the scaled text draws").kind {
        native_gui::PartKind::Text { glyphs, scale, .. } => {
            assert_eq!(glyphs.len(), 1);
            assert_eq!(*scale, 0.5);
        }
        other => panic!("{other:?}"),
    }
}

/// Inline icons reached through a mission window's localised text join its sprite assets.
#[test]
fn inline_icons_in_mission_texts_are_acquired() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let source = synthetic_with("text = \"100%\"", "text = \"EFFICIENCY_KEY\"");
    let windows = windows_of(&source);
    let localisation: installed_content::localisation::Localisation =
        [("EFFICIENCY_KEY".to_owned(), "£mission_icon 5".to_owned())]
            .into_iter()
            .collect();
    let registry = native::resources::Registry::default();
    let assets = mission_assets(&windows, &registry, &localisation);
    assert!(assets.sprites.contains(b"GFX_mission_icon".as_slice()));
}

/// Forms the removed per-widget layout refused now lay out as the game lays them out: a
/// right-aligned list, an entry that clips, and an entry holding another element and a
/// control nested in a container.
#[test]
fn modded_forms_lay_out_rather_than_being_refused() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

/// `SYNTHETIC_MISSION_GUI` with `extra` on the entry window, after its `position` (which the
/// reader also takes as the show and hide positions).
fn entry_with(extra: &str) -> String {
    let buttons = "orientation = upper_left\n\n\t\tbuttonType";
    synthetic_with(
        buttons,
        &format!("orientation = upper_left {extra}\n\n\t\tbuttonType"),
    )
}

/// The buttons of the one live icon's drawn entries, in paint order.
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

/// Clearing the list detaches no entry. Removed entries retain their places and draw
/// below listed entries until their Hide transition finishes. Listed fading entries
/// restart fades each frame through Hide then Show; a long frame reaches full opacity
/// before removal, so fade-out starts at 1. Hide movement already in progress continues
/// because Show does not stop it.
#[test]
fn an_outgoing_entry_draws_until_its_hide_ends() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    for (extra, listed) in [
        ("fade_type = linear fade_time = 1000", 2.0),
        (
            "hide_animation_type = linear hide_animation_time = 1000 \
             hide_position = { x = 0 y = 50 }",
            0.1,
        ),
    ] {
        let res = Res::new();
        let mut harness = Harness::new(&entry_with(extra));
        let two = kinds(2);
        harness.frame(&res, &[(1, Some(&two), [0, 0])], 0.0);
        harness.frame(&res, &[(1, Some(&two), [0, 0])], listed);
        let one = kinds(1);
        harness.frame(&res, &[(1, Some(&one), [0, 0])], 0.01);
        assert_eq!(
            drawn_buttons(&mut harness, &one),
            [MISSION_SPRITES[1].1, MISSION_SPRITES[0].1],
            "{extra}"
        );
        assert!(harness.windows.take_active());
        for _ in 0..11 {
            harness.frame(&res, &[(1, Some(&one), [0, 0])], 0.1);
        }
        assert_eq!(
            drawn_buttons(&mut harness, &one),
            [MISSION_SPRITES[0].1],
            "{extra}"
        );
    }
    // A window that neither fades nor moves is hidden at once.
    let res = Res::new();
    let mut harness = Harness::new(SYNTHETIC_MISSION_GUI);
    let two = kinds(2);
    harness.frame(&res, &[(1, Some(&two), [0, 0])], 0.0);
    let one = kinds(1);
    harness.frame(&res, &[(1, Some(&one), [0, 0])], 0.01);
    assert_eq!(drawn_buttons(&mut harness, &one), [MISSION_SPRITES[0].1]);
}

/// Shown sprite clocks advance before window transitions, including the frame when
/// a root finishes hiding its entries. Show retains that phase for the pooled slot's
/// next owner. The round-robin cursor returns to the freed slot after other slots fill.
#[test]
fn an_entry_hidden_by_its_roots_ended_fade_keeps_that_frames_clock() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let source = entry_with(
        "iconType = { name = \"anim\" spriteType = \"GFX_anim\" position = { x = 0 y = 0 } }",
    )
    .replacen(
        "name = \"naval_mission_mapicon\"",
        "name = \"naval_mission_mapicon\" fade_type = linear fade_time = 1000",
        1,
    );
    let mut res = Res::new();
    // Two frames of 0.25 s: the loop is 0.5 s.
    res.animated.push(("GFX_anim", 0.25));
    let mut harness = Harness::new(&source);
    let one = kinds(1);
    let slots = MapIconPool::Mission.initial() as u32;
    // Every region reached; the first gated as `first` says, the others holding `one`.
    let owners = |first: bool| {
        (1..=slots)
            .map(|region| (region, (first || region != 1).then_some(&one[..]), [0, 0]))
            .collect::<Vec<Owner<'_>>>()
    };
    harness.frame(&res, &owners(true), 0.0);
    let slot = harness
        .windows
        .missions
        .pool
        .as_ref()
        .unwrap()
        .slot_of(&1)
        .unwrap();
    // Fully faded in: 2 s of entry clock.
    harness.frame(&res, &owners(true), 2.0);
    // Erased: its 1 s fade out runs, the entry still shown (2.5 s).
    harness.frame(&res, &owners(false), 0.5);
    fn entry(harness: &Harness, slot: SlotId) -> &Instance {
        &harness.windows.missions.icons[&slot].entries[0]
    }
    assert!(entry(&harness, slot).drawn());
    // The fade ends in this frame's transitions, after the entry's clock took its 0.75 s.
    harness.frame(&res, &owners(false), 0.75);
    assert!(!entry(&harness, slot).drawn());
    // The same slot again: the entry shown at 3.25 s, frame 13 of the loop (without the
    // frame the hide ended, 2.5 s: frame 10).
    harness.frame(&res, &owners(true), 0.0);
    let pool = harness.windows.missions.pool.as_ref().unwrap();
    assert_eq!(pool.slot_of(&1), Some(slot));
    assert!(entry(&harness, slot).drawn());
    let windows = MissionWindows::find(&harness.native).unwrap();
    let layout = entry(&harness, slot)
        .layout(windows.entry, &res, None)
        .unwrap()
        .unwrap();
    assert_eq!(sprite_frame(part(&layout, "anim").unwrap()), 1);
}

/// The count of the gaps whose reason contains `what`, taken from the report.
fn gaps(harness: &mut Harness, what: &str) -> usize {
    std::mem::take(&mut harness.report)
        .naval_mission_gaps
        .iter()
        .filter(|(reason, _)| reason.contains(what))
        .map(|(_, count)| count)
        .sum()
}

/// Pride visibility follows each record's registrations. Undecidable registrations
/// make no visibility write: retain the marker, withhold and count the entry, and
/// repair it only with a decided write.
#[test]
fn an_undecided_pride_leaves_its_entry_unknown_until_a_decided_write() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

/// Unrecorded pool-growth timing can change round-robin slot ownership. A later slot
/// may retain another region's history: entries wait for their next update and a
/// listing's count/place writes ([`ENTRY_KEPT`]); icons wait for a shown entry to write
/// the list's place ([`LIST_PLACE`]).
#[test]
fn a_slot_allocated_after_a_divergence_leaves_what_an_update_may_keep_unknown() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    const MOVED: &str = "its pool slot may not be the game's";
    let res = Res::new();
    // An entry with a hide movement moves out from the list's first Hide, listed or not,
    // and draws until the movement ends.
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
    // One region more than the pool holds: its request starts the growth.
    let mut more = full.clone();
    more.push((slots + 1, Some(&two[..]), [0, 0]));
    harness.frame(&res, &more, 0.01);
    assert_eq!(
        harness.windows.missions.diverged.as_deref(),
        Some(naval_windows::GROWTH_FRAME)
    );
    std::mem::take(&mut harness.report);
    // Region 1 leaves; the region asking next takes its freed slot and lists one entry, and
    // the entry the slot's last update listed moves out under it.
    let mut next = full.clone();
    next[0].1 = None;
    next.push((slots + 1, Some(&one[..]), [0, 0]));
    harness.frame(&res, &next, 0.01);
    let pool = harness.windows.missions.pool.as_ref().unwrap();
    assert_eq!(pool.slot_of(&(slots + 1)), first);
    // Both entries are withheld: the one it lists for its fade and movement, which its hide
    // movement keeps unknown (`TRANSITION`), the other for its count and place too.
    let laid = harness.laid(&res, &two);
    assert_eq!(laid.iter().filter(|(_, icon)| icon.entries == 0).count(), 1);
    assert_eq!(gaps(&mut harness, MOVED), 2);
    // A new region that shows no entry leaves its icon's list place unknown.
    next.push((slots + 2, Some(&none[..]), [0, 0]));
    harness.frame(&res, &next, 0.01);
    harness.laid(&res, &two);
    let icon_moved = format!("{MISSION_WINDOW} is not drawn: {MOVED}");
    assert_eq!(gaps(&mut harness, &icon_moved), 1);
}

/// Pool divergence or a failed tick can leave unknown transition history. Fade and
/// movement ([`TRANSITION`]) withhold the window until it stays out of transitions
/// for the longest registered duration ([`Instance::longest_transition`]). Windows
/// without transitions become known immediately.
#[test]
fn a_mission_windows_unknown_transition_settles_as_the_other_naval_windows_do() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fading = synthetic_with(
        "name = \"naval_mission_mapicon\"",
        "name = \"naval_mission_mapicon\" fade_type = linear fade_time = 500",
    );
    let res = Res::new();
    let one = kinds(1);
    let transition = |harness: &mut Harness| gaps(harness, naval_windows::TRANSITION);
    // After a divergence: the newly allocated icon is withheld until it has been out of a
    // transition for its longest, 0.5 s, then drawn.
    let mut harness = Harness::new(&fading);
    harness.windows.missions.diverged = Some("a test".into());
    harness.frame(&res, &[(1, Some(&one), [0, 0])], 0.0);
    let mut withheld = 0;
    for _ in 0..20 {
        if drawn_buttons(&mut harness, &one).is_empty() {
            assert_eq!(transition(&mut harness), 1);
            withheld += 1;
        } else {
            assert_eq!(transition(&mut harness), 0);
            break;
        }
        harness.frame(&res, &[(1, Some(&one), [0, 0])], 0.1);
    }
    assert!((5..20).contains(&withheld), "{withheld}");
    // The entries have no transition: known at once, never withheld for it.
    let mut harness = Harness::new(SYNTHETIC_MISSION_GUI);
    harness.windows.missions.diverged = Some("a test".into());
    harness.frame(&res, &[(1, Some(&one), [0, 0])], 0.0);
    assert_eq!(drawn_buttons(&mut harness, &one).len(), 1);
    assert_eq!(transition(&mut harness), 0);
    // A tick that fails (here asked about a root of another reading, which the lifecycle
    // refuses) leaves every mission window's transition unknown: the fading icon is withheld.
    let mut harness = Harness::new(&fading);
    harness.frame(&res, &[(1, Some(&one), [0, 0])], 0.0);
    harness.frame(&res, &[(1, Some(&one), [0, 0])], 1.0);
    assert_eq!(drawn_buttons(&mut harness, &one).len(), 1);
    let content = "\t\tcontainerWindowType = {\n\t\t\tname = \"content\"";
    let other = windows_of(&fading.replacen(
        content,
        &format!(
            "iconType = {{ name = \"extra\" spriteType = \"GFX_x\" }}
{content}"
        ),
        1,
    ));
    let native = harness.native.clone();
    let own = MissionWindows::find(&native).unwrap();
    let foreign = MissionWindows::find(&other).unwrap();
    let windows = MissionWindows {
        root: foreign.root,
        ..own
    };
    let tick = harness.tick(0.1);
    naval_windows::tick(
        &mut harness.windows,
        &native,
        Some(&windows),
        &res,
        tick,
        &mut harness.report,
    );
    assert!(!harness.report.naval_window_notes.is_empty());
    std::mem::take(&mut harness.report);
    assert!(drawn_buttons(&mut harness, &one).is_empty());
    assert_eq!(transition(&mut harness), 1);
}

/// A frame whose own calls stop part-way (here an erasure's Hide asked about a root of another
/// reading, which the lifecycle refuses) leaves the icons it did not reach as they were, and
/// fails the same way every frame: every icon is withheld and counted until the windows are
/// built again, though later updates make all their calls.
#[test]
fn a_frame_that_stops_withholds_every_icon_until_rebuilt() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    const STOPPED: &str = "its frame does not update";
    let res = Res::new();
    let two = kinds(2);
    let both = [(1, Some(&two[..]), [0, 0]), (2, Some(&two[..]), [0, 0])];
    let mut harness = Harness::new(SYNTHETIC_MISSION_GUI);
    harness.frame(&res, &both, 0.0);
    assert_eq!(drawn_buttons(&mut harness, &two).len(), 4);
    // Another reading whose root holds one more element.
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
            point: [0, 0],
        },
        Reached {
            region: 2,
            entries: Some(&two),
            point: [0, 0],
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

/// Failed mission calls stop an update part-way. Structural failures, such as a list
/// from a different file reading, withhold and count the icon and its drawn entries
/// until reconstruction; later successful updates cannot repair that failure.
#[test]
fn a_structural_refusal_withholds_the_icon_and_its_entries_until_rebuilt() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        point: [0, 0],
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
    // Complete updates follow; the refusal stands.
    for _ in 0..3 {
        harness.frame(&res, &[(1, Some(&two), [0, 0])], 0.01);
    }
    assert!(drawn_buttons(&mut harness, &two).is_empty());
    assert_eq!(gaps(&mut harness, REFUSED), 1);
    // The windows built again (a new registry): drawn.
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

/// Each update's Show lays out listed entries. Refused parent layout withholds and
/// counts those entries until an update lists and lays them out again. Clearing moves
/// nothing, so entries removed first remain withheld throughout their Hide transition.
#[test]
fn a_list_placement_refusal_withholds_the_entries_until_they_are_placed_again() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    // The update's root layout is refused: the three entries' places are unknown. A long
    // frame brings them to full opacity (a listed fading entry restarts its fade each
    // update, the list's Hide then its Show), so the two that leave next fade out from 1.
    harness.frame(&refusing, &[(1, Some(&three), [0, 0])], 2.0);
    assert!(drawn_buttons(&mut harness, &three).is_empty());
    assert_eq!(gaps(&mut harness, UNPLACED), 3);
    // The patrol entry is placed again; the other two leave the list and fade out
    // withheld.
    harness.frame(&res, &[(1, Some(&one), [0, 0])], 0.01);
    assert_eq!(drawn_buttons(&mut harness, &one), [MISSION_SPRITES[0].1]);
    assert_eq!(gaps(&mut harness, UNPLACED), 2);
    // Listed and placed again: every entry draws.
    harness.frame(&res, &[(1, Some(&three), [0, 0])], 0.01);
    assert_eq!(drawn_buttons(&mut harness, &three).len(), 3);
    assert_eq!(gaps(&mut harness, UNPLACED), 0);
}

/// Every mission record receives alert, efficiency and pride writes, listed or not.
/// Without registrations, patrol alerts; raiding alerts with zero efficiency; escort
/// remains undecided; other kinds show nothing. Buttons first take frame 1, then
/// counted kinds take frame 2.
#[test]
fn unlisted_records_take_the_code_writes_over_no_registration() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    // On the windows: the raiding entry, dropped from the list, keeps its efficiency shown.
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

/// Crossing the cutoff erases and hides the icon and its listed entries. Returning
/// selects the pool cursor's slot and shows it, restarting a fading root's fade.
#[test]
fn crossing_the_cutoff_erases_and_shows_again() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    // Above the cutoff: the gate fails for every region.
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
    // Below it again.
    harness.frame(&res, &[(4, Some(&two), [10, 20])], 0.0);
    let pool = harness.windows.missions.pool.as_ref().unwrap();
    let (_, again, window) = pool.live().next().unwrap();
    assert_ne!(again, first);
    assert!(window.drawn());
    assert_eq!(window.position(), [10, 20]);
    assert_eq!(drawn_buttons(&mut harness, &two).len(), 2);
    // A fading root fades in again from nothing.
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
    // Fully faded in, then erased: its fade out runs, but it is out of the manager's visible
    // list, so nothing draws.
    harness.frame(&res, &[(4, Some(&two), [10, 20])], 2.0);
    harness.frame(&res, &[(4, None, [10, 20])], 0.1);
    assert_eq!(opacity(&harness), None);
    assert!(harness.windows.take_active());
    assert!(harness.laid(&res, &two).is_empty());
    harness.frame(&res, &[(4, Some(&two), [10, 20])], 0.0);
    assert!(opacity(&harness).unwrap() < 0.01);
}

/// Frames reach regions at their first province in the shared update list
/// ([`Schedule::listed`]): visible provinces and dirty off-screen provinces. Regions
/// leaving view retain updates until the schedule cleans each province, then sweep
/// their icons without a refusal count.
#[test]
fn a_region_out_of_view_is_reached_until_its_provinces_are_clean() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let provinces = |region: u32| -> &'static [u32] {
        match region {
            4 => &[140, 141],
            5 => &[150],
            _ => &[],
        }
    };
    let all = [140, 141, 150];
    let mut schedule = Schedule::default();
    // Settled (every province out of view updated once, and so listed that frame), then a
    // frame at the same camera: only the provinces in view are listed.
    schedule.advance(all, |province| province == 140, true);
    let candidates = || BTreeSet::from([4, 5]);
    assert_eq!(
        reach(candidates(), provinces, &schedule),
        [(4, true), (5, false)]
    );
    schedule.advance(all, |province| province == 140, false);
    assert_eq!(reach(candidates(), provinces, &schedule), [(4, true)]);
    // Region 5 comes into view: the regions follow their first listed province.
    schedule.advance(all, |province| province == 140 || province == 150, false);
    assert_eq!(
        reach(candidates(), provinces, &schedule),
        [(4, true), (5, true)]
    );
    // The camera leaves both. Their provinces stay dirty, so both are still reached, out
    // of view, until each province's phase turn clears it: 140 at phase 40, 150 at 50.
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
    // Three advances came before these (phases 1 to 3): frame 37 is phase 40, whose update
    // of 140 out of view still lists it, so region 4 leaves at frame 38, region 5 at 48.
    assert_eq!(left, BTreeMap::from([(4, 38), (5, 48)]));
    assert!(!schedule.listed(140) && !schedule.listed(141) && !schedule.listed(150));

    // On the windows: an icon kept while its region is reached out of view is not counted
    // as a limitation, and its sweep erases it.
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

/// Moving windows request frames from their first zero-duration tick until movement settles.
#[test]
fn a_moving_root_keeps_frames_coming_until_it_settles() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    // After the root's `position`, which the reader also takes as its show position.
    let content = "\t\tcontainerWindowType = {\n\t\t\tname = \"content\"";
    let source = synthetic_with(
        content,
        &format!(
            "size = {{ width = 10 height = 10 }} show_animation_type = linear \
             show_animation_time = 100 show_position = {{ x = 50 y = 0 }}\n{content}"
        ),
    );
    let res = Res::new();
    let mut harness = Harness::new(&source);
    let none: [MissionEntry; 0] = [];
    harness.frame(&res, &[(4, Some(&none), [0, 0])], 0.0);
    assert!(harness.windows.take_active());
    for _ in 0..5 {
        harness.frame(&res, &[(4, Some(&none), [0, 0])], 0.05);
    }
    harness.frame(&res, &[(4, Some(&none), [0, 0])], 0.05);
    assert!(!harness.windows.take_active());
}

/// Percentage roots lay out in the GUI viewport. Nested entry windows receive Show
/// and fade on their own clocks.
#[test]
fn descendants_and_percentage_roots_run_on_the_instances() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let source = synthetic_with(
        "name = \"naval_mission_mapicon\"",
        "name = \"naval_mission_mapicon\" size = { width = 10% height = 5% }",
    )
    .replacen(
        "iconType = {\n\t\t\tname = \"alert\"",
        "containerWindowType = { name = \"nested\" size = { width = 10 height = 10 } \
         fade_type = linear fade_time = 1000 \
         iconType = { name = \"badge\" spriteType = \"GFX_x\" } }\n\t\ticonType = {\n\t\t\tname = \"alert\"",
        1,
    );
    let res = Res::new();
    let mut harness = Harness::new(&source);
    let one = kinds(1);
    harness.frame(&res, &[(1, Some(&one), [0, 0])], 0.0);
    let laid = harness.laid(&res, &one);
    assert_eq!(laid[0].1.layout.size, [192, 54]);
    let badge = laid[0]
        .1
        .items
        .iter()
        .flatten()
        .find(|part| part.path.ends_with("badge"))
        .expect("the nested window's icon draws");
    // Shown this frame: its fade has begun with the minimum one-millisecond step.
    assert!(badge.color[3] < 0.01, "{}", badge.color[3]);
    harness.frame(&res, &[(1, Some(&one), [0, 0])], 0.25);
    assert!(harness.windows.take_active());
}

/// A windowType entry creates a fixed window. Reading it as a container reaches
/// uninitialized callback storage and is refused by name and counted.
#[test]
fn a_fixed_window_entry_is_refused_by_name() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    // The diagnostic stays on one line without source line breaks.
    assert!(!refusal.detail.contains("  "), "{}", refusal.detail);
    assert!(
        refusal
            .detail
            .contains("the icon's construction reads it as a container window"),
        "{}",
        refusal.detail
    );
}

/// Political mode clears grids without hiding their backgrounds. Counted containers
/// are hidden because only naval mode supplies their counts.
#[test]
fn political_mode_keeps_the_empty_grids_and_hides_the_counted_containers() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    // Every grid holds its (empty) items where the game would add them.
    for (name, _, _) in GRIDS {
        assert!(
            icon.layout
                .holders
                .iter()
                .any(|holder| holder.path.ends_with(name)),
            "{name}"
        );
    }
}

/// The modes that fill a grid or count a container name the inputs no recording holds.
#[test]
fn naval_modes_name_the_inputs_they_need() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let windows = windows_of(SYNTHETIC_MISSION_GUI);
    let windows = MissionWindows::find(&windows).unwrap();
    let refused = |windows: &MissionWindows<'_>, mode| {
        let mut root = Instance::construct(windows.root, VIEWPORT, &Res::new()).unwrap();
        content_calls(windows, mode, &mut root)
            .err()
            .map(|refusals| refusals[0].to_string())
    };
    let reason = refused(&windows, 5).expect("mode 5 fills the own unit grid");
    assert!(reason.contains("unit_grid") && reason.contains("own task forces"));
    let reason = refused(&windows, NAVAL_UI_MODE).expect("the naval map mode fills the grids");
    assert!(reason.contains("unit_grid"), "{reason}");
    let grids = SYNTHETIC_MISSION_GUI.find("gridBoxType").unwrap()
        ..SYNTHETIC_MISSION_GUI
            .find("# synthetic status controls")
            .unwrap();
    let mut without_grids = SYNTHETIC_MISSION_GUI.to_owned();
    without_grids.replace_range(grids, "");
    let windows = windows_of(&without_grids);
    let windows = MissionWindows::find(&windows).unwrap();
    let reason =
        refused(&windows, NAVAL_UI_MODE).expect("the naval map mode counts the containers");
    assert!(reason.contains("mine values"), "{reason}");
    assert_eq!(refused(&windows, DRAWN_UI_MODE), None);
}

/// Absent mission windows are named; undrawn entries are counted under their reasons;
/// unanchored icons and undefined sprites are named and counted.
#[test]
fn gaps_and_unanchored_icons_are_named_and_counted() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
