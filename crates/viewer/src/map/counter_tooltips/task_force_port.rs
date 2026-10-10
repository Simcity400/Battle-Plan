//! Political port-item tooltips.
//! The item's update retains its metadata; it has no arrival query.

use super::*;
use crate::map::{counter_inputs::NavalMember, unit_selection};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Control {
    #[default]
    Normal,
    InvalidOrders,
    NoOrders,
    Carrier,
    Pride,
    Spotting,
    /// A drawn opaque object with no callback. It blocks siblings beneath it.
    Unbound,
}

pub use super::gui_authored::Authored;

/// Independent metadata fields retained by port-item updates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Metadata {
    pub country: u32,
    pub visibility: u16,
    pub enemy: Result<bool, String>,
    pub hold_fire: Option<bool>,
    pub idle: Option<bool>,
    pub no_regions: Option<bool>,
    pub pride: Option<bool>,
}

fn any(terms: impl IntoIterator<Item = Option<bool>>) -> Option<bool> {
    terms.into_iter().fold(Some(false), merge)
}

fn merge(a: Option<bool>, b: Option<bool>) -> Option<bool> {
    match (a, b) {
        (Some(true), _) | (_, Some(true)) => Some(true),
        (Some(false), Some(false)) => Some(false),
        _ => None,
    }
}

impl Metadata {
    pub(crate) fn from_members(
        members: &[NavalMember<'_>],
        aggression: &[i64],
        country: u32,
        visibility: u16,
        enemy: Result<bool, String>,
    ) -> Self {
        let mut hold_fire = Some(false);
        let mut idle = Some(false);
        let mut no_regions = Some(false);
        for member in members.iter().filter(|member| member.own) {
            let Some(inputs) = member.inputs else {
                hold_fire = merge(hold_fire, None);
                idle = merge(idle, None);
                no_regions = merge(no_regions, None);
                continue;
            };
            if inputs.repair_parent != 0 {
                continue;
            }
            hold_fire = merge(
                hold_fire,
                if matches!(member.mission, 2 | 3) {
                    counter_inputs::engagement_refused(
                        inputs.running_away,
                        inputs.engagement_rule,
                        member.mission,
                        aggression,
                    )
                } else {
                    Some(false)
                },
            );
            idle = merge(
                idle,
                if member.has_fleet {
                    member.fleet_regions.map(|count| {
                        count != 0
                            && counter_inputs::effective_mission(
                                member.mission,
                                inputs.detached_activity,
                                inputs.repair_last_mission,
                            ) == 0
                    })
                } else {
                    Some(false)
                },
            );
            no_regions = merge(
                no_regions,
                Some(
                    counter_inputs::needs_regions(member.mission)
                        && inputs.mission_region_count == 0,
                ),
            );
        }
        Self {
            country,
            visibility,
            enemy,
            hold_fire,
            idle,
            no_regions,
            pride: any(members.iter().map(|member| member.pride)),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hover {
    pub control: Control,
    /// Index of this item's typed plate in the same CounterReport.
    pub unit_hit: usize,
    pub rect: [f32; 4],
    pub supported: bool,
    pub(crate) hit: Option<crate::map::native_gui::HitGeometry>,
    pub members: Arc<[u64]>,
    pub metadata: Metadata,
    /// Static content is used only when this object's callback is absent or false.
    pub authored: Option<Authored>,
}

impl Hover {
    pub fn has_tooltip(&self) -> bool {
        self.control != Control::Unbound || self.authored.is_some()
    }

    pub fn contains(&self, point: [f32; 2]) -> bool {
        self.hit.as_ref().map_or_else(
            || crate::combat::contains(self.rect, point),
            |hit| hit.contains(point),
        )
    }

    pub fn matches(&self, unit: &unit_selection::Hit) -> bool {
        unit.port && unit.kind == unit_selection::Kind::TaskForce && unit.members == self.members
    }
}

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

/// The callback resolves the first live reference before any arm, then its
/// generic summary rebuilds the live vector. A complete missing row is dangling.
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
            authored.fallback(context, &mut out);
        }
        return out;
    }
    let members = match resolved(context, &input.members) {
        Ok(members) if members.is_empty() => {
            if let Some(authored) = &input.authored {
                authored.fallback(context, &mut out);
            }
            return out;
        }
        Ok(members) => members,
        Err(reason) => {
            // A missing recording is unknown callback admission, not native
            // false. Do not invent authored fallback over this uncertainty.
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
            // Unlike the at-sea callback, this description is the delayed body.
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

/// Port pride uses the metadata country, then the registered pride ship's
/// current task force. It does not scan only this item's member force names.
fn pride(context: &Context<'_>, metadata: &Metadata, out: &mut Tooltip) -> bool {
    if metadata.pride == Some(false) {
        return false;
    }
    let chosen: Result<Option<(&replay::rows::CountryCounterInputRow, u64)>, String> = (|| {
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
    use crate::map::{test_frame, test_put};
    use replay::rows::{TableRows, TaskForceCounterInputRow, TaskForceRow};

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

    fn with_context(frame: &DecodedFrame, run: impl FnOnce(&Context<'_>)) {
        let loc: Localisation = [
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
        let names = installed_content::division_names::Database::default();
        let units = Err("unreached".into());
        let vision = Vision::sharing(1, []);
        run(&Context {
            nato: false,
            content: None,
            dig_in_factor: Err("unreached".into()),
            loc: &loc,
            recorded_language: &loc.language,
            icons: &crate::combat::binding::NoIcons,
            names: &names,
            units: &units,
            supply_grace: Err("unreached".into()),
            comms_max_distance: Err("unreached".into()),
            frame,
            vision: &vision,
            links: frame
                .complete::<tables::CountryLinks>()
                .map_err(|e| e.to_string()),
        });
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
                control: Control::InvalidOrders,
                unit_hit: 0,
                rect: [0.; 4],
                supported: true,
                hit: None,
                members: Arc::from([7]),
                authored: None,
                metadata: Metadata {
                    hold_fire: Some(true),
                    idle: Some(true),
                    ..metadata()
                },
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
        // Unknown own inputs do not invalidate a known positive flag.
        let members = [members[2], member(true, 2, None)];
        let metadata = Metadata::from_members(&members, &[], 1, 100, Ok(false));
        assert_eq!(metadata.hold_fire, None);
        assert_eq!(metadata.idle, Some(true));
        assert_eq!(metadata.no_regions, Some(true));
    }

    #[test]
    fn port_hover_matches_only_its_port_task_force_plate() {
        let hover = Hover {
            control: Control::Normal,
            unit_hit: 3,
            rect: [0.; 4],
            supported: true,
            hit: None,
            members: Arc::from([7]),
            metadata: metadata(),
            authored: None,
        };
        let mut hit = unit_selection::Hit {
            ours: true,
            port: true,
            rect: [0.; 4],
            kind: unit_selection::Kind::TaskForce,
            members: Arc::clone(&hover.members),
            layer: 0,
        };
        assert!(hover.matches(&hit));
        hit.port = false;
        assert!(!hover.matches(&hit));
        hit.port = true;
        hit.kind = unit_selection::Kind::RailwayGun;
        assert!(!hover.matches(&hit));
    }

    #[test]
    fn unbound_port_authored_fallback_keeps_raw_lookup_and_bound_localisation_distinct() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let frame = test_frame(0);
        let gui = installed_content::gui::native::read(
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
                    control: Control::Unbound,
                    unit_hit: 0,
                    rect: [0.; 4],
                    supported: true,
                    hit: None,
                    members: Arc::from([999]),
                    metadata: metadata(),
                    authored: Authored::read(gui.template(name).unwrap(), false),
                };
                assert!(input.has_tooltip());
                let text = build(context, &input);
                assert_eq!(text.title, expected);
                assert!(text.body.is_empty() && text.refused.is_empty());
            }
            let input = Hover {
                control: Control::Unbound,
                unit_hit: 0,
                rect: [0.; 4],
                supported: true,
                hit: None,
                members: Arc::from([999]),
                metadata: metadata(),
                authored: None,
            };
            assert!(!input.has_tooltip());
            assert_eq!(build(context, &input), Tooltip::default());
        });
    }

    #[test]
    fn port_callback_handled_empty_controls_suppress_authored_fallback() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let gui = installed_content::gui::native::read(
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
                        control,
                        unit_hit: 0,
                        rect: [0.; 4],
                        supported: true,
                        hit: None,
                        members: Arc::from([999, 7]),
                        metadata: metadata(),
                        authored: authored.clone(),
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
                    control: Control::Carrier,
                    unit_hit: 0,
                    rect: [0.; 4],
                    supported: true,
                    hit: None,
                    members: Arc::from([7]),
                    metadata: metadata(),
                    authored: authored.clone(),
                },
            );
            assert_eq!(text, Tooltip::default());
        });
    }

    #[test]
    fn port_callback_false_allows_fallback_but_unknown_admission_does_not() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let gui = installed_content::gui::native::read(
            b"guiTypes = { guiButtonType = { name = authored pdx_tooltip = RAW } }",
        );
        let input = Hover {
            control: Control::NoOrders,
            unit_hit: 0,
            rect: [0.; 4],
            supported: true,
            hit: None,
            members: Arc::from([999]),
            metadata: metadata(),
            authored: Authored::read(gui.template("authored").unwrap(), false),
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
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let gui = installed_content::gui::native::read(
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
                    .fallback(context, &mut text);
                assert_eq!((text.title.as_str(), text.body.as_str()), (title, body));
                assert!(text.refused.is_empty());
            }
        });
    }

    #[test]
    fn port_pride_uses_metadata_country_and_registered_ships_current_force_name() {
        use replay::rows::{CountryCounterInputRow, CountryLinkRow, CountryRow, ShipRow};
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
                forward_visibility: replay::rows::Recorded::NotRecorded,
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
        // Full-reference mismatch is a dangling native ship and falls through.
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
