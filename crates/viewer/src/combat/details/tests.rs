use super::*;
use crate::combat::binding::NoIcons;

#[test]
fn naval_disengage_requires_viewing_country_on_a_primary_side() {
    let mut detail = replay::rows::NavalCombatDetailsRow::default();
    detail.sides[0].summary.countries = vec![12];
    detail.sides[1].summary.countries = vec![56];
    let aliases = BTreeMap::from([(12, 340), (56, 560)]);
    let key = "header_container/disengage";
    let mut out = Bindings::default();
    for participant in [340, 560] {
        bind_naval_disengage(
            &mut out,
            7,
            naval_disengage_state(&detail, &aliases, participant),
        );
        assert!(!out.values[key].hidden);
    }
    // A spectator's allied/shared vision is irrelevant to 1417D4650.
    bind_naval_disengage(&mut out, 7, naval_disengage_state(&detail, &aliases, 999));
    assert!(out.values[key].hidden);
    assert!(out.refused().is_empty());
    // A proven defender match survives an unresolved attacker alias.
    detail.sides[0].summary.countries.push(78);
    bind_naval_disengage(&mut out, 7, naval_disengage_state(&detail, &aliases, 560));
    assert!(!out.values[key].hidden);
    bind_naval_disengage(&mut out, 7, naval_disengage_state(&detail, &aliases, 999));
    assert!(out.values[key].hidden);
    assert_eq!(out.refused().values().sum::<usize>(), 1);
    assert!(
        out.refused()
            .keys()
            .next()
            .unwrap()
            .contains("attacker primary country 78")
    );
    detail.sides[0].summary.countries.clear();
    detail.sides[1].summary.countries.clear();
    assert_eq!(naval_disengage_state(&detail, &aliases, 999), Ok(None));
}

#[test]
fn naval_disengage_uses_the_matched_sides_retreat_state() {
    let mut detail = replay::rows::NavalCombatDetailsRow::default();
    detail.sides[0].summary.countries = vec![12];
    detail.sides[1].summary.countries = vec![56];
    let aliases = BTreeMap::from([(12, 340), (56, 560)]);
    let key = "header_container/disengage";
    let mut out = Bindings::default();
    for side in 0..2 {
        for retreating in [0, 1, 2] {
            detail.sides[side].retreating = retreating;
            detail.sides[1 - side].retreating = u8::from(retreating == 0);
            bind_naval_disengage(
                &mut out,
                7,
                naval_disengage_state(&detail, &aliases, [340, 560][side]),
            );
            assert!(!out.values[key].hidden);
            assert_eq!(out.values[key].enabled, Some(retreating == 0));
        }
    }
    // 1417D4650 returns the attacker if both primary lists match.
    detail.sides[0].summary.countries.push(56);
    detail.sides[0].retreating = 1;
    detail.sides[1].retreating = 0;
    assert_eq!(
        naval_disengage_state(&detail, &aliases, 560),
        Ok(Some(false))
    );
    // A known defender match does not settle which retreat byte applies
    // if an unresolved attacker alias could match and the states differ.
    detail.sides[0].summary.countries = vec![78];
    bind_naval_disengage(&mut out, 7, naval_disengage_state(&detail, &aliases, 560));
    assert!(out.values[key].hidden);
    assert_eq!(out.values[key].enabled, None);
    assert_eq!(out.refused().values().sum::<usize>(), 1);
}

#[test]
fn combat_raster_initiative_border_rasterises_before_display_stretch() {
    use crate::map::native_gui::{
        self,
        lifecycle::Tick,
        tests::{fixture, windows},
    };
    use crate::map::ui_scale::{
        Setting, extent,
        tests::{MONITOR, borderless},
    };
    let (raster, note) = Setting::MatchGame.resolve_raster(Ok(&borderless()), Ok(&[MONITOR]));
    assert!(note.is_none());
    let pixels = raster.target(MONITOR.physical);
    let mut resources = fixture();
    let sprite = resources.sprites.get_mut("GFX_even").unwrap();
    // Synthetic frame strip and land-control placement. No game artwork.
    sprite.texture = [174, 118];
    sprite.frames = 2;
    sprite.hit = native_gui::Hit::Through;
    sprite.kind = native_gui::SpriteKind::FrameAnimated {
        seconds_per_frame: 2. / 3.,
        looping: true,
        play_on_show: true,
        pause: 0.,
    };
    let windows = windows(br#"guiTypes={containerWindowType={name=landcombatview
        size={width=560 height=600} clipping=no
        iconType={name=attacker_leader_initiative_overlay_glow position={x=4 y=108} spriteType=GFX_even}
        containerWindowType={name=attacker_window
            containerWindowType={name=committed_window gridBoxType={name=list slotsize={width=1 height=1} max_slots_horizontal=1}}
            containerWindowType={name=reserves_window gridBoxType={name=list slotsize={width=1 height=1} max_slots_horizontal=1}}}
        containerWindowType={name=defender_window
            containerWindowType={name=committed_window gridBoxType={name=list slotsize={width=1 height=1} max_slots_horizontal=1}}
            containerWindowType={name=reserves_window gridBoxType={name=list slotsize={width=1 height=1} max_slots_horizontal=1}}}}"#);
    let mut gui = super::super::gui::Windows::default();
    let drawn = gui
        .draw(
            &windows,
            &resources,
            &Default::default(),
            1,
            crate::combat::Selection::Land(1),
            [0.; 2],
            false,
            &mut None,
            extent(pixels, raster.ui_scale),
            false,
            Tick { time: 0., dt: 0. },
            false,
        )
        .unwrap();
    assert!(drawn.refused.is_empty(), "{:?}", drawn.refused);
    let glow = drawn.batch.quads[0].exact.as_ref().unwrap();
    assert_eq!(glow.scissor, None);
    assert_eq!(glow.uv, [[0., 0.], [0.5, 0.], [0.5, 1.], [0., 1.]]);
    let corners = glow
        .corners
        .map(|corner| corner.map(|value| value * raster.ui_scale));
    assert_eq!(
        corners,
        [[4., 108.], [91., 108.], [91., 226.], [4., 226.]],
        "14222CB90 geometry must rasterise in the native frame, before desktop stretch"
    );
    assert_eq!(pixels, [2048, 1152]);
    assert_eq!(raster.screen_scale(), 1.25);
}

#[test]
fn combat_raster_active_row_ellipsis_and_thin_glyphs() {
    use crate::map::native_gui::{
        Resources,
        lifecycle::Tick,
        tests::{fixture, windows},
    };
    let windows = windows(br#"guiTypes={
        containerWindowType={name=landcombatview size={width=650 height=400}
            containerWindowType={name=attacker_window size={width=300 height=300}
                containerWindowType={name=committed_window size={width=280 height=175} clipping=yes
                    gridBoxType={name=list position={x=31 y=10} size={width=221 height=140}
                        slotsize={width=244 height=40} max_slots_horizontal=1}}
                containerWindowType={name=reserves_window position={x=0 y=200} size={width=280 height=100}
                    gridBoxType={name=list size={width=270 height=80} slotsize={width=119 height=26} max_slots_horizontal=2}}}
            containerWindowType={name=defender_window position={x=320 y=0} size={width=300 height=300}
                containerWindowType={name=committed_window size={width=280 height=175} clipping=yes
                    gridBoxType={name=list position={x=10 y=10} size={width=221 height=140}
                        slotsize={width=244 height=40} max_slots_horizontal=1}}
                containerWindowType={name=reserves_window position={x=0 y=200} size={width=280 height=100}
                    gridBoxType={name=list size={width=270 height=80} slotsize={width=119 height=26} max_slots_horizontal=2}}}}
        containerWindowType={name=division_combat_attacker_entry size={width=244 height=43}
            instantTextBoxType={name=name position={x=48 y=6} font=Main_14 maxWidth=153 maxHeight=16 fixedsize=yes}}
        containerWindowType={name=division_combat_defender_entry size={width=244 height=43}
            instantTextBoxType={name=name position={x=64 y=6} font=Main_14 maxWidth=153 maxHeight=16 fixedsize=yes format=right}}
        containerWindowType={name=reserve_combat_entry size={width=119 height=26}
            instantTextBoxType={name=name font=Main_14 maxWidth=74 maxHeight=16 fixedsize=yes}}
    }"#);
    let mut resources = fixture();
    // Independently authored proportional font: a one-texel i and a one-row
    // dot, unlike the broad monospace diagnostic that masked the raster bug.
    resources.font = crate::map::native_text::monospace("Main_14", 5, 14);
    for (ch, width, height, advance, yoffset) in
        [('i', 1, 8, 2, 1), ('.', 2, 1, 3, 10), (' ', 0, 0, 3, 0)]
    {
        let glyph = resources.font.glyphs.glyphs.get_mut(&(ch as u32)).unwrap();
        glyph.width = width;
        glyph.height = height;
        glyph.xadvance = advance;
        glyph.yoffset = yoffset;
    }
    let (raster, note) = crate::map::ui_scale::Setting::MatchGame.resolve_raster(
        Ok(&crate::map::ui_scale::tests::borderless()),
        Ok(&[crate::map::ui_scale::tests::MONITOR]),
    );
    assert!(note.is_none());
    let pixels = raster.target([2560, 1440]);
    let extent = crate::map::ui_scale::extent(pixels, raster.ui_scale);
    let flag = "GER_group".to_owned();
    let mut bindings = Bindings::default();
    for side in 0..4 {
        let row = LandCombatDivisionRow {
            side: (side % 2) as u8,
            ..Default::default()
        };
        let mut values = entry(
            &row,
            Some(format!("MMMMMi{}", "M".repeat(23))),
            side < 2,
            None,
            Some(&flag),
            Values::new(),
        );
        values.retain(|key, _| key == "name");
        bindings.entries[side].push((side as u64, values));
    }
    let dot = resources.font(b"Main_14").unwrap().quad('.').unwrap().uv;
    let mut gui = super::super::gui::Windows::default();
    for generation in [1, 1, 2] {
        let drawn = gui
            .draw(
                &windows,
                &resources,
                &bindings,
                generation,
                super::super::Selection::Land(1),
                [0.; 2],
                false,
                &mut None,
                extent,
                false,
                Tick { time: 0., dt: 0. },
                false,
            )
            .unwrap();
        assert!(drawn.refused.is_empty(), "{:?}", drawn.refused);
        let dots = drawn
            .batch
            .quads
            .iter()
            .filter(|quad| {
                quad.exact
                    .as_ref()
                    .is_some_and(|exact| exact.uv[0] == [dot[0], dot[1]])
            })
            .count();
        assert_eq!(
            dots, 12,
            "three native dots on each active/reserve row, including retained refreshes"
        );
        let thin = resources.font.quad('i').unwrap().uv;
        let mut thin_count = 0;
        for quad in &drawn.batch.quads {
            let exact = quad.exact.as_ref().unwrap();
            let is_dot = exact.uv[0] == [dot[0], dot[1]];
            let is_thin = exact.uv[0] == [thin[0], thin[1]];
            if !is_dot && !is_thin {
                continue;
            }
            thin_count += usize::from(is_thin);
            let corners = exact
                .corners
                .map(|corner| corner.map(|value| value * raster.ui_scale));
            assert_eq!(corners[1][0] - corners[0][0], if is_dot { 2. } else { 1. });
            assert_eq!(corners[2][1] - corners[1][1], if is_dot { 1. } else { 8. });
            assert!(corners.iter().flatten().all(|value| value.fract() == 0.));
        }
        assert_eq!(
            thin_count, 4,
            "the i reaches draw on both active and both reserve rows"
        );
        assert_eq!(
            pixels,
            [2048, 1152],
            "desktop stretch follows preparation and rasterisation"
        );
    }
}

#[test]
fn naval_progress_uses_defender_list_for_both_participant_perspectives() {
    let aliases = BTreeMap::from([(12, 120), (34, 340), (56, 560)]);
    let mut detail = replay::rows::NavalCombatDetailsRow::default();
    detail.sides[0].summary.countries = vec![12, 34];
    detail.sides[1].summary.countries = vec![56];
    let mut values = Values::new();
    let attacker = naval_progress_skin(&detail, &aliases, 340).unwrap();
    bind_naval_progress(&mut values, attacker, 65_000);
    assert!(!values["header_container/progress_attacker"].hidden);
    assert!(values["header_container/progress_defender"].hidden);
    assert_eq!(
        values["header_container/progress_attacker"].fill,
        Some(0.65)
    );
    let defender = naval_progress_skin(&detail, &aliases, 560).unwrap();
    bind_naval_progress(&mut values, defender, 35_000);
    assert!(values["header_container/progress_attacker"].hidden);
    assert!(!values["header_container/progress_defender"].hidden);
    assert_eq!(
        values["header_container/progress_defender"].fill,
        Some(0.35)
    );
    assert_eq!(
        naval_progress_skin(&detail, &aliases, 999),
        Ok("progress_attacker")
    );
    detail.sides[1].summary.countries = vec![12, 78];
    assert!(naval_progress_skin(&detail, &aliases, 560).is_err());
    detail.sides[1].summary.countries = vec![340, 78];
    assert_eq!(
        naval_progress_skin(&detail, &aliases, 340),
        Err("the viewing country's combat side is unavailable: defender primary country 340 has no CountryLinks row".to_owned())
    );
    detail.sides[1].summary.countries = vec![];
    assert_eq!(
        naval_progress_skin(&detail, &aliases, 340),
        Ok("progress_attacker")
    );
}

/// The BM_ keys are the executable's: a missing one is refused, never
/// shown as its own text, and the delayed key is named. Border factors
/// wrap their entry in the stored colour byte.
#[test]
fn executable_modifier_keys_must_exist() {
    let loc: Localisation = [
        ("HEADER", "$KEY$"),
        ("BM_STACKING", "Stacking"),
        ("BORDER_CONFLICT_FACTOR_ENTRY", "$VAL|%0$"),
        ("BORDER_CONFLICT_TERRAIN_FACTOR", "Terrain $VAL$"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    let (main, delayed) = global_modifier_tooltip(&NoIcons, &loc, 5, None, None);
    assert_eq!(main, Ok(Some("Stacking".to_owned())));
    // A missing top-level key shows its own bytes (142230940).
    assert_eq!(delayed, Some(Ok("BM_STACKING_DESC".to_owned())));
    let (main, _) = global_modifier_tooltip(&NoIcons, &loc, 7, None, None);
    assert_eq!(main, Ok(Some("BM_DECRYPTION".to_owned())));
    let (main, delayed) =
        global_modifier_tooltip(&NoIcons, &loc, 8, None, Some([0, 100_000, 80_000]));
    assert_eq!(main, Ok(Some("Terrain \u{11}R80%\u{11}!".to_owned())));
    assert_eq!(delayed, None);
    let (main, _) = global_modifier_tooltip(&NoIcons, &loc, 8, None, None);
    assert!(main.unwrap_err().contains("border-war factors"));
}
const HALF: Condition = Condition {
    strength: 50_000,
    max_strength: Some(100_000),
    organisation: 30_000,
    max_organisation: Some(60_000),
};

#[test]
fn committed_strength_and_side_attack_use_distinct_native_operands() {
    let row = LandCombatDivisionRow {
        strength_combat_factor: 50_000,
        scales: [200_000, 50_000, 150_000, 100_000],
        stat_values: [8_000_000, 6_000_000, 0, 10_000_000, 2_000_000],
        ..Default::default()
    };
    let entry = |committed| {
        let mut values = entry(&row, None, committed, Some(10_000), None, Values::new());
        condition_bars(
            &mut Bindings::default(),
            &mut values,
            &row,
            committed,
            Ok(HALF),
            "",
        );
        values
    };
    let active = entry(true);
    let reserve = entry(false);
    assert_eq!(active["str_bar"].fill, Some(0.25));
    assert_eq!(reserve["str_bar"].fill, Some(0.5));
    assert_eq!(active["org_bar"].fill, Some(0.5));
    assert_eq!(active["soft_attack"].text.as_deref(), Some("330"));
    assert_eq!(active["hard_attack"].text.as_deref(), Some("66"));
    assert_eq!(active["defense"].text.as_deref(), Some("30"));
    assert_eq!(fraction(50, 0), 0.5); // Native maximum-stat clamp is 100 raw.
}

#[test]
fn missing_parent_keeps_recorded_row_values_and_truncates_live_defence() {
    // Reference07, hour60828492: 169th 'Madison' Division shows365.
    let row = LandCombatDivisionRow {
        side: 1,
        strength_combat_factor: 100_000,
        scales: [99_000, 99_000, 100_000, 100_000],
        stat_values: [36_958_320, 7_635_582, 833, 23_127_045, 5_736_750],
        ..Default::default()
    };
    let mut out = Bindings::default();
    let mut active = entry(
        &row,
        Some("169th 'Madison' Division".into()),
        true,
        None,
        None,
        Values::new(),
    );
    condition_bars(&mut out, &mut active, &row, true, Ok(HALF), "");
    assert_eq!(
        active["name"].text.as_deref(),
        Some("169th 'Madison' Division")
    );
    assert_eq!(active["str_bar"].fill, Some(0.5));
    assert_eq!(active["org_bar"].fill, Some(0.5));
    assert_eq!(active["defense"].text.as_deref(), Some("365"));
    assert_eq!(active["soft_attack"].text, None);
    assert_eq!(active["hard_attack"].text, None);
    // The same entry bound again without its name or condition keeps
    // neither earlier value: both are cleared and hidden, the bars'
    // refusal counted once per bind.
    let mut stale = entry(&row, None, true, None, None, active);
    for _ in 0..2 {
        condition_bars(
            &mut out,
            &mut stale,
            &row,
            true,
            Err("Land combat 5 division 9: bars refused.".into()),
            "",
        );
    }
    assert!(stale["name"].hidden && stale["name"].text.is_none());
    for bar in ["str_bar", "org_bar"] {
        assert!(stale[bar].hidden && stale[bar].fill.is_none(), "{bar}");
    }
    assert_eq!(
        out.refused()
            .get("Land combat 5 division 9: bars refused.")
            .copied(),
        Some(2)
    );
}

/// A division with its Divisions row but no DivisionStatus row keeps a
/// condition; each bar is hidden only for its own missing maximum, and
/// the refusal names the table's state.
#[test]
fn a_missing_status_row_hides_each_bar_by_its_own_maximum() {
    use replay::rows::DivisionRow;
    let mut frame = window_frame(None, None);
    *frame.tables[TableId::Divisions.index()] =
        TableView::Complete(TableRows::Divisions(vec![DivisionRow {
            id: 9,
            hit_points: 50_000,
            organisation: 30_000,
            ..Default::default()
        }]));
    *frame.tables[TableId::DivisionStatus.index()] = TableView::Truncated {
        rows: TableRows::DivisionStatus(Vec::new()),
        missing: 2,
    };
    let bars = conditions(&frame).unwrap();
    let status_gap = absence(&rows::<tables::DivisionStatus>(&frame).unwrap());
    let row = LandCombatDivisionRow {
        combat: COMBAT,
        division: 9,
        ..Default::default()
    };
    let mut out = Bindings::default();
    let mut values = Values::new();
    condition_bars(
        &mut out,
        &mut values,
        &row,
        false,
        Ok(bars[&9]),
        &status_gap,
    );
    for bar in ["str_bar", "org_bar"] {
        assert!(values[bar].hidden && values[bar].fill.is_none(), "{bar}");
    }
    assert!(refused(
        &out,
        "Land combat 62 division 9: its DivisionStatus row is missing (the table is truncated, 2 rows unread), so its hit points maximum is unknown; its strength bar is hidden."
    ));
    assert!(refused(
        &out,
        "so its organisation maximum is unknown; its organisation bar is hidden."
    ));
    // A known maximum binds its own bar although the other is missing.
    let mut values = Values::new();
    let strength_only = Condition {
        max_strength: Some(100_000),
        ..bars[&9]
    };
    condition_bars(
        &mut out,
        &mut values,
        &row,
        false,
        Ok(strength_only),
        &status_gap,
    );
    assert_eq!(values["str_bar"].fill, Some(0.5));
    assert!(!values["str_bar"].hidden);
    assert!(values["org_bar"].hidden && values["org_bar"].fill.is_none());
}

#[test]
fn committed_names_carry_the_owner_flag_on_the_side_the_native_entry_writes() {
    let name = || "144ya Strelkovaya Diviziya".to_owned();
    let flag = "SOV_communism".to_string();
    assert_eq!(
        entry_name(name(), 0, true, Some(&flag)),
        format!("144ya Strelkovaya Diviziya {INLINE_FLAG}SOV_communism")
    );
    assert_eq!(
        entry_name(name(), 0, false, Some(&flag)),
        "144ya Strelkovaya Diviziya"
    );
    assert_eq!(
        entry_name(name(), 1, true, Some(&flag)),
        format!("{INLINE_FLAG}SOV_communism 144ya Strelkovaya Diviziya")
    );
    assert_eq!(
        entry_name(name(), 1, true, None),
        "144ya Strelkovaya Diviziya"
    );
}

#[test]
fn names_prefer_a_custom_name_then_the_selected_group_in_the_content() {
    let mut localisation = Localisation::default();
    localisation.language = "english".into();
    let names = installed_content::division_names::parse(
        br#"SOV_INF_01 = { ordered = { 144 = { "%dya Strelkovaya Diviziya" } } }"#,
        &installed_content::key_names::KeyNames::default(),
    )
    .unwrap();
    let mut row = LandCombatDivisionRow {
        name_group: b"SOV_INF_01".to_vec(),
        name_order: 144,
        name_ordered: 1,
        ..Default::default()
    };
    assert_eq!(
        division_name(&row, &names, &localisation, "english").as_deref(),
        Ok("144ya Strelkovaya Diviziya")
    );
    row.name_custom = b"Guards".to_vec();
    assert_eq!(
        division_name(&row, &names, &localisation, "english").as_deref(),
        Ok("Guards")
    );
    // A programmatic member with a group: the capture records no stored
    // name, and the group's entry is shown.
    row.name_programmatic = 1;
    row.name_custom = b"Ignored programmatic text".to_vec();
    assert_eq!(
        division_name(&row, &names, &localisation, "english").as_deref(),
        Ok("144ya Strelkovaya Diviziya")
    );
    // With a null group the capture records the stored name, which
    // 1409B5640 shows.
    row.name_group.clear();
    row.name_custom = b"1-ya Strelkovaya".to_vec();
    assert_eq!(
        division_name(&row, &names, &localisation, "english").as_deref(),
        Ok("1-ya Strelkovaya")
    );
    localisation.language = "french".into();
    let refused = division_name(&row, &names, &localisation, "english").unwrap_err();
    assert!(matches!(&refused, Unnamed::StoredText(_)));
    assert!(refused.to_string().contains("stored in english"));
    let warnings = unnamed_warnings(9, std::iter::once(&row), |row| {
        shown_name(row, &names, &NoIcons, &localisation, "english")
    });
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("1 division names are unavailable"));
    assert!(shown_name(&row, &names, &NoIcons, &localisation, "english").is_err());
    row.name_programmatic = 0;
    assert_eq!(
        division_name(&row, &names, &localisation, "english").as_deref(),
        Ok("1-ya Strelkovaya")
    );
    // Without one the game builds the legacy name.
    row.name_custom.clear();
    assert_eq!(
        division_name(&row, &names, &localisation, "english"),
        Err(Unnamed::Unresolved(Unresolved::NoGroup))
    );
    row.name_group = b"SOV_INF_01".to_vec();
    row.name_type = 1;
    assert_eq!(
        division_name(&row, &names, &localisation, "english"),
        Err(Unnamed::ForeignType(1))
    );
}

#[test]
fn combat_names_use_the_retained_getter_and_never_mask_cache_failures() {
    use replay::rows::{DivisionStatusRow, Recorded};
    let names = Database::default();
    let mut loc = Localisation::default();
    loc.language = "english".into();
    let row = LandCombatDivisionRow {
        division: 7,
        name_custom: b"Stale combat projection".to_vec(),
        ..Default::default()
    };
    let mut status = DivisionStatusRow {
        division: 7,
        name: Recorded::Value(Some(b"Retained legacy allocation".to_vec())),
        naming: Recorded::Value(None),
        ..Default::default()
    };
    let shown = |status: Option<&DivisionStatusRow>| {
        shown_recorded_name(
            &row,
            status,
            "unavailable",
            &names,
            &NoIcons,
            &loc,
            "english",
        )
    };
    assert_eq!(
        shown(Some(&status)).as_deref(),
        Ok("Retained legacy allocation")
    );
    status.name = Recorded::Value(Some(Vec::new()));
    assert_eq!(shown(Some(&status)).as_deref(), Ok(""));
    status.name = Recorded::Value(None);
    assert!(
        shown(Some(&status))
            .unwrap_err()
            .to_string()
            .contains("DivisionStatus.name unavailable")
    );
    assert!(
        shown(None)
            .unwrap_err()
            .to_string()
            .contains("no cached-name row")
    );
    status.name = Recorded::NotRecorded;
    assert_eq!(
        shown(Some(&status)).as_deref(),
        Ok("Stale combat projection")
    );
}

/// Each cause is one warning naming every division it leaves unnamed,
/// with its group and ordinal.
#[test]
fn unnamed_divisions_are_reported_by_cause_with_their_identities() {
    let names = installed_content::division_names::parse(
        br#"INF = { ordered = { 1 = { "First" } } unordered = { loose = { "Loose" } } }"#,
        &installed_content::key_names::KeyNames::default(),
    )
    .unwrap();
    let row = |division, group: &[u8], order, ordered: u8| LandCombatDivisionRow {
        division,
        name_group: group.to_vec(),
        name_order: order,
        name_ordered: ordered,
        ..Default::default()
    };
    let rows = [
        row(1, b"INF", 1, 1),
        row(2, b"INF", 0, 1),
        row(3, b"INF", 2, 1),
        row(4, b"INF", 1, 0),
        row(5, b"GER\xff", 1, 1),
        row(6, b"ABSENT", 3, 1),
        LandCombatDivisionRow {
            name_custom: b"\xffGuards".to_vec(),
            ..row(7, b"INF", 1, 1)
        },
        LandCombatDivisionRow {
            name_type: 2,
            ..row(8, b"INF", 1, 1)
        },
        // A player's name holding the stored flag control 0x12.
        LandCombatDivisionRow {
            name_custom: b"\x12GER Guards".to_vec(),
            ..row(10, b"INF", 1, 1)
        },
    ];
    struct NoIcons;
    impl TextIcons for NoIcons {
        fn icon(&self, _: &str) -> Option<(u32, i64)> {
            None
        }
    }
    let warnings = unnamed_warnings(9, rows.iter(), |row| {
        shown_name(row, &names, &NoIcons, &Localisation::default(), "")
    });
    let expected = [
        (Unnamed::CustomNotText, "division 7 (group INF, ordered 1)"),
        (Unnamed::ForeignType(2), "division 8 (group INF, ordered 1)"),
        (
            Unnamed::Unresolved(Unresolved::GroupAbsent),
            "division 5 (group GER\\xff, ordered 1); division 6 (group ABSENT, ordered 3)",
        ),
        (
            Unnamed::Unresolved(Unresolved::NoFallback),
            "division 3 (group INF, ordered 2); division 4 (group INF, unordered 1)",
        ),
        (
            Unnamed::Unresolved(Unresolved::UnassignedOrder),
            "division 2 (group INF, ordered 0)",
        ),
        (
            Unnamed::Control(
                "the inline flag 0x12 \"GER\" (an authored `@`), which the viewer does not draw"
                    .into(),
            ),
            "division 10 (group INF, ordered 1)",
        ),
    ];
    assert_eq!(warnings.len(), expected.len(), "{warnings:#?}");
    for (cause, divisions) in expected {
        let count = divisions.split("; ").count();
        let warning = format!(
            "Land combat 9: {count} division names are unavailable because {cause}: {divisions}."
        );
        assert!(warnings.contains(&warning), "{warning}\n{warnings:#?}");
    }
}

#[test]
fn initiative_adds_the_reconnaissance_define_before_comparing_skills() {
    // pairL_land_danzig.png at 60828493: the defender glows.
    assert_eq!(initiative_side([4, 5], [0, 200_000], 5), Some(1));
    // Reconnaissance can carry the lower skill past the higher one.
    assert_eq!(initiative_side([4, 5], [200_000, 0], 5), Some(0));
    // A tie in the score shows neither glow; equal reconnaissance adds nothing.
    assert_eq!(initiative_side([3, 3], [100_000, 100_000], 5), None);
    assert_eq!(initiative_side([0, 0], [0, 1], 0), None);
}

#[test]
fn several_directions_compare_locations_or_the_source_in_the_combat_province() {
    let combat = replay::rows::LandCombatRow {
        province: 362,
        attackers: vec![1, 2],
        ..Default::default()
    };
    let rows = [
        LandCombatDivisionRow {
            division: 1,
            province: 9263,
            ..Default::default()
        },
        LandCombatDivisionRow {
            division: 2,
            province: 389,
            ..Default::default()
        },
    ];
    let mut divisions: BTreeMap<_, _> = rows.iter().map(|r| ((0, r.division), r)).collect();
    let mut movement = BTreeMap::new();
    // pairJ_land_5.png: 144ya at 9263 and 2-p Morit at 389 show the icon.
    assert_eq!(
        several_directions(&combat, false, &divisions, &movement),
        Some(true)
    );
    // A border war never has several directions.
    assert_eq!(
        several_directions(&combat, true, &divisions, &movement),
        Some(false)
    );
    let entered = [
        LandCombatDivisionRow {
            division: 1,
            province: 362,
            ..Default::default()
        },
        LandCombatDivisionRow {
            division: 2,
            province: 9263,
            ..Default::default()
        },
    ];
    divisions = entered.iter().map(|r| ((0, r.division), r)).collect();
    assert_eq!(
        several_directions(&combat, false, &divisions, &movement),
        None
    );
    movement.insert(1, 9263);
    assert_eq!(
        several_directions(&combat, false, &divisions, &movement),
        Some(false)
    );
    movement.insert(1, 0);
    assert_eq!(
        several_directions(&combat, false, &divisions, &movement),
        Some(true)
    );
}

#[test]
fn a_missing_modifier_row_leaves_the_combat_class_undecided() {
    use replay::rows::LandCombatModifierRow;
    let row = |combat, side, border_values| LandCombatModifierRow {
        combat,
        side,
        border_values,
        ..Default::default()
    };
    assert_eq!(border_war(&[], 5), None);
    assert_eq!(border_war(&[row(4, 0, Some([0; 3]))], 5), None);
    // Either side's row decides; a projected-out side does not matter.
    assert_eq!(border_war(&[row(5, 1, None)], 5), Some(false));
    assert_eq!(
        border_war(
            &[row(4, 0, None), row(5, 1, Some([0, 100_000, 100_000]))],
            5
        ),
        Some(true)
    );
}

#[test]
fn fallback_roles_use_full_side_ordinal_without_inventing_commitment() {
    let rows = [
        LandCombatDivisionRow {
            division: 90,
            role: 1,
            ordinal: 3,
            ..Default::default()
        },
        LandCombatDivisionRow {
            division: 80,
            role: 2,
            ordinal: 0,
            ..Default::default()
        },
        LandCombatDivisionRow {
            division: 70,
            role: 0,
            ordinal: 1,
            ..Default::default()
        },
        LandCombatDivisionRow {
            division: 60,
            role: 1,
            ordinal: 2,
            ..Default::default()
        },
        LandCombatDivisionRow {
            division: 50,
            side: 1,
            role: 1,
            ordinal: 0,
            ..Default::default()
        },
    ];
    assert_eq!(role_rosters(&rows, 0), [vec![60, 90], vec![80]]);
    assert_eq!(role_rosters(&rows, 1), [vec![50], vec![]]);
}

use replay::{
    frame::{Metrics, TableView},
    rows::{
        LandCombatDetailsRow, LandCombatRow, LandCombatSideDetails, TableId, TableRows,
        WeatherProvinceRow,
    },
};

const COMBAT: u64 = 62;
const PROVINCE: u32 = 3_000;

fn window_frame(detail: Option<LandCombatDetailsRow>, weather: Option<TableView>) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 0,
        hour: 7,
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "not recorded".into(),
            }
            .into()
        }),
        metrics: Metrics::default(),
    };
    *frame.tables[TableId::LandCombats.index()] =
        TableView::Complete(TableRows::LandCombats(vec![LandCombatRow {
            id: COMBAT,
            province: PROVINCE,
            ..Default::default()
        }]));
    *frame.tables[TableId::LandCombatDetails.index()] =
        TableView::Complete(TableRows::LandCombatDetails(detail.into_iter().collect()));
    if let Some(weather) = weather {
        *frame.tables[TableId::WeatherProvinces.index()] = weather;
    }
    frame
}

fn snow(snow: i64) -> Option<TableView> {
    Some(TableView::Complete(TableRows::WeatherProvinces(vec![
        WeatherProvinceRow {
            id: PROVINCE,
            snow,
            ..Default::default()
        },
    ])))
}

/// The window as an earlier binding left it: a first-member flag on each
/// side and the summer terrain picture.
fn stale() -> Bindings {
    let mut out = Bindings::default();
    for key in ["attacker_flag", "defender_flag"] {
        out.bind(key, FLAG, Ok("SOV".into()));
    }
    out.bind("terrain_picture", SPRITE, Ok("GFX_terrain_plains".into()));
    out
}

fn bind_pictures(frame: &DecodedFrame, sprites: &[&str]) -> Bindings {
    let terrain = BTreeMap::from([(PROVINCE, "plains".to_owned())]);
    let flags = BTreeMap::from([(7, Ok("GER".to_owned()))]);
    let has_sprite = |name: &str| sprites.contains(&name);
    let mut out = stale();
    pictures(&mut out, frame, COMBAT, &terrain, &has_sprite, &flags).unwrap();
    out
}

fn refused(out: &Bindings, fragment: &str) -> bool {
    out.refused()
        .keys()
        .any(|refusal| refusal.contains(fragment))
}

/// 1415C5F10 returns the side's country with or without a commander, and
/// 1415C9860 binds the flag from it: without a commander the recorded
/// first primary country (or province controller) replaces the earlier
/// first-member flag; a country without artwork hides the flag.
#[test]
fn a_side_without_a_commander_takes_its_recorded_country_flag() {
    let detail = LandCombatDetailsRow {
        combat: COMBAT,
        province: PROVINCE,
        sides: [
            LandCombatSideDetails {
                commander: 0,
                commander_country: 7,
                ..Default::default()
            },
            LandCombatSideDetails {
                commander: 0,
                commander_country: 9,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let out = bind_pictures(
        &window_frame(Some(detail), snow(0)),
        &["GFX_terrain_plains"],
    );
    let attacker = &out.values["attacker_flag"];
    assert_eq!(attacker.flag.as_deref(), Some("GER"));
    assert!(!attacker.hidden);
    let defender = &out.values["defender_flag"];
    assert_eq!(defender.flag, None, "the earlier SOV flag is cleared");
    assert!(defender.hidden);
    assert!(refused(
        &out,
        "Land combat 62 defender flag is hidden: no combat flag resolves for country 9"
    ));
    // Without LandCombatDetails the side country is unknown: neither the
    // earlier flag nor a first-member guess is kept.
    let out = bind_pictures(&window_frame(None, snow(0)), &["GFX_terrain_plains"]);
    for key in ["attacker_flag", "defender_flag"] {
        assert!(out.values[key].hidden && out.values[key].flag.is_none());
    }
    assert!(refused(
        &out,
        "attacker flag is hidden: LandCombatDetails has no row for the combat"
    ));
}

/// 1415C7520 adds `_winter` for positive province snow. A missing winter
/// sprite or a missing weather row hides the picture and names the gap,
/// rather than keeping the earlier summer picture or assuming no snow.
#[test]
fn the_terrain_picture_is_decided_once_and_hidden_on_refusal() {
    let both = ["GFX_terrain_plains", "GFX_terrain_plains_winter"];
    let picture = |out: &Bindings| {
        let widget = &out.values["terrain_picture"];
        (widget.sprite.clone(), widget.hidden)
    };
    let out = bind_pictures(&window_frame(None, snow(0)), &both);
    assert_eq!(picture(&out), (Some("GFX_terrain_plains".into()), false));
    let out = bind_pictures(&window_frame(None, snow(1)), &both);
    assert_eq!(
        picture(&out),
        (Some("GFX_terrain_plains_winter".into()), false)
    );
    // Snow with no winter sprite: the summer picture bound earlier goes.
    let out = bind_pictures(&window_frame(None, snow(1)), &["GFX_terrain_plains"]);
    assert_eq!(picture(&out), (None, true));
    assert!(refused(
        &out,
        "Land combat 62 terrain picture is hidden: the content lacks sprite GFX_terrain_plains_winter (1415C7520)."
    ));
    // Missing weather is not zero snow, whatever the table's state.
    for (weather, state) in [
        (None, "the table is unavailable: not recorded"),
        (
            Some(TableView::Complete(TableRows::WeatherProvinces(Vec::new()))),
            "the complete table lacks it",
        ),
        (
            Some(TableView::Truncated {
                rows: TableRows::WeatherProvinces(Vec::new()),
                missing: 4,
            }),
            "the table is truncated, 4 rows unread",
        ),
    ] {
        let out = bind_pictures(&window_frame(None, weather), &both);
        assert_eq!(picture(&out), (None, true), "{state}");
        assert!(
            refused(
                &out,
                &format!(
                    "WeatherProvinces has no row for province 3000 ({state}), so its snow and the summer or `_winter` picture are unknown"
                )
            ),
            "{:?}",
            out.refused()
        );
    }
}

/// A value bound after a refusal shows the widget again; a repeated
/// refusal is counted, not repeated.
#[test]
fn a_binding_after_a_refusal_shows_the_widget_again() {
    let mut out = Bindings::default();
    out.bind("name", TEXT, Ok("early".into()));
    out.bind("name", TEXT, Err("name refused".into()));
    out.bind("name", TEXT, Err("name refused".into()));
    assert!(out.values["name"].hidden && out.values["name"].text.is_none());
    assert_eq!(out.refused().get("name refused").copied(), Some(2));
    out.bind("name", TEXT, Ok("late".into()));
    assert!(!out.values["name"].hidden);
    assert_eq!(out.values["name"].text.as_deref(), Some("late"));
}
