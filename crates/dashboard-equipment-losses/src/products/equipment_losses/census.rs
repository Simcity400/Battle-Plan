//! Country counts from the collector's existing authenticated division walk.

use super::*;
#[cfg(test)]
use replay_frame::frame::ViewIdentity;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Membership {
    grouped: u32,
    ungrouped: u32,
    unresolved: bool,
}

impl Membership {
    pub(super) fn coverage(self, prune_observed: bool) -> Coverage {
        if self.unresolved {
            Coverage::Unknown
        } else if self.grouped == 0 && self.ungrouped != 0 {
            Coverage::NotLogged
        } else if self.ungrouped != 0 {
            Coverage::Partial {
                ungrouped: self.ungrouped,
            }
        } else if prune_observed {
            Coverage::Logged
        } else {
            Coverage::PruneUnobserved
        }
    }
}

#[derive(Default)]
#[cfg(test)]
pub(super) struct Census {
    mapping: Option<(ViewIdentity, u32)>,
    counts: Option<ViewIdentity>,
    membership: Option<Membership>,
}

#[cfg(test)]
impl Census {
    pub(super) fn advance(
        &mut self,
        decoded: &DecodedFrame,
        slot: Option<u32>,
    ) -> Option<Membership> {
        let links_view = &decoded.tables[TableId::CountryLinks.index()];
        let (Ok(rows), Ok(links), Some(slot)) = (
            decoded.complete::<tables::DivisionCensus>(),
            decoded.complete::<tables::CountryLinks>(),
            slot,
        ) else {
            self.membership = None;
            self.mapping = None;
            return None;
        };
        if self
            .counts
            .as_ref()
            .is_some_and(|identity| identity.is(&decoded.tables[TableId::DivisionCensus.index()]))
            && self
                .mapping
                .as_ref()
                .is_some_and(|(identity, old_slot)| identity.is(links_view) && *old_slot == slot)
        {
            return self.membership;
        }
        self.membership = count_memberships(rows, links).get(&slot).copied().flatten();
        self.mapping = Some((links_view.identity(), slot));
        self.counts = Some(decoded.tables[TableId::DivisionCensus.index()].identity());
        self.membership
    }
}

/// Resolve every country's division counts once for the prepared frame.
pub(super) fn memberships(decoded: &DecodedFrame) -> Option<BTreeMap<u32, Option<Membership>>> {
    Some(count_memberships(
        decoded.complete::<tables::DivisionCensus>().ok()?,
        decoded.complete::<tables::CountryLinks>().ok()?,
    ))
}
fn count_memberships(
    rows: &[replay_data::rows::DivisionCensusRow],
    links: &[CountryLinkRow],
) -> BTreeMap<u32, Option<Membership>> {
    let mut values: BTreeMap<u32, Option<Membership>> = BTreeMap::new();
    let mut unresolved = false;
    for row in rows {
        match link_slot(links, row.country) {
            Some(slot) => {
                let membership = values.entry(slot).or_insert(Some(Membership::default()));
                *membership = membership.and_then(|mut value| {
                    value.grouped = value.grouped.checked_add(row.grouped)?;
                    value.ungrouped = value.ungrouped.checked_add(row.ungrouped)?;
                    Some(value)
                });
            }
            None if row.grouped != 0 || row.ungrouped != 0 => unresolved = true,
            _ => {}
        }
    }
    if unresolved {
        for value in values.values_mut().flatten() {
            value.unresolved = true;
        }
    }
    values
}
