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

use snapshot::canon_value::{key_cmp, CanonValue};
use snapshot::sha256::Sha256;

mod grains;
pub mod intern;
pub use intern::KeyRef;

use grains::GrainStore;

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
///
/// The body is held as an INTERNED [`KeyRef`] rather than a `String`, which is
/// what makes this type `Copy`. Three consequences, all of them on the mutation
/// path: cloning an address is one allocation instead of one per segment;
/// comparing two segments is a pointer test or a `str` compare instead of
/// `key_cmp`'s backslash scan of both bodies; and `check_path` reads a cached
/// `is_valid_object_key` instead of rescanning and re-decoding the name. See
/// [`intern`] for the case-by-case argument that the ORDER is unchanged.
#[derive(Debug, Clone, Copy)]
pub enum Seg {
    Key(KeyRef),
    Index(u32),
}

impl Seg {
    /// A `Key` segment from a member-name body, interning it.
    pub fn key(body: &str) -> Self {
        Seg::Key(KeyRef::intern(body))
    }
    fn is_key(&self) -> bool {
        matches!(self, Seg::Key(_))
    }
    /// Same address, different source spelling.
    fn respells(&self, other: &Self) -> bool {
        match (self, other) {
            (Seg::Key(left), Seg::Key(right)) => {
                !left.same_spelling(right) && left.cmp_key(right).is_eq()
            }
            _ => false,
        }
    }
}

/// Equality is ADDRESS equality, deliberately delegated to [`Ord`].
///
/// A derived `PartialEq` would compare the escaped spelling while `Ord` compares
/// the decoded text, and the two would disagree on `a` versus `a`. Any
/// container keyed on such a type is a trap: it looks up by one of them while
/// every other caller reasons by the other, so one address would be two keys to
/// half the code and one key to the other half. [`grains`] depends on the two
/// agreeing twice over -- its order is `Ord` and its index confirms with `==`.
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
            // instead would put `a` after `b`. `cmp_key` is `key_cmp` with
            // the decoding already done; the case analysis is in [`intern`].
            (Seg::Key(left), Seg::Key(right)) => left.cmp_key(right),
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
            (Seg::Key(left), Seg::Key(right)) => left.same_spelling(right),
            (Seg::Index(left), Seg::Index(right)) => left == right,
            _ => false,
        })
}

fn encode_path_into(path: &[Seg], out: &mut Vec<u8>) {
    write_varint(out, path.len() as u64);
    for seg in path {
        match seg {
            // `SEG_KEY || varint(len) || body`, frozen on the interned entry
            // when the name was first seen. Every mutation encodes its address
            // for the state-root accumulator, so this is a per-mutation loop
            // turned into a per-mutation memcpy.
            Seg::Key(name) => out.extend_from_slice(name.encoded()),
            Seg::Index(index) => {
                out.push(SEG_INDEX);
                write_varint(out, u64::from(*index));
            }
        }
    }
}

pub(crate) const SEG_KEY: u8 = 0;
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
    /// This grain's own term in the state-root accumulator, computed once when
    /// the grain was installed.
    ///
    /// It is a CACHE of `Accumulator::contribution(stored path, digest)` and is
    /// held for one reason: retracting a grain has to hash the same bytes that
    /// adding it hashed, so a state that did not keep the answer paid the hash
    /// twice for every mutation -- once to put the old contribution back and
    /// once to add the new one. Keeping it makes the accumulator cost one hash
    /// per mutation instead of two, and it cannot go stale beside the grain
    /// because there is no way to change `bytes` or `digest` without replacing
    /// the whole `Grain`.
    ///
    /// A wrong cached value is not silent: it is exactly what
    /// [`SectionState::recomputed_state_root`] recomputes from the path and the
    /// digest and compares against, every hour, in the driver and in the tests.
    contribution: [u64; 4],
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
    /// `scratch` is the caller's buffer, not a fresh `Vec`, because this runs on
    /// every grain a mutation touches and the corpus moves thousands an hour: a
    /// `Vec::with_capacity(64)` here is an allocation and a free there, for a
    /// buffer whose contents nothing outlives.
    fn contribution(path: &[Seg], digest: &[u8; 32], scratch: &mut Vec<u8>) -> [u64; 4] {
        scratch.clear();
        // Domain separation and length prefixes, so no two different
        // (path, digest) pairs can present the same byte sequence.
        scratch.push(0x01);
        encode_path_into(path, scratch);
        scratch.push(0x00);
        scratch.extend_from_slice(digest);
        let bytes = Sha256::digest(scratch);
        let mut limbs = [0u64; 4];
        for (index, limb) in limbs.iter_mut().enumerate() {
            let mut word = [0u8; 8];
            word.copy_from_slice(&bytes[index * 8..index * 8 + 8]);
            *limb = u64::from_le_bytes(word);
        }
        limbs
    }

    fn add(&mut self, limbs: [u64; 4]) {
        for (slot, value) in self.0.iter_mut().zip(limbs) {
            *slot = slot.wrapping_add(value);
        }
    }

    fn remove(&mut self, limbs: [u64; 4]) {
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

/// What one address held when the hour opened.
///
/// `spelling` is `Some` only when the grain was STORED under a different
/// escaped spelling of the same address than the key this baseline is filed
/// under -- the case an escaped member name creates and nothing else. Spelling
/// it as an option rather than always keeping a copy of the stored path is what
/// keeps the common mutation from cloning a `Vec<Seg>` full of `String`s to
/// record a spelling it already has.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Before {
    spelling: Option<Path>,
    digest: [u8; 32],
}

impl Before {
    /// The address as the replica holds it: the stored respelling if there was
    /// one, otherwise the key this baseline is filed under.
    fn stored<'a>(&'a self, key: &'a [Seg]) -> &'a [Seg] {
        match &self.spelling {
            Some(spelling) => spelling,
            None => key,
        }
    }
}

pub struct SectionState {
    spec: SectionSpec,
    kind: RootKind,
    /// The whole of the state. Private, with no `&mut` accessor, no mutable
    /// iterator, no `entry`, and no `IndexMut`: the transaction is the only way
    /// in, which is what makes "every change is journalled" a property of the
    /// type rather than of the caller's discipline.
    ///
    /// A dense slot table with a hash index and a separately maintained
    /// canonical order, rather than the `BTreeMap` it was: an address lookup is
    /// the mutation path's whole traffic with the state, and a comparison-
    /// ordered container answers it in ~28 pointer-chasing key comparisons on
    /// the corpus's largest section. See [`grains`] for the order argument --
    /// the sequence this store iterates in is `<[Seg]>::cmp` ascending, which
    /// is the order the map was keyed on and the one the projection and the
    /// delta encoding require.
    grains: GrainStore,
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
    ///
    /// It is an APPEND-ONLY log rather than a map, sorted and deduplicated once
    /// at commit. A map made every mutation pay two `BTreeMap` descents and a
    /// node allocation to answer "have I already recorded this address" -- a
    /// question that only has to be answered once, at the end, over the whole
    /// hour. The sort is stable and the dedup keeps the FIRST entry for each
    /// address, which is the same "record it once, as it stood when the hour
    /// opened" rule the map enforced on the way in.
    hour_touched: Vec<(Path, Option<Before>)>,
    /// What the root kind was when the hour opened, recorded on first change.
    hour_root_baseline: Option<RootKind>,
    revision: u64,
    /// Scratch for [`Accumulator::contribution`]. Owned by the section so the
    /// hot path reuses one buffer instead of allocating one per mutation.
    scratch: Vec<u8>,
    /// Scratch for the upsert half of [`HourTxn::commit`], kept across hours for
    /// the same reason `hour_touched` is.
    commit_upserts: Vec<u8>,
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
            grains: GrainStore::new(),
            accumulator: Accumulator::default(),
            hour_touched: Vec::new(),
            hour_root_baseline: None,
            revision: 0,
            scratch: Vec::with_capacity(128),
            commit_upserts: Vec::new(),
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
        self.hour_touched.clear();
        self.hour_root_baseline = None;
    }
    pub fn grain(&self, path: &[Seg]) -> Option<&Grain> {
        self.grains
            .get_slot(path)
            .map(|slot| self.grains.grain_of(slot))
    }

    /// The grain at `path` together with the address it is STORED under.
    ///
    /// The two can differ: an address is its decoded text, and a caller that
    /// looked one up under a different escaped spelling would otherwise conclude
    /// that nothing had changed while the projection kept writing the old
    /// spelling.
    pub fn grain_entry(&self, path: &[Seg]) -> Option<(&Path, &Grain)> {
        self.grains
            .get_slot(path)
            .map(|slot| (self.grains.path_of(slot), self.grains.grain_of(slot)))
    }

    /// Every address the state holds, in canonical order. Read-only: a caller
    /// that wants to change one has to go through a transaction.
    pub fn grain_paths(&self) -> impl Iterator<Item = &Path> {
        self.grains.iter().map(|(path, _)| path)
    }

    /// Bytes of grain payload retained. The state's own accounting, so the
    /// memory question can be answered without trusting a process-level number.
    pub fn retained_grain_bytes(&self) -> u64 {
        self.grains
            .iter()
            .map(|(_, grain)| grain.bytes.len() as u64)
            .sum()
    }

    /// Bytes of grain ADDRESSES retained -- the paths' own heap.
    ///
    /// A `Seg` is now `Copy` and lives INSIDE the path's own allocation, so a
    /// path costs its `Vec` of segments and nothing per name. The names
    /// themselves moved to the interner, which is shared by every path in every
    /// section and by both states, and is reported once by
    /// [`intern::retained_bytes`] rather than counted here per section -- adding
    /// it in would charge one copy of the vocabulary to each of the 39 sections.
    pub fn retained_path_bytes(&self) -> u64 {
        self.grains.retained_path_bytes()
    }

    /// Bytes the section retains BESIDE its grains and their addresses: the
    /// three buffers it deliberately keeps across hours so the allocator stays
    /// off the mutation path.
    ///
    /// Capacity, not length. Every one of these is empty between hours and that
    /// is exactly why it has to be measured as capacity: an hour that wrote a
    /// ten-megabyte upsert phase leaves a ten-megabyte buffer behind it, and a
    /// report that read `len()` would call it nothing. `commit_upserts` and
    /// `scratch` are the ones this crate's one-pass commit and per-mutation
    /// accumulator introduced; `hour_touched` was retained before either and was
    /// not being counted either, so it is counted here rather than left out on
    /// the grounds that it is somebody else's omission.
    ///
    /// The grain store's own overhead is added for the same reason and is the
    /// same kind of thing: its hash index, its two order arrays and its free
    /// list are held across hours on purpose, and so is every slot the table
    /// has reserved that no grain is standing in. The slots that DO hold a
    /// grain are not counted here -- the driver's per-grain overhead constant
    /// charges those -- so nothing is counted twice. See
    /// [`grains::GrainStore::retained_overhead_bytes`].
    pub fn retained_scratch_bytes(&self) -> u64 {
        (self.commit_upserts.capacity()
            + self.scratch.capacity()
            + self.hour_touched.capacity() * std::mem::size_of::<(Path, Option<Before>)>())
            as u64
            + self.grains.retained_overhead_bytes()
    }

    /// The store's neighbour queries, so a test can compare them against the
    /// `BTreeMap` range they replaced. `check_path` is the only caller in
    /// production and it consumes the answer rather than returning it.
    #[cfg(test)]
    pub(crate) fn first_at_or_after_for_test(&self, path: &[Seg]) -> Option<Path> {
        self.grains.first_at_or_after(path).cloned()
    }

    #[cfg(test)]
    pub(crate) fn last_before_for_test(&self, path: &[Seg]) -> Option<Path> {
        self.grains.last_before(path).cloned()
    }

    /// `(rows, live, tombstoned, free)` in the grain store's slot table. See
    /// [`grains::GrainStore::accounting`].
    #[cfg(test)]
    pub(crate) fn slot_accounting_for_test(&self) -> (usize, usize, usize, usize) {
        self.grains.accounting()
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
        let mut scratch = Vec::with_capacity(128);
        for (path, grain) in self.grains.iter() {
            // From the path and the digest, NOT from `grain.contribution`. The
            // cached term is part of what this is disbelieving: a contribution
            // computed from the wrong path or a superseded digest would leave a
            // root that is stable, self-consistent and wrong, and recomputing
            // from the cache would agree with it.
            accumulator.add(Accumulator::contribution(path, &grain.digest, &mut scratch));
        }
        accumulator.root(self.kind)
    }

    /// Open the hour. The returned transaction is the only handle that can
    /// mutate this section, and it must be committed.
    pub fn begin_hour(&mut self) -> Result<HourTxn<'_>, FoldV3Error> {
        if let Some(reason) = &self.poison {
            return Err(FoldV3Error::new(format!("section is poisoned: {reason}")));
        }
        if !self.hour_touched.is_empty() || self.hour_root_baseline.is_some() {
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

    /// Advance the section revision, or POISON AND EMPTY the section.
    ///
    /// Setting the flag alone is not enough, and used to be covered for by
    /// accident: the mutation primitives happened to have recorded an hour
    /// baseline before the counter could fail, so dropping the transaction found
    /// pending changes and emptied the section on the way out. That is a
    /// property of the order two unrelated statements happened to be in, not of
    /// the counter -- and `erase` now takes its baseline after the bump, which
    /// would have left a poisoned section still serving its grains through
    /// `project`, `grain` and `state_root`, with an incremental root that still
    /// agreed with a full recomputation. Emptying here makes the fail-closed
    /// contract belong to the failure rather than to its surroundings.
    fn bump(&mut self) -> Result<u64, FoldV3Error> {
        match self.revision.checked_add(1) {
            Some(next) => {
                self.revision = next;
                Ok(next)
            }
            None => {
                let reason = "section revision counter overflowed".to_string();
                self.poison_and_empty(reason.clone());
                Err(FoldV3Error::new(reason))
            }
        }
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
                // Frozen on the interned entry, not rescanned and re-decoded
                // here. Every newly addressed grain used to pay a scan and a
                // full decode per segment for an answer that depends only on
                // the name.
                if !name.valid_member_name() {
                    return Err(FoldV3Error::new(format!(
                        "path segment {name:?} is not a valid JSON member name body"
                    )));
                }
            }
        }
        // An ancestor holding a grain would be a value where this path needs a
        // container.
        for depth in 1..path.len() {
            if self.grains.get_slot(&path[..depth]).is_some() {
                return Err(FoldV3Error::new(
                    "a grain already stands at an ancestor of this path",
                ));
            }
        }
        // The three remaining questions are all about the addresses either side
        // of this one, so they are asked ONCE. This runs on every newly
        // addressed grain, the corpus installs thousands an hour, and each
        // order query is a chain of random reads into the slot table.
        //
        // The query borrows `path` as a slice rather than cloning it into a
        // `Vec`, for the same reason: an allocation here would be an allocation
        // there, inside the very region this prototype exists to price.
        let (before, after) = self.grains.neighbours(path);
        // A descendant would be a container where this path needs a value.
        if let Some(candidate) = after {
            if candidate.len() > path.len() && candidate[..path.len()] == *path {
                return Err(FoldV3Error::new(
                    "a grain already stands beneath this path",
                ));
            }
        }
        // Siblings must agree on how their container addresses them, and
        // checking the two immediate NEIGHBOURS is enough to know it.
        //
        // Every path sharing a prefix occupies one contiguous run of the order,
        // because the order is lexicographic. Within such a run, `Seg::cmp` puts
        // every name before every ordinal, so a container addressed both ways
        // would have its names and its ordinals in two adjacent blocks -- and an
        // incoming path lands either at that boundary or with a neighbour that
        // is already on the wrong side of it. A disagreement therefore cannot
        // hide several entries away while both neighbours agree.
        let mut neighbours = before.into_iter().chain(after);
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
    ///
    /// `before` is what the caller's own lookup already found there, passed in
    /// rather than looked up again: `install` and `erase` both have to probe the
    /// grain anyway, and probing twice per mutation is a whole index probe per
    /// mutation spent re-deriving something the caller is holding.
    /// Appended unconditionally: a second record for one address inside one hour
    /// is discarded at commit, where the log is already being sorted, instead of
    /// being prevented here at the cost of a lookup on every mutation.
    fn remember(&mut self, path: &[Seg], before: Option<Before>) {
        self.hour_touched.push((path.to_vec(), before));
    }

    /// Install `bytes` at `path`.
    ///
    /// ONE probe of the state, from which everything below is decided: what
    /// stands at this address, under which spelling, whether it already holds
    /// these bytes, and what has to be retracted from the state root. That was
    /// the intent before the store was dense too, but a `BTreeMap` could not
    /// keep it: a lookup that hands back a position cannot also let
    /// `check_path` read the map, and `BTreeMap::insert` keeps the ORIGINAL key
    /// when the new one compares equal, so a respelling had to be removed
    /// before it could be replaced. The fast/general split and the sticky
    /// `may_respell` flag that routed between them existed for those two
    /// reasons, and the store having slot numbers instead of tree positions
    /// removes both: the slot survives `check_path`, and a respelling is
    /// written over the slot's own `path` field.
    ///
    /// The retraction is addressed by the spelling the grain was ADDED under.
    /// Retracting under the incoming spelling would subtract a different
    /// contribution and leave the root silently wrong, because the spelling is
    /// inside the hash -- which is exactly why the contribution is carried on
    /// the grain and not recomputed here from `path`.
    fn install(&mut self, path: Path, bytes: Vec<u8>) -> Result<bool, FoldV3Error> {
        self.stats.calls += 1;
        let mut rekey = false;
        let mut retract: Option<[u64; 4]> = None;
        let mut before: Option<Before> = None;
        let probe = self.grains.get_slot(&path);
        let known_address = if let Some(slot) = probe {
            rekey = same_address_other_spelling(self.grains.path_of(slot), &path);
            let existing = self.grains.grain_of(slot);
            // Bytes, not digests. A digest comparison would answer this question
            // through a hash of the incoming value; comparing the bytes answers
            // it directly, costs one memcmp over the changed grain, removes the
            // one way this design could serve a wrong document while every check
            // agreed -- and means an unchanged grain is never hashed at all.
            if !rekey && existing.bytes == bytes {
                self.stats.no_ops += 1;
                return Ok(false);
            }
            retract = Some(existing.contribution);
            let digest = existing.digest;
            before = Some(Before {
                spelling: rekey.then(|| self.grains.path_of(slot).clone()),
                digest,
            });
            true
        } else {
            false
        };
        // `check_path` validates the path SET -- that no ancestor or descendant
        // of this address holds a grain, that its siblings agree on how their
        // container addresses them, that its member names are quotable, and that
        // the root's addressing mode admits it. An address that ALREADY STANDS
        // under this exact spelling went through all of that when it was
        // installed, and none of those facts can have moved since: the set has
        // not changed, the shape is fixed for the section's lifetime, and
        // `set_kind` refuses every root kind that would contradict the grains.
        // Re-deriving them is five index probes and three order queries per
        // mutation for an answer already known. A REKEY does change the stored
        // spelling, so it is checked like a new address.
        if !known_address || rekey {
            self.check_path(&path)?;
        }
        let digest = Sha256::digest(&bytes);
        let contribution = Accumulator::contribution(&path, &digest, &mut self.scratch);
        // Advance the counter BEFORE the state root moves, so an overflow
        // poisons a state whose root is still the root of its grains.
        let revision = self.bump()?;
        self.remember(&path, before);
        if let Some(previous) = retract {
            self.accumulator.remove(previous);
        }
        self.stats.upserts += 1;
        self.stats.grain_bytes_written += bytes.len() as u64;
        self.accumulator.add(contribution);
        let grain = Grain {
            bytes,
            digest,
            rev: revision,
            contribution,
        };
        match probe {
            // A REKEY writes the incoming spelling over the slot's own `path`.
            // The address is unchanged by definition, so the slot does not move
            // in the order and its index bucket does not change -- the store
            // hashes the address, not the spelling. The DELTA carries whatever
            // spelling stands at commit, because it is derived from the state
            // and not accumulated.
            Some(slot) => self.grains.replace(slot, rekey.then_some(path), grain),
            None => self.grains.insert(path, grain),
        }
        Ok(true)
    }

    fn erase(&mut self, path: &[Seg]) -> Result<bool, FoldV3Error> {
        self.stats.calls += 1;
        // One probe, as in `install`. The stored spelling need not be the one
        // the caller looked the grain up with, and both the baseline and the
        // retraction need the stored one.
        let Some(slot) = self.grains.get_slot(path) else {
            self.stats.no_ops += 1;
            return Ok(false);
        };
        let stored_path = self.grains.path_of(slot);
        let before = Before {
            spelling: (!same_spelling(stored_path, path)).then(|| stored_path.clone()),
            digest: self.grains.grain_of(slot).digest,
        };
        let contribution = self.grains.grain_of(slot).contribution;
        // The counter moves BEFORE the state does, so an overflow leaves a state
        // whose root still describes its grains and needs no rollback. It also
        // empties the store on the way out, which is why nothing may touch the
        // slot number after this point without going through `?`.
        self.bump()?;
        // BEFORE the grain goes: the hour's baseline is what stood here when the
        // hour opened, and a baseline taken after the removal would record that
        // nothing did.
        self.remember(path, Some(before));
        self.grains.remove(slot);
        self.stats.removes += 1;
        self.accumulator.remove(contribution);
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
            RootKind::Scalar if self.grains.iter().any(|(path, _)| !path.is_empty()) => {
                return Err(FoldV3Error::new(
                    "a section cannot become a scalar while it still holds children",
                ))
            }
            RootKind::Object | RootKind::Array if self.grains.get_slot(&[]).is_some() => {
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
            match self.grain(&[]) {
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
        for (path, grain) in self.grains.iter() {
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
    ///   encoder never emits one;
    /// * and the ordinals of every container the delta touched must still run
    ///   `0..k` once it has all landed. An array's ordinals are ADDRESSES and
    ///   are never written out, so a state holding only ordinal 1 projects that
    ///   element at position 0 -- a document different from the one its own
    ///   paths describe. A delta is untrusted input, and a shape the projection
    ///   cannot write back has to stop here rather than become a state.
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

    /// Is there a grain AT `prefix` or anywhere beneath it?
    ///
    /// One ordered lookup, not a scan: every path sharing a prefix occupies one
    /// contiguous run, so the first entry at or after `prefix` is the only
    /// candidate that could be in it.
    fn occupied(&self, prefix: &[Seg]) -> bool {
        self.grains
            .first_at_or_after(prefix)
            .is_some_and(|candidate| {
                candidate.len() >= prefix.len() && candidate[..prefix.len()] == *prefix
            })
    }

    /// Every ordinal-addressed container whose ordinals are not `0..k`, as a
    /// message, or `None` if the state is dense throughout.
    ///
    /// This is a WHOLE-STATE scan and is the reference the cheap incremental
    /// check in [`Self::apply_delta`] is measured against, not something the hot
    /// path calls.
    ///
    /// Density is not a nicety. `project` writes an array's elements out in
    /// ascending-ordinal order and the ordinals themselves are never written, so
    /// a state holding only `[1]` under some member emits a one-element array
    /// whose element is at position 0 -- a different document from the one the
    /// paths describe, produced silently, with a state root that commits the
    /// paths nobody can read back.
    pub fn ordinal_density_error(&self) -> Option<String> {
        // Walk the sorted paths and, at every depth, watch the ordinal in that
        // position advance. A container's children are one contiguous run, so
        // the previous path's segment at the same depth is the only neighbour
        // that matters.
        let mut previous: Option<&Path> = None;
        for (path, _) in self.grains.iter() {
            for (depth, seg) in path.iter().enumerate() {
                let Seg::Index(index) = seg else { continue };
                let before = previous.and_then(|previous| {
                    // Same container only: the prefixes above this depth must
                    // match, or this is the first child of a new container.
                    (previous.len() > depth && previous[..depth] == path[..depth])
                        .then(|| match &previous[depth] {
                            Seg::Index(previous) => Some(*previous),
                            Seg::Key(_) => None,
                        })
                        .flatten()
                });
                let expected = before.map_or(0, |before| before + 1);
                if *index != expected && Some(*index) != before {
                    return Some(format!(
                        "an ordinal-addressed container jumps from {} to {index} at depth \
                         {depth}, so its elements do not project back to the positions they \
                         are addressed by",
                        before.map_or("nothing".to_string(), |before| before.to_string()),
                    ));
                }
            }
            previous = Some(path);
        }
        None
    }

    /// The incremental form of [`Self::ordinal_density_error`], for a state that
    /// was dense before the delta.
    ///
    /// A delta can only break density at an ordinal it touched, so only those
    /// need checking, and each costs two ordered lookups rather than a scan
    /// of the container: an ordinal that is now PRESENT needs its predecessor,
    /// and one that is now GONE must not have left a successor stranded.
    ///
    /// EVERY ordinal position along each touched path, not just the last. An
    /// array whose elements are objects -- which is most of them -- has its
    /// ordinals in the MIDDLE of its grains' paths, and a check that looked only
    /// at the final segment would see `[a, 1, x]` as a name and pass the very
    /// gap it exists to catch.
    fn touched_ordinals_stay_dense(&self, touched: &[Path]) -> Result<(), FoldV3Error> {
        let mut probe = Path::new();
        for path in touched {
            for depth in 0..path.len() {
                let Seg::Index(index) = path[depth] else {
                    continue;
                };
                probe.clear();
                probe.extend_from_slice(&path[..depth]);
                // Whether this element is still there at all -- anything at or
                // beneath the address, because an element can be a container.
                if self.occupied(&path[..=depth]) {
                    if index == 0 {
                        continue;
                    }
                    probe.push(Seg::Index(index - 1));
                    if !self.occupied(&probe) {
                        return Err(FoldV3Error::new(format!(
                            "a delta installs ordinal {index} into a container with no ordinal \
                             {} beside it, so the element would project at a position it is \
                             not addressed by",
                            index - 1
                        )));
                    }
                } else {
                    let Some(next) = index.checked_add(1) else {
                        continue;
                    };
                    probe.push(Seg::Index(next));
                    if self.occupied(&probe) {
                        return Err(FoldV3Error::new(format!(
                            "a delta removes ordinal {index} and leaves ordinal {next} standing, \
                             so every later element would project one position early"
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    fn apply_delta_inner(&mut self, bytes: &[u8]) -> Result<u64, FoldV3Error> {
        let mut reader = ByteReader::new(bytes);
        let mut applied = 0u64;
        let mut previous: Option<Path> = None;
        // Ordinal-addressed paths this delta touched, checked for density once
        // the whole delta has landed. Checking as they arrive would be wrong:
        // removals ascend, so a container losing its ordinals 0 and 2 is
        // legitimately non-dense between the two records.
        let mut touched_ordinals: Vec<Path> = Vec::new();
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
            if path.iter().any(|seg| matches!(seg, Seg::Index(_))) {
                touched_ordinals.push(path.clone());
            }
            previous = Some(path);
        }
        self.touched_ordinals_stay_dense(&touched_ordinals)?;
        // What the replica accrued while applying is not a delta of its own.
        self.hour_touched.clear();
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
            .iter()
            .map(|(path, _)| path)
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
    /// Coalescing is one stable sort and one dedup over the hour's touched
    /// addresses, not a pass over the state: the sort puts the log into the
    /// canonical order the encoding requires, and because it is STABLE the first
    /// record for each address -- the one taken when the hour opened -- is the
    /// one the dedup keeps. A second write to a grain inside one hour therefore
    /// contributes nothing beyond the state it left behind, which is the whole
    /// point of netting against a baseline rather than accumulating operations.
    /// An hour with no changes commits an empty delta, which is a fact about the
    /// hour and not an omission.
    pub fn commit(mut self) -> SectionDelta {
        self.committed = true;
        let mut baseline = std::mem::take(&mut self.state.hour_touched);
        baseline.sort_by(|left, right| left.0.cmp(&right.0));
        // `dedup_by` hands each candidate the entry it would follow and drops it
        // when the closure agrees, so this keeps the FIRST of every run.
        baseline.dedup_by(|later, first| later.0 == first.0);
        let root_baseline = self.state.hour_root_baseline.take();
        let mut bytes = Vec::new();
        // The upsert phase is built BESIDE the removal phase and appended after
        // the root record, so the whole baseline is walked ONCE.
        //
        // Two passes over it meant two lookups per touched address, one to ask
        // whether the grain is still there and one to fetch it. When the state
        // was a `BTreeMap` that was two descents -- ~28 key comparisons each
        // over a 105,000-entry map on the largest section, and the term the
        // per-mutation constant was made of. It is now two index probes, which
        // is a much smaller saving, but it is still one lookup rather than two
        // and the bytes are identical either way: the baseline is sorted, so
        // removals come out ascending in the first buffer and upserts ascending
        // in the second, which is exactly what two ordered passes produced.
        //
        // The price is one memcpy of the upsert phase at the end -- a linear
        // copy of bytes already in cache -- and the measured net is what settles
        // it, not this comment.
        let mut upserts = std::mem::take(&mut self.state.commit_upserts);
        upserts.clear();
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
            let Some((stored, grain)) = self
                .state
                .grains
                .get_slot(path)
                .map(|slot| (self.state.grains.path_of(slot), self.state.grains.grain_of(slot)))
            else {
                // Present at the start and gone now. The removal is addressed by
                // the spelling the replica holds, which is the one it was stored
                // under.
                if let Some(before) = before {
                    bytes.push(OP_REMOVE);
                    encode_path_into(before.stored(path), &mut bytes);
                    mutations += 1;
                }
                continue;
            };
            if let Some(before) = before {
                if before.digest == grain.digest && same_spelling(before.stored(path), stored) {
                    continue;
                }
            }
            upserts.push(OP_UPSERT);
            encode_path_into(stored, &mut upserts);
            write_varint(&mut upserts, grain.bytes.len() as u64);
            upserts.extend_from_slice(&grain.bytes);
            mutations += 1;
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
        bytes.extend_from_slice(&upserts);
        // Hand the buffer back rather than dropping it: the next hour touches
        // roughly as many addresses as this one, and a section that reallocated
        // its journal from nothing every hour would put the allocator back on
        // the very path this design exists to keep off it.
        baseline.clear();
        self.state.hour_touched = baseline;
        upserts.clear();
        self.state.commit_upserts = upserts;
        // Close the hour with ONE array of addresses in canonical order.
        //
        // The store can always answer an ordered question with two arrays and a
        // tombstone skip, so this is not needed for correctness. It is needed
        // because of who reads the order: everything that walks the WHOLE
        // section -- the projection, the driver's own scan for retired
        // addresses, the density check -- and a walk of two arrays pays a path
        // comparison per grain to interleave them, which a walk of one does not.
        // Those readers all run between hours, so merging here is the difference
        // between paying that comparison 58,961 times an hour on the corpus's
        // churning section and not paying it at all.
        //
        // It also opens the next hour with an empty `pending`, which halves the
        // binary searches every newly addressed grain makes.
        //
        // The merge itself is charged to the hour that earned it, inside the
        // timed region, and it is skipped entirely by a section whose address
        // set did not move -- which on capture t30 is the largest section there
        // is.
        self.state.grains.compact();
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
        if !self.state.hour_touched.is_empty() || self.state.hour_root_baseline.is_some() {
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
            out.extend_from_slice(name.body().as_bytes());
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
            for item in items.iter() {
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
                path.push(Seg::key(&key));
                previous = Some(key);
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
            for (name, member) in members.iter() {
                // Interned, not cloned. The corpus re-presents the same member
                // names every hour, so after the first hour this is a hash
                // lookup returning a `Copy` handle instead of an allocation and
                // a copy of the name.
                path.push(Seg::key(name));
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

pub(crate) fn write_varint(out: &mut Vec<u8>, mut value: u64) {
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
                    path.push(Seg::key(name));
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
