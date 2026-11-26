use crate::products::focus::{self, Fold, FocusHourParams, FocusMissing};
use crate::products::focus_artwork::{self, DatedFocusArtwork, FocusArtworkParams};
use dashboard_base::Source;
use dashboard_base::products::{LoadContext, Product, check_reply_budget, validate_country};
use dashboard_core::dto::FocusDatabase;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub(crate) const UNDATED: &str = "The focus was already complete when the list was first read";
pub(crate) const NO_GAIN: &str = "Daily progress was not recorded";
pub(crate) const NO_DURATION: &str = "The focus's duration was not recorded";
const FIXED: i64 = 100_000;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FocusCardArt {
    pub token: String,
    pub name: Option<String>,
    pub name_missing: Option<String>,
    pub icon: Option<String>,
    pub icon_missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FocusCardCompleted {
    pub art: FocusCardArt,
    pub completed_by_hour: Option<i32>,
    pub date_missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FocusCardCurrent {
    pub art: FocusCardArt,
    pub continuous: bool,
    pub paused: bool,
    pub total_days: Option<f64>,
    pub elapsed_days: Option<u32>,
    pub permyriad: Option<u32>,
    pub days_left: Option<u32>,
    pub progress_missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FocusCardReply {
    pub hour: i32,
    pub valid_through: i32,
    pub missing: Option<FocusMissing>,
    pub current: Option<FocusCardCurrent>,
    pub completed: Vec<FocusCardCompleted>,
    pub undated: u32,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(FocusCardReply);

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CardShape {
    pub valid_through: i32,
    pub missing: Option<FocusMissing>,
    pub current: Option<(String, bool, bool)>,
    pub progress: Option<(i64, Option<i64>)>,
    pub completed: Vec<(String, Option<i32>, i32)>,
    pub undated: u32,
}

fn first_seen(fold: &Fold, at: usize, ordinal: u32, token: &str) -> i32 {
    let held = |segment: &crate::products::focus::Segment| segment.order.completed.iter().any(|entry| entry.ordinal == ordinal && entry.token == token);
    let mut first = fold.segments[at].first;
    for back in (0..at).rev() {
        let (earlier, later) = (&fold.segments[back], &fold.segments[back + 1]);
        if earlier.id != later.id || earlier.slot != later.slot || earlier.last.saturating_add(1) != later.first || !held(earlier) {
            break;
        }
        first = earlier.first;
    }
    first
}

pub(crate) fn card_at(fold: &Fold, hour: i32) -> CardShape {
    let Some((index, segment)) = fold.segment_at(hour) else {
        let at = fold.missing.partition_point(|missing| missing.last_hour < hour);
        let missing = fold.missing.get(at).filter(|missing| missing.first_hour <= hour).cloned();
        let next_segment = fold.segments.iter().map(|segment| segment.first).find(|first| *first > hour);
        let valid_through = missing.as_ref().map(|missing| missing.last_hour).or(next_segment.map(|first| first - 1)).unwrap_or(hour);
        return CardShape { valid_through, missing, current: None, progress: None, completed: Vec::new(), undated: 0 };
    };
    let mut dated: Vec<(String, Option<i32>, i32)> = Vec::new();
    let mut undated: Vec<(String, Option<i32>, i32)> = Vec::new();
    for entry in segment.order.completed.iter().rev() {
        match entry.completed_by_hour {
            Some(when) => dated.push((entry.token.clone(), Some(when), when)),
            None => undated.push((entry.token.clone(), None, first_seen(fold, index, entry.ordinal, &entry.token))),
        }
    }
    dated.sort_by_key(|(_, when, _)| std::cmp::Reverse(*when));
    undated.reverse();
    let undated_count = undated.len() as u32;
    dated.extend(undated);
    let current = match (&segment.order.current, &segment.order.continuous) {
        (Some(token), _) => Some((token.clone(), false, segment.order.paused)),
        (None, Some(token)) => Some((token.clone(), true, segment.order.paused)),
        _ => None,
    };
    let at = fold.progress.partition_point(|run| run.last < hour);
    let ordinary = current.as_ref().is_some_and(|(_, continuous, _)| !continuous);
    let progress = if ordinary { fold.progress(hour) } else { None };
    let valid_through = if ordinary && let Some(run) = fold.progress.get(at) {
        segment.last.min(if run.first <= hour { run.last } else { run.first - 1 })
    } else {
        segment.last
    };
    CardShape { valid_through, missing: None, current, progress, completed: dated, undated: undated_count }
}

pub(crate) fn progress_figures(progress: Option<(i64, Option<i64>)>, total_days: Option<f64>) -> (Option<u32>, Option<u32>, Option<u32>, Option<String>) {
    let Some((raw, gain)) = progress else {
        return (None, None, None, Some(NO_GAIN.into()));
    };
    let Some(days) = total_days.filter(|days| days.is_finite() && *days > 0.0) else {
        return (None, None, None, Some(NO_DURATION.into()));
    };
    let total = (days * FIXED as f64).round() as i64;
    let done = raw.clamp(0, total);
    let remaining = total - done;
    let permyriad = u32::try_from(done * 10_000 / total).unwrap_or(10_000);
    let elapsed = u32::try_from(done / FIXED).unwrap_or(0);
    match gain {
        Some(gain) if gain > 0 => {
            let left = u32::try_from((remaining + gain - 1) / gain).unwrap_or(u32::MAX);
            (Some(elapsed), Some(permyriad), Some(left), None)
        }
        _ => (Some(elapsed), Some(permyriad), None, Some(NO_GAIN.into())),
    }
}

type Shown = BTreeMap<String, DatedFocusArtwork>;
type Resolved = Arc<(Shown, Vec<String>, i32)>;
type ArtKey = (FocusDatabase, i32, Vec<String>);

fn resolved_at(source: &Source, context: &LoadContext, params: &FocusHourParams, branch: u64, key: ArtKey, cancelled: &AtomicBool) -> io::Result<Resolved> {
    let (database, at, tokens) = key.clone();
    dashboard_base::products::breakdown::held(source, context, (branch, params.country.clone(), key), || shown(source, context, params, at, database, tokens, cancelled))
}

pub(crate) fn observed_hours(shape: &CardShape) -> BTreeMap<i32, Vec<String>> {
    let mut by_hour: BTreeMap<i32, Vec<String>> = BTreeMap::new();
    for (token, _, seen) in &shape.completed {
        by_hour.entry(*seen).or_default().push(token.clone());
    }
    for tokens in by_hour.values_mut() {
        tokens.sort_unstable();
        tokens.dedup();
    }
    by_hour
}

fn shown(source: &Source, context: &LoadContext, params: &FocusHourParams, at: i32, database: FocusDatabase, tokens: Vec<String>, cancelled: &AtomicBool) -> io::Result<(Shown, Vec<String>, i32)> {
    if tokens.is_empty() {
        return Ok((Shown::new(), Vec::new(), at));
    }
    let asked = FocusArtworkParams {
        branch_id: params.branch_id.clone(),
        hour: at,
        country: params.country.clone(),
        database,
        tokens,
        text_only: None,
        include_descriptions: Some(false),
        icons_only: None,
    };
    match focus_artwork::load(source, &asked, context, cancelled) {
        Ok(reply) => Ok((reply.items.into_iter().map(|item| (item.artwork.token.clone(), item)).collect(), reply.warnings, reply.valid_through)),
        Err(error) if error.kind() == io::ErrorKind::Interrupted => Err(error),
        Err(error) => Ok((Shown::new(), vec![format!("Focus names and icons refused: {error}")], at)),
    }
}

fn art_of(token: &str, shown: &Shown, fallback: &str) -> FocusCardArt {
    let item = shown.get(token);
    let name = item.and_then(|item| item.artwork.name.clone()).filter(|name| !name.trim().is_empty());
    let icon = item.and_then(|item| item.artwork.icon.clone());
    let reason = item.and_then(|item| item.reason.clone()).unwrap_or_else(|| fallback.to_owned());
    FocusCardArt {
        token: token.to_owned(),
        name_missing: name.is_none().then(|| reason.clone()),
        name,
        icon_missing: icon.is_none().then_some(reason),
        icon,
    }
}

pub struct FocusCard;

impl Product for FocusCard {
    const KEY: &'static str = "focus-card";
    const CONTENT_DEPENDENT: bool = true;
    type Params = FocusHourParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, hour: Read];
    type Reply = FocusCardReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![FocusHourParams::decl(), FocusCardArt::decl(), FocusCardCompleted::decl(), FocusCardCurrent::decl(), FocusCardReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        validate_country(&params.country)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let fold = focus::fold(source, branch, &params.country, cancelled)?;
        let shape = card_at(&fold, params.hour);
        const UNNAMED: &str = "The focus's name and icon were not recorded";
        let mut warnings: Vec<String> = Vec::new();
        let mut completed_art: BTreeMap<(i32, String), FocusCardArt> = BTreeMap::new();
        for (hour, tokens) in observed_hours(&shape) {
            let held = resolved_at(source, context, params, branch, (FocusDatabase::National, hour, tokens.clone()), cancelled)?;
            warnings.extend(held.1.iter().cloned());
            for token in tokens {
                let art = art_of(&token, &held.0, UNNAMED);
                completed_art.insert((hour, token), art);
            }
        }
        let completed = shape
            .completed
            .iter()
            .map(|(token, when, seen)| {
                let art = completed_art.get(&(*seen, token.clone())).cloned().unwrap_or_else(|| art_of(token, &Shown::new(), UNNAMED));
                FocusCardCompleted { art, completed_by_hour: *when, date_missing: when.is_none().then(|| UNDATED.to_owned()) }
            })
            .collect();
        let mut valid_through = shape.valid_through;
        let current = match shape.current.as_ref() {
            None => None,
            Some((token, is_continuous, paused)) => {
            let database = if *is_continuous { FocusDatabase::Continuous } else { FocusDatabase::National };
            let held = resolved_at(source, context, params, branch, (database, params.hour, vec![token.clone()]), cancelled)?;
            warnings.extend(held.1.iter().cloned());
            valid_through = valid_through.min(held.2);
            let total_days = held.0.get(token).and_then(|item| item.artwork.base_duration_days);
            let (elapsed_days, permyriad, days_left, progress_missing) =
                if *is_continuous { (None, None, None, None) } else { progress_figures(shape.progress, total_days) };
            Some(FocusCardCurrent { art: art_of(token, &held.0, UNNAMED), continuous: *is_continuous, paused: *paused, total_days, elapsed_days, permyriad, days_left, progress_missing })
            }
        };
        warnings.sort_unstable();
        warnings.dedup();
        let reply = FocusCardReply {
            hour: params.hour,
            valid_through: valid_through.max(params.hour),
            missing: shape.missing,
            current,
            completed,
            undated: shape.undated,
            warnings,
        };
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}
