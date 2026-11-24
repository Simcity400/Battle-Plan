
use super::*;
#[cfg(test)]
use viewer_native_gui::counter_hovers::gui_authored::Authored;
use viewer_native_gui::counter_hovers::task_force_port::{Control, Hover, Metadata};
#[cfg(test)]
use viewer_unit_reads::counter_inputs::NavalMember;

fn append_flag(
    context: &Context<'_>,
    out: &mut Tooltip,
    value: Option<bool>,
    body: bool,
    line: &str,
    key: &str,
) {
    match value {
        Some(true) => super::append(out, context, body, line, localise(context.loc, key, &[])),
        Some(false) => (),
        None => refuse(out, line, "retained naval mission operands are unavailable"),
    }
}

fn resolved(context: &Context<'_>, members: &[u64]) -> Result<Vec<u64>, String> {
    if members.is_empty() {
        return Ok(Vec::new());
    }
    let rows = context
        .frame
        .complete::<tables::TaskForces>()
        .map_err(|e| e.to_string())?;
    Ok(members
        .iter()
        .copied()
        .filter(|id| *id != 0 && rows.binary_search_by_key(id, |row| row.id).is_ok())
        .collect())
}

pub(super) fn build(context: &Context<'_>, input: &Hover) -> Tooltip {
    let mut out = Tooltip::default();
    if input.control == Control::Unbound {
        if let Some(authored) = &input.authored {
            authored.fallback(context.loc, context.icons, &mut out);
        }
        return out;
    }
    let members = match resolved(context, &input.members) {
        Ok(members) if members.is_empty() => {
            if let Some(authored) = &input.authored {
                authored.fallback(context.loc, context.icons, &mut out);
            }
            return out;
        }
        Ok(members) => members,
        Err(reason) => {
            refuse(&mut out, "port task force callback admission", reason);
            return out;
        }
    };
    match input.control {
        Control::NoOrders => {
            append_flag(
                context,
                &mut out,
                input.metadata.no_regions,
                false,
                "port task force no orders",
                "ARMY_NAVIES_MISSION_NO_REGIONS",
            );
            return out;
        }
        Control::InvalidOrders => {
            append_flag(
                context,
                &mut out,
                input.metadata.hold_fire,
                false,
                "port task force engagement",
                "SHIP_ENGAGEMENT_MAP_ICON",
            );
            if input.metadata.hold_fire == Some(true) {
                super::append(
                    &mut out,
                    context,
                    true,
                    "port task force engagement description",
                    localise(context.loc, "SHIP_ENGAGEMENT_BUTTON_HOLD_FIRE_DESC", &[]),
                );
            }
            append_flag(
                context,
                &mut out,
                input.metadata.idle,
                false,
                "port task force idle",
                "ARMY_NAVIES_MISSION_IDLE",
            );
            return out;
        }
        Control::Carrier => {
            super::task_force_children::carrier(context, &members, &mut out);
            return out;
        }
        Control::Pride if pride(context, &input.metadata, &mut out) => return out,
        _ => (),
    }
    super::task_force_port_mission::append_searching(
        context,
        members[0],
        input.metadata.visibility,
        &mut out,
    );
    super::task_force_summary::append(context, &members, false, &mut out);
    if context.diagnostics
        && context
            .slot(input.metadata.country)
            .is_ok_and(|slot| slot == context.vision.ours())
    {
        super::naval_management::append(context, &members, &mut out);
    }
    if members.len() == 1 {
        match &input.metadata.enemy {
            Ok(true) => super::task_force_port_mission::append(
                context,
                members[0],
                input.metadata.visibility,
                &mut out,
            ),
            Ok(false) => (),
            Err(reason) => refuse(
                &mut out,
                "port task force mission admission",
                reason.clone(),
            ),
        }
    }
    out
}

fn pride(context: &Context<'_>, metadata: &Metadata, out: &mut Tooltip) -> bool {
    if metadata.pride == Some(false) {
        return false;
    }
    let chosen: Result<Option<(&replay_data::rows::CountryCounterInputRow, u64)>, String> =
        (|| {
            if metadata.pride.is_none() {
                return Err("retained naval mission pride flag is unavailable".to_owned());
            }
            let slot = context.slot(metadata.country)?;
            let countries = context
                .frame
                .complete::<tables::Countries>()
                .map_err(|e| e.to_string())?;
            let raw = countries
                .iter()
                .find(|row| row.canonical_slot == slot)
                .map(|row| row.id)
                .ok_or_else(|| format!("Countries canonical slot {slot} is absent"))?;
            let inputs = context
                .frame
                .complete::<tables::CountryCounterInputs>()
                .map_err(|e| e.to_string())?;
            let country = inputs
                .binary_search_by_key(&raw, |row| row.country)
                .map(|index| &inputs[index])
                .map_err(|_| format!("CountryCounterInputs country {raw} is absent"))?;
            if country.pride_of_the_fleet == 0 {
                return Ok(None);
            }
            let ships = context
                .frame
                .complete::<tables::Ships>()
                .map_err(|e| e.to_string())?;
            Ok(ships
                .binary_search_by_key(&country.pride_of_the_fleet, |row| row.id)
                .ok()
                .map(|index| (country, ships[index].task_force)))
        })();
    match chosen {
        Ok(None) => false,
        Ok(Some((country, task_force))) => {
            super::append(
                out,
                context,
                false,
                "port task force pride name",
                super::task_force_children::pride_name(context, country, task_force),
            );
            true
        }
        Err(reason) => {
            refuse(out, "port task force pride", reason);
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{rows::TableRows, rows::TaskForceCounterInputRow, rows::TaskForceRow};
    use viewer_foundation::test_support::{test_frame, test_put};

    fn metadata() -> Metadata {
        Metadata {
            country: 1,
            visibility: 250,
            enemy: Ok(false),
            hold_fire: Some(false),
            idle: Some(false),
            no_regions: Some(false),
            pride: Some(false),
        }
    }

    fn hover(control: Control, members: &[u64]) -> Hover {
        Hover {
            control,
            unit_hit: 0,
            rect: [0.; 4],
            supported: true,
            hit: None,
            members: members.into(),
            authored: None,
            metadata: metadata(),
        }
    }

    fn with_context(frame: &DecodedFrame, run: impl FnOnce(&Context<'_>)) {
        let mut loc: Localisation = [
            ("SHIP_ENGAGEMENT_MAP_ICON".into(), "engagement".into()),
            (
                "SHIP_ENGAGEMENT_BUTTON_HOLD_FIRE_DESC".into(),
                "hold fire".into(),
            ),
            ("ARMY_NAVIES_MISSION_IDLE".into(), "idle".into()),
            ("ARMY_NAVIES_MISSION_NO_REGIONS".into(), "no regions".into()),
            ("RAW".into(), "$OTHER$".into()),
            ("OTHER".into(), "expanded".into()),
            ("EMPTY".into(), "".into()),
            ("BODY".into(), "raw body".into()),
            ("ZERO".into(), "first\0ignored".into()),
            (
                "SPECIFIC_NAVY_CONTAINS_PRIDE_OF_FLEET".into(),
                "$NAVY_NAME$: $NAME$".into(),
            ),
        ]
        .into_iter()
        .collect();
        loc.language = "english".into();
        super::super::task_force::tests::with_test_context(
            frame,
            &loc,
            "unreached",
            None,
            |context| {
                run(&context);
            },
        );
    }

    #[test]
    fn port_invalid_orders_returns_early_and_keeps_hold_fire_in_the_delayed_body() {
        let mut frame = test_frame(0);
        test_put(
            &mut frame,
            TableRows::TaskForces(vec![TaskForceRow {
                id: 7,
                ..Default::default()
            }]),
        );
        with_context(&frame, |context| {
            let input = Hover {
                metadata: Metadata {
                    hold_fire: Some(true),
                    idle: Some(true),
                    ..metadata()
                },
                ..hover(Control::InvalidOrders, &[7])
            };
            let text = build(context, &input);
            assert_eq!(text.title, "engagementidle");
            assert_eq!(text.body, "hold fire");
            assert!(text.refused.is_empty());
            let empty = build(
                context,
                &Hover {
                    metadata: metadata(),
                    ..input
                },
            );
            assert!(empty.title.is_empty() && empty.body.is_empty() && empty.refused.is_empty());
        });
    }

    #[test]
    fn retained_port_flags_skip_foreign_and_repair_members_and_keep_detached_idle() {
        let repair = TaskForceCounterInputRow {
            repair_parent: 9,
            running_away: 1,
            ..Default::default()
        };
        let detached = TaskForceCounterInputRow {
            detached_activity: 2,
            repair_last_mission: 0,
            mission_region_count: 0,
            ..Default::default()
        };
        let member = |own, mission, inputs| NavalMember {
            visible: false,
            own,
            mission,
            has_fleet: true,
            inputs,
            fleet_regions: Some(2),
            pride: Some(false),
        };
        let members = [
            member(false, 2, None),
            member(true, 2, Some(&repair)),
            member(true, 1, Some(&detached)),
        ];
        let metadata = Metadata::from_members(&members, &[], 1, 100, Ok(false));
        assert_eq!(metadata.hold_fire, Some(false));
        assert_eq!(metadata.idle, Some(true));
        assert_eq!(metadata.no_regions, Some(true));
        let members = [members[2], member(true, 2, None)];
        let metadata = Metadata::from_members(&members, &[], 1, 100, Ok(false));
        assert_eq!(metadata.hold_fire, None);
        assert_eq!(metadata.idle, Some(true));
        assert_eq!(metadata.no_regions, Some(true));
    }

    #[test]
    fn unbound_port_authored_fallback_keeps_raw_lookup_and_bound_localisation_distinct() {
        content_reader_script::test_game::enter();
        let frame = test_frame(0);
        let gui = content_reader_gui::gui::native::read(
            br#"guiTypes = {
            guiButtonType = { name = raw pdx_tooltip = RAW }
            guiButtonType = { name = missing pdx_tooltip = MISSING }
            guiButtonType = { name = bound bound_tooltip = RAW }
            guiButtonType = { name = zero pdx_tooltip = ZERO }
        }"#,
        );
        with_context(&frame, |context| {
            for (name, expected) in [
                ("raw", "$OTHER$\n"),
                ("missing", ""),
                ("bound", "expanded\n"),
                ("zero", "first\n"),
            ] {
                let input = Hover {
                    authored: Authored::read(gui.template(name).unwrap(), false),
                    ..hover(Control::Unbound, &[999])
                };
                assert!(input.has_tooltip());
                let text = build(context, &input);
                assert_eq!(text.title, expected);
                assert!(text.body.is_empty() && text.refused.is_empty());
            }
            let input = hover(Control::Unbound, &[999]);
            assert!(!input.has_tooltip());
            assert_eq!(build(context, &input), Tooltip::default());
        });
    }

    #[test]
    fn port_callback_handled_empty_controls_suppress_authored_fallback() {
        content_reader_script::test_game::enter();
        let gui = content_reader_gui::gui::native::read(
            b"guiTypes = { guiButtonType = { name = authored pdx_tooltip = RAW } }",
        );
        let authored = Authored::read(gui.template("authored").unwrap(), false);
        let mut frame = test_frame(0);
        test_put(
            &mut frame,
            TableRows::TaskForces(vec![TaskForceRow {
                id: 7,
                ..Default::default()
            }]),
        );
        with_context(&frame, |context| {
            for control in [Control::NoOrders, Control::InvalidOrders] {
                let text = build(
                    context,
                    &Hover {
                        authored: authored.clone(),
                        ..hover(control, &[999, 7])
                    },
                );
                assert_eq!(text, Tooltip::default());
            }
        });
        test_put(&mut frame, TableRows::Ships(Vec::new()));
        with_context(&frame, |context| {
            let text = build(
                context,
                &Hover {
                    authored: authored.clone(),
                    ..hover(Control::Carrier, &[7])
                },
            );
            assert_eq!(text, Tooltip::default());
        });
    }

    #[test]
    fn port_callback_false_allows_fallback_but_unknown_admission_does_not() {
        content_reader_script::test_game::enter();
        let gui = content_reader_gui::gui::native::read(
            b"guiTypes = { guiButtonType = { name = authored pdx_tooltip = RAW } }",
        );
        let input = Hover {
            authored: Authored::read(gui.template("authored").unwrap(), false),
            ..hover(Control::NoOrders, &[999])
        };
        let mut frame = test_frame(0);
        with_context(&frame, |context| {
            let unknown = build(context, &input);
            assert!(unknown.title.is_empty());
            assert_eq!(unknown.refused.len(), 1);
            let empty = build(
                context,
                &Hover {
                    members: Arc::from([]),
                    ..input.clone()
                },
            );
            assert_eq!(empty.title, "$OTHER$\n");
            assert!(empty.refused.is_empty());
        });
        test_put(&mut frame, TableRows::TaskForces(Vec::new()));
        with_context(&frame, |context| {
            let dangling = build(context, &input);
            assert_eq!(dangling.title, "$OTHER$\n");
            assert!(dangling.refused.is_empty());
        });
    }

    #[test]
    fn authored_fallback_replaces_only_present_hash_fields_and_appends_unscoped_bound() {
        content_reader_script::test_game::enter();
        let gui = content_reader_gui::gui::native::read(
            br#"guiTypes = {
            guiButtonType = { name = absent pdx_tooltip = MISSING pdx_tooltip_delayed = BODY }
            guiButtonType = { name = empty pdx_tooltip = EMPTY }
            guiButtonType = { name = raw pdx_tooltip = RAW }
            guiButtonType = { name = bound bound_tooltip = RAW }
            guiButtonType = { name = zero pdx_tooltip = ZERO }
        }"#,
        );
        let frame = test_frame(0);
        with_context(&frame, |context| {
            for (name, title, body) in [
                ("absent", "native title", "raw body\n"),
                ("empty", "", "native body"),
                ("raw", "$OTHER$\n", "native body"),
                ("bound", "native titleexpanded\n", "native body"),
                ("zero", "first\n", "native body"),
            ] {
                let mut text = Tooltip {
                    title: "native title".into(),
                    body: "native body".into(),
                    ..Default::default()
                };
                Authored::read(gui.template(name).unwrap(), false)
                    .unwrap()
                    .fallback(context.loc, context.icons, &mut text);
                assert_eq!((text.title.as_str(), text.body.as_str()), (title, body));
                assert!(text.refused.is_empty());
            }
        });
    }

    #[test]
    fn port_pride_uses_metadata_country_and_registered_ships_current_force_name() {
        use replay_data::{
            rows::CountryCounterInputRow, rows::CountryLinkRow, rows::CountryRow, rows::ShipRow,
        };
        let mut frame = test_frame(0);
        test_put(
            &mut frame,
            TableRows::CountryLinks(vec![CountryLinkRow {
                id: 5,
                canonical_slot: 1,
            }]),
        );
        test_put(
            &mut frame,
            TableRows::Countries(vec![CountryRow {
                id: 7,
                canonical_slot: 1,
                ..Default::default()
            }]),
        );
        test_put(
            &mut frame,
            TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
                forward_visibility: replay_data::rows::Recorded::NotRecorded,
                country: 7,
                pride_of_the_fleet: 11,
                pride_name: Some(b"ship eleven".to_vec()).into(),
                ..Default::default()
            }]),
        );
        test_put(
            &mut frame,
            TableRows::Ships(vec![ShipRow {
                id: 11,
                task_force: 99,
                ..Default::default()
            }]),
        );
        test_put(
            &mut frame,
            TableRows::TaskForceCounterInputs(vec![TaskForceCounterInputRow {
                task_force: 99,
                name: Some(b"current navy".to_vec()).into(),
                ..Default::default()
            }]),
        );
        with_context(&frame, |context| {
            let mut text = Tooltip::default();
            assert!(pride(
                context,
                &Metadata {
                    country: 5,
                    pride: Some(true),
                    ..metadata()
                },
                &mut text
            ));
            assert_eq!(text.title, "current navy: ship eleven");
            assert!(text.refused.is_empty());
        });
        test_put(
            &mut frame,
            TableRows::Ships(vec![ShipRow {
                id: (1 << 32) | 11,
                task_force: 99,
                ..Default::default()
            }]),
        );
        with_context(&frame, |context| {
            let mut text = Tooltip::default();
            assert!(!pride(
                context,
                &Metadata {
                    country: 5,
                    pride: Some(true),
                    ..metadata()
                },
                &mut text
            ));
            assert_eq!(text, Tooltip::default());
        });
    }
}
