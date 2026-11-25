use super::{
    Content, Design, Domain, ProductionPurpose, ProductionSpendingParams, ProductionSpendingReply, ProductionSpendingScope, ProductionSpendingUnknown, REPAIR_DENOMINATOR,
    RepairRead, SpanRead, UNRESOLVED_MARK, cached_rocket_name, designs, spans,
    runs::{Amount as RunAmount, Piece},
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
        Self { read, designs, type_ordinals, notes }
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

    fn type_node(&self, variant: u64) -> Node {
        if let Some(design) = self.designs.get(&variant) {
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

    fn domain(&self, variant: u64) -> Option<Domain> {
        self.designs.get(&variant).and_then(|design| design.domain)
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

fn repair_since(repairs: &RepairRead, hour: i32) -> Option<i32> {
    repairs
        .total()
        .into_iter()
        .find(|&(first, _, amount)| first <= hour && matches!(amount, RunAmount::Known(_)))
        .map(|(first, _, _)| first)
}

struct RepairSummary {
    recorded: bool,
    designs: BTreeMap<u64, i128>,
    unattributed: i128,
    gap: Option<ProductionSpendingUnknown>,
    split_incomplete: bool,
}

fn repair_summary(repairs: &RepairRead, hour: i32) -> RepairSummary {
    let Some(since) = repair_since(repairs, hour) else {
        return RepairSummary { recorded: false, designs: BTreeMap::new(), unattributed: 0, gap: None, split_incomplete: false };
    };
    let mut designs: BTreeMap<u64, i128> = BTreeMap::new();
    let mut unattributed: i128 = 0;
    let mut gap: Option<ProductionSpendingUnknown> = None;
    let mut split_incomplete = false;
    let mut edges: BTreeSet<i32> = BTreeSet::new();
    edges.insert(since);
    if let Some(after) = hour.checked_add(1) {
        edges.insert(after);
    }
    let collect = |edges: &mut BTreeSet<i32>, first: i32, last: i32| {
        if last < since || first > hour {
            return;
        }
        edges.insert(first.max(since));
        if let Some(after) = last.min(hour).checked_add(1) {
            edges.insert(after);
        }
    };
    for &(first, last, _) in &repairs.total {
        collect(&mut edges, first, last);
    }
    for pieces in repairs.parts.values() {
        for &(first, last, _) in pieces {
            collect(&mut edges, first, last);
        }
    }
    let points: Vec<i32> = edges.into_iter().collect();
    for pair in points.windows(2) {
        let seg_first = pair[0];
        let seg_last = pair[1] - 1;
        if seg_first > seg_last || seg_last < since || seg_first > hour {
            continue;
        }
        let hours = i128::from(seg_last - seg_first + 1);
        match piece_at(&repairs.total, seg_first).and_then(|piece| piece.2) {
            None => {
                gap.get_or_insert(ProductionSpendingUnknown::NotRecorded);
            }
            Some((None, _)) => {
                gap.get_or_insert(ProductionSpendingUnknown::SpendingUnavailable);
            }
            Some((Some(total_rate), split_known)) => {
                if !split_known {
                    unattributed += i128::from(total_rate) * hours;
                    if total_rate != 0 {
                        split_incomplete = true;
                    }
                    continue;
                }
                let mut missing_part = false;
                let mut part_rates: Vec<(Option<u64>, i64)> = Vec::new();
                for (&design, pieces) in &repairs.parts {
                    match piece_at(pieces, seg_first).and_then(|piece| piece.2) {
                        None => {}
                        Some(None) => {
                            missing_part = true;
                            break;
                        }
                        Some(Some(rate)) => {
                            if rate != 0 {
                                part_rates.push((design, rate));
                            }
                        }
                    }
                }
                if missing_part {
                    unattributed += i128::from(total_rate) * hours;
                    if total_rate != 0 {
                        split_incomplete = true;
                    }
                } else {
                    for (design, rate) in part_rates {
                        let added = i128::from(rate) * hours;
                        match design {
                            Some(variant) => *designs.entry(variant).or_default() += added,
                            None => unattributed += added,
                        }
                    }
                }
            }
        }
    }
    RepairSummary { recorded: true, designs, unattributed, gap, split_incomplete }
}

fn tighten_repairs(through: &mut i32, repairs: &RepairRead, hour: i32) {
    let total = repairs.total();
    if let Some(next_hour) = hour.checked_add(1)
        && let Some(piece) = piece_at(&total, next_hour)
        && matches!(piece.2, RunAmount::Known(rate) if rate != 0)
    {
        *through = (*through).min(hour);
        return;
    }
    tighten(through, next_piece_edge(&total, hour));
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

impl SpendingIndex {
    pub(crate) fn at(&self, hour: i32, scope: ProductionSpendingScope, include_refits: bool) -> ProductionSpendingReply {
        if let Some((_, last, reason)) = covering(&self.unknown, hour, |&(first, last, _)| (first, last)) {
            return ProductionSpendingReply {
                from: self.from,
                hour,
                through: (*last).min(self.to),
                breakdown: not_recorded(format!("Who holds {} is not known: {reason}", self.country_label)),
                warnings: self.warnings.clone(),
            };
        }
        if let Some((_, last)) = covering(&self.unresolved, hour, |&(first, last)| (first, last)) {
            return ProductionSpendingReply {
                from: self.from,
                hour,
                through: (*last).min(self.to),
                breakdown: not_recorded(format!("Who holds {} is not known: {TAG_UNLINKED}", self.country_label)),
                warnings: self.warnings.clone(),
            };
        }

        let Some(span_at) = self.spans.iter().position(|span| span.read.span.first <= hour && hour <= span.read.span.last) else {
            let mut through = self.to;
            tighten(&mut through, next_span_start(&self.spans, hour, |span| span.read.span.first));
            tighten(&mut through, next_span_start(&self.unknown, hour, |&(first, _, _)| first));
            tighten(&mut through, next_span_start(&self.unresolved, hour, |&(first, _)| first));
            return ProductionSpendingReply {
                from: self.from,
                hour,
                through: through.max(hour),
                breakdown: breakdown::breakdown(&[], &Unrecorded::default()),
                warnings: self.warnings.clone(),
            };
        };

        let span = &self.spans[span_at];
        let mut through = span.read.span.last.min(self.to);
        tighten(&mut through, next_span_start(&self.unknown, hour, |&(first, _, _)| first));
        tighten(&mut through, next_span_start(&self.unresolved, hour, |&(first, _)| first));
        tighten(&mut through, span.next_name_edge(hour, scope));

        if self.spans[..span_at].iter().any(|prev| prev.read.span.id != span.read.span.id) {
            return ProductionSpendingReply {
                from: self.from,
                hour,
                through: through.max(hour),
                breakdown: not_recorded(unknown_reason(ProductionSpendingUnknown::CountryChanged)),
                warnings: self.warnings.clone(),
            };
        }

        let mut warnings = self.warnings.clone();
        warnings.extend(span.notes.iter().cloned());

        let equipment_pieces = span.read.equipment(include_refits || matches!(scope, ProductionSpendingScope::DockyardPurposes | ProductionSpendingScope::AirPurposes | ProductionSpendingScope::LandPurposes));
        tighten(&mut through, next_piece_edge(&equipment_pieces, hour));
        let equipment_at = piece_at(&equipment_pieces, hour).map_or(RunAmount::Unknown(ProductionSpendingUnknown::NotRecorded), |piece| piece.2);

        let mut leaves: Vec<Leaf> = Vec::new();
        let mut reasons: Vec<String> = Vec::new();

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
                    if !rocket && span.domain(variant) == Some(Domain::Naval) {
                        continue;
                    }
                    tighten(&mut through, next_piece_edge(row_pieces, hour));
                }
                match (equipment_at, rocket_at) {
                    (RunAmount::Unknown(eq_why), RunAmount::Unknown(_)) => {
                        return ProductionSpendingReply {
                            from: self.from,
                            hour,
                            through: through.max(hour),
                            breakdown: not_recorded(unknown_reason(eq_why)),
                            warnings,
                        };
                    }
                    (RunAmount::Unknown(eq_why), RunAmount::Known(0)) => {
                        return ProductionSpendingReply {
                            from: self.from,
                            hour,
                            through: through.max(hour),
                            breakdown: not_recorded(unknown_reason(eq_why)),
                            warnings,
                        };
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
                        if rocket || span.domain(variant) == Some(Domain::Naval) {
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
                        let domain = span.domain(variant);
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
                        leaves.push(Leaf { path: vec![source, span.type_node(variant), span.design_node(variant, false, hour)], amount, count: None, last: None });
                    }
                    if unresolved_nonzero {
                        reasons.push("Some designs\u{2019} equipment types are unknown".into());
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
                                path: vec![rocket_node(self.rocket_source.as_ref()), span.type_node(variant), span.design_node(variant, true, hour)],
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
                    if matches!(span.domain(variant), Some(Domain::Land | Domain::Air)) {
                        continue;
                    }
                    tighten(&mut through, next_piece_edge(row_pieces, hour));
                }
                if let RunAmount::Unknown(why) = equipment_at {
                    return ProductionSpendingReply {
                        from: self.from,
                        hour,
                        through: through.max(hour),
                        breakdown: not_recorded(unknown_reason(why)),
                        warnings,
                    };
                }
                let mut by_variant: BTreeMap<u64, (i128, Option<ProductionSpendingUnknown>)> = BTreeMap::new();
                for key in span.read.rows.keys() {
                    let (rocket, variant, purpose) = *key;
                    if rocket || matches!(span.domain(variant), Some(Domain::Land | Domain::Air)) {
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
                    let domain = span.domain(variant);
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
                    leaves.push(Leaf { path: vec![source, span.type_node(variant), span.design_node(variant, false, hour)], amount, count: None, last: None });
                }
                if unresolved_nonzero {
                    reasons.push("Some designs\u{2019} equipment types are unknown".into());
                }
            }
            ProductionSpendingScope::DockyardPurposes => {
                for (key, row_pieces) in &span.read.rows {
                    let (rocket, variant, _) = *key;
                    if rocket || matches!(span.domain(variant), Some(Domain::Land | Domain::Air)) {
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
                    let mut unresolved_purposes: BTreeSet<ProductionPurpose> = BTreeSet::new();
                    for key in span.read.rows.keys() {
                        let (rocket, variant, purpose) = *key;
                        if rocket {
                            continue;
                        }
                        let domain = span.domain(variant);
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
                        if domain.is_none() {
                            if !matches!(spent_at, RunAmount::Known(0)) {
                                unresolved_purposes.insert(purpose);
                            }
                            continue;
                        }
                        let amount = match spent_at {
                            RunAmount::Known(0) => continue,
                            RunAmount::Known(value) => exact_spent(value),
                            RunAmount::Unknown(why) => Amount::NotRecorded(unknown_reason(why).into()),
                        };
                        leaves.push(Leaf {
                            path: vec![purpose_node(purpose, true), span.type_node(variant), span.design_node(variant, false, hour)],
                            amount,
                            count: None,
                            last: None,
                        });
                    }
                    for purpose in unresolved_purposes {
                        leaves.push(Leaf {
                            path: vec![purpose_node(purpose, true), Node::new(UNRESOLVED_TYPE_KEY, "Type not resolved", u32::MAX).other()],
                            amount: Amount::NotRecorded("Some designs\u{2019} equipment types are unknown".into()),
                            count: None,
                            last: None,
                        });
                    }
                    if unproven_purpose {
                        reasons.push("Whether some of it was refits was not recorded".into());
                    }
                }
                let repair_root = Node::new("repair", "Repairs", 2);
                match span.read.repairs.as_ref().map(|repairs| repair_summary(repairs, hour)) {
                    None | Some(RepairSummary { recorded: false, .. }) => {
                        leaves.push(Leaf { path: vec![repair_root], amount: Amount::NotRecorded("Spending was not recorded".into()), count: None, last: None });
                    }
                    Some(summary) => {
                        let mut any_repair = false;
                        for (variant, sum) in summary.designs {
                            if sum == 0 {
                                continue;
                            }
                            any_repair = true;
                            leaves.push(Leaf {
                                path: vec![repair_root.clone(), span.type_node(variant), span.design_node(variant, false, hour)],
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
                        if let Some(gap_why) = summary.gap {
                            if any_repair {
                                leaves.push(Leaf {
                                    path: vec![repair_root, Node::new("~unrecorded", "Unrecorded hours", u32::MAX).other()],
                                    amount: Amount::NotRecorded(unknown_reason(gap_why).into()),
                                    count: None,
                                    last: None,
                                });
                            } else {
                                leaves.push(Leaf { path: vec![repair_root], amount: Amount::NotRecorded(unknown_reason(gap_why).into()), count: None, last: None });
                            }
                        }
                        if summary.split_incomplete {
                            warnings.push("Some dockyard repair spending\u{2019}s split by design is not known.".into());
                        }
                    }
                }
            }
            ProductionSpendingScope::AirPurposes | ProductionSpendingScope::LandPurposes => {
                let target = if scope == ProductionSpendingScope::AirPurposes { Domain::Air } else { Domain::Land };
                for (key, row_pieces) in &span.read.rows {
                    let (rocket, variant, _) = *key;
                    if rocket || (span.domain(variant).is_some() && span.domain(variant) != Some(target)) {
                        continue;
                    }
                    tighten(&mut through, next_piece_edge(row_pieces, hour));
                }
                if let RunAmount::Unknown(why) = equipment_at {
                    return ProductionSpendingReply {
                        from: self.from,
                        hour,
                        through: through.max(hour),
                        breakdown: not_recorded(unknown_reason(why)),
                        warnings,
                    };
                }
                let mut unproven_purpose = false;
                let mut unresolved_purposes: BTreeSet<ProductionPurpose> = BTreeSet::new();
                for key in span.read.rows.keys() {
                    let (rocket, variant, purpose) = *key;
                    if rocket {
                        continue;
                    }
                    let domain = span.domain(variant);
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
                    if domain.is_none() {
                        if !matches!(spent_at, RunAmount::Known(0)) {
                            unresolved_purposes.insert(purpose);
                        }
                        continue;
                    }
                    let amount = match spent_at {
                        RunAmount::Known(0) => continue,
                        RunAmount::Known(value) => exact_spent(value),
                        RunAmount::Unknown(why) => Amount::NotRecorded(unknown_reason(why).into()),
                    };
                    leaves.push(Leaf {
                        path: vec![purpose_node(purpose, false), span.type_node(variant), span.design_node(variant, false, hour)],
                        amount,
                        count: None,
                        last: None,
                    });
                }
                for purpose in unresolved_purposes {
                    leaves.push(Leaf {
                        path: vec![purpose_node(purpose, false), Node::new(UNRESOLVED_TYPE_KEY, "Type not resolved", u32::MAX).other()],
                        amount: Amount::NotRecorded("Some designs\u{2019} equipment types are unknown".into()),
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
        let unrecorded = Unrecorded { hours: 0, reason: (!reasons.is_empty()).then(|| reasons.join("; ")) };
        warnings.dedup();
        ProductionSpendingReply {
            from: self.from,
            hour,
            through: through.max(hour),
            breakdown: breakdown::breakdown(&leaves, &unrecorded),
            warnings,
        }
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
    let reply = index.at(params.hour, params.scope, params.include_refits);
    check_reply_budget(&reply)?;
    Ok(reply)
}
