//! The Wave-2 registration scaffold: fifteen replay carriers the extractor owes,
//! declared once so fifteen later lanes never touch the same region twice.
//!
//! # The hazard this removes
//!
//! `build_snapshot` inserts the `_r2_*` sidecar carriers into the snapshot root
//! inside one contiguous run of lines. Fifteen Wave-3/Wave-5 lanes each need to
//! add one key there. Two concurrent lanes editing that run is a merge-conflict
//! factory, and until it is removed *no* pair of downstream lanes has a disjoint
//! footprint, so none of them may start.
//!
//! The scaffold converts that shared region into a set of disjoint per-carrier
//! FILES. One file per carrier, holding that carrier's declaration and its two
//! functions (`fill`, which reads the producer, and `emit`, which applies the
//! consumer's null-vs-omit rule), plus one frozen call list here.
//! A later lane edits its own carrier file(s) and nothing else — three
//! Wave-3 lanes own two carriers apiece, so "one file per lane" would be wrong;
//! what holds, and what matters, is that no two lanes' file sets intersect.
//! Putting all fifteen
//! carriers in a single `wave2_scaffold.rs` would have moved the collision down
//! one level rather than removing it.
//!
//! # The two functions every carrier owns
//!
//! ```text
//! fn fill(fold: &WorldFold, feed: &mut Wave2Feed)      // reads the producer
//! fn emit(feed: &Wave2Feed) -> SidecarEmission         // applies the wire rule
//! ```
//!
//! They are split because they change on different occasions, by different
//! people. `fill` is what a Wave-3/Wave-5 lane rewrites when its producer lands:
//! it is the only function here that touches `WorldFold`, and it does nothing
//! else. `emit` encodes the consumer's null-vs-omit rule — a fact about replay,
//! not about the producer — and must NOT move when a producer arrives. Keeping
//! them apart is what stops a producer lane from quietly changing a carrier's
//! absence semantics while adding its payload.
//!
//! `emit` is pure over the feed, and that is not stylistic. `build_snapshot`
//! takes `&self`, so nothing in this module can call `WorldFold::note` or
//! `record_domain_problem` (both need `&mut self`), and the snapshot's
//! `provenance` block is assembled well before the sidecar region is reached. A
//! design in which each carrier "records its own problem" at the point of
//! insertion does not compile and would be too late if it did. Refusals are
//! therefore returned as data, collected once by [`build_wave2_sidecars`], and
//! merged into the hour's notes at the point where notes are still open.
//!
//! # An unfed carrier publishes a named absence, never silence — but HOW depends
//! on the consumer
//!
//! The plan this scaffold implements asserted that "each replay decoder already
//! treats null as `DomainUnavailable`". That is true of nine of the fifteen and
//! false of six, and the difference is not cosmetic:
//!
//! * Nine consumers have an explicit `if value.is_null()` arm. For them a typed
//!   JSON null is strictly better than absence: it separates "this recorder
//!   build knows about the carrier and did not obtain it" from "this recorder
//!   build has never heard of it".
//! * Six do not. XUI12 and XUI15 read their payload/authority pair and map
//!   `(None, None)` to a clean `Ok(None)`; a null pair strict-decodes and
//!   becomes `InvalidProducerAuthority`. XUI23 maps absence to the precise
//!   reason `EIR-NE-009: canonical UI-open-independent all-country
//!   command-ability carrier is absent`, and a null to "authority record is
//!   malformed". XMAP05 collapses null and absence to the same error text, so a
//!   null buys nothing and only perturbs the frame hashes described below.
//!
//! For those six, emitting null would replace an honest "nothing was produced"
//! with a false "what was produced is corrupt". They emit no key, and their
//! consumers' own absent-paths already produce the named reason — XUI23's is
//! literally the ledger row: "EIR-NE-009: canonical UI-open-independent
//! all-country command-ability carrier is absent". `WireAbsence` (in
//! `compact_contract::pending_producer`) carries that per-consumer fact, read
//! from the consumer's source.
//!
//! # Why the declarations are NOT written into every hour's notes
//!
//! An earlier form of this scaffold pushed all thirteen carriers' refusals into
//! `provenance.notes` on every hour. It was wrong on the product's own terms.
//! Those refusals are STATIC — identical in hour 1 and hour 700 — and at roughly
//! 250 bytes each they add ~3 KB per hour, ~2 MB per recording, of byte-for-byte
//! identical prose. Smallest pre-compression size is one of the four product
//! pillars, and the recording is already far above its target; spending 2 MB to
//! restate a compile-time constant 700 times is the opposite of the intent.
//!
//! It also does not match how this codebase reports a producer that was never
//! built. A domain that fails THIS HOUR gets a `domain_problems` entry and a
//! note; a producer that does not exist contributes an absent or null key and
//! nothing else. `notes` is a per-hour observation channel, and a static fact
//! put there once per hour is noise that makes real per-hour findings harder to
//! see.
//!
//! So each carrier's named absence lives where it does not repeat: the typed
//! null on the wire for the nine (replay reports `DomainUnavailable`, a named
//! status), the consumer's own absent-path reason for the six, and the full
//! declaration — ledger row, consumer citation, required schema, reason — in
//! this module, enumerable in code and gated by
//! `runs/hu_wave2_scaffold/wave2_scope_enumerate.py`.
//!
//! [`Wave2Batch::refusals`] therefore carries exactly one thing: a KEY COLLISION,
//! which is a real per-hour event and must never happen.
//!
//! # The XUI12/XUI15 pair is one carrier, not two
//!
//! Both consumers reject one-sided presence outright. A scaffold offering two
//! independently callable insert functions would let a later lane fill one half
//! and ship a frame the consumer refuses. Each pair is therefore a single
//! function returning both keys or neither, and
//! [`SidecarEmission::Pair`] is the only way to express it.
//!
//! # What this scaffold deliberately does NOT do
//!
//! It does not add `fold_domain_unknown`/`fold_domain_seen` guards. A guard
//! answers "did domain N report this hour", and every carrier here is owed by a
//! producer that does not exist, so there is no domain whose state means
//! anything about it. Guarding an unfed carrier on an unrelated domain would
//! make its null blink in and out for reasons unconnected to the carrier. The
//! guard arrives with the producer, in the lane that writes it.
//!
//! When that lane adds one, it must know that `fold_domain_seen` is
//! LIFETIME-success state, not current-hour freshness
//! (`world_fold.rs::fold_domain_seen`). A real payload additionally needs a
//! same-hour identity check, exactly as the existing carriers do with their
//! `frame_hour` comparisons.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use compact_contract::pending_producer::PendingProducerSlot;

/// Re-exported so a carrier file can write `use super::WorldFold`.
///
/// A carrier file's `super` is THIS module, not `world_fold`, so without this
/// every one of the thirteen would have to spell `super::super::WorldFold` or
/// `crate::world_fold::WorldFold`. The re-export keeps each carrier file's
/// imports uniform and short, which matters when the whole design is "a lane
/// edits one file and copies its neighbour's shape".
pub(crate) use super::WorldFold;

mod combat_presentation;
mod decision_evaluation;
mod intelligence_presentation;
mod national_focus_observer;
mod occupation_ui_supplement;
mod peace_conference_presentation;
mod politics_presentation;
mod power_balance_state;
mod weather_visual_lifecycle;
/// `pub(crate)` alone among the thirteen: `build_snapshot` derives the three
/// `map.*_by_*` TAG tables from this carrier's census so the mirrors and the
/// envelope's digests cannot disagree. See its module docs.
pub(crate) mod xmap05_control_authority;
mod xui12_production_lines;
mod xui15_trade_catalogue;
mod xui23_command_abilities;

/// Where a carrier's key lives in the snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SidecarScope {
    /// The snapshot root object, alongside the existing `_r2_*` block.
    Root,
    /// The snapshot's `map` object.
    Map,
}

/// What a carrier function put on the wire this hour.
///
/// There is no "emit one half of a pair" variant, by construction.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SidecarEmission {
    /// The consumer's clean-absence state: no key at all. The consumer's own
    /// absent-path produces the named reason.
    NoKey,
    /// One key with one value. `Value::Null` is the typed-unavailable spelling
    /// for a null-tolerant consumer; a real payload uses the same variant.
    Single(Value),
    /// Both halves of an atomic pair, together or not at all.
    Pair {
        payload: Value,
        authority: Value,
    },
}

/// One carrier's declaration, the function that READS its producer off the
/// fold, and the function that turns that reading into wire keys.
///
/// The two functions are separate for a reason that decides whether this whole
/// wave works. `fill` is the ONE line a producer lane changes, and it lives in
/// the carrier's own file. `emit` encodes the null-vs-omit rule, which is a
/// fact about the consumer and must not change when a producer lands. If a lane
/// had to reach into a central feed-construction site to deliver its value, the
/// shared-region hazard would simply have moved from `build_snapshot` to that
/// site — which is exactly what an earlier revision of this scaffold did.
pub(crate) struct Wave2Carrier {
    /// The declaration, including the schema its consumer requires.
    pub(crate) slot: PendingProducerSlot,
    /// The second key, for the atomic pairs only.
    pub(crate) paired_key: Option<&'static str>,
    pub(crate) scope: SidecarScope,
    /// Reads THIS carrier's producer state off the fold into the feed. Every
    /// implementation is a one-line `None` today; a Wave-3/Wave-5 lane replaces
    /// its own and touches no other file.
    pub(crate) fill: fn(&super::WorldFold, &mut Wave2Feed),
    pub(crate) emit: fn(&Wave2Feed) -> SidecarEmission,
}

/// What the producer lanes fill in.
///
/// Every field is `None` today, and every field is written by exactly one
/// carrier's [`Wave2Carrier::fill`] — never by a central assembly site. The
/// struct exists so the emit rule can be tested against an explicit producer
/// state without constructing a whole [`super::WorldFold`].
#[derive(Debug, Clone, Default)]
pub(crate) struct Wave2Feed {
    pub(crate) politics_presentation: Option<Value>,
    pub(crate) combat_presentation: Option<Value>,
    pub(crate) intelligence_presentation: Option<Value>,
    pub(crate) peace_conference_presentation: Option<Value>,
    pub(crate) national_focus_observer: Option<Value>,
    pub(crate) power_balance_state: Option<Value>,
    pub(crate) occupation_ui_supplement: Option<Value>,
    pub(crate) decision_evaluation: Option<Value>,
    pub(crate) weather_visual_lifecycle: Option<Value>,
    pub(crate) xmap05_control_authority: Option<Value>,
    pub(crate) xui23_command_abilities: Option<Value>,
    /// Atomic pair: both halves or neither.
    pub(crate) xui12_production_lines: Option<(Value, Value)>,
    /// Atomic pair: both halves or neither.
    pub(crate) xui15_trade_catalogue: Option<(Value, Value)>,
    /// Per-hour, per-carrier reasons a producer that EXISTS had nothing to
    /// publish this hour.
    ///
    /// Deliberately not the carriers' static declarations — those are identical
    /// in hour 1 and hour 700 and were removed from the wire for exactly that
    /// reason (see the module docs). A landed producer's absence is different:
    /// its cause varies hour to hour, and "the census had 41 regions whose
    /// visual epoch this recorder never witnessed" is not derivable from any
    /// static string. A `fill` that pushes here is saying something only this
    /// hour knows.
    pub(crate) dynamic_refusals: Vec<String>,
    /// The identity witness of every country key the snapshot EMITS, keyed
    /// exactly as it is emitted.
    ///
    /// Filled before any `fill` runs. A producer that certifies anything about
    /// the country census must read this rather than `WorldFold::countries`,
    /// which is only the pre-projection copy.
    pub(crate) emitted_country_identities: BTreeMap<String, EmittedCountryIdentity>,
}

impl Wave2Feed {
    /// The state every hour is in until a producer lane lands.
    pub(crate) fn unfed() -> Self {
        Self::default()
    }
}

/// What one emitted `countries` key carries as its D18 replay-join witness,
/// in the three states the consumer's `exact_d18_country_roster` distinguishes.
///
/// This exists because `WorldFold::countries` is NOT the object the snapshot
/// emits: `build_snapshot` copies it and then a dozen projections add to and
/// edit the copy. A producer that certified a census of `fold.countries` would
/// be describing a different object from the one replay reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EmittedCountryIdentity {
    /// `_r2_tag_db_id` is present and usable. The consumer builds a roster row
    /// `(key, id)` from exactly this.
    Resolved(u32),
    /// No `_r2_tag_db_id` key at all: this hour could not name the country.
    /// EXCLUDABLE at country grain on both sides.
    Absent,
    /// `_r2_tag_db_id` is present and cannot be an identity (null, wrong-typed,
    /// zero, out of u32 range), or the country value is not an object. The
    /// consumer refuses the whole roster on this and so must the producer.
    Malformed,
}

fn emitted_country_identity(country: &Value) -> EmittedCountryIdentity {
    let Some(country) = country.as_object() else {
        return EmittedCountryIdentity::Malformed;
    };
    let Some(raw) = country.get("_r2_tag_db_id") else {
        return EmittedCountryIdentity::Absent;
    };
    match raw
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value != 0)
    {
        Some(id) => EmittedCountryIdentity::Resolved(id),
        None => EmittedCountryIdentity::Malformed,
    }
}

/// `emitted_countries` is the snapshot's OWN `countries` object, mid-build and
/// after every projection has run. Only the identity witness is indexed, not
/// the objects: this runs once per hour inside the 48 ms fold budget and the
/// full object is tens of megabytes.
/// Collect every carrier's producer state off the fold.
///
/// This is the call `build_snapshot` makes, and it never changes: the loop is
/// over the frozen carrier list, and each carrier contributes through its own
/// `fill`. Landing a producer therefore edits one carrier file and nothing
/// else — the property the whole wave exists to create.
pub(crate) fn collect_feed(
    fold: &super::WorldFold,
    emitted_countries: &Map<String, Value>,
) -> Wave2Feed {
    let mut feed = Wave2Feed::unfed();
    feed.emitted_country_identities = emitted_countries
        .iter()
        .map(|(key, country)| (key.clone(), emitted_country_identity(country)))
        .collect();
    for carrier in carriers() {
        (carrier.fill)(fold, &mut feed);
    }
    feed
}

/// The frozen call list. Fifteen keys, thirteen functions (two are atomic pairs).
///
/// This list does not change when a lane fills a carrier — only that carrier's
/// own file does. That is what keeps downstream footprints disjoint.
pub(crate) fn carriers() -> [Wave2Carrier; 13] {
    [
        politics_presentation::CARRIER,
        combat_presentation::CARRIER,
        intelligence_presentation::CARRIER,
        peace_conference_presentation::CARRIER,
        national_focus_observer::CARRIER,
        power_balance_state::CARRIER,
        occupation_ui_supplement::CARRIER,
        decision_evaluation::CARRIER,
        xui23_command_abilities::CARRIER,
        xui12_production_lines::CARRIER,
        xui15_trade_catalogue::CARRIER,
        xmap05_control_authority::CARRIER,
        weather_visual_lifecycle::CARRIER,
    ]
}

/// Everything the scaffold contributes to one hour's snapshot.
///
/// Computed once, early, while the hour's notes are still open — then merged at
/// three natural boundaries. Nothing here depends on when it is merged, which is
/// the property that makes the merge points safe to choose for locality rather
/// than for ordering.
#[derive(Debug, Default, Clone)]
pub(crate) struct Wave2Batch {
    /// Keys destined for the snapshot root.
    pub(crate) root: Vec<(String, Value)>,
    /// Keys destined for the snapshot's `map` object.
    pub(crate) map: Vec<(String, Value)>,
    /// Key collisions — a scaffold entry that would have overwritten a value
    /// the fold already emitted — plus the per-hour reasons carriers put in
    /// [`Wave2Feed::dynamic_refusals`]. Still NOT the carriers' static
    /// declarations: see the module docs for why those never reach the per-hour
    /// wire.
    pub(crate) refusals: Vec<String>,
}

/// Build the hour's scaffold contribution.
///
/// Pure over the feed. `WorldFold` is not borrowed at all, so this can be called
/// anywhere in `build_snapshot` without interacting with the `&self` borrow that
/// runs through it.
pub(crate) fn build_wave2_sidecars(feed: &Wave2Feed) -> Wave2Batch {
    let mut batch = Wave2Batch::default();
    // Carried straight through: `fill` already decided this hour owes a reason,
    // and the reasons ride the same vector the key-collision refusals do, which
    // `merge_root_into_snapshot` files into `provenance.notes`.
    batch.refusals.extend(feed.dynamic_refusals.iter().cloned());
    for carrier in carriers() {
        let entries: Vec<(String, Value)> = match (carrier.emit)(feed) {
            SidecarEmission::NoKey => Vec::new(),
            SidecarEmission::Single(value) => vec![(carrier.slot.wire_key.to_owned(), value)],
            SidecarEmission::Pair { payload, authority } => {
                let paired_key = carrier
                    .paired_key
                    .expect("an atomic pair carrier declares its second key");
                vec![
                    (carrier.slot.wire_key.to_owned(), payload),
                    (paired_key.to_owned(), authority),
                ]
            }
        };
        let entries = match registered_entries_or_refusal(carrier.scope, entries) {
            Ok(entries) => entries,
            Err(refusal) => {
                batch.refusals.push(refusal);
                continue;
            }
        };
        let target = match carrier.scope {
            SidecarScope::Root => &mut batch.root,
            SidecarScope::Map => &mut batch.map,
        };
        target.extend(entries);
    }
    batch
}

/// Fail closed against the SHARED base-frame registry, at emission, per CARRIER.
///
/// Three replay carriers bind a `replay_frame_identity_sha256` computed over the
/// base-frame projection, which is the snapshot minus every key in
/// `snapshot::replay_frame_identity`'s registry. A scaffold entry whose key is
/// NOT in that registry lands inside all three preimages, so the hour it appears
/// in silently invalidates three carriers its lane never heard of — the second
/// half of GP-F10, which no per-hour signal would otherwise show. A test alone
/// would not hold this: it would have to be remembered. This sits on the path
/// every hour takes.
///
/// ALL-OR-NOTHING per carrier, and that is the point rather than tidiness. An
/// earlier revision filtered key by key, so a pair with one registered half
/// would have emitted that half alone — and both pair consumers reject
/// one-sided presence as a malformed carrier
/// (`production_lines.rs::xui12_producer_payload_from_frame`,
/// `trade_compositor.rs::xui15_producer_payload_from_frame`) rather than as a
/// clean absence. That turns "nothing was produced" into "what was produced is
/// corrupt", which is a worse hour than the perturbation being prevented.
///
/// The refusal names both halves of the answer: which keys are unregistered, and
/// which keys are being withheld because of them.
fn registered_entries_or_refusal(
    scope: SidecarScope,
    entries: Vec<(String, Value)>,
) -> Result<Vec<(String, Value)>, String> {
    let registered: &[&str] = match scope {
        SidecarScope::Root => &snapshot::replay_frame_identity::REGISTERED_ROOT_SIDECAR_KEYS,
        SidecarScope::Map => &snapshot::replay_frame_identity::REGISTERED_MAP_SIDECAR_KEYS,
    };
    let unregistered: Vec<&str> = entries
        .iter()
        .map(|(key, _)| key.as_str())
        .filter(|key| !registered.contains(key))
        .collect();
    if unregistered.is_empty() {
        return Ok(entries);
    }
    let withheld: Vec<&str> = entries.iter().map(|(key, _)| key.as_str()).collect();
    Err(format!(
        "{}: emitted by the wave-2 scaffold but absent from \
         portable_replay_format::replay_frame_identity's registered sidecar keys, so it would \
         enter the base-frame preimage and move the XUI12, XUI15 and XUI23 replay-frame \
         identities; this carrier's keys ({}) are withheld TOGETHER until the registry names \
         them, because a half-emitted pair is a malformed carrier rather than a clean absence",
        unregistered.join(", "),
        withheld.join(", ")
    ))
}

/// Merge one half of a batch into a snapshot object, refusing to overwrite.
///
/// `Map::insert` returns the displaced value and every caller in this file
/// checks it. Silently overwriting is the specific accident this guards: if a
/// producer lane ever emits its carrier through the ordinary fold AND leaves its
/// scaffold entry emitting, the scaffold's null would replace a real payload and
/// the capture would look cleanly unavailable rather than broken. That is a
/// worse failure than a loud one, so the collision is named and the existing
/// value is kept.
pub(crate) fn merge_into(
    object: &mut Map<String, Value>,
    entries: Vec<(String, Value)>,
    refusals: &mut Vec<String>,
) {
    for (key, value) in entries {
        if let Some(displaced) = object.insert(key.clone(), value) {
            object.insert(key.clone(), displaced);
            refusals.push(format!(
                "{key}: wave-2 scaffold refused to overwrite a value the fold already emitted; \
                 the scaffold entry for this carrier must be retired in the same change that \
                 makes the fold emit it"
            ));
        }
    }
}

/// Merge the root half of a batch into the assembled snapshot, and put any
/// refusal where the hour can still carry it.
///
/// The root merge happens at the end of the `_r2_*` sidecar region, which is
/// AFTER `provenance.notes` has been built. A collision discovered there
/// therefore cannot be pushed into the `notes` vector — it is already moved. It
/// is appended to the constructed `provenance.notes` array instead.
///
/// If that array is somehow not reachable — impossible by construction, since
/// the same function builds it unconditionally a few hundred lines earlier, but
/// not something to assume silently — the refusals go to a dedicated root key
/// rather than being dropped. A refusal that cannot be filed is still a refusal.
pub(crate) fn merge_root_into_snapshot(snapshot: &mut Value, entries: Vec<(String, Value)>) {
    let mut refusals = Vec::new();
    let Some(object) = snapshot.as_object_mut() else {
        // The caller only reaches this with an object snapshot; nothing to do
        // and nothing that could be filed anywhere if it were not.
        return;
    };
    merge_into(object, entries, &mut refusals);
    if refusals.is_empty() {
        return;
    }
    let filed = object
        .get_mut("provenance")
        .and_then(Value::as_object_mut)
        .and_then(|provenance| provenance.get_mut("notes"))
        .and_then(Value::as_array_mut)
        .map(|notes| {
            for refusal in &refusals {
                notes.push(Value::String(refusal.clone()));
            }
        })
        .is_some();
    if !filed {
        // APPEND, never replace. A bare insert here would discard an array a
        // previous call had already filed, which would lose refusals in exactly
        // the branch that exists so refusals are never lost.
        let mut unfiled = match object.remove("_r2_wave2_scaffold_unfiled_refusals") {
            Some(Value::Array(existing)) => existing,
            Some(other) => vec![other],
            None => Vec::new(),
        };
        unfiled.extend(refusals.into_iter().map(Value::String));
        object.insert(
            "_r2_wave2_scaffold_unfiled_refusals".into(),
            Value::Array(unfiled),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::pending_producer::WireAbsence;
    use std::collections::BTreeSet;

    /// The fifteen keys this scaffold owes, as the consumers spell them.
    const EXPECTED_KEYS: [&str; 15] = [
        "_r2_politics_presentation_exact",
        "_r2_combat_presentation_exact",
        "_r2_intelligence_presentation_exact",
        "_r2_peace_conference_presentation_exact",
        "_r2_national_focus_observer_exact",
        "_r2_power_balance_state_live",
        "_r2_occupation_ui_supplement_live",
        "_r2_decision_evaluation_exact",
        "_r2_xui23_passive_command_abilities_authority",
        "_r2_xui12_production_lines_payload",
        "_r2_xui12_producer_canvas_authority",
        "_r2_xui15_trade_catalogue_payload",
        "_r2_xui15_trade_catalogue_authority",
        "_r2_xmap05_control_authority",
        "_r2_weather_visual_lifecycle",
    ];

    fn declared_keys() -> BTreeSet<String> {
        let mut keys = BTreeSet::new();
        for carrier in carriers() {
            keys.insert(carrier.slot.wire_key.to_owned());
            if let Some(paired) = carrier.paired_key {
                keys.insert(paired.to_owned());
            }
        }
        keys
    }

    /// kills: dropping a carrier from `carriers()`, or dropping a pair's second
    /// key -- the Wave-3/Wave-5 lane for that carrier would go back to editing
    /// the shared `_r2_*` region in `build_snapshot`, which is the exact hazard
    /// this wave exists to remove.
    #[test]
    fn every_owed_carrier_key_is_registered_exactly_once() {
        let declared = declared_keys();
        let expected: BTreeSet<String> =
            EXPECTED_KEYS.iter().map(|key| (*key).to_owned()).collect();
        assert_eq!(declared, expected);
        assert_eq!(declared.len(), 15, "fifteen keys across thirteen carriers");
    }

    /// kills: adding a carrier here without registering its key in the shared
    /// base-frame registry — the exclusion set the three replay-frame
    /// identities are computed against. An unregistered key enters all three
    /// preimages, so the first hour it appears in invalidates XUI12's, XUI15's
    /// and XUI23's carriers, none of which the adding lane touched (GP-F10).
    /// Asserted BOTH ways and per scope: the registry is not allowed to drift
    /// ahead of the emitters either, because a key excluded from the identity
    /// that nothing emits is a hole nobody is watching.
    #[test]
    fn every_registered_key_is_emitted_here_and_every_emitted_key_is_registered() {
        use snapshot::replay_frame_identity::{
            REGISTERED_MAP_SIDECAR_KEYS, REGISTERED_ROOT_SIDECAR_KEYS,
        };
        let mut declared_root = BTreeSet::new();
        let mut declared_map = BTreeSet::new();
        for carrier in carriers() {
            let target = match carrier.scope {
                SidecarScope::Root => &mut declared_root,
                SidecarScope::Map => &mut declared_map,
            };
            target.insert(carrier.slot.wire_key.to_owned());
            if let Some(paired) = carrier.paired_key {
                target.insert(paired.to_owned());
            }
        }
        let registered_root: BTreeSet<String> = REGISTERED_ROOT_SIDECAR_KEYS
            .iter()
            .map(|key| (*key).to_owned())
            .collect();
        let registered_map: BTreeSet<String> = REGISTERED_MAP_SIDECAR_KEYS
            .iter()
            .map(|key| (*key).to_owned())
            .collect();
        assert_eq!(
            declared_root, registered_root,
            "the root carriers and the shared base-frame registry disagree"
        );
        assert_eq!(
            declared_map, registered_map,
            "the map carriers and the shared base-frame registry disagree"
        );
    }

    /// kills: dropping the emission-time registry check in
    /// `build_wave2_sidecars`, and — the defect review round 2 caught — making
    /// it filter key by key instead of carrier by carrier. A pair with one
    /// registered half would then emit that half alone, and both pair consumers
    /// read one-sided presence as a MALFORMED carrier rather than a clean
    /// absence, which is a worse hour than the perturbation the guard prevents.
    ///
    /// Exercised through `registered_entries_or_refusal`, the same function
    /// `build_wave2_sidecars` calls — an earlier version of this test
    /// reimplemented the filtering and so could not have seen the split-pair
    /// defect at all.
    #[test]
    fn an_unregistered_scaffold_key_withholds_its_whole_carrier_and_names_it() {
        // The production path, on the real frozen carrier list: no refusal.
        let hour = build_wave2_sidecars(&Wave2Feed::unfed());
        assert!(hour.refusals.is_empty(), "{:?}", hour.refusals);

        // A fully registered pair passes through untouched.
        let good = vec![
            ("_r2_xui12_production_lines_payload".to_owned(), Value::Null),
            (
                "_r2_xui12_producer_canvas_authority".to_owned(),
                Value::Null,
            ),
        ];
        assert_eq!(
            registered_entries_or_refusal(SidecarScope::Root, good.clone()),
            Ok(good)
        );

        // A pair with exactly ONE unregistered half is withheld ENTIRELY.
        let half_registered = vec![
            ("_r2_xui12_production_lines_payload".to_owned(), Value::Null),
            ("_r2_not_registered_anywhere".to_owned(), Value::Null),
        ];
        let refusal = registered_entries_or_refusal(SidecarScope::Root, half_registered)
            .expect_err("one unregistered half withholds the pair");
        assert!(refusal.contains("_r2_not_registered_anywhere"), "{refusal}");
        assert!(
            refusal.contains("_r2_xui12_production_lines_payload"),
            "the registered half is withheld too, and the refusal must say so: {refusal}"
        );
        assert!(refusal.contains("withheld TOGETHER"), "{refusal}");
        // And the refusal states the consequence, not merely "unknown key": a
        // reader has to learn WHY the key cannot ride this hour.
        assert!(
            refusal.contains("XUI12, XUI15 and XUI23 replay-frame identities"),
            "the refusal must name the three identities it protects: {refusal}"
        );

        // A map-scoped key is checked against the MAP registry, not the root
        // one: swapping the two lists would let a root key ride in `map`.
        assert!(registered_entries_or_refusal(
            SidecarScope::Map,
            vec![("_r2_politics_presentation_exact".to_owned(), Value::Null)]
        )
        .is_err());
        assert!(registered_entries_or_refusal(
            SidecarScope::Map,
            vec![("_r2_weather_visual_lifecycle".to_owned(), Value::Null)]
        )
        .is_ok());
    }

    /// kills: leaving a carrier's `slot` with an empty reason, a blank ledger
    /// row, or a consumer citation whose tail is not a line number -- the
    /// carrier would stop being able to say why it is unfed or where its
    /// contract is read from, which is the difference between a declared
    /// absence and silence.
    #[test]
    fn every_carrier_declaration_is_well_formed() {
        for carrier in carriers() {
            carrier.slot.validate().unwrap_or_else(|error| {
                panic!("wave-2 carrier declaration is malformed: {error}")
            });
            // The citation has to be WELL FORMED, not merely colon-shaped: an
            // earlier `contains(':')` check accepted a non-numeric line tail.
            // Well formed is not the same as openable, and this deliberately does
            // not try to be -- whether the file exists and defines the item is
            // `scripts/verify_citation_integrity.py`'s job, across the whole tree.
            //
            // Two spellings, `path:line` and `path::item`. The rule lives in ONE
            // place -- `compact_contract::pending_producer::citation_is_well_formed`,
            // the same predicate `slot.validate()` uses. An earlier revision
            // reimplemented it here and the two copies immediately disagreed: this
            // one accepted an item containing spaces. A shape rule with two
            // implementations is a shape rule with two answers.
            let citation = carrier.slot.consumer_citation;
            assert!(
                compact_contract::pending_producer::citation_is_well_formed(citation),
                "{} cites {:?}, which is neither path:line nor path::item",
                carrier.slot.wire_key,
                citation
            );
        }
    }

    /// kills: changing an atomic pair into two independent single carriers --
    /// XUI12 and XUI15 both reject one-sided presence
    /// (`production_lines.rs:748`, `trade_compositor.rs:522`), so a lane able to
    /// fill one half alone could ship a frame the consumer refuses outright.
    #[test]
    fn the_two_atomic_pairs_are_one_carrier_each_and_never_one_sided() {
        let all = carriers();
        let paired: Vec<&Wave2Carrier> = all
            .iter()
            .filter(|carrier| carrier.paired_key.is_some())
            .collect();
        assert_eq!(paired.len(), 2, "exactly XUI12 and XUI15 are pairs");
        let feed = Wave2Feed::unfed();
        for carrier in paired {
            match (carrier.emit)(&feed) {
                SidecarEmission::NoKey => {}
                SidecarEmission::Pair { .. } => {}
                SidecarEmission::Single(_) => panic!(
                    "{} is an atomic pair and must never emit a single key",
                    carrier.slot.wire_key
                ),
            }
        }
    }

    /// kills: flipping a carrier's `wire_absence` from `OmitKey` to
    /// `TypedNull` -- the six non-null-tolerant consumers would receive a null
    /// that strict-decodes as a malformed carrier, turning "nothing was
    /// produced" into a false corruption report. Verified against what the
    /// carrier actually emits when unfed, not against the declaration alone.
    #[test]
    fn an_unfed_carrier_emits_a_key_only_where_its_consumer_tolerates_null() {
        let feed = Wave2Feed::unfed();
        let mut null_tolerant = 0;
        let mut omitted = 0;
        for carrier in carriers() {
            let emission = (carrier.emit)(&feed);
            match carrier.slot.wire_absence {
                WireAbsence::TypedNull => {
                    null_tolerant += 1;
                    assert_eq!(
                        emission,
                        SidecarEmission::Single(Value::Null),
                        "{} declares a null-tolerant consumer and must emit a typed null",
                        carrier.slot.wire_key
                    );
                }
                WireAbsence::OmitKey => {
                    omitted += 1;
                    assert_eq!(
                        emission,
                        SidecarEmission::NoKey,
                        "{} declares a consumer that rejects null and must emit no key",
                        carrier.slot.wire_key
                    );
                }
            }
        }
        assert_eq!(null_tolerant, 9, "nine consumers have an explicit null arm");
        assert_eq!(omitted, 4, "four carrier functions cover the six omitted keys");
    }

    /// kills: making an unfed carrier push its static declaration into
    /// `refusals` (and so into every hour's `provenance.notes`). Those
    /// declarations are compile-time constants; restating them ~700 times per
    /// recording costs roughly 2 MB of byte-identical prose against a product
    /// pillar that is already far above its size target, and it buries the real
    /// per-hour findings `notes` exists to carry. An unfed hour must contribute
    /// its nine typed nulls and NOTHING else.
    #[test]
    fn an_unfed_hour_costs_nine_nulls_and_no_prose() {
        let batch = build_wave2_sidecars(&Wave2Feed::unfed());
        assert!(
            batch.refusals.is_empty(),
            "a healthy hour carries no scaffold refusal; got {:?}",
            batch.refusals
        );
        assert_eq!(batch.root.len(), 8, "eight null-tolerant root keys");
        assert_eq!(batch.map.len(), 1, "one null-tolerant map key");
        for (_, value) in batch.root.iter().chain(batch.map.iter()) {
            assert!(value.is_null(), "an unfed carrier emits only typed nulls");
        }
    }

    /// kills: replacing a carrier's `fill` with a shared central assembly (or
    /// pointing several carriers at one `fill`) -- a producer lane would then
    /// have to edit a site outside its own file to deliver its value, which is
    /// the shared-region hazard this wave exists to remove, relocated rather
    /// than removed. Thirteen carriers must have thirteen DISTINCT `fill`
    /// function pointers.
    #[test]
    fn every_carrier_owns_a_distinct_fill_so_no_lane_edits_a_shared_site() {
        let all = carriers();
        let mut seen: Vec<usize> = all
            .iter()
            .map(|carrier| carrier.fill as usize)
            .collect();
        let total = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(
            seen.len(),
            total,
            "two carriers share a fill function, so one lane's edit would move another's carrier"
        );
        assert_eq!(total, 13);
    }

    /// kills: dropping a carrier's `reason`, `ledger_row`, `consumer_citation`
    /// or `required_schema` from its declaration. The declaration is where a
    /// carrier's named absence lives now that it is not restated per hour, so
    /// it has to carry every part of the answer on its own.
    #[test]
    fn every_declaration_states_the_whole_answer_without_the_wire() {
        for carrier in carriers() {
            let refusal = carrier.slot.refusal();
            assert!(refusal.contains(carrier.slot.wire_key), "{refusal}");
            assert!(refusal.contains(carrier.slot.ledger_row), "{refusal}");
            assert!(refusal.contains(carrier.slot.consumer_citation), "{refusal}");
            assert!(refusal.contains(carrier.slot.reason), "{refusal}");
            assert!(refusal.contains("requires"), "{refusal}");
        }
    }

    /// kills: making `merge_into` use a bare `object.insert(..)` and ignore the
    /// displaced value -- a scaffold null would silently replace a real payload
    /// the fold already emitted, and the capture would read as cleanly
    /// unavailable instead of broken.
    #[test]
    fn merging_never_overwrites_a_value_the_fold_already_emitted() {
        let mut object = Map::new();
        object.insert("_r2_politics_presentation_exact".into(), Value::from(1));
        let mut refusals = Vec::new();
        merge_into(
            &mut object,
            vec![("_r2_politics_presentation_exact".into(), Value::Null)],
            &mut refusals,
        );
        assert_eq!(
            object.get("_r2_politics_presentation_exact"),
            Some(&Value::from(1)),
            "the real value survives"
        );
        assert_eq!(refusals.len(), 1);
        assert!(refusals[0].contains("refused to overwrite"), "{}", refusals[0]);
    }

    /// kills: making `merge_root_into_snapshot` drop a collision refusal when
    /// `provenance.notes` is unreachable -- the hour would silently lose the one
    /// record that a scaffold entry collided with a real producer.
    #[test]
    fn a_root_collision_is_filed_in_provenance_or_on_a_dedicated_key() {
        let mut with_provenance = serde_json::json!({
            "provenance": {"notes": ["existing"]},
            "_r2_politics_presentation_exact": 1,
        });
        merge_root_into_snapshot(
            &mut with_provenance,
            vec![("_r2_politics_presentation_exact".into(), Value::Null)],
        );
        let notes = with_provenance["provenance"]["notes"]
            .as_array()
            .expect("provenance notes survive");
        assert_eq!(notes.len(), 2);
        assert!(notes[1].as_str().unwrap().contains("refused to overwrite"));
        assert_eq!(with_provenance["_r2_politics_presentation_exact"], 1);

        let mut without_provenance = serde_json::json!({
            "_r2_politics_presentation_exact": 1,
        });
        merge_root_into_snapshot(
            &mut without_provenance,
            vec![("_r2_politics_presentation_exact".into(), Value::Null)],
        );
        let unfiled = without_provenance["_r2_wave2_scaffold_unfiled_refusals"]
            .as_array()
            .expect("an unfilable refusal is never dropped");
        assert_eq!(unfiled.len(), 1);

        // A second unfilable collision APPENDS. A bare insert would discard the
        // first, losing refusals in the branch whose whole purpose is not to.
        without_provenance["_r2_politics_presentation_exact"] = Value::from(2);
        merge_root_into_snapshot(
            &mut without_provenance,
            vec![("_r2_politics_presentation_exact".into(), Value::Null)],
        );
        let unfiled = without_provenance["_r2_wave2_scaffold_unfiled_refusals"]
            .as_array()
            .expect("the fallback array survives a second collision");
        assert_eq!(unfiled.len(), 2, "the earlier refusal was not overwritten");
    }

    /// kills: making `emit` ignore the feed and always return `Value::Null` --
    /// the carrier would keep publishing "unavailable" after its producer
    /// landed, and the Wave-3 lane's payload would never reach replay. This is
    /// the seam every producer lane depends on, so it is pinned here rather
    /// than left for the first lane to discover.
    #[test]
    fn a_fed_carrier_emits_its_payload_in_place_of_the_null() {
        let mut feed = Wave2Feed::unfed();
        feed.politics_presentation = Some(serde_json::json!({"schema_version": 7}));
        let batch = build_wave2_sidecars(&feed);
        let emitted = batch
            .root
            .iter()
            .find(|(key, _)| key == "_r2_politics_presentation_exact")
            .expect("a fed carrier still emits its key");
        assert_eq!(emitted.1, serde_json::json!({"schema_version": 7}));
        assert_eq!(batch.root.len(), 8, "no key appears or vanishes when one is fed");
        assert!(batch.refusals.is_empty());
    }

    /// kills: dropping either map-scoped carrier's second required identity
    /// (the `schema` token it must carry ALONGSIDE its integer version) --
    /// `RequiredSchema` holds one spelling, both consumers check two, and a
    /// producer satisfying only the integer is refused
    /// (`xmap05.rs:1162`–`:1166`, `world.rs:9303`–`:9305`).
    ///
    /// It also kills a STALE declared version. The two map carriers used to
    /// share one version and this test asserted `Some(1)` for both; XMAP05 went
    /// to MAPC1 v2 and the shared assertion happily kept passing while the
    /// carrier declared a contract its own consumer refuses. Each carrier is now
    /// pinned to its own version, so a producer that bumps `SCHEMA_VERSION`
    /// without telling this table fails here -- which is exactly what it did
    /// when XMAP05 went from MAPC1 v2 to v3.
    #[test]
    fn the_map_carriers_record_the_schema_token_their_integer_version_does_not_carry() {
        assert_eq!(xmap05_control_authority::REQUIRED_SCHEMA_TOKEN, "MAPC1");
        assert_eq!(
            weather_visual_lifecycle::REQUIRED_SCHEMA_TOKEN,
            "hoi4.weather-visual-lifecycle.a729.v1"
        );
        let expected_version = |wire_key: &str| match wire_key {
            "_r2_xmap05_control_authority" => Some(3),
            _ => Some(1),
        };
        let mut map_carriers = 0;
        for carrier in carriers() {
            if carrier.scope == SidecarScope::Map {
                map_carriers += 1;
                assert_eq!(
                    carrier.slot.required_schema.version(),
                    expected_version(carrier.slot.wire_key),
                    "{} declares the wrong schema_version",
                    carrier.slot.wire_key
                );
            }
        }
        assert_eq!(map_carriers, 2, "the map-scoped carrier census drifted");
    }

    /// kills: letting a pair be fed one-sided through `Wave2Feed` -- the type
    /// makes it impossible, and this pins that the type, not a runtime check,
    /// is what enforces it. A future edit splitting the tuple into two
    /// independent `Option`s would fail to compile against this test.
    #[test]
    fn a_pair_feed_carries_both_halves_or_neither() {
        let mut feed = Wave2Feed::unfed();
        feed.xui12_production_lines = Some((
            serde_json::json!({"payload": true}),
            serde_json::json!({"authority": true}),
        ));
        let batch = build_wave2_sidecars(&feed);
        assert!(batch
            .root
            .iter()
            .any(|(key, _)| key == "_r2_xui12_production_lines_payload"));
        assert!(batch
            .root
            .iter()
            .any(|(key, _)| key == "_r2_xui12_producer_canvas_authority"));
    }
}
