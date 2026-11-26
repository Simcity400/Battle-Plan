use super::{
    Content, Design, Domain, ProductionPurpose, ProductionSpendingParams, ProductionSpendingReply, ProductionSpendingScope, ProductionSpendingUnknown, REPAIR_DENOMINATOR,
    RepairRead, SpanRead, UNRESOLVED_MARK, cached_rocket_name, designs, spans,
    runs::{Amount as RunAmount, Piece},
};
use dashboard_base::{
    Source,
    products::{
        Country, LoadContext,
        breakdown::{self, Amount, Breakdown, BreakdownPart, BreakdownState, Exact, Leaf, Node, Unrecorded},
        check_reply_budget,
    },
};
use dashboard_query::{
    Query,
    identity::{Holder, Key, TAG_UNLINKED},
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::atomic::AtomicBool,
};

const SPENT_SCALE: i128 = 100_000;
const REPAIR_SCALE: i128 = REPAIR_DENOMINATOR as i128 * SPENT_SCALE;
const UNRESOLVED_TYPE_KEY: &str = "~unresolved";
const REPAIR_KEY: &str = "repair";
const UNASSIGNED: &str = "Some designs\u{2019} equipment types are not recorded at this hour, so which factories spent their IC is not known";
const REPAIR_SPLIT_INCOMPLETE: &str = "Some dockyard repair spending\u{2019}s split by design is not known";

pub(crate) fn unknown_reason(why: ProductionSpendingUnknown) -> &'static str {
    match why {
        ProductionSpendingUnknown::NotRecorded => "Spending was not recorded",
        ProductionSpendingUnknown::SpendingUnavailable => "Spending is unavailable at this time",
        ProductionSpendingUnknown::LostOutput => "Final production output was not recorded",
        ProductionSpendingUnknown::MissedPublication => "Production spending publication was missed",
        ProductionSpendingUnknown::RecordingGap => "Some hours were not recorded",
        ProductionSpendingUnknown::ObservationRestarted => "Production observation restarted",
        ProductionSpendingUnknown::ObservationMismatch => "Production totals start on different dates",
        ProductionSpendingUnknown::CountryChanged => "Country changed during this period",
        ProductionSpendingUnknown::RocketTableUnavailable => "Rocket spending was not recorded",
        ProductionSpendingUnknown::RocketInputInexact => "Rocket spending total was not recorded exactly",
        ProductionSpendingUnknown::RocketOwnerUnreadable => "Rocket producer was not recorded",
        ProductionSpendingUnknown::RocketDesignUnreadable => "Rocket design was not recorded",
    }
}

fn not_recorded(reason: impl Into<String>) -> Breakdown {
    breakdown::breakdown(&[Leaf { path: Vec::new(), amount: Amount::NotRecorded(reason.into()), count: None, last: None }], &Unrecorded::default())
}

fn exact_spent(raw: i128) -> Amount {
    match Exact::new(raw, SPENT_SCALE) {
        Some(value) => Amount::Known(value),
        None => Amount::NotRecorded("Spending could not be represented exactly".into()),
    }
}

fn exact_repair(raw: i128) -> Amount {
    match Exact::new(raw, REPAIR_SCALE) {
        Some(value) => Amount::Known(value),
        None => Amount::NotRecorded("Repair spending could not be represented exactly".into()),
    }
}

fn piece_at<T>(pieces: &[Piece<T>], hour: i32) -> Option<&Piece<T>> {
    let at = pieces.partition_point(|piece| piece.1 < hour);
    pieces.get(at).filter(|piece| piece.0 <= hour && hour <= piece.1)
}

fn next_piece_edge<T>(pieces: &[Piece<T>], hour: i32) -> Option<i32> {
    let at = pieces.partition_point(|piece| piece.1 < hour);
    match pieces.get(at) {
        Some(piece) if piece.0 <= hour => piece.1.checked_add(1).filter(|&edge| edge > hour),
        Some(piece) => Some(piece.0),
        None => None,
    }
}

fn next_availability_edge(pieces: &[Piece<RunAmount>], hour: i32) -> Option<i32> {
    let known = |amount: &RunAmount| matches!(amount, RunAmount::Known(_));
    let at = pieces.partition_point(|piece| piece.1 < hour);
    let current = pieces.get(at)?;
    if current.0 > hour {
        return Some(current.0);
    }
    let mut end = current.1;
    for piece in &pieces[at + 1..] {
        if i64::from(piece.0) != i64::from(end) + 1 || known(&piece.2) != known(&current.2) || (!known(&piece.2) && piece.2 != current.2) {
            return end.checked_add(1);
        }
        end = piece.1;
    }
    end.checked_add(1)
}

fn tighten(through: &mut i32, edge: Option<i32>) {
    if let Some(edge) = edge
        && let Some(until) = edge.checked_sub(1)
    {
        *through = (*through).min(until);
    }
}

fn covering<'a, T>(spans: &'a [T], hour: i32, bounds: impl Fn(&T) -> (i32, i32)) -> Option<&'a T> {
    let at = spans.partition_point(|item| bounds(item).1 < hour);
    spans.get(at).filter(|item| {
        let (first, last) = bounds(item);
        first <= hour && hour <= last
    })
}

fn next_span_start<T>(spans: &[T], hour: i32, first: impl Fn(&T) -> i32) -> Option<i32> {
    let at = spans.partition_point(|item| first(item) <= hour);
    spans.get(at).map(first)
}

fn merge_spans(mut spans: Vec<(i32, i32)>) -> Vec<(i32, i32)> {
    spans.sort_unstable();
    let mut out: Vec<(i32, i32)> = Vec::with_capacity(spans.len());
    for (first, last) in spans {
        match out.last_mut() {
            Some(open) if i64::from(first) <= i64::from(open.1) + 1 => open.1 = open.1.max(last),
            _ => out.push((first, last)),
        }
    }
    out
}

pub(crate) struct IndexedSpan {
    pub(crate) read: SpanRead,
    pub(crate) designs: HashMap<u64, Design>,
    pub(crate) type_ordinals: HashMap<String, u32>,
    pub(crate) classified: HashMap<u64, i32>,
    pub(crate) notes: Vec<String>,
}

impl IndexedSpan {
    pub(crate) fn new(read: SpanRead, designs: HashMap<u64, Design>, notes: Vec<String>) -> Self {
        let mut type_keys: BTreeSet<String> = designs.values().map(|design| design.type_key.clone()).collect();
        for &(_, variant, _) in read.rows.keys() {
            if !designs.contains_key(&variant) {
                type_keys.insert(format!("{UNRESOLVED_MARK}variant/{variant}"));
            }
        }
        let type_ordinals = type_keys.into_iter().enumerate().map(|(at, key)| (key, at as u32)).collect();
        let classified = designs
            .iter()
            .filter(|(_, design)| design.domain.is_some())
            .filter_map(|(&variant, design)| read.archetype_since.get(&variant).copied().into_iter().chain(design.typed_since).min().map(|hour| (variant, hour)))
            .collect();
        Self { read, designs, type_ordinals, classified, notes }
    }

    pub(crate) fn classified(&self, variant: u64, hour: i32) -> Option<&Design> {
        self.designs.get(&variant).filter(|_| self.classified.get(&variant).is_some_and(|&from| from <= hour))
    }

    fn next_classification(&self, hour: i32) -> Option<i32> {
        self.classified.values().copied().filter(|&from| from > hour).min()
    }

    fn design_node(&self, variant: u64, rocket: bool, hour: i32) -> Node {
        let key = if rocket { format!("rocket:{variant}") } else { format!("variant:{variant}") };
        let ordinal = variant as u32;
        if let Some(design) = self.designs.get(&variant)
            && let Some(entry) = design.names.iter().rev().find(|entry| entry.first_hour <= hour)
            && entry.known
            && let Some(name) = entry.name.as_deref()
        {
            return Node::new(key, name, ordinal);
        }
        Node::new(key, format!("Design {variant}"), ordinal).name_missing(Some("Design name is not recorded at this hour".into()))
    }

    fn type_node(&self, variant: u64, hour: i32) -> Node {
        if let Some(design) = self.classified(variant, hour) {
            if !design.type_key.starts_with(UNRESOLVED_MARK) {
                let ordinal = self.type_ordinals.get(&design.type_key).copied().unwrap_or(0);
                return match design.name.as_deref() {
                    Some(name) => Node::new(&design.type_key, name, ordinal),
                    None => Node::new(&design.type_key, &design.type_key, ordinal).name_missing(Some("Equipment type name is not in the recorded localisation".into())),
                };
            }
            return Node::new(&design.type_key, "Equipment type name unavailable", u32::MAX).other();
        }
        Node::new(format!("{UNRESOLVED_MARK}variant/{variant}"), "Equipment type name unavailable", u32::MAX).other()
    }

    fn domain(&self, variant: u64, hour: i32) -> Option<Domain> {
        self.classified(variant, hour).and_then(|design| design.domain)
    }

    fn next_name_edge(&self, hour: i32, scope: ProductionSpendingScope) -> Option<i32> {
        self.designs
            .values()
            .filter(|design| match scope {
                ProductionSpendingScope::Military => design.domain != Some(Domain::Naval),
                ProductionSpendingScope::Dockyards | ProductionSpendingScope::DockyardPurposes => !matches!(design.domain, Some(Domain::Land | Domain::Air)),
                ProductionSpendingScope::AirPurposes => !matches!(design.domain, Some(Domain::Land | Domain::Naval)),
                ProductionSpendingScope::LandPurposes => !matches!(design.domain, Some(Domain::Air | Domain::Naval)),
            })
            .flat_map(|design| design.names.iter().map(|entry| entry.first_hour))
            .filter(|&first| first > hour)
            .min()
    }
}

pub(crate) struct SpendingIndex {
    pub(crate) from: i32,
    pub(crate) to: i32,
    pub(crate) country_label: String,
    pub(crate) unknown: Vec<(i32, i32, String)>,
    pub(crate) unresolved: Vec<(i32, i32)>,
    pub(crate) spans: Vec<IndexedSpan>,
    pub(crate) rocket_source: Option<Result<(String, Option<String>), String>>,
    pub(crate) warnings: Vec<String>,
}

enum RepairSegment {
    Gap(ProductionSpendingUnknown),
    Unsplit(i64),
    Split(Vec<(Option<u64>, i64)>),
}

impl RepairSegment {
    fn add(&self, hours: i128, sign: i128, designs: &mut BTreeMap<u64, i128>, unattributed: &mut i128) {
        match self {
            Self::Gap(_) => {}
            Self::Unsplit(rate) => *unattributed += sign * i128::from(*rate) * hours,
            Self::Split(parts) => {
                for &(design, rate) in parts {
                    let added = sign * i128::from(rate) * hours;
                    match design {
                        Some(variant) => *designs.entry(variant).or_default() += added,
                        None => *unattributed += added,
                    }
                }
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct RepairIndex {
    total: Vec<Piece<RunAmount>>,
    segments: Vec<(i32, i32, RepairSegment)>,
    unattributed: Vec<i128>,
    designs: BTreeMap<u64, Vec<(usize, i128)>>,
    gaps: Vec<usize>,
    incomplete: Vec<usize>,
}

impl RepairIndex {
    pub(crate) fn of(repairs: &RepairRead) -> Self {
        let mut edges: BTreeSet<i32> = BTreeSet::new();
        let mut collect = |first: i32, last: i32| {
            edges.insert(first);
            edges.extend(last.checked_add(1));
        };
        for &(first, last, _) in &repairs.total {
            collect(first, last);
        }
        for pieces in repairs.parts.values() {
            for &(first, last, _) in pieces {
                collect(first, last);
            }
        }
        let points: Vec<i32> = edges.into_iter().collect();
        let mut index = Self { total: repairs.total(), unattributed: vec![0], ..Self::default() };
        for pair in points.windows(2) {
            let (first, last) = (pair[0], pair[1] - 1);
            let segment = match piece_at(&repairs.total, first).and_then(|piece| piece.2) {
                None => RepairSegment::Gap(ProductionSpendingUnknown::NotRecorded),
                Some((None, _)) => RepairSegment::Gap(ProductionSpendingUnknown::SpendingUnavailable),
                Some((Some(rate), false)) => RepairSegment::Unsplit(rate),
                Some((Some(rate), true)) => {
                    let mut parts: Vec<(Option<u64>, i64)> = Vec::new();
                    let mut missing = false;
                    for (&design, pieces) in &repairs.parts {
                        match piece_at(pieces, first).and_then(|piece| piece.2) {
                            None => {}
                            Some(None) => {
                                missing = true;
                                break;
                            }
                            Some(Some(part)) if part != 0 => parts.push((design, part)),
                            Some(Some(_)) => {}
                        }
                    }
                    if missing { RepairSegment::Unsplit(rate) } else { RepairSegment::Split(parts) }
                }
            };
            let at = index.segments.len();
            let hours = i128::from(last - first + 1);
            let mut designs = BTreeMap::new();
            let mut unattributed = 0;
            segment.add(hours, 1, &mut designs, &mut unattributed);
            index.unattributed.push(index.unattributed[at] + unattributed);
            for (variant, added) in designs {
                let held = index.designs.entry(variant).or_default();
                let before = held.last().map_or(0, |entry| entry.1);
                held.push((at, before + added));
            }
            match segment {
                RepairSegment::Gap(_) => index.gaps.push(at),
                RepairSegment::Unsplit(rate) if rate != 0 => index.incomplete.push(at),
                _ => {}
            }
            index.segments.push((first, last, segment));
        }
        index
    }

    fn since(&self, hour: i32) -> Option<i32> {
        self.total.iter().find(|&&(first, _, amount)| first <= hour && matches!(amount, RunAmount::Known(_))).map(|piece| piece.0)
    }

    pub(super) fn sum(&self, since: i32, hour: i32) -> (BTreeMap<u64, i128>, i128, Option<ProductionSpendingUnknown>, bool) {
        let mut designs: BTreeMap<u64, i128> = BTreeMap::new();
        let a = self.segments.partition_point(|segment| segment.1 < since);
        let b = self.segments.partition_point(|segment| segment.0 <= hour);
        if a >= b {
            return (designs, 0, (since <= hour).then_some(ProductionSpendingUnknown::NotRecorded), false);
        }
        let first_gap = self.gaps.get(self.gaps.partition_point(|&at| at < a)).filter(|&&at| at < b).map(|&at| match self.segments[at].2 {
            RepairSegment::Gap(why) => why,
            _ => ProductionSpendingUnknown::NotRecorded,
        });
        let gap = if self.segments[a].0 > since { Some(ProductionSpendingUnknown::NotRecorded) } else { first_gap }
            .or_else(|| (self.segments[b - 1].1 < hour).then_some(ProductionSpendingUnknown::NotRecorded));
        let incomplete = self.incomplete.get(self.incomplete.partition_point(|&at| at < a)).is_some_and(|&at| at < b);
        let mut unattributed = self.unattributed[b] - self.unattributed[a];
        for (&variant, held) in &self.designs {
            let before = |at: usize| held[..held.partition_point(|entry| entry.0 < at)].last().map_or(0, |entry| entry.1);
            let sum = before(b) - before(a);
            if sum != 0 {
                designs.insert(variant, sum);
            }
        }
        let (first, _, segment) = &self.segments[a];
        if *first < since {
            segment.add(i128::from(since - first), -1, &mut designs, &mut unattributed);
        }
        let (_, last, segment) = &self.segments[b - 1];
        if *last > hour {
            segment.add(i128::from(last - hour), -1, &mut designs, &mut unattributed);
        }
        designs.retain(|_, sum| *sum != 0);
        (designs, unattributed, gap, incomplete)
    }
}

struct RepairSummary {
    recorded: bool,
    designs: BTreeMap<u64, i128>,
    unattributed: i128,
    gap: Option<ProductionSpendingUnknown>,
    split_incomplete: bool,
}

fn repair_summary(repairs: &RepairRead, hour: i32, floor: Option<i32>) -> RepairSummary {
    let start = repairs.total.first().map_or(i32::MIN, |piece| piece.0).max(floor.unwrap_or(i32::MIN));
    let Some(known) = repairs.index.since(hour) else {
        return RepairSummary { recorded: false, designs: BTreeMap::new(), unattributed: 0, gap: None, split_incomplete: false };
    };
    let since = known.max(start);
    let (designs, unattributed, gap, split_incomplete) = repairs.index.sum(since, hour);
    let gap = if known > start { Some(ProductionSpendingUnknown::NotRecorded) } else { gap };
    RepairSummary { recorded: true, designs, unattributed, gap, split_incomplete }
}

fn tighten_repairs(through: &mut i32, repairs: &RepairRead, hour: i32) {
    let total = &repairs.index.total;
    if let Some(next_hour) = hour.checked_add(1)
        && let Some(piece) = piece_at(total, next_hour)
        && matches!(piece.2, RunAmount::Known(rate) if rate != 0)
    {
        *through = (*through).min(hour);
        return;
    }
    tighten(through, next_piece_edge(total, hour));
    for pieces in repairs.parts.values() {
        tighten(through, next_piece_edge(pieces, hour));
    }
}

fn purpose_node(purpose: ProductionPurpose, dockyard: bool) -> Node {
    if dockyard {
        match purpose {
            ProductionPurpose::New => Node::new("new", "New ships", 0),
            ProductionPurpose::Refit => Node::new("refit", "Refits", 1),
            ProductionPurpose::Unknown => Node::new("purpose-unrecorded", "Purpose not recorded", 3).other(),
            ProductionPurpose::Converted => Node::new("other-purpose", "Other purposes", 4).other(),
        }
    } else {
        match purpose {
            ProductionPurpose::New => Node::new("new", "New", 0),
            ProductionPurpose::Converted => Node::new("converted", "Converted", 1),
            ProductionPurpose::Unknown => Node::new("purpose-unrecorded", "Purpose not recorded", 2).other(),
            ProductionPurpose::Refit => Node::new("other-purpose", "Other purposes", 3).other(),
        }
    }
}

fn rocket_node(source: Option<&Result<(String, Option<String>), String>>) -> Node {
    let label = match source {
        Some(Ok((key, name))) => name.as_deref().unwrap_or(key.as_str()),
        _ => "Rocket sites",
    };
    Node::new("rocket", label, 2)
}

pub(crate) struct Built {
    span: usize,
    from: i32,
    rocket_from: i32,
    leaves: Vec<Leaf>,
    reasons: Vec<String>,
    unassigned: bool,
    repair_gap: Option<&'static str>,
    repair_split_incomplete: bool,
    through: i32,
    warnings: Vec<String>,
}

const PURPOSE_KEYS: [&str; 5] = ["new", "refit", "converted", "purpose-unrecorded", "other-purpose"];

fn unassigned_node() -> Node {
    Node::new(UNRESOLVED_TYPE_KEY, "Type not resolved", u32::MAX).other()
}

fn leaf_key(leaf: &Leaf) -> Vec<String> {
    match leaf.path.last() {
        Some(node) if node.key.starts_with("variant:") || node.key.starts_with("rocket:") => {
            let root = match leaf.path.first() {
                Some(first) if first.key == UNRESOLVED_TYPE_KEY => leaf.path.get(1),
                first => first,
            };
            let purpose = root.filter(|node| PURPOSE_KEYS.contains(&node.key.as_str())).map_or("", |node| node.key.as_str());
            vec![purpose.to_owned(), node.key.clone()]
        }
        _ => leaf.path.iter().map(|node| node.key.clone()).collect(),
    }
}

fn append_reason(target: &mut Option<String>, reason: &str) {
    *target = Some(match target.take() {
        Some(held) if held.contains(reason) => held,
        Some(held) => format!("{held}; {reason}"),
        None => reason.to_owned(),
    });
}

fn qualify_repair_children(parts: &mut [BreakdownPart], gap_reason: Option<&str>, split_incomplete: bool) {
    for part in parts {
        part.share = None;
        part.parent_share = None;
        let is_unrecorded = part.key == "~unrecorded";
        let is_unattributed = part.key == "~unattributed";
        if !is_unrecorded {
            if let Some(reason) = gap_reason {
                if part.state == BreakdownState::Known {
                    part.state = BreakdownState::Partial;
                }
                append_reason(&mut part.reason, reason);
            }
            if split_incomplete && !is_unattributed {
                if part.state == BreakdownState::Known {
                    part.state = BreakdownState::Partial;
                }
                append_reason(&mut part.reason, REPAIR_SPLIT_INCOMPLETE);
            }
        }
        qualify_repair_children(&mut part.children, gap_reason, split_incomplete);
    }
}

impl Built {
    fn reply(self, hour: i32) -> ProductionSpendingReply {
        let mut breakdown = if self.unassigned {
            breakdown::breakdown_of(&self.leaves, &Amount::NotRecorded(UNASSIGNED.into()))
        } else {
            breakdown::breakdown_qualified(&self.leaves, &self.reasons)
        };
        if (self.repair_gap.is_some() || self.repair_split_incomplete)
            && let Some(repair) = breakdown.parts.iter_mut().find(|part| part.key == REPAIR_KEY)
        {
            qualify_repair_children(&mut repair.children, self.repair_gap, self.repair_split_incomplete);
        }
        ProductionSpendingReply { from: self.from, hour, through: self.through, breakdown, warnings: self.warnings }
    }

    fn minus(mut self, start: &Built, classified: impl Fn(u64) -> bool) -> Built {
        let counters: Vec<&Leaf> = start.leaves.iter().filter(|leaf| leaf.path.first().is_none_or(|node| node.key != REPAIR_KEY)).collect();
        let held: HashMap<Vec<String>, &Amount> = counters.iter().map(|leaf| (leaf_key(leaf), &leaf.amount)).collect();
        for leaf in &mut self.leaves {
            if leaf.path.first().is_some_and(|node| node.key == REPAIR_KEY) {
                continue;
            }
            let before = held.get(&leaf_key(leaf)).copied();
            let next = match (&leaf.amount, before) {
                (amount, None) => amount.clone(),
                (Amount::Known(end), Some(Amount::Known(start))) => end
                    .numerator()
                    .checked_mul(start.denominator())
                    .zip(start.numerator().checked_mul(end.denominator()))
                    .and_then(|(a, b)| a.checked_sub(b))
                    .zip(end.denominator().checked_mul(start.denominator()))
                    .and_then(|(numerator, denominator)| Exact::new(numerator, denominator))
                    .map_or_else(|| Amount::NotRecorded("Spending could not be represented exactly".into()), Amount::Known),
                (Amount::NotRecorded(reason) | Amount::NotRecordedSigned(reason), _) | (_, Some(Amount::NotRecorded(reason) | Amount::NotRecordedSigned(reason))) => Amount::NotRecorded(reason.clone()),
                (Amount::Partial(_, reason), _) | (_, Some(Amount::Partial(_, reason))) => Amount::NotRecorded(format!("The amount at the range start is not known exactly: {reason}")),
            };
            leaf.amount = next;
        }
        let keep: std::collections::HashSet<Vec<String>> = self.leaves.iter().map(leaf_key).collect();
        let design = |leaf: &Leaf| leaf.path.last().and_then(|node| node.key.strip_prefix("variant:")).and_then(|id| id.parse::<u64>().ok());
        for leaf in counters {
            if keep.contains(&leaf_key(leaf)) || matches!(leaf.amount, Amount::Known(value) if value.numerator() == 0) || design(leaf).is_some_and(&classified) {
                continue;
            }
            self.leaves.push(Leaf { path: leaf.path.clone(), amount: Amount::NotRecorded("Spent before the range start and not found at its end".into()), count: None, last: None });
        }
        self.leaves.retain(|leaf| !matches!(leaf.amount, Amount::Known(value) if value.numerator() == 0));
        self.unassigned = self.leaves.iter().any(|leaf| leaf.path.first().is_some_and(|node| node.key == UNRESOLVED_TYPE_KEY));
        self
    }
}

impl SpendingIndex {
    fn build(&self, hour: i32, scope: ProductionSpendingScope, include_refits: bool, repairs_from: Option<i32>) -> Result<Built, Box<ProductionSpendingReply>> {
        let held = self.spans.iter().any(|span| span.read.span.first <= hour && hour <= span.read.span.last);
        let held_from = next_span_start(&self.spans, hour, |span| span.read.span.first).map_or(i32::MAX, |first| first - 1);
        if !held && let Some((_, last, reason)) = covering(&self.unknown, hour, |&(first, last, _)| (first, last)) {
            return Err(Box::new(ProductionSpendingReply {
                from: self.from,
                hour,
                through: (*last).min(self.to).min(held_from).max(hour),
                breakdown: not_recorded(format!("Who holds {} is not known: {reason}", self.country_label)),
                warnings: self.warnings.clone(),
            }));
        }
        if !held && let Some((_, last)) = covering(&self.unresolved, hour, |&(first, last)| (first, last)) {
            return Err(Box::new(ProductionSpendingReply {
                from: self.from,
                hour,
                through: (*last).min(self.to).min(held_from).max(hour),
                breakdown: not_recorded(format!("Who holds {} is not known: {TAG_UNLINKED}", self.country_label)),
                warnings: self.warnings.clone(),
            }));
        }

        let Some(span_at) = self.spans.iter().position(|span| span.read.span.first <= hour && hour <= span.read.span.last) else {
            let mut through = self.to;
            tighten(&mut through, next_span_start(&self.spans, hour, |span| span.read.span.first));
            tighten(&mut through, next_span_start(&self.unknown, hour, |&(first, _, _)| first));
            tighten(&mut through, next_span_start(&self.unresolved, hour, |&(first, _)| first));
            return Err(Box::new(ProductionSpendingReply {
                from: self.from,
                hour,
                through: through.max(hour),
                breakdown: breakdown::breakdown(&[], &Unrecorded::default()),
                warnings: self.warnings.clone(),
            }));
        };

        let span = &self.spans[span_at];
        let since = |observations: &BTreeSet<i32>| observations.range(..=hour).next_back().map_or(span.read.span.first, |&since| since.max(span.read.span.first));
        let (observed_from, rocket_from) = (since(&span.read.equipment_observations), since(&span.read.rocket_observations));
        let mut through = span.read.span.last.min(self.to);
        tighten(&mut through, span.read.equipment_observations.range(hour.saturating_add(1)..).next().copied());
        tighten(&mut through, span.read.rocket_observations.range(hour.saturating_add(1)..).next().copied());
        tighten(&mut through, span.next_name_edge(hour, scope));
        tighten(&mut through, span.next_classification(hour));

        if self.spans[..span_at].iter().any(|prev| prev.read.span.id != span.read.span.id) {
            return Err(Box::new(ProductionSpendingReply {
                from: self.from,
                hour,
                through: through.max(hour),
                breakdown: not_recorded(unknown_reason(ProductionSpendingUnknown::CountryChanged)),
                warnings: self.warnings.clone(),
            }));
        }

        let mut warnings = self.warnings.clone();
        warnings.extend(span.notes.iter().cloned());

        let equipment_pieces = span.read.equipment(include_refits || matches!(scope, ProductionSpendingScope::DockyardPurposes | ProductionSpendingScope::AirPurposes | ProductionSpendingScope::LandPurposes));
        tighten(&mut through, next_availability_edge(&equipment_pieces, hour));
        let equipment_at = piece_at(&equipment_pieces, hour).map_or(RunAmount::Unknown(ProductionSpendingUnknown::NotRecorded), |piece| piece.2);

        let mut leaves: Vec<Leaf> = Vec::new();
        let mut reasons: Vec<String> = Vec::new();
        let mut unassigned = false;
        let mut repair_gap = None;
        let mut repair_split_incomplete = false;

        match scope {
            ProductionSpendingScope::Military => {
                let rocket_pieces = span.read.rocket_total();
                tighten(&mut through, next_piece_edge(&rocket_pieces, hour));
                let rocket_at = piece_at(&rocket_pieces, hour).map_or(RunAmount::Unknown(ProductionSpendingUnknown::RocketTableUnavailable), |piece| piece.2);
                for (key, row_pieces) in &span.read.rows {
                    let (rocket, variant, purpose) = *key;
                    if !rocket && !include_refits && purpose.map(ProductionPurpose::of) == Some(ProductionPurpose::Refit) {
                        continue;
                    }
                    if !rocket && span.domain(variant, hour) == Some(Domain::Naval) {
                        continue;
                    }
                    tighten(&mut through, next_piece_edge(row_pieces, hour));
                }
                match (equipment_at, rocket_at) {
                    (RunAmount::Unknown(eq_why), RunAmount::Unknown(_)) => {
                        return Err(Box::new(ProductionSpendingReply {
                            from: self.from,
                            hour,
                            through: through.max(hour),
                            breakdown: not_recorded(unknown_reason(eq_why)),
                            warnings,
                        }));
                    }
                    (RunAmount::Unknown(eq_why), RunAmount::Known(0)) => {
                        return Err(Box::new(ProductionSpendingReply {
                            from: self.from,
                            hour,
                            through: through.max(hour),
                            breakdown: not_recorded(unknown_reason(eq_why)),
                            warnings,
                        }));
                    }
                    _ => {}
                }
                let mut rocket_present = false;
                if let RunAmount::Unknown(eq_why) = equipment_at {
                    leaves.push(Leaf {
                        path: vec![Node::new("~equipment", "Military factories", u32::MAX).other()],
                        amount: Amount::NotRecorded(unknown_reason(eq_why).into()),
                        count: None,
                        last: None,
                    });
                } else {
                    let mut by_variant: BTreeMap<u64, (i128, Option<ProductionSpendingUnknown>)> = BTreeMap::new();
                    for key in span.read.rows.keys() {
                        let (rocket, variant, purpose) = *key;
                        if rocket || span.domain(variant, hour) == Some(Domain::Naval) {
                            continue;
                        }
                        if !include_refits && purpose.map(ProductionPurpose::of) == Some(ProductionPurpose::Refit) {
                            continue;
                        }
                        let pieces = span.read.amounts(key, |row| row.spent);
                        let amount = piece_at(&pieces, hour).map_or(RunAmount::Known(0), |piece| piece.2);
                        let entry = by_variant.entry(variant).or_insert((0, None));
                        match amount {
                            RunAmount::Known(value) => entry.0 += value,
                            RunAmount::Unknown(why) => {
                                entry.1.get_or_insert(why);
                            }
                        }
                    }
                    let mut unresolved_nonzero = false;
                    for (variant, (sum, unknown)) in by_variant {
                        let domain = span.domain(variant, hour);
                        let source = match domain {
                            Some(Domain::Land) => Node::new("land", "Land", 0),
                            Some(Domain::Air) => Node::new("air", "Air", 1),
                            Some(Domain::Naval) => continue,
                            None => Node::new(UNRESOLVED_TYPE_KEY, "Type not resolved", u32::MAX).other(),
                        };
                        let amount = match (sum, unknown) {
                            (0, None) => continue,
                            (value, None) => exact_spent(value),
                            (0, Some(why)) => Amount::NotRecorded(unknown_reason(why).into()),
                            (value, Some(why)) => match Exact::new(value, SPENT_SCALE) {
                                Some(exact) => Amount::Partial(exact, unknown_reason(why).into()),
                                None => Amount::NotRecorded(unknown_reason(why).into()),
                            },
                        };
                        if domain.is_none() {
                            unresolved_nonzero = true;
                        }
                        leaves.push(Leaf { path: vec![source, span.type_node(variant, hour), span.design_node(variant, false, hour)], amount, count: None, last: None });
                    }
                    if unresolved_nonzero {
                        unassigned = true;
                    }
                }
                match rocket_at {
                    RunAmount::Unknown(why) => {
                        leaves.push(Leaf {
                            path: vec![rocket_node(self.rocket_source.as_ref())],
                            amount: Amount::NotRecorded(unknown_reason(why).into()),
                            count: None,
                            last: None,
                        });
                    }
                    RunAmount::Known(_) => {
                        for key in span.read.rows.keys().filter(|key| key.0) {
                            let (_, variant, _) = *key;
                            let pieces = span.read.amounts(key, |row| row.spent);
                            let amount = match piece_at(&pieces, hour).map_or(RunAmount::Known(0), |piece| piece.2) {
                                RunAmount::Known(0) => continue,
                                RunAmount::Known(value) => exact_spent(value),
                                RunAmount::Unknown(why) => Amount::NotRecorded(unknown_reason(why).into()),
                            };
                            rocket_present = true;
                            leaves.push(Leaf {
                                path: vec![rocket_node(self.rocket_source.as_ref()), span.type_node(variant, hour), span.design_node(variant, true, hour)],
                                amount,
                                count: None,
                                last: None,
                            });
                        }
                    }
                }
                if rocket_present && let Some(Err(reason)) = &self.rocket_source {
                    warnings.push(format!("Rocket production source name is unavailable: {reason}."));
                }
            }
            ProductionSpendingScope::Dockyards => {
                for (key, row_pieces) in &span.read.rows {
                    let (rocket, variant, purpose) = *key;
                    if rocket || (!include_refits && purpose.map(ProductionPurpose::of) == Some(ProductionPurpose::Refit)) {
                        continue;
                    }
                    if matches!(span.domain(variant, hour), Some(Domain::Land | Domain::Air)) {
                        continue;
                    }
                    tighten(&mut through, next_piece_edge(row_pieces, hour));
                }
                if let RunAmount::Unknown(why) = equipment_at {
                    return Err(Box::new(ProductionSpendingReply {
                        from: self.from,
                        hour,
                        through: through.max(hour),
                        breakdown: not_recorded(unknown_reason(why)),
                        warnings,
                    }));
                }
                let mut by_variant: BTreeMap<u64, (i128, Option<ProductionSpendingUnknown>)> = BTreeMap::new();
                for key in span.read.rows.keys() {
                    let (rocket, variant, purpose) = *key;
                    if rocket || matches!(span.domain(variant, hour), Some(Domain::Land | Domain::Air)) {
                        continue;
                    }
                    if !include_refits && purpose.map(ProductionPurpose::of) == Some(ProductionPurpose::Refit) {
                        continue;
                    }
                    let pieces = span.read.amounts(key, |row| row.spent);
                    let amount = piece_at(&pieces, hour).map_or(RunAmount::Known(0), |piece| piece.2);
                    let entry = by_variant.entry(variant).or_insert((0, None));
                    match amount {
                        RunAmount::Known(value) => entry.0 += value,
                        RunAmount::Unknown(why) => {
                            entry.1.get_or_insert(why);
                        }
                    }
                }
                let mut unresolved_nonzero = false;
                for (variant, (sum, unknown)) in by_variant {
                    let domain = span.domain(variant, hour);
                    let source = match domain {
                        Some(Domain::Naval) => Node::new("naval", "Naval", 0),
                        None => Node::new(UNRESOLVED_TYPE_KEY, "Type not resolved", u32::MAX).other(),
                        _ => continue,
                    };
                    let amount = match (sum, unknown) {
                        (0, None) => continue,
                        (value, None) => exact_spent(value),
                        (0, Some(why)) => Amount::NotRecorded(unknown_reason(why).into()),
                        (value, Some(why)) => match Exact::new(value, SPENT_SCALE) {
                            Some(exact) => Amount::Partial(exact, unknown_reason(why).into()),
                            None => Amount::NotRecorded(unknown_reason(why).into()),
                        },
                    };
                    if domain.is_none() {
                        unresolved_nonzero = true;
                    }
                    leaves.push(Leaf { path: vec![source, span.type_node(variant, hour), span.design_node(variant, false, hour)], amount, count: None, last: None });
                }
                if unresolved_nonzero {
                    unassigned = true;
                }
            }
            ProductionSpendingScope::DockyardPurposes => {
                for (key, row_pieces) in &span.read.rows {
                    let (rocket, variant, _) = *key;
                    if rocket || matches!(span.domain(variant, hour), Some(Domain::Land | Domain::Air)) {
                        continue;
                    }
                    tighten(&mut through, next_piece_edge(row_pieces, hour));
                }
                if let Some(repairs) = &span.read.repairs {
                    tighten_repairs(&mut through, repairs, hour);
                }
                if let RunAmount::Unknown(why) = equipment_at {
                    let text = unknown_reason(why);
                    leaves.push(Leaf { path: vec![purpose_node(ProductionPurpose::New, true)], amount: Amount::NotRecorded(text.into()), count: None, last: None });
                    leaves.push(Leaf { path: vec![purpose_node(ProductionPurpose::Refit, true)], amount: Amount::NotRecorded(text.into()), count: None, last: None });
                } else {
                    let mut unproven_purpose = false;
                    for key in span.read.rows.keys() {
                        let (rocket, variant, purpose) = *key;
                        if rocket {
                            continue;
                        }
                        let domain = span.domain(variant, hour);
                        if matches!(domain, Some(Domain::Land | Domain::Air)) {
                            continue;
                        }
                        let purpose = purpose.map_or(ProductionPurpose::Unknown, ProductionPurpose::of);
                        let spent_pieces = span.read.amounts(key, |row| row.spent);
                        let spent_at = piece_at(&spent_pieces, hour).map_or(RunAmount::Known(0), |piece| piece.2);
                        if purpose == ProductionPurpose::Unknown {
                            let unproven_pieces = span.read.amounts(key, |row| row.unproven);
                            if !matches!(piece_at(&unproven_pieces, hour).map_or(RunAmount::Known(0), |piece| piece.2), RunAmount::Known(0)) {
                                unproven_purpose = true;
                            }
                        }
                        if domain.is_none() && !matches!(spent_at, RunAmount::Known(0)) {
                            unassigned = true;
                        }
                        let amount = match spent_at {
                            RunAmount::Known(0) => continue,
                            RunAmount::Known(value) => exact_spent(value),
                            RunAmount::Unknown(why) => Amount::NotRecorded(unknown_reason(why).into()),
                        };
                        leaves.push(Leaf {
                            path: if domain.is_some() { vec![purpose_node(purpose, true), span.type_node(variant, hour), span.design_node(variant, false, hour)] } else { vec![unassigned_node(), purpose_node(purpose, true), span.design_node(variant, false, hour)] },
                            amount,
                            count: None,
                            last: None,
                        });
                    }
                    if unproven_purpose {
                        reasons.push("Whether some of it was refits was not recorded".into());
                    }
                }
                let repair_root = Node::new(REPAIR_KEY, "Repairs", 2);
                match span.read.repairs.as_ref().map(|repairs| repair_summary(repairs, hour, repairs_from)) {
                    None | Some(RepairSummary { recorded: false, .. }) => {
                        leaves.push(Leaf { path: vec![repair_root], amount: Amount::NotRecorded("Spending was not recorded".into()), count: None, last: None });
                    }
                    Some(summary) => {
                        let gap_reason = summary.gap.map(unknown_reason);
                        let mut any_repair = false;
                        for (variant, sum) in summary.designs {
                            if sum == 0 {
                                continue;
                            }
                            any_repair = true;
                            leaves.push(Leaf {
                                path: vec![repair_root.clone(), span.type_node(variant, hour), span.design_node(variant, false, hour)],
                                amount: exact_repair(sum),
                                count: None,
                                last: None,
                            });
                        }
                        if summary.unattributed != 0 {
                            any_repair = true;
                            leaves.push(Leaf {
                                path: vec![repair_root.clone(), Node::new("~unattributed", "Unattributed", u32::MAX).other()],
                                amount: exact_repair(summary.unattributed),
                                count: None,
                                last: None,
                            });
                        }
                        if let Some(reason) = gap_reason {
                            repair_gap = Some(reason);
                            if any_repair {
                                leaves.push(Leaf {
                                    path: vec![repair_root, Node::new("~unrecorded", "Unrecorded hours", u32::MAX).other()],
                                    amount: Amount::NotRecorded(reason.into()),
                                    count: None,
                                    last: None,
                                });
                            } else {
                                leaves.push(Leaf { path: vec![repair_root], amount: Amount::NotRecorded(reason.into()), count: None, last: None });
                            }
                        }
                        if summary.split_incomplete {
                            repair_split_incomplete = true;
                            warnings.push(format!("{REPAIR_SPLIT_INCOMPLETE}."));
                        }
                    }
                }
            }
            ProductionSpendingScope::AirPurposes | ProductionSpendingScope::LandPurposes => {
                let target = if scope == ProductionSpendingScope::AirPurposes { Domain::Air } else { Domain::Land };
                for (key, row_pieces) in &span.read.rows {
                    let (rocket, variant, _) = *key;
                    if rocket || (span.domain(variant, hour).is_some() && span.domain(variant, hour) != Some(target)) {
                        continue;
                    }
                    tighten(&mut through, next_piece_edge(row_pieces, hour));
                }
                if let RunAmount::Unknown(why) = equipment_at {
                    return Err(Box::new(ProductionSpendingReply {
                        from: self.from,
                        hour,
                        through: through.max(hour),
                        breakdown: not_recorded(unknown_reason(why)),
                        warnings,
                    }));
                }
                let mut unproven_purpose = false;
                for key in span.read.rows.keys() {
                    let (rocket, variant, purpose) = *key;
                    if rocket {
                        continue;
                    }
                    let domain = span.domain(variant, hour);
                    if domain.is_some() && domain != Some(target) {
                        continue;
                    }
                    let purpose = purpose.map_or(ProductionPurpose::Unknown, ProductionPurpose::of);
                    let spent_pieces = span.read.amounts(key, |row| row.spent);
                    let spent_at = piece_at(&spent_pieces, hour).map_or(RunAmount::Known(0), |piece| piece.2);
                    if purpose == ProductionPurpose::Unknown {
                        let unproven_pieces = span.read.amounts(key, |row| row.unproven);
                        if !matches!(piece_at(&unproven_pieces, hour).map_or(RunAmount::Known(0), |piece| piece.2), RunAmount::Known(0)) {
                            unproven_purpose = true;
                        }
                    }
                    if domain.is_none() && !matches!(spent_at, RunAmount::Known(0)) {
                        unassigned = true;
                    }
                    let amount = match spent_at {
                        RunAmount::Known(0) => continue,
                        RunAmount::Known(value) => exact_spent(value),
                        RunAmount::Unknown(why) => Amount::NotRecorded(unknown_reason(why).into()),
                    };
                    leaves.push(Leaf {
                        path: if domain.is_some() { vec![purpose_node(purpose, false), span.type_node(variant, hour), span.design_node(variant, false, hour)] } else { vec![unassigned_node(), purpose_node(purpose, false), span.design_node(variant, false, hour)] },
                        amount,
                        count: None,
                        last: None,
                    });
                }
                if unproven_purpose {
                    reasons.push("Whether some of it was converted was not recorded".into());
                }
            }
        }

        reasons.sort();
        reasons.dedup();
        warnings.dedup();
        Ok(Built { span: span_at, from: observed_from, rocket_from, leaves, reasons, unassigned, repair_gap, repair_split_incomplete, through: through.max(hour), warnings })
    }

    pub(crate) fn at(&self, hour: i32, scope: ProductionSpendingScope, include_refits: bool) -> ProductionSpendingReply {
        match self.build(hour, scope, include_refits, None) {
            Ok(built) if scope == ProductionSpendingScope::Military && built.rocket_from != built.from => self.between(built.from.max(built.rocket_from), hour, scope, include_refits),
            Ok(built) => built.reply(hour),
            Err(reply) => *reply,
        }
    }

    fn counted_before(&self, from: i32, until: i32) -> bool {
        self.spans.iter().any(|span| span.read.span.first < until && span.read.span.last >= from)
    }

    pub(crate) fn between(&self, from: i32, hour: i32, scope: ProductionSpendingScope, include_refits: bool) -> ProductionSpendingReply {
        let end = match self.build(hour, scope, include_refits, Some(from)) {
            Ok(end) => end,
            Err(reply) => return ProductionSpendingReply { from, ..*reply },
        };
        let through = end.through;
        let refused = |reason: &str| ProductionSpendingReply { from, hour, through, breakdown: not_recorded(reason), warnings: end.warnings.clone() };
        let rocket = |leaf: &Leaf| leaf.path.first().is_some_and(|node| node.key == "rocket");
        let rockets = scope == ProductionSpendingScope::Military;
        let counters = [Some(end.from), rockets.then_some(end.rocket_from)];
        if counters.into_iter().flatten().any(|since| since > from && self.counted_before(from, since)) {
            return refused(unknown_reason(ProductionSpendingUnknown::ObservationRestarted));
        }
        let (equipment_baseline, rocket_baseline) = (end.from < from, rockets && end.rocket_from < from);
        if !equipment_baseline && !rocket_baseline {
            return ProductionSpendingReply { from, ..end.reply(hour) };
        }
        let mut start = match self.build(from - 1, scope, include_refits, None) {
            Ok(start) if start.span == end.span && (!equipment_baseline || start.from == end.from) && (!rocket_baseline || start.rocket_from == end.rocket_from) => start,
            Ok(_) => return refused(unknown_reason(ProductionSpendingUnknown::ObservationRestarted)),
            Err(_) if !equipment_baseline && piece_at(&self.spans[end.span].read.rocket_total(), from - 1).is_some_and(|piece| matches!(piece.2, RunAmount::Known(0))) => {
                Built { span: end.span, from: end.from, rocket_from: end.rocket_from, leaves: Vec::new(), reasons: Vec::new(), unassigned: false, repair_gap: None, repair_split_incomplete: false, through: end.through, warnings: Vec::new() }
            }
            Err(reply) => return refused(reply.breakdown.total.reason.as_deref().unwrap_or(unknown_reason(ProductionSpendingUnknown::NotRecorded))),
        };
        start.leaves.retain(|leaf| if rocket(leaf) { rocket_baseline } else { equipment_baseline });
        let span = &self.spans[end.span];
        let mut reply = end.minus(&start, |variant| span.classified(variant, hour).is_some()).reply(hour);
        reply.from = from;
        reply
    }
}

fn unresolved_spans(query: &Query<'_>, country: &Country) -> Vec<(i32, i32)> {
    let spans: Vec<(i32, i32)> = match country {
        Country::Tag(tag) => tag.runs(&query.holders).iter().filter(|run| run.holder == Holder::Unresolved).map(|run| (run.first, run.last)).collect(),
        Country::Slot(_) => query.holders.tags.values().flatten().filter(|run| run.holder == Holder::Unresolved).map(|run| (run.first, run.last)).collect(),
    };
    merge_spans(spans)
}

fn build_index(source: &Source, query: &Query<'_>, country: &Country, repairs: bool, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<SpendingIndex> {
    let mut warnings = Vec::new();
    let content = Content::read(source, context, &mut warnings);
    let mut indexed = Vec::new();
    let mut any_rocket = false;
    for span in spans(query, country) {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let read = SpanRead::read(query, span, repairs)?;
        if read.rows.keys().any(|key| key.0) {
            any_rocket = true;
        }
        let mut variants: BTreeMap<u64, u32> = read.rows.keys().map(|&(_, variant, _)| (variant, read.archetypes.get(&variant).copied().unwrap_or(0))).collect();
        for design in read.repairs.iter().flat_map(|repairs| repairs.parts.keys().flatten()) {
            variants.entry(*design).or_insert(0);
        }
        let mut notes = BTreeSet::new();
        let named = designs(query, &read.span, &variants, &content, &mut notes)?;
        indexed.push(IndexedSpan::new(read, named, notes.into_iter().collect()));
    }
    let rocket_source = content.catalogue.as_ref().filter(|_| any_rocket).map(|catalogue| cached_rocket_name(source, catalogue));
    Ok(SpendingIndex {
        from: query.timeline.first,
        to: query.timeline.last,
        country_label: country.to_string(),
        unknown: query.holders.unknown.clone(),
        unresolved: unresolved_spans(query, country),
        spans: indexed,
        rocket_source,
        warnings,
    })
}

pub(crate) fn load(source: &Source, params: &ProductionSpendingParams, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<ProductionSpendingReply> {
    dashboard_text::metadata::check_cancel(cancelled)?;
    let branch = params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
    let query = dashboard_base::queries::query(source, branch)?;
    let repairs = params.scope == ProductionSpendingScope::DockyardPurposes;
    let key = (params.branch_id.clone(), params.country.clone(), repairs, query.timeline.first, query.timeline.last);
    let index = breakdown::held(source, context, key, || build_index(source, &query, &params.country, repairs, context, cancelled))?;
    let reply = match params.from_hour {
        Some(from) => index.between(from, params.hour, params.scope, params.include_refits),
        None => index.at(params.hour, params.scope, params.include_refits),
    };
    check_reply_budget(&reply)?;
    Ok(reply)
}

#[cfg(test)]
mod key_tests {
    use super::{Leaf, Node, REPAIR_KEY, leaf_key};
    use dashboard_base::products::breakdown::{Amount, Exact};

    #[test]
    fn a_counter_leaf_is_keyed_by_its_purpose_position_never_by_a_type_named_like_a_purpose() {
        let leaf = |path: Vec<Node>| Leaf { path, amount: Amount::Known(Exact::whole(1)), count: None, last: None };
        let repair = leaf(vec![Node::new(REPAIR_KEY, "Repairs", 2), Node::new("new", "A type named new", 0), Node::new("variant:7", "Design", 7)]);
        let built = leaf(vec![Node::new("new", "New ships", 0), Node::new("hull", "Hull", 0), Node::new("variant:7", "Design", 7)]);
        assert_ne!(leaf_key(&repair), leaf_key(&built), "a repair is never matched to a shipbuilding baseline");
        assert_eq!(leaf_key(&built), ["new", "variant:7"]);
    }
}
