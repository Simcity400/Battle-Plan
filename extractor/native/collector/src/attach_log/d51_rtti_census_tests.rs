//! Lane SE2, owner ruling 2 (MEASURE FIRST): d51's cached-RTTI cold-arm census.
//!
//! The ruling did not fund the window merges. It funded the ONE number that
//! decides them: how many scopes per frame enter
//! [`domain51_object_matches_cached_rtti`] with no cached vtable, and how many
//! of those are admitted while still leaving the cache unset -- the branch that
//! makes the cold count unbounded by anything in the code.
//!
//! Two of the gate's three cold outcomes are drivable here, because the type
//! name is read through the caller's [`domain51::MemoryRead`] while only the
//! vtable seed goes through the live-process primitive `read_u64_stable`. So a
//! fixture decides "reject" versus "admit", and this process -- which has
//! nothing mapped at the fixture's addresses -- decides "unseeded". The third
//! outcome (a SEEDED admit) needs a real mapped vtable and is covered by the
//! warm test instead: whatever seeds the cache, the warm arm that follows must
//! not be counted.

use super::*;

/// A hand-built module image and heap object, enough for
/// [`domain51::rtti_type_name`] to resolve a chosen type name and no more.
///
/// Addresses are deliberately far outside anything this test process maps, so
/// `read_u64_stable` against them refuses. Every test that depends on that
/// asserts it rather than assuming it.
struct RttiFixture {
    segments: Vec<(u64, Vec<u8>)>,
}

const FIXTURE_MODULE_BASE: u64 = 0x7FF7_0000_0000;
const FIXTURE_OBJECT: u64 = 0x2000_0000_1000;

impl RttiFixture {
    /// An object whose RTTI chain resolves to `type_name`.
    fn with_type_name(type_name: &str) -> Self {
        let vtable = FIXTURE_MODULE_BASE + 0x5000;
        let col = FIXTURE_MODULE_BASE + 0x6000;
        let type_descriptor = FIXTURE_MODULE_BASE + 0x7000;

        let mut object = vec![0u8; 0x40];
        object[..8].copy_from_slice(&vtable.to_le_bytes());

        // `rtti_type_name` reads the complete-object locator at `vtable - 8`.
        let mut vtable_prefix = vec![0u8; 16];
        vtable_prefix[..8].copy_from_slice(&col.to_le_bytes());

        let mut col_bytes = vec![0u8; 0x20];
        col_bytes[0x0C..0x10].copy_from_slice(&0x7000u32.to_le_bytes());

        // `read_cstr` asks for 200 bytes in one go, so the descriptor segment
        // has to be able to serve them.
        let mut descriptor = vec![0u8; 0x200];
        descriptor[0x10..0x10 + type_name.len()].copy_from_slice(type_name.as_bytes());

        Self {
            segments: vec![
                (FIXTURE_OBJECT, object),
                (vtable - 8, vtable_prefix),
                (col, col_bytes),
                (type_descriptor, descriptor),
            ],
        }
    }
}

impl domain51::MemoryRead for RttiFixture {
    fn read_exact(&self, addr: u64, len: usize) -> Option<Vec<u8>> {
        for (start, bytes) in &self.segments {
            if addr < *start {
                continue;
            }
            let offset = usize::try_from(addr - *start).ok()?;
            if let Some(slice) = bytes.get(offset..offset.checked_add(len)?) {
                return Some(slice.to_vec());
            }
        }
        None
    }
}

/// The fixture's addresses must be unmapped for the "unseeded admit" tests to
/// mean what they say. Asserted, never assumed: if this process ever did map
/// them the tests below would quietly start proving something else.
fn assert_vtable_seed_refuses() {
    assert!(
        read_u64_stable(FIXTURE_OBJECT as usize).is_none(),
        "this test drives the branch where the vtable seed refuses; \
         {FIXTURE_OBJECT:#x} is mapped in this process and it no longer does"
    );
}

#[test]
fn a_cold_scope_rejected_on_its_type_name_counts_as_cold_and_not_as_an_admit() {
    let mem = RttiFixture::with_type_name(".?AVSomethingElse@@");
    let mut vtable = None;
    let mut census = Domain51RttiColdArmCensus::default();

    let admitted = domain51_object_matches_cached_rtti(
        &mem,
        FIXTURE_MODULE_BASE,
        FIXTURE_OBJECT as usize,
        domain51::RTTI_VARIABLES,
        &mut vtable,
        &mut census,
    );

    assert!(!admitted, "a wrong type name closes the scope");
    assert_eq!(
        census.cold_arm_scopes, 1,
        "a rejection still paid the cold arm's two type-name walks"
    );
    assert_eq!(
        census.unseeded_admits, 0,
        "nothing was admitted, so nothing was admitted unseeded"
    );
    assert_eq!(vtable, None, "a rejected object must not seed the cache");
}

#[test]
fn a_cold_scope_admitted_without_a_cachable_vtable_counts_in_both_and_stays_cold() {
    // The branch the ruling names. The gate admits on the type-name evidence
    // alone when the seeding read refuses, and the cache is left unset -- so
    // the NEXT scope of this family pays the cold arm again, and the one after
    // that, with nothing in the code bounding the count. The second call is the
    // point of this test: one call would only show the branch exists.
    assert_vtable_seed_refuses();
    let mem = RttiFixture::with_type_name(domain51::RTTI_VARIABLES);
    let mut vtable = None;
    let mut census = Domain51RttiColdArmCensus::default();

    for expected in 1..=2u32 {
        let admitted = domain51_object_matches_cached_rtti(
            &mem,
            FIXTURE_MODULE_BASE,
            FIXTURE_OBJECT as usize,
            domain51::RTTI_VARIABLES,
            &mut vtable,
            &mut census,
        );
        assert!(admitted, "the type name confirmed twice is the admission");
        assert_eq!(vtable, None, "there was no vtable to cache");
        assert_eq!(
            (census.cold_arm_scopes, census.unseeded_admits),
            (expected, expected),
            "an unseeded admit leaves the next scope cold as well"
        );
    }
}

#[test]
fn a_warm_scope_is_not_counted_at_all() {
    // THE KILL for where the counter sits. The census is only worth reading if
    // it separates the two arms: a capture that counted warm hits too would
    // report ~3042 every frame whether the gate was healthy or pathological,
    // which is the same as reporting nothing. Moving the increment above the
    // early return -- the obvious "simplification" -- fails here.
    assert_vtable_seed_refuses();
    let mem = RttiFixture::with_type_name(domain51::RTTI_VARIABLES);
    let mut vtable = Some(0xDEAD_BEEF_usize);
    let mut census = Domain51RttiColdArmCensus::default();

    let admitted = domain51_object_matches_cached_rtti(
        &mem,
        FIXTURE_MODULE_BASE,
        FIXTURE_OBJECT as usize,
        domain51::RTTI_VARIABLES,
        &mut vtable,
        &mut census,
    );

    assert!(
        !admitted,
        "the warm arm compares the object's vtable against the cache, and this \
         object's is unreadable here"
    );
    assert_eq!(
        (census.cold_arm_scopes, census.unseeded_admits),
        (0, 0),
        "the warm arm walks no type name, so it is not a cold-arm entry"
    );
}

/// A census with two DIFFERENT values, so a test that reads them back can tell
/// the two units apart. `648` is the healthy whole-capture cold count for the
/// reference capture (2 per frame x 324 frames); `7` is not a number any
/// healthy run produces, which is the point - swapping the two bindings in
/// production moves both numbers.
const CENSUS: Domain51RttiColdArmCensus = Domain51RttiColdArmCensus {
    cold_arm_scopes: 648,
    unseeded_admits: 7,
};

/// The record the payload tests price. Values are arbitrary; only the
/// diagnostic trailer differs between the two payloads compared.
fn census_perf_record() -> CompactDomainPerfRecord {
    CompactDomainPerfRecord {
        domain_id: COMPACT_DOMAIN_SCRIPTED_STATE_FLAGS_VARIABLES_LIVE,
        read_time_us: 18_160,
        row_count: 3_042,
        changed_rows: 60,
        unit_count: 60,
        changed_unit_count: 60,
        status_flags: 0,
    }
}

/// One published unit, flattened to something comparable. `DeclaredDiagnosticUnit`
/// is deliberately left without `PartialEq`/`Debug` derives, and this lane is
/// not adding them to a struct eleven producers share.
fn shape(unit: &DeclaredDiagnosticUnit) -> (&'static str, u64, u8, u8) {
    (
        unit.name,
        unit.value,
        unit.role.code(),
        unit.presence.code(),
    )
}

/// A read shaped exactly as the worker hands one to the publish site, so the
/// tests below call the builder the way production calls it. The frame is the
/// empty one; nothing in the diagnostic list reads it.
fn read_carrying(
    country_roster_failure: Option<Domain51CountryRosterFailure>,
    rtti_census: Domain51RttiColdArmCensus,
) -> CompactScriptedStateRead {
    CompactScriptedStateRead {
        frame: domain51_empty_frame(0, 0),
        read_time_us: 18_160,
        status_flags: 0,
        country_roster_failure,
        rtti_census,
    }
}

fn published_read(
    country_roster_failure: Option<Domain51CountryRosterFailure>,
) -> CompactScriptedStateRead {
    read_carrying(country_roster_failure, CENSUS)
}

fn roster_failure() -> Domain51CountryRosterFailure {
    Domain51CountryRosterFailure::new(Domain51CountryRosterFailureKind::HeaderCountsDisagree)
        .with_counts(512, 440, 441)
}

const COUNTER: u8 = 4; // UnitRole::Counter
const OBSERVED: u8 = 1; // UnitPresence::Observed
const UNDECLARED_ROLE: u8 = 5; // UnitRole::Undeclared
const UNDECLARED_PRESENCE: u8 = 5; // UnitPresence::Undeclared

/// What the census must look like on the wire, once, so the tests below cannot
/// drift apart from each other.
fn expected_census_shapes() -> Vec<(&'static str, u64, u8, u8)> {
    vec![
        ("d51_rtti_cold_arm_scopes", 648, COUNTER, OBSERVED),
        ("d51_rtti_cache_unseeded_admits", 7, COUNTER, OBSERVED),
    ]
}

#[test]
fn a_healthy_frame_publishes_the_census_and_nothing_else() {
    // The roster units appear only when the country roster fails. A census that
    // came and went with them could not tell "the gate was never cold" from
    // "the counter was not running", so on a healthy frame the published list
    // is exactly these two units.
    //
    // kills: publishing the census only inside the failure arm; binding both
    // units to the same field; swapping the two fields (the values differ);
    // renaming either unit; publishing a constant.
    let published = domain51_perf_diagnostic_units(&published_read(None));

    assert_eq!(
        published.iter().map(shape).collect::<Vec<_>>(),
        expected_census_shapes()
    );
}

#[test]
fn a_failing_roster_keeps_its_seven_units_first_and_its_bytes_undeclared() {
    // Order is the contract:
    // `domain51_country_roster_diagnostics_are_separate_for_d51_and_d63` reads
    // the roster units index-positionally, and the untyped append every other
    // producer still uses lifts its input as Undeclared/Undeclared. So the
    // census is APPENDED and the roster is not migrated.
    //
    // kills: inserting the census in front of the roster; giving the roster
    // units a role "while we are here"; dropping the census when the roster
    // also has something to say.
    let published = domain51_perf_diagnostic_units(&published_read(Some(roster_failure())));
    let roster = domain51_country_roster_diagnostic_units(
        COMPACT_DOMAIN_SCRIPTED_STATE_FLAGS_VARIABLES_LIVE,
        roster_failure(),
    );

    assert_eq!(published.len(), 9);
    let expected: Vec<(&'static str, u64, u8, u8)> = roster
        .iter()
        .map(|unit| (unit.name, unit.value, UNDECLARED_ROLE, UNDECLARED_PRESENCE))
        .chain(expected_census_shapes())
        .collect();
    assert_eq!(published.iter().map(shape).collect::<Vec<_>>(), expected);
}

#[test]
fn the_census_is_a_pure_suffix_on_the_bytes_the_untyped_path_gave_the_roster() {
    // The publish site changed from the untyped append to the declared one so
    // it could carry two typed units. That is only safe because the untyped
    // append is itself a wrapper that lifts its input through
    // `DeclaredDiagnosticUnit::undeclared` and calls the declared builder. This
    // holds it to that at the byte level.
    let roster: Vec<DeclaredDiagnosticUnit> = domain51_country_roster_diagnostic_units(
        COMPACT_DOMAIN_SCRIPTED_STATE_FLAGS_VARIABLES_LIVE,
        roster_failure(),
    )
    .into_iter()
    .map(DeclaredDiagnosticUnit::undeclared)
    .collect();
    let published = domain51_perf_diagnostic_units(&published_read(Some(roster_failure())));

    let record = census_perf_record();
    let (before, before_flags) = compact_domain_perf_payload_with_declared(&record, 7, &roster);
    let (after, after_flags) = compact_domain_perf_payload_with_declared(&record, 7, &published);

    assert_eq!(
        before_flags, after_flags,
        "the trailer flag does not depend on how many units are in the trailer"
    );
    // 40 scalar bytes, then the trailer's 4-byte magic and 2-byte version.
    const COUNT_OFFSET: usize = 46;
    assert_eq!(
        before[..COUNT_OFFSET],
        after[..COUNT_OFFSET],
        "the scalar half and the trailer header are untouched"
    );
    let unit_count =
        |bytes: &[u8]| u16::from_le_bytes([bytes[COUNT_OFFSET], bytes[COUNT_OFFSET + 1]]);
    assert_eq!(unit_count(&before), 7, "the roster is seven units");
    assert_eq!(
        unit_count(&after),
        9,
        "the census is two more, not a replacement"
    );
    assert_eq!(
        before[COUNT_OFFSET + 2..],
        after[COUNT_OFFSET + 2..before.len()],
        "every roster unit keeps the bytes the untyped path gave it"
    );
    // 12 fixed bytes per unit plus its name.
    let census_bytes =
        12 * 2 + "d51_rtti_cold_arm_scopes".len() + "d51_rtti_cache_unseeded_admits".len();
    assert_eq!(
        after.len(),
        before.len() + census_bytes,
        "and the census is exactly what was added"
    );
}

/// Decode a PDU1 trailer back into the shapes `shape` produces.
fn decode_trailer(payload: &[u8]) -> Vec<(String, u64, u8, u8)> {
    assert_eq!(
        &payload[40..44],
        &COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC[..]
    );
    assert_eq!(
        u16::from_le_bytes([payload[44], payload[45]]),
        COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION,
        "the declaration lives in v2's reserved u16; v1 bytes cannot carry a role"
    );
    let unit_count = u16::from_le_bytes([payload[46], payload[47]]) as usize;
    let mut cursor = 48;
    let mut decoded = Vec::new();
    for _ in 0..unit_count {
        let name_len = u16::from_le_bytes([payload[cursor], payload[cursor + 1]]) as usize;
        let declaration = u16::from_le_bytes([payload[cursor + 2], payload[cursor + 3]]);
        let mut value = [0u8; 8];
        value.copy_from_slice(&payload[cursor + 4..cursor + 12]);
        let name = String::from_utf8(payload[cursor + 12..cursor + 12 + name_len].to_vec())
            .expect("unit names are ascii");
        decoded.push((
            name,
            u64::from_le_bytes(value),
            (declaration & 0x00FF) as u8,
            (declaration >> 8) as u8,
        ));
        cursor += 12 + name_len;
    }
    assert_eq!(
        cursor,
        payload.len(),
        "the trailer ends where it says it does"
    );
    decoded
}

#[test]
fn the_published_trailer_decodes_as_observed_counters_carrying_the_census() {
    // What the orchestrator will actually read out of the capture, decoded from
    // the bytes the production list produces - not from a list this file built.
    //
    // kills: a change to the declaration packing that still calls these
    // counters; swapping the two values on the way to the wire.
    let record = census_perf_record();
    let published = domain51_perf_diagnostic_units(&published_read(None));
    let (payload, flags) = compact_domain_perf_payload_with_declared(&record, 7, &published);

    assert_eq!(
        flags,
        compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER
    );
    let expected: Vec<(String, u64, u8, u8)> = expected_census_shapes()
        .into_iter()
        .map(|(name, value, role, presence)| (name.to_string(), value, role, presence))
        .collect();
    assert_eq!(decode_trailer(&payload), expected);
}

#[test]
fn the_role_and_presence_codes_this_file_asserts_are_the_contract_ones() {
    // The four constants above are written as literals so the tests read as
    // wire facts. This is the one place that ties them back to the enums, so a
    // renumbering of the diagnostic channel cannot leave the rest of the file
    // asserting stale codes.
    use compact_contract::diagnostic_channel::{UnitPresence, UnitRole};

    assert_eq!(UnitRole::Counter.code(), COUNTER);
    assert_eq!(UnitPresence::Observed.code(), OBSERVED);
    assert_eq!(UnitRole::Undeclared.code(), UNDECLARED_ROLE);
    assert_eq!(UnitPresence::Undeclared.code(), UNDECLARED_PRESENCE);
}

#[test]
fn every_name_the_census_publishes_is_summed_across_frames() {
    // The diagnostic channel classifies by NAME STRING alone - there is no
    // roster to register with, and `unit_accumulation` silently defaults. A
    // rename into one of the raw-word or bitmask suffixes would still emit and
    // would report one frame's count as the whole session's.
    //
    // The names come from the PRODUCTION list, so renaming
    // `d51_rtti_cold_arm_scopes` to `d51_rtti_cold_arm_scope_index`,
    // `..._mask` or `..._sample` fails here instead of passing a hard-coded
    // pair that no longer matches what is published.
    use compact_contract::diagnostic_channel::{unit_accumulation, Accumulation};

    // Only the census: the roster units are not this lane's to classify, and
    // one of them (`..._tag_db_id`) legitimately resolves to `LastValue`
    // through the raw-word suffix table.
    let published = domain51_perf_diagnostic_units(&published_read(None));
    assert_eq!(
        published.len(),
        2,
        "a healthy frame publishes the census alone"
    );
    for unit in &published {
        assert_eq!(
            unit_accumulation(unit.name),
            Accumulation::PerFrame,
            "{} is a per-frame count; the manifest must sum it",
            unit.name
        );
    }
}

#[test]
fn the_published_values_are_the_ones_on_the_read_and_not_a_constant() {
    // Codex round 3, finding 2. Every other test in this file uses ONE census,
    // so a builder that ignored its argument and returned `{ 648, 7 }` outright
    // left them all green while every real frame published a fabricated count
    // for the manifest to sum.
    //
    // Four censuses, one of them the healthy shape and one of them equal in
    // both fields (which no swap can distinguish, so the others carry that
    // half) - and each of them run BOTH with and without a roster failure.
    //
    // The roster half is not decoration. Codex round 4, finding 1: with the
    // varied censuses only ever on the healthy arm, a builder that special-
    // cased `read.country_roster_failure.is_some()` and returned `{648, 7}`
    // there passed everything, and a failing-roster frame would have published
    // fabricated counts for the manifest to sum.
    for (cold, unseeded) in [(648u32, 7u32), (0, 0), (2, 0), (99_999, 4_242)] {
        let census = Domain51RttiColdArmCensus {
            cold_arm_scopes: cold,
            unseeded_admits: unseeded,
        };
        let expected = vec![
            (
                "d51_rtti_cold_arm_scopes",
                u64::from(cold),
                COUNTER,
                OBSERVED,
            ),
            (
                "d51_rtti_cache_unseeded_admits",
                u64::from(unseeded),
                COUNTER,
                OBSERVED,
            ),
        ];
        for roster in [None, Some(roster_failure())] {
            let published = domain51_perf_diagnostic_units(&read_carrying(roster, census));
            let census_tail = published.len() - 2;

            assert_eq!(
                published[census_tail..]
                    .iter()
                    .map(shape)
                    .collect::<Vec<_>>(),
                expected,
                "the census on the read is the census on the wire, whatever the \
                 roster did"
            );
            assert_eq!(
                census_tail,
                if roster.is_some() { 7 } else { 0 },
                "and the roster's own units are still in front of it"
            );
        }
    }
}

#[test]
fn the_record_the_publish_path_actually_appends_carries_the_census() {
    // Codex rounds 2, 3 and 5. Three earlier versions of this test read the
    // publish arm's SOURCE and each was walked past in turn: a shadowed local,
    // then a filtered local, then a helper interposed on the argument
    // (`discard(&domain51_perf_diagnostic_units(&read))`) which a source-shape
    // assertion cannot see at all, because the helper is defined elsewhere.
    //
    // So this one does not read the seam, it RUNS it.
    // `append_compact_domain51_perf_record` is the whole publish path for d51's
    // PERF record - list and append together - and the bytes it produces are
    // decoded here. Anything interposed anywhere inside it is something this
    // test executes.
    for roster in [None, Some(roster_failure())] {
        let census = Domain51RttiColdArmCensus {
            cold_arm_scopes: 4_242,
            unseeded_admits: 9,
        };
        let mut out = Vec::new();
        append_compact_domain51_perf_record(&mut out, 17, 1_000, &read_carrying(roster, census), 7);

        let header_len = usize::from(COMPACT_RECORD_HEADER_LEN);
        assert_eq!(&out[..4], COMPACT_RECORD_MAGIC, "one d51 PERF record");
        let decoded = decode_trailer(&out[header_len..]);
        let tail = decoded.len() - 2;

        assert_eq!(
            decoded[tail..],
            [
                (
                    "d51_rtti_cold_arm_scopes".to_string(),
                    4_242,
                    COUNTER,
                    OBSERVED
                ),
                (
                    "d51_rtti_cache_unseeded_admits".to_string(),
                    9,
                    COUNTER,
                    OBSERVED
                ),
            ],
            "the census the read carried is on the record this path appended"
        );
        assert_eq!(
            tail,
            if roster.is_some() { 7 } else { 0 },
            "with the roster's units, if any, still in front of it"
        );
    }
}

#[test]
fn only_one_function_in_this_file_builds_d51s_perf_record_and_the_publish_arm_calls_it() {
    // The wiring the executable test above cannot reach.
    // `the_record_the_publish_path_actually_appends_carries_the_census` proves
    // `append_compact_domain51_perf_record` publishes the census; it cannot
    // prove that the publish arm still calls THAT function rather than a
    // sibling emitting the same PERF scalars with an empty diagnostic slice
    // (Codex round 6, finding 2). `build_compact_extraction_gate_records` needs
    // a whole live frame, so this is held from the source - which is how this
    // file's other callers are held too, see `publish_late_tests`.
    //
    // The property is FILE-GLOBAL and does not depend on reading the arm
    // correctly: a d51 PERF record can only be built by naming d51 in a
    // `CompactDomainPerfRecord`, and there is exactly one place in this file
    // that does, inside the function under test. A sibling helper has to add a
    // second one.
    let source = include_str!("../attach_log.rs").replace("\r\n", "\n");
    const D51_PERF_RECORD: &str = "domain_id: COMPACT_DOMAIN_SCRIPTED_STATE_FLAGS_VARIABLES_LIVE,";

    assert_eq!(
        source.matches(D51_PERF_RECORD).count(),
        1,
        "exactly one place builds a PERF record for d51"
    );
    let publisher_start = source
        .find("fn append_compact_domain51_perf_record(")
        .expect("the d51 PERF publisher");
    let publisher = &source[publisher_start..];
    let publisher = &publisher[..publisher.find("\n}\n").expect("it closes at column zero")];
    assert!(
        publisher.contains(D51_PERF_RECORD),
        "and that place is the function the executable test drives"
    );

    let arm_start = source
        .find("    if let Some(read) = scripted_state_flags_variables_live_read {")
        .expect("the d51 publish arm");
    let arm = &source[arm_start..];
    let arm = &arm[..arm
        .find("\n    }\n")
        .expect("the arm closes at column four")];
    let arm = super::d51_rewitness_tests::without_comments(arm);
    assert_eq!(
        arm.matches("append_compact_domain51_perf_record(").count(),
        1,
        "and the publish arm calls it, once"
    );
    // UNCONDITIONALLY. Codex round 7, finding 1, and the only finding since
    // round 4 that a plausible edit could produce rather than a determined
    // author: wrapping the call in `if read.country_roster_failure.is_none() {`
    // - conditioning diagnostics on clean-versus-failing status is an ordinary
    // thing to write - satisfied every property above, while a failing frame
    // published neither the census nor its seven roster units.
    //
    // Depth is counted inside the arm, whose own `if let` opens depth 1, so the
    // call sits at 1 and anything enclosing it at 2 or more.
    assert_eq!(
        super::d51_rewitness_tests::statement_depth(&arm, "append_compact_domain51_perf_record("),
        1,
        "d51's PERF record is published on every frame the domain ran, not on \
         the frames some other condition happens to allow"
    );
}
