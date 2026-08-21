//! Lane PH: the equivalence gate and the measurement instrument for the two
//! d47 walk phases this lane changed — `portrait_phase` and
//! `appointed_advisors_from_country_active_ideas`.
//!
//! `LEGACY_*` below are VERBATIM transcriptions of the replaced code, kept so
//! the gate compares two live implementations over the same input rather than
//! comparing the new one against a hand-written expectation. Nothing in this
//! file is production code; the legacy arms exist only to be refuted.
//!
//! Profile: `cargo test -p characters_leaders_cabinet_live --release --
//! --ignored --nocapture ph_walk_phase_profile`.

use std::collections::{BTreeMap, BTreeSet};

use super::*;

// ---------------------------------------------------------------------------
// The replaced code, transcribed.
// ---------------------------------------------------------------------------

/// Verbatim transcription of the portrait phase as it stood at master
/// 11c403f3, lifted out of `build_lane_frame_inner` with the surrounding
/// bookkeeping (`stats.rejected_roots`, `frame.character_portraits`) turned
/// into the `out` / return-value seam the shipped `portrait_phase` uses.
fn legacy_portrait_phase(
    portraits: &[CharacterPortraitInput],
    cached: Option<&CachedWalk>,
    known_characters: &BTreeSet<(u32, u32)>,
    out: &mut Vec<CharacterPortraitRecord>,
) -> u32 {
    let mut rejected = 0u32;
    let live_only_admitted = cached.map_or(false, |c| {
        portraits.iter().any(|portrait| {
            let key = (portrait.country_tag_db_id, portrait.char_id);
            !c.knows(&key) && known_characters.contains(&key)
        })
    });
    let admit = |key: &(u32, u32)| -> bool {
        known_characters.contains(key) || cached.map_or(false, |c| c.knows(key))
    };
    for portrait in portraits {
        let key = (portrait.country_tag_db_id, portrait.char_id);
        let cached_key = cached.map_or(false, |c| c.knows(&key));
        if cached_key && !live_only_admitted {
            continue;
        }
        if admit(&key) {
            out.push(CharacterPortraitRecord {
                country_tag_db_id: portrait.country_tag_db_id,
                char_id: portrait.char_id,
                role: portrait.role.clone(),
                size: portrait.size.clone(),
                gfx: portrait.gfx.clone(),
                gfx_encoding: portrait.gfx_encoding,
            });
        } else {
            rejected = rejected.saturating_add(1);
        }
    }
    rejected
}

/// Verbatim transcription of `appointed_advisors_from_country_active_ideas` as
/// it stood at master 11c403f3.
fn legacy_appointed_advisors(
    roster: &[AdvisorRecord],
    countries: &[CountryPoliticsRoot],
) -> Vec<AdvisorRecord> {
    let active_by_country = countries
        .iter()
        .map(|country| {
            (
                country.country_tag_db_id,
                country
                    .active_idea_tokens
                    .iter()
                    .map(String::as_str)
                    .collect::<BTreeSet<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut seen = BTreeSet::new();
    roster
        .iter()
        .filter(|row| {
            active_by_country
                .get(&row.country_tag_db_id)
                .map(|tokens| tokens.contains(row.idea_token.as_str()))
                .unwrap_or(false)
        })
        .filter(|row| seen.insert((row.country_tag_db_id, row.char_id, row.idea_token.clone())))
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------
// Shapes.
// ---------------------------------------------------------------------------

/// A deterministic, seedable generator. Not for cryptography — for making the
/// shapes below reproducible from a seed printed in a failure message.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // splitmix64
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

/// oz_tip's shape, read out of `captures/oz_tip/native_session_manifest.json`
/// and divided by the 443 measured invocations:
/// `domain47_prewarm_portrait_rows` 7,040,291 -> 15,892 portrait input rows;
/// `domain47_adapter_character_rows` 5,935,828 -> 13,399 characters;
/// `domain47_adapter_advisor_roster_rows` 3,816,445 -> 8,615 roster rows;
/// `domain47_adapter_country_roots_seen` 194,477 -> 439 countries;
/// `domain47_adapter_active_ruler_rows` 194,034 -> 438 active rulers, which is
/// what `known_characters` holds on a reused frame.
const OZ_PORTRAIT_ROWS: usize = 15_892;
const OZ_CHARACTERS: usize = 13_399;
const OZ_ROSTER_ROWS: usize = 8_615;
const OZ_COUNTRIES: usize = 439;
const OZ_ACTIVE_RULERS: usize = 438;

struct PortraitShape {
    portraits: Vec<CharacterPortraitInput>,
    cached: CachedWalk,
    known_characters: BTreeSet<(u32, u32)>,
}

/// `unknown_rows` input rows carry a key the cache does not know;
/// `live_only_rows` of those keys are also in `known_characters`, which is
/// exactly what makes `live_only_admitted` true.
fn portrait_shape(
    seed: u64,
    portrait_rows: usize,
    characters: usize,
    active_rulers: usize,
    unknown_rows: usize,
    live_only_rows: usize,
    index: bool,
) -> PortraitShape {
    let mut rng = Rng(seed);
    let known_keys: Vec<(u32, u32)> = (0..characters).map(|i| (i as u32 % 250, i as u32)).collect();
    let mut sorted_known = known_keys.clone();
    sorted_known.sort_unstable();
    sorted_known.dedup();

    let mut portraits = Vec::with_capacity(portrait_rows);
    for row in 0..portrait_rows {
        // Keys repeat across roles and sizes, which is why the index has to
        // count ROWS and not just distinct keys.
        let key = if row < unknown_rows {
            // Codex round 1, finding 2: these keys must REPEAT, or a mutant
            // that stored the deduplicated key COUNT instead of the row count
            // would agree with the scan on every shape. Three rows per key.
            (900 + ((row as u32 / 3) % 50), 1_000_000 + (row as u32 / 3))
        } else {
            sorted_known[rng.below(sorted_known.len())]
        };
        portraits.push(CharacterPortraitInput {
            country_tag_db_id: key.0,
            char_id: key.1,
            role: if row % 2 == 0 { "civilian" } else { "army" }.to_string(),
            size: "large".to_string(),
            gfx: format!("GFX_portrait_{}_{}", key.0, key.1),
            gfx_encoding: compact::PortraitIdentityEncoding::Utf8,
        });
    }

    let mut known_characters = BTreeSet::new();
    for i in 0..active_rulers.min(sorted_known.len()) {
        known_characters.insert(sorted_known[i]);
    }
    for row in 0..live_only_rows.min(unknown_rows) {
        known_characters.insert((
            900 + ((row as u32 / 3) % 50),
            1_000_000 + (row as u32 / 3),
        ));
    }

    let mut cached = CachedWalk {
        families: compact::FAMILY_CHARACTER,
        known_keys: sorted_known,
        ..CachedWalk::default()
    };
    // What `adopt_cached_walk` retains: the cached-key subset of the adopting
    // frame's admitted portraits.
    cached.character_portraits = portraits
        .iter()
        .filter(|p| cached.knows(&(p.country_tag_db_id, p.char_id)))
        .map(|p| CharacterPortraitRecord {
            country_tag_db_id: p.country_tag_db_id,
            char_id: p.char_id,
            role: p.role.clone(),
            size: p.size.clone(),
            gfx: p.gfx.clone(),
            gfx_encoding: p.gfx_encoding,
        })
        .collect();
    if index {
        cached.portrait_index = Some(build_portrait_index(&portraits, &cached));
    }
    PortraitShape {
        portraits,
        cached,
        known_characters,
    }
}

/// The precompute `NativeScheduledObservation::adopt_cached_walk` performs,
/// expressed over the input list (on an adopting frame the two are the same
/// list — see the comment at that call site).
fn build_portrait_index(
    portraits: &[CharacterPortraitInput],
    cached: &CachedWalk,
) -> CachedPortraitIndex {
    let mut portrait_unknown_keys: Vec<(u32, u32)> = portraits
        .iter()
        .map(|p| (p.country_tag_db_id, p.char_id))
        .filter(|key| !cached.knows(key))
        .collect();
    let portrait_unknown_rows = portrait_unknown_keys.len().min(u32::MAX as usize) as u32;
    portrait_unknown_keys.sort_unstable();
    portrait_unknown_keys.dedup();
    CachedPortraitIndex {
        portrait_input_rows: portraits.len().min(u32::MAX as usize) as u32,
        portrait_unknown_keys,
        portrait_unknown_rows,
    }
}

struct AppointedShape {
    roster: Vec<AdvisorRecord>,
    countries: Vec<CountryPoliticsRoot>,
}

fn appointed_shape(
    seed: u64,
    roster_rows: usize,
    countries: usize,
    ideas_per_country: usize,
    match_rate: usize,
    sorted_tokens: bool,
    duplicate_tags: bool,
) -> AppointedShape {
    let mut rng = Rng(seed);
    let mut country_rows = Vec::with_capacity(countries);
    for tag in 0..countries {
        let tag = if duplicate_tags && tag % 7 == 0 {
            (tag / 7) as u32
        } else {
            tag as u32
        };
        // Codex round 1, finding 2: a duplicated tag's two rows must hold
        // DIFFERENT tokens, or `collect()`-into-BTreeMap's last-wins and a
        // first-wins regression produce the same join. `dup_ordinal` counts how
        // many rows have already claimed this tag.
        let dup_ordinal = country_rows
            .iter()
            .filter(|c: &&CountryPoliticsRoot| c.country_tag_db_id == tag)
            .count();
        let mut active_idea_tokens: Vec<String> = (0..ideas_per_country)
            .map(|idea| format!("idea_{tag}_{idea:03}_national_spirit_v{dup_ordinal}"))
            .collect();
        if sorted_tokens {
            active_idea_tokens.sort();
            active_idea_tokens.dedup();
        } else {
            // Shuffle. The shipped join scans rather than bisects, so it never
            // sorts anything — what this shape proves is that the membership
            // answer does not depend on the order the tokens arrive in.
            for i in (1..active_idea_tokens.len()).rev() {
                active_idea_tokens.swap(i, rng.below(i + 1));
            }
        }
        country_rows.push(CountryPoliticsRoot {
            country_addr: 0x4000 + tag as u64 * 0x100,
            country_tag_db_id: tag,
            ruling_party: Some("democratic".to_string()),
            ruler_name: Some(format!("Ruler {tag}")),
            ruler_token: Some(format!("TAG{tag}_ruler")),
            active_idea_tokens,
        });
    }
    let mut roster = Vec::with_capacity(roster_rows);
    for row in 0..roster_rows {
        // With no countries at all the roster still names tags; every row must
        // then fall through `active_by_country.get(..) -> None`.
        let tag = (row % countries.max(1)) as u32;
        // `match_rate` out of every 100 roster rows name an idea their country
        // really holds; the rest name one it does not.
        // Names the FIRST row's token set for a duplicated tag (`_v0`), which
        // last-wins must REJECT and first-wins would admit.
        let idea_token = if ideas_per_country > 0 && row % 100 < match_rate {
            format!("idea_{tag}_{:03}_national_spirit_v0", row % ideas_per_country)
        } else {
            format!("idea_{tag}_{row}_not_active")
        };
        roster.push(AdvisorRecord {
            country_tag_db_id: tag,
            // Deliberate key collisions, so `seen` really has to dedup.
            char_id: (row / 3) as u32,
            slot: Some("political_advisor".to_string()),
            ledger: Some("civilian".to_string()),
            idea_token,
        });
    }
    // Exact duplicates, so `seen` really has something to drop: an advisor can
    // hold the same idea in the same country twice across the roster.
    for row in 1..roster.len() {
        if row % 37 == 0 {
            roster[row] = roster[row - 1].clone();
        }
    }
    AppointedShape {
        roster,
        countries: country_rows,
    }
}

// ---------------------------------------------------------------------------
// The equivalence gate.
// ---------------------------------------------------------------------------

#[test]
fn both_portrait_forms_agree_on_every_shape() {
    // (label, portrait rows, characters, active rulers, unknown rows,
    //  live-only rows, index present)
    let shapes: &[(&str, usize, usize, usize, usize, usize, bool)] = &[
        ("oz_tip steady state", OZ_PORTRAIT_ROWS, OZ_CHARACTERS, OZ_ACTIVE_RULERS, 0, 0, true),
        ("oz_tip, no index (the scan arm)", OZ_PORTRAIT_ROWS, OZ_CHARACTERS, OZ_ACTIVE_RULERS, 0, 0, false),
        ("unknown keys, none live", 4_000, 3_000, 200, 130, 0, true),
        ("unknown keys, none live, no index", 4_000, 3_000, 200, 130, 0, false),
        ("ONE live-only admission", 4_000, 3_000, 200, 130, 1, true),
        ("ONE live-only admission, no index", 4_000, 3_000, 200, 130, 1, false),
        ("every unknown key live", 4_000, 3_000, 200, 130, 130, true),
        ("all rows unknown, none live", 900, 3_000, 200, 900, 0, true),
        ("all rows unknown, all live", 900, 3_000, 200, 900, 900, true),
        ("no active rulers at all", 4_000, 3_000, 0, 130, 0, true),
        ("no portraits", 0, 3_000, 200, 0, 0, true),
        ("one portrait, known", 1, 3_000, 200, 0, 0, true),
        ("one portrait, unknown, not live", 1, 3_000, 200, 1, 0, true),
        ("one portrait, unknown, live", 1, 3_000, 200, 1, 1, true),
    ];
    let mut exercised = Vec::new();
    let mut fast_path_taken = 0;
    let mut rows_exceed_keys = 0;
    for &(label, rows, characters, rulers, unknown, live_only, index) in shapes {
        let shape = portrait_shape(0x5EED, rows, characters, rulers, unknown, live_only, index);
        let cached_arms: [Option<&CachedWalk>; 2] = [Some(&shape.cached), None];
        for cached in cached_arms {
            let mut legacy_out = Vec::new();
            let legacy_rejected = legacy_portrait_phase(
                &shape.portraits,
                cached,
                &shape.known_characters,
                &mut legacy_out,
            );
            let mut shipped_out = Vec::new();
            let (shipped_rejected, arm) = portrait_phase(
                &shape.portraits,
                cached,
                &shape.known_characters,
                &mut shipped_out,
            );
            assert_eq!(
                legacy_out, shipped_out,
                "{label} (cached={}): the two portrait forms admitted different rows",
                cached.is_some()
            );
            assert_eq!(
                legacy_rejected, shipped_rejected,
                "{label} (cached={}): the two portrait forms rejected a different count",
                cached.is_some()
            );
            // The arm PRODUCTION took. Without this the two forms agree by
            // construction, so deleting the O(1) return and always scanning
            // would keep every assertion above green (Codex round 1).
            let expect_index = cached.is_some() && index && live_only == 0;
            assert_eq!(
                arm,
                if expect_index {
                    PortraitPhaseArm::Index
                } else {
                    PortraitPhaseArm::Scan
                },
                "{label} (cached={}): the phase took the wrong arm",
                cached.is_some()
            );
            if arm == PortraitPhaseArm::Index {
                fast_path_taken += 1;
            }
        }
        // The unknown ROW count must exceed the unknown KEY count somewhere, or
        // a mutant storing the deduplicated count would agree everywhere.
        if let Some(built) = shape.cached.portrait_index.as_ref() {
            assert_eq!(
                built.portrait_unknown_rows as usize, unknown,
                "{label}: the fixture's unknown ROW count is not what the index recorded"
            );
            if built.portrait_unknown_rows as usize > built.portrait_unknown_keys.len() {
                rows_exceed_keys += 1;
            }
        }
        exercised.push(label);
    }
    assert_eq!(exercised.len(), shapes.len());
    assert!(
        fast_path_taken >= 7,
        "only {fast_path_taken} runs reported PortraitPhaseArm::Index"
    );
    assert!(
        rows_exceed_keys >= 2,
        "only {rows_exceed_keys} shapes had more unknown ROWS than unknown KEYS, so a          mutant counting keys instead of rows would survive"
    );
}

#[test]
fn a_portrait_index_taken_over_another_list_is_refused() {
    // The belt-and-braces half of the licence: an index whose
    // `portrait_input_rows` does not match the list in hand must fall back to
    // the scan rather than answer from stale counts. Build an index over a
    // DIFFERENT list, attach it, and require the scan's answer.
    let shape = portrait_shape(0x5EED, 4_000, 3_000, 200, 130, 0, false);
    let other = portrait_shape(0xF00D, 3_999, 3_000, 200, 4_000, 4_000, false);
    let mut cached = shape.cached.clone();
    cached.portrait_index = Some(build_portrait_index(&other.portraits, &other.cached));
    assert_ne!(
        cached
            .portrait_index
            .as_ref()
            .expect("attached")
            .portrait_unknown_rows,
        130,
        "the mismatched index must differ from the right answer, or this proves nothing"
    );
    let mut legacy_out = Vec::new();
    let legacy_rejected = legacy_portrait_phase(
        &shape.portraits,
        Some(&shape.cached),
        &shape.known_characters,
        &mut legacy_out,
    );
    let mut shipped_out = Vec::new();
    let (shipped_rejected, arm) = portrait_phase(
        &shape.portraits,
        Some(&cached),
        &shape.known_characters,
        &mut shipped_out,
    );
    assert_eq!(legacy_out, shipped_out);
    assert_eq!(legacy_rejected, shipped_rejected);
    assert_eq!(legacy_rejected, 130, "the scan's answer, not the index's");
    assert_eq!(
        arm,
        PortraitPhaseArm::Scan,
        "a mismatched index must be refused and the row walk taken"
    );
}

#[test]
fn both_appointed_forms_agree_on_every_shape() {
    // (label, roster rows, countries, ideas/country, match rate %,
    //  tokens arrive sorted, duplicate country tags)
    let shapes: &[(&str, usize, usize, usize, usize, bool, bool)] = &[
        ("oz_tip, nothing matches", OZ_ROSTER_ROWS, OZ_COUNTRIES, 20, 0, true, false),
        ("oz_tip, a fifth matches", OZ_ROSTER_ROWS, OZ_COUNTRIES, 20, 20, true, false),
        ("oz_tip, everything matches", OZ_ROSTER_ROWS, OZ_COUNTRIES, 20, 100, true, false),
        ("no active ideas at all", OZ_ROSTER_ROWS, OZ_COUNTRIES, 0, 50, true, false),
        ("one idea per country", 2_000, 200, 1, 50, true, false),
        ("forty ideas per country", 2_000, 200, 40, 50, true, false),
        ("tokens arrive UNSORTED", 2_000, 200, 20, 50, false, false),
        ("duplicate country tags", 2_000, 200, 20, 50, true, true),
        ("duplicate tags, unsorted", 2_000, 200, 20, 50, false, true),
        ("no countries", 2_000, 0, 0, 50, true, false),
        ("no roster", 0, 200, 20, 50, true, false),
        ("one roster row, matching", 1, 200, 20, 100, true, false),
    ];
    let mut matched_somewhere = 0;
    let mut deduped_somewhere = 0;
    for &(label, roster_rows, countries, ideas, rate, sorted, dup) in shapes {
        let shape = appointed_shape(0x5EED, roster_rows, countries, ideas, rate, sorted, dup);
        let legacy = legacy_appointed_advisors(&shape.roster, &shape.countries);
        let shipped = appointed_advisors_from_country_active_ideas(&shape.roster, &shape.countries);
        assert_eq!(
            legacy, shipped,
            "{label}: the two appointed-advisor forms disagreed"
        );
        if !legacy.is_empty() {
            matched_somewhere += 1;
            let candidates = shape
                .roster
                .iter()
                .filter(|row| {
                    shape
                        .countries
                        .iter()
                        .rev()
                        .find(|c| c.country_tag_db_id == row.country_tag_db_id)
                        .map(|c| c.active_idea_tokens.contains(&row.idea_token))
                        .unwrap_or(false)
                })
                .count();
            if candidates > legacy.len() {
                deduped_somewhere += 1;
            }
        }
    }
    assert!(
        matched_somewhere >= 6,
        "only {matched_somewhere} shapes produced any appointed advisor — the gate would be \
         comparing two empty vectors"
    );
    assert!(
        deduped_somewhere >= 1,
        "no shape exercised the `seen` deduplication"
    );
}

#[test]
fn the_appointed_join_is_a_set_test_whatever_order_the_tokens_arrive_in() {
    // The shipped form scans each country's slice, so it neither sorts it nor
    // relies on `domain47_compose_politics_snapshot` having sorted it. Prove
    // that the same country, with the same tokens in a different order, joins
    // the same — the property that makes the scan a set test.
    let sorted = appointed_shape(0x5EED, 2_000, 200, 20, 50, true, false);
    let shuffled = appointed_shape(0x5EED, 2_000, 200, 20, 50, false, false);
    let mut any_reordered = false;
    for (a, b) in sorted.countries.iter().zip(shuffled.countries.iter()) {
        assert_eq!(a.country_tag_db_id, b.country_tag_db_id);
        let mut a_set = a.active_idea_tokens.clone();
        let mut b_set = b.active_idea_tokens.clone();
        a_set.sort();
        b_set.sort();
        assert_eq!(a_set, b_set, "the two shapes must hold the same token SET");
        if a.active_idea_tokens != b.active_idea_tokens {
            any_reordered = true;
        }
    }
    assert!(any_reordered, "the shuffle did not reorder anything");
    assert_eq!(sorted.roster, shuffled.roster);
    assert_eq!(
        appointed_advisors_from_country_active_ideas(&sorted.roster, &sorted.countries),
        appointed_advisors_from_country_active_ideas(&shuffled.roster, &shuffled.countries),
    );
}

// ---------------------------------------------------------------------------
// The measurement instrument.
// ---------------------------------------------------------------------------

fn min_us<F: FnMut() -> usize>(reps: usize, mut body: F) -> (f64, usize) {
    let mut best = f64::MAX;
    let mut sink = 0usize;
    for _ in 0..reps {
        let started = std::time::Instant::now();
        sink = sink.wrapping_add(body());
        let us = started.elapsed().as_nanos() as f64 / 1000.0;
        if us < best {
            best = us;
        }
    }
    (best, sink)
}

/// Interleaved arms in ONE process, order flipped every repetition, minimised
/// over `reps` — the shape PC and PE used, for the same reason: this box
/// cannot carry a small claim across two builds.
fn interleaved<A: FnMut() -> usize, B: FnMut() -> usize>(
    reps: usize,
    mut legacy: A,
    mut shipped: B,
) -> (f64, f64) {
    let mut legacy_best = f64::MAX;
    let mut shipped_best = f64::MAX;
    for rep in 0..reps {
        let (l, s) = if rep % 2 == 0 {
            let l = min_us(1, &mut legacy).0;
            let s = min_us(1, &mut shipped).0;
            (l, s)
        } else {
            let s = min_us(1, &mut shipped).0;
            let l = min_us(1, &mut legacy).0;
            (l, s)
        };
        legacy_best = legacy_best.min(l);
        shipped_best = shipped_best.min(s);
    }
    (legacy_best, shipped_best)
}

const PROFILE_REPS: usize = 51;

#[test]
#[ignore = "measurement instrument; run explicitly with --ignored --nocapture"]
fn ph_walk_phase_profile_portraits() {
    println!("ph_walk_phase_profile_portraits reps={PROFILE_REPS}");
    for &(label, rows, characters, rulers, unknown, live_only) in &[
        ("oz_tip steady state", OZ_PORTRAIT_ROWS, OZ_CHARACTERS, OZ_ACTIVE_RULERS, 0, 0),
        ("oz_tip, 100 unknown rows, none live", OZ_PORTRAIT_ROWS, OZ_CHARACTERS, OZ_ACTIVE_RULERS, 100, 0),
        ("oz_tip, ONE live-only admission", OZ_PORTRAIT_ROWS, OZ_CHARACTERS, OZ_ACTIVE_RULERS, 100, 1),
        ("half oz_tip", OZ_PORTRAIT_ROWS / 2, OZ_CHARACTERS, OZ_ACTIVE_RULERS, 0, 0),
    ] {
        let shape = portrait_shape(0x5EED, rows, characters, rulers, unknown, live_only, true);
        let (legacy, shipped) = interleaved(
            PROFILE_REPS,
            || {
                let mut out = Vec::new();
                let rejected = legacy_portrait_phase(
                    &shape.portraits,
                    Some(&shape.cached),
                    &shape.known_characters,
                    &mut out,
                );
                out.len() + rejected as usize
            },
            || {
                let mut out = Vec::new();
                let (rejected, _arm) = portrait_phase(
                    &shape.portraits,
                    Some(&shape.cached),
                    &shape.known_characters,
                    &mut out,
                );
                out.len() + rejected as usize
            },
        );
        println!(
            "  {label:<40} legacy {legacy:8.1} us -> shipped {shipped:8.1} us  \
             (cut {:6.1} us, {:5.1}%)",
            legacy - shipped,
            (legacy - shipped) / legacy * 100.0
        );
    }
}

#[test]
#[ignore = "measurement instrument; run explicitly with --ignored --nocapture"]
fn ph_walk_phase_profile_appointed() {
    println!("ph_walk_phase_profile_appointed reps={PROFILE_REPS}");
    for &(label, roster_rows, countries, ideas, rate) in &[
        ("oz_tip, 0 ideas, no match", OZ_ROSTER_ROWS, OZ_COUNTRIES, 0, 0),
        ("oz_tip, 2 ideas, no match", OZ_ROSTER_ROWS, OZ_COUNTRIES, 2, 0),
        ("oz_tip, 5 ideas, no match", OZ_ROSTER_ROWS, OZ_COUNTRIES, 5, 0),
        ("oz_tip, 8 ideas, no match", OZ_ROSTER_ROWS, OZ_COUNTRIES, 8, 0),
        ("oz_tip, 11 ideas, no match", OZ_ROSTER_ROWS, OZ_COUNTRIES, 11, 0),
        ("oz_tip, 16 ideas, no match", OZ_ROSTER_ROWS, OZ_COUNTRIES, 16, 0),
        ("oz_tip, 20 ideas, no match", OZ_ROSTER_ROWS, OZ_COUNTRIES, 20, 0),
        ("oz_tip, 20 ideas, 20% match", OZ_ROSTER_ROWS, OZ_COUNTRIES, 20, 20),
        ("oz_tip, 32 ideas, no match", OZ_ROSTER_ROWS, OZ_COUNTRIES, 32, 0),
        ("oz_tip, 48 ideas, no match", OZ_ROSTER_ROWS, OZ_COUNTRIES, 48, 0),
        ("oz_tip, 63 ideas, no match", OZ_ROSTER_ROWS, OZ_COUNTRIES, 63, 0),
        ("oz_tip, 64 ideas, no match", OZ_ROSTER_ROWS, OZ_COUNTRIES, 64, 0),
        ("oz_tip, 96 ideas, no match", OZ_ROSTER_ROWS, OZ_COUNTRIES, 96, 0),
        ("oz_tip, 128 ideas, no match", OZ_ROSTER_ROWS, OZ_COUNTRIES, 128, 0),
        ("oz_tip, 192 ideas, no match", OZ_ROSTER_ROWS, OZ_COUNTRIES, 192, 0),
        ("oz_tip, 40 ideas, 20% match", OZ_ROSTER_ROWS, OZ_COUNTRIES, 40, 20),
    ] {
        let shape = appointed_shape(0x5EED, roster_rows, countries, ideas, rate, true, false);
        let (legacy, shipped) = interleaved(
            PROFILE_REPS,
            || legacy_appointed_advisors(&shape.roster, &shape.countries).len(),
            || appointed_advisors_from_country_active_ideas(&shape.roster, &shape.countries).len(),
        );
        println!(
            "  {label:<40} legacy {legacy:8.1} us -> shipped {shipped:8.1} us  \
             (cut {:6.1} us, {:5.1}%)",
            legacy - shipped,
            (legacy - shipped) / legacy * 100.0
        );
    }
}

#[test]
#[ignore = "measurement instrument; run explicitly with --ignored --nocapture"]
fn ph_walk_phase_profile_noise_floor() {
    // The SAME arm against itself, three times, so a reader can tell a real
    // cut from this box's spread.
    let shape = portrait_shape(0x5EED, OZ_PORTRAIT_ROWS, OZ_CHARACTERS, OZ_ACTIVE_RULERS, 0, 0, true);
    for round in 0..3 {
        let (a, b) = interleaved(
            PROFILE_REPS,
            || {
                let mut out = Vec::new();
                legacy_portrait_phase(&shape.portraits, Some(&shape.cached), &shape.known_characters, &mut out) as usize
            },
            || {
                let mut out = Vec::new();
                legacy_portrait_phase(&shape.portraits, Some(&shape.cached), &shape.known_characters, &mut out) as usize
            },
        );
        println!(
            "ph_walk_phase_profile_noise_floor round {round}: {a:.1} / {b:.1} us  (spread {:.2}%)",
            (a - b).abs() / a.min(b) * 100.0
        );
    }
}
