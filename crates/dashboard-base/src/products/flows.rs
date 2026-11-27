use dashboard_query::identity::{Country, Holders};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FlowPart {
    pub key: String,
    pub name: Option<String>,
    pub name_missing: Option<String>,
    #[doc = " The part's recorded id, for a colour that stays with it from hour to hour."]
    pub ordinal: u32,
    pub icon: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FlowAmount {
    #[doc = " Index into `parts`."]
    pub part: u32,
    #[doc = " A fixed-point decimal string at the reply's `decimals`; null where not recorded."]
    pub amount: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FlowEndpoint {
    #[doc = " The partner's recorded identity: a slot, a reference the recording cannot link, or the unidentified partners the record combines."]
    pub key: String,
    #[doc = " The recorded country slot; null where the record names no country the recording can link."]
    pub slot: Option<u32>,
    #[doc = " The tag the slot holds at the hour."]
    pub tag: Option<String>,
    #[doc = " The amount combines unidentified partners; how many is not recorded."]
    pub collective: bool,
    #[doc = " Why the endpoint cannot be named at the hour."]
    pub unknown: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FlowEnd {
    Slot(u32),
    Raw(u32),
    Collective,
}

impl FlowEnd {
    pub fn slot(self) -> Option<u32> {
        match self {
            Self::Slot(slot) => Some(slot),
            Self::Raw(_) | Self::Collective => None,
        }
    }

    fn key(self) -> String {
        match self {
            Self::Slot(slot) => format!("s{slot}"),
            Self::Raw(raw) => format!("r{raw}"),
            Self::Collective => "collective".to_owned(),
        }
    }

    fn ranks(self) -> bool {
        self != Self::Collective
    }
}

impl From<u32> for FlowEnd {
    fn from(slot: u32) -> Self {
        Self::Slot(slot)
    }
}

impl FlowEndpoint {
    pub fn at(holders: &Holders, end: FlowEnd, hour: i32, unlinked: &str) -> Self {
        let Some(slot) = end.slot() else { return Self::unidentified(end, unlinked.to_owned()) };
        let country = Country::Slot(slot);
        let tag = holders.tag_at(&country, hour).map(str::to_owned);
        let unknown = tag.is_none().then(|| holders.unnamed(&country, hour));
        Self { key: end.key(), slot: Some(slot), tag, collective: false, unknown }
    }

    pub fn unidentified(end: FlowEnd, reason: String) -> Self {
        Self { key: end.key(), slot: end.slot(), tag: None, collective: end == FlowEnd::Collective, unknown: Some(reason) }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FlowLink {
    #[doc = " Indexes into `endpoints`; null on the side folded into the others."]
    pub from: Option<u32>,
    pub to: Option<u32>,
    #[doc = " How many partners the others hold, where this link folds them."]
    pub others: Option<u32>,
    pub amounts: Vec<FlowAmount>,
    pub total: Option<String>,
    #[doc = " Why the total is not known, where an amount was not recorded."]
    pub missing: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FlowSide {
    #[doc = " Known amounts largest first, ties by slot, the rest of them folded into one link; then every link whose amount or partner is unknown."]
    pub links: Vec<FlowLink>,
    pub total: Option<String>,
    #[doc = " Why the side's total, order or emptiness is not proven at the hour."]
    pub missing: Option<String>,
    #[doc = " Why the asked top partners are not applied: their order is not proven at the hour, so every witnessed link is kept."]
    pub unranked: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FlowTableRow {
    #[doc = " Index into `endpoints`."]
    pub partner: u32,
    #[doc = " One signed amount per part (in to the subject positive, out negative, zero where none); null where not recorded."]
    pub cells: Vec<Option<String>>,
    pub net: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CountryFlowReply {
    pub endpoints: Vec<FlowEndpoint>,
    #[doc = " The centre endpoint, when the subject is one country."]
    pub subject: Option<u32>,
    pub decimals: u8,
    pub parts: Vec<FlowPart>,
    pub incoming: FlowSide,
    pub outgoing: FlowSide,
    #[doc = " A group subject: every link between its senders and receivers."]
    pub links: Vec<FlowLink>,
    #[doc = " A group subject: each sender's total (`from`) and each receiver's (`to`)."]
    pub senders: FlowSide,
    pub receivers: FlowSide,
    #[doc = " Partners by part, signed, largest net first; only when asked."]
    pub table: Vec<FlowTableRow>,
    #[doc = " The largest known side or node amount, for the scale."]
    pub largest: String,
    #[doc = " The largest single link, for a colour scale."]
    pub largest_link: String,
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

crate::reply_warnings!(CountryFlowReply);

impl CountryFlowReply {
    pub fn not_recorded(reason: String, decimals: u8, warnings: Vec<String>) -> Self {
        Self { decimals, largest: "0".to_owned(), largest_link: "0".to_owned(), missing: Some(reason), warnings, ..Self::default() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlowEdge {
    pub from: FlowEnd,
    pub to: FlowEnd,
    pub part: u32,
    pub amount: Option<i128>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GapEnd {
    Any,
    Slot(u32),
    Unidentified,
}

impl GapEnd {
    fn covers(self, key: Key) -> bool {
        match self {
            Self::Any => true,
            Self::Slot(slot) => key == FlowEnd::Slot(slot),
            Self::Unidentified => key.slot().is_none(),
        }
    }

    fn known(self) -> Option<u32> {
        match self {
            Self::Slot(slot) => Some(slot),
            Self::Any | Self::Unidentified => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlowGap {
    pub from: GapEnd,
    pub to: GapEnd,
    pub part: Option<u32>,
    pub reason: String,
    pub partial: bool,
}

impl FlowGap {
    pub fn everything(reason: String) -> Self {
        Self { from: GapEnd::Any, to: GapEnd::Any, part: None, reason, partial: false }
    }

    pub fn partial(reason: String) -> Self {
        Self { partial: true, ..Self::everything(reason) }
    }

    fn covers(&self, from: Key, to: Key) -> bool {
        self.from.covers(from) && self.to.covers(to)
    }
}

pub fn validate_chosen(subject: Option<&Country>, countries: &[Country], group: bool) -> Result<(), &'static str> {
    match (subject.is_some(), !countries.is_empty(), group) {
        (true, true, _) | (true, _, true) => Err("Ask one country, or a group or countries, never both"),
        (_, true, true) => Err("Ask a group or countries, never both"),
        _ => Ok(()),
    }
}

pub struct FlowShape<'a> {
    pub subject: Option<u32>,
    pub chosen: Option<&'a HashSet<u32>>,
    pub between: bool,
    pub limit: Option<usize>,
    pub top: Option<usize>,
    pub side_totals_only: bool,
    pub table: bool,
    pub not_recorded: &'a str,
    pub unlinked: &'a str,
}

type Key = FlowEnd;
type Sums = Vec<Option<i128>>;

struct Grouped {
    sums: Sums,
    present: Vec<bool>,
    known: i128,
    open: bool,
    missing: Option<String>,
}

impl Grouped {
    fn new(parts: usize) -> Self {
        Self { sums: vec![Some(0); parts], present: vec![false; parts], known: 0, open: false, missing: None }
    }

    fn add(&mut self, part: u32, amount: Option<i128>, not_recorded: &str) {
        let slot = &mut self.sums[part as usize];
        *slot = slot.zip(amount).map(|(a, b)| a + b);
        self.present[part as usize] = true;
        self.known += amount.unwrap_or(0);
        if amount.is_none() {
            self.missing.get_or_insert_with(|| not_recorded.to_owned());
        }
    }

    fn withhold(&mut self, gap: &FlowGap) {
        if gap.partial {
            for (part, (sum, here)) in self.sums.iter_mut().zip(&self.present).enumerate() {
                if *here && gap.part.is_none_or(|only| only as usize == part) {
                    *sum = None;
                }
            }
            if gap.part.is_none() {
                self.open = true;
            }
            self.missing.get_or_insert_with(|| gap.reason.clone());
            return;
        }
        match gap.part {
            Some(part) if self.present[part as usize] => return,
            Some(part) => {
                self.sums[part as usize] = None;
                self.present[part as usize] = true;
            }
            None => self.open = true,
        }
        self.missing.get_or_insert_with(|| gap.reason.clone());
    }

    fn total(&self) -> Option<i128> {
        if self.open {
            return None;
        }
        self.sums.iter().zip(&self.present).filter(|(_, here)| **here).try_fold(0i128, |held, (sum, _)| sum.map(|sum| held + sum))
    }

    fn absorb(&mut self, other: &Grouped) {
        for (part, here) in other.present.iter().enumerate() {
            if *here {
                let slot = &mut self.sums[part];
                *slot = slot.zip(other.sums[part]).map(|(a, b)| a + b);
                self.present[part] = true;
            }
        }
        self.known += other.known;
        self.open |= other.open;
        if self.missing.is_none() {
            self.missing.clone_from(&other.missing);
        }
    }
}

fn withhold(by: &mut BTreeMap<Key, Grouped>, gaps: &[FlowGap], covers: impl Fn(&FlowGap, Key) -> bool) {
    for (key, grouped) in by.iter_mut() {
        for gap in gaps.iter().filter(|gap| covers(gap, *key)) {
            grouped.withhold(gap);
        }
    }
}

struct Endpoints<'f> {
    list: Vec<FlowEndpoint>,
    index: HashMap<Key, u32>,
    describe: &'f dyn Fn(Key) -> FlowEndpoint,
}

impl Endpoints<'_> {
    fn of(&mut self, key: Key) -> u32 {
        if let Some(&at) = self.index.get(&key) {
            return at;
        }
        let at = self.list.len() as u32;
        self.list.push((self.describe)(key));
        self.index.insert(key, at);
        at
    }
}

fn link(from: Option<u32>, to: Option<u32>, others: Option<u32>, grouped: &Grouped, not_recorded: &str) -> FlowLink {
    let amounts = grouped
        .present
        .iter()
        .enumerate()
        .filter(|(_, here)| **here)
        .map(|(part, _)| FlowAmount { part: part as u32, amount: grouped.sums[part].map(|sum| sum.to_string()) })
        .collect();
    let total = grouped.total();
    FlowLink { from, to, others, amounts, total: total.map(|sum| sum.to_string()), missing: total.is_none().then(|| grouped.missing.clone().unwrap_or_else(|| not_recorded.to_owned())) }
}

struct Ranked {
    side: FlowSide,
    kept: HashSet<Key>,
    known: i128,
}

fn rank(by: BTreeMap<Key, Grouped>, parts: usize, shape: &FlowShape<'_>, missing: Option<&str>, names: &mut Endpoints<'_>, place: impl Fn(Option<u32>) -> (Option<u32>, Option<u32>)) -> Ranked {
    let mut whole = Grouped::new(parts);
    for grouped in by.values() {
        whole.absorb(grouped);
    }
    let (mut known, mut held): (Vec<(Key, Grouped)>, Vec<(Key, Grouped)>) = by.into_iter().partition(|(key, grouped)| key.ranks() && grouped.total().is_some());
    known.sort_by(|a, b| b.1.total().cmp(&a.1.total()).then(a.0.cmp(&b.0)));
    held.sort_by_key(|(key, _)| *key);
    let unproven = missing.map(str::to_owned).or_else(|| held.first().map(|(_, grouped)| grouped.missing.clone().unwrap_or_else(|| shape.unlinked.to_owned())));
    let unproven = unproven.or_else(|| whole.total().is_none().then(|| whole.missing.clone().unwrap_or_else(|| shape.not_recorded.to_owned())));
    let shown = shape.limit.map_or(known.len(), |limit| if known.len() > limit + 1 { limit } else { known.len() });
    let keep = match (shape.top, &unproven) {
        (Some(top), None) => top.min(known.len()),
        _ => shown,
    };
    let unranked = shape.top.and(unproven);
    let mut links = Vec::with_capacity(keep + held.len() + 1);
    let mut kept = HashSet::new();
    let mut rest = Grouped::new(parts);
    let mut folded = 0u32;
    for (at, (key, grouped)) in known.into_iter().enumerate() {
        if at < keep {
            let (from, to) = place(Some(names.of(key)));
            links.push(link(from, to, None, &grouped, shape.not_recorded));
            kept.insert(key);
        } else {
            rest.absorb(&grouped);
            folded += 1;
        }
    }
    if folded > 0 {
        let (from, to) = place(None);
        links.push(link(from, to, Some(folded), &rest, shape.not_recorded));
    }
    for (key, grouped) in held {
        let (from, to) = place(Some(names.of(key)));
        links.push(link(from, to, None, &grouped, shape.not_recorded));
        kept.insert(key);
    }
    let total = whole.total();
    let missing = missing.map(str::to_owned).or_else(|| total.is_none().then(|| whole.missing.clone().unwrap_or_else(|| shape.not_recorded.to_owned())));
    let total = if missing.is_some() { None } else { total.map(|sum| sum.to_string()) };
    Ranked { side: FlowSide { links, total, missing, unranked }, kept, known: whole.known }
}

enum Belongs {
    Yes,
    No,
    Maybe,
}

fn belongs(edge: &FlowEdge, chosen: Option<&HashSet<u32>>, between: bool) -> Belongs {
    let Some(chosen) = chosen else { return Belongs::Yes };
    let ends = [edge.from.slot(), edge.to.slot()];
    let unknown = ends.iter().any(Option::is_none);
    let mut known = ends.iter().flatten();
    let decided = if between { known.any(|slot| !chosen.contains(slot)) } else { known.any(|slot| chosen.contains(slot)) };
    match (decided, between, unknown) {
        (true, true, _) => Belongs::No,
        (true, false, _) => Belongs::Yes,
        (false, _, true) => Belongs::Maybe,
        (false, true, false) => Belongs::Yes,
        (false, false, false) => Belongs::No,
    }
}

fn table(incoming: &BTreeMap<Key, Grouped>, outgoing: &BTreeMap<Key, Grouped>, count: usize, names: &mut Endpoints<'_>) -> Vec<FlowTableRow> {
    let open = |key: Key| [incoming, outgoing].iter().any(|held| held.get(&key).is_some_and(|grouped| grouped.open));
    let partners: BTreeSet<Key> = incoming.keys().chain(outgoing.keys()).copied().collect();
    let side = |held: &BTreeMap<Key, Grouped>, key: Key, part: usize| -> Option<Option<i128>> { held.get(&key).filter(|grouped| grouped.present[part]).map(|grouped| grouped.sums[part]) };
    let mut rows: Vec<(Key, Sums, Option<i128>)> = partners
        .into_iter()
        .map(|key| {
            let cells: Sums = (0..count)
                .map(|part| match (side(incoming, key, part), side(outgoing, key, part)) {
                    (Some(None), _) | (_, Some(None)) => None,
                    (inbound, outbound) => Some(inbound.flatten().unwrap_or(0) - outbound.flatten().unwrap_or(0)),
                })
                .collect();
            let net = if open(key) { None } else { cells.iter().try_fold(0i128, |held, cell| cell.map(|cell| held + cell)) };
            (key, cells, net)
        })
        .collect();
    rows.sort_by(|a, b| match (a.2.filter(|_| a.0.ranks()), b.2.filter(|_| b.0.ranks())) {
        (Some(x), Some(y)) => y.abs().cmp(&x.abs()).then(a.0.cmp(&b.0)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.0.cmp(&b.0),
    });
    rows.into_iter()
        .map(|(key, cells, net)| FlowTableRow { partner: names.of(key), cells: cells.iter().map(|cell| cell.map(|value| value.to_string())).collect(), net: net.map(|value| value.to_string()) })
        .collect()
}

pub fn build_flows(edges: &[FlowEdge], gaps: &[FlowGap], parts: Vec<FlowPart>, decimals: u8, shape: &FlowShape<'_>, describe: &dyn Fn(Key) -> FlowEndpoint) -> CountryFlowReply {
    let count = parts.len();
    let mut names = Endpoints { list: Vec::new(), index: HashMap::new(), describe };
    let mut reply = CountryFlowReply { decimals, largest: "0".to_owned(), largest_link: "0".to_owned(), ..CountryFlowReply::default() };
    if let Some(subject) = shape.subject {
        let at = names.of(FlowEnd::Slot(subject));
        reply.subject = Some(at);
        let mut incoming: BTreeMap<Key, Grouped> = BTreeMap::new();
        let mut outgoing: BTreeMap<Key, Grouped> = BTreeMap::new();
        let (mut in_maybe, mut out_maybe) = (false, false);
        for edge in edges {
            match (edge.from == FlowEnd::Slot(subject), edge.to == FlowEnd::Slot(subject)) {
                (true, false) => outgoing.entry(edge.to).or_insert_with(|| Grouped::new(count)).add(edge.part, edge.amount, shape.not_recorded),
                (false, true) => incoming.entry(edge.from).or_insert_with(|| Grouped::new(count)).add(edge.part, edge.amount, shape.not_recorded),
                (true, true) => {}
                (false, false) => {
                    in_maybe |= edge.to.slot().is_none();
                    out_maybe |= edge.from.slot().is_none();
                }
            }
        }
        let into = |gap: &&FlowGap| gap.to.covers(FlowEnd::Slot(subject)) && gap.from != GapEnd::Slot(subject);
        let out_of = |gap: &&FlowGap| gap.from.covers(FlowEnd::Slot(subject)) && gap.to != GapEnd::Slot(subject);
        withhold(&mut incoming, gaps, |gap, partner| into(&gap) && gap.from.covers(partner));
        withhold(&mut outgoing, gaps, |gap, partner| out_of(&gap) && gap.to.covers(partner));
        if shape.table && !shape.side_totals_only {
            reply.table = table(&incoming, &outgoing, count, &mut names);
        }
        let in_missing = gaps.iter().find(into).map(|gap| gap.reason.as_str()).or(in_maybe.then_some(shape.unlinked));
        let out_missing = gaps.iter().find(out_of).map(|gap| gap.reason.as_str()).or(out_maybe.then_some(shape.unlinked));
        let inbound = rank(incoming, count, shape, in_missing, &mut names, |partner| (partner, Some(at)));
        let outbound = rank(outgoing, count, shape, out_missing, &mut names, |partner| (Some(at), partner));
        reply.largest = inbound.known.max(outbound.known).to_string();
        reply.incoming = inbound.side;
        reply.outgoing = outbound.side;
    } else {
        let mut pairs: BTreeMap<(Key, Key), Grouped> = BTreeMap::new();
        let mut sent: BTreeMap<Key, Grouped> = BTreeMap::new();
        let mut received: BTreeMap<Key, Grouped> = BTreeMap::new();
        let mut maybe = false;
        for edge in edges.iter().filter(|edge| edge.from.slot().is_none() || edge.from != edge.to) {
            match belongs(edge, shape.chosen, shape.between) {
                Belongs::No => continue,
                Belongs::Maybe => {
                    maybe = true;
                    continue;
                }
                Belongs::Yes => {}
            }
            if !shape.side_totals_only {
                pairs.entry((edge.from, edge.to)).or_insert_with(|| Grouped::new(count)).add(edge.part, edge.amount, shape.not_recorded);
            }
            sent.entry(edge.from).or_insert_with(|| Grouped::new(count)).add(edge.part, edge.amount, shape.not_recorded);
            received.entry(edge.to).or_insert_with(|| Grouped::new(count)).add(edge.part, edge.amount, shape.not_recorded);
        }
        let reaching: Vec<&FlowGap> = gaps
            .iter()
            .filter(|gap| {
                let end = |at: GapEnd| at.known().map_or(FlowEnd::Collective, FlowEnd::Slot);
                !matches!(belongs(&FlowEdge { from: end(gap.from), to: end(gap.to), part: 0, amount: None }, shape.chosen, shape.between), Belongs::No)
            })
            .collect();
        for ((from, to), grouped) in pairs.iter_mut() {
            for gap in reaching.iter().filter(|gap| gap.covers(*from, *to)) {
                grouped.withhold(gap);
            }
        }
        for (node, grouped) in sent.iter_mut() {
            reaching.iter().filter(|gap| gap.from.covers(*node)).for_each(|gap| grouped.withhold(gap));
        }
        for (node, grouped) in received.iter_mut() {
            reaching.iter().filter(|gap| gap.to.covers(*node)).for_each(|gap| grouped.withhold(gap));
        }
        let largest = sent.values().chain(received.values()).map(|grouped| grouped.known).max().unwrap_or(0);
        let missing = reaching.first().map(|gap| gap.reason.as_str()).or(maybe.then_some(shape.unlinked));
        let senders = rank(sent, count, shape, missing, &mut names, |node| (node, None));
        let receivers = rank(received, count, shape, missing, &mut names, |node| (None, node));
        let mut folded: BTreeMap<(Option<u32>, Option<u32>), (Grouped, HashSet<(bool, Key)>)> = BTreeMap::new();
        for ((from, to), grouped) in &pairs {
            let a = senders.kept.contains(from).then(|| names.of(*from));
            let b = receivers.kept.contains(to).then(|| names.of(*to));
            if a.is_some() && b.is_some() {
                reply.links.push(link(a, b, None, grouped, shape.not_recorded));
                continue;
            }
            let held = folded.entry((a, b)).or_insert_with(|| (Grouped::new(count), HashSet::new()));
            held.0.absorb(grouped);
            if a.is_none() {
                held.1.insert((false, *from));
            }
            if b.is_none() {
                held.1.insert((true, *to));
            }
        }
        for ((a, b), (grouped, partners)) in folded {
            reply.links.push(link(a, b, Some(partners.len() as u32), &grouped, shape.not_recorded));
        }
        reply.senders = senders.side;
        reply.receivers = receivers.side;
        reply.largest = largest.to_string();
    }
    let sides = reply.incoming.links.iter().chain(&reply.outgoing.links);
    let biggest = sides.chain(&reply.links).filter(|link| link.others.is_none()).filter_map(|link| link.total.as_deref()?.parse::<i128>().ok()).max().unwrap_or(0);
    reply.largest_link = biggest.to_string();
    reply.endpoints = names.list;
    reply.parts = parts;
    reply
}

#[cfg(test)]
mod tests {
    use super::*;

    const USA: u32 = 1;
    const HOL: u32 = 2;
    const RAJ: u32 = 3;
    const TUR: u32 = 4;
    const IRN: u32 = 5;
    const ENG: u32 = 6;
    const JAP: u32 = 7;
    const SOV: u32 = 8;
    const SPR: u32 = 9;
    const ITA: u32 = 10;
    const SPD: u32 = 11;
    const GER: u32 = 12;
    const POR: u32 = 13;
    const UNKNOWN: Key = FlowEnd::Collective;
    type Slot = Option<u32>;

    fn part(key: &str, ordinal: u32) -> FlowPart {
        FlowPart { key: key.to_owned(), name: Some(key.to_owned()), name_missing: None, ordinal, icon: None }
    }

    fn edge(from: impl Into<Key>, to: impl Into<Key>, part: u32, amount: Option<i128>) -> FlowEdge {
        FlowEdge { from: from.into(), to: to.into(), part, amount }
    }

    fn describe(key: Key) -> FlowEndpoint {
        match key.slot() {
            Some(slot) => FlowEndpoint { key: key.key(), slot: Some(slot), tag: Some(format!("S{slot}")), collective: false, unknown: None },
            None => FlowEndpoint::unidentified(key, "unlinked".to_owned()),
        }
    }

    fn shape(subject: Option<u32>, limit: Option<usize>, table: bool) -> FlowShape<'static> {
        FlowShape { subject, chosen: None, between: false, limit, top: None, side_totals_only: false, table, not_recorded: "not recorded", unlinked: "unlinked" }
    }

    fn build(edges: &[FlowEdge], parts: usize, shape: &FlowShape<'_>) -> CountryFlowReply {
        build_gapped(edges, &[], parts, shape)
    }

    fn build_gapped(edges: &[FlowEdge], gaps: &[FlowGap], parts: usize, shape: &FlowShape<'_>) -> CountryFlowReply {
        build_flows(edges, gaps, (0..parts as u32).map(|at| part(&format!("p{at}"), at)).collect(), 0, shape, &describe)
    }

    fn slot(reply: &CountryFlowReply, at: Option<u32>) -> Option<Slot> {
        at.map(|at| reply.endpoints[at as usize].slot)
    }

    fn rows(reply: &CountryFlowReply, side: &FlowSide, from: bool) -> Vec<(Option<Slot>, Option<String>, Option<u32>)> {
        side.links.iter().map(|link| (slot(reply, if from { link.from } else { link.to }), link.total.clone(), link.others)).collect()
    }

    #[test]
    fn a_request_names_one_country_or_a_group_or_countries_never_two_of_them() {
        let one = Country::Tag("GER".into());
        assert!(validate_chosen(Some(&one), &[], true).is_err(), "a centre outside the group would silently drop the group");
        assert!(validate_chosen(Some(&one), std::slice::from_ref(&one), false).is_err());
        assert!(validate_chosen(None, std::slice::from_ref(&one), true).is_err());
        assert!(validate_chosen(Some(&one), &[], false).is_ok());
        assert!(validate_chosen(None, &[], true).is_ok());
    }

    #[test]
    fn a_country_sees_its_partners_both_ways_largest_first_and_the_rest_folded() {
        let edges = [
            edge(HOL, USA, 0, Some(26)),
            edge(RAJ, USA, 1, Some(24)),
            edge(TUR, USA, 0, Some(12)),
            edge(IRN, USA, 0, Some(9)),
            edge(USA, ENG, 0, Some(40)),
            edge(USA, ENG, 1, Some(34)),
            edge(USA, JAP, 0, Some(18)),
        ];
        let reply = build(&edges, 2, &shape(Some(USA), Some(2), false));
        assert_eq!(rows(&reply, &reply.incoming, true), [(Some(Some(HOL)), Some("26".to_owned()), None), (Some(Some(RAJ)), Some("24".to_owned()), None), (None, Some("21".to_owned()), Some(2))]);
        assert_eq!(rows(&reply, &reply.outgoing, false), [(Some(Some(ENG)), Some("74".to_owned()), None), (Some(Some(JAP)), Some("18".to_owned()), None)]);
        assert_eq!((reply.incoming.total.as_deref(), reply.incoming.missing.as_deref()), (Some("71"), None));
        assert_eq!(reply.outgoing.total.as_deref(), Some("92"));
        assert_eq!(reply.largest, "92");
    }

    #[test]
    fn a_partner_on_both_sides_is_one_endpoint_with_a_link_on_each_side() {
        let reply = build(&[edge(ENG, USA, 0, Some(5)), edge(USA, ENG, 0, Some(45))], 1, &shape(Some(USA), None, false));
        assert_eq!(reply.endpoints.iter().map(|endpoint| endpoint.slot).collect::<Vec<_>>(), [Some(USA), Some(ENG)]);
        assert_eq!(reply.incoming.links[0].from, reply.outgoing.links[0].to);
        assert_eq!((reply.incoming.links[0].total.as_deref(), reply.outgoing.links[0].total.as_deref()), (Some("5"), Some("45")));
    }

    #[test]
    fn an_unrecorded_amount_is_kept_after_the_ranked_links_never_folded_and_withholds_the_total() {
        let edges = [edge(HOL, USA, 0, Some(26)), edge(RAJ, USA, 0, None), edge(TUR, USA, 0, Some(12)), edge(IRN, USA, 0, Some(9))];
        let reply = build(&edges, 1, &shape(Some(USA), Some(1), false));
        assert_eq!(rows(&reply, &reply.incoming, true), [(Some(Some(HOL)), Some("26".to_owned()), None), (None, Some("21".to_owned()), Some(2)), (Some(Some(RAJ)), None, None)]);
        assert_eq!(reply.incoming.links[2].missing.as_deref(), Some("not recorded"));
        assert_eq!((reply.incoming.total.as_deref(), reply.incoming.missing.as_deref()), (None, Some("not recorded")));
        assert_eq!(reply.largest, "47");
    }

    #[test]
    fn a_licence_with_an_unknown_giver_stays_one_licence_from_an_unknown_endpoint() {
        let reply = build(&[edge(HOL, USA, 0, Some(1)), edge(UNKNOWN, USA, 0, Some(1))], 1, &shape(Some(USA), None, false));
        assert_eq!(rows(&reply, &reply.incoming, true), [(Some(Some(HOL)), Some("1".to_owned()), None), (Some(None), Some("1".to_owned()), None)]);
        let unknown = &reply.endpoints[reply.incoming.links[1].from.unwrap() as usize];
        assert_eq!((unknown.slot, unknown.unknown.as_deref()), (None, Some("unlinked")));
        assert_eq!((reply.incoming.total.as_deref(), reply.incoming.missing.as_deref()), (Some("2"), None));
    }

    #[test]
    fn a_record_that_may_belong_to_the_subject_leaves_its_side_unproven_never_empty() {
        let reply = build(&[edge(ENG, UNKNOWN, 0, Some(3))], 1, &shape(Some(USA), None, false));
        assert!(reply.incoming.links.is_empty());
        assert_eq!((reply.incoming.total.as_deref(), reply.incoming.missing.as_deref()), (None, Some("unlinked")));
        assert_eq!((reply.outgoing.total.as_deref(), reply.outgoing.missing.as_deref()), (Some("0"), None));
        let reply = build_gapped(&[edge(HOL, USA, 0, Some(4))], &[FlowGap::everything("not every sender was read".to_owned())], 1, &shape(Some(USA), None, false));
        assert_eq!(rows(&reply, &reply.incoming, true), [(Some(Some(HOL)), None, None)], "a witnessed link is kept; what the unread rows may add to it is not");
        assert_eq!(reply.incoming.links[0].amounts, [FlowAmount { part: 0, amount: Some("4".to_owned()) }]);
        assert_eq!(reply.incoming.links[0].missing.as_deref(), Some("not every sender was read"));
        assert_eq!((reply.incoming.total.as_deref(), reply.incoming.missing.as_deref()), (None, Some("not every sender was read")));
        assert_eq!(reply.outgoing.missing.as_deref(), Some("not every sender was read"));
    }

    fn keys<'a>(reply: &'a CountryFlowReply, side: &'a FlowSide, from: bool) -> Vec<(Option<&'a str>, Option<&'a str>, Option<u32>)> {
        side.links
            .iter()
            .map(|link| (if from { link.from } else { link.to }.map(|at| reply.endpoints[at as usize].key.as_str()), link.total.as_deref(), link.others))
            .collect()
    }

    #[test]
    fn distinct_unlinked_partners_stay_distinct_and_a_combined_amount_is_one_collective_endpoint_with_its_reason() {
        let edges = [edge(FlowEnd::Raw(50), USA, 0, Some(1)), edge(FlowEnd::Raw(60), USA, 0, Some(1)), edge(UNKNOWN, USA, 0, Some(5)), edge(HOL, USA, 0, Some(2))];
        let reply = build(&edges, 1, &shape(Some(USA), None, false));
        assert_eq!(keys(&reply, &reply.incoming, true), [(Some("s2"), Some("2"), None), (Some("r50"), Some("1"), None), (Some("r60"), Some("1"), None), (Some("collective"), Some("5"), None)]);
        let collective = &reply.endpoints[reply.incoming.links[3].from.unwrap() as usize];
        assert_eq!((collective.collective, collective.unknown.as_deref()), (true, Some("unlinked")), "a combined amount says it is combined and why");
        let raw = &reply.endpoints[reply.incoming.links[1].from.unwrap() as usize];
        assert_eq!((raw.collective, raw.slot, raw.unknown.as_deref()), (false, None, Some("unlinked")));
        assert_eq!(reply.incoming.total.as_deref(), Some("9"));
    }

    #[test]
    fn an_explicit_top_is_applied_only_where_the_order_is_proven_and_never_by_the_viewport_rule() {
        let complete = [edge(HOL, USA, 0, Some(26)), edge(TUR, USA, 0, Some(12)), edge(IRN, USA, 0, Some(9))];
        let topped = |edges: &[FlowEdge], top: usize| build(edges, 1, &FlowShape { top: Some(top), ..shape(Some(USA), Some(6), false) });
        let reply = topped(&complete, 2);
        assert_eq!(rows(&reply, &reply.incoming, true), [(Some(Some(HOL)), Some("26".to_owned()), None), (Some(Some(TUR)), Some("12".to_owned()), None), (None, Some("9".to_owned()), Some(1))], "top 2 of 3 folds the third");
        assert_eq!(reply.incoming.unranked, None);
        let wide = build(&complete, 1, &FlowShape { top: Some(3), ..shape(Some(USA), Some(1), false) });
        assert_eq!(rows(&wide, &wide.incoming, true).len(), 3, "an explicit top of 3 names all three even where the viewport would fold after one");
        let unknown = [edge(HOL, USA, 0, Some(26)), edge(RAJ, USA, 0, None), edge(TUR, USA, 0, Some(12)), edge(IRN, USA, 0, Some(9))];
        let reply = topped(&unknown, 1);
        assert_eq!(
            rows(&reply, &reply.incoming, true),
            [(Some(Some(HOL)), Some("26".to_owned()), None), (Some(Some(TUR)), Some("12".to_owned()), None), (Some(Some(IRN)), Some("9".to_owned()), None), (Some(Some(RAJ)), None, None)],
            "an unknown amount may be the largest: no top is claimed and every witnessed link is kept"
        );
        assert_eq!(reply.incoming.unranked.as_deref(), Some("not recorded"));
        let combined = [edge(HOL, USA, 0, Some(26)), edge(UNKNOWN, USA, 0, Some(30)), edge(TUR, USA, 0, Some(12))];
        let reply = topped(&combined, 1);
        assert_eq!(reply.incoming.links.len(), 3, "a combined amount of unknown partners is not one partner and leaves the order unproven");
        assert_eq!(reply.incoming.unranked.as_deref(), Some("unlinked"));
        let gapped = build_gapped(&complete, &[FlowGap::everything("not every deal was read".to_owned())], 1, &FlowShape { top: Some(1), ..shape(Some(USA), None, false) });
        assert_eq!((gapped.incoming.links.len(), gapped.incoming.unranked.as_deref()), (3, Some("not every deal was read")));
    }

    #[test]
    fn a_partner_with_a_known_amount_ranks_even_when_its_country_cannot_be_named() {
        let edges = [edge(HOL, USA, 0, Some(5)), edge(FlowEnd::Raw(70), USA, 0, Some(40)), edge(TUR, USA, 0, Some(3))];
        let reply = build(&edges, 1, &FlowShape { top: Some(1), ..shape(Some(USA), None, false) });
        assert_eq!(keys(&reply, &reply.incoming, true), [(Some("r70"), Some("40"), None), (None, Some("8"), Some(2))]);
        assert_eq!(reply.incoming.unranked, None);
    }

    #[test]
    fn a_partial_read_keeps_every_witnessed_link_and_withholds_what_its_unread_rows_may_add() {
        let edges = [edge(HOL, USA, 0, Some(4)), edge(USA, ENG, 0, Some(7))];
        let reply = build_gapped(&edges, &[FlowGap::partial("deals read only in part".to_owned())], 1, &FlowShape { top: Some(1), ..shape(Some(USA), None, false) });
        assert_eq!(rows(&reply, &reply.incoming, true), [(Some(Some(HOL)), None, None)]);
        assert_eq!(reply.incoming.links[0].amounts, [FlowAmount { part: 0, amount: None }], "a sum over a partial read is not the pair's amount");
        assert_eq!(reply.incoming.links[0].missing.as_deref(), Some("deals read only in part"));
        assert_eq!(rows(&reply, &reply.outgoing, false), [(Some(Some(ENG)), None, None)]);
        assert_eq!((reply.outgoing.total.as_deref(), reply.outgoing.missing.as_deref()), (None, Some("deals read only in part")));
    }

    #[test]
    fn the_table_signs_imports_positive_and_exports_negative_and_sorts_by_net() {
        let edges = [edge(ENG, USA, 0, Some(5)), edge(USA, ENG, 0, Some(45)), edge(USA, ENG, 1, Some(24)), edge(HOL, USA, 1, Some(26)), edge(UNKNOWN, USA, 0, Some(2))];
        let reply = build(&edges, 2, &shape(Some(USA), None, true));
        let table: Vec<(Slot, Vec<Option<&str>>, Option<&str>)> =
            reply.table.iter().map(|row| (reply.endpoints[row.partner as usize].slot, row.cells.iter().map(|cell| cell.as_deref()).collect(), row.net.as_deref())).collect();
        assert_eq!(table, [(Some(ENG), vec![Some("-40"), Some("-24")], Some("-64")), (Some(HOL), vec![Some("0"), Some("26")], Some("26")), (None, vec![Some("2"), Some("0")], Some("2"))]);
    }

    #[test]
    fn a_group_lists_its_links_with_senders_and_receivers_ranked_and_folded() {
        let edges = [edge(SOV, SPR, 0, Some(4)), edge(ITA, SPD, 0, Some(9)), edge(GER, SPD, 0, Some(2)), edge(POR, SPD, 0, Some(3))];
        let reply = build(&edges, 1, &shape(None, Some(2), false));
        assert_eq!(rows(&reply, &reply.senders, true), [(Some(Some(ITA)), Some("9".to_owned()), None), (Some(Some(SOV)), Some("4".to_owned()), None), (None, Some("5".to_owned()), Some(2))]);
        assert_eq!(reply.receivers.links.iter().map(|link| slot(&reply, link.to)).collect::<Vec<_>>(), [Some(Some(SPD)), Some(Some(SPR))]);
        let folded = reply.links.iter().find(|link| link.from.is_none()).unwrap();
        assert_eq!((folded.total.as_deref(), folded.others), (Some("5"), Some(2)));
        assert_eq!(reply.largest, "14");
        assert_eq!(reply.largest_link, "9");
    }

    #[test]
    fn a_group_keeps_a_country_that_both_sends_and_receives_on_both_sides() {
        let reply = build(&[edge(ITA, SPD, 0, Some(9)), edge(SPD, ITA, 0, Some(1))], 1, &shape(None, None, false));
        let sender = reply.senders.links.iter().find(|link| slot(&reply, link.from) == Some(Some(ITA))).unwrap();
        let receiver = reply.receivers.links.iter().find(|link| slot(&reply, link.to) == Some(Some(ITA))).unwrap();
        assert_eq!((sender.total.as_deref(), receiver.total.as_deref()), (Some("9"), Some("1")));
        assert_eq!(reply.links.len(), 2);
    }

    #[test]
    fn if_the_last_named_partner_fits_it_uses_the_fold_row() {
        let edges: Vec<_> = (1..=6).map(|from| edge(from, from + 6, 0, Some(1))).collect();
        let reply = build(&edges, 1, &shape(None, Some(5), false));
        assert_eq!(reply.links.len(), 6);
        for side in [reply.senders, reply.receivers] {
            assert_eq!(side.links.len(), 6);
            assert!(side.links.iter().all(|link| link.others.is_none()));
        }
    }

    #[test]
    fn side_totals_fit_the_reply_budget_regardless_of_pair_count() {
        let countries = 1_500;
        let partners = 256;
        let ring = |width, amount| {
            (0..countries).flat_map(|from| {
                (1..=width).map(move |offset| edge(from, (from + offset) % countries, 0, Some(amount)))
            }).collect::<Vec<_>>()
        };
        let drill = FlowShape { side_totals_only: true, ..shape(None, None, false) };
        let sparse = build(&ring(1, i128::from(partners)), 1, &drill);
        let dense = build(&ring(partners, 1), 1, &drill);

        assert_eq!(dense.endpoints.len(), countries as usize);
        assert_eq!(dense.senders.links.len(), dense.endpoints.len());
        assert_eq!(dense.receivers.links.len(), dense.endpoints.len());
        assert!(dense.links.is_empty());
        assert_eq!(serde_json::to_vec(&dense).unwrap(), serde_json::to_vec(&sparse).unwrap());
        crate::products::check_reply_budget(&dense).unwrap();
    }

    #[test]
    fn between_chosen_countries_a_link_to_an_unknown_country_is_not_asserted_and_says_why() {
        let chosen: HashSet<u32> = [SOV, ITA].into_iter().collect();
        let between = FlowShape { chosen: Some(&chosen), between: true, ..shape(None, None, false) };
        let reply = build(&[edge(SOV, ITA, 0, Some(2)), edge(SOV, UNKNOWN, 0, Some(5)), edge(SOV, GER, 0, Some(7))], 1, &between);
        assert_eq!(reply.links.len(), 1);
        assert_eq!((reply.senders.total.as_deref(), reply.senders.missing.as_deref()), (None, Some("unlinked")));
        let either = FlowShape { chosen: Some(&chosen), between: false, ..shape(None, None, false) };
        let reply = build(&[edge(SOV, UNKNOWN, 0, Some(5)), edge(UNKNOWN, GER, 0, Some(1))], 1, &either);
        assert_eq!(rows(&reply, &reply.receivers, false), [(Some(None), Some("5".to_owned()), None)]);
        assert_eq!(reply.receivers.missing.as_deref(), Some("unlinked"));
    }
}
