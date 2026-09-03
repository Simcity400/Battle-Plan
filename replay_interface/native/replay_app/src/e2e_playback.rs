
use replay_core::{CountryIdentityLeaf, CountryIdentityRosterLeaf};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use replay_core::{open_recording, Leaf, WorldFrame, WorldTimeline};

use crate::observer::{
    ObserverMode, PovPicker, PovRenderStatus, PovState, POV_RECORDED_STATE_UNAVAILABLE,
};
use crate::timeline::{OwnerProvider, RecordingTimeline};
use crate::validate::{open_and_validate, OpenOutcome};


fn fixture_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures")
        .join(rel)
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn open_recording_timeline(rel: &str) -> (WorldTimeline, RecordingTimeline) {
    let path = fixture_path(rel);
    let source = open_recording(&path).expect("open recording source");
    let timeline = WorldTimeline::open(source).expect("open world timeline");
    let provider_source = open_recording(&path).expect("reopen recording source");
    let provider_timeline = WorldTimeline::open(provider_source).expect("reopen world timeline");
    let provider = RecordingTimeline::new_with_policy(provider_timeline, None);
    (timeline, provider)
}


fn owner_map_encoding(map: &BTreeMap<u32, Leaf<replay_core::CountryIdentityRef>>) -> String {
    let mut out = String::new();
    for (state, leaf) in map {
        if let Leaf::Measured(tag) = leaf {
            out.push_str(&format!("{state}={};", tag.tag()));
        }
    }
    out
}

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

fn effective_visible_set(frame: &WorldFrame, pov: &PovState) -> BTreeSet<u32> {
    match pov.active_pov().and_then(|selection| selection.named_in(frame)) {
        None => world_visible_universe(frame),
        Some(named) => pov_visible_set(frame, named.tag()),
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

fn effective_spotted_encoding(frame: &WorldFrame, pov: &PovState) -> String {
    match pov.active_pov().and_then(|selection| selection.named_in(frame)) {
        None => "ALL".to_string(),
        Some(named) => {
            let s = frame.spotted_units_for(named.tag());
            format!(
                "div={};fleets={};wings={}",
                leaf_list_encoding(&s.divisions),
                leaf_list_encoding(&s.fleets),
                leaf_list_encoding(&s.wings),
            )
        }
    }
}

fn render_marker(frame: &WorldFrame, pov: &PovState) -> String {
    match pov.render_status(Some(frame)) {
        PovRenderStatus::Omniscient => "render=OMNISCIENT".to_string(),
        PovRenderStatus::Recorded {
            pov,
            province_slot_count,
            ..
        } => format!("render=RECORDED:{pov}:{province_slot_count}"),
        PovRenderStatus::Unavailable { pov, .. } => {
            format!(
                "render=FOWUNAVAILABLE:{pov}:typed POV FoW authority is unavailable; no POV presentation is authenticated"
            )
        }
    }
}

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

fn state_hash(frame: &WorldFrame, pov: &PovState, topbar_tags: &[&str]) -> u64 {
    let mode = match pov.mode() {
        ObserverMode::Omniscient => "OMNISCIENT".to_string(),
        // THE NAME THIS FRAME ADMITS. A POV the census refuses hashes as one
        // unnamed POV rather than as the key it held, which is the same thing
        // the render status shows.
        ObserverMode::Pov(selection) => match selection.named_in(frame) {
            Some(named) => format!("POV:{}", named.tag()),
            None => "POV:<unnamed>".to_string(),
        },
    };
    let visible: Vec<String> = effective_visible_set(frame, pov)
        .iter()
        .map(u32::to_string)
        .collect();
    let canonical = format!(
        "hour={};mode={};{};owners={};controls={};visible=[{}];spotted={};topbar={}",
        frame.hour_index(),
        mode,
        render_marker(frame, pov),
        owner_map_encoding(frame.owner_by_state()),
        owner_map_encoding(frame.control_by_state()),
        visible.join(","),
        effective_spotted_encoding(frame, pov),
        topbar_encoding(frame, topbar_tags),
    );
    fnv1a(canonical.as_bytes())
}


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

// regenerated fixture. A failure against these constants means an encoded

const GOLDEN_CHECKPOINTS: [(i64, u64, u64); 4] = [
    (16959365, 12970066525203477097, 15686549579718863552),
    (16959370, 12570333928860468155, 6550089962123306605),
    (16959372, 2494252011149070333, 9597366305995786683),
    (16959376, 5910307060992128889, 9401900488825342487),
];

const GOLDEN_TOPBAR_SEQ: [u64; 12] = [
    8849946347385598713,
    9818959127643207644,
    15317364410669334441,
    6039244865259841516,
    10567013171782251579,
    3974769653907373032,
    3498716706374584453,
    6669294869916526424,
    8858521383902197739,
    4943888700828941628,
    5236560312844676846,
    1731959386161646928,
];


#[test]
fn e2e_pov_visibility_open_play_scrub_pov_toggle() {
    let rel = "positive/fx_pov_visibility.battleplan";

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

    let (timeline, provider) = open_recording_timeline(rel);
    let hours: Vec<i64> = timeline.hours().to_vec();
    assert_eq!(hours.len(), 12);

    match provider.pov_picker().expect("picker frame must authenticate") {
        PovPicker::Unavailable(reason) => {
            assert!(reason.contains("d18"), "typed gap names d18: {reason}");
        }
        PovPicker::Available { choices, .. } => {
            panic!("session-player subset must not become a country picker: {choices:?}")
        }
    }

    let topbar_tags = ["GER", "ENG", "SOV"];
    let checkpoints = [0usize, 5, 7, 11];

    let run_pass = || -> Vec<(i64, u64, u64)> {
        let mut record = Vec::new();
        for &tick in &checkpoints {
            let hour = hours[tick];
            assert_eq!(provider.seek_hour(hour), provider.play_to_hour(hour));
            assert_eq!(provider.seek_hour(hour), Ok(hour));
            let _ = provider.owners_at(hour).expect("provider owners");

            let frame = timeline.state_at(hour).expect("frame at checkpoint");

            let mut pov = PovState::new();
            let world_hash = state_hash(&frame, &pov, &topbar_tags);
            pov.enter_pov("GER");
            let pov_hash = state_hash(&frame, &pov, &topbar_tags);
            record.push((hour, world_hash, pov_hash));
        }
        record
    };

    let pass_a = run_pass();
    assert_eq!(
        pass_a,
        GOLDEN_CHECKPOINTS.to_vec(),
        "checkpoint hash sequence must match the pinned golden values \
         (an encoded surface drifted)"
    );
    let pass_b = run_pass();
    assert_eq!(
        pass_a, pass_b,
        "hash sequence must be deterministic across passes"
    );

    for (i, &tick) in checkpoints.iter().enumerate() {
        let hour = hours[tick];
        let frame = timeline.state_at(hour).expect("frame");

        let world = PovState::new();
        let world_visible = effective_visible_set(&frame, &world);
        let expected_universe: BTreeSet<u32> = POV_UNIVERSE.iter().copied().collect();
        assert_eq!(
            world_visible, expected_universe,
            "omniscient universe @tick {tick}"
        );

        let mut pov = PovState::new();
        pov.enter_pov("GER");

        match pov.render_status(Some(&frame)) {
            PovRenderStatus::Unavailable { pov: t, diagnostic } => {
                assert_eq!(t, "GER");
                assert!(diagnostic.contains(POV_RECORDED_STATE_UNAVAILABLE));
            }
            other => panic!("legacy POV input must fail closed @tick {tick}: {other:?}"),
        }

        let pov_visible = effective_visible_set(&frame, &pov);
        assert_eq!(
            pov_visible,
            expected_ger_visible(tick),
            "GER visible @tick {tick}"
        );

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

        pov.exit_to_world();
        assert_eq!(
            state_hash(&frame, &pov, &topbar_tags),
            rec_world,
            "toggle-back must restore the world hash @tick {tick}"
        );
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

    let frame0 = timeline.state_at(hours[0]).expect("frame0");
    let ger0 = frame0.spotted_units_for("GER");
    assert_eq!(ger0.divisions, Leaf::Measured(vec!["201:1".to_string()]));
    assert_eq!(ger0.fleets, Leaf::Measured(vec!["501".to_string()]));

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
        .any(|d| d.div_uid == "201:1" && d.owner.tag() == Leaf::Measured("SOV"));
    assert!(
        world_still_has_sov_div,
        "omniscient world still carries the SOV division the POV no longer sees"
    );

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
    assert_eq!(
        a,
        GOLDEN_TOPBAR_SEQ.to_vec(),
        "topbar hash sequence must match the pinned golden values \
         (an encoded surface drifted)"
    );
    let b = run();
    assert_eq!(a, b, "topbar hash sequence deterministic across passes");
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
