use super::{
    Span, DEFAULT_PROVEN_EXTENT_GAP_BYTES, GENERIC_GAP_BYTES, MAX_PROVEN_WINDOW_GAP_BYTES,
    PROVEN_OBJECT_GAP_BYTES,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AnchorKind {
    Root(u64),
    Pointer(u64),
    Mixed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Proof {
    None,
    Object { base: u64, len: u64 },
    Extent { base: u64, len: u64 },
}

impl Proof {
    pub fn region(&self) -> Option<Span> {
        match *self {
            Proof::None => None,
            Proof::Object { base, len } | Proof::Extent { base, len } => Some(Span::new(base, len)),
        }
    }

    fn same_region_as(&self, other: &Proof) -> bool {
        match (self, other) {
            (Proof::Object { base: a, len: la }, Proof::Object { base: b, len: lb }) => {
                a == b && la == lb
            }
            (Proof::Extent { base: a, len: la }, Proof::Extent { base: b, len: lb }) => {
                a == b && la == lb
            }
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Provenance {
    pub anchor: AnchorKind,
    pub proof: Proof,
}

impl Provenance {
    pub const fn new(anchor: AnchorKind, proof: Proof) -> Self {
        Self { anchor, proof }
    }

    #[cfg(test)]
    pub const fn from_pointer(value: u64) -> Self {
        Self::new(AnchorKind::Pointer(value), Proof::None)
    }

    #[cfg(test)]
    pub const fn from_root(addr: u64) -> Self {
        Self::new(AnchorKind::Root(addr), Proof::None)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyRefusal {
    NonMonotonic {
        generic: u32,
        proven_object: u32,
        proven_extent: u32,
    },
    WindowTooWide {
        configured: u32,
        cap: u32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GapPolicy {
    generic: u32,
    proven_object: u32,
    proven_extent: u32,
}

impl GapPolicy {
    pub fn new(
        generic: u32,
        proven_object: u32,
        proven_extent: u32,
    ) -> Result<Self, PolicyRefusal> {
        if generic > proven_object || proven_object > proven_extent {
            return Err(PolicyRefusal::NonMonotonic {
                generic,
                proven_object,
                proven_extent,
            });
        }
        if proven_extent > MAX_PROVEN_WINDOW_GAP_BYTES {
            return Err(PolicyRefusal::WindowTooWide {
                configured: proven_extent,
                cap: MAX_PROVEN_WINDOW_GAP_BYTES,
            });
        }
        Ok(Self {
            generic,
            proven_object,
            proven_extent,
        })
    }

    pub const fn exact() -> Self {
        Self {
            generic: 0,
            proven_object: 0,
            proven_extent: 0,
        }
    }

    pub const fn generic(&self) -> u32 {
        self.generic
    }
    pub const fn proven_object(&self) -> u32 {
        self.proven_object
    }
    pub const fn proven_extent(&self) -> u32 {
        self.proven_extent
    }
}

impl Default for GapPolicy {
    fn default() -> Self {
        Self {
            generic: GENERIC_GAP_BYTES,
            proven_object: PROVEN_OBJECT_GAP_BYTES,
            proven_extent: DEFAULT_PROVEN_EXTENT_GAP_BYTES,
        }
    }
}

pub(super) fn merge_allowance(
    a: &Provenance,
    b: &Provenance,
    merged: Span,
    policy: &GapPolicy,
) -> u32 {
    if a.proof.same_region_as(&b.proof) {
        if let Some(region) = a.proof.region() {
            if region.contains_span(&merged) {
                return match a.proof {
                    Proof::Extent { .. } => policy.proven_extent,
                    Proof::Object { .. } => policy.proven_object,
                    Proof::None => 0,
                };
            }
        }
    }
    match (a.anchor, b.anchor) {
        (AnchorKind::Pointer(x), AnchorKind::Pointer(y)) if x == y => policy.generic,
        _ => 0,
    }
}

fn merged_provenance(a: &Provenance, b: &Provenance, merged: Span) -> Provenance {
    let proof = if a.proof.same_region_as(&b.proof)
        && a.proof
            .region()
            .map(|r| r.contains_span(&merged))
            .unwrap_or(false)
    {
        a.proof
    } else {
        Proof::None
    };
    let anchor = match (a.anchor, b.anchor) {
        (AnchorKind::Pointer(x), AnchorKind::Pointer(y)) if x == y => AnchorKind::Pointer(x),
        (AnchorKind::Root(x), AnchorKind::Root(y)) if x == y => AnchorKind::Root(x),
        _ => AnchorKind::Mixed,
    };
    Provenance { anchor, proof }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CopyRequest {
    pub span: Span,
    pub provenance: Provenance,
    pub task: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PlannedCopy {
    pub span: Span,
    pub provenance: Provenance,
    pub sources_start: u32,
    pub sources_len: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct CopyPlan {
    pub copies: Vec<PlannedCopy>,
    pub sources: Vec<u32>,
    order: Vec<u32>,
    keys: Vec<SortKey>,
}

type SortKey = (u64, u64, u32);

impl CopyPlan {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.copies.len()
    }

    pub fn capacity_bytes(&self) -> u64 {
        super::arena::vec_bytes(&self.copies)
            + super::arena::vec_bytes(&self.sources)
            + super::arena::vec_bytes(&self.order)
            + super::arena::vec_bytes(&self.keys)
    }

    pub fn order(&self) -> &[u32] {
        &self.order
    }

    pub fn keys_mut(&mut self) -> &mut Vec<SortKey> {
        &mut self.keys
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.copies.is_empty()
    }

    pub fn sources_of(&self, copy: &PlannedCopy) -> &[u32] {
        let start = copy.sources_start as usize;
        &self.sources[start..start + copy.sources_len as usize]
    }

    pub fn total_bytes(&self) -> u64 {
        self.copies
            .iter()
            .fold(0u64, |acc, copy| acc.saturating_add(copy.span.len))
    }

    pub fn widest(&self) -> Option<u64> {
        self.copies.iter().map(|copy| copy.span.len).max()
    }
}

/// [`super::dedup::OwnershipMap`], which removes the duplicate copy entirely —
#[cfg(test)]
pub(super) fn plan_copies(requests: &[CopyRequest], policy: &GapPolicy) -> CopyPlan {
    let mut plan = CopyPlan::new();
    plan_copies_into(requests, policy, &mut plan);
    plan
}

pub(super) fn plan_copies_into(requests: &[CopyRequest], policy: &GapPolicy, plan: &mut CopyPlan) {
    plan.copies.clear();
    plan.sources.clear();
    plan.order.clear();
    let mut ascending = true;
    let mut previous = (0u64, 0u64);
    for index in 0..requests.len() as u32 {
        let span = requests[index as usize].span;
        if span.is_empty() {
            continue;
        }
        let key = (span.start, span.len);
        if !plan.order.is_empty() && key <= previous {
            ascending = false;
        }
        previous = key;
        plan.order.push(index);
    }
    if !ascending {
        plan.keys.clear();
        for &index in &plan.order {
            let span = requests[index as usize].span;
            plan.keys.push((span.start, span.len, !index));
        }
        plan.keys.sort_unstable();
        plan.order.clear();
        plan.order
            .extend(plan.keys.iter().map(|&(_, _, complement)| !complement));
    }

    let order = std::mem::take(&mut plan.order);
    for &index in &order {
        let req = &requests[index as usize];
        let mut merged_into_last = false;
        if let Some(last) = plan.copies.last_mut() {
            if let Some(gap) = last.span.gap_to(&req.span) {
                let hull = last.span.hull(&req.span);
                let allowance = merge_allowance(&last.provenance, &req.provenance, hull, policy);
                if gap <= u64::from(allowance) {
                    last.provenance = merged_provenance(&last.provenance, &req.provenance, hull);
                    last.span = hull;
                    last.sources_len += 1;
                    plan.sources.push(req.task);
                    merged_into_last = true;
                }
            }
        }
        if !merged_into_last {
            let sources_start = plan.sources.len() as u32;
            plan.sources.push(req.task);
            plan.copies.push(PlannedCopy {
                span: req.span,
                provenance: req.provenance,
                sources_start,
                sources_len: 1,
            });
        }
    }
    plan.order = order;
}
