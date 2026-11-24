use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FlowPart {
    pub key: String,
    pub name: Option<String>,
    pub name_missing: Option<String>,
    #[doc = " The part's place in the recording's own order, for its colour."]
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
pub struct FlowLink {
    #[doc = " Indexes into `countries`; null on the side folded into the others."]
    pub from: Option<u32>,
    pub to: Option<u32>,
    #[doc = " How many partners the others hold, where this link folds them."]
    pub others: Option<u32>,
    pub amounts: Vec<FlowAmount>,
    pub total: Option<String>,
    #[doc = " Why the total is not known, where an amount was not recorded."]
    pub missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FlowTableRow {
    pub country: u32,
    #[doc = " One signed amount per part (in to the subject positive, out negative, zero where none); null where not recorded."]
    pub cells: Vec<Option<String>>,
    pub net: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CountryFlowReply {
    pub countries: Vec<String>,
    #[doc = " The centre country, when the subject is one country."]
    pub subject: Option<u32>,
    pub decimals: u8,
    pub parts: Vec<FlowPart>,
    #[doc = " Into the subject, largest first; ties by tag; the rest folded into one last link."]
    pub incoming: Vec<FlowLink>,
    #[doc = " Out of the subject, in the same order."]
    pub outgoing: Vec<FlowLink>,
    pub incoming_total: Option<String>,
    pub outgoing_total: Option<String>,
    #[doc = " A group subject: every link between its senders and receivers."]
    pub links: Vec<FlowLink>,
    #[doc = " A group subject: each sender's total (`from`) and each receiver's (`to`), largest first, the rest folded into one last entry."]
    pub senders: Vec<FlowLink>,
    pub receivers: Vec<FlowLink>,
    #[doc = " Partners by part, signed, largest net first; only when asked."]
    pub table: Vec<FlowTableRow>,
    #[doc = " The largest side or node total, for the scale."]
    pub largest: String,
    #[doc = " The largest single link, for a colour scale."]
    pub largest_link: String,
    pub missing: Option<String>,
    #[doc = " Why some of the flows at the hour are left out."]
    pub partial: Option<String>,
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
    pub from: String,
    pub to: String,
    pub part: u32,
    pub amount: Option<i128>,
}

pub struct FlowShape<'a> {
    pub subject: Option<&'a str>,
    pub chosen: Option<&'a HashSet<String>>,
    pub between: bool,
    pub limit: Option<usize>,
    pub table: bool,
    pub not_recorded: &'a str,
}

type Sums = Vec<Option<i128>>;

fn add(sums: &mut Sums, part: u32, amount: Option<i128>) {
    let slot = &mut sums[part as usize];
    *slot = slot.zip(amount).map(|(a, b)| a + b);
}

fn total(sums: &Sums, present: &[bool]) -> Option<i128> {
    sums.iter().zip(present).filter(|(_, here)| **here).try_fold(0i128, |held, (sum, _)| sum.map(|sum| held + sum))
}

struct Grouped {
    sums: Sums,
    present: Vec<bool>,
}

impl Grouped {
    fn new(parts: usize) -> Self {
        Self { sums: vec![Some(0); parts], present: vec![false; parts] }
    }

    fn add(&mut self, part: u32, amount: Option<i128>) {
        add(&mut self.sums, part, amount);
        self.present[part as usize] = true;
    }

    fn total(&self) -> Option<i128> {
        total(&self.sums, &self.present)
    }

    fn absorb(&mut self, other: &Grouped) {
        for (part, here) in other.present.iter().enumerate() {
            if *here {
                self.add(part as u32, other.sums[part]);
            }
        }
    }
}

struct Interner {
    countries: Vec<String>,
    index: HashMap<String, u32>,
}

impl Interner {
    fn of(&mut self, tag: &str) -> u32 {
        if let Some(&at) = self.index.get(tag) {
            return at;
        }
        let at = self.countries.len() as u32;
        self.countries.push(tag.to_owned());
        self.index.insert(tag.to_owned(), at);
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
    FlowLink { from, to, others, amounts, total: total.map(|sum| sum.to_string()), missing: total.is_none().then(|| not_recorded.to_owned()) }
}

fn ranked(by: BTreeMap<String, Grouped>) -> Vec<(String, Grouped)> {
    let mut out: Vec<(String, Grouped)> = by.into_iter().collect();
    out.sort_by(|a, b| match (a.1.total(), b.1.total()) {
        (Some(x), Some(y)) => y.cmp(&x).then(a.0.cmp(&b.0)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.0.cmp(&b.0),
    });
    out
}

fn side(ranked: Vec<(String, Grouped)>, limit: Option<usize>, parts: usize, subject: u32, inbound: bool, names: &mut Interner, not_recorded: &str) -> (Vec<FlowLink>, Option<i128>) {
    let mut whole = Grouped::new(parts);
    for (_, grouped) in &ranked {
        whole.absorb(grouped);
    }
    let keep = limit.map_or(ranked.len(), |limit| if ranked.len() > limit + 1 { limit } else { ranked.len() });
    let mut out = Vec::with_capacity(keep + 1);
    let mut rest = Grouped::new(parts);
    let mut folded = 0u32;
    for (at, (tag, grouped)) in ranked.into_iter().enumerate() {
        if at < keep {
            let partner = names.of(&tag);
            let (from, to) = if inbound { (Some(partner), Some(subject)) } else { (Some(subject), Some(partner)) };
            out.push(link(from, to, None, &grouped, not_recorded));
        } else {
            rest.absorb(&grouped);
            folded += 1;
        }
    }
    if folded > 0 {
        let (from, to) = if inbound { (None, Some(subject)) } else { (Some(subject), None) };
        out.push(link(from, to, Some(folded), &rest, not_recorded));
    }
    (out, whole.total())
}

pub fn build_flows(edges: &[FlowEdge], parts: Vec<FlowPart>, decimals: u8, shape: &FlowShape<'_>) -> CountryFlowReply {
    let count = parts.len();
    let mut names = Interner { countries: Vec::new(), index: HashMap::new() };
    let mut reply = CountryFlowReply { decimals, largest: "0".to_owned(), largest_link: "0".to_owned(), ..CountryFlowReply::default() };
    if let Some(subject) = shape.subject {
        let at = names.of(subject);
        reply.subject = Some(at);
        let mut incoming: BTreeMap<String, Grouped> = BTreeMap::new();
        let mut outgoing: BTreeMap<String, Grouped> = BTreeMap::new();
        for edge in edges {
            if edge.to == subject && edge.from != subject {
                incoming.entry(edge.from.clone()).or_insert_with(|| Grouped::new(count)).add(edge.part, edge.amount);
            } else if edge.from == subject && edge.to != subject {
                outgoing.entry(edge.to.clone()).or_insert_with(|| Grouped::new(count)).add(edge.part, edge.amount);
            }
        }
        if shape.table {
            let partners: std::collections::BTreeSet<&String> = incoming.keys().chain(outgoing.keys()).collect();
            let mut rows: Vec<(String, Sums, Option<i128>)> = partners
                .into_iter()
                .map(|tag| {
                    let side = |held: &BTreeMap<String, Grouped>, part: usize| -> Option<Option<i128>> {
                        held.get(tag).filter(|grouped| grouped.present[part]).map(|grouped| grouped.sums[part])
                    };
                    let cells: Sums = (0..count)
                        .map(|part| match (side(&incoming, part), side(&outgoing, part)) {
                            (Some(None), _) | (_, Some(None)) => None,
                            (inbound, outbound) => Some(inbound.flatten().unwrap_or(0) - outbound.flatten().unwrap_or(0)),
                        })
                        .collect();
                    let net = cells.iter().try_fold(0i128, |held, cell| cell.map(|cell| held + cell));
                    (tag.clone(), cells, net)
                })
                .collect();
            rows.sort_by(|a, b| match (a.2, b.2) {
                (Some(x), Some(y)) => y.abs().cmp(&x.abs()).then(a.0.cmp(&b.0)),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.0.cmp(&b.0),
            });
            for (tag, cells, net) in rows {
                let country = names.of(&tag);
                reply.table.push(FlowTableRow { country, cells: cells.iter().map(|cell| cell.map(|value| value.to_string())).collect(), net: net.map(|value| value.to_string()) });
            }
        }
        let (inbound, in_total) = side(ranked(incoming), shape.limit, count, at, true, &mut names, shape.not_recorded);
        let (outbound, out_total) = side(ranked(outgoing), shape.limit, count, at, false, &mut names, shape.not_recorded);
        reply.incoming = inbound;
        reply.outgoing = outbound;
        reply.incoming_total = in_total.map(|sum| sum.to_string());
        reply.outgoing_total = out_total.map(|sum| sum.to_string());
        reply.largest = in_total.unwrap_or(0).max(out_total.unwrap_or(0)).to_string();
    } else {
        let keep = |edge: &FlowEdge| match shape.chosen {
            None => true,
            Some(chosen) if shape.between => chosen.contains(&edge.from) && chosen.contains(&edge.to),
            Some(chosen) => chosen.contains(&edge.from) || chosen.contains(&edge.to),
        };
        let mut pairs: BTreeMap<(String, String), Grouped> = BTreeMap::new();
        let mut sent: BTreeMap<String, Grouped> = BTreeMap::new();
        let mut received: BTreeMap<String, Grouped> = BTreeMap::new();
        for edge in edges.iter().filter(|edge| keep(edge) && edge.from != edge.to) {
            pairs.entry((edge.from.clone(), edge.to.clone())).or_insert_with(|| Grouped::new(count)).add(edge.part, edge.amount);
            sent.entry(edge.from.clone()).or_insert_with(|| Grouped::new(count)).add(edge.part, edge.amount);
            received.entry(edge.to.clone()).or_insert_with(|| Grouped::new(count)).add(edge.part, edge.amount);
        }
        let largest = sent.values().chain(received.values()).filter_map(Grouped::total).max().unwrap_or(0);
        let fold = |ranked: Vec<(String, Grouped)>, outbound: bool, names: &mut Interner| -> (Vec<FlowLink>, HashSet<String>) {
            let keep = shape.limit.map_or(ranked.len(), |limit| if ranked.len() > limit + 1 { limit } else { ranked.len() });
            let mut out = Vec::with_capacity(keep + 1);
            let mut kept = HashSet::new();
            let mut rest = Grouped::new(count);
            let mut folded = 0u32;
            for (at, (tag, grouped)) in ranked.into_iter().enumerate() {
                if at < keep {
                    let index = Some(names.of(&tag));
                    let (from, to) = if outbound { (index, None) } else { (None, index) };
                    out.push(link(from, to, None, &grouped, shape.not_recorded));
                    kept.insert(tag);
                } else {
                    rest.absorb(&grouped);
                    folded += 1;
                }
            }
            if folded > 0 {
                out.push(link(None, None, Some(folded), &rest, shape.not_recorded));
            }
            (out, kept)
        };
        let (senders, kept_sources) = fold(ranked(sent), true, &mut names);
        let (receivers, kept_targets) = fold(ranked(received), false, &mut names);
        reply.senders = senders;
        reply.receivers = receivers;
        let mut folded: BTreeMap<(Option<u32>, Option<u32>), (Grouped, HashSet<String>)> = BTreeMap::new();
        for ((from, to), grouped) in &pairs {
            let a = kept_sources.contains(from).then(|| names.of(from));
            let b = kept_targets.contains(to).then(|| names.of(to));
            if a.is_some() && b.is_some() {
                reply.links.push(link(a, b, None, grouped, shape.not_recorded));
            } else {
                let held = folded.entry((a, b)).or_insert_with(|| (Grouped::new(count), HashSet::new()));
                held.0.absorb(grouped);
                if a.is_none() {
                    held.1.insert(from.clone());
                }
                if b.is_none() {
                    held.1.insert(to.clone());
                }
            }
        }
        for ((a, b), (grouped, partners)) in folded {
            reply.links.push(link(a, b, Some(partners.len() as u32), &grouped, shape.not_recorded));
        }
        reply.largest = largest.to_string();
    }
    let biggest = reply.incoming.iter().chain(&reply.outgoing).chain(&reply.links).filter(|link| link.others.is_none()).filter_map(|link| link.total.as_deref()?.parse::<i128>().ok()).max().unwrap_or(0);
    reply.largest_link = biggest.to_string();
    reply.countries = names.countries;
    reply.parts = parts;
    reply
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(key: &str, ordinal: u32) -> FlowPart {
        FlowPart { key: key.to_owned(), name: Some(key.to_owned()), name_missing: None, ordinal, icon: None }
    }

    fn edge(from: &str, to: &str, part: u32, amount: Option<i128>) -> FlowEdge {
        FlowEdge { from: from.to_owned(), to: to.to_owned(), part, amount }
    }

    fn shape(subject: Option<&str>, limit: Option<usize>, table: bool) -> FlowShape<'_> {
        FlowShape { subject, chosen: None, between: false, limit, table, not_recorded: "not recorded" }
    }

    #[test]
    fn a_country_sees_its_partners_both_ways_largest_first_and_the_rest_folded() {
        let edges = [
            edge("HOL", "USA", 0, Some(26)),
            edge("RAJ", "USA", 1, Some(24)),
            edge("TUR", "USA", 0, Some(12)),
            edge("IRN", "USA", 0, Some(9)),
            edge("USA", "ENG", 0, Some(40)),
            edge("USA", "ENG", 1, Some(34)),
            edge("USA", "JAP", 0, Some(18)),
        ];
        let reply = build_flows(&edges, vec![part("oil", 0), part("steel", 1)], 0, &shape(Some("USA"), Some(2), false));
        let tag = |at: Option<u32>| at.map(|at| reply.countries[at as usize].as_str());
        assert_eq!(reply.incoming.iter().map(|link| (tag(link.from), link.total.as_deref(), link.others)).collect::<Vec<_>>(), [
            (Some("HOL"), Some("26"), None),
            (Some("RAJ"), Some("24"), None),
            (None, Some("21"), Some(2)),
        ]);
        assert_eq!(reply.outgoing.iter().map(|link| (tag(link.to), link.total.as_deref())).collect::<Vec<_>>(), [(Some("ENG"), Some("74")), (Some("JAP"), Some("18"))]);
        assert_eq!(reply.incoming_total.as_deref(), Some("71"));
        assert_eq!(reply.outgoing_total.as_deref(), Some("92"));
        assert_eq!(reply.largest, "92");
    }

    #[test]
    fn an_unrecorded_amount_makes_its_link_and_side_total_missing_never_zero() {
        let edges = [edge("HOL", "USA", 0, Some(26)), edge("RAJ", "USA", 0, None)];
        let reply = build_flows(&edges, vec![part("oil", 0)], 0, &shape(Some("USA"), None, false));
        assert_eq!(reply.incoming[0].total.as_deref(), Some("26"));
        assert_eq!(reply.incoming[1].total, None);
        assert_eq!(reply.incoming[1].missing.as_deref(), Some("not recorded"));
        assert_eq!(reply.incoming_total, None);
    }

    #[test]
    fn the_table_signs_imports_positive_and_exports_negative_and_sorts_by_net() {
        let edges = [edge("ENG", "USA", 0, Some(5)), edge("USA", "ENG", 0, Some(45)), edge("USA", "ENG", 1, Some(24)), edge("HOL", "USA", 1, Some(26))];
        let reply = build_flows(&edges, vec![part("oil", 0), part("steel", 1)], 0, &shape(Some("USA"), None, true));
        let rows: Vec<(&str, Vec<Option<&str>>, Option<&str>)> = reply
            .table
            .iter()
            .map(|row| (reply.countries[row.country as usize].as_str(), row.cells.iter().map(|cell| cell.as_deref()).collect(), row.net.as_deref()))
            .collect();
        assert_eq!(rows, [("ENG", vec![Some("-40"), Some("-24")], Some("-64")), ("HOL", vec![Some("0"), Some("26")], Some("26"))]);
    }

    #[test]
    fn a_group_lists_its_links_with_senders_and_receivers_ranked_and_folded() {
        let edges = [edge("SOV", "SPR", 0, Some(4)), edge("ITA", "SPD", 0, Some(9)), edge("GER", "SPD", 0, Some(2)), edge("POR", "SPD", 0, Some(3))];
        let reply = build_flows(&edges, vec![part("volunteers", 0)], 0, &shape(None, Some(2), false));
        let tag = |at: Option<u32>| at.map(|at| reply.countries[at as usize].clone());
        assert_eq!(reply.senders.iter().map(|link| (tag(link.from), link.total.clone(), link.others)).collect::<Vec<_>>(), [
            (Some("ITA".to_owned()), Some("9".to_owned()), None),
            (Some("SOV".to_owned()), Some("4".to_owned()), None),
            (None, Some("5".to_owned()), Some(2)),
        ]);
        assert_eq!(reply.receivers.iter().map(|link| tag(link.to)).collect::<Vec<_>>(), [Some("SPD".to_owned()), Some("SPR".to_owned())]);
        let folded = reply.links.iter().find(|link| link.from.is_none()).unwrap();
        assert_eq!((folded.total.as_deref(), folded.others), (Some("5"), Some(2)));
        assert_eq!(reply.largest, "14");
        assert_eq!(reply.largest_link, "9");
    }
}
