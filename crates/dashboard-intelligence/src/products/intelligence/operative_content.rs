
use dashboard_base::Source;
use dashboard_base::{
    catalogue::{self, Catalogue, View},
    products::{
        character_portraits::{self, RecordedPortrait},
        politics_content::{self, PoliticsContent},
    },
};
use dashboard_core::config::LoadContext;
use engine_native_memory::{cdate::HOUR_EPOCH, cgame_date::NULL_RAW_HOUR};
use content_reader_military::intelligence::Intelligence;
use content_reader_key_names::unit_leader::{Leaders, Skills};
use content_reader_localisation::localisation::{Argument, Localisation};
use replay_data::rows::{CharacterPortraitRow, OperativeMembershipRow, PORTRAIT_LARGE, Recorded};
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, Mutex},
};

pub use super::placements::{CAPTURED, MISSION, OPERATION};

pub const COOLDOWN: u32 = 1;
pub const DISBANDED: u32 = 2;
pub const KILLED: u32 = 5;

const OPERATIVE_ROLE: u32 = 4;
pub const OPERATIVE_CATEGORY: i32 = 3;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Art {
    Portrait(RecordedPortrait),
    Icon { mission: bool, name: String, frame: u32 },
    Sprite(String),
}

pub type Shown = Result<String, String>;

pub struct Name {
    pub name: Option<String>,
    pub unnamed: bool,
    pub missing: Option<String>,
}

#[derive(Debug, Default, PartialEq)]
pub struct Experience {
    pub text: Option<String>,
    pub ratio: Option<f64>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Assignment {
    pub name: Option<String>,
    pub duration: Option<String>,
    pub progress: Option<f64>,
    pub missing: Option<String>,
    pub icon: Option<Art>,
}

pub struct OperativeContent {
    catalogue: Arc<Catalogue>,
    politics: Arc<PoliticsContent>,
    skills: Skills,
    operations: Intelligence,
    capture_days: Result<i32, String>,
    pub warnings: Vec<String>,
    drawn: Mutex<BTreeMap<Art, Shown>>,
}

type Kept = Option<(usize, Arc<OperativeContent>)>;

pub fn content(source: &Source, load: &LoadContext) -> Result<Arc<OperativeContent>, String> {
    let options = load.content().map_err(|error| error.to_string())?;
    let politics = politics_content::politics_content(source, options).map_err(|error| error.to_string())?;
    let binding = Arc::as_ptr(&politics) as usize;
    let state = source.state::<Kept>();
    if let Some((held, content)) = &*state.lock().unwrap_or_else(|error| error.into_inner())
        && *held == binding
    {
        return Ok(Arc::clone(content));
    }
    let content = Arc::new(OperativeContent::load(politics).map_err(|error| error.to_string())?);
    *state.lock().unwrap_or_else(|error| error.into_inner()) = Some((binding, Arc::clone(&content)));
    Ok(content)
}

impl OperativeContent {
    pub fn load(politics: Arc<PoliticsContent>) -> io::Result<Self> {
        let catalogue = Arc::clone(&politics.catalogue);
        let layers = &catalogue.content.layers;
        let leaders = Leaders::<String>::load(layers, &|_| None)?;
        let operations = content_reader_military::intelligence::read(layers)?;
        let capture_days = content_reader_defines::defines::read(layers)
            .and_then(|defines| defines.int32("NOperatives", "OPERATIVE_CAPTURE_DURATION_IN_DAYS"))
            .map_err(|error| format!("NDefines.NOperatives.OPERATIVE_CAPTURE_DURATION_IN_DAYS is unavailable: {error}"));
        let mut warnings = politics.warnings.clone();
        warnings.extend(leaders.misreads.iter().cloned());
        warnings.extend(leaders.skills.warnings().iter().cloned());
        warnings.extend(operations.warnings.iter().cloned());
        Ok(Self { catalogue, politics, skills: leaders.skills, operations, capture_days, warnings, drawn: Mutex::default() })
    }

    pub fn check(&self) -> Vec<(String, String)> {
        let mut shown: Vec<(String, Assignment)> = (0..=8).map(|variant| (format!("mission {variant}"), self.mission(variant))).collect();
        shown.extend([2, 3].map(|kind| (format!("cooldown {kind}"), self.cooldown_name(kind))));
        shown.extend(self.operations.operations.keys().map(|token| (format!("operation {token}"), self.operation(token))));
        let mut refused: Vec<(String, String)> = self.capture_days.as_ref().err().map(|why| ("captivity days".to_owned(), why.clone())).into_iter().collect();
        for (subject, assignment) in shown {
            let icon = assignment.icon.as_ref().and_then(|art| self.draw(art).err());
            refused.extend(assignment.missing.into_iter().chain(icon).map(|why| (subject.clone(), why)));
        }
        refused.extend(self.operations.operation_refusals.iter().map(|why| ("operations".to_owned(), why.clone())));
        refused
    }

    pub fn localisation(&self) -> &Localisation {
        &self.catalogue.localisation
    }

    pub fn draw(&self, art: &Art) -> Shown {
        if let Some(drawn) = self.drawn.lock().unwrap_or_else(|error| error.into_inner()).get(art) {
            return drawn.clone();
        }
        let drawn = match art {
            Art::Portrait(portrait) => character_portraits::portrait_png(&self.politics, portrait),
            Art::Icon { mission, name, frame } => icon_png(&self.catalogue, *mission, name, *frame),
            Art::Sprite(name) => sprite_png(&self.catalogue, name),
        };
        self.drawn.lock().unwrap_or_else(|error| error.into_inner()).insert(art.clone(), drawn.clone());
        drawn
    }

    pub fn portrait(&self, character: Option<u32>, portraits: Option<&[CharacterPortraitRow]>) -> Result<Art, String> {
        let Some(character) = character else {
            return Err("No recorded operative-to-character portrait link; the operative's own portrait path is not recorded".into());
        };
        match character_portraits::recorded_portrait(portraits, character, OPERATIVE_ROLE, PORTRAIT_LARGE) {
            selected @ RecordedPortrait::Entry { .. } => Ok(Art::Portrait(selected)),
            other => Err(character_portraits::portrait_png(&self.politics, &other).expect_err("only a recorded entry is drawn")),
        }
    }

    pub fn experience(&self, skill: Option<i32>, raw: i64) -> Experience {
        experience(self.localisation(), &self.skills, skill, raw)
    }

    pub fn assignment(&self, row: &OperativeMembershipRow, now: i32, operation: Option<&str>, capturer: Option<&str>) -> Assignment {
        match row.assignment {
            CAPTURED => self.captured(row.captured_hour, now, capturer),
            COOLDOWN => match cooldown(row) {
                Ok(Some((kind, start, end))) => self.cooldown(kind, start, end, now),
                Ok(None) => Assignment::default(),
                Err(reason) => Assignment { missing: Some(reason), ..Assignment::default() },
            },
            DISBANDED => Assignment { missing: Some("Disbanded assignment has no active agency status text.".into()), ..Assignment::default() },
            MISSION => match mission_variant(row.action_variant) {
                Ok(_) => self.mission(row.action_variant.unwrap_or(0)),
                Err(reason) => Assignment { missing: Some(reason), ..Assignment::default() },
            },
            OPERATION => match operation {
                Some(token) => self.operation(token),
                None => Assignment { missing: Some("The operation definition was not recorded at this read.".into()), ..Assignment::default() },
            },
            KILLED => Assignment { missing: Some("Killed assignment has no active agency status text.".into()), ..Assignment::default() },
            other => Assignment { missing: Some(format!("Unsupported native operative assignment {other}.")), ..Assignment::default() },
        }
    }

    fn days(&self, days: i32) -> Shown {
        catalogue::localised_with(self.localisation(), "DURATION_DAYS", &[("NUM", Argument::Int32(days))], View::Plain)
    }

    fn captured(&self, captured_hour: i32, now: i32, capturer: Option<&str>) -> Assignment {
        let mut status = match capturer {
            Some(name) => named(catalogue::localised_with(self.localisation(), "OPERATIVE_CAPTURED_BY", &[("CAPTURER", Argument::Text(name))], View::Plain)),
            None => Assignment { missing: Some("The capturer's name at this hour is unknown.".into()), ..Assignment::default() },
        };
        let duration = if captured_hour == NULL_RAW_HOUR {
            Err("Captured duration and progress have no recorded capture date.".to_owned())
        } else {
            self.capture_days.clone()
        };
        let duration = duration.and_then(|duration| {
            let (remaining, progress) = capture_progress(captured_hour, now, duration);
            status.progress = Some(f64::from(progress));
            self.days(remaining)
        });
        match duration {
            Ok(duration) => status.duration = Some(duration),
            Err(reason) => status.missing = Some(joined(status.missing.take(), reason)),
        }
        status
    }

    fn cooldown(&self, kind: u32, start: i32, end: i32, now: i32) -> Assignment {
        let (remaining, progress) = cooldown_progress(kind, start, end, now);
        let mut status = self.cooldown_name(kind);
        status.progress = Some(f64::from(progress));
        match self.days(remaining) {
            Ok(duration) => status.duration = Some(duration),
            Err(reason) => status.missing = Some(joined(status.missing.take(), reason)),
        }
        status
    }

    fn cooldown_name(&self, kind: u32) -> Assignment {
        let key = match kind {
            2 => "OPERATIVE_HARMED_IN_MISSION",
            3 => "OPERATIVE_FORCED_INTO_HIDING",
            _ => return Assignment { name: Some(String::new()), ..Assignment::default() },
        };
        named(catalogue::localised_with(self.localisation(), key, &[], View::Plain))
    }

    fn mission(&self, variant: u32) -> Assignment {
        let Some(key) = mission_key(variant) else {
            return Assignment { missing: Some(format!("Unsupported native operative mission discriminator {variant}.")), ..Assignment::default() };
        };
        let mut value = named(catalogue::localised_with(self.localisation(), &format!("{key}_TITLE"), &[], View::Plain));
        value.icon = (variant != 0).then(|| Art::Icon { mission: true, name: "operative_mission_icons_small".to_owned(), frame: variant - 1 });
        value
    }

    pub fn operation_tokens(&self) -> impl Iterator<Item = &str> {
        self.operations.operations.keys().map(String::as_str)
    }

    pub fn operation_type(&self, token: &str) -> (Shown, Result<Art, String>) {
        match self.operations.operation(token) {
            Ok(Some(metadata)) => (
                catalogue::localised_with(self.localisation(), &metadata.name, &[], View::Plain),
                if metadata.sprite.is_empty() { Err(format!("Operation {token} authors no icon")) } else { Ok(Art::Sprite(metadata.sprite.clone())) },
            ),
            Ok(None) => {
                let why = format!("Recorded content does not define operation {token}.");
                (Err(why.clone()), Err(why))
            }
            Err(reason) => (Err(reason.clone()), Err(reason)),
        }
    }

    fn operation(&self, token: &str) -> Assignment {
        let metadata = match self.operations.operation(token) {
            Ok(Some(metadata)) => metadata,
            Ok(None) => return Assignment { missing: Some(format!("Recorded content does not define operation {token}.")), ..Assignment::default() },
            Err(reason) => return Assignment { missing: Some(reason), ..Assignment::default() },
        };
        let mut value = named(catalogue::localised_with(self.localisation(), &metadata.name, &[], View::Plain));
        value.icon = (!metadata.map_icon.is_empty()).then(|| Art::Icon { mission: false, name: metadata.map_icon.clone(), frame: 0 });
        value
    }
}

fn joined(first: Option<String>, then: String) -> String {
    match first {
        Some(first) => format!("{first} {then}"),
        None => then,
    }
}

fn named(name: Shown) -> Assignment {
    match name {
        Ok(name) => Assignment { name: Some(name), ..Assignment::default() },
        Err(missing) => Assignment { missing: Some(missing), ..Assignment::default() },
    }
}

fn icon_png(catalogue: &Catalogue, mission: bool, name: &str, frame: u32) -> Shown {
    let context = if mission { format!("Mission text icon {name}") } else { format!("Operative icon {name}") };
    let sprite = if mission { catalogue::text_icon_sprite(&catalogue.sprites, name) } else { catalogue::created_gui_sprite(&catalogue.sprites, name) }
        .map_err(|error| format!("{context}: {error}"))?;
    let decode = *catalogue.gui_decode.as_ref().map_err(|why| format!("{context}: {why}"))?;
    catalogue::sprite_frame_png(&catalogue.content.layers, sprite, decode, frame, 96).map_err(|error| {
        let context = if mission { format!("{context} frame{}", frame + 1) } else { context };
        format!("{context}: {error}")
    })
}

fn sprite_png(catalogue: &Catalogue, name: &str) -> Shown {
    let context = format!("Operation icon {name}");
    let sprite = catalogue.sprites.lookup(name).map_err(|error| format!("{context}: {error}"))?;
    let decode = *catalogue.gui_decode.as_ref().map_err(|why| format!("{context}: {why}"))?;
    catalogue::sprite_frame_png(&catalogue.content.layers, sprite, decode, 0, 96).map_err(|error| format!("{context}: {error}"))
}

pub fn name(localisation: Option<&Localisation>, row: &OperativeMembershipRow) -> Name {
    let bytes = match &row.name {
        Recorded::Value(bytes) => bytes,
        Recorded::NotRecorded => return Name { name: None, unnamed: false, missing: Some("name of operative memberships was not recorded".into()) },
    };
    let literal = Localisation::default();
    match catalogue::localised_bytes(localisation.unwrap_or(&literal), bytes, View::Plain) {
        Ok(name) => Name { unnamed: name.is_empty(), name: (!name.is_empty()).then_some(name), missing: None },
        Err(reason) => Name { name: None, unnamed: false, missing: Some(reason) },
    }
}

pub fn cooldown(row: &OperativeMembershipRow) -> Result<Option<(u32, i32, i32)>, String> {
    if row.assignment != COOLDOWN {
        return Ok(None);
    }
    let kind = *row.cooldown_kind.require("cooldown kind of operative memberships")?;
    let start = *row.cooldown_start_hour.require("cooldown start hour of operative memberships")?;
    let end = *row.cooldown_end_hour.require("cooldown end hour of operative memberships")?;
    match (kind, start, end) {
        (Some(kind), Some(start), Some(end)) => Ok(Some((kind, start, end))),
        _ => Err("Cooldown kind or dates were not recorded".into()),
    }
}

pub fn mission_variant(variant: Option<u32>) -> Result<bool, String> {
    match variant {
        None | Some(0) => Ok(false),
        Some(1..=8) => Ok(true),
        Some(variant) => Err(format!("Unsupported operative mission variant {variant}")),
    }
}

pub fn capture_progress(captured_hour: i32, now: i32, duration: i32) -> (i32, f32) {
    let captured_day = captured_hour.wrapping_sub(HOUR_EPOCH) / 24;
    let now_day = now.wrapping_sub(HOUR_EPOCH) / 24;
    let remaining = captured_day.wrapping_sub(now_day).wrapping_add(duration);
    let elapsed = now_day.wrapping_sub(captured_day).max(0);
    let denominator = elapsed.wrapping_add(remaining).max(1);
    (remaining, elapsed as f32 / denominator as f32)
}

pub fn cooldown_remaining(kind: u32, end: i32, now: i32) -> i32 {
    if kind == 0 {
        return 0;
    }
    let end = end.wrapping_sub(HOUR_EPOCH);
    let now = now.wrapping_sub(HOUR_EPOCH);
    end.wrapping_add(23_i32.wrapping_sub(now)) / 24
}

pub fn cooldown_progress(kind: u32, start: i32, end: i32, now: i32) -> (i32, f32) {
    let start_day = start.wrapping_sub(HOUR_EPOCH) / 24;
    let end_day = end.wrapping_sub(HOUR_EPOCH) / 24;
    let duration = end_day.wrapping_sub(start_day).max(1);
    let remaining = cooldown_remaining(kind, end, now);
    let elapsed = duration.wrapping_sub(remaining).clamp(0, duration);
    (remaining, elapsed as f32 / duration as f32)
}

fn mission_key(variant: u32) -> Option<&'static str> {
    Some(match variant {
        0 => "OPERATIVE_MISSION_NO_MISSION",
        1 => "OPERATIVE_MISSION_BUILD_INTEL_NETWORK",
        2 => "OPERATIVE_MISSION_QUIET_INTEL_NETWORK",
        3 => "OPERATIVE_MISSION_COUNTER_INTELLIGENCE",
        4 => "OPERATIVE_MISSION_ROOT_OUT_RESISTANCE",
        5 => "OPERATIVE_MISSION_BOOST_IDEOLOGY",
        6 => "OPERATIVE_MISSION_CONTROL_TRADE",
        7 => "OPERATIVE_MISSION_DIPLOMATIC_PRESSURE",
        8 => "OPERATIVE_MISSION_PROPAGANDA",
        _ => return None,
    })
}

pub fn native_progress(raw: i64, cost: i32) -> i64 {
    let denominator = i64::from(cost) * 100_000;
    if denominator != 0 && (raw as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd {
        raw.wrapping_mul(100_000) / denominator
    } else {
        0xffff_ffff
    }
}

pub fn experience(localisation: &Localisation, skills: &Skills, skill: Option<i32>, raw: i64) -> Experience {
    let Some(skill) = skill else {
        return Experience { missing: Some("The operative's native skill definition was not recorded.".into()), ..Experience::default() };
    };
    if !skills.conditions().is_empty() {
        let conditions: Vec<String> = skills.conditions().iter().map(|condition| format!("{} at {}", condition.modifier, condition.place)).collect();
        return Experience { missing: Some(format!("Operative skill levels depend on an unrecorded modifier registry: {}.", conditions.join("; "))), ..Experience::default() };
    }
    let Some(cost) = skills.next_required_experience(skill, OPERATIVE_CATEGORY) else {
        return Experience { missing: Some(format!("Operative skill levels are unsupported: {}", skills.warnings().join("; "))), ..Experience::default() };
    };
    let progress = cost.map(|cost| native_progress(raw, cost));
    let text = match progress {
        Some(progress) => catalogue::localised_with(localisation, "LEADER_SKILL_WITH_PROGRESS", &[("SKILL", Argument::Int32(skill)), ("PROGRESS", Argument::Fixed5(progress))], View::Plain),
        None => catalogue::localised_with(localisation, "LEADER_SKILL_AT_CAP", &[("SKILL", Argument::Int32(skill))], View::Plain),
    };
    let ratio = progress.map(|raw| raw as f64 / 100_000.0);
    match text {
        Ok(text) => Experience { text: Some(text), ratio, missing: None },
        Err(missing) => Experience { text: None, ratio, missing: Some(missing) },
    }
}
