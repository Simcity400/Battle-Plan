//! H4CP **v3 PROTOTYPE**: a canonical-binary fold state plus a mutation journal
//! for one top-level section of the world document.
//!
//! # Why this exists
//!
//! The v2 fold materializes a ~26 MB `serde_json::Value` world document every
//! game hour, serializes it, re-parses it into a [`CanonValue`], SHA-256s it,
//! and structurally diffs that tree against the previous hour's retained tree.
//! Every one of those five terms is proportional to the WHOLE document, while
//! the information that actually moved is a couple of megabytes. The design
//! study's stage-2 proposal is to stop rebuilding: keep each covered section's
//! state as canonical binary with stable keys, and route every change through
//! an API that simultaneously
//!
//! 1. updates the state,
//! 2. advances a monotonic subtree revision,
//! 3. emits the exact canonical cache mutation bytes, and
//! 4. updates the rolling state root.
//!
//! This is deliberately **not** dirty-mark memoization, the shape that was
//! evaluated and refused. There is no separately cached answer that can go stale
//! beside a live one, because the canonical bytes ARE the state: the only way to
//! read a section's bytes is [`SectionState::project`], which concatenates the
//! very bytes the mutations installed. A missed mutation therefore cannot
//! produce a stale-but-self-consistent answer; it produces a section that does
//! not match the oracle and a state root that does not match the replica's.
//!
//! # Grain
//!
//! A section is decomposed into GRAINS: the nodes at a configured depth, plus
//! any shallower node that cannot be descended (a scalar, an empty object, an
//! empty array). A grain retains its canonical JSON bytes, its SHA-256, and the
//! section revision at which it was installed. Everything above grain depth is
//! skeleton, and the skeleton is not stored at all -- it is implied by the
//! sorted grain paths, so there is no second structure that can disagree with
//! the first.
//!
//! A grain is addressed by a PATH, not by a single top-level key. That is
//! load-bearing: three of the six byte-dominant sections (`map`, `supply`,
//! `special_projects`) have only a handful of direct children and put their
//! megabytes inside keyed collections one or two levels down, so a
//! top-level-child grain would re-emit a multi-megabyte member for one changed
//! leaf -- structurally worse than v2's recursive diff. Grain depth is the
//! prototype's one real tuning knob and it is a parameter, not a constant,
//! precisely so the experiment can price it per section.
//!
//! # Hour boundaries
//!
//! Mutations are only reachable through [`SectionState::begin_hour`], which
//! hands out the single [`HourTxn`] that owns the section for the hour and is
//! consumed by [`HourTxn::commit`]. Dropping a transaction that made changes
//! without committing POISONS the section: every later operation fails. An hour
//! whose mutations were silently discarded is the exact shape that would make a
//! delta series and a state disagree, so it fails loudly instead.
//!
//! # What this crate is not
//!
//! It writes no sealed artifact, it is not wired into the production fold or the
//! v2 cache format, and no production binary depends on it. It exists to be
//! measured against the v2 path on real captures.

use std::collections::BTreeMap;
use std::ops::Bound::{Excluded, Included, Unbounded};

use snapshot::canon_value::{is_valid_object_key, key_cmp, CanonValue};
use snapshot::sha256::Sha256;

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

/// One step from a section root towards a grain.
///
/// `Key` carries the member name's SOURCE TOKEN BODY, exactly as
/// [`CanonValue::Object`] stores it, so projection can write it back verbatim.
/// Two bodies that DECODE to the same text are the same address -- that is what
/// [`Ord`] below says, and it is what a canonical object means by "no duplicate
/// members" -- so installing `a` where `a` stands must also replace the
/// stored spelling, which [`SectionState::install`] does explicitly.
#[derive(Debug, Clone)]
pub enum Seg {
    Key(String),
    Index(u32),
}

impl Seg {
    fn is_key(&self) -> bool {
        matches!(self, Seg::Key(_))
    }
    /// Same address, different source spelling.
    fn respells(&self, other: &Self) -> bool {
        match (self, other) {
            (Seg::Key(left), Seg::Key(right)) => left != right && key_cmp(left, right).is_eq(),
            _ => false,
        }
    }
}

/// Equality is ADDRESS equality, deliberately delegated to [`Ord`].
///
/// A derived `PartialEq` would compare the escaped spelling while `Ord` compares
/// the decoded text, and the two would disagree on `a` versus `a`. A
/// `BTreeMap` keyed on such a type is a trap: it looks up by `Ord` and every
/// other caller reasons by `==`, so one address would be two keys to half the
/// code and one key to the other half.
impl PartialEq for Seg {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}

impl Eq for Seg {}

impl Ord for Seg {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            // Member names order by their DECODED text -- the order
            // `serde_json` writes a `BTreeMap` in, and the order `CanonValue`
            // requires of a canonical object. Ordering by the escaped spelling
            // instead would put `a` after `b`.
            (Seg::Key(left), Seg::Key(right)) => key_cmp(left, right),
            (Seg::Index(left), Seg::Index(right)) => left.cmp(right),
            // Never compared in a well-formed document: two siblings of one
            // container are both names or both ordinals. Still a total order.
            (Seg::Key(_), Seg::Index(_)) => std::cmp::Ordering::Less,
            (Seg::Index(_), Seg::Key(_)) => std::cmp::Ordering::Greater,
        }
    }
}

impl PartialOrd for Seg {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// A grain's address within its section. The empty path addresses the whole
/// section, which is how a section spelled as a scalar (`"divisions":null` on an
/// hour the domain was unknown) is represented.
pub type Path = Vec<Seg>;

/// Whether two paths are the same ADDRESS but not the same SPELLING.
///
/// Only reachable with an escaped member name, which no capture in a 16 GB
/// census has produced -- and which the projection would still get wrong if this
/// were not handled, because it writes the spelling it stored.
fn same_address_other_spelling(left: &[Seg], right: &[Seg]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(a, b)| a.cmp(b).is_eq())
        && left.iter().zip(right).any(|(a, b)| a.respells(b))
}

/// Whether two addresses are WRITTEN the same way, not merely the same address.
///
/// `==` on a path answers the address question, because that is what a canonical
/// object means by a member name. A caller that cares which bytes the projection
/// will emit -- anyone deciding whether a grain needs reinstalling -- has to ask
/// this one instead.
pub fn same_spelling(left: &[Seg], right: &[Seg]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(a, b)| match (a, b) {
            (Seg::Key(left), Seg::Key(right)) => left == right,
            (Seg::Index(left), Seg::Index(right)) => left == right,
            _ => false,
        })
}

fn encode_path_into(path: &[Seg], out: &mut Vec<u8>) {
    write_varint(out, path.len() as u64);
    for seg in path {
        match seg {
            Seg::Key(name) => {
                out.push(SEG_KEY);
                write_varint(out, name.len() as u64);
                out.extend_from_slice(name.as_bytes());
            }
            Seg::Index(index) => {
                out.push(SEG_INDEX);
                write_varint(out, u64::from(*index));
            }
        }
    }
}

const SEG_KEY: u8 = 0;
const SEG_INDEX: u8 = 1;

// ---------------------------------------------------------------------------
// Section shape and root kind
// ---------------------------------------------------------------------------

/// How a section's ROOT container is spelled when it is present, and how its
/// direct children are addressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shape {
    /// `{"k":v,...}` -- direct children addressed by member name.
    Object,
    /// `[v,...]` -- direct children addressed by ordinal position.
    ///
    /// Positional addressing is what makes v2's array diff collapse to a whole
    /// array replacement the moment one element is inserted or removed, so this
    /// shape exists for completeness and is not what `divisions` uses.
    Array,
    /// `[v,...]` where every element is an object carrying a stable identity in
    /// `key_member`, and the array's order is that identity ascending.
    ///
    /// This is the grain the design study flags as missing: `divisions` is
    /// spelled as an array but is PRODUCED from a `BTreeMap<uid, object>`, so
    /// its elements do have stable keys and its order is exactly those keys
    /// ascending. Addressing by the key instead of the ordinal is what stops an
    /// insertion in the middle from re-emitting every element after it.
    ///
    /// The order that must be reproduced is Rust `String` order over the UID's
    /// exact spelling, which is NOT numeric order: `10:1` precedes `2:1`.
    KeyedArray { key_member: String },
}

impl Shape {
    /// The root kind this shape declares when the section IS present as a
    /// container. A section can still be a [`RootKind::Scalar`] on an hour the
    /// fold spelled it `null`.
    pub fn declared_root_kind(&self) -> RootKind {
        self.root_kind()
    }
    fn root_kind(&self) -> RootKind {
        match self {
            Shape::Object => RootKind::Object,
            Shape::Array | Shape::KeyedArray { .. } => RootKind::Array,
        }
    }
}

/// What the section's root IS this hour.
///
/// A section is not always its shape: the fold spells `divisions` and
/// `special_projects` as `null` on an hour their domain is unknown. A state that
/// could only hold containers would have to represent that as "no children",
/// which is a different document and a different meaning. The kind is part of
/// the state, it is journalled when it changes, and it is bound into the state
/// root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootKind {
    /// The section holds one scalar (or `null`) at the empty path.
    Scalar,
    Object,
    Array,
}

impl RootKind {
    fn tag(self) -> u8 {
        match self {
            RootKind::Scalar => 0,
            RootKind::Object => 1,
            RootKind::Array => 2,
        }
    }
    fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            0 => Some(RootKind::Scalar),
            1 => Some(RootKind::Object),
            2 => Some(RootKind::Array),
            _ => None,
        }
    }
    fn open(self) -> u8 {
        match self {
            RootKind::Object => b'{',
            _ => b'[',
        }
    }
    fn close(self) -> u8 {
        match self {
            RootKind::Object => b'}',
            _ => b']',
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionSpec {
    pub name: String,
    pub shape: Shape,
    /// Depth at which grains are cut. Depth 1 makes each direct child of the
    /// section root one grain.
    pub grain_depth: usize,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldV3Error(String);

impl FoldV3Error {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for FoldV3Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for FoldV3Error {}

// ---------------------------------------------------------------------------
// Grain
// ---------------------------------------------------------------------------

/// One unit of state. The bytes are canonical and authoritative: `project`
/// writes them out verbatim, so nothing can hold a "current" value that
/// disagrees with what a reader would see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grain {
    bytes: Vec<u8>,
    digest: [u8; 32],
    rev: u64,
}

impl Grain {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn digest(&self) -> &[u8; 32] {
        &self.digest
    }
    /// The section revision at which these bytes were installed.
    ///
    /// Revisions come from a section-global counter rather than a per-grain one
    /// so that a delete followed by a reinsertion of the same key still advances:
    /// a per-child counter dies with the child, and the reinserted child would
    /// restart at a revision the key had already used.
    pub fn rev(&self) -> u64 {
        self.rev
    }
}

// ---------------------------------------------------------------------------
// State root accumulator
// ---------------------------------------------------------------------------

/// An additive multiset accumulator over `SHA-256(0x01 || path || 0x00 || grain
/// digest)`.
///
/// A state root has to be updatable in O(1) per changed grain, because the whole
/// point of v3 is that no per-hour term may be proportional to the section's
/// size. A Merkle tree over the sorted grains is O(n) per insertion (every
/// ordinal after the insertion point moves); a bucketed Merkle is O(n/buckets)
/// and needs an auxiliary index that can itself drift. Adding each grain's
/// contribution into a 256-bit register and subtracting it again when the grain
/// changes is O(1) and needs no auxiliary structure at all.
///
/// It is CONTENT-addressed: two states holding the same grains have the same
/// root whatever order the mutations arrived in. That is exactly what this
/// prototype needs it for -- a stale subtree, a dropped mutation and a
/// duplicated one all change the content -- and it is why the replica check is
/// meaningful. It is deliberately NOT a commitment against an adversary who may
/// choose grain digests: a multiset sum is malleable by construction. Nothing
/// here is a security boundary; the sealed artifact's SHA-256 remains the
/// integrity contract.
///
/// The hash is SHA-256 throughout, never a `DefaultHasher`: a randomly seeded
/// hash would make the root differ between processes and quietly disable every
/// cross-process comparison built on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Accumulator([u64; 4]);

impl Accumulator {
    fn contribution(path: &[Seg], digest: &[u8; 32]) -> [u64; 4] {
        let mut encoded = Vec::with_capacity(64);
        // Domain separation and length prefixes, so no two different
        // (path, digest) pairs can present the same byte sequence.
        encoded.push(0x01);
        encode_path_into(path, &mut encoded);
        encoded.push(0x00);
        encoded.extend_from_slice(digest);
        let bytes = Sha256::digest(&encoded);
        let mut limbs = [0u64; 4];
        for (index, limb) in limbs.iter_mut().enumerate() {
            let mut word = [0u8; 8];
            word.copy_from_slice(&bytes[index * 8..index * 8 + 8]);
            *limb = u64::from_le_bytes(word);
        }
        limbs
    }

    fn add(&mut self, path: &[Seg], digest: &[u8; 32]) {
        let limbs = Self::contribution(path, digest);
        for (slot, value) in self.0.iter_mut().zip(limbs) {
            *slot = slot.wrapping_add(value);
        }
    }

    fn remove(&mut self, path: &[Seg], digest: &[u8; 32]) {
        let limbs = Self::contribution(path, digest);
        for (slot, value) in self.0.iter_mut().zip(limbs) {
            *slot = slot.wrapping_sub(value);
        }
    }

    fn root(&self, kind: RootKind) -> [u8; 32] {
        let mut bytes = [0u8; 33];
        bytes[0] = kind.tag();
        for (index, limb) in self.0.iter().enumerate() {
            bytes[1 + index * 8..1 + index * 8 + 8].copy_from_slice(&limb.to_le_bytes());
        }
        Sha256::digest(&bytes)
    }
}

// ---------------------------------------------------------------------------
// Journal
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Op {
    Upsert(Vec<u8>),
    Remove,
}

pub const OP_REMOVE: u8 = 0;
pub const OP_UPSERT: u8 = 1;
/// Sets the root kind. At most one per delta, and it is always the first record.
pub const OP_ROOT: u8 = 2;

/// One hour's committed mutations for one section, canonically encoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionDelta {
    pub section: String,
    pub bytes: Vec<u8>,
    pub mutations: u64,
    /// The state root the section reached AFTER this delta. A replica that
    /// applies `bytes` to the previous state must arrive at exactly this.
    pub state_root: [u8; 32],
}

// ---------------------------------------------------------------------------
// Per-hour counters
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MutationStats {
    /// API calls that installed bytes.
    pub upserts: u64,
    pub removes: u64,
    /// API calls that found the grain already equal and therefore changed
    /// nothing. Real reducers make these constantly; counting them is how the
    /// driver can say whether it is exercising the API the way a reducer would.
    pub no_ops: u64,
    /// Root-kind changes.
    pub root_changes: u64,
    /// Canonical bytes of every grain value installed this hour, BEFORE
    /// coalescing. Against the committed delta size this is the cost of writing
    /// one grain repeatedly within an hour.
    pub grain_bytes_written: u64,
    /// API calls made, including the ones that changed nothing.
    pub calls: u64,
}

// ---------------------------------------------------------------------------
// Section state
// ---------------------------------------------------------------------------

pub struct SectionState {
    spec: SectionSpec,
    kind: RootKind,
    /// The whole of the state. Private, with no `&mut` accessor, no mutable
    /// iterator, no `entry`, and no `IndexMut`: the transaction is the only way
    /// in, which is what makes "every change is journalled" a property of the
    /// type rather than of the caller's discipline.
    grains: BTreeMap<Path, Grain>,
    accumulator: Accumulator,
    /// What every address touched this hour held when the hour OPENED: its
    /// stored spelling and digest, or `None` if nothing stood there.
    ///
    /// The delta is derived from this against the current state at commit, not
    /// accumulated as a list of operations. Accumulating operations makes the
    /// delta a record of what the producer DID, and a replica needs a record of
    /// what CHANGED. The two differ whenever an hour doubles back: a grain
    /// inserted and then removed inside one hour would emit a removal the
    /// replica never held, and a grain removed and then restored would emit an
    /// upsert that installs the bytes already there -- both of which the strict
    /// decoder rejects, correctly. Netting against the hour's starting state
    /// makes those hours emit nothing, which is what happened to them.
    hour_baseline: BTreeMap<Path, Option<(Path, [u8; 32])>>,
    /// What the root kind was when the hour opened, recorded on first change.
    hour_root_baseline: Option<RootKind>,
    revision: u64,
    stats: MutationStats,
    /// Set when a transaction with pending changes was dropped instead of
    /// committed, or when a revision counter overflowed. A poisoned section
    /// refuses every later operation rather than serving a state whose journal
    /// no longer describes it.
    poison: Option<String>,
}

impl SectionState {
    /// A new, empty section. `Object`/`Array` shapes start as their empty
    /// container; the first assignment fixes the real kind.
    pub fn new(spec: SectionSpec) -> Self {
        let kind = spec.shape.root_kind();
        Self {
            spec,
            kind,
            grains: BTreeMap::new(),
            accumulator: Accumulator::default(),
            hour_baseline: BTreeMap::new(),
            hour_root_baseline: None,
            revision: 0,
            stats: MutationStats::default(),
            poison: None,
        }
    }

    pub fn spec(&self) -> &SectionSpec {
        &self.spec
    }
    pub fn root_kind(&self) -> RootKind {
        self.kind
    }
    pub fn grain_count(&self) -> usize {
        self.grains.len()
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn stats(&self) -> MutationStats {
        self.stats
    }
    pub fn take_stats(&mut self) -> MutationStats {
        std::mem::take(&mut self.stats)
    }
    pub fn poisoned(&self) -> Option<&str> {
        self.poison.as_deref()
    }

    /// Poison the section AND empty it.
    ///
    /// A flag alone is not enough. `project`, `state_root` and `grain` are reads
    /// on the hot path and returning a `Result` from all three would put a
    /// `?` on every call site to describe a state that must never be consulted
    /// anyway. Emptying is the enforcement: a poisoned section holds nothing, so
    /// there is no partial state left to serve, and every read of it is
    /// obviously and loudly wrong rather than subtly and quietly wrong.
    fn poison_and_empty(&mut self, reason: String) {
        self.poison = Some(reason);
        self.grains.clear();
        self.accumulator = Accumulator::default();
        self.hour_baseline.clear();
        self.hour_root_baseline = None;
    }
    pub fn grain(&self, path: &[Seg]) -> Option<&Grain> {
        self.grains.get(path)
    }

    /// The grain at `path` together with the address it is STORED under.
    ///
    /// The two can differ: an address is its decoded text, and a caller that
    /// looked one up under a different escaped spelling would otherwise conclude
    /// that nothing had changed while the projection kept writing the old
    /// spelling.
    pub fn grain_entry(&self, path: &[Seg]) -> Option<(&Path, &Grain)> {
        self.grains.get_key_value(path)
    }

    /// Every address the state holds, in canonical order. Read-only: a caller
    /// that wants to change one has to go through a transaction.
    pub fn grain_paths(&self) -> impl Iterator<Item = &Path> {
        self.grains.keys()
    }

    /// Bytes of grain payload retained. The state's own accounting, so the
    /// memory question can be answered without trusting a process-level number.
    pub fn retained_grain_bytes(&self) -> u64 {
        self.grains.values().map(|g| g.bytes.len() as u64).sum()
    }

    /// Bytes of grain ADDRESSES retained -- the paths' own heap.
    pub fn retained_path_bytes(&self) -> u64 {
        self.grains
            .keys()
            .map(|path| {
                path.iter()
                    .map(|seg| match seg {
                        Seg::Key(name) => name.len() as u64 + 24,
                        Seg::Index(_) => 24,
                    })
                    .sum::<u64>()
            })
            .sum()
    }

    /// A commitment to the section's CONTENT: its root kind and the set of
    /// (address, grain digest) pairs.
    ///
    /// Deliberately not a commitment to the section's HISTORY. A section taken
    /// from `1` to `2` and back to `1` returns to the root it started from, even
    /// though its revisions have moved. That is the right property for what this
    /// root is used for -- deciding whether a replica driven only by the deltas
    /// holds the same state as the producer, which is a question about content --
    /// and it is the wrong property for anyone wanting to prove which sequence of
    /// deltas produced a state. Nothing here asks that second question, and
    /// nothing should start without adding the revisions to the hash.
    pub fn state_root(&self) -> [u8; 32] {
        self.accumulator.root(self.kind)
    }

    /// Recompute the state root from the grains, ignoring the accumulator.
    ///
    /// The incremental root is the whole point and therefore the thing most
    /// worth disbelieving: an add that is not matched by its subtraction leaves a
    /// root that is stable, self-consistent and wrong. This is the reference the
    /// driver checks it against, outside every timed region.
    pub fn recomputed_state_root(&self) -> [u8; 32] {
        let mut accumulator = Accumulator::default();
        for (path, grain) in &self.grains {
            accumulator.add(path, &grain.digest);
        }
        accumulator.root(self.kind)
    }

    /// Open the hour. The returned transaction is the only handle that can
    /// mutate this section, and it must be committed.
    pub fn begin_hour(&mut self) -> Result<HourTxn<'_>, FoldV3Error> {
        if let Some(reason) = &self.poison {
            return Err(FoldV3Error::new(format!("section is poisoned: {reason}")));
        }
        if !self.hour_baseline.is_empty() || self.hour_root_baseline.is_some() {
            return Err(FoldV3Error::new(
                "an hour is already open on this section",
            ));
        }
        Ok(HourTxn {
            state: self,
            committed: false,
        })
    }

    // -- the mutation primitives; reachable only through `HourTxn` -----------

    fn bump(&mut self) -> Result<u64, FoldV3Error> {
        self.revision = self.revision.checked_add(1).ok_or_else(|| {
            let reason = "section revision counter overflowed".to_string();
            self.poison = Some(reason.clone());
            FoldV3Error::new(reason)
        })?;
        Ok(self.revision)
    }

    /// Refuse a path the projection could not write back.
    ///
    /// `project` reconstructs the skeleton from the grain paths alone, so the
    /// path SET is the document's structure and not merely an index into it.
    /// Three shapes have no JSON to project to, and the mutation API is where
    /// they have to be stopped, because by the time `project` sees them the
    /// state is already wrong:
    ///
    /// * a member name that is not a valid JSON string body -- it is written
    ///   between quotes verbatim, so an unescaped quote in a key would emit
    ///   something no parser accepts;
    /// * a path that is a strict prefix of an existing grain's, or has one as a
    ///   strict prefix -- that is a value and a container at one address, which
    ///   projects as a duplicate member;
    /// * a sibling addressed by an ordinal where its neighbours are addressed by
    ///   name, or the reverse -- one container cannot be both.
    ///
    /// The empty path is the whole section, which only a scalar root has.
    fn check_path(&self, path: &[Seg]) -> Result<(), FoldV3Error> {
        if path.is_empty() {
            if self.kind != RootKind::Scalar {
                return Err(FoldV3Error::new(
                    "the empty path addresses the whole section and only a scalar root has one",
                ));
            }
            return Ok(());
        }
        if self.kind == RootKind::Scalar {
            return Err(FoldV3Error::new(
                "a scalar-rooted section has no children to address",
            ));
        }
        // The ROOT's addressing mode comes from the section's shape, not from
        // the segment. Everywhere below the root the container's kind is read
        // off the address that reaches into it, so it cannot disagree; the root
        // has no such address and would otherwise accept an ordinal under an
        // object and project `{1}`.
        match (&self.spec.shape, &path[0]) {
            (Shape::Object, Seg::Key(_)) => {}
            (Shape::KeyedArray { .. }, Seg::Key(_)) => {}
            (Shape::Array, Seg::Index(_)) => {}
            (Shape::Object, Seg::Index(_)) => {
                return Err(FoldV3Error::new(
                    "an object section addresses its children by name, not by ordinal",
                ))
            }
            (Shape::KeyedArray { key_member }, Seg::Index(_)) => {
                return Err(FoldV3Error::new(format!(
                    "a keyed-array section addresses its children by `{key_member}`, not by ordinal"
                )))
            }
            (Shape::Array, Seg::Key(_)) => {
                return Err(FoldV3Error::new(
                    "an array section addresses its children by ordinal, not by name",
                ))
            }
        }
        for seg in path {
            if let Seg::Key(name) = seg {
                if !is_valid_object_key(name) {
                    return Err(FoldV3Error::new(format!(
                        "path segment {name:?} is not a valid JSON member name body"
                    )));
                }
            }
        }
        // An ancestor holding a grain would be a value where this path needs a
        // container.
        for depth in 1..path.len() {
            if self.grains.contains_key(&path[..depth]) {
                return Err(FoldV3Error::new(
                    "a grain already stands at an ancestor of this path",
                ));
            }
        }
        // And a descendant would be a container where this path needs a value.
        //
        // The ranges borrow `path` as a slice rather than cloning it into a
        // `Vec`. This runs on every installed grain, and the corpus installs
        // ~18,000 an hour, so two allocations here would be two allocations
        // there -- and they would land inside the very region this prototype
        // exists to price.
        if let Some((candidate, _)) = self.grains.range::<[Seg], _>((Included(path), Unbounded)).next()
        {
            if candidate.len() > path.len() && candidate[..path.len()] == *path {
                return Err(FoldV3Error::new(
                    "a grain already stands beneath this path",
                ));
            }
        }
        // Siblings must agree on how their container addresses them, and
        // checking the two immediate NEIGHBOURS is enough to know it.
        //
        // Every path sharing a prefix occupies one contiguous run of the map,
        // because the order is lexicographic. Within such a run, `Seg::cmp` puts
        // every name before every ordinal, so a container addressed both ways
        // would have its names and its ordinals in two adjacent blocks -- and an
        // incoming path lands either at that boundary or with a neighbour that
        // is already on the wrong side of it. A disagreement therefore cannot
        // hide several entries away while both neighbours agree.
        let mut neighbours = self
            .grains
            .range::<[Seg], _>((Unbounded, Excluded(path)))
            .next_back()
            .into_iter()
            .map(|(candidate, _)| candidate)
            .chain(
                self.grains
                    .range::<[Seg], _>((Included(path), Unbounded))
                    .next()
                    .map(|(candidate, _)| candidate),
            );
        neighbours.try_for_each(|candidate| {
            let shared = candidate
                .iter()
                .zip(path)
                .take_while(|(left, right)| left.cmp(right).is_eq())
                .count();
            match (candidate.get(shared), path.get(shared)) {
                (Some(left), Some(right)) if left.is_key() != right.is_key() => Err(
                    FoldV3Error::new("two siblings of one container disagree on how it addresses them"),
                ),
                _ => Ok(()),
            }
        })
    }

    /// Record what an address held when this hour opened, once.
    fn remember(&mut self, path: &[Seg]) {
        if self.hour_baseline.contains_key(path) {
            return;
        }
        let before = self
            .grains
            .get_key_value(path)
            .map(|(stored, grain)| (stored.clone(), grain.digest));
        self.hour_baseline.insert(path.to_vec(), before);
    }

    fn install(&mut self, path: Path, bytes: Vec<u8>) -> Result<bool, FoldV3Error> {
        self.stats.calls += 1;
        self.check_path(&path)?;
        let digest = Sha256::digest(&bytes);
        // What has to be retracted from the state root, addressed by the
        // spelling it was ADDED under. Retracting under the incoming spelling
        // would subtract a different contribution and leave the root silently
        // wrong, because the spelling is inside the hash.
        let mut retract: Option<(Path, [u8; 32])> = None;
        let mut rekey = false;
        if let Some((stored_path, existing)) = self.grains.get_key_value(&path) {
            rekey = same_address_other_spelling(stored_path, &path);
            // A digest match is not a byte match. Comparing the bytes as well
            // costs one memcmp over the changed grain and removes the one way
            // this design could serve a wrong document while every check agreed.
            if existing.digest == digest && existing.bytes == bytes && !rekey {
                self.stats.no_ops += 1;
                return Ok(false);
            }
            retract = Some((stored_path.clone(), existing.digest));
        }
        // Advance the counter BEFORE the state root moves, so an overflow
        // poisons a state whose root is still the root of its grains.
        let revision = self.bump()?;
        self.remember(&path);
        if let Some((stored_path, previous_digest)) = &retract {
            self.accumulator.remove(stored_path, previous_digest);
        }
        self.stats.upserts += 1;
        self.stats.grain_bytes_written += bytes.len() as u64;
        self.accumulator.add(&path, &digest);
        if rekey {
            // `BTreeMap::insert` keeps the ORIGINAL key when the new one
            // compares equal, so an escaped respelling has to be removed before
            // it can be replaced, or the projection would keep writing the old
            // spelling. The DELTA carries whatever spelling stands at commit,
            // because it is derived from the state and not accumulated.
            self.grains.remove(&path);
        }
        self.grains.insert(
            path,
            Grain {
                bytes,
                digest,
                rev: revision,
            },
        );
        Ok(true)
    }

    fn erase(&mut self, path: &[Seg]) -> Result<bool, FoldV3Error> {
        self.stats.calls += 1;
        if !self.grains.contains_key(path) {
            self.stats.no_ops += 1;
            return Ok(false);
        }
        // BEFORE the grain goes: the hour's baseline is what stood here when the
        // hour opened, and a baseline taken after the removal would record that
        // nothing did.
        self.remember(path);
        // `remove_entry` rather than `remove`: the accumulator has to retract the
        // contribution under the STORED spelling, which need not be the one the
        // caller looked it up with.
        let (stored_path, existing) = self
            .grains
            .remove_entry(path)
            .expect("the grain was there a statement ago");
        if let Err(error) = self.bump() {
            // Put it back: a poisoned section still has to be a state whose root
            // describes its grains.
            self.grains.insert(stored_path, existing);
            return Err(error);
        }
        self.stats.removes += 1;
        self.accumulator.remove(&stored_path, &existing.digest);
        Ok(true)
    }

    fn set_kind(&mut self, kind: RootKind) -> Result<bool, FoldV3Error> {
        if self.kind == kind {
            return Ok(false);
        }
        // A root kind and the grains under it have to agree, and the only
        // moment that can be checked is here: `project` reads the kind to decide
        // whether it is writing a scalar or a container, so a kind that
        // disagrees with the grains produces a document that silently omits
        // them while the state root still commits them.
        match kind {
            RootKind::Scalar if self.grains.keys().any(|path| !path.is_empty()) => {
                return Err(FoldV3Error::new(
                    "a section cannot become a scalar while it still holds children",
                ))
            }
            RootKind::Object | RootKind::Array if self.grains.contains_key(&Vec::new()) => {
                return Err(FoldV3Error::new(
                    "a section cannot become a container while it still holds a whole-section value",
                ))
            }
            _ => {}
        }
        self.bump()?;
        self.stats.root_changes += 1;
        if self.hour_root_baseline.is_none() {
            self.hour_root_baseline = Some(self.kind);
        }
        self.kind = kind;
        Ok(true)
    }

    /// Write the section back as canonical JSON.
    ///
    /// Pure concatenation: no value is re-serialized, because the grains already
    /// hold the exact bytes that were installed. The skeleton above grain depth
    /// is reconstructed from the sorted paths, so it cannot drift from the
    /// grains it brackets.
    pub fn project(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.retained_grain_bytes() as usize + 1024);
        self.project_into(&mut out);
        out
    }

    pub fn project_into(&self, out: &mut Vec<u8>) {
        if self.kind == RootKind::Scalar {
            match self.grains.get(&Vec::new()) {
                Some(grain) => out.extend_from_slice(&grain.bytes),
                // A scalar root with no grain is a state nothing ever assigned.
                None => out.extend_from_slice(b"null"),
            }
            return;
        }
        out.push(self.kind.open());
        // Entering segment and closing byte of every container open below the
        // root, outermost first.
        let mut stack: Vec<(Seg, u8)> = Vec::new();
        let mut first = true;
        for (path, grain) in &self.grains {
            if path.is_empty() {
                // Unreachable: `check_path` refuses the empty path under a
                // container root, and `decompose` refuses to produce one, so a
                // container root's grains all have a name or an ordinal. Skipping
                // rather than asserting because a projection is a read and a read
                // should not be the thing that aborts.
                continue;
            }
            let parent = &path[..path.len() - 1];
            let mut common = 0usize;
            while common < stack.len() && common < parent.len() && stack[common].0 == parent[common]
            {
                common += 1;
            }
            while stack.len() > common {
                let (_, close) = stack.pop().expect("stack is longer than the common prefix");
                out.push(close);
            }
            if !first {
                out.push(b',');
            }
            for depth in common..parent.len() {
                let named = self.container_is_object(&stack, depth);
                write_member_prefix(out, &parent[depth], named);
                // A container's kind is the kind of the address that reaches
                // INTO it.
                let inner = if depth + 1 < parent.len() {
                    &parent[depth + 1]
                } else {
                    &path[path.len() - 1]
                };
                let (open, close) = if inner.is_key() {
                    (b'{', b'}')
                } else {
                    (b'[', b']')
                };
                out.push(open);
                stack.push((parent[depth].clone(), close));
            }
            let named = self.container_is_object(&stack, path.len() - 1);
            write_member_prefix(out, &path[path.len() - 1], named);
            out.extend_from_slice(&grain.bytes);
            first = false;
        }
        while let Some((_, close)) = stack.pop() {
            out.push(close);
        }
        out.push(self.kind.close());
    }

    /// Does the container that a segment at `depth` sits in write `"name":`
    /// prefixes?
    ///
    /// At depth 0 that is the section root, and a `KeyedArray` root is the case
    /// the answer is not obvious for: its children are ADDRESSED by name and
    /// WRITTEN without one, because the identity already lives inside each
    /// element.
    fn container_is_object(&self, stack: &[(Seg, u8)], depth: usize) -> bool {
        if depth == 0 {
            matches!(self.spec.shape, Shape::Object)
        } else {
            stack[depth - 1].1 == b'}'
        }
    }

    /// Apply an encoded delta produced by [`HourTxn::commit`].
    ///
    /// This is the REPLICA side of the equivalence: an independent state that
    /// has only ever seen the committed bytes must reach the same state root as
    /// the state that produced them. It is strict on purpose --
    ///
    /// * every removal precedes every upsert, and paths ascend strictly within
    ///   each of those two phases, so a reordered or duplicated record is
    ///   rejected rather than silently absorbed by a last-write-wins map;
    /// * `OP_ROOT`, if present, must be the first record and appear once;
    /// * a removal of a grain that is not there is rejected, because the encoder
    ///   only ever emits a removal for a grain it held;
    /// * an upsert that does not change its target is rejected, because the
    ///   encoder never emits one.
    ///
    /// Without those, a dropped or reordered mutation could still land on a
    /// state that happens to agree, and the mutation tests would not be able to
    /// fail.
    pub fn apply_delta(&mut self, bytes: &[u8]) -> Result<u64, FoldV3Error> {
        if let Some(reason) = &self.poison {
            return Err(FoldV3Error::new(format!("section is poisoned: {reason}")));
        }
        // A delta is applied record by record, so a rejection part-way through
        // leaves a state that is neither the old one nor the new one. Rolling
        // back would mean keeping an undo log of an arbitrarily large delta; the
        // honest alternative is to refuse to serve the state at all. A caller
        // that wants to survive a bad delta reopens from a checkpoint, which is
        // what a reader does anyway.
        let outcome = self.apply_delta_inner(bytes);
        if let Err(error) = &outcome {
            self.poison_and_empty(format!("a delta was rejected part-way through: {error}"));
        }
        outcome
    }

    fn apply_delta_inner(&mut self, bytes: &[u8]) -> Result<u64, FoldV3Error> {
        let mut reader = ByteReader::new(bytes);
        let mut applied = 0u64;
        let mut previous: Option<Path> = None;
        // A canonical delta has exactly three phases in this order: removals,
        // at most one root-kind change, upserts. The order is not a convention
        // -- it is the only order that is always applicable, because a grain and
        // its ancestor cannot stand at once and a root kind cannot contradict
        // its grains. Paths ascend strictly within each path phase, and the
        // ordering restarts at the crossing.
        const PHASE_REMOVE: u8 = 0;
        const PHASE_ROOT: u8 = 1;
        const PHASE_UPSERT: u8 = 2;
        let mut phase = PHASE_REMOVE;
        while !reader.is_empty() {
            let op = reader.u8()?;
            if op == OP_ROOT {
                if phase != PHASE_REMOVE {
                    return Err(FoldV3Error::new(
                        "a delta's root-kind record must follow its removals and precede its \
                         upserts, and may appear once",
                    ));
                }
                phase = PHASE_ROOT;
                let kind = RootKind::from_tag(reader.u8()?)
                    .ok_or_else(|| FoldV3Error::new("unknown root kind"))?;
                if !self.set_kind(kind)? {
                    return Err(FoldV3Error::new(
                        "delta sets the root kind it already holds",
                    ));
                }
                applied += 1;
                continue;
            }
            match op {
                OP_REMOVE if phase != PHASE_REMOVE => {
                    return Err(FoldV3Error::new(
                        "a delta's removals must all precede its root-kind record and its upserts",
                    ))
                }
                OP_UPSERT if phase != PHASE_UPSERT => {
                    phase = PHASE_UPSERT;
                    previous = None;
                }
                _ => {}
            }
            let path = reader.path()?;
            if let Some(previous) = &previous {
                if path.as_slice() <= previous.as_slice() {
                    return Err(FoldV3Error::new(
                        "delta records are not in strictly increasing path order",
                    ));
                }
            }
            match op {
                OP_REMOVE => {
                    if !self.erase(&path)? {
                        return Err(FoldV3Error::new(
                            "delta removes a grain the state does not hold",
                        ));
                    }
                }
                OP_UPSERT => {
                    let len = reader.varint()? as usize;
                    let value = reader.take(len)?.to_vec();
                    // A delta is untrusted input -- it is the file a reader will
                    // follow -- so its bodies are parsed before they are
                    // installed. The producer side skips this because it
                    // serialized the bytes itself.
                    CanonValue::parse(&value).map_err(|error| {
                        FoldV3Error::new(format!("delta grain body is not canonical: {error}"))
                    })?;
                    if !self.install(path.clone(), value)? {
                        return Err(FoldV3Error::new(
                            "delta upserts a grain that already holds those bytes",
                        ));
                    }
                }
                other => return Err(FoldV3Error::new(format!("unknown delta opcode {other}"))),
            }
            applied += 1;
            previous = Some(path);
        }
        // What the replica accrued while applying is not a delta of its own.
        self.hour_baseline.clear();
        self.hour_root_baseline = None;
        self.stats = MutationStats::default();
        Ok(applied)
    }
}

/// The hour's exclusive handle on a section.
///
/// Every mutation entry point lives here and nowhere else, and the handle is
/// consumed by [`Self::commit`]. That is what closes the "mutate after commit",
/// "commit twice" and "drop an hour's changes on the floor" bypasses: the first
/// two do not typecheck, and the third poisons the section.
pub struct HourTxn<'a> {
    state: &'a mut SectionState,
    committed: bool,
}

impl HourTxn<'_> {
    /// Install `value` at `path`.
    ///
    /// Serializes the value canonically, hashes it, compares BOTH the digest and
    /// the bytes against what stands there, and -- only if something actually
    /// moved -- replaces the bytes, advances the section revision, retracts the
    /// old contribution from the state root and adds the new one, and journals
    /// the exact bytes a reader will be given. All of that happens here or none
    /// of it does; there is no path that updates one and not the others.
    pub fn upsert(&mut self, path: Path, value: &CanonValue) -> Result<bool, FoldV3Error> {
        let mut bytes = Vec::with_capacity(64);
        value.write_json(&mut bytes);
        self.state.install(path, bytes)
    }

    /// [`Self::upsert`] for a caller that already holds canonical bytes.
    ///
    /// The bytes are validated as canonical JSON before they are installed: an
    /// unchecked `&[u8]` is not "canonical bytes" by type, and a state that
    /// accepted malformed or non-canonically-ordered bytes would project a
    /// document no reader could parse while every digest agreed.
    pub fn upsert_bytes(&mut self, path: Path, bytes: Vec<u8>) -> Result<bool, FoldV3Error> {
        CanonValue::parse(&bytes).map_err(|error| {
            FoldV3Error::new(format!("grain bytes are not canonical JSON: {error}"))
        })?;
        self.state.install(path, bytes)
    }

    /// [`Self::upsert_bytes`] without the validating parse.
    ///
    /// Only for a caller that PRODUCED the bytes by serializing a
    /// [`CanonValue`], where canonicality holds by construction and the parse
    /// would be a re-derivation of something already known. A domain reducer is
    /// such a caller; a decoder reading a file is not, which is why
    /// [`SectionState::apply_delta`] validates and this does not.
    pub fn upsert_canonical_bytes(
        &mut self,
        path: Path,
        bytes: Vec<u8>,
    ) -> Result<bool, FoldV3Error> {
        self.state.install(path, bytes)
    }

    /// Delete the grain at `path`.
    pub fn remove(&mut self, path: &[Seg]) -> Result<bool, FoldV3Error> {
        self.state.erase(path)
    }

    /// Change what the section's root IS -- container to `null` and back.
    ///
    /// Separate from the grains because it is a different fact: a section that
    /// became `null` because its domain was unknown this hour is not a section
    /// that lost all its children.
    pub fn set_root_kind(&mut self, kind: RootKind) -> Result<bool, FoldV3Error> {
        self.state.set_kind(kind)
    }

    /// Replace the WHOLE section with `value`, including a change of root kind.
    ///
    /// This is the entry point for the shape the refused design could not see: a
    /// wholesale assignment that drops the previous state on the floor. It
    /// cannot bypass the journal here, because it is implemented AS the exact
    /// set of upserts and removals that carries the current state to the new one
    /// -- every one of which goes through the same primitives a one-field change
    /// uses. A caller that assigns the section wholesale therefore emits the same
    /// delta as a caller that made the same changes one at a time, and pays a
    /// cost proportional to the difference, not to the assignment.
    ///
    /// It is transactional in the sense that matters: the whole replacement is
    /// decomposed and validated BEFORE the first mutation, so a malformed
    /// assignment leaves the state exactly as it was.
    ///
    /// Its scan of the current grains IS proportional to the section size, which
    /// is the price of the shape; the design's claim is that it happens on a
    /// domain baseline record, not every hour.
    pub fn assign_section(&mut self, value: &CanonValue) -> Result<u64, FoldV3Error> {
        let spec_kind = self.state.spec.shape.root_kind();
        let incoming_kind = match (&self.state.spec.shape, value) {
            (Shape::Object, CanonValue::Object(_)) => spec_kind,
            (Shape::Array | Shape::KeyedArray { .. }, CanonValue::Array(_)) => spec_kind,
            // Anything else -- including the `null` the fold writes for a
            // section whose domain was unknown this hour -- is a scalar root.
            _ => RootKind::Scalar,
        };
        let incoming: Vec<(Path, Vec<u8>)> = if incoming_kind == RootKind::Scalar {
            vec![(Path::new(), value.to_json_bytes())]
        } else {
            decompose(
                value,
                self.state.spec.grain_depth,
                &self.state.spec.shape,
            )?
        };
        let mut changed = 0u64;
        let mut seen: BTreeMap<Path, ()> = BTreeMap::new();
        for (path, _) in &incoming {
            seen.insert(path.clone(), ());
        }
        // STALE GRAINS GO FIRST. A member that was a value and became a
        // container -- `supply.hubs` on capture t30 -- needs `[hubs]` gone
        // before `[hubs, x]` can stand, or the state would briefly hold a value
        // and a container at one address, which is the shape `check_path`
        // refuses and which no document could spell.
        let stale: Vec<Path> = self
            .state
            .grains
            .keys()
            .filter(|path| !seen.contains_key(*path))
            .cloned()
            .collect();
        for path in stale {
            if self.state.erase(&path)? {
                changed += 1;
            }
        }
        // Between the two: the root kind can only move once the grains that
        // contradict it are gone, and before the ones that need it arrive.
        if self.state.set_kind(incoming_kind)? {
            changed += 1;
        }
        for (path, bytes) in incoming {
            if self.state.install(path, bytes)? {
                changed += 1;
            }
        }
        Ok(changed)
    }

    pub fn state(&self) -> &SectionState {
        self.state
    }

    /// Close the hour: coalesce, sort canonically, encode.
    ///
    /// Coalescing is structural rather than a pass: the journal is keyed by
    /// path, so a second write to one grain within the hour has already replaced
    /// the first, and the map's iteration order IS the canonical order the
    /// encoding requires. An hour with no changes commits an empty delta, which
    /// is a fact about the hour and not an omission.
    pub fn commit(mut self) -> SectionDelta {
        self.committed = true;
        let baseline = std::mem::take(&mut self.state.hour_baseline);
        let root_baseline = self.state.hour_root_baseline.take();
        let mut bytes = Vec::new();
        let mut mutations = 0u64;
        // REMOVALS, then the root kind, then upserts -- each path phase
        // ascending.
        //
        // One ascending pass is not a legal order. A node that was a value and
        // became a container -- `supply.hubs` going from a scalar to an object,
        // which capture t30 does -- turns one grain at `[hubs]` into grains at
        // `[hubs, ...]`, and `[hubs]` sorts BEFORE `[hubs, x]`. A single
        // ascending pass therefore asks the reader to install a value at an
        // address that still has a container under it. Removals cannot collide
        // with each other, because a state never holds a grain and its ancestor
        // at once, so two ascending phases are a total order that is always
        // applicable.
        for (path, before) in &baseline {
            if self.state.grains.contains_key(path) {
                continue;
            }
            // Present at the start and gone now. The removal is addressed by the
            // spelling the replica holds, which is the one it was stored under.
            if let Some((stored, _)) = before {
                bytes.push(OP_REMOVE);
                encode_path_into(stored, &mut bytes);
                mutations += 1;
            }
        }
        // The root kind sits BETWEEN the phases, for the same reason the
        // producer moves it there: it can only change once the grains that
        // contradict it are gone and before the ones that need it arrive. An
        // hour that changed it and changed it back changed nothing, and says so.
        if root_baseline.is_some_and(|before| before != self.state.kind) {
            bytes.push(OP_ROOT);
            bytes.push(self.state.kind.tag());
            mutations += 1;
        }
        for (path, before) in &baseline {
            let Some((stored, grain)) = self.state.grains.get_key_value(path) else {
                continue;
            };
            if let Some((before_path, before_digest)) = before {
                if before_digest == &grain.digest && same_spelling(before_path, stored) {
                    continue;
                }
            }
            bytes.push(OP_UPSERT);
            encode_path_into(stored, &mut bytes);
            write_varint(&mut bytes, grain.bytes.len() as u64);
            bytes.extend_from_slice(&grain.bytes);
            mutations += 1;
        }
        SectionDelta {
            section: self.state.spec.name.clone(),
            bytes,
            mutations,
            state_root: self.state.state_root(),
        }
    }
}

impl Drop for HourTxn<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        if !self.state.hour_baseline.is_empty() || self.state.hour_root_baseline.is_some() {
            self.state.poison_and_empty(
                "an hour with pending mutations was dropped instead of committed".to_string(),
            );
        }
    }
}

fn write_member_prefix(out: &mut Vec<u8>, seg: &Seg, named: bool) {
    if !named {
        return;
    }
    match seg {
        Seg::Key(name) => {
            out.push(b'"');
            out.extend_from_slice(name.as_bytes());
            out.push(b'"');
            out.push(b':');
        }
        // An ordinal never names a member; reaching here would mean an array
        // container was mistaken for an object one.
        Seg::Index(_) => debug_assert!(false, "an ordinal address cannot name a member"),
    }
}

// ---------------------------------------------------------------------------
// Decomposition
// ---------------------------------------------------------------------------

/// Cut `value` into `(path, canonical bytes)` grains at `grain_depth`.
///
/// A node shallower than the grain depth that cannot be descended -- a scalar,
/// an empty object, an empty array -- becomes a grain where it stands. Empty
/// containers must be grains and not simply absent: the skeleton is implied by
/// the grain paths, so an empty container with no grain beneath it would vanish
/// from the projection.
pub fn decompose(
    value: &CanonValue,
    grain_depth: usize,
    shape: &Shape,
) -> Result<Vec<(Path, Vec<u8>)>, FoldV3Error> {
    Ok(decompose_ref(value, grain_depth, shape)?
        .into_iter()
        .map(|(path, value)| (path, value.to_json_bytes()))
        .collect())
}

/// [`decompose`] without serializing anything.
///
/// The grains are BORROWED from `value`, so a caller that only needs to know
/// which grains moved does not pay a copy of the section to find out. That
/// matters here: the whole experiment is about what costs are proportional to
/// the section rather than to the change, and an instrument that cloned the
/// section to answer the question would have added one.
pub fn decompose_ref<'a>(
    value: &'a CanonValue,
    grain_depth: usize,
    shape: &Shape,
) -> Result<Vec<(Path, &'a CanonValue)>, FoldV3Error> {
    if grain_depth == 0 {
        return Err(FoldV3Error::new(
            "grain depth 0 would make the whole section one grain, which a container \
             root cannot hold",
        ));
    }
    let mut out = Vec::new();
    let mut path = Path::new();
    match shape {
        Shape::KeyedArray { key_member } => {
            let CanonValue::Array(items) = value else {
                return Err(FoldV3Error::new(
                    "a keyed-array section must be spelled as a JSON array",
                ));
            };
            let mut previous: Option<String> = None;
            for item in items {
                let CanonValue::Object(members) = item else {
                    return Err(FoldV3Error::new(
                        "a keyed-array element must be a JSON object",
                    ));
                };
                let key = members
                    .iter()
                    .find(|(name, _)| name == key_member)
                    .and_then(|(_, value)| match value {
                        CanonValue::String(body) => Some(body.clone()),
                        _ => None,
                    })
                    .ok_or_else(|| {
                        FoldV3Error::new(format!(
                            "a keyed-array element has no string `{key_member}` to address it by"
                        ))
                    })?;
                // The projection writes the elements back in ascending key
                // order, so a source array that is NOT in that order would come
                // back reordered, and a duplicate identity would come back once.
                // Refusing here is what keeps the keyed grain from silently
                // rewriting a document. Note the order is the KEY's string
                // order, which is not its numeric order: `10:1` precedes `2:1`.
                if let Some(previous) = &previous {
                    if key_cmp(previous, &key) != std::cmp::Ordering::Less {
                        return Err(FoldV3Error::new(format!(
                            "keyed-array elements are not in strictly ascending `{key_member}` order at {key}"
                        )));
                    }
                }
                previous = Some(key.clone());
                path.push(Seg::Key(key));
                decompose_into(item, grain_depth.saturating_sub(1), &mut path, &mut out);
                path.pop();
            }
            Ok(out)
        }
        Shape::Object => {
            let CanonValue::Object(members) = value else {
                return Err(FoldV3Error::new(
                    "an object section must be spelled as a JSON object",
                ));
            };
            // An EMPTY container root is the root kind and nothing else. Emitting
            // a grain at the empty path for it would put a whole-section value
            // under a container root, which is the one path set the projection
            // cannot write back.
            if members.is_empty() {
                return Ok(out);
            }
            decompose_into(value, grain_depth, &mut path, &mut out);
            Ok(out)
        }
        Shape::Array => {
            let CanonValue::Array(items) = value else {
                return Err(FoldV3Error::new(
                    "an array section must be spelled as a JSON array",
                ));
            };
            if items.is_empty() {
                return Ok(out);
            }
            decompose_into(value, grain_depth, &mut path, &mut out);
            Ok(out)
        }
    }
}

fn decompose_into<'a>(
    value: &'a CanonValue,
    depth_left: usize,
    path: &mut Path,
    out: &mut Vec<(Path, &'a CanonValue)>,
) {
    if depth_left == 0 {
        out.push((path.clone(), value));
        return;
    }
    match value {
        CanonValue::Object(members) if !members.is_empty() => {
            for (name, member) in members {
                path.push(Seg::Key(name.clone()));
                decompose_into(member, depth_left - 1, path, out);
                path.pop();
            }
        }
        CanonValue::Array(items) if !items.is_empty() => {
            for (index, item) in items.iter().enumerate() {
                path.push(Seg::Index(index as u32));
                decompose_into(item, depth_left - 1, path, out);
                path.pop();
            }
        }
        _ => out.push((path.clone(), value)),
    }
}

// ---------------------------------------------------------------------------
// Varints and reading
// ---------------------------------------------------------------------------

fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

struct ByteReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> ByteReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }
    fn is_empty(&self) -> bool {
        self.position >= self.bytes.len()
    }
    fn u8(&mut self) -> Result<u8, FoldV3Error> {
        let byte = *self
            .bytes
            .get(self.position)
            .ok_or_else(|| FoldV3Error::new("delta ends inside a record"))?;
        self.position += 1;
        Ok(byte)
    }
    fn take(&mut self, len: usize) -> Result<&'a [u8], FoldV3Error> {
        let end = self
            .position
            .checked_add(len)
            .ok_or_else(|| FoldV3Error::new("delta length overflows"))?;
        if end > self.bytes.len() {
            return Err(FoldV3Error::new("delta ends inside a value"));
        }
        let slice = &self.bytes[self.position..end];
        self.position = end;
        Ok(slice)
    }
    fn varint(&mut self) -> Result<u64, FoldV3Error> {
        let mut value = 0u64;
        let mut shift = 0u32;
        loop {
            let byte = self.u8()?;
            if shift >= 64 {
                return Err(FoldV3Error::new("delta varint is too long"));
            }
            // The tenth group has one bit left to land in. Without this, nine
            // continuation bytes followed by `0x02` decodes to zero -- a second
            // spelling of a value the writer spells in one byte, and therefore a
            // second encoding of one delta.
            if shift == 63 && byte & 0x7e != 0 {
                return Err(FoldV3Error::new("delta varint overflows 64 bits"));
            }
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                // One encoding per value, or the delta bytes are not canonical
                // and two different files could carry the same state.
                if byte == 0 && shift > 0 {
                    return Err(FoldV3Error::new("delta varint is not minimally encoded"));
                }
                return Ok(value);
            }
            shift += 7;
        }
    }
    fn path(&mut self) -> Result<Path, FoldV3Error> {
        let len = self.varint()? as usize;
        if len > 64 {
            return Err(FoldV3Error::new("delta path is implausibly deep"));
        }
        let mut path = Path::with_capacity(len);
        for _ in 0..len {
            match self.u8()? {
                SEG_KEY => {
                    let bytes = self.varint()? as usize;
                    let body = self.take(bytes)?;
                    let name = std::str::from_utf8(body)
                        .map_err(|_| FoldV3Error::new("delta path segment is not UTF-8"))?;
                    path.push(Seg::Key(name.to_owned()));
                }
                SEG_INDEX => {
                    let index = self.varint()?;
                    let index = u32::try_from(index)
                        .map_err(|_| FoldV3Error::new("delta path ordinal does not fit u32"))?;
                    path.push(Seg::Index(index));
                }
                other => {
                    return Err(FoldV3Error::new(format!(
                        "unknown delta path segment tag {other}"
                    )))
                }
            }
        }
        Ok(path)
    }
}

#[cfg(test)]
mod tests;
