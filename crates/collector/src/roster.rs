//! The callback copies bounded, exact roster bytes; the spool worker verifies IDs.

use engine::{
    cdlc_manager::{self, Descriptor},
    memory::LiveMemory,
    read,
    walk::Outcome,
};
use replay::identity::{Identity, Roster as IdentityRoster, Verification};
use serde_json::{Value, json};
use std::{collections::BTreeSet, mem::size_of};
use windows_sys::Win32::System::Performance::QueryPerformanceCounter;

const CAPACITY: usize = 64;
const NAMES: [&str; 5] = [
    "DLC filter strings",
    "requested mods",
    "DLC map",
    "mod map",
    "successful content mounts",
];

struct Pass {
    dlc_filter_strings: Vec<Descriptor>,
    requested_mods: Vec<Descriptor>,
    dlcs: Vec<cdlc_manager::Entry>,
    mods: Vec<cdlc_manager::Entry>,
    observation: cdlc_manager::Observation,
    mounts: Vec<Descriptor>,
    mount_outcome: Outcome,
}

impl Pass {
    fn new() -> Self {
        let unavailable = Outcome::Unavailable {
            reason: "not sampled",
        };
        Self {
            dlc_filter_strings: vec![Descriptor::default(); CAPACITY],
            requested_mods: vec![Descriptor::default(); CAPACITY],
            dlcs: vec![cdlc_manager::Entry::default(); CAPACITY],
            mods: vec![cdlc_manager::Entry::default(); CAPACITY],
            mounts: vec![Descriptor::default(); CAPACITY],
            mount_outcome: unavailable,
            observation: cdlc_manager::Observation {
                dlc_filter_strings: unavailable,
                requested_mods: unavailable,
                dlcs: unavailable,
                mods: unavailable,
            },
        }
    }

    fn allocated_bytes(&self) -> usize {
        (self.dlc_filter_strings.capacity()
            + self.requested_mods.capacity()
            + self.mounts.capacity())
            * size_of::<Descriptor>()
            + (self.dlcs.capacity() + self.mods.capacity()) * size_of::<cdlc_manager::Entry>()
    }

    fn outcomes(&self) -> [Outcome; 5] {
        let o = self.observation;
        [
            o.dlc_filter_strings,
            o.requested_mods,
            o.dlcs,
            o.mods,
            self.mount_outcome,
        ]
    }
}

pub struct Roster {
    passes: [Pass; 2],
    sampled: bool,
    hour: Option<i32>,
    mount_root_stable: bool,
    pub ticks: i64,
}

impl Roster {
    /// Allocate both passes on the worker before exposing this buffer to the hook.
    pub fn new() -> Self {
        Self {
            passes: std::array::from_fn(|_| Pass::new()),
            sampled: false,
            hour: None,
            mount_root_stable: false,
            ticks: 0,
        }
    }

    pub fn allocated_bytes(&self) -> usize {
        size_of::<Self>() + self.passes.iter().map(Pass::allocated_bytes).sum::<usize>()
    }

    pub fn sampled(&self) -> bool {
        self.sampled
    }

    /// No allocation, normalization, or I/O occurs in this once-only callback read.
    pub fn sample(&mut self, memory: &LiveMemory, base: usize, hour: Option<i32>) {
        if self.sampled {
            return;
        }
        let start = qpc();
        self.hour = hour;
        let root_address = base.checked_add(cdlc_manager::SEARCH_PATH_RVA);
        let root_before = root_address.and_then(|at| read::<u64>(memory, at));
        for pass in &mut self.passes {
            pass.observation = cdlc_manager::observe(
                memory,
                base,
                cdlc_manager::Buffers {
                    dlc_filter_strings: &mut pass.dlc_filter_strings,
                    requested_mods: &mut pass.requested_mods,
                    dlcs: &mut pass.dlcs,
                    mods: &mut pass.mods,
                },
            );
            pass.mount_outcome = cdlc_manager::observe_search_path(memory, base, &mut pass.mounts);
        }
        let root_after = root_address.and_then(|at| read::<u64>(memory, at));
        self.mount_root_stable =
            root_before.is_some_and(|root| root != 0) && root_before == root_after;
        self.sampled = true;
        self.ticks = qpc() - start;
    }

    /// Worker only. `expected_mounts` is the complete successful search path in
    /// highest-priority-first order, resolved from the identity's content layers
    /// and known roots with no covered content. Paths never enter the replay.
    /// `None` means the caller found no unresolved ordering or coverage limitation.
    /// Every failure is recorded and returned without discarding captured state.
    pub fn verify(
        &self,
        identity: &mut Identity,
        expected_mounts: &[String],
        content_order_note: Option<&str>,
    ) -> Result<(), String> {
        let launcher = identity.roster();
        let mut reasons = Vec::new();
        if !self.sampled {
            reasons.push("content roster was not sampled".to_owned());
        } else if self.hour.is_none() {
            reasons.push("content roster sample had no readable hour".to_owned());
        }
        if !self.mount_root_stable {
            reasons.push(
                "successful content mount root changed or was unreadable across both reads"
                    .to_owned(),
            );
        }
        for (index, pass) in self.passes.iter().enumerate() {
            for (name, outcome) in NAMES.into_iter().zip(pass.outcomes()) {
                match outcome {
                    Outcome::Rows { .. } => {}
                    Outcome::Truncated { len, unread_rows } => reasons.push(format!(
                        "{name} pass {} truncated after {len} rows; {unread_rows} rows unread",
                        index + 1,
                    )),
                    Outcome::Unavailable { reason } => {
                        reasons.push(format!("{name} pass {} unavailable: {reason}", index + 1));
                    }
                }
            }
        }
        let [left, right] = &self.passes;
        let matches = [
            equal_rows(
                &left.dlc_filter_strings,
                &right.dlc_filter_strings,
                left.observation.dlc_filter_strings,
                right.observation.dlc_filter_strings,
            ),
            equal_rows(
                &left.requested_mods,
                &right.requested_mods,
                left.observation.requested_mods,
                right.observation.requested_mods,
            ),
            equal_rows(
                &left.dlcs,
                &right.dlcs,
                left.observation.dlcs,
                right.observation.dlcs,
            ),
            equal_rows(
                &left.mods,
                &right.mods,
                left.observation.mods,
                right.observation.mods,
            ),
            equal_rows(
                &left.mounts,
                &right.mounts,
                left.mount_outcome,
                right.mount_outcome,
            ),
        ];
        for (name, matching) in NAMES.into_iter().zip(matches) {
            if !matching {
                reasons.push(format!("{name} changed between roster reads"));
            }
        }

        let mut memory = IdentityRoster {
            dlc: observed_ids(
                &left.dlcs,
                left.observation.dlcs,
                dlc_id,
                "DLC",
                true,
                &mut reasons,
            ),
            mods: observed_ids(
                &left.mods,
                left.observation.mods,
                mod_id,
                "mod",
                false,
                &mut reasons,
            ),
        };
        if !memory.mods.is_empty()
            || !launcher.mods.is_empty()
            || !left.observation.requested_mods.is_empty()
        {
            reasons.push(
                "mod descriptor to content ID mapping is not proven by this collector".to_owned(),
            );
        }
        if !same_membership(&memory.dlc, &launcher.dlc)
            || !same_membership(&memory.mods, &launcher.mods)
        {
            reasons.push("launcher and in-memory content membership differ".to_owned());
        }
        if !mounts_match(&left.mounts[..left.mount_outcome.len()], expected_mounts) {
            reasons.push(format!(
                "successful content mounts differ in membership or priority order (observed {}, expected {})",
                left.mount_outcome.len(), expected_mounts.len(),
            ));
        }
        if let Some(note) = content_order_note {
            reasons.push(format!("content loading order is unverified: {note}"));
        }
        if reasons.is_empty() {
            // The diagnostic map is ordered by display name. Successful mounts
            // independently matched the identity's layer order, so record that
            // proven order after checking exact diagnostic membership as well.
            memory.dlc.clone_from(&launcher.dlc);
            memory.mods.clone_from(&launcher.mods);
            identity.verification = Verification::Verified { launcher, memory };
            Ok(())
        } else {
            let reason = reasons.join("; ");
            identity.verification = Verification::Unverified {
                reason: reason.clone(),
                launcher,
                memory,
            };
            Err(reason)
        }
    }

    /// Worker-only local sidecar data. Contains machine paths and must never be
    /// included in the replay identity or in any spool/replay control record.
    pub fn diagnostics(&self, expected_mounts: &[String], verification: &Verification) -> Value {
        fn bytes(bytes: &[u8]) -> Value {
            match std::str::from_utf8(bytes) {
                Ok(text) => json!(text),
                Err(_) => json!({ "invalid_utf8_bytes": bytes }),
            }
        }
        fn outcome(outcome: Outcome) -> Value {
            match outcome {
                Outcome::Rows { len } => json!({ "state": "rows", "rows": len }),
                Outcome::Truncated { len, unread_rows } => {
                    json!({ "state": "truncated", "rows": len, "unread_rows": unread_rows })
                }
                Outcome::Unavailable { reason } => {
                    json!({ "state": "unavailable", "reason": reason })
                }
            }
        }
        fn entries(rows: &[cdlc_manager::Entry]) -> Vec<Value> {
            rows.iter()
                .map(|row| {
                    json!({
                        "name": bytes(row.name.as_bytes()),
                        "descriptor": bytes(row.descriptor.as_bytes()),
                        "reported_active": row.reported_active,
                    })
                })
                .collect()
        }
        let passes: Vec<_> = self.passes.iter().enumerate().map(|(index, pass)| {
            let outcomes: serde_json::Map<_, _> = NAMES.into_iter().zip(pass.outcomes())
                .map(|(name, read)| (name.to_owned(), outcome(read))).collect();
            json!({
                "pass": index + 1,
                "outcomes": outcomes,
                "mount_paths": pass.mounts[..pass.mount_outcome.len()].iter()
                    .map(|path| bytes(path.as_bytes())).collect::<Vec<_>>(),
                "dlcs": entries(&pass.dlcs[..pass.observation.dlcs.len()]),
                "mods": entries(&pass.mods[..pass.observation.mods.len()]),
                "requested_mods": pass.requested_mods[..pass.observation.requested_mods.len()].iter()
                    .map(|path| bytes(path.as_bytes())).collect::<Vec<_>>(),
            })
        }).collect();
        let verification = match verification {
            Verification::Unchecked => json!({ "state": "unchecked" }),
            Verification::Verified { .. } => json!({ "state": "verified" }),
            Verification::Unverified { reason, .. } => {
                json!({ "state": "unverified", "reason": reason })
            }
        };
        json!({
            "sampled": self.sampled,
            "hour": self.hour,
            "callback_ticks": self.ticks,
            "mount_root_stable": self.mount_root_stable,
            "expected_mounts": expected_mounts,
            "passes": passes,
            "verification": verification,
        })
    }
}

fn qpc() -> i64 {
    let mut ticks = 0;
    unsafe { QueryPerformanceCounter(&mut ticks) };
    ticks
}

fn equal_rows<T: PartialEq>(left: &[T], right: &[T], a: Outcome, b: Outcome) -> bool {
    a == b && left[..a.len()] == right[..b.len()]
}

fn same_membership(a: &[String], b: &[String]) -> bool {
    a.len() == b.len() && a.iter().collect::<BTreeSet<_>>() == b.iter().collect::<BTreeSet<_>>()
}

fn mounts_match(observed: &[Descriptor], expected: &[String]) -> bool {
    // Empty lists cannot establish even the vanilla content root. Normalize only
    // equivalent Windows path notation, without case folding, component
    // resolution, or replacement of invalid UTF-8.
    !expected.is_empty()
        && observed.len() == expected.len()
        && observed.iter().zip(expected).all(|(actual, wanted)| {
            let actual = actual.as_str().and_then(mount_path);
            let wanted = mount_path(wanted);
            actual.is_some() && actual == wanted
        })
}

fn mount_path(path: &str) -> Option<String> {
    if path.is_empty() || path.chars().any(char::is_control) {
        return None;
    }
    let slashes = path.replace('\\', "/");
    let mut normalized = if let Some(unc) = slashes.strip_prefix("//?/UNC/") {
        format!("//{unc}")
    } else {
        slashes.strip_prefix("//?/").unwrap_or(&slashes).to_owned()
    };
    let bytes = normalized.as_bytes();
    let root_len = if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && bytes[2] == b'/'
    {
        3 // C:/ must never become the drive-relative path C:.
    } else {
        usize::from(normalized.starts_with('/'))
    };
    normalized.truncate(normalized.trim_end_matches('/').len().max(root_len));
    (!normalized.is_empty()).then_some(normalized)
}

fn observed_ids(
    rows: &[cdlc_manager::Entry],
    outcome: Outcome,
    normalize: fn(&[u8]) -> Result<String, &'static str>,
    kind: &str,
    check_inactive: bool,
    reasons: &mut Vec<String>,
) -> Vec<String> {
    let mut ids = Vec::new();
    let mut seen = BTreeSet::new();
    for (index, row) in rows[..outcome.len()].iter().enumerate() {
        if !row.reported_active && !check_inactive {
            continue;
        }
        match normalize(row.descriptor.as_bytes()) {
            Ok(id) => {
                // Two descriptors in one DLC folder are ambiguous even when only
                // one is active: its folder ID cannot identify which was mounted.
                if !seen.insert(id.clone()) {
                    reasons.push(format!("duplicate in-memory {kind} ID: {id}"));
                }
                if row.reported_active && !ids.contains(&id) {
                    ids.push(id);
                }
            }
            Err(reason) => reasons.push(format!(
                "{kind} descriptor {} has no recordable content ID: {reason}",
                index + 1,
            )),
        }
    }
    ids
}

fn components(bytes: &[u8]) -> Result<Vec<&str>, &'static str> {
    let text = std::str::from_utf8(bytes).map_err(|_| "descriptor is not UTF-8")?;
    if text.chars().any(|c| c.is_control() || c == ':') {
        return Err("descriptor contains a control character or drive separator");
    }
    let parts: Vec<_> = text.split(['/', '\\']).collect();
    if parts
        .iter()
        .any(|part| part.is_empty() || matches!(*part, "." | ".."))
    {
        return Err("descriptor has an empty or relative path component");
    }
    Ok(parts)
}

fn dlc_id(bytes: &[u8]) -> Result<String, &'static str> {
    let parts = components(bytes)?;
    match parts.as_slice() {
        ["dlc", id, descriptor] if id.starts_with("dlc") && descriptor.ends_with(".dlc") => {
            Ok((*id).to_owned())
        }
        _ => Err("expected dlc/<DLC folder>/<descriptor>.dlc"),
    }
}

fn mod_id(bytes: &[u8]) -> Result<String, &'static str> {
    let parts = components(bytes)?;
    match parts.as_slice() {
        ["mod", descriptor] if descriptor.ends_with(".mod") => Ok((*descriptor).to_owned()),
        _ => Err("expected mod/<descriptor>.mod; remote file ID is not present in memory"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::pdx::Text;
    use replay::identity::{COVERAGE_POLICY_VERSION, Dlc};

    fn identity(ids: &[&str]) -> Identity {
        Identity {
            game_version: "test".into(),
            exe_hash: [0; 32],
            exe_size: 1,
            vanilla_hash: [0; 32],
            dlc: ids
                .iter()
                .map(|id| Dlc {
                    id: (*id).into(),
                    hash: [0; 32],
                    declared_checksum: None,
                })
                .collect(),
            mods: Vec::new(),
            coverage_policy: COVERAGE_POLICY_VERSION,
            language: "english".into(),
            recorder_version: "test".into(),
            collector_hash: [0; 32],
            verification: Verification::Unchecked,
        }
    }

    fn text<const N: usize>(bytes: &[u8]) -> Text<N> {
        let mut text = Text {
            len: bytes.len(),
            ..Text::default()
        };
        text.bytes[..bytes.len()].copy_from_slice(bytes);
        text
    }

    fn captured(descriptors: &[&[u8]]) -> Roster {
        let mut roster = Roster::new();
        roster.sampled = true;
        roster.hour = Some(100);
        roster.mount_root_stable = true;
        let ids: BTreeSet<_> = descriptors.iter().filter_map(|d| dlc_id(d).ok()).collect();
        let paths: Vec<_> = ids
            .iter()
            .rev()
            .map(|id| format!("F:/game/dlc/{id}"))
            .chain(std::iter::once("F:/game".to_owned()))
            .collect();
        for pass in &mut roster.passes {
            pass.mount_outcome = Outcome::Rows { len: paths.len() };
            for (row, path) in pass.mounts.iter_mut().zip(&paths) {
                *row = text(path.as_bytes());
            }
            pass.observation = cdlc_manager::Observation {
                dlc_filter_strings: Outcome::Rows { len: 0 },
                requested_mods: Outcome::Rows { len: 0 },
                dlcs: Outcome::Rows {
                    len: descriptors.len(),
                },
                mods: Outcome::Rows { len: 0 },
            };
            for (row, descriptor) in pass.dlcs.iter_mut().zip(descriptors) {
                row.descriptor = text(descriptor);
                row.reported_active = true;
            }
        }
        roster
    }

    fn check(roster: &Roster, identity: &mut Identity, note: Option<&str>) -> Result<(), String> {
        let expected: Vec<_> = identity
            .dlc
            .iter()
            .rev()
            .map(|layer| format!("F:/game/dlc/{}", layer.id))
            .chain(std::iter::once("F:/game".to_owned()))
            .collect();
        roster.verify(identity, &expected, note)
    }

    #[test]
    fn membership_and_proven_order_are_both_required() {
        let roster = captured(&[b"dlc/dlc_b/b.dlc", b"dlc/dlc_a/a.dlc"]);
        let mut value = identity(&["dlc_a", "dlc_b"]);
        check(&roster, &mut value, None).unwrap();
        value.validate().unwrap();
        assert!(matches!(value.verification, Verification::Verified { .. }));
        assert!(check(&roster, &mut value, Some("unresolved root coverage")).is_err());
        value.validate().unwrap();
        let Verification::Unverified { memory, .. } = &value.verification else {
            panic!()
        };
        assert_eq!(memory.dlc, ["dlc_b", "dlc_a"]);
        value.dlc.reverse();
        assert!(
            check(&roster, &mut value, None)
                .unwrap_err()
                .contains("priority order")
        );
        value.validate().unwrap();
    }

    #[test]
    fn partial_and_changed_observations_cannot_verify() {
        let mut roster = captured(&[b"dlc/dlc_a/a.dlc"]);
        roster.passes[1].observation.dlcs = Outcome::Truncated {
            len: 1,
            unread_rows: 7,
        };
        let mut value = identity(&["dlc_a"]);
        let reason = check(&roster, &mut value, None).unwrap_err();
        assert!(reason.contains("7 rows unread"));
        assert!(reason.contains("changed between roster reads"));
        value.validate().unwrap();
        roster.passes[1].observation.dlcs = Outcome::Rows { len: 1 };
        roster.passes[1].dlcs[0].name = text(b"different");
        assert!(
            check(&roster, &mut value, None)
                .unwrap_err()
                .contains("changed between roster reads")
        );
    }

    #[test]
    fn inactive_descriptor_cannot_hide_a_swapped_active_descriptor() {
        let mut roster = captured(&[b"dlc/dlc_a/a.dlc", b"dlc/dlc_a/b.dlc"]);
        for pass in &mut roster.passes {
            pass.dlcs[0].reported_active = false;
        }
        let mut value = identity(&["dlc_a"]);
        assert!(
            check(&roster, &mut value, None)
                .unwrap_err()
                .contains("duplicate in-memory DLC ID")
        );
        value.validate().unwrap();
    }

    #[test]
    fn ambiguous_descriptors_never_become_machine_paths_or_guessed_ids() {
        for bytes in [
            b"C:/game/dlc/dlc_a/a.dlc".as_slice(),
            b"dlc/../a.dlc",
            b"dlc/dlc_a/\xff.dlc",
        ] {
            let roster = captured(&[bytes]);
            let mut value = identity(&[]);
            assert!(check(&roster, &mut value, None).is_err());
            value.validate().unwrap();
            let Verification::Unverified { memory, reason, .. } = value.verification else {
                panic!()
            };
            assert!(memory.dlc.is_empty());
            assert!(!reason.contains("C:/game"));
        }
        assert_eq!(dlc_id(b"dlc\\dlc_a\\a.dlc").unwrap(), "dlc_a");
        assert_eq!(mod_id(b"mod/ugc_123.mod").unwrap(), "ugc_123.mod");
    }

    #[test]
    fn mods_and_failed_reads_remain_explicitly_unverified() {
        let mut roster = captured(&[]);
        for pass in &mut roster.passes {
            pass.observation.mods = Outcome::Rows { len: 1 };
            pass.mods[0].descriptor = text(b"mod/ugc_123.mod");
            pass.mods[0].reported_active = true;
        }
        let mut value = identity(&[]);
        assert!(
            check(&roster, &mut value, None)
                .unwrap_err()
                .contains("mapping is not proven")
        );
        value.validate().unwrap();
        roster.passes[0].observation.requested_mods = Outcome::Unavailable {
            reason: "bad pointer",
        };
        assert!(
            check(&roster, &mut value, None)
                .unwrap_err()
                .contains("bad pointer")
        );
        assert!(
            check(&Roster::new(), &mut value, None)
                .unwrap_err()
                .contains("not sampled")
        );
    }

    #[test]
    fn extra_changed_or_truncated_mounts_cannot_verify_or_leak_paths() {
        let mut roster = captured(&[]);
        for pass in &mut roster.passes {
            pass.mounts[1] = text(b"C:/private/user/interface-override");
            pass.mount_outcome = Outcome::Rows { len: 2 };
        }
        let mut value = identity(&[]);
        let reason = check(&roster, &mut value, None).unwrap_err();
        assert!(reason.contains("observed 2, expected 1"));
        assert!(!reason.contains("private"));
        value.validate().unwrap();
        let diagnostics = roster.diagnostics(&["F:/game".to_owned()], &value.verification);
        assert_eq!(
            diagnostics["passes"][0]["mount_paths"][1],
            "C:/private/user/interface-override"
        );
        assert_eq!(
            diagnostics["passes"][1]["mount_paths"][1],
            "C:/private/user/interface-override"
        );
        assert_eq!(diagnostics["verification"]["state"], "unverified");
        roster.passes[1].mount_outcome = Outcome::Truncated {
            len: 2,
            unread_rows: 3,
        };
        assert!(
            check(&roster, &mut value, None)
                .unwrap_err()
                .contains("3 rows unread")
        );
        roster = captured(&[]);
        roster.mount_root_stable = false;
        assert!(
            check(&roster, &mut value, None)
                .unwrap_err()
                .contains("mount root changed")
        );
        assert!(mounts_match(&[text(b"F:/game")], &["F:\\game".to_owned()]));
        assert!(!mounts_match(&[text(b"F:/game")], &["F:/GAME".to_owned()]));
    }

    #[test]
    fn equivalent_mount_notation_preserves_root_membership_and_priority() {
        let actual = [text(b"F:/game/dlc/a/"), text(b"F:/game/")];
        let expected = [r"\\?\F:\game\dlc\a".to_owned(), r"\\?\F:\game".to_owned()];
        assert!(mounts_match(&actual, &expected));
        assert!(mounts_match(&[text(b"F:/")], &[r"\\?\F:\".to_owned()]));
        assert_eq!(mount_path("/"), Some("/".to_owned()));
        assert!(!mounts_match(&[text(b"F:/")], &["F:".to_owned()]));
        assert!(!mounts_match(
            &[text(b"F:/game/")],
            &[r"\\?\C:\game".to_owned()]
        ));
        assert!(!mounts_match(
            &actual,
            &[expected[1].clone(), expected[0].clone()]
        ));
        assert!(mounts_match(
            &[text(b"//server/share/game/")],
            &[r"\\?\UNC\server\share\game".to_owned()]
        ));
        assert!(!mounts_match(
            &[text(b"F:/game\xff")],
            &["F:/game".to_owned()]
        ));
        assert!(!mounts_match(
            &[text(b"F:/game")],
            &["F:/game\0".to_owned()]
        ));
    }
}
