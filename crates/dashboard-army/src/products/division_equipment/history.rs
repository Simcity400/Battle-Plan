//! Every country's division equipment reply at every hour of a branch, prepared as a family on
//! the segment driver: an operator on the walk's world projects each hour's changed rows
//! ([`facts`]), an engine keeps each division's part of its country's replies and their sums
//! ([`engine`]), and the emit writes the values those sums change into the family's series
//! root ([`emit`]), which the history reply is read back from ([`query`]). Names, icons and
//! design names resolve at query time from the operands the root catalogues.
use super::{ArmyAttribution, DivisionEquipmentReply};
use crate::products::army_content::{self, Refusals, TemplateArtwork};
use dashboard_base::{
    products::{Product, equipment::RawVariant},
    source::Source,
};
use dashboard_keyed::query::KeyedWindow;
use dashboard_text::template_facts::TemplateNaming;
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    hash::{Hash, Hasher},
    io,
    sync::Arc,
};
use ts_rs::TS;

pub mod emit;
mod engine;
pub mod facts;
pub mod query;
pub mod served;
#[cfg(test)]
mod tests;

/// The family's name.
pub const NAME: &str = "division-equipment";
/// The prepared query reading it.
pub const QUERY: &str = "division-equipment-history";

/// How long a supplying read carries: the reached product's look-back.
const CADENCE: i32 = replay_data::cadence::STAGGER_PERIOD;

/// The designs' tables a lookup reads ([`army_content::Designs`]).
const DESIGNS: [TableId; 5] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::LexicalTokens,
    TableId::VariantDefinitions,
];

pub struct DivisionEquipmentHistory;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DivisionEquipmentHistoryParams {
    pub branch_id: String,
    pub tag: String,
    pub count_for: ArmyAttribution,
    #[ts(optional)]
    pub first_hour: Option<i32>,
    #[ts(optional)]
    pub last_hour: Option<i32>,
}

/// An equipment type's key and its name in the shown language.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HistoryKind {
    pub id: String,
    pub archetype: Option<String>,
    pub name: Option<String>,
    pub name_missing: Option<String>,
}

/// A refusal as the history keeps it. When a table an hour's replies read cannot be read
/// whole, its refusal starts `hour H <Table>`, as `DecodedFrame::complete` words it: that
/// part is kept as the table alone, so each reply names its own hour ([`KeptReason::at`]).
/// The text around it is kept as written: `text` holds one more part than `tables`, each
/// table going between two of them.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct KeptReason {
    pub text: Vec<String>,
    pub tables: Vec<String>,
}

impl KeptReason {
    fn literal(text: impl Into<String>) -> Self {
        Self {
            text: vec![text.into()],
            tables: Vec::new(),
        }
    }

    /// The refusal `before` and the failed read of `table` at the frame's hour give: the
    /// read's own `hour H <Table>` start is kept as the table, the rest of its words as
    /// written. Made where the read fails, never found again in other text.
    fn unreadable(before: &str, frame: &DecodedFrame, table: TableId, error: &io::Error) -> Self {
        let text = error.to_string();
        match text.strip_prefix(&format!("hour {} {table:?}", frame.hour)) {
            Some(rest) => Self {
                text: vec![before.to_owned(), rest.to_owned()],
                tables: vec![format!("{table:?}")],
            },
            None => Self::literal(format!("{before}{text}")),
        }
    }

    /// The refusal as a reply at `hour` words it.
    pub fn at(&self, hour: i32) -> String {
        let mut text = String::new();
        for (at, part) in self.text.iter().enumerate() {
            if at > 0
                && let Some(table) = self.tables.get(at - 1)
            {
                text.push_str(&format!("hour {hour} {table}"));
            }
            text.push_str(part);
        }
        text
    }
}

/// A template's naming operands, a failed read of its tables kept as that read's refusal.
fn kept_naming(frame: &DecodedFrame, handle: u64) -> Result<TemplateNaming, KeptReason> {
    // In the naming's order: its dictionaries, its dictionary row, then its icons. A missing
    // row is refused before the icons are read, in the naming's own words.
    let dictionaries = match frame.complete::<tables::DivisionTemplates>() {
        Ok(rows) => rows,
        Err(error) => {
            return Err(KeptReason::unreadable(
                "",
                frame,
                TableId::DivisionTemplates,
                &error,
            ));
        }
    };
    if dictionaries
        .binary_search_by_key(&handle, |row| row.template)
        .is_ok()
        && let Err(error) = frame.complete::<tables::DivisionTemplateIcons>()
    {
        return Err(KeptReason::unreadable(
            "",
            frame,
            TableId::DivisionTemplateIcons,
            &error,
        ));
    }
    army_content::template_naming(frame, handle).map_err(KeptReason::literal)
}

/// A template's artwork operands, a failed read of its icons kept as that read's refusal.
fn kept_artwork(frame: &DecodedFrame, handle: u64) -> Result<TemplateArtwork, KeptReason> {
    if let Err(error) = frame.complete::<tables::DivisionTemplateIcons>() {
        return Err(KeptReason::unreadable(
            army_content::ARTWORK_UNAVAILABLE,
            frame,
            TableId::DivisionTemplateIcons,
            &error,
        ));
    }
    army_content::template_artwork(frame, handle).map_err(KeptReason::literal)
}

/// What `answer` gives for a kept operand: a kept refusal carried into its words.
fn answered<T: Clone, U>(
    operand: &Result<T, KeptReason>,
    answer: impl FnOnce(&Result<T, String>) -> Result<U, String>,
) -> (Option<U>, Option<KeptReason>) {
    // Stands for the kept refusal in this call alone, which words nothing else of its own.
    const KEPT: &str = "\u{0}kept refusal\u{0}";
    match answer(&operand.clone().map_err(|_| KEPT.to_owned())) {
        Ok(value) => (Some(value), None),
        Err(text) => match (operand, text.split_once(KEPT)) {
            (Err(kept), Some((before, after))) => {
                let mut text = kept.text.clone();
                text[0] = format!("{before}{}", text[0]);
                let last = text.len() - 1;
                text[last].push_str(after);
                (
                    None,
                    Some(KeptReason {
                        text,
                        tables: kept.tables.clone(),
                    }),
                )
            }
            _ => (None, Some(KeptReason::literal(text))),
        },
    }
}

/// A template's name from one of its naming operands.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HistoryName {
    pub name: Option<String>,
    pub name_missing: Option<KeptReason>,
}

/// A template's icon from one of its artwork operands.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HistoryIcon {
    pub icon: Option<String>,
    pub icon_missing: Option<KeptReason>,
}

/// A refusal counted at hours, worded for each.
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRefusal {
    pub reason: KeptReason,
    pub times: usize,
}

/// Hours' counted refusals, and their line for artwork drawn in the recording's style. A
/// reply words, counts and orders the refusals for its own hour, as the reached product does.
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HistoryWarnings {
    pub first_hour: i32,
    pub last_hour: i32,
    pub refusals: Vec<HistoryRefusal>,
    pub fallback: Option<String>,
}

/// One country's history in a window; names lack their `TAG|mode|` part.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DivisionEquipmentHistoryReply {
    pub branch_id: String,
    pub tag: String,
    pub count_for: ArmyAttribution,
    pub window: KeyedWindow,
    pub kinds: Vec<HistoryKind>,
    pub namings: BTreeMap<String, HistoryName>,
    pub artworks: BTreeMap<String, HistoryIcon>,
    pub designs: BTreeMap<String, Option<String>>,
    pub hours: Vec<HistoryWarnings>,
    /// The counted refusals of an hour the country does not exist.
    pub absent: Vec<String>,
    /// The lines every served reply adds, as of serving ([`served_lines`]).
    pub served: Vec<String>,
    pub warnings: Vec<String>,
}
dashboard_base::reply_warnings!(DivisionEquipmentHistoryReply);

impl Product for DivisionEquipmentHistory {
    const KEY: &'static str = QUERY;
    const CONTENT_DEPENDENT: bool = true;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
    type Params = DivisionEquipmentHistoryParams;
    type Reply = DivisionEquipmentHistoryReply;
    fn types() -> Vec<String> {
        vec![
            DivisionEquipmentHistoryParams::decl(),
            DivisionEquipmentHistoryReply::decl(),
            HistoryKind::decl(),
            HistoryName::decl(),
            HistoryIcon::decl(),
            KeptReason::decl(),
            HistoryRefusal::decl(),
            HistoryWarnings::decl(),
        ]
    }
    fn validate(p: &Self::Params) -> Result<(), &'static str> {
        dashboard_base::products::validate_tag(&p.tag)?;
        dashboard_keyed::query::validate_window(&p.branch_id, p.first_hour, p.last_hour)
    }
}

/// The lines a served reply adds to its own, as the reached product's server adds them.
pub fn served_lines(
    source: &Source,
    content: Option<&dashboard_core::config::ContentOptions>,
) -> Vec<String> {
    let mut lines = Vec::new();
    source.append_reply_warnings(&mut lines, content);
    source.append_presentation_warnings(&mut lines, content);
    lines
}

/// A design's naming operands without the hour of the read that gave them.
#[allow(clippy::type_complexity)]
fn operands_of(
    raw: &RawVariant,
) -> (
    u64,
    &str,
    &str,
    &str,
    bool,
    Option<&str>,
    Option<&str>,
    Option<&str>,
    &[(String, String)],
) {
    let RawVariant {
        id,
        first_hour: _,
        token,
        display_name,
        suffix,
        append,
        name_tag,
        type_tag,
        design,
        refusals,
    } = raw;
    (
        *id,
        token.as_str(),
        display_name.as_str(),
        suffix.as_str(),
        *append,
        name_tag.as_deref(),
        type_tag.as_deref(),
        design.as_deref(),
        refusals.as_slice(),
    )
}

/// A design's naming operands, whichever read gave them: equal operands, one design.
#[derive(Clone, Debug)]
struct NamingOperands(Arc<RawVariant>);

impl PartialEq for NamingOperands {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || operands_of(&self.0) == operands_of(&other.0)
    }
}

impl Eq for NamingOperands {}

impl Hash for NamingOperands {
    fn hash<H: Hasher>(&self, state: &mut H) {
        operands_of(&self.0).hash(state);
    }
}

/// The value `runs` hold at `hour`.
fn at(runs: &[dashboard_keyed::query::KeyedRun], hour: i32) -> Option<&str> {
    let at = runs.partition_point(|run| run.1 < hour);
    runs.get(at)
        .filter(|run| run.0 <= hour)
        .map(|run| run.2.as_str())
}

/// The reply a history holds for `hour`, served; none where it does not answer then.
pub fn reply_at(
    history: &DivisionEquipmentHistoryReply,
    hour: i32,
) -> Option<DivisionEquipmentReply> {
    use super::{EquipmentTemplate, EquipmentType, EquipmentVariant};
    let window = &history.window;
    if hour < window.first_hour
        || hour > window.last_hour
        || !window
            .known
            .iter()
            .any(|span| span.first_hour <= hour && hour <= span.last_hour)
    {
        return None;
    }
    let values: BTreeMap<&str, &str> = window
        .series
        .iter()
        .filter_map(|series| Some((series.name.as_str(), at(&series.runs, hour)?)))
        .collect();
    if values.contains_key("refused") {
        return None;
    }
    let number = |name: &str| {
        values
            .get(name)
            .and_then(|value| value.parse::<i128>().ok())
    };
    let count = |name: &str| number(name).and_then(|value| u32::try_from(value).ok());
    let mut reply = DivisionEquipmentReply {
        branch_id: history.branch_id.clone(),
        hour,
        tag: history.tag.clone(),
        count_for: history.count_for,
        absent: false,
        pool_hour: number("*|pool").map(|hour| hour as i32),
        divisions: 0,
        divisions_short: 0,
        unknown_divisions: 0,
        types: Vec::new(),
        templates: Vec::new(),
        division_rows: None,
        pool: None,
        warnings: Vec::new(),
    };
    if !values.contains_key("state") {
        reply.absent = true;
        reply.warnings = [history.absent.as_slice(), &history.served].concat();
        return Some(reply);
    }
    reply.divisions = count("divisions")?;
    reply.divisions_short = count("short")?;
    reply.unknown_divisions = count("unknown")?;
    let text = |name: String| values.get(name.as_str()).map(|value| (*value).to_owned());
    let stock_hour = number("*|stock").map(|hour| hour as i32);
    for kind in &history.kinds {
        let base = format!("type|{}|", kind.id);
        let Some(held) = text(format!("{base}held")) else {
            continue;
        };
        let prefix = format!("{base}variant|");
        let mut variants: Vec<(u64, EquipmentVariant)> = values
            .iter()
            .filter_map(|(name, value)| Some((name.strip_prefix(prefix.as_str())?, value)))
            .map(|(handle, value)| {
                let name = text(format!("{base}design|{handle}"))
                    .and_then(|id| history.designs.get(&id).cloned().flatten());
                Some((
                    handle.parse().ok()?,
                    EquipmentVariant {
                        variant: handle.to_owned(),
                        name,
                        held_raw: (*value).to_owned(),
                    },
                ))
            })
            .collect::<Option<_>>()?;
        variants.sort_by_key(|(handle, _)| *handle);
        let stockpile_raw = text(format!("{base}stock"));
        let stockpile_hour = stockpile_raw.as_ref().and(stock_hour);
        reply.types.push(EquipmentType {
            archetype: kind.archetype.clone(),
            name: kind.name.clone(),
            name_missing: kind.name_missing.clone(),
            held_raw: held,
            needed_raw: text(format!("{base}needed")),
            filled_raw: text(format!("{base}filled")),
            missing_raw: text(format!("{base}missing")),
            divisions_short: count(&format!("{base}short"))?,
            unknown_divisions: count(&format!("{base}unknown"))?,
            stockpile_raw,
            stockpile_hour,
            variants: variants.into_iter().map(|(_, variant)| variant).collect(),
        });
    }
    let mut templates: Vec<u64> = values
        .keys()
        .filter_map(|name| name.strip_prefix("template|")?.strip_suffix("|divisions"))
        .map(|handle| handle.parse().ok())
        .collect::<Option<_>>()?;
    templates.sort_unstable();
    for handle in templates {
        let base = format!("template|{handle}|");
        let naming = text(format!("{base}naming")).and_then(|id| history.namings.get(&id));
        let artwork = text(format!("{base}artwork")).and_then(|id| history.artworks.get(&id));
        reply.templates.push(EquipmentTemplate {
            template: handle.to_string(),
            name: naming.and_then(|name| name.name.clone()),
            name_missing: naming
                .and_then(|name| name.name_missing.as_ref().map(|kept| kept.at(hour))),
            icon: artwork.and_then(|icon| icon.icon.clone()),
            icon_missing: artwork
                .and_then(|icon| icon.icon_missing.as_ref().map(|kept| kept.at(hour))),
            divisions: count(&format!("{base}divisions"))?,
            manpower: text(format!("{base}manpower")),
            manpower_required: text(format!("{base}required")),
            hp_raw: text(format!("{base}hp")),
            hp_max_raw: text(format!("{base}hpMax")),
            needed_raw: text(format!("{base}needed")),
            held_raw: text(format!("{base}held")),
            types_short: count(&format!("{base}typesShort"))?,
            unknown_divisions: count(&format!("{base}unknown"))?,
        });
    }
    let span = history
        .hours
        .iter()
        .find(|span| span.first_hour <= hour && hour <= span.last_hour)?;
    // Worded for the hour, then counted and ordered by those words, as the reached product's.
    let mut refusals = Refusals::default();
    for refusal in &span.refusals {
        refusals.note_times(refusal.reason.at(hour), refusal.times);
    }
    reply.warnings = refusals.lines();
    reply.warnings.extend(history.served.iter().cloned());
    reply.warnings.extend(span.fallback.clone());
    // The reached product refuses a reply over its transport budget.
    dashboard_base::products::check_reply_budget(&reply).ok()?;
    Some(reply)
}
