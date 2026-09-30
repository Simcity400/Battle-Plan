//! `_CountriesForOriginalTags` (CGameState+0x988), derived from recorded
//! data.
//!
//! Per original tag, the countries whose original tag (CCountry+0x130C,
//! recorded as `CountryAppearance.name_tag`) it is, in the order
//! ResolveScriptCountryTag (140BA5D10) walks through 1401DA520:
//! - every load rebuilds it in country order (1401C85E0 -> 1401DA010, the
//!   countries with +0x130C > 0 by index), so a branch that starts at a
//!   load (`BranchReason::LoadSignal`) starts in canonical slot order;
//! - afterwards CCountry::ResetForDynamicReuse (140703760) removes the
//!   country from every list, keeping the others' order (1401D1BC0), and
//!   CGameState::SetOriginalTag (1401EE1C0) appends it to its new tag's
//!   list, so each recorded `name_tag` change is a removal and an append,
//!   taken in hour order.
//!
//! What stays unknown is kept as a group of members whose order is not
//! known: countries appended within one recorded interval (one hour, or
//! the hours a missing `CountryAppearance` spans), and, for a branch that
//! does not start at a load, the members it starts with. A group matters
//! only when it is the first holding a state-owning country and holds two:
//! then the resolver refuses, naming the tag, the interval and the
//! candidates. A reuse that keeps the same original tag moves the country
//! to the end natively but changes no recorded field; it is not seen.

use replay::{
    Hour,
    frame::{DecodedFrame, TableView},
    rows::{TableSchema, tables},
};
use std::collections::BTreeMap;

/// Countries whose relative order in a list is known up to the group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    /// Canonical slots, in slot order when the group is unordered.
    pub members: Vec<u32>,
    /// When they were appended, for a refusal: `hour 12`, `hours 10..12`,
    /// or the branch start.
    pub when: String,
}

/// The derived lists at one hour.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OriginalTagLists {
    lists: BTreeMap<Vec<u8>, Vec<Group>>,
    /// Each slot's last recorded `name_tag`.
    current: BTreeMap<u32, Vec<u8>>,
    /// The last hour whose `CountryAppearance` was complete.
    last_hour: Option<Hour>,
    /// Hours since then whose `CountryAppearance` was not complete.
    pending_since: Option<Hour>,
}

/// Each slot's `name_tag`, or `None` when the hour lacks the full table.
fn name_tags(frame: &DecodedFrame) -> Option<BTreeMap<u32, Vec<u8>>> {
    let complete =
        |id: replay::rows::TableId| matches!(&*frame.tables[id.index()], TableView::Complete(_));
    if !complete(tables::CountryAppearance::ID) || !complete(tables::CountryLinks::ID) {
        return None;
    }
    let links: BTreeMap<u32, u32> = frame
        .complete::<tables::CountryLinks>()
        .ok()?
        .iter()
        .map(|link| (link.id, link.canonical_slot))
        .collect();
    Some(
        frame
            .complete::<tables::CountryAppearance>()
            .ok()?
            .iter()
            .filter_map(|row| Some((*links.get(&row.country)?, row.name_tag.clone())))
            .filter(|(_, tag)| !tag.is_empty())
            .collect(),
    )
}

impl OriginalTagLists {
    /// The lists at a branch's first frame: in slot order when the branch
    /// starts at a load (1401DA010), otherwise one unordered group per tag.
    pub fn start(frame: &DecodedFrame, at_load: bool) -> Self {
        let mut lists = Self::default();
        let Some(current) = name_tags(frame) else {
            lists.pending_since = Some(frame.hour);
            return lists;
        };
        let mut members: BTreeMap<Vec<u8>, Vec<u32>> = BTreeMap::new();
        for (slot, tag) in &current {
            members.entry(tag.clone()).or_default().push(*slot);
        }
        for (tag, slots) in members {
            let groups = if at_load {
                slots
                    .into_iter()
                    .map(|slot| Group {
                        members: vec![slot],
                        when: format!("the load at hour {}", frame.hour),
                    })
                    .collect()
            } else {
                vec![Group {
                    members: slots,
                    when: format!(
                        "before hour {}, where the branch starts without a load",
                        frame.hour
                    ),
                }]
            };
            lists.lists.insert(tag, groups);
        }
        lists.current = current;
        lists.last_hour = Some(frame.hour);
        lists
    }

    /// One hour's order unknown: the lists as one frame alone shows them.
    pub fn unordered(frame: &DecodedFrame) -> Self {
        let mut lists = Self::start(frame, false);
        for groups in lists.lists.values_mut() {
            for group in groups {
                group.when = format!(
                    "at hour {}, read without the branch's earlier hours",
                    frame.hour
                );
            }
        }
        lists
    }

    /// Applies the next recorded frame of the branch: each `name_tag`
    /// change removes the country from its old list and appends it to the
    /// new one (1401D1BC0, 1401EE1C0); appends of one interval form one
    /// group.
    pub fn advance(&mut self, frame: &DecodedFrame) {
        let Some(next) = name_tags(frame) else {
            self.pending_since.get_or_insert(frame.hour);
            return;
        };
        let when = match (self.pending_since.take(), self.last_hour) {
            (Some(since), _) => format!("hours {since}..{}", frame.hour),
            (None, Some(last)) if last + 1 < frame.hour => {
                format!("hours {}..{}", last + 1, frame.hour)
            }
            _ => format!("hour {}", frame.hour),
        };
        let slots: Vec<u32> = self
            .current
            .keys()
            .chain(next.keys())
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut appended: BTreeMap<Vec<u8>, Vec<u32>> = BTreeMap::new();
        for slot in slots {
            let (old, new) = (self.current.get(&slot).cloned(), next.get(&slot));
            if old.as_ref() == new {
                continue;
            }
            if let Some(old) = old {
                self.remove(&old, slot);
            }
            if let Some(new) = new {
                appended.entry(new.clone()).or_default().push(slot);
            }
        }
        for (tag, members) in appended {
            self.lists.entry(tag).or_default().push(Group {
                members,
                when: when.clone(),
            });
        }
        self.current = next;
        self.last_hour = Some(frame.hour);
    }

    fn remove(&mut self, tag: &[u8], slot: u32) {
        if let Some(groups) = self.lists.get_mut(tag) {
            for group in groups.iter_mut() {
                group.members.retain(|member| *member != slot);
            }
            groups.retain(|group| !group.members.is_empty());
        }
    }

    /// The list of `tag`, first to last.
    pub fn list(&self, tag: &str) -> &[Group] {
        self.lists
            .get(tag.as_bytes())
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// A country's original tag (CCountry+0x130C, `CountryAppearance.name_tag`)
    /// as last applied; `None` for a country without one.
    pub fn name_tag(&self, slot: u32) -> Option<&[u8]> {
        self.current.get(&slot).map(Vec::as_slice)
    }

    /// Whether the last frame applied lacked a complete `CountryAppearance`,
    /// so the lists at that hour are not known.
    pub fn pending(&self) -> Option<Hour> {
        self.pending_since
    }

    /// Derives the lists at every hour of `hours` on `branch`, reading the
    /// branch from its first frame (the tables `CountryLinks` and
    /// `CountryAppearance` only).
    pub fn derive(
        replay: &replay::container::Replay,
        branch: replay::BranchId,
        hours: &std::collections::BTreeSet<Hour>,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> std::io::Result<BTreeMap<Hour, Self>> {
        let at_load = replay.branches.iter().any(|start| {
            start.id == branch && start.reason == replay::branches::BranchReason::LoadSignal
        });
        let Some(&last) = hours.last() else {
            return Ok(BTreeMap::new());
        };
        let selected = [
            replay::rows::TableId::CountryLinks,
            replay::rows::TableId::CountryAppearance,
        ];
        let mut lists: Option<Self> = None;
        let mut out = BTreeMap::new();
        let mut blocks: Vec<_> = replay
            .index
            .iter()
            .filter(|block| block.branch == branch && block.first_hour <= last)
            .collect();
        blocks.sort_by_key(|block| block.first_hour);
        for block in blocks {
            if cancelled.load(std::sync::atomic::Ordering::Acquire) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "request cancelled",
                ));
            }
            let mut decoder = replay::frame::FrameDecoder::for_tables(&selected);
            for frame in replay.read_block_selected(block, &selected)? {
                if frame.hour > last {
                    break;
                }
                let decoded = decoder.advance(&frame)?;
                match &mut lists {
                    None => lists = Some(Self::start(decoded, at_load)),
                    Some(lists) => lists.advance(decoded),
                }
                if hours.contains(&decoded.hour)
                    && let Some(lists) = &lists
                {
                    out.insert(decoded.hour, lists.clone());
                }
            }
        }
        Ok(out)
    }
}
