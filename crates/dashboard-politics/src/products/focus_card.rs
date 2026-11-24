use crate::products::focus::{self, Fold, FocusHourParams, FocusMissing};
use crate::products::focus_artwork::{DatedFocusArtwork, FocusArtwork, FocusArtworkParams};
use dashboard_base::Source;
use dashboard_base::products::{Country, LoadContext, Product, check_reply_budget, validate_country};
use dashboard_core::dto::FocusDatabase;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, PoisonError, atomic::AtomicBool},
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
    pub completed: Vec<(String, Option<i32>)>,
    pub undated: u32,
    pub segment_first: Option<i32>,
}

pub(crate) fn card_at(fold: &Fold, hour: i32) -> CardShape {
    let Some((_, segment)) = fold.segment_at(hour) else {
        let at = fold.missing.partition_point(|missing| missing.last_hour < hour);
        let missing = fold.missing.get(at).filter(|missing| missing.first_hour <= hour).cloned();
        let next_segment = fold.segments.iter().map(|segment| segment.first).find(|first| *first > hour);
        let valid_through = missing.as_ref().map(|missing| missing.last_hour).or(next_segment.map(|first| first - 1)).unwrap_or(hour);
        return CardShape { valid_through, missing, current: None, progress: None, completed: Vec::new(), undated: 0, segment_first: None };
    };
    let mut dated: Vec<(String, Option<i32>)> = Vec::new();
    let mut undated: Vec<(String, Option<i32>)> = Vec::new();
    for entry in segment.order.completed.iter().rev() {
        match entry.completed_by_hour {
            Some(when) => dated.push((entry.token.clone(), Some(when))),
            None => undated.push((entry.token.clone(), None)),
        }
    }
    undated.reverse();
    let undated_count = undated.len() as u32;
    dated.extend(undated);
    let current = match (&segment.order.current, &segment.order.continuous) {
        (Some(token), _) => Some((token.clone(), false, segment.order.paused)),
        (None, Some(token)) => Some((token.clone(), true, segment.order.paused)),
        _ => None,
    };
    let at = fold.progress.partition_point(|run| run.last < hour);
    let run_last = fold.progress.get(at).filter(|run| run.first <= hour).map(|run| run.last);
    let progress = if current.as_ref().is_some_and(|(_, continuous, _)| !continuous) { fold.progress(hour) } else { None };
    let valid_through = match (progress.is_some(), run_last) {
        (true, Some(last)) => last.min(segment.last),
        _ => segment.last,
    };
    CardShape { valid_through, missing: None, current, progress, completed: dated, undated: undated_count, segment_first: Some(segment.first) }
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

#[derive(Default)]
struct Kept(Option<((u64, Country, i32), Arc<(Shown, Shown, Vec<String>)>)>);

fn shown(source: &Source, context: &LoadContext, params: &FocusHourParams, at: i32, database: FocusDatabase, tokens: Vec<String>, cancelled: &AtomicBool) -> (Shown, Vec<String>) {
    if tokens.is_empty() {
        return (Shown::new(), Vec::new());
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
    match FocusArtwork::load(source, &asked, context, cancelled) {
        Ok(reply) => (reply.items.into_iter().map(|item| (item.artwork.token.clone(), item)).collect(), reply.warnings),
        Err(error) => (Shown::new(), vec![format!("Focus names and icons refused: {error}")]),
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
        vec![FocusCardArt::decl(), FocusCardCompleted::decl(), FocusCardCurrent::decl(), FocusCardReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        validate_country(&params.country)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let fold = focus::fold(source, branch, &params.country, cancelled)?;
        let shape = card_at(&fold, params.hour);
        let held = match shape.segment_first {
            None => Arc::new((Shown::new(), Shown::new(), Vec::new())),
            Some(first) => {
                let key = (branch, params.country.clone(), first);
                let state = source.state::<Kept>();
                let kept = state.lock().unwrap_or_else(PoisonError::into_inner).0.as_ref().filter(|(at, _)| *at == key).map(|(_, held)| Arc::clone(held));
                match kept {
                    Some(held) => held,
                    None => {
                        let mut national: Vec<String> = shape.completed.iter().map(|(token, _)| token.clone()).collect();
                        let mut continuous = Vec::new();
                        if let Some((token, is_continuous, _)) = &shape.current {
                            if *is_continuous { continuous.push(token.clone()) } else { national.push(token.clone()) }
                        }
                        national.sort_unstable();
                        national.dedup();
                        let (national, mut warnings) = shown(source, context, params, first, FocusDatabase::National, national, cancelled);
                        let (continuous, more) = shown(source, context, params, first, FocusDatabase::Continuous, continuous, cancelled);
                        warnings.extend(more);
                        warnings.sort_unstable();
                        warnings.dedup();
                        let held = Arc::new((national, continuous, warnings));
                        state.lock().unwrap_or_else(PoisonError::into_inner).0 = Some((key, Arc::clone(&held)));
                        held
                    }
                }
            }
        };
        let (national, continuous, warnings) = &*held;
        const UNNAMED: &str = "The focus's name and icon were not recorded";
        let completed = shape
            .completed
            .iter()
            .map(|(token, when)| FocusCardCompleted { art: art_of(token, national, UNNAMED), completed_by_hour: *when, date_missing: when.is_none().then(|| UNDATED.to_owned()) })
            .collect();
        let current = shape.current.as_ref().map(|(token, is_continuous, paused)| {
            let shown = if *is_continuous { continuous } else { national };
            let total_days = shown.get(token).and_then(|item| item.artwork.base_duration_days);
            let (elapsed_days, permyriad, days_left, progress_missing) =
                if *is_continuous { (None, None, None, None) } else { progress_figures(shape.progress, total_days) };
            FocusCardCurrent { art: art_of(token, shown, UNNAMED), continuous: *is_continuous, paused: *paused, total_days, elapsed_days, permyriad, days_left, progress_missing }
        });
        let reply = FocusCardReply {
            hour: params.hour,
            valid_through: shape.valid_through.max(params.hour),
            missing: shape.missing,
            current,
            completed,
            undated: shape.undated,
            warnings: warnings.clone(),
        };
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}
