//! G2 end-to-end playback gate (plan REPLAY_PRODUCTION_PLAN.md §G, item G2).
//!
//! A scripted, headless run over the committed synthetic recordings that drives
//! the PUBLIC product/consumer path: open through the shared `replay_core`
//! validator (`crate::validate::open_and_validate`), step the timeline with the
//! same seek/play_to hooks the scrub UI uses (`crate::timeline::RecordingTimeline`),
//! enter a country POV through the e-shell observer state
//! (`crate::observer::PovState`), and assert every keyed surface updates. It is
//! compiled only under `#[cfg(test)]` (see `main.rs`), so it never enters the
//! product binary.
//!
//! No pixels are hashed — the run is headless AND the FoW *render* layer is
//! still law-blocked (plan C8 is tooling-only). The gate is on the DATA-level
//! typed views the renderer keys off: owner/controller maps, per-POV province
//! visibility set, spotted-unit set, and topbar bindings, PLUS the fail-closed
//! render marker (`observer::FOW_NOT_LAW_CERTIFIED`). Each checkpoint is folded
//! into a deterministic FNV-1a state hash; the gate is (a) the hash sequence is
//! stable across independent runs and (b) the POV-vs-world diff is exactly the
//! fixture's fogged set.
//!
//! POV expectations are derived from the fixture generator SOURCE
//! (`replay_fixture_gen::fx_pov_visibility`), never from the code under test.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use replay_core::{open_recording, Leaf, WorldFrame, WorldTimeline};

use crate::observer::{ObserverMode, PovPicker, PovRenderStatus, PovState, FOW_NOT_LAW_CERTIFIED};
use crate::timeline::{OwnerProvider, RecordingTimeline};
use crate::validate::{open_and_validate, OpenOutcome};

// ---------------------------------------------------------------------------
// Fixture location + deterministic hash primitive.
// ---------------------------------------------------------------------------

fn fixture_path(rel: &str) -> PathBuf {
    // CARGO_MANIFEST_DIR = .../native/replay_app → .../replay_interface/fixtures
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures")
        .join(rel)
}

/// Deterministic 64-bit FNV-1a. std's `DefaultHasher` is deterministic too, but
/// a hand-rolled FNV removes any doubt about cross-version stability and lets
/// the exact hash points be reproduced from the LANE_REPORT.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Open a fixture twice: one `WorldTimeline` for typed-view frame access and a
/// second, independent `RecordingTimeline` (the product scrub provider) for the
/// seek/play_to hooks. Both back onto the same committed file, so they resolve
/// identically — this mirrors the product, which validates/opens then plays.
fn open_recording_timeline(rel: &str) -> (WorldTimeline, RecordingTimeline) {
    let path = fixture_path(rel);
    let source = open_recording(&path).expect("open recording source");
    let timeline = WorldTimeline::open(source).expect("open world timeline");
    let provider_source = open_recording(&path).expect("reopen recording source");
    let provider_timeline = WorldTimeline::open(provider_source).expect("reopen world timeline");
    let provider = RecordingTimeline::new_with_policy(provider_timeline, None);
    (timeline, provider)
}

// ---------------------------------------------------------------------------
// Typed-view encoders → the deterministic state hash the renderer keys off.
// ---------------------------------------------------------------------------

/// Measured owner/controller entries only (the renderer's keyed color table;
/// `Absent`/`Unobtained` states never enter it).
fn owner_map_encoding(map: &BTreeMap<u32, Leaf<String>>) -> String {
    let mut out = String::new();
    for (state, leaf) in map {
        if let Leaf::Measured(tag) = leaf {
            out.push_str(&format!("{state}={tag};"));
        }
    }
    out
}

/// The full province universe carried by the visibility domain — what the
/// default *omniscient* view shows (no fog).
fn world_visible_universe(frame: &WorldFrame) -> BTreeSet<u32> {
    let mut set = BTreeSet::new();
    let povs: Vec<String> = frame.visibility_povs().map(str::to_string).collect();
    for pov in povs {
        for pid in frame.visibility_for(&pov).provinces.keys() {
            set.insert(*pid);
        }
    }
    set
}

/// The provinces the given POV can actually see (`level == "visible"`).
fn pov_visible_set(frame: &WorldFrame, tag: &str) -> BTreeSet<u32> {
    let mut set = BTreeSet::new();
    for (&pid, leaf) in frame.visibility_for(tag).provinces.iter() {
        if let Leaf::Measured(v) = leaf {
            if v.level == "visible" {
                set.insert(pid);
            }
        }
    }
    set
}

/// What the map renders as *visible* under the current observer mode: omniscient
/// = the whole universe (no fog); a POV = that country's visible set.
fn effective_visible_set(frame: &WorldFrame, pov: &PovState) -> BTreeSet<u32> {
    match pov.active_pov() {
        None => world_visible_universe(frame),
        Some(tag) => pov_visible_set(frame, tag),
    }
}

fn leaf_list_encoding(leaf: &Leaf<Vec<String>>) -> String {
    match leaf {
        Leaf::Measured(v) => {
            let mut sorted = v.clone();
            sorted.sort();
            format!("M[{}]", sorted.join(","))
        }
        Leaf::Unobtained => "U".to_string(),
        Leaf::Absent => "A".to_string(),
    }
}

/// The foreign units the current mode reveals: omniscient shows everything
/// (`ALL`); a POV shows only its spotted set (unit-hiding surface).
fn effective_spotted_encoding(frame: &WorldFrame, pov: &PovState) -> String {
    match pov.active_pov() {
        None => "ALL".to_string(),
        Some(tag) => {
            let s = frame.spotted_units_for(tag);
            format!(
                "div={};fleets={};wings={}",
                leaf_list_encoding(&s.divisions),
                leaf_list_encoding(&s.fleets),
                leaf_list_encoding(&s.wings),
            )
        }
    }
}

/// The fail-closed render contract marker (POV surfaces the C8 tooling-only
/// diagnostic and keeps the omniscient look; it never fakes fog).
fn render_marker(pov: &PovState) -> String {
    match pov.render_status() {
        PovRenderStatus::Omniscient => "render=OMNISCIENT".to_string(),
        PovRenderStatus::FowBlocked { pov, diagnostic } => {
            format!("render=FOWBLOCKED:{pov}:{diagnostic}")
        }
    }
}

/// Per-POV topbar bindings (the derived scalar surface). `Debug` of the typed
/// view is deterministic (BTreeMap ordering + explicit `Leaf` variants).
fn topbar_encoding(frame: &WorldFrame, tags: &[&str]) -> String {
    let mut out = String::new();
    for tag in tags {
        match frame.topbar_scalars(tag) {
            Some(scalars) => out.push_str(&format!("{tag}:{scalars:?}|")),
            None => out.push_str(&format!("{tag}:none|")),
        }
    }
    out
}

/// The full deterministic state hash for one frame under one observer mode. It
/// folds every surface the renderer keys off into a single canonical string,
/// then FNV-1a's it. The observer mode + render marker are part of the input,
/// so world and POV hashes differ by construction; the *sets* are asserted
/// separately for the expected direction.
fn state_hash(frame: &WorldFrame, pov: &PovState, topbar_tags: &[&str]) -> u64 {
    let mode = match pov.mode() {
        ObserverMode::Omniscient => "OMNISCIENT".to_string(),
        ObserverMode::Pov(tag) => format!("POV:{tag}"),
    };
    let visible: Vec<String> = effective_visible_set(frame, pov)
        .iter()
        .map(u32::to_string)
        .collect();
    let canonical = format!(
        "hour={};mode={};{};owners={};controls={};visible=[{}];spotted={};topbar={}",
        frame.hour_index(),
        mode,
        render_marker(pov),
        owner_map_encoding(frame.owner_by_state()),
        owner_map_encoding(frame.control_by_state()),
        visible.join(","),
        effective_spotted_encoding(frame, pov),
        topbar_encoding(frame, topbar_tags),
    );
    fnv1a(canonical.as_bytes())
}

// ---------------------------------------------------------------------------
// POV expectations — derived from the fixture generator SOURCE, not the SUT.
//
// replay_fixture_gen `fx_pov_visibility` (main.rs): the province universe is
// {1000,1001,2000,3000}; per snapshot tick:
//   GER: 1000 visible when tick<5 else fogged; 1001 always visible;
//        2000 & 3000 always fogged.
//        spotted foreign units: {div:[201:1], fleets:[501:1]} when tick<5, else empty.
//   ENG: 3000 always visible; 2000 visible when tick>=7; 1000 & 1001 fogged.
// ---------------------------------------------------------------------------

const POV_UNIVERSE: [u32; 4] = [1000, 1001, 2000, 3000];

fn expected_ger_visible(tick: usize) -> BTreeSet<u32> {
    let mut set = BTreeSet::new();
    set.insert(1001);
    if tick < 5 {
        set.insert(1000);
    }
    set
}

fn expected_ger_fogged(tick: usize) -> BTreeSet<u32> {
    let visible = expected_ger_visible(tick);
    POV_UNIVERSE
        .iter()
        .copied()
        .filter(|p| !visible.contains(p))
        .collect()
}

fn expected_eng_visible(tick: usize) -> BTreeSet<u32> {
    let mut set = BTreeSet::new();
    set.insert(3000);
    if tick >= 7 {
        set.insert(2000);
    }
    set
}

// ---------------------------------------------------------------------------
// The scripted G2 end-to-end run.
// ---------------------------------------------------------------------------

#[test]
fn e2e_pov_visibility_open_play_scrub_pov_toggle() {
    let rel = "positive/fx_pov_visibility.battleplan";

    // 1. OPEN through the shared product validator.
    let report = match open_and_validate(&fixture_path(rel)) {
        OpenOutcome::Opened(r) => r,
        other => panic!("fx_pov_visibility must open through the validator: {other:?}"),
    };
    assert_eq!(report.game_id, "fx_pov_visibility");
    assert_eq!(report.hour_count, 12, "fx_pov_visibility hour count");
    assert!(
        report.warnings.is_empty(),
        "clean fixture: {:?}",
        report.warnings
    );

    // Build the timeline + the product scrub provider.
    let (timeline, provider) = open_recording_timeline(rel);
    let hours: Vec<i64> = timeline.hours().to_vec();
    assert_eq!(hours.len(), 12);

    // 2. Country picker: the recording session carries player countries under
    //    world.scenario.players, so the observer picker is Available.
    let choices = match provider.pov_picker() {
        PovPicker::Available(c) => c,
        PovPicker::Unavailable(reason) => panic!("picker should be Available: {reason}"),
    };
    let tags: Vec<&str> = choices.iter().map(|c| c.tag.as_str()).collect();
    assert!(
        tags.contains(&"GER") && tags.contains(&"ENG") && tags.contains(&"SOV"),
        "picker player tags: {tags:?}"
    );

    let topbar_tags = ["GER", "ENG", "SOV"];
    // Representative scrub checkpoints (tick index into the 12-hour series):
    // tick 0 (GER sees 1000), tick 5 (GER loses 1000 + its spotted units),
    // tick 7 (ENG gains 2000), tick 11 (end).
    let checkpoints = [0usize, 5, 7, 11];

    // One scripted pass → the ordered (hour, world_hash, pov_hash) record. Each
    // pass runs the full open→play→scrub→POV script and folds the deterministic
    // hashes at every checkpoint.
    let run_pass = || -> Vec<(i64, u64, u64)> {
        let mut record = Vec::new();
        for &tick in &checkpoints {
            let hour = hours[tick];
            // The scrub UI's seek/play_to hooks resolve identically and land the
            // exact recorded hour (E1 conformance, re-checked end-to-end here).
            assert_eq!(provider.seek_hour(hour), provider.play_to_hour(hour));
            assert_eq!(provider.seek_hour(hour), Some(hour));
            // Owners flow through the same primitive the scrub reads.
            let _ = provider.owners_at(hour);

            let frame = timeline.state_at(hour).expect("frame at checkpoint");

            let mut pov = PovState::new();
            let world_hash = state_hash(&frame, &pov, &topbar_tags);
            pov.enter_pov("GER");
            let pov_hash = state_hash(&frame, &pov, &topbar_tags);
            record.push((hour, world_hash, pov_hash));
        }
        record
    };

    // 3. DETERMINISM: two independent passes produce identical hash sequences.
    let pass_a = run_pass();
    let pass_b = run_pass();
    assert_eq!(
        pass_a, pass_b,
        "hash sequence must be deterministic across runs"
    );

    // 4. POV-vs-world diff is the expected fogged set + toggle-back idempotence.
    for (i, &tick) in checkpoints.iter().enumerate() {
        let hour = hours[tick];
        let frame = timeline.state_at(hour).expect("frame");

        // World view is omniscient: the whole province universe is visible.
        let world = PovState::new();
        let world_visible = effective_visible_set(&frame, &world);
        let expected_universe: BTreeSet<u32> = POV_UNIVERSE.iter().copied().collect();
        assert_eq!(
            world_visible, expected_universe,
            "omniscient universe @tick {tick}"
        );

        // Enter GER POV via the e-shell observer state.
        let mut pov = PovState::new();
        pov.enter_pov("GER");

        // Fail-closed render marker while a POV is active (plan C8 tooling-only).
        match pov.render_status() {
            PovRenderStatus::FowBlocked { pov: t, diagnostic } => {
                assert_eq!(t, "GER");
                assert_eq!(diagnostic, FOW_NOT_LAW_CERTIFIED);
            }
            other => panic!("POV must fail closed on render @tick {tick}: {other:?}"),
        }

        let pov_visible = effective_visible_set(&frame, &pov);
        assert_eq!(
            pov_visible,
            expected_ger_visible(tick),
            "GER visible @tick {tick}"
        );

        // The diff (world visible MINUS pov visible) is exactly the fogged set,
        // non-empty (POV hides ≥1 province), and pov ⊂ world.
        let diff: BTreeSet<u32> = world_visible.difference(&pov_visible).copied().collect();
        assert_eq!(diff, expected_ger_fogged(tick), "fogged diff @tick {tick}");
        assert!(
            !diff.is_empty(),
            "POV must hide at least one province @tick {tick}"
        );
        assert!(
            pov_visible.is_subset(&world_visible),
            "pov ⊂ world @tick {tick}"
        );

        // Hash relations: POV differs from world; world hash matches the record.
        let (rec_hour, rec_world, rec_pov) = pass_a[i];
        assert_eq!(rec_hour, hour);
        assert_ne!(
            rec_world, rec_pov,
            "POV must change the keyed state @tick {tick}"
        );
        assert_eq!(
            state_hash(&frame, &pov, &topbar_tags),
            rec_pov,
            "pov hash reproduces @tick {tick}"
        );

        // 5. TOGGLE BACK → the state hash returns to the world-view hash.
        pov.exit_to_world();
        assert_eq!(
            state_hash(&frame, &pov, &topbar_tags),
            rec_world,
            "toggle-back must restore the world hash @tick {tick}"
        );
        // A paired toggle();toggle(); returns to the POV hash (idempotent pair).
        let paired = {
            let mut p = PovState::new();
            p.enter_pov("GER");
            p.toggle();
            p.toggle();
            state_hash(&frame, &p, &topbar_tags)
        };
        assert_eq!(
            paired, rec_pov,
            "paired toggle returns to POV hash @tick {tick}"
        );
    }

    // 6. UNIT HIDING: at tick 0 GER spots the foreign SOV division; from tick 5
    //    GER spots nothing, yet the omniscient world still carries that division
    //    — a unit the POV hides but the world view shows.
    let frame0 = timeline.state_at(hours[0]).expect("frame0");
    let ger0 = frame0.spotted_units_for("GER");
    assert_eq!(ger0.divisions, Leaf::Measured(vec!["201:1".to_string()]));
    assert_eq!(ger0.fleets, Leaf::Measured(vec!["501:1".to_string()]));

    let frame5 = timeline.state_at(hours[5]).expect("frame5");
    let ger5 = frame5.spotted_units_for("GER");
    assert_eq!(
        ger5.divisions,
        Leaf::Measured(Vec::new()),
        "GER spots nothing @tick 5"
    );
    let world_still_has_sov_div = frame5
        .divisions()
        .iter()
        .any(|d| d.div_uid == "201:1" && d.owner == Leaf::Measured("SOV".to_string()));
    assert!(
        world_still_has_sov_div,
        "omniscient world still carries the SOV division the POV no longer sees"
    );

    // 7. POV INDEPENDENCE: ENG's visible set differs from GER's and matches its
    //    own fixture-derived expectation.
    for &tick in &[0usize, 7] {
        let frame = timeline.state_at(hours[tick]).expect("frame");
        let mut eng = PovState::new();
        eng.enter_pov("ENG");
        assert_eq!(
            effective_visible_set(&frame, &eng),
            expected_eng_visible(tick),
            "ENG visible @tick {tick}"
        );
    }
}

/// Companion gate for the topbar-binding surface: the topbar-scalar sweep
/// fixture has no visibility domain, so it isolates the derived topbar bindings.
/// Open → scrub every hour → the binding hash sequence is deterministic across
/// runs AND every swept hour has a distinct binding (the surface updates).
#[test]
fn e2e_topbar_bindings_scrub_determinism() {
    let rel = "positive/fx_topbar_scalars_sweep.battleplan";
    let outcome = open_and_validate(&fixture_path(rel));
    assert!(
        matches!(outcome, OpenOutcome::Opened(_)),
        "topbar fixture opens: {outcome:?}"
    );

    let (timeline, provider) = open_recording_timeline(rel);
    let hours: Vec<i64> = timeline.hours().to_vec();
    assert_eq!(hours.len(), 12);
    let tags = ["GER", "ENG", "SOV"];

    let run = || -> Vec<u64> {
        let mut seq = Vec::new();
        for &hour in &hours {
            assert_eq!(provider.seek_hour(hour), provider.play_to_hour(hour));
            let frame = timeline.state_at(hour).expect("frame");
            let pov = PovState::new();
            seq.push(state_hash(&frame, &pov, &tags));
        }
        seq
    };
    let a = run();
    let b = run();
    assert_eq!(a, b, "topbar hash sequence deterministic across runs");
    assert_ne!(
        a.first(),
        a.last(),
        "topbar bindings must update across the scrub"
    );
    let distinct: BTreeSet<u64> = a.iter().copied().collect();
    assert_eq!(
        distinct.len(),
        a.len(),
        "each swept hour has a distinct binding hash"
    );
}
