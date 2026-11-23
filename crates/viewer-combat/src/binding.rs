use super::{Assets, Selection, Values, Widget};
use content_reader_localisation::localisation::Argument;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io};
use viewer_engagements::engagement_history::CombatFlags;
use viewer_engagements::engagement_progress::FIXED;
use viewer_engagements::engagement_progress::Progresses;
use viewer_engagements::engagement_progress::sides as engagement_progress_sides;
use content_reader_localisation::localisation::icon_lookup;
use viewer_foundation::perspective::Vision;
use viewer_foundation::templates::custom_icon_index;
use viewer_foundation::tooltip_text::{TextIcons, builtin, token};
use super::content::Content as StaticContent;
use viewer_foundation::tables::rows;

pub(super) type NavalPenalty = (i64, i32, i32);

#[derive(Debug, Default)]
pub struct Bindings {
    pub(super) air_chart: Option<super::air_menu::Chart>,
    pub values: Values,
    pub(super) tactics: super::tactics::Menu,
    pub entries: [Vec<(u64, Values)>; 4],
    pub warnings: Vec<String>,
    pub naval_entries: Vec<NavalEntry>,
    pub(super) naval_penalties: [Option<Result<NavalPenalty, String>>; 2],
    refused: BTreeMap<String, usize>,
}
impl Bindings {
    pub(super) fn refuse(&mut self, refusal: String) {
        *self.refused.entry(refusal).or_default() += 1;
    }
    pub(super) fn bind<T>(&mut self, key: &str, field: Field<T>, value: Result<T, String>) {
        bind(&mut self.refused, &mut self.values, key, field, value);
    }
    pub(super) fn bind_in<T>(
        &mut self,
        values: &mut Values,
        key: &str,
        field: Field<T>,
        value: Result<T, String>,
    ) {
        bind(&mut self.refused, values, key, field, value);
    }
    pub(super) fn withhold_in(&mut self, values: &mut Values, keys: &[&str], refusal: String) {
        for key in keys {
            let widget = values.entry((*key).into()).or_default();
            widget.text = None;
            widget.sprite = None;
            widget.retain_sprite = false;
            widget.flag = None;
            widget.fill = None;
            widget.hidden = true;
        }
        self.refuse(refusal);
    }
    pub(super) fn withhold(&mut self, keys: &[&str], refusal: String) {
        let mut values = std::mem::take(&mut self.values);
        self.withhold_in(&mut values, keys, refusal);
        self.values = values;
    }
    pub fn refused(&self) -> &BTreeMap<String, usize> {
        &self.refused
    }
}
pub(super) struct Field<T>(fn(&mut Widget) -> &mut Option<T>);
impl<T> Clone for Field<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Field<T> {}
pub(super) const TEXT: Field<String> = Field(|widget| &mut widget.text);
pub(super) const SPRITE: Field<String> = Field(|widget| {
    widget.retain_sprite = false;
    &mut widget.sprite
});
pub(super) const FLAG: Field<String> = Field(|widget| &mut widget.flag);
pub(super) const FILL: Field<f32> = Field(|widget| &mut widget.fill);
fn bind<T>(
    refused: &mut BTreeMap<String, usize>,
    values: &mut Values,
    key: &str,
    field: Field<T>,
    value: Result<T, String>,
) {
    let widget = values.entry(key.into()).or_default();
    widget.hidden = value.is_err();
    match value {
        Ok(value) => *(field.0)(widget) = Some(value),
        Err(refusal) => {
            *(field.0)(widget) = None;
            *refused.entry(refusal).or_default() += 1;
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavalLayout {
    Ship,
    Sunk,
    AirLeft,
    AirRight,
}

#[derive(Debug)]
pub struct NavalEntry {
    pub layout: NavalLayout,
    pub side: u8,
    pub unknown: bool,
    pub grid: String,
    pub offset: Option<[f32; 2]>,
    pub size: [f32; 2],
    pub values: Values,
    pub battle_slots: Vec<u32>,
}
pub(super) fn text(values: &mut Values, key: &str, value: impl Into<String>) {
    values.entry(key.into()).or_default().text = Some(value.into());
}
impl TextIcons for Assets {
    fn icon(&self, name: &str) -> Option<(u32, i64)> {
        let (names, frame) = icon_lookup(name);
        names.iter().find_map(|name| {
            let sprite = self.drawable_sprite(name)?;
            Some((sprite.frames.max(1) as u32, i64::from(frame.unwrap_or(0))))
        })
    }
}
pub(super) fn hide(values: &mut Values, key: &str) {
    values.entry(key.into()).or_default().hidden = true;
}
pub fn sprite(values: &mut Values, key: &str, value: String) {
    values.entry(key.into()).or_default().sprite = Some(value);
}
pub(super) fn fill(values: &mut Values, key: &str, value: f32) {
    values.entry(key.into()).or_default().fill = Some(value);
}
pub(super) type Flags = BTreeMap<u32, Result<String, String>>;

pub(super) fn country_flag(flags: &Flags, country: u32) -> Result<String, String> {
    flags.get(&country).cloned().unwrap_or_else(|| Err({
        format!(
            "no combat flag resolves for country {country} (the hour's country rows or the content's flag artwork lack it)"
        )
    }))
}
fn ratio(value: i64, maximum: i64) -> Option<f32> {
    (maximum > 0).then(|| (value as f64 / maximum as f64).clamp(0.0, 1.0) as f32)
}
pub(super) fn flags(static_content: &StaticContent, frame: &DecodedFrame) -> io::Result<Flags> {
    Ok(dashboard_text::flag_choices::choices(frame)?
        .into_iter()
        .map(|choice| {
            let key = choice
                .key(|key| static_content.assets.has_flag(key))
                .map(str::to_owned);
            (choice.raw, key)
        })
        .collect())
}
impl Bindings {
    #[allow(clippy::too_many_arguments, reason = "the window's read inputs, each from another owner")]
    pub fn read(
        static_content: &StaticContent,
        frame: &DecodedFrame,
        selection: Selection,
        vision: &Vision,
        engaged: &CombatFlags,
        paused: bool,
        nato: bool,
        navy_sources: &viewer_unit_reads::naval_dominance_sources::RegionalDominanceSourcesMap,
        rule_relations: Option<&super::RuleRelations>,
    ) -> io::Result<Self> {
        let mut out = Self::default();
        if let Selection::State(province) = selection {
            return super::state_view::bind(static_content, frame, vision, province, nato, rule_relations);
        }
        if let Selection::NavalRegion(region) = selection {
            return super::naval_region_view::bind(static_content, frame, vision, region, engaged, paused, navy_sources);
        }
        if let Selection::AirRegion(region) = selection {
            return super::air_menu::bind(
                static_content,
                frame,
                vision,
                region,
                &Default::default(),
            );
        }
        out.warnings.extend(
            super::availability::combat_limitations(frame, selection)
                .lines()
                .iter()
                .cloned(),
        );
        let flags = flags(static_content, frame)?;
        match selection {
            Selection::Land(id) => out.land(static_content, frame, id, vision)?,
            Selection::Naval(id) => out.naval(static_content, frame, id, &flags)?,
            Selection::NavalResult(id) => out.naval_result(static_content, frame, id, &flags)?,
            Selection::AirRegion(_) | Selection::State(_) | Selection::NavalRegion(_) => unreachable!(),
        }
        super::details::bind(&mut out, static_content, frame, selection, &flags, vision)?;
        if let Selection::Naval(id) = selection {
            super::naval_environment::bind(
                &mut out,
                static_content,
                frame,
                id,
                engaged,
                vision,
                paused,
            )?;
        }
        super::naval_details::bind(&mut out, static_content, frame, selection, &flags, vision)?;
        super::presentation::bind(&mut out, static_content, frame, selection, &flags)?;
        super::tooltips::bind(&mut out, static_content, frame, selection, vision)?;
        super::division_tooltips::bind(&mut out, static_content, frame, selection, vision)?;
        super::commander_tooltips::bind(
            &mut out,
            super::commander_tooltips::BindContent {
                icons: static_content.assets.as_ref(),
                loc: &static_content.localisation,
                commanders: &static_content.assets.commander_tooltips,
                windows: &static_content.assets.native_windows,
                modifier_names: &static_content.modifier_names,
            },
            frame,
            selection,
            vision,
        )?;
        super::commander_tooltips::bind_naval(&mut out, static_content, frame, selection)?;
        super::land_terrain::bind(&mut out, static_content, frame, selection)?;
        super::tactics::bind(&mut out, static_content, frame, selection, vision, nato)?;
        super::tooltips::bind_authored(
            &mut out,
            static_content.assets.as_ref(),
            &static_content.localisation,
            &static_content.assets.native_windows,
            selection,
        );
        Ok(out)
    }
    fn naval_result(
        &mut self,
        static_content: &StaticContent,
        frame: &DecodedFrame,
        id: u64,
        flags: &Flags,
    ) -> io::Result<()> {
        for name in [
            "attacker_details",
            "defender_details",
            "attacker_hq_leader",
            "defender_hq_leader",
        ] {
            hide(&mut self.values, name);
        }
        let label = token(
            static_content.assets.as_ref(),
            &static_content.localisation,
            b"BUTTON_DISMISS",
        )
        .map_err(|reason| format!("Naval result {id} dismiss button text is hidden: {reason}."));
        self.bind("dismiss_button", TEXT, label);
        if let Some(result) = rows::<tables::NavalResults>(frame)?
            .rows()
            .iter()
            .find(|row| row.id == id)
        {
            let name = super::battle_name::title(
                static_content,
                frame,
                result.province,
                result.flag_a != 0,
            )?
            .map_err(|reason| format!("Naval result {id} name is hidden: {reason}."));
            self.bind("name", TEXT, name);
            let countries = rows::<tables::NavalResultCountries>(frame)?;
            for (side, prefix) in ["attacker", "defender"].iter().enumerate() {
                let flag = countries
                    .rows()
                    .iter()
                    .filter(|r| r.result == id && r.list == side as u8)
                    .min_by_key(|r| r.ordinal)
                    .ok_or_else(|| "NavalResultCountries lists no country for the side".to_owned())
                    .and_then(|r| country_flag(flags, r.country))
                    .map_err(|reason| {
                        format!("Naval result {id} {prefix} flag is hidden: {reason}.")
                    });
                self.bind(&format!("{prefix}_leader/flag"), FLAG, flag);
            }
        } else {
            self.withhold(
                &["name", "attacker_leader/flag", "defender_leader/flag"],
                format!(
                    "Naval result {id} is absent from NavalResults at hour {}; its name and flags are hidden.",
                    frame.hour
                ),
            );
        }
        self.warnings.push("Retained naval results use their recorded sides, members, hit vectors and commander snapshots. Results-window acceptance and fresh-save verification remain unverified.".into());
        Ok(())
    }
    fn land(
        &mut self,
        static_content: &StaticContent,
        frame: &DecodedFrame,
        id: u64,
        vision: &Vision,
    ) -> io::Result<()> {
        let title = self.values.entry("window_title".into()).or_default();
        title.text = None;
        title.hidden = true;
        for side in ["attacker", "defender"] {
            for suffix in [
                "leader_initiative_overlay_glow",
                "leader_army_hq_icon",
                "countered_tactic_overlay",
                "tactic",
                "air_icon",
                "air_count",
                "air_text",
            ] {
                hide(&mut self.values, &format!("{side}_{suffix}"));
            }
        }
        hide(&mut self.values, "combat_heading");
        let Some(combat) = rows::<tables::LandCombats>(frame)?
            .rows()
            .iter()
            .find(|r| r.id == id)
        else {
            self.warnings.push(format!("Land combat {id} is absent at hour {}. This does not establish a finished battle or its result; the window is empty.",frame.hour));
            return Ok(());
        };
        let aliases: BTreeMap<_, _> = rows::<tables::CountryLinks>(frame)?
            .rows()
            .iter()
            .map(|r| (r.id, r.canonical_slot))
            .collect();
        let sides = engagement_progress_sides(frame)?;
        let ours = |side: usize| {
            sides.get(&id).is_some_and(|row| {
                [&row.attacker_primary, &row.defender_primary][side]
                    .first()
                    .and_then(|raw| aliases.get(raw))
                    == Some(&vision.ours())
            })
        };
        let modifiers = rows::<tables::LandCombatModifiers>(frame)?;
        let title = match super::details::border_war(modifiers.rows(), id) {
            None => Err(
                "no LandCombatModifiers row records whether it is a border war, so the title is undecided".to_owned(),
            ),
            Some(true) => Ok("BORDER_CONFLICT"),
            Some(false) if !sides.contains_key(&id) => Err(
                "its LandCombatSides row, which holds each side's first primary country, is missing, so the title is undecided".to_owned(),
            ),
            Some(false) if ours(0) => Ok("BATTLE_ATTACK"),
            Some(false) if ours(1) => Ok("BATTLE_DEFENCE"),
            Some(false) => Ok("BATTLE_BATTLE"),
        }
        .and_then(|key| {
            builtin(
                static_content.assets.as_ref(),
                &static_content.localisation,
                key,
                &[],
            )
        });
        self.bind(
            "window_title",
            TEXT,
            title.map_err(|reason| format!("Land combat {id} title is hidden: {reason}.")),
        );
        let progress = Progresses::read(frame)?
            .attacker(id)
            .map(|progress| progress as f32 / FIXED as f32)
            .ok_or_else(|| {
                format!(
                    "Land combat {id} progress bar is hidden: no recorded progress for the combat."
                )
            });
        self.bind("combat_progress", FILL, progress);
        let divisions: BTreeMap<_, _> = rows::<tables::Divisions>(frame)?
            .rows()
            .iter()
            .map(|r| (r.id, r))
            .collect();
        let panels: BTreeMap<_, _> = rows::<tables::DivisionPanelStats>(frame)?
            .rows()
            .iter()
            .map(|r| (r.division, r))
            .collect();
        let templates: BTreeMap<_, _> = rows::<tables::DivisionTemplateIcons>(frame)?
            .rows()
            .iter()
            .map(|r| (r.template, r))
            .collect();
        for (side, ids) in [&combat.attackers, &combat.defenders]
            .into_iter()
            .enumerate()
        {
            let prefix = if side == 0 { "attacker" } else { "defender" };
            let label = builtin(
                static_content.assets.as_ref(),
                &static_content.localisation,
                "COMBAT_COMMITED_LABEL",
                &[("COUNT", Argument::Text("—"))],
            )
            .map_err(|reason| format!("Land combat {prefix} committed label is hidden: {reason}."));
            self.bind(
                &format!("{prefix}_window/{prefix}_committed_label"),
                TEXT,
                label,
            );
            for id in ids {
                let mut row = Values::new();
                hide(&mut row, "unit_button");
                let Some(division) = divisions.get(id) else {
                    self.withhold_in(
                        &mut row,
                        &["unit_icon", "str_bar", "org_bar"],
                        format!(
                            "Land combat division {id}: its Divisions row is missing; its icon and bars are hidden."
                        ),
                    );
                    self.entries[side].push((*id, row));
                    continue;
                };
                let icon = match templates.get(&division.template) {
                    Some(template) => {
                        let icon = if let Some(index) = custom_icon_index(template.custom_icon) {
                            format!("GFX_div_templ_{index}_large")
                        } else {
                            format!(
                                "GFX_unit_{}_icon_medium",
                                String::from_utf8_lossy(&template.priority_unit_name)
                            )
                        };
                        if static_content.assets.has_sprite(&icon) {
                            Ok(icon)
                        } else {
                            Err(format!(
                                "Land combat division icon is hidden: the content lacks sprite {icon}."
                            ))
                        }
                    }
                    None => Err(format!(
                        "Land combat division {id}: template {} has no DivisionTemplateIcons row; its icon is hidden.",
                        division.template
                    )),
                };
                self.bind_in(&mut row, "unit_icon", SPRITE, icon);
                match panels.get(id) {
                    Some(panel) => {
                        for (bar, value, maximum) in [
                            ("str_bar", division.hit_points, panel.maximum_hit_points()),
                            ("org_bar", division.organisation, panel.maximum_organisation()),
                        ] {
                            let value = match maximum {
                                Some(maximum) => ratio(value, maximum).ok_or_else(|| {
                                    format!(
                                        "Land combat division {id}: its recorded {bar} maximum {maximum} is not positive; the bar is hidden."
                                    )
                                }),
                                None => Err(format!(
                                    "Land combat division {id}: its {bar} maximum was refused when recorded; the bar is hidden."
                                )),
                            };
                            self.bind_in(&mut row, bar, FILL, value);
                        }
                    }
                    None => self.withhold_in(
                        &mut row,
                        &["str_bar", "org_bar"],
                        format!(
                            "Land combat division {id}: its DivisionPanelStats row is missing; its bars are hidden."
                        ),
                    ),
                }
                self.entries[side].push((*id, row));
            }
        }
        self.warnings.push("Combat UI: LandCombatDetails/LandCombatDivisions/LandCombatModifiers are not bound in this recording. Commander identity/portrait/skill, division names, active-versus-reserve membership, tactics, combat widths, side strength/result tint, combat heading, effective attack/defense, modifiers, battle weather and CAS values remain empty or dash. DivisionTemplateIcons supplies icons; it does not supply individual division names. Legacy participant order is shown in the main list; commitment placement is unavailable until LandCombatDivisions is available.".into());
        Ok(())
    }
    fn naval(
        &mut self,
        static_content: &StaticContent,
        frame: &DecodedFrame,
        id: u64,
        flags: &Flags,
    ) -> io::Result<()> {
        let prefix = "header_container";
        let province = rows::<tables::NavalCombats>(frame)?
            .rows()
            .iter()
            .find(|row| row.id == id)
            .map(|combat| combat.province);
        let title = match province {
            Some(province) => super::battle_name::title(static_content, frame, province, false)?,
            None => Err(format!("the NavalCombats table has no row for combat {id}")),
        }
        .map_err(|reason| format!("Naval combat {id} title is hidden: {reason}."));
        self.bind(&format!("{prefix}/naval_battle_title"), TEXT, title);
        for key in [
            "progress_defender",
            "carrier_penalty_attacker",
            "carrier_penalty_defender",
            "attacker_stats",
            "defender_stats",
            "leader_attacker_ships_count",
            "leader_defender_ships_count",
        ] {
            hide(&mut self.values, &format!("{prefix}/{key}"));
        }
        let Some(combat) = rows::<tables::NavalCombats>(frame)?
            .rows()
            .iter()
            .find(|r| r.id == id)
        else {
            self.warnings.push(format!("Naval combat {id} is absent at hour {}. Absence is not a result event; no victory, defeat or losses are inferred.",frame.hour));
            return Ok(());
        };
        hide(&mut self.values, &format!("{prefix}/terrain_background"));
        let participants = rows::<tables::NavalCombatParticipants>(frame)?;
        let ships: BTreeMap<_, _> = rows::<tables::Ships>(frame)?
            .rows()
            .iter()
            .map(|r| (r.id, r))
            .collect();
        for side in 0..2 {
            let label = if side == 0 { "attacker" } else { "defender" };
            let wnd = if side == 0 { "left" } else { "right" };
            let members: Vec<_> = participants
                .rows()
                .iter()
                .filter(|r| r.combat == id && r.side == side as u8)
                .collect();
            if let Some(flag) = members.iter().find_map(|r| flags.get(&r.owner)) {
                self.bind(&format!("{prefix}/flag_{label}"), FLAG, flag.clone());
            }
            let positioning = if side == 0 {
                combat.attacker_positioning
            } else {
                combat.defender_positioning
            };
            let anti_air = if side == 0 {
                combat.attacker_anti_air
            } else {
                combat.defender_anti_air
            };
            text(
                &mut self.values,
                &format!("{prefix}/{wnd}_wnd/stat_positioning"),
                content_reader_parser::fixed5::fixed5_percent(positioning, 0, true),
            );
            text(
                &mut self.values,
                &format!("{prefix}/{wnd}_wnd/stat_anti_air"),
                content_reader_parser::fixed5::fixed5_to_string(anti_air, 1),
            );
            for member in members {
                let ship = ships.get(&member.ship).copied();
                let category = ship.map(|s| s.category);
                let Some(column) = naval_column(member.state, category, member.ship == 0) else {
                    self.warnings.push(format!("Naval slot {}: state {} or category {:?} has no verified combat box; omitted.", member.battle_slot, member.state, category));
                    continue;
                };
                let sprite_category = if member.ship == 0 {
                    Some("convoy")
                } else {
                    ship_sprite(category)
                };
                let Some(name) = sprite_category.map(|c| format!("GFX_navalcombat_ship_icon_{c}"))
                else {
                    self.refuse(format!(
                        "Naval combat {id} ship entry: ship category {category:?} has no verified ship icon; the entry is omitted."
                    ));
                    continue;
                };
                let Some(asset) = static_content.assets.sprite_size(&name) else {
                    self.refuse(format!(
                        "Naval combat ship entry: the content lacks sprite {name}; the entry is omitted."
                    ));
                    continue;
                };
                let mut row = Values::new();
                for key in [
                    "icon_hidden",
                    "escape_progress",
                    "escape_icon_left",
                    "escape_icon_right",
                    "pride",
                    "bar_str",
                    "bar_org",
                    "desc_left",
                    "desc_right",
                ] {
                    hide(&mut row, key);
                }
                row.insert(
                    "icon".into(),
                    Widget {
                        sprite: Some(name),
                        frame: Some(super::naval_details::icon_frame(
                            side as u8,
                            matches!(member.state, 8 | 16),
                        )),
                        ..Widget::default()
                    },
                );
                super::naval_details::descriptions(
                    self,
                    &mut row,
                    member.owner,
                    1,
                    side as u8,
                    flags,
                );
                if member.state == 8 {
                    row.insert("escape_progress".into(), Widget::default());
                    row.insert(
                        if side == 0 {
                            "escape_icon_left"
                        } else {
                            "escape_icon_right"
                        }
                        .into(),
                        Widget::default(),
                    );
                }
                let grid = match column {
                    "hidden" | "visible" => {
                        format!("battle_window/middle_wnd/{wnd}_wnd/grid_{column}")
                    }
                    "sunk" => format!("bottom_wnd/{wnd}_wnd/grid"),
                    "reserve" => format!("battle_window/outside_wnd/{wnd}_wnd/grid"),
                    _ => format!("battle_window/top_wnd/{wnd}_wnd/grid_{column}"),
                };
                self.naval_entries.push(NavalEntry {
                    layout: if column == "sunk" {
                        NavalLayout::Sunk
                    } else {
                        NavalLayout::Ship
                    },
                    side: if wnd == "left" { 0 } else { 1 },
                    unknown: false,
                    grid,
                    offset: super::naval_details::entry_offset(column, 0.0, 0.0),
                    size: asset,
                    values: row,
                    battle_slots: Vec::new(),
                });
            }
        }
        self.warnings.push("Combat UI: NavalCombatDetails/NavalCombatShips are not bound in this recording. Screening, phase, fleet/commander identity, ship names/maxima/damage frames, escape progress and results are empty or dash. NavalCombatParticipants supplies side/slot/ship/owner; raw state is not treated as a damage percentage. State 8 reverses the ship frame and shows the escape widget; state 32 uses the sunk strip. Category boxes use sprite-height row spacing. Submarine reveal, damage/model grouping and inline flag placement require NavalCombatShips and remain unverified; legacy slots stay separate. Ship strength/organisation are absolute values and cannot fill bars without maxima.".into());
        Ok(())
    }
}

fn ship_sprite(category: Option<u32>) -> Option<&'static str> {
    match category? {
        2 => Some("submarine"),
        3 => Some("destroyer"),
        5 => Some("light_cruiser"),
        6 => Some("heavy_cruiser"),
        8 => Some("battle_cruiser"),
        9 => Some("battleship"),
        10 => Some("carrier"),
        _ => None,
    }
}
fn naval_column(state: u32, category: Option<u32>, convoy: bool) -> Option<&'static str> {
    match state {
        32 => Some("sunk"),
        4 | 16 => Some("reserve"),
        1 | 2 | 8 => {
            if convoy {
                Some("carrier")
            } else {
                match category? {
                    2 => Some("hidden"),
                    3 | 5 => Some("screen"),
                    6 | 8 | 9 => Some("battle"),
                    10 => Some("carrier"),
                    _ => None,
                }
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_maximum_is_not_an_empty_health_bar() {
        assert_eq!(ratio(50, 0), None);
        assert_eq!(ratio(50, -1), None);
        assert_eq!(ratio(25, 100), Some(0.25));
        assert_eq!(ratio(i64::MAX, i64::MAX), Some(1.0));
    }

    #[test]
    fn naval_states_choose_sections_without_discarding_high_bits() {
        assert_eq!(naval_column(2, Some(3), false), Some("screen"));
        assert_eq!(naval_column(8, Some(9), false), Some("battle"));
        assert_eq!(naval_column(8, None, true), Some("carrier"));
        assert_eq!(naval_column(32, None, false), Some("sunk"));
        assert_eq!(naval_column(4, Some(9), false), Some("reserve"));
        assert_eq!(naval_column(0x102, Some(3), false), None);
        assert_eq!(naval_column(2, None, false), None);
    }
}
