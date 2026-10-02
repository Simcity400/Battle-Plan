//! Country counts from the collector's existing authenticated division walk.

use super::*;
use replay::frame::ViewIdentity;

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
pub(super) struct Census {
    mapping: Option<(ViewIdentity, u32)>,
    counts: Option<ViewIdentity>,
    membership: Option<Membership>,
}

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
        self.membership = None;
        let mut membership = Membership::default();
        let mut found = false;
        for row in rows {
            match link_slot(links, row.country) {
                Some(owner) if owner == slot => {
                    found = true;
                    membership.grouped = membership.grouped.checked_add(row.grouped)?;
                    membership.ungrouped = membership.ungrouped.checked_add(row.ungrouped)?;
                }
                None if row.grouped != 0 || row.ungrouped != 0 => membership.unresolved = true,
                _ => {}
            }
        }
        self.mapping = Some((links_view.identity(), slot));
        self.counts = Some(decoded.tables[TableId::DivisionCensus.index()].identity());
        self.membership = found.then_some(membership);
        self.membership
    }
}
