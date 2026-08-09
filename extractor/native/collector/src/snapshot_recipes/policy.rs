//! Gap-coalescing policy: WHICH unread bytes a copy descriptor may bridge.
//!
//! The re-pricing's rule is one sentence long — "never merge merely because
//! virtual addresses are close" — and the gap-curve replay measured exactly
//! what the permissive version buys
//! (`runs/s0_census/t35_gap_curve/verdict.txt`). Every row of that curve is an
//! UPPER BOUND on a safety-constrained policy, because the curve merges on
//! proximity alone and this module refuses to.
//!
//! # The merge is gated on provenance, not on distance
//!
//! [`merge_allowance`] takes two [`Provenance`] values and returns how many
//! unread bytes may be bridged between them. It is the only place in the
//! substrate that answers that question, and it cannot be asked without
//! provenance: [`CopyRequest`] has no constructor that omits one, and
//! [`plan_copies`] takes `CopyRequest`s. There is no `merge(a: Span, b: Span,
//! gap: u64)` anywhere in this module, so "these two addresses are 8 bytes
//! apart, merge them" is not a call that can be written.
//!
//! Three tiers, in decreasing strength:
//!
//! | Evidence | Allowance | Where it comes from |
//! |---|---:|---|
//! | Both requests inside the SAME decoded container extent | `proven_extent` (default 128, cap 4096) | a `DecodeExtent` op read the count and base out of already-copied bytes |
//! | Both requests inside the SAME proven object/allocation | `proven_object` (default 64) | a `CopyHeader` op declared the object's width |
//! | Both requests reached through the SAME dereferenced pointer | `generic` (default 32) | a `DecodePointer` op produced the anchor |
//! | anything else | **0** | — |
//!
//! An allowance of zero still merges touching and overlapping ranges: those
//! bridge no unread bytes at all, so they are not an over-copy and the
//! question of safety does not arise. That is the same rule
//! [`crate::range_census::coalesce`] uses for its gap-0 column.
//!
//! # Root reads are not anchors
//!
//! §4 of the gap-curve verdict is explicit: the re-pricing's per-family window
//! recipes were anchored on the census `roots` column, which counts root READ
//! EVENTS, not root addresses, and for d42/d43 the two differ by more than an
//! order of magnitude. Anchored honestly — on the pointer values the bracket
//! dereferenced — d42's 2 KiB windows reach 71% of its bytes instead of 0.27%.
//!
//! So [`AnchorKind::Root`] never satisfies the generic tier. A root address is
//! a static or global address; its neighbours are not proven to be one object,
//! and merging across them is precisely the proximity merge the policy
//! refuses. Roots merge only when a `CopyHeader` has proven the object width.

use super::{
    Span, DEFAULT_PROVEN_EXTENT_GAP_BYTES, GENERIC_GAP_BYTES, MAX_PROVEN_WINDOW_GAP_BYTES,
    PROVEN_OBJECT_GAP_BYTES,
};

/// What reached the bytes of a copy request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AnchorKind {
    /// A root address: a literal in the recipe, i.e. a static/global. Never a
    /// window anchor — see the module header.
    Root(u64),
    /// A pointer VALUE the observation dereferenced. This is the only kind of
    /// anchor a generic merge may rest on.
    Pointer(u64),
    /// The result of merging requests with different anchors. Satisfies no
    /// anchor-based tier, by construction.
    Mixed,
}

/// What is PROVEN about the region a copy request sits in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Proof {
    /// Nothing. The request is a bare window at an address.
    None,
    /// The request lies inside an object/allocation whose width the recipe
    /// declared (a `CopyHeader`).
    Object { base: u64, len: u64 },
    /// The request lies inside a container extent whose element count and base
    /// were decoded out of already-copied bytes (a `DecodeExtent`).
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

/// Where a copy request's bytes came from. Carried by every request, merged
/// conservatively, and consulted by [`merge_allowance`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Provenance {
    pub anchor: AnchorKind,
    pub proof: Proof,
}

impl Provenance {
    pub const fn new(anchor: AnchorKind, proof: Proof) -> Self {
        Self { anchor, proof }
    }

    /// A request reached from a dereferenced pointer with nothing else proven.
    ///
    /// Test-only. The executor always knows a proof as well, so it builds its
    /// provenance with [`Self::new`]; these two exist so a policy test can
    /// state the bare-anchor cases directly.
    #[cfg(test)]
    pub const fn from_pointer(value: u64) -> Self {
        Self::new(AnchorKind::Pointer(value), Proof::None)
    }

    /// A request at a root address with nothing else proven. Test-only, as
    /// above.
    #[cfg(test)]
    pub const fn from_root(addr: u64) -> Self {
        Self::new(AnchorKind::Root(addr), Proof::None)
    }
}

/// Why a policy is not usable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyRefusal {
    /// A weaker tier allowed more than a stronger one, which would let an
    /// unproven merge bridge a hole a proven one may not.
    NonMonotonic {
        generic: u32,
        proven_object: u32,
        proven_extent: u32,
    },
    /// A tier was configured past [`MAX_PROVEN_WINDOW_GAP_BYTES`].
    WindowTooWide { configured: u32, cap: u32 },
}

/// The coalescing policy. DATA, not code: it is a field of a recipe, and two
/// recipes in the same observation may carry different ones.
///
/// Fields are private so a policy can only exist having passed [`Self::new`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GapPolicy {
    generic: u32,
    proven_object: u32,
    proven_extent: u32,
}

impl GapPolicy {
    /// Build a policy, or refuse.
    ///
    /// Monotonicity is required, not merely conventional: if the generic tier
    /// allowed more than the proven-object tier, a request pair with WEAKER
    /// evidence could bridge a hole that a stronger pair could not, which
    /// makes the whole table meaningless.
    pub fn new(generic: u32, proven_object: u32, proven_extent: u32) -> Result<Self, PolicyRefusal> {
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

    /// The policy that merges nothing but touching/overlapping ranges. What a
    /// family with no proven structure should start on.
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
    /// The re-pricing's preferred starting policy: 32 B generic, 64 B inside a
    /// proven object, 128 B inside a proven extent.
    fn default() -> Self {
        Self {
            generic: GENERIC_GAP_BYTES,
            proven_object: PROVEN_OBJECT_GAP_BYTES,
            proven_extent: DEFAULT_PROVEN_EXTENT_GAP_BYTES,
        }
    }
}

/// How many unread bytes may be bridged between two requests whose merged
/// footprint would be `merged`.
///
/// Returns 0 when nothing is proven, which still permits touching/overlapping
/// merges (they bridge nothing).
///
/// A proven tier additionally requires that `merged` stays INSIDE the proven
/// region: bridging a hole that leaves the allocation is not a safe over-copy
/// however small the hole is.
pub(super) fn merge_allowance(a: &Provenance, b: &Provenance, merged: Span, policy: &GapPolicy) -> u32 {
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
        // Root anchors and mixed anchors prove nothing about their
        // neighbourhood. This arm is the "never merge merely because virtual
        // addresses are close" rule, and it is the only outcome available to a
        // caller that has no evidence to offer.
        _ => 0,
    }
}

/// The provenance a merged request carries: the strongest claim that survives
/// covering `merged`.
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

/// One span a recipe asked for, with the evidence that reached it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CopyRequest {
    pub span: Span,
    pub provenance: Provenance,
    /// Index of the discovery task that asked. Carried through so the executor
    /// can attach dependency edges to the right consumers after coalescing.
    pub task: u32,
}

/// One physical copy transaction, after coalescing.
///
/// The requests a transaction serves are a CONTIGUOUS run of
/// [`CopyPlan::sources`], not a `Vec` of its own: a per-transaction `Vec` is a
/// heap allocation per copy, and this type is instantiated once per copy
/// transaction in every wave of every observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PlannedCopy {
    pub span: Span,
    pub provenance: Provenance,
    pub sources_start: u32,
    pub sources_len: u32,
}

/// One wave's worth of coalesced copies for one recipe.
///
/// Reusable: [`plan_copies_into`] clears and refills it, so an observation
/// pays for these two buffers once rather than once per wave.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct CopyPlan {
    pub copies: Vec<PlannedCopy>,
    /// Request indices, grouped by copy and ordered by address.
    pub sources: Vec<u32>,
    /// The non-empty request indices in address order. Held here so it is
    /// allocated once, and exposed by [`CopyPlan::order`] because the
    /// executor's dependency attach needs exactly the same ordering the
    /// coalescer already paid for.
    order: Vec<u32>,
}

impl CopyPlan {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.copies.len()
    }

    /// Bytes this plan's three buffers hold, for the observation's custody
    /// accounting. Capacity, not length: they are reused across waves, so
    /// what they took from the allocator is what stays taken.
    pub fn capacity_bytes(&self) -> u64 {
        super::arena::vec_bytes(&self.copies)
            + super::arena::vec_bytes(&self.sources)
            + super::arena::vec_bytes(&self.order)
    }

    /// The indices of the requests this plan was built from, in ascending
    /// address order, empty requests excluded.
    ///
    /// A caller that has to visit every request AND wants them in address
    /// order — the executor's dependency attach is the one that does — reads
    /// this rather than sorting again.
    pub fn order(&self) -> &[u32] {
        &self.order
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.copies.is_empty()
    }

    /// The request indices `copy` serves.
    ///
    /// The executor's dependency attach walks requests rather than
    /// transactions, but the per-op qualifications a merged transaction has to
    /// honour — its tolerance, its sample count, its stage — are a join over
    /// exactly this run, so it is on the executor's path as well now.
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

/// Coalesce one recipe's requests for one wave into physical copies.
///
/// Input need not be sorted. Output is sorted by address and pairwise
/// separated by more than the applicable allowance.
///
/// COALESCING IS PER-RECIPE, not global. The policy is a per-recipe value, so
/// a merge spanning two recipes' requests would have no defined policy to
/// answer to. Cross-recipe sharing is handled downstream by
/// [`super::dedup::OwnershipMap`], which removes the duplicate copy entirely —
/// a strictly better outcome than bridging it.
#[cfg(test)]
pub(super) fn plan_copies(requests: &[CopyRequest], policy: &GapPolicy) -> CopyPlan {
    let mut plan = CopyPlan::new();
    plan_copies_into(requests, policy, &mut plan);
    plan
}

/// [`plan_copies`] into a buffer the caller owns and reuses.
pub(super) fn plan_copies_into(requests: &[CopyRequest], policy: &GapPolicy, plan: &mut CopyPlan) {
    plan.copies.clear();
    plan.sources.clear();
    plan.order.clear();
    // Requests arrive in task order, and a wave's tasks are frequently already
    // in address order (roots declared in address order, an extent's elements,
    // a fixed window list). Establishing that costs one sequential pass over
    // the requests; sorting when it is already true costs a comparison sort
    // whose every comparison is two random loads into the request array.
    let mut ascending = true;
    let mut previous = (0u64, 0u64);
    for index in 0..requests.len() as u32 {
        let span = requests[index as usize].span;
        if span.is_empty() {
            continue;
        }
        let key = (span.start, span.len);
        if !plan.order.is_empty() && key < previous {
            ascending = false;
        }
        previous = key;
        plan.order.push(index);
    }
    if !ascending {
        // NOT MATERIALISED, and that is a deliberate refusal rather than an
        // oversight. This closure is called once per COMPARISON, so at a wave
        // of tens of thousands of requests it is a million dependent loads into
        // an array wider than L2, and sorting materialised keys instead measured
        // as worth a low single-digit percent of a whole observation.
        //
        // It is not taken because `(start, len)` is not a TOTAL order over the
        // requests: two tasks can produce the identical span, and which of them
        // ends up last among the equal keys decides the `op` and `stage` a
        // coalesced copy is attributed to (`Observation::copy_attributes`), and
        // therefore what a refusal witness names. `sort_unstable` leaves that
        // permutation unspecified, so ANY re-spelling of this sort — including
        // one that only changes the element type it sorts — can change an
        // observable field for an input with equal spans. Making the order
        // total (by request index, say) would fix the attribution and would
        // also CHANGE it. That is a ruling about what the substrate promises,
        // not an optimisation, and it belongs to whoever owns the witness
        // contract.
        plan.order.sort_unstable_by_key(|&i| {
            (requests[i as usize].span.start, requests[i as usize].span.len)
        });
    }

    let order = std::mem::take(&mut plan.order);
    for &index in &order {
        let req = &requests[index as usize];
        let mut merged_into_last = false;
        if let Some(last) = plan.copies.last_mut() {
            // `gap_to` returns None only when the later span starts before the
            // earlier one, which the sort above has already ruled out.
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
