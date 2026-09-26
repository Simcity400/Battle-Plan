//! The callback copies bounded, exact roster bytes; the spool worker verifies IDs.

use engine::{
    Memory,
    cdlc_manager::{self, Descriptor, mounts_match},
    walk::Outcome,
};
use replay::identity::{Identity, Roster as IdentityRoster, Verification};
use serde_json::{Value, json};
use std::{collections::BTreeSet, mem::size_of};
use windows_sys::Win32::System::Performance::QueryPerformanceCounter;

/// Floor for every sample buffer, and the whole size when no live count hint
/// is available. Larger lists are sized from `cdlc_manager::observe_counts`.
const MINIMUM_CAPACITY: usize = 64;
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

/// Owned rows for one list: the live count plus headroom for names the manager
/// can still add before the sample, never below the floor. An unallocatable
/// hint keeps the floor; the sample then reports its own truncation rather
/// than failing the allocation inside the game process.
fn buffer<T: Default + Clone>(hint: usize) -> Vec<T> {
    let wanted = hint.saturating_add(hint / 4).max(MINIMUM_CAPACITY);
    let mut rows = Vec::new();
    let len = if rows.try_reserve_exact(wanted).is_ok() {
        wanted
    } else {
        MINIMUM_CAPACITY
    };
    rows.resize(len, T::default());
    rows
}

impl Pass {
    fn new(counts: cdlc_manager::Counts) -> Self {
        let unavailable = Outcome::Unavailable {
            reason: "not sampled",
        };
        Self {
            dlc_filter_strings: buffer(counts.dlc_filter_strings),
            requested_mods: buffer(counts.requested_mods),
            dlcs: buffer(counts.dlcs),
            mods: buffer(counts.mods),
            mounts: buffer(counts.mounts),
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

    fn capacities(&self) -> [usize; 5] {
        [
            self.dlc_filter_strings.len(),
            self.requested_mods.len(),
            self.dlcs.len(),
            self.mods.len(),
            self.mounts.len(),
        ]
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
    pass: Pass,
    sampled: bool,
    hour: Option<i32>,
    pub ticks: i64,
}

impl Roster {
    /// Minimum-sized roster for synthetic tests; the loader uses `with_counts`.
    #[cfg(test)]
    pub fn new() -> Self {
        Self::with_counts(cdlc_manager::Counts::default())
    }

    /// Size each list from live counts read before the hook is installed
    /// (`cdlc_manager::observe_counts`), so a large DLC/mod roster is sampled
    /// whole. Allocation happens here, never in the once-only callback read.
    pub fn with_counts(counts: cdlc_manager::Counts) -> Self {
        Self {
            pass: Pass::new(counts),
            sampled: false,
            hour: None,
            ticks: 0,
        }
    }

    pub fn allocated_bytes(&self) -> usize {
        size_of::<Self>() + self.pass.allocated_bytes()
    }

    pub fn sampled(&self) -> bool {
        self.sampled
    }

    /// No allocation, normalization, or I/O occurs in this once-only callback read.
    pub fn sample(&mut self, memory: &impl Memory, base: usize, hour: Option<i32>) {
        if self.sampled {
            return;
        }
        let start = qpc();
        self.hour = hour;
        let pass = &mut self.pass;
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
        let pass = &self.pass;
        for (name, outcome) in NAMES.into_iter().zip(pass.outcomes()) {
            match outcome {
                Outcome::Rows { .. } => {}
                Outcome::Truncated { len, unread_rows } => reasons.push(format!(
                    "{name} truncated after {len} rows; {unread_rows} rows unread",
                )),
                Outcome::Unavailable { reason } => {
                    reasons.push(format!("{name} unavailable: {reason}"));
                }
            }
        }

        let mut memory = IdentityRoster {
            dlc: observed_ids(
                &pass.dlcs,
                pass.observation.dlcs,
                dlc_id,
                "DLC",
                true,
                &mut reasons,
            ),
            mods: observed_ids(
                &pass.mods,
                pass.observation.mods,
                mod_id,
                "mod",
                false,
                &mut reasons,
            ),
        };
        if !memory.mods.is_empty()
            || !launcher.mods.is_empty()
            || !pass.observation.requested_mods.is_empty()
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
        if !mounts_match(&pass.mounts[..pass.mount_outcome.len()], expected_mounts) {
            reasons.push(format!(
                "successful content mounts differ in membership or priority order (observed {}, expected {})",
                pass.mount_outcome.len(), expected_mounts.len(),
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
        let sample = {
            let pass = &self.pass;
            let outcomes: serde_json::Map<_, _> = NAMES
                .into_iter()
                .zip(pass.outcomes())
                .map(|(name, read)| (name.to_owned(), outcome(read)))
                .collect();
            let capacities: serde_json::Map<_, _> = NAMES
                .into_iter()
                .zip(pass.capacities())
                .map(|(name, capacity)| (name.to_owned(), json!(capacity)))
                .collect();
            json!({
                "outcomes": outcomes,
                "capacities": capacities,
                "mount_paths": pass.mounts[..pass.mount_outcome.len()].iter()
                    .map(|path| bytes(path.as_bytes())).collect::<Vec<_>>(),
                "dlcs": entries(&pass.dlcs[..pass.observation.dlcs.len()]),
                "mods": entries(&pass.mods[..pass.observation.mods.len()]),
                "requested_mods": pass.requested_mods[..pass.observation.requested_mods.len()].iter()
                    .map(|path| bytes(path.as_bytes())).collect::<Vec<_>>(),
            })
        };
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
            "expected_mounts": expected_mounts,
            "sample": sample,
            "verification": verification,
        })
    }
}

fn qpc() -> i64 {
    let mut ticks = 0;
    unsafe { QueryPerformanceCounter(&mut ticks) };
    ticks
}

fn same_membership(a: &[String], b: &[String]) -> bool {
    a.len() == b.len() && a.iter().collect::<BTreeSet<_>>() == b.iter().collect::<BTreeSet<_>>()
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
    use engine::msvc_string::Text;
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
        let ids: BTreeSet<_> = descriptors.iter().filter_map(|d| dlc_id(d).ok()).collect();
        let paths: Vec<_> = ids
            .iter()
            .rev()
            .map(|id| format!("F:/game/dlc/{id}"))
            .chain(std::iter::once("F:/game".to_owned()))
            .collect();
        {
            let pass = &mut roster.pass;
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
    fn partial_observations_cannot_verify() {
        let mut roster = captured(&[b"dlc/dlc_a/a.dlc"]);
        roster.pass.observation.dlcs = Outcome::Truncated {
            len: 1,
            unread_rows: 7,
        };
        let mut value = identity(&["dlc_a"]);
        let reason = check(&roster, &mut value, None).unwrap_err();
        assert!(reason.contains("7 rows unread"));
        value.validate().unwrap();
    }

    #[test]
    fn inactive_descriptor_cannot_hide_a_swapped_active_descriptor() {
        let mut roster = captured(&[b"dlc/dlc_a/a.dlc", b"dlc/dlc_a/b.dlc"]);
        roster.pass.dlcs[0].reported_active = false;
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
        roster.pass.observation.mods = Outcome::Rows { len: 1 };
        roster.pass.mods[0].descriptor = text(b"mod/ugc_123.mod");
        roster.pass.mods[0].reported_active = true;
        let mut value = identity(&[]);
        assert!(
            check(&roster, &mut value, None)
                .unwrap_err()
                .contains("mapping is not proven")
        );
        value.validate().unwrap();
        roster.pass.observation.requested_mods = Outcome::Unavailable {
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
    fn extra_or_truncated_mounts_cannot_verify_or_leak_paths() {
        let mut roster = captured(&[]);
        roster.pass.mounts[1] = text(b"C:/private/user/interface-override");
        roster.pass.mount_outcome = Outcome::Rows { len: 2 };
        let mut value = identity(&[]);
        let reason = check(&roster, &mut value, None).unwrap_err();
        assert!(reason.contains("observed 2, expected 1"));
        assert!(!reason.contains("private"));
        value.validate().unwrap();
        let diagnostics = roster.diagnostics(&["F:/game".to_owned()], &value.verification);
        assert_eq!(
            diagnostics["sample"]["mount_paths"][1],
            "C:/private/user/interface-override"
        );
        assert_eq!(diagnostics["verification"]["state"], "unverified");
        roster.pass.mount_outcome = Outcome::Truncated {
            len: 2,
            unread_rows: 3,
        };
        assert!(
            check(&roster, &mut value, None)
                .unwrap_err()
                .contains("3 rows unread")
        );
        roster = captured(&[]);
        roster.pass.mount_outcome = Outcome::Unavailable {
            reason: "mount root unreadable",
        };
        assert!(
            check(&roster, &mut value, None)
                .unwrap_err()
                .contains("mount root unreadable")
        );
    }

    /// A synthetic DLC manager and search path with `dlcs` DLC map entries,
    /// `filters` filter strings, no mods and one mount per DLC plus the root.
    struct Live {
        bytes: Vec<u8>,
        heap: usize,
    }
    impl Live {
        const MANAGER: usize = 0x100;
        const ROOT: usize = 0x1000;
        fn u64(&mut self, at: usize, value: usize) {
            self.bytes[at..at + 8].copy_from_slice(&(value as u64).to_le_bytes());
        }
        fn c_string(&mut self, text: &[u8]) -> usize {
            let at = self.heap;
            self.bytes[at..at + text.len()].copy_from_slice(text);
            self.heap += text.len() + 1;
            at
        }
        /// Heap-backed MSVC std::string (capacity above the inline 15).
        fn string(&mut self, at: usize, text: &[u8]) {
            let data = self.c_string(text);
            self.u64(at, data);
            self.u64(at + 16, text.len());
            self.u64(at + 24, text.len().max(31));
        }
        fn node(&mut self, at: usize, left: usize, parent: usize, right: usize, nil: bool) {
            use engine::msvc_tree_node as tree;
            self.u64(at + tree::LEFT, left);
            self.u64(at + tree::PARENT, parent);
            self.u64(at + tree::RIGHT, right);
            self.bytes[at + tree::NIL] = u8::from(nil);
        }
        fn new(ids: &[String], filters: usize) -> Self {
            use engine::{dlc_content_entry as entry, dlc_content_map_node as content, msvc_map};
            let mut live = Self {
                bytes: vec![0; 0x100_0000],
                heap: 0x80_0000,
            };
            let manager = Self::MANAGER;
            let strings = 0x10000;
            for (offset, count, data) in [
                (cdlc_manager::DLC_FILTER_STRINGS, filters, strings),
                (cdlc_manager::REQUESTED_MODS, 0, 0),
            ] {
                live.u64(manager + offset, data);
                let count = (count as u64) << 32 | count as u64;
                live.u64(manager + offset + 8, count as usize);
            }
            for index in 0..filters {
                let text = format!("filter_{index:04}");
                live.string(strings + index * 32, text.as_bytes());
            }
            // Empty mod map: a sentinel linked to itself.
            let mods = 0x20000;
            live.u64(manager + cdlc_manager::MODS + msvc_map::HEAD, mods);
            live.node(mods, mods, mods, mods, true);
            // DLC map as a right-leaning chain, in id order.
            let head = 0x30000;
            let node = |index: usize| 0x40000 + index * 0x300;
            let last = node(ids.len() - 1);
            live.u64(manager + cdlc_manager::DLCS + msvc_map::HEAD, head);
            live.u64(manager + cdlc_manager::DLCS + msvc_map::COUNT, ids.len());
            live.node(head, node(0), node(0), last, true);
            for (index, id) in ids.iter().enumerate() {
                let at = node(index);
                let parent = if index == 0 { head } else { node(index - 1) };
                let right = if at == last { head } else { node(index + 1) };
                live.node(at, head, parent, right, false);
                let value = at + content::VALUE;
                live.string(value + entry::NAME, id.to_uppercase().as_bytes());
                live.string(
                    value + entry::DESCRIPTOR,
                    format!("dlc/{id}/{id}.dlc").as_bytes(),
                );
                live.bytes[value + entry::DLC_REPORTED_ACTIVE] = 1;
            }
            let mounts: Vec<String> = ids
                .iter()
                .rev()
                .map(|id| format!("F:/game/dlc/{id}"))
                .chain(std::iter::once("F:/game".to_owned()))
                .collect();
            for (index, mount) in mounts.iter().enumerate() {
                let path = live.c_string(mount.as_bytes());
                live.u64(Self::ROOT + index * 8, path);
            }
            live
        }
    }
    impl engine::Memory for Live {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let root = match address {
                cdlc_manager::SINGLETON_RVA => Self::MANAGER,
                cdlc_manager::SEARCH_PATH_RVA => Self::ROOT,
                _ => {
                    return engine::memory::SliceMemory {
                        base: 0,
                        bytes: &self.bytes,
                    }
                    .copy(address, output);
                }
            };
            output.copy_from_slice(&(root as u64).to_le_bytes());
            true
        }
    }

    #[test]
    fn live_counts_size_a_content_roster_beyond_the_floor() {
        let ids: Vec<String> = (0..150).map(|index| format!("dlc{index:03}")).collect();
        let live = Live::new(&ids, 90);
        let counts = cdlc_manager::observe_counts(&live, 0);
        assert_eq!(
            (counts.dlcs, counts.dlc_filter_strings, counts.mounts),
            (150, 90, 151)
        );
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();

        let mut roster = Roster::with_counts(counts);
        roster.sample(&live, 0, Some(100));
        assert_eq!(
            roster.pass.outcomes(),
            [
                Outcome::Rows { len: 90 },
                Outcome::Rows { len: 0 },
                Outcome::Rows { len: 150 },
                Outcome::Rows { len: 0 },
                Outcome::Rows { len: 151 },
            ]
        );
        assert_eq!(
            roster.pass.dlc_filter_strings[89].as_bytes(),
            b"filter_0089"
        );
        let mut value = identity(&refs);
        check(&roster, &mut value, None).unwrap();
        value.validate().unwrap();
        let Verification::Verified { memory, .. } = &value.verification else {
            panic!("a complete roster must verify")
        };
        assert_eq!(memory.dlc, ids);
        let diagnostics = roster.diagnostics(&[], &value.verification);
        assert_eq!(diagnostics["sample"]["dlcs"].as_array().unwrap().len(), 150);
        assert!(diagnostics["sample"]["capacities"]["DLC map"].as_u64() >= Some(150));

        // Without the hint the floor still reports, and never verifies, a
        // truncated roster.
        let mut floor = Roster::new();
        floor.sample(&live, 0, Some(100));
        assert_eq!(
            floor.pass.observation.dlcs,
            Outcome::Truncated {
                len: 64,
                unread_rows: 86
            }
        );
        let mut value = identity(&refs);
        assert!(
            check(&floor, &mut value, None)
                .unwrap_err()
                .contains("86 rows unread")
        );
        // An unreadable manager gives no hint and keeps the floor.
        assert_eq!(
            Roster::with_counts(cdlc_manager::Counts::default())
                .pass
                .capacities(),
            [MINIMUM_CAPACITY; 5]
        );
    }
}
