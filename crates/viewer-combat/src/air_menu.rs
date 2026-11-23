mod aircraft;
pub(super) mod combat;
mod equipment;
pub(super) mod history;
mod power;
pub(crate) mod summary;

use super::{binding::Bindings, content::Content};
use content_reader_localisation::localisation::Argument;
use replay_frame::frame::DecodedFrame;
use std::io;
use viewer_engagements::combat_selection::Selection;
use viewer_foundation::{perspective::Vision, tooltip_text::displayable};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Details,
    Enemy(bool),
    Mission(usize),
    Range(usize),
    Channel(usize),
}

impl Action {
    pub(crate) fn from_path(path: &str) -> Option<Self> {
        let name = path.rsplit('/').next()?;
        Some(match name {
            "details_open" => Self::Details,
            "our_missions_btn" => Self::Enemy(false),
            "enemy_missions" => Self::Enemy(true),
            "btn_days" => Self::Range(0),
            "btn_months" => Self::Range(1),
            "btn_years" => Self::Range(2),
            "channel_checkbox" => Self::Channel(
                path.split('/')
                    .find_map(|p| p.strip_prefix("channel_")?.parse::<usize>().ok())?
                    .checked_sub(1)?,
            ),
            _ if name.starts_with("btn_channel_") => Self::Channel(
                name.strip_prefix("btn_channel_")?
                    .parse::<usize>()
                    .ok()?
                    .checked_sub(1)?,
            ),
            _ if name.starts_with("mission_btn_") => Self::Mission(
                name.strip_prefix("mission_btn_")?
                    .parse::<usize>()
                    .ok()?
                    .checked_sub(1)?,
            ),
            _ => return None,
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct State {
    pub details: bool,
    pub enemy: bool,
    pub missions: u32,
    pub mission_groups: Result<[u32; 3], String>,
    pub range: usize,
    pub channels: u32,
    pub shift: bool,
    pub control: bool,
    pub initial_side: bool,
    pub pass: Result<Option<i32>, String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            details: false,
            enemy: false,
            missions: replay_derive::air_region_derive::DEFAULT_MISSIONS,
            mission_groups: Err("air mission filters need the recorded DLC mask".into()),
            range: 0,
            channels: 0,
            shift: false,
            control: false,
            initial_side: true,
            pass: Err("the air combat daily roster history was not read".into()),
        }
    }
}

impl State {
    pub fn click(&mut self, action: &Action) {
        match *action {
            Action::Details => self.details = !self.details,
            Action::Enemy(enemy) => {
                self.enemy = enemy;
                self.initial_side = false;
            }
            Action::Mission(index) => {
                if let Ok(groups) = &self.mission_groups
                    && let Some(&mask) = groups.get(index)
                {
                    if self.control {
                        let all = groups.iter().fold(0, |all, mask| all | mask);
                        self.missions = if self.missions & all == 0 { all } else { 0 };
                        return;
                    }
                    if self.shift {
                        self.missions = mask;
                        return;
                    }
                    if self.missions & mask != 0 {
                        self.missions &= !mask;
                    } else {
                        self.missions |= mask;
                    }
                }
            }
            Action::Range(range) => self.range = range,
            Action::Channel(channel) if channel < 24 => {
                self.channels ^= 1 << channel;
            }
            Action::Channel(_) => {}
        }
    }
}

#[derive(Debug)]
pub(crate) struct Chart {
    pub graphs: Vec<Vec<[f32; 4]>>,
}

pub(super) fn loc(
    content: &Content,
    key: &str,
    arguments: &[(&str, Argument<'_>)],
) -> Result<String, String> {
    content
        .localisation
        .localise_with(key.as_bytes(), arguments)
        .map_err(|why| format!("air menu {key}: {why}"))
        .and_then(|text| displayable(content.assets.as_ref(), text))
}

pub(super) fn text(out: &mut Bindings, key: &str, value: Result<String, String>) {
    out.bind(key, super::binding::TEXT, value);
}

pub(super) fn bind_hover(
    out: &mut Bindings,
    key: &str,
    primary: Result<String, String>,
    delayed: Result<String, String>,
) {
    let mut counted = |value: Result<String, String>| match value {
        Ok(value) => super::tooltips::hover_text(&value),
        Err(why) => {
            out.refuse(why);
            None
        }
    };
    let primary = counted(primary);
    let delayed = counted(delayed);
    super::tooltips::set_hover(&mut out.values, key, primary, delayed);
}

pub(super) fn seed(content: &Content, out: &mut Bindings) {
    let Some(root) = content.assets.native_windows.gui_type(b"strategicairview") else {
        out.refuse("air combat window is absent from the recorded content stack".into());
        return;
    };
    for (path, element) in super::gui::elements(root) {
        let authored = element
            .text_box()
            .map(|v| v.text.as_slice())
            .or_else(|| element.button().map(|v| v.text.as_slice()));
        if let Some(authored) = authored {
            let value = std::str::from_utf8(authored)
                .map_err(|_| "air menu text key is not UTF-8".to_owned())
                .and_then(|key| {
                    if key.is_empty() || key.starts_with("AIRVIEW_DETAILS_CHANNEL_") {
                        Ok(String::new())
                    } else if key
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || byte == b'.')
                    {
                        Ok(key.to_owned())
                    } else {
                        loc(content, key, &[])
                    }
                });
            text(out, &path, value);
        }
    }
}

pub(super) fn bind(
    content: &Content,
    frame: &DecodedFrame,
    vision: &Vision,
    region: u32,
    state: &State,
) -> io::Result<Bindings> {
    content.assets.prepare_air_icons(&content.scripted);
    let mut out = Bindings::default();
    seed(content, &mut out);
    let name = content
        .strategic_regions
        .get(&region)
        .ok_or_else(|| "air combat region is absent from the content stack".to_owned())
        .and_then(|definition| loc(content, &definition.name_key, &[]));
    text(&mut out, "region_name", name);
    out.values.entry("details_wnd".into()).or_default().hidden = !state.details;
    out.values.entry("details_open".into()).or_default().frame =
        Some(if state.details { 1 } else { 0 });
    for (key, selected) in [
        ("our_missions_btn", !state.enemy),
        ("enemy_missions", state.enemy),
    ] {
        out.values
            .entry(format!("air_combat_wnd/{key}"))
            .or_default()
            .frame = Some(if selected { 1 } else { 0 });
    }
    for index in 0..3 {
        let key = format!("air_combat_wnd/mission_btn_win/mission_btn_{}", index + 1);
        match &state.mission_groups {
            Ok(groups) => {
                out.values.entry(key.clone()).or_default().frame =
                    Some(if state.missions & groups[index] != 0 {
                        0
                    } else {
                        1
                    })
            }
            Err(why) => out.withhold(&[&key], why.clone()),
        }
        let tooltip = state
            .mission_groups
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|groups| mission_tooltip(content, groups, index));
        let delayed = state
            .mission_groups
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|groups| {
                let mut text = loc(content, "AIR_VIEW_FILTER_CLICK", &[])?;
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(&loc(
                    content,
                    if state.missions & groups.iter().fold(0, |all, mask| all | mask) != 0 {
                        "AIR_VIEW_FILTER_VIEW_NONE"
                    } else {
                        "AIR_VIEW_FILTER_VIEW_ALL"
                    },
                    &[],
                )?);
                Ok(text)
            });
        bind_hover(&mut out, &key, tooltip, delayed);
    }
    for (key, loc_key) in [
        ("our_missions_btn", "AIR_VIEW_SHOW_OUR_MISSIONS"),
        ("enemy_missions", "AIR_VIEW_SHOW_ENEMY_MISSIONS"),
    ] {
        bind_hover(
            &mut out,
            &format!("air_combat_wnd/{key}"),
            loc(content, loc_key, &[]),
            Ok(String::new()),
        );
    }
    summary::bind(content, frame, vision, region, &mut out);
    combat::bind(content, frame, vision, region, state, &mut out);
    history::bind(content, frame, vision, region, state, &mut out);
    super::tooltips::bind_authored(
        &mut out,
        content.assets.as_ref(),
        &content.localisation,
        &content.assets.native_windows,
        Selection::AirRegion(region),
    );
    Ok(out)
}

fn mission_tooltip(content: &Content, groups: &[u32; 3], index: usize) -> Result<String, String> {
    use replay_data::rows::{
        AIR_MENU_LOGISTICS_MISSIONS, AIR_MENU_MINE_MISSIONS, AIR_MENU_RECON_MISSIONS,
        AIR_MENU_SUPPLY_MISSIONS,
    };
    let (header, mut missions) = match index {
        0 => (
            "FIGHTER_SUPPORT_MISSIONS",
            vec!["AIR_SUPERIORITY", "CAS", "INTERCEPTION"],
        ),
        1 => (
            "NAVAL_MISSIONS",
            vec![
                "NAVAL_BOMBER",
                "NAVAL_KAMIKAZE",
                "PORT_STRIKE",
                "NAVAL_PATROL",
            ],
        ),
        _ => (
            if groups[2] & (AIR_MENU_LOGISTICS_MISSIONS | AIR_MENU_SUPPLY_MISSIONS) != 0 {
                "BOMBING_LOGISTICS_MISSIONS"
            } else {
                "BOMBING_MISSIONS"
            },
            vec!["STRATEGIC_BOMBER"],
        ),
    };
    match index {
        0 if groups[0] & AIR_MENU_RECON_MISSIONS != 0 => missions.push("RECON"),
        1 if groups[1] & AIR_MENU_MINE_MISSIONS != 0 => {
            missions.extend(["NAVAL_MINE_SWEEPING", "NAVAL_MINE_PLANTING"])
        }
        2 => {
            if groups[2] & AIR_MENU_LOGISTICS_MISSIONS != 0 {
                missions.push("ATTACK_LOGISTICS");
            }
            if groups[2] & AIR_MENU_SUPPLY_MISSIONS != 0 {
                missions.push("AIR_SUPPLY");
            }
        }
        _ => {}
    }
    let mut text = loc(content, &format!("AIRWING_MISSION_TYPE_{header}"), &[])?;
    for mission in missions {
        text.push_str(&format!(
            "\n\u{11}Y{}\u{11}!",
            loc(content, &format!("AIRWING_MISSION_TYPE_{mission}"), &[])?
        ));
    }
    Ok(text)
}
