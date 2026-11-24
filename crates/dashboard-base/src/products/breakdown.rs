use serde::Serialize;
use std::collections::HashMap;
use ts_rs::TS;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum BreakdownState {
    Known,
    Partial,
    AtLeast,
    NotRecorded,
}

#[derive(Clone, Debug, Serialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
pub struct BreakdownTotal {
    #[doc = " Scaled by `decimals`; none when nothing is known."]
    pub raw: Option<String>,
    pub state: BreakdownState,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
pub struct BreakdownPart {
    #[doc = " Unique among its siblings."]
    pub key: String,
    pub name: String,
    #[doc = " Its place in the recording's own order of the parts at its level: colours follow it, so a"]
    #[doc = " filtered or reordered breakdown keeps them."]
    pub ordinal: u32,
    #[doc = " Unattributed or other: drawn in the reserved grey."]
    pub other: bool,
    #[doc = " Scaled by `decimals`; none when not recorded."]
    pub raw: Option<String>,
    pub state: BreakdownState,
    pub reason: Option<String>,
    #[doc = " Of the whole; none where the whole is only a lower bound or the part is not positive."]
    pub share: Option<f64>,
    pub count: Option<u64>,
    #[doc = " The latest hour one of its amounts was read; none where the product does not date them."]
    pub last: Option<i32>,
    #[doc = " In display order."]
    pub children: Vec<BreakdownPart>,
}

#[derive(Clone, Debug, Serialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
pub struct Breakdown {
    pub decimals: u8,
    pub total: BreakdownTotal,
    #[doc = " In display order: the largest known part first, then parts at zero or below, then other,"]
    #[doc = " then parts not recorded."]
    pub parts: Vec<BreakdownPart>,
    #[doc = " Hours of the period that were not recorded."]
    pub unrecorded_hours: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Amount {
    Known(i128),
    Partial(i128, String),
    NotRecorded(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub key: String,
    pub name: String,
    pub ordinal: u32,
    pub other: bool,
}

impl Node {
    pub fn new(key: impl Into<String>, name: impl Into<String>, ordinal: u32) -> Self {
        Self { key: key.into(), name: name.into(), ordinal, other: false }
    }

    pub fn other(mut self) -> Self {
        self.other = true;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leaf {
    pub path: Vec<Node>,
    pub amount: Amount,
    pub count: Option<u64>,
    pub last: Option<i32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Unrecorded {
    pub hours: u32,
    pub reason: Option<String>,
}

#[derive(Default)]
struct Sum {
    known: i128,
    measured: u32,
    partial: Vec<String>,
    missing: Vec<String>,
    count: Option<u64>,
    last: Option<i32>,
}

impl Sum {
    fn add(&mut self, amount: &Amount, count: Option<u64>, last: Option<i32>) {
        if let Some(hour) = last {
            self.last = Some(self.last.map_or(hour, |held| held.max(hour)));
        }
        match amount {
            Amount::Known(value) => {
                self.known = self.known.saturating_add(*value);
                self.measured += 1;
            }
            Amount::Partial(value, reason) => {
                self.known = self.known.saturating_add(*value);
                self.measured += 1;
                remember(&mut self.partial, reason);
            }
            Amount::NotRecorded(reason) => remember(&mut self.missing, reason),
        }
        if let Some(count) = count {
            self.count = Some(self.count.unwrap_or(0).saturating_add(count));
        }
    }

    fn state(&self) -> BreakdownState {
        match (self.measured, self.missing.is_empty(), self.partial.is_empty()) {
            (0, false, _) => BreakdownState::NotRecorded,
            (_, false, _) => BreakdownState::AtLeast,
            (_, true, false) => BreakdownState::Partial,
            _ => BreakdownState::Known,
        }
    }

    fn reason(&self) -> Option<String> {
        let reasons: Vec<&str> = self.missing.iter().chain(&self.partial).map(String::as_str).collect();
        (!reasons.is_empty()).then(|| reasons.join("; "))
    }

    fn raw(&self) -> Option<String> {
        (self.measured > 0 || self.missing.is_empty()).then(|| self.known.to_string())
    }
}

fn remember(reasons: &mut Vec<String>, reason: &str) {
    if !reasons.iter().any(|held| held == reason) {
        reasons.push(reason.to_owned());
    }
}

struct Branch {
    node: Node,
    sum: Sum,
    children: Vec<Branch>,
    index: HashMap<String, usize>,
}

impl Branch {
    fn new(node: Node) -> Self {
        Self { node, sum: Sum::default(), children: Vec::new(), index: HashMap::new() }
    }

    fn child(&mut self, node: &Node) -> &mut Branch {
        let at = match self.index.get(&node.key) {
            Some(&at) => at,
            None => {
                self.children.push(Branch::new(node.clone()));
                self.index.insert(node.key.clone(), self.children.len() - 1);
                self.children.len() - 1
            }
        };
        &mut self.children[at]
    }
}

fn class(branch: &Branch) -> u8 {
    match branch.sum.state() {
        BreakdownState::NotRecorded => 3,
        _ if branch.node.other => 2,
        _ if branch.sum.known > 0 => 0,
        _ => 1,
    }
}

fn parts(children: Vec<Branch>, whole: Option<i128>) -> Vec<BreakdownPart> {
    let mut children = children;
    children.sort_by(|a, b| {
        class(a)
            .cmp(&class(b))
            .then_with(|| if class(a) < 2 { b.sum.known.cmp(&a.sum.known) } else { std::cmp::Ordering::Equal })
            .then_with(|| a.node.ordinal.cmp(&b.node.ordinal))
    });
    children
        .into_iter()
        .map(|branch| {
            let state = branch.sum.state();
            let share = match (whole, state) {
                (Some(whole), BreakdownState::Known | BreakdownState::Partial) if whole > 0 && branch.sum.known > 0 => Some(branch.sum.known as f64 / whole as f64),
                _ => None,
            };
            BreakdownPart {
                key: branch.node.key,
                name: branch.node.name,
                ordinal: branch.node.ordinal,
                other: branch.node.other,
                raw: branch.sum.raw(),
                state,
                reason: branch.sum.reason(),
                share,
                count: branch.sum.count,
                last: branch.sum.last,
                children: parts(branch.children, whole),
            }
        })
        .collect()
}

pub fn breakdown(decimals: u8, leaves: &[Leaf], unrecorded: &Unrecorded) -> Breakdown {
    let mut root = Branch::new(Node::new("", "", 0));
    for leaf in leaves {
        root.sum.add(&leaf.amount, leaf.count, leaf.last);
        let mut at = &mut root;
        for node in &leaf.path {
            at = at.child(node);
            at.sum.add(&leaf.amount, leaf.count, leaf.last);
        }
    }
    let mut state = root.sum.state();
    let mut reason = root.sum.reason();
    if unrecorded.hours > 0 {
        if state == BreakdownState::Known {
            state = BreakdownState::Partial;
        }
        if let Some(gap) = &unrecorded.reason {
            reason = Some(match reason {
                Some(held) if !held.contains(gap.as_str()) => format!("{gap}; {held}"),
                Some(held) => held,
                None => gap.clone(),
            });
        }
    }
    let whole = matches!(state, BreakdownState::Known | BreakdownState::Partial).then_some(root.sum.known);
    Breakdown {
        decimals,
        total: BreakdownTotal { raw: root.sum.raw(), state, reason },
        parts: parts(root.children, whole),
        unrecorded_hours: unrecorded.hours,
    }
}
