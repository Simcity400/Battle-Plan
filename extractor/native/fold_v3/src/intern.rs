//! Interned path-segment member names, and the per-name facts the mutation
//! path used to re-derive on every call.
//!
//! # Why
//!
//! A grain address is a [`crate::Path`], and before this module every segment of
//! one owned its own `String`. Four costs followed from that, all of them paid
//! per mutation rather than per distinct name:
//!
//! * **Comparison, and the cache misses under it.** The state is a `BTreeMap`
//!   keyed on the path, so a mutation pays several descents' worth of segment
//!   comparisons -- and on the corpus's largest section a descent is ~28 of
//!   them. Every one used to dereference a `String`'s heap buffer before it
//!   could look at a byte, and then run
//!   `snapshot::canon_value::key_cmp`, which SCANS BOTH NAMES for a backslash
//!   before it can decide whether it may compare the bodies directly. The
//!   measurement that motivated this module is that the per-mutation constant
//!   tracks the section's GRAIN COUNT (1.5 us at 624 grains, 7.0 us at 105,248)
//!   which is the signature of a pointer-chasing descent, not of hashing: the
//!   same section's mean grain is 17 bytes, one SHA-256 block.
//! * **Allocation.** `SectionState::remember` clones the address of every
//!   mutation into the hour's baseline log, and a clone of a depth-3 path was
//!   four allocations and three memcpys. `decompose` built the whole section's
//!   addresses from scratch every hour for the same reason.
//! * **Validation.** `check_path` calls `is_valid_object_key` on every segment
//!   of every newly addressed grain, which is a second scan and a full decode.
//! * **Encoding.** Every mutation encodes its address into the state-root
//!   accumulator one varint and one byte-slice at a time.
//!
//! None of those depend on the mutation. They depend on the NAME, and the corpus
//! reuses a few hundred thousand names across every hour of a capture.
//! Interning computes them once, at the moment a name is first seen, and hands
//! every later use a [`KeyRef`] -- a `Copy` handle to the frozen answer, small
//! enough to live INSIDE the path's own allocation.
//!
//! # The comparison head
//!
//! [`KeyRef`] carries the first eight bytes of the name's ordering text INLINE,
//! as a big-endian `u64`. Two names whose heads differ are ordered by their
//! heads, which is a register compare of a value already in the cache line the
//! path was read from -- no dereference of the entry, and none of the name. The
//! head is only trusted between two PLAIN names (bodies with no backslash),
//! because only for those is the ordering text the body itself under every one
//! of `key_cmp`'s cases; an escaped name always takes the exact path below.
//!
//! Zero-padding is what makes the head an order: a plain body cannot contain a
//! NUL (`is_json_string_body` refuses every raw byte below 0x20), so a shorter
//! name pads with a byte no real name can present, and "is a prefix of" comes
//! out as "is less than". Where two heads TIE the comparison falls through to
//! the full text, so the head can only ever decide a case it decides correctly.
//!
//! # What is preserved exactly
//!
//! The ORDER is the contract, because it is what the projection writes and what
//! the delta encoding requires. [`KeyRef::cmp_key`] reproduces
//! `snapshot::canon_value::key_cmp` case for case:
//!
//! | bodies                        | `key_cmp`                    | here                          |
//! |-------------------------------|------------------------------|-------------------------------|
//! | neither escaped               | compare bodies               | `order_text` IS the body      |
//! | both escaped, both decode     | compare decoded texts        | `order_text` is the decoding  |
//! | one escaped, it decodes       | decoded vs the other body    | `order_text` vs `order_text`  |
//! | either escape fails to decode | compare RAW bodies           | `undecodable` forces the same |
//!
//! and adds one fast path that is not a behaviour change: two handles to the
//! same entry are the same body, so they compare equal without touching the
//! text. Interning is what makes that fast path reachable -- one body has
//! exactly one entry.
//!
//! # Leaking
//!
//! Entries are `Box::leak`ed, which is what makes [`KeyRef`] `Copy` and `Send`
//! rather than a refcount. The vocabulary is bounded by the capture's distinct
//! member names, it is reported by [`retained_bytes`] rather than hidden, and it
//! REPLACES the per-path copies it interns -- a name that stood in the producer
//! state, the replica state and the hour's baseline log at once was three
//! allocations of the same bytes and is now one.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use snapshot::canon_value::{decode_json_string_body, is_valid_object_key};

use crate::{write_varint, SEG_KEY};

/// Everything about one distinct member-name body that does not fit inline in a
/// [`KeyRef`], computed once when the body is first interned.
#[derive(Debug)]
pub struct KeyEntry {
    /// The SOURCE TOKEN BODY, exactly as `CanonValue::Object` stores it. This is
    /// what the projection writes back between quotes.
    body: &'static str,
    /// The text this body ORDERS by: its decoding when the body carries an
    /// escape that decodes, and the body itself otherwise.
    order_text: &'static str,
    /// The body carries an escape that does NOT decode. `key_cmp` falls back to
    /// comparing raw bodies whenever either side is in this state, and so does
    /// [`KeyRef::cmp_key`]. Unreachable through `check_path`, which refuses such
    /// a name -- but a lookup happens BEFORE that check, so the order still has
    /// to be total and still has to be the same one.
    undecodable: bool,
    /// `is_valid_object_key(body)`, so `check_path` reads it instead of
    /// rescanning and re-decoding the name on every newly addressed grain.
    valid_member_name: bool,
    /// `SEG_KEY`, the body's length as a varint, and the body -- the exact bytes
    /// `encode_path_into` would have written a piece at a time.
    encoded: &'static [u8],
    /// The two facts [`KeyRef`] carries INLINE, kept here as well so that
    /// handing out a handle for an already-interned name is a copy of two
    /// scalars rather than a rescan of the body and a reread of the ordering
    /// text. Written once, when the entry is built; there is no way to change
    /// either without building a new entry.
    head: u64,
    plain: bool,
}

/// A handle to one interned member name.
///
/// `Copy`, so cloning a [`crate::Path`] is one allocation and a memcpy rather
/// than one allocation per segment, and so the segments live inside the path's
/// own buffer instead of behind a pointer each.
#[derive(Clone, Copy)]
pub struct KeyRef {
    /// First eight bytes of `entry.order_text`, big-endian, zero-padded. See
    /// the module docs: where two heads differ they differ in the same
    /// direction the full texts do.
    head: u64,
    /// The body carries no backslash, so it is its own ordering text under
    /// every case of `key_cmp` and `head` may decide a comparison against
    /// another plain name. Inline rather than in the entry precisely so the
    /// fast path never dereferences.
    plain: bool,
    entry: &'static KeyEntry,
}

/// The first eight bytes of `text`, big-endian, zero-padded.
fn head_of(text: &str) -> u64 {
    let bytes = text.as_bytes();
    let take = bytes.len().min(8);
    let mut word = [0u8; 8];
    word[..take].copy_from_slice(&bytes[..take]);
    u64::from_be_bytes(word)
}

impl KeyRef {
    /// Intern `body` -- the member name's SOURCE TOKEN BODY, not its decoding.
    pub fn intern(body: &str) -> Self {
        {
            let table = interner().lock().expect("the name interner is poisoned");
            if let Some(entry) = table.by_body.get(body) {
                return KeyRef::of(entry);
            }
        }
        Self::intern_slow(body)
    }

    /// The first sighting of a name. Split out so the common path is a lookup
    /// and a return, and so the decode, the validation and the encoding are
    /// obviously paid once per NAME.
    #[cold]
    fn intern_slow(body: &str) -> Self {
        let mut table = interner().lock().expect("the name interner is poisoned");
        // Re-checked under the write lock: another thread may have interned this
        // body between the read above and here, and two entries for one body
        // would break the pointer-equality fast path in `cmp_key`.
        if let Some(entry) = table.by_body.get(body) {
            return KeyRef::of(entry);
        }
        let body: &'static str = Box::leak(body.to_owned().into_boxed_str());
        let escaped = body.as_bytes().contains(&b'\\');
        let decoded = if escaped {
            decode_json_string_body(body)
        } else {
            // An unescaped body IS its own decoding, and `key_cmp` says so by
            // comparing the bodies directly. Decoding it here would allocate a
            // copy of every name in the corpus to prove they are unchanged.
            None
        };
        let undecodable = escaped && decoded.is_none();
        let order_text: &'static str = match decoded {
            Some(text) => Box::leak(text.into_boxed_str()),
            None => body,
        };
        let mut encoded = Vec::with_capacity(body.len() + 6);
        encoded.push(SEG_KEY);
        write_varint(&mut encoded, body.len() as u64);
        encoded.extend_from_slice(body.as_bytes());
        let encoded: &'static [u8] = Box::leak(encoded.into_boxed_slice());
        let entry: &'static KeyEntry = Box::leak(Box::new(KeyEntry {
            body,
            order_text,
            undecodable,
            valid_member_name: is_valid_object_key(body),
            encoded,
            head: head_of(order_text),
            plain: !escaped,
        }));
        table.bytes += (body.len()
            + if escaped { order_text.len() } else { 0 }
            + encoded.len()
            + std::mem::size_of::<KeyEntry>()
            // The table's own row: a `&str` key and a pointer, plus a share of
            // the hash map's slack.
            + 40) as u64;
        table.by_body.insert(body, entry);
        KeyRef::of(entry)
    }

    fn of(entry: &'static KeyEntry) -> Self {
        // A COPY of two scalars the entry already holds. Recomputing them here
        // would put a scan of the body back on the path that exists to avoid
        // one, on every sighting of every already-known name.
        KeyRef {
            head: entry.head,
            plain: entry.plain,
            entry,
        }
    }

    /// The SOURCE TOKEN BODY, for the projection and the delta encoding.
    pub fn body(&self) -> &'static str {
        self.entry.body
    }

    /// `SEG_KEY || varint(len) || body`, precomputed.
    pub(crate) fn encoded(&self) -> &'static [u8] {
        self.entry.encoded
    }

    /// Whether this name is a body the projection could write back.
    pub(crate) fn valid_member_name(&self) -> bool {
        self.entry.valid_member_name
    }

    /// The body carries no backslash, so no other spelling can denote it.
    pub(crate) fn plain(&self) -> bool {
        self.plain
    }

    /// Same body, byte for byte.
    ///
    /// Pointer equality is exact here and only here: interning gives one body
    /// one entry, so two handles are the same entry precisely when they were
    /// interned from the same bytes.
    pub(crate) fn same_spelling(&self, other: &Self) -> bool {
        std::ptr::eq(self.entry, other.entry)
    }

    /// `snapshot::canon_value::key_cmp`, with the escape scan and the decoding
    /// already done. See the table in the module docs for the case analysis.
    pub(crate) fn cmp_key(&self, other: &Self) -> Ordering {
        if self.plain && other.plain {
            // Neither body carries an escape, so `key_cmp` compares the bodies
            // and the bodies ARE the ordering texts. A head difference is
            // therefore the answer, without touching either entry.
            if self.head != other.head {
                return self.head.cmp(&other.head);
            }
            if std::ptr::eq(self.entry, other.entry) {
                return Ordering::Equal;
            }
            return self.entry.order_text.cmp(other.entry.order_text);
        }
        if std::ptr::eq(self.entry, other.entry) {
            return Ordering::Equal;
        }
        if self.entry.undecodable || other.entry.undecodable {
            return self.entry.body.cmp(other.entry.body);
        }
        self.entry.order_text.cmp(other.entry.order_text)
    }
}

impl std::fmt::Debug for KeyRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The BODY, so a failing assertion reads like the document does.
        std::fmt::Debug::fmt(self.entry.body, f)
    }
}

impl From<&str> for KeyRef {
    fn from(body: &str) -> Self {
        KeyRef::intern(body)
    }
}

impl From<String> for KeyRef {
    fn from(body: String) -> Self {
        KeyRef::intern(&body)
    }
}

impl From<&String> for KeyRef {
    fn from(body: &String) -> Self {
        KeyRef::intern(body)
    }
}

struct Interner {
    by_body: HashMap<&'static str, &'static KeyEntry>,
    bytes: u64,
}

fn interner() -> &'static Mutex<Interner> {
    static INTERNER: OnceLock<Mutex<Interner>> = OnceLock::new();
    INTERNER.get_or_init(|| {
        Mutex::new(Interner {
            by_body: HashMap::new(),
            bytes: 0,
        })
    })
}

/// Bytes the interner holds, so the memory a section no longer spends on its own
/// copies of its names is accounted where it went rather than simply vanishing
/// from the report.
pub fn retained_bytes() -> u64 {
    interner()
        .lock()
        .expect("the name interner is poisoned")
        .bytes
}

/// Distinct member-name bodies interned so far.
pub fn interned_names() -> u64 {
    interner()
        .lock()
        .expect("the name interner is poisoned")
        .by_body
        .len() as u64
}
