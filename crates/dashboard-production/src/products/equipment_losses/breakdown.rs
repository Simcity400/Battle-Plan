use super::{
    AMOUNT_SCALE, Cause, EquipmentLossesParams, EquipmentLossesReply, LandSource, LossCountry, LossDomain, LossName, LossSource, RUN_KEY, content::{Content, DatedDesign},
    coverage::{Coverage, Held as Covered, UnknownReason, UnknownSpan},
    coverage_key, identity_range, leaf_of, ship_log, ships,
};
use dashboard_base::{
    Source,
    products::{
        Country, LoadContext,
        breakdown::{self, Amount, Breakdown, Exact, Leaf, Node, Unrecorded},
        check_reply_budget,
    },
};
use dashboard_query::{
    Query,
    coverage::row_coverage,
    identity::{Holder, HolderRun, Holders, TAG_UNLINKED},
};
use replay_columnar::manifest::Status;
use replay_data::rows::{EQUIPMENT_DESTRUCTION_PARTIAL_REASON, EquipmentDestructionCause, EquipmentLossCoverageRow, EquipmentLossTotalRow, TableId};
use replay_lanes::lanes::{self, Cell, History, Keys, Reading, StatusRun};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::atomic::AtomicBool,
};

const NOT_OBSERVED: &str = "Equipment losses were not recorded at this hour";
const NOT_PRESENT: &str = "This country is not present at this hour";
const HOLDER_UNRESOLVED: &str = "This country's holder was not recorded at this hour";
const DESTRUCTION_NOT_COUNTING: &str = "Equipment destruction was not yet counting at this hour";
const NO_DESTRUCTION_READ: &str = "Equipment destruction was not recorded at this hour";
const DOMAIN_UNKNOWN: &str = "The recorded content does not resolve this destruction equipment's domain; its contribution in Combat Log mode is not known";
const UNRESOLVED_TYPE_KEY: &str = "~unresolved";

fn source_key(source: LossSource) -> String {
    match source {
        LossSource::CombatLog(reason) => format!("combatLog:{reason}"),
        LossSource::Recovered => "recovered".into(),
        LossSource::SunkShips => "sunkShips".into(),
        LossSource::SunkConvoys => "sunkConvoys".into(),
        LossSource::LendLeaseSunk => "lendLeaseSunk".into(),
        LossSource::LendLeaseSent => "lendLeaseSent".into(),
        LossSource::Destruction(Cause(cause)) => format!("destruction:{}", cause.as_str()),
    }
}

fn cause_label(name: &LossName) -> (String, Option<String>) {
    if let Some(text) = name.text.as_deref().filter(|text| !text.is_empty()) {
        return (text.to_owned(), None);
    }
    if let Some(template) = name.template.as_deref().filter(|text| !text.is_empty()) {
        return (template.to_owned(), None);
    }
    if let Some(key) = name.key.as_deref().filter(|text| !text.is_empty()) {
        return (key.to_owned(), Some(format!("Localisation holds no name for {key}")));
    }
    ("Cause name unavailable".to_owned(), Some("Cause name unavailable".to_owned()))
}

fn unlinked_spans(holders: &Holders) -> Vec<(i32, i32)> {
    let mut spans: Vec<(i32, i32)> = holders.tags.values().flatten().filter(|run| run.holder == Holder::Unresolved).map(|run| (run.first, run.last)).collect();
    spans.sort_unstable();
    let mut out: Vec<(i32, i32)> = Vec::with_capacity(spans.len());
    for (first, last) in spans {
        match out.last_mut() {
            Some(held) if i64::from(first) <= i64::from(held.1) + 1 => held.1 = held.1.max(last),
            _ => out.push((first, last)),
        }
    }
    out
}

fn covering<'a, T>(runs: &'a [T], hour: i32, bounds: impl Fn(&T) -> (i32, i32)) -> Option<&'a T> {
    let at = runs.partition_point(|run| bounds(run).1 < hour);
    runs.get(at).filter(|run| bounds(run).0 <= hour)
}

fn next_span_edge<T>(runs: &[T], hour: i32, bounds: impl Fn(&T) -> (i32, i32)) -> Option<i32> {
    let at = runs.partition_point(|run| bounds(run).1 < hour);
    let run = runs.get(at)?;
    let (first, last) = bounds(run);
    if first <= hour { last.checked_add(1).filter(|&edge| edge > hour) } else { Some(first) }
}

fn tighten(through: &mut i32, edge: Option<i32>) {
    if let Some(edge) = edge
        && let Some(until) = edge.checked_sub(1)
    {
        *through = (*through).min(until);
    }
}

fn status_at<'a>(statuses: &'a [StatusRun], reasons: &'a HashMap<u32, String>, hour: i32) -> Option<(Status, Option<&'a str>)> {
    let run = covering(statuses, hour, |run| (run.first, run.last))?;
    let reason = match run.status {
        Status::Unavailable { reason } => reasons.get(&reason).map(String::as_str),
        _ => None,
    };
    Some((run.status, reason))
}

fn not_recorded(reason: impl Into<String>) -> Breakdown {
    breakdown::breakdown(&[Leaf { path: Vec::new(), amount: Amount::NotRecorded(reason.into()), count: None, last: None }], &Unrecorded::default())
}

fn exact_loss(raw: i128) -> Amount {
    match Exact::new(raw, i128::from(AMOUNT_SCALE)) {
        Some(value) => Amount::Known(value),
        None => Amount::NotRecorded("Equipment losses could not be represented exactly".into()),
    }
}

fn plain_destruction_reason(text: &str) -> String {
    fn translate(part: &str) -> &str {
        match part {
            EquipmentLossCoverageRow::DRAIN_WITHHELD => "Equipment destruction was withheld for this run",
            EquipmentLossCoverageRow::DRAIN_TRUNCATED => "Equipment destruction was truncated and withheld for this run",
            EquipmentLossCoverageRow::DRAIN_SUPPLEMENTAL_NOT_RECORDED => "Equipment destruction coverage was not recorded",
            EquipmentLossCoverageRow::DRAIN_OWNER_UNKNOWN => "Some destroyed equipment has no recorded country link",
            EquipmentLossCoverageRow::DRAIN_NOT_RECORDED => "Equipment destruction was not recorded in this recording",
            EquipmentLossCoverageRow::DRAIN_NOT_READ => "Equipment destruction was not read at this hour",
            EquipmentLossCoverageRow::DRAIN_LOST => "Equipment destruction was not recorded at some hours",
            EQUIPMENT_DESTRUCTION_PARTIAL_REASON => "Equipment destruction did not cover the full period from the start",
            other => other,
        }
    }
    text.split("; ").map(translate).collect::<Vec<_>>().join("; ")
}

fn unknown_detail(span: &UnknownSpan) -> String {
    match span.detail.as_deref() {
        Some(detail) => plain_destruction_reason(detail),
        None => match span.reason {
            UnknownReason::Unread => "Equipment loss cause was not read at this hour",
            UnknownReason::EndedAfterLastRead => "A lend-lease exchange ended after its last read",
            UnknownReason::DestructionWithheld => "Equipment destruction table was withheld",
            UnknownReason::DestructionGap => "Equipment destruction was not recorded",
            UnknownReason::DestructionUnresolved => "Equipment destruction owner was not resolved",
        }
        .to_owned(),
    }
}

struct ShipRow {
    signature: String,
    listed_hour: i32,
    sink_hour: i32,
    variant: Option<u32>,
    count: u32,
}

#[derive(Default)]
struct ShipIndex {
    hours: Vec<i32>,
    variants: BTreeMap<Option<u32>, Vec<(i32, i128)>>,
}

impl ShipIndex {
    fn of(rows: Vec<ShipRow>) -> Self {
        let mut order: Vec<(i32, usize)> = rows.iter().enumerate().map(|(at, ship)| (ship.listed_hour.max(ship.sink_hour), at)).collect();
        order.sort_unstable();
        let mut most: HashMap<&str, (usize, Option<u32>, u32)> = HashMap::new();
        let mut index = Self::default();
        let add = |variants: &mut BTreeMap<Option<u32>, Vec<(i32, i128)>>, variant: Option<u32>, hour: i32, change: i128| {
            let steps = variants.entry(variant).or_default();
            let before = steps.last().map_or(0, |step| step.1);
            match steps.last_mut() {
                Some(step) if step.0 == hour => step.1 += change,
                _ => steps.push((hour, before + change)),
            }
        };
        for (visible, at) in order {
            let ship = &rows[at];
            let held = most.get(ship.signature.as_str()).copied();
            if held.is_some_and(|(index, _, count)| ship.count < count || (ship.count == count && index < at)) {
                continue;
            }
            if let Some((_, variant, count)) = held {
                add(&mut index.variants, variant, visible, -i128::from(count));
            }
            add(&mut index.variants, ship.variant, visible, i128::from(ship.count));
            most.insert(ship.signature.as_str(), (at, ship.variant, ship.count));
            if index.hours.last() != Some(&visible) {
                index.hours.push(visible);
            }
        }
        index
    }

    fn next_after(&self, hour: i32) -> Option<i32> {
        self.hours.get(self.hours.partition_point(|&at| at <= hour)).copied()
    }

    fn counts_at(&self, hour: i32) -> impl Iterator<Item = (Option<u32>, i128)> + '_ {
        self.variants.iter().filter_map(move |(&variant, steps)| {
            let count = steps[..steps.partition_point(|step| step.0 <= hour)].last().map_or(0, |step| step.1);
            (count != 0).then_some((variant, count))
        })
    }
}

struct SegmentData {
    first: i32,
    last: i32,
    totals: History,
    total_pos: usize,
    incomplete_pos: usize,
    covered: Covered,
    ships: ShipIndex,
}

struct CauseMeta {
    source: LossSource,
    ordinal: u32,
    key: String,
    label: String,
    name_missing: Option<String>,
    unknown: Option<String>,
}

struct LossesIndex {
    from: i32,
    to: i32,
    land_source: LandSource,
    unknown_holders: Vec<(i32, i32, String)>,
    unlinked_holders: Vec<(i32, i32)>,
    holders: Vec<HolderRun>,
    segments: Vec<SegmentData>,
    causes: Vec<CauseMeta>,
    designs: HashMap<u32, DatedDesign>,
    type_ordinals: HashMap<String, u32>,
    design_ordinals: HashMap<u32, u32>,
    content_warnings: Vec<String>,
}

pub(super) fn loss_amount(total: Option<i64>, issue: Option<&str>) -> Option<Amount> {
    Some(match total {
        Some(0) => Amount::NotRecorded(issue?.to_owned()),
        Some(value) => match exact_loss(i128::from(value)) {
            Amount::Known(exact) => match issue {
                Some(reason) => Amount::Partial(exact, reason.to_owned()),
                None => Amount::Known(exact),
            },
            other => other,
        },
        None => Amount::NotRecorded(issue.unwrap_or(NOT_OBSERVED).to_owned()),
    })
}

impl LossesIndex {
    fn next_change(&self, hour: i32, segment: Option<&SegmentData>) -> i32 {
        let mut through = self.to.max(hour);
        if !covering(&self.holders, hour, |run| (run.first, run.last)).is_some_and(|run| matches!(run.holder, Holder::Found { .. })) {
            tighten(&mut through, next_span_edge(&self.unknown_holders, hour, |&(a, b, _)| (a, b)));
            tighten(&mut through, next_span_edge(&self.unlinked_holders, hour, |&(a, b)| (a, b)));
        }
        tighten(&mut through, next_span_edge(&self.holders, hour, |run| (run.first, run.last)));
        tighten(&mut through, next_span_edge(&self.segments, hour, |seg| (seg.first, seg.last)));
        let Some(seg) = segment else {
            return through.max(hour);
        };
        through = through.min(seg.last);
        tighten(&mut through, next_span_edge(&seg.totals.statuses, hour, |run| (run.first, run.last)));
        let combat_log = self.land_source == LandSource::CombatLog;
        let shown: BTreeSet<LossSource> = LossSource::shown(self.land_source).into_iter().filter(|source| !matches!(source, LossSource::Recovered | LossSource::LendLeaseSent)).collect();
        for &(key, ref steps) in &seg.totals.keys {
            let Some((source, _)) = leaf_of(key) else { continue };
            if !shown.contains(&source) {
                continue;
            }
            let at = steps.partition_point(|step| step.hour <= hour);
            if let Some(next) = steps.get(at) {
                tighten(&mut through, Some(next.hour));
            }
        }
        tighten(&mut through, seg.ships.next_after(hour));
        for &(a, b, _) in &seg.covered.run {
            if a > hour {
                tighten(&mut through, Some(a));
            } else if b >= hour {
                tighten(&mut through, b.checked_add(1));
            }
        }
        if combat_log {
            for &(a, b, _) in &seg.covered.country {
                if a > hour {
                    tighten(&mut through, Some(a));
                } else if b >= hour {
                    tighten(&mut through, b.checked_add(1));
                }
            }
        } else {
            for &(a, b, ref cells) in &seg.covered.country {
                if cells.as_ref().is_some_and(|cells| cells.exchanges_ended.unwrap_or(0) > 0 || cells.convoy_drops.unwrap_or(0) > 0) {
                    if a > hour {
                        tighten(&mut through, Some(a));
                    } else if b >= hour {
                        tighten(&mut through, b.checked_add(1));
                    }
                }
            }
        }
        for design in self.designs.values() {
            if let Some(first) = design.first_hour
                && first > hour
            {
                tighten(&mut through, Some(first));
            }
        }
        through.max(hour)
    }

    fn type_node(&self, design: Option<&DatedDesign>) -> Node {
        match design {
            Some(design) => match (design.archetype.as_deref(), design.archetype_name.as_deref()) {
                (Some(key), Some(name)) => Node::new(format!("type:{key}"), name, self.type_ordinals.get(key).copied().unwrap_or(u32::MAX)),
                (Some(key), None) => Node::new(format!("type:{key}"), key, self.type_ordinals.get(key).copied().unwrap_or(u32::MAX))
                    .name_missing(Some(format!("Localisation holds no name for {key}"))),
                (None, Some(name)) => Node::new(UNRESOLVED_TYPE_KEY, name, u32::MAX).other(),
                (None, None) => Node::new(UNRESOLVED_TYPE_KEY, "Equipment type name unavailable", u32::MAX).other(),
            },
            None => Node::new(UNRESOLVED_TYPE_KEY, "Equipment type name unavailable", u32::MAX).other(),
        }
    }

    fn design_node(&self, variant: u32, hour: i32) -> Node {
        let ordinal = self.design_ordinals.get(&variant).copied().unwrap_or(u32::MAX);
        let key = format!("design:{variant}");
        match self.designs.get(&variant) {
            Some(design) if design.first_hour.is_some_and(|first| first <= hour) => match design.name.as_deref().filter(|text| !text.is_empty()) {
                Some(name) => Node::new(key, name, ordinal),
                None => Node::new(key, "Name unavailable", ordinal).other(),
            },
            _ => Node::new(key, "Name unavailable", ordinal).other(),
        }
    }

    fn at(&self, hour: i32) -> EquipmentLossesReply {
        let segment = covering(&self.segments, hour, |seg| (seg.first, seg.last));
        let through = self.next_change(hour, segment);
        let reply = |breakdown: Breakdown, warnings: Vec<String>| EquipmentLossesReply {
            from: segment.map_or(self.from, |seg| seg.first),
            hour,
            through,
            land_source: self.land_source,
            breakdown,
            warnings,
        };
        match covering(&self.holders, hour, |run| (run.first, run.last)).map(|run| run.holder) {
            Some(Holder::Found { .. }) => {}
            held => {
                if let Some((_, _, reason)) = covering(&self.unknown_holders, hour, |&(a, b, _)| (a, b)) {
                    return reply(not_recorded(reason.clone()), self.content_warnings.clone());
                }
                if covering(&self.unlinked_holders, hour, |&(a, b)| (a, b)).is_some() {
                    return reply(not_recorded(TAG_UNLINKED), self.content_warnings.clone());
                }
                let why = if held.is_some() { HOLDER_UNRESOLVED } else { NOT_PRESENT };
                return reply(not_recorded(why), self.content_warnings.clone());
            }
        }
        let Some(seg) = segment else {
            return reply(not_recorded(NOT_OBSERVED), self.content_warnings.clone());
        };
        let mut warnings = self.content_warnings.clone();
        warnings.extend(seg.covered.warnings_through(hour, self.land_source == LandSource::CombatLog));
        let truncated_reason = match status_at(&seg.totals.statuses, &seg.totals.reasons, hour) {
            Some((Status::Complete, _)) => None,
            Some((status @ Status::Truncated { .. }, reason)) => row_coverage(TableId::EquipmentLossTotals, status, reason, false).into_missing(),
            Some((status, reason)) => {
                let why = row_coverage(TableId::EquipmentLossTotals, status, reason, false).into_missing().unwrap_or_else(|| NOT_OBSERVED.into());
                return reply(not_recorded(why), warnings);
            }
            None => return reply(not_recorded(NOT_OBSERVED), warnings),
        };
        let combat_log = self.land_source == LandSource::CombatLog;
        let destruction_runs = seg.covered.destruction_runs();
        let destruction_run = covering(&destruction_runs, hour, |run| (run.first_hour, run.last_hour));
        let destruction_floor = destruction_run.and_then(|run| run.counted_from.filter(|&counted| hour >= counted).map(|counted| counted.max(run.first_hour)));
        let destruction_withheld = destruction_run.and_then(|run| run.withheld_from).is_some_and(|withheld| hour >= withheld);
        let destruction_spans = seg.covered.destruction_unknown(hour);
        let destruction_reason: Option<String> = if destruction_withheld {
            destruction_spans
                .iter()
                .find(|span| span.reason == UnknownReason::DestructionWithheld && span.first_hour <= hour && hour <= span.last_hour)
                .map(unknown_detail)
                .or_else(|| Some("Equipment destruction table was withheld".into()))
        } else if destruction_floor.is_none() {
            seg.covered
                .destruction_coverage()
                .iter()
                .find(|span| span.first_hour <= hour && hour <= span.last_hour)
                .and_then(|span| span.reason.as_deref().map(plain_destruction_reason))
                .or_else(|| Some(if destruction_run.is_none() { NO_DESTRUCTION_READ } else { DESTRUCTION_NOT_COUNTING }.into()))
        } else {
            destruction_spans
                .iter()
                .find(|span| span.first_hour <= hour && hour <= span.last_hour)
                .map(unknown_detail)
        };
        let land_spans = if combat_log { seg.covered.land() } else { Vec::new() };
        let land_reason: Option<String> = if combat_log {
            land_spans.iter().find(|span| span.first_hour <= hour && span.coverage != Coverage::Logged).map(|span| match span.coverage {
                Coverage::NotLogged => "The game did not log this country's land losses at some hours".into(),
                Coverage::Partial { ungrouped } => format!("{ungrouped} divisions had no orders group, so their land losses were not logged"),
                Coverage::PruneUnobserved => "Combat Log retention could not be observed at some hours".into(),
                Coverage::Unknown => "Land-loss coverage was not recorded at some hours".into(),
                Coverage::Logged => String::new(),
            })
        } else {
            None
        };
        let unread_reason = |bit: u8| -> Option<String> {
            seg.covered
                .unread(bit, hour)
                .into_iter()
                .find(|span| span.first_hour <= hour && span.last_hour >= hour)
                .map(|span| unknown_detail(&span))
        };
        let combat_log_unread = combat_log.then(|| unread_reason(EquipmentLossCoverageRow::UNREAD_COMBAT_LOG)).flatten();
        let ships_unread = unread_reason(EquipmentLossCoverageRow::UNREAD_SHIPS);
        let convoys_unread = unread_reason(EquipmentLossCoverageRow::UNREAD_CONVOYS);
        let lend_lease_unread = unread_reason(EquipmentLossCoverageRow::UNREAD_LEND_LEASE).or_else(|| {
            seg.covered
                .ended(hour)
                .into_iter()
                .find(|span| span.first_hour <= hour && hour <= span.last_hour)
                .map(|span| unknown_detail(&span))
        });

        let mut by_source: BTreeMap<LossSource, Vec<(Option<u32>, Option<i64>, Option<i32>, Option<String>)>> = BTreeMap::new();
        for &(key, _) in &seg.totals.keys {
            let Some((source, variant)) = leaf_of(key) else { continue };
            match seg.totals.reading(key, hour) {
                Reading::Cells(cells) => {
                    let total = match cells.get(seg.total_pos).copied() {
                        Some(Cell::Number(value)) => Some(value),
                        _ => None,
                    };
                    let incomplete_from = match cells.get(seg.incomplete_pos).copied() {
                        Some(Cell::Number(value)) => Some(value as i32),
                        _ => None,
                    };
                    by_source.entry(source).or_default().push((variant, total, incomplete_from, None));
                }
                Reading::Absent => {}
                Reading::Unread { status, reason } => {
                    let why = row_coverage(TableId::EquipmentLossTotals, status, reason, false).into_missing().unwrap_or_else(|| NOT_OBSERVED.into());
                    by_source.entry(source).or_default().push((variant, None, None, Some(why)));
                }
            }
        }

        let ship_counts: Vec<(Option<u32>, i128)> = seg.ships.counts_at(hour).collect();

        let mut leaves: Vec<Leaf> = Vec::new();
        let mut destruction_qualified = false;
        let mut combat_log_qualified = false;
        for meta in &self.causes {
            let cause = Node::new(meta.key.clone(), meta.label.clone(), meta.ordinal).name_missing(meta.name_missing.clone());
            if let Some(unknown) = &meta.unknown {
                leaves.push(Leaf { path: vec![cause], amount: Amount::NotRecorded(unknown.clone()), count: None, last: None });
                if matches!(meta.source, LossSource::CombatLog(_)) {
                    combat_log_qualified = true;
                }
                continue;
            }
            if meta.source == LossSource::SunkShips {
                let mut had_ship = false;
                for &(variant, count) in &ship_counts {
                    if count == 0 {
                        continue;
                    }
                    had_ship = true;
                    let amount = match &ships_unread {
                        Some(reason) => Amount::Partial(Exact::whole(count), reason.clone()),
                        None => Amount::Known(Exact::whole(count)),
                    };
                    let path = match variant {
                        Some(variant) => vec![cause.clone(), self.type_node(self.designs.get(&variant)), self.design_node(variant, hour)],
                        None => vec![cause.clone()],
                    };
                    leaves.push(Leaf { path, amount, count: None, last: None });
                }
                if !had_ship
                    && let Some(reason) = &ships_unread
                {
                    leaves.push(Leaf { path: vec![cause], amount: Amount::NotRecorded(reason.clone()), count: None, last: None });
                }
                continue;
            }

            let is_destruction = matches!(meta.source, LossSource::Destruction(_));
            if is_destruction && (destruction_floor.is_none() || destruction_withheld) {
                continue;
            }
            let cause_issue: Option<&str> = match meta.source {
                LossSource::Destruction(_) => destruction_reason.as_deref(),
                LossSource::CombatLog(_) => combat_log_unread.as_deref().or(land_reason.as_deref()),
                LossSource::SunkConvoys => convoys_unread.as_deref(),
                LossSource::LendLeaseSunk => lend_lease_unread.as_deref(),
                _ => None,
            };
            let rows = by_source.get(&meta.source).map(Vec::as_slice).unwrap_or_default();
            let mut domain_unresolved = false;
            let mut emitted_for_cause = false;
            for &(variant, total, incomplete_from, ref unread_why) in rows {
                if combat_log && meta.source.mixed() {
                    let domain = variant.and_then(|variant| self.designs.get(&variant)).and_then(|design| design.domain);
                    let stockpile = meta.source == LossSource::Destruction(Cause(EquipmentDestructionCause::ScriptedStockpileFraction));
                    let proven = domain == Some(LossDomain::Air) || domain == Some(if stockpile { LossDomain::Naval } else { LossDomain::Land });
                    if !proven {
                        if !(stockpile && domain == Some(LossDomain::Land)) {
                            domain_unresolved = true;
                        }
                        continue;
                    }
                }
                let path = match variant {
                    Some(variant) => vec![cause.clone(), self.type_node(self.designs.get(&variant)), self.design_node(variant, hour)],
                    None => vec![cause.clone()],
                };
                let leaf_issue = unread_why
                    .as_deref()
                    .or(cause_issue)
                    .or_else(|| incomplete_from.filter(|&from| from <= hour).map(|_| "Some losses for this item were not recorded"));
                let Some(amount) = loss_amount(total, leaf_issue) else { continue };
                emitted_for_cause = true;
                if is_destruction && leaf_issue.is_some() {
                    destruction_qualified = true;
                }
                if matches!(meta.source, LossSource::CombatLog(_)) && leaf_issue.is_some() {
                    combat_log_qualified = true;
                }
                leaves.push(Leaf { path, amount, count: None, last: None });
            }
            if domain_unresolved {
                emitted_for_cause = true;
                destruction_qualified = true;
                leaves.push(Leaf { path: vec![cause.clone()], amount: Amount::NotRecorded(DOMAIN_UNKNOWN.into()), count: None, last: None });
            }
            if !emitted_for_cause
                && !is_destruction
                && !matches!(meta.source, LossSource::CombatLog(_))
                && let Some(reason) = cause_issue
            {
                leaves.push(Leaf { path: vec![cause], amount: Amount::NotRecorded(reason.to_owned()), count: None, last: None });
            }
        }
        if let Some(reason) = &destruction_reason
            && !destruction_qualified
        {
            leaves.push(Leaf { path: vec![Node::new("~destruction-unread", "Unread destruction losses", u32::MAX).other()], amount: Amount::NotRecorded(reason.clone()), count: None, last: None });
        }
        if combat_log
            && let Some(reason) = combat_log_unread.as_deref().or(land_reason.as_deref())
            && !combat_log_qualified
        {
            leaves.push(Leaf { path: vec![Node::new("~combat-log-unread", "Unread land losses", u32::MAX).other()], amount: Amount::NotRecorded(reason.to_owned()), count: None, last: None });
        }
        if let Some(reason) = truncated_reason {
            leaves.push(Leaf { path: vec![Node::new("~unread", "Unread equipment losses", u32::MAX).other()], amount: Amount::NotRecorded(reason), count: None, last: None });
        }
        reply(breakdown::breakdown(&leaves, &Unrecorded::default()), warnings)
    }
}

fn build_index(source: &Source, query: &Query<'_>, params: &EquipmentLossesParams, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<LossesIndex> {
    let recording = query.replay.recording();
    let content = Content::load(source, context, params.land_source)?;
    let total_col = dashboard_declare::field(TableId::EquipmentLossTotals, "total")?;
    let incomplete_col = dashboard_declare::field(TableId::EquipmentLossTotals, "incomplete_from")?;
    let log = ship_log()?;
    let reader = dashboard_query::log::Reader::new(query, log)?;

    let first = query.timeline.first;
    let last = query.timeline.last;
    let holders = query.holders.within(&params.country, first, last);
    let unknown_holders = query.holders.unknown.clone();
    let unlinked_holders = if matches!(params.country, Country::Slot(_)) { unlinked_spans(&query.holders) } else { Vec::new() };

    let mut segments: Vec<SegmentData> = Vec::new();
    let mut variants: BTreeSet<u32> = BTreeSet::new();
    for stretch in &query.timeline.stretches {
        dashboard_text::metadata::check_cancel(cancelled)?;
        for run in &holders {
            let Holder::Found { slot, id } = run.holder else { continue };
            let seg_first = stretch.first.max(run.first);
            let seg_last = stretch.last.min(run.last);
            if seg_first > seg_last {
                continue;
            }
            let (low, high) = identity_range(slot, id);
            let totals = dashboard_query::cost::storage(|| {
                lanes::history(recording, stretch.run, TableId::EquipmentLossTotals, &[total_col, incomplete_col], Keys::Range(low, high), seg_first, seg_last)
            })?;
            let total_pos = totals.columns.binary_search(&total_col).map_err(|_| dashboard_core::invalid("EquipmentLossTotals missing total column"))?;
            let incomplete_pos = totals.columns.binary_search(&incomplete_col).map_err(|_| dashboard_core::invalid("EquipmentLossTotals missing incomplete_from column"))?;
            for &(key, _) in &totals.keys {
                if let Some((_, Some(variant))) = leaf_of(key) {
                    variants.insert(variant);
                }
            }
            let mut covered = Covered::default();
            covered.read(recording, stretch.run, coverage_key(slot, id), RUN_KEY, seg_first, seg_last)?;
            let listed = ships(&reader, stretch.run, LossCountry { slot, country_id: id }, seg_first, seg_last)?;
            let mut ship_rows = Vec::with_capacity(listed.len());
            for (signature, listed_hour, sink_hour, handle, count) in listed {
                let variant = EquipmentLossTotalRow::variant_id(handle);
                if let Some(variant) = variant {
                    variants.insert(variant);
                }
                ship_rows.push(ShipRow { signature, listed_hour, sink_hour, variant, count });
            }
            segments.push(SegmentData { first: seg_first, last: seg_last, totals, total_pos, incomplete_pos, covered, ships: ShipIndex::of(ship_rows) });
        }
    }
    segments.sort_by_key(|seg| seg.first);

    let mut warnings = content.warnings.clone();
    let designs = content.designs_dated(query, &variants, &mut warnings)?;
    let mut type_keys: BTreeSet<String> = BTreeSet::new();
    for design in designs.values() {
        if let Some(key) = &design.archetype {
            type_keys.insert(key.clone());
        }
    }
    let type_ordinals: HashMap<String, u32> = type_keys.into_iter().enumerate().map(|(i, key)| (key, i as u32)).collect();
    let design_ordinals: HashMap<u32, u32> = variants.into_iter().enumerate().map(|(i, variant)| (variant, i as u32)).collect();

    let combat_log = params.land_source == LandSource::CombatLog;
    let land_unknown = combat_log.then(|| content.retention_refusal()).flatten();
    let causes: Vec<CauseMeta> = LossSource::shown(params.land_source)
        .into_iter()
        .filter(|source| !matches!(source, LossSource::Recovered | LossSource::LendLeaseSent))
        .enumerate()
        .map(|(ordinal, source)| {
            let name = content.cause_name(source);
            let (label, name_missing) = cause_label(&name);
            let unknown = land_unknown.clone().filter(|_| matches!(source, LossSource::CombatLog(_)));
            CauseMeta { source, ordinal: ordinal as u32, key: source_key(source), label, name_missing, unknown }
        })
        .collect();

    Ok(LossesIndex {
        from: first,
        to: last,
        land_source: params.land_source,
        unknown_holders,
        unlinked_holders,
        holders,
        segments,
        causes,
        designs,
        type_ordinals,
        design_ordinals,
        content_warnings: warnings,
    })
}

pub(super) fn load(source: &Source, params: &EquipmentLossesParams, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<EquipmentLossesReply> {
    let branch_id = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid equipment losses branch"))?;
    let query = dashboard_base::queries::query(source, branch_id)?;
    let key = (params.branch_id.clone(), params.country.clone(), params.land_source, query.timeline.first, query.timeline.last);
    let held = breakdown::held(source, context, key, || build_index(source, &query, params, context, cancelled))?;
    let reply = held.at(params.hour);
    check_reply_budget(&reply)?;
    Ok(reply)
}

#[cfg(test)]
mod ship_tests {
    use super::{ShipIndex, ShipRow};

    #[test]
    fn ship_counts_step_at_each_hour_a_ship_is_both_listed_and_sunk_and_a_ship_counts_once_at_its_largest_listing() {
        let ship = |signature: &str, listed_hour: i32, sink_hour: i32, variant: u32, count: u32| ShipRow { signature: signature.into(), listed_hour, sink_hour, variant: Some(variant), count };
        let index = ShipIndex::of(vec![ship("a", 10, 20, 1, 1), ship("b", 5, 8, 2, 1), ship("a", 30, 20, 1, 2), ship("c", 30, 25, 3, 1)]);
        assert_eq!(index.hours, [8, 20, 30]);
        assert_eq!((index.next_after(8), index.next_after(30)), (Some(20), None));
        assert_eq!(index.counts_at(20).collect::<Vec<_>>(), [(Some(1), 1), (Some(2), 1)]);
        assert_eq!(index.counts_at(30).collect::<Vec<_>>(), [(Some(1), 2), (Some(2), 1), (Some(3), 1)], "the larger listing replaces the smaller, never adds to it");
        assert_eq!(index.variants[&Some(1)], [(20, 1), (30, 2)], "one entry per change of a design, not a copy of every count");
    }
}
