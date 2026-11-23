
use engine_allocation as allocation;
use engine_dlc::cdlc_manager::{self, Folders, MountDifference, mount_difference};
use engine_native_memory::{Memory, capture_text::Owned, walk::Outcome};
use replay_data::{identity::Identity, identity::Roster as IdentityRoster, identity::Verification};
use serde::{
    Serialize, Serializer,
    ser::{SerializeMap, SerializeSeq},
};
use std::{
    fmt::{self, Write as _},
    io,
    mem::size_of,
};
use windows_sys::Win32::System::Performance::QueryPerformanceCounter;

const NAMES: [&str; 5] = [
    "DLC filter strings",
    "requested mods",
    "DLC map",
    "mod map",
    "successful content mounts",
];

struct Pass {
    dlc_filter_strings: Vec<Owned>,
    requested_mods: Vec<Owned>,
    dlcs: Vec<cdlc_manager::Entry<Owned, Owned>>,
    mods: Vec<cdlc_manager::Entry<Owned, Owned>>,
    observation: cdlc_manager::Observation,
    mounts: Vec<Owned>,
    mount_outcome: Outcome,
}

fn buffer<T: Default>(count: usize) -> Vec<T> {
    allocation::owned_vec(allocation::default_vec(count))
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
            * size_of::<Owned>()
            + (self.dlcs.capacity() + self.mods.capacity())
                * size_of::<cdlc_manager::Entry<Owned, Owned>>()
            + self
                .dlc_filter_strings
                .iter()
                .chain(&self.requested_mods)
                .chain(&self.mounts)
                .map(Owned::allocated_bytes)
                .sum::<usize>()
            + self
                .dlcs
                .iter()
                .chain(&self.mods)
                .map(|row| row.name.allocated_bytes() + row.descriptor.allocated_bytes())
                .sum::<usize>()
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

#[cfg(test)]
impl Default for Roster {
    fn default() -> Self {
        Self::new()
    }
}

impl Roster {
    #[cfg(test)]
    pub fn new() -> Self {
        Self::with_counts(cdlc_manager::Counts::default())
    }

    pub fn with_counts(counts: cdlc_manager::Counts) -> Self {
        Self {
            pass: Pass::new(counts),
            sampled: false,
            hour: None,
            ticks: 0,
        }
    }

    pub fn prepare(&mut self, memory: &impl Memory, base: usize) {
        allocation::advisory(|| {
            let pass = &mut self.pass;
            let _ = cdlc_manager::prepare(
                memory,
                base,
                cdlc_manager::Buffers {
                    dlc_filter_strings: &mut pass.dlc_filter_strings,
                    requested_mods: &mut pass.requested_mods,
                    dlcs: &mut pass.dlcs,
                    mods: &mut pass.mods,
                },
            );
            let _ = cdlc_manager::observe_search_path_owned(memory, base, &mut pass.mounts, true);
        });
    }

    pub fn allocated_bytes(&self) -> usize {
        size_of::<Self>() + self.pass.allocated_bytes()
    }

    pub fn sampled(&self) -> bool {
        self.sampled
    }

    pub fn sample(&mut self, memory: &impl Memory, base: usize, hour: Option<i32>) {
        if self.sampled {
            return;
        }
        let start = qpc();
        self.hour = hour;
        let pass = &mut self.pass;
        pass.observation = cdlc_manager::observe_prepared(
            memory,
            base,
            cdlc_manager::Buffers {
                dlc_filter_strings: &mut pass.dlc_filter_strings,
                requested_mods: &mut pass.requested_mods,
                dlcs: &mut pass.dlcs,
                mods: &mut pass.mods,
            },
        );
        pass.mount_outcome =
            cdlc_manager::observe_search_path_owned(memory, base, &mut pass.mounts, false);
        self.sampled = true;
        self.ticks = qpc() - start;
    }

    pub fn verify(
        &self,
        identity: &mut Identity,
        expected_mounts: &[String],
        mod_descriptors: &[String],
        content_order_note: Option<&str>,
    ) -> io::Result<()> {
        let launcher = identity.try_roster()?;
        let mut reasons = String::new();
        if !self.sampled {
            add_reason(&mut reasons, format_args!("content roster was not sampled"))?;
        } else if self.hour.is_none() {
            add_reason(
                &mut reasons,
                format_args!("content roster sample had no readable hour"),
            )?;
        }
        let pass = &self.pass;
        for (name, outcome) in NAMES.into_iter().zip(pass.outcomes()) {
            match outcome {
                Outcome::Rows { .. } => {}
                Outcome::Truncated { len, unread_rows } => add_reason(
                    &mut reasons,
                    format_args!("{name} truncated after {len} rows; {unread_rows} rows unread",),
                )?,
                Outcome::Unavailable { reason } => {
                    add_reason(&mut reasons, format_args!("{name} unavailable: {reason}",))?
                }
            }
        }
        let excluded = &pass.dlc_filter_strings[..pass.observation.dlc_filter_strings.len()];
        let mut memory = IdentityRoster {
            dlc: observed_ids(
                &pass.dlcs,
                pass.observation.dlcs,
                dlc_id,
                "DLC",
                true,
                |row| {
                    row.reported_active
                        && !excluded
                            .iter()
                            .any(|filter| filter.as_bytes() == row.descriptor.as_bytes())
                },
                &mut reasons,
            )?,
            mods: observed_ids(
                &pass.mods,
                pass.observation.mods,
                mod_id,
                "mod",
                false,
                |row| row.reported_active,
                &mut reasons,
            )?,
        };
        if mod_descriptors.len() != launcher.mods.len() {
            add_reason(
                &mut reasons,
                format_args!(
                    "{} mod descriptor files supplied for {} recorded mods",
                    mod_descriptors.len(),
                    launcher.mods.len(),
                ),
            )?;
        }
        membership("DLC", &launcher.dlc, &memory.dlc, &mut reasons)?;
        membership("mod", mod_descriptors, &memory.mods, &mut reasons)?;
        if let Some(difference) =
            mount_difference(&pass.mounts[..pass.mount_outcome.len()], expected_mounts)
        {
            add_reason(
                &mut reasons,
                format_args!(
                    "successful content mounts differ in membership or priority order (observed {}, expected {}): {}",
                    pass.mount_outcome.len(),
                    expected_mounts.len(),
                    Unlike(difference),
                ),
            )?;
        }
        if let Some(note) = content_order_note {
            add_reason(
                &mut reasons,
                format_args!("content loading order is unverified: {note}"),
            )?;
        }
        if reasons.is_empty() {
            memory.dlc = copy_strings(&launcher.dlc)?;
            memory.mods = copy_strings(&launcher.mods)?;
            identity.verification = Verification::Verified { launcher, memory };
            Ok(())
        } else {
            identity.verification = Verification::Unverified {
                reason: reasons,
                launcher,
                memory,
            };
            Err(io::ErrorKind::InvalidData.into())
        }
    }

    pub fn diagnostics<'a>(
        &'a self,
        expected_mounts: &'a [String],
        verification: &'a Verification,
    ) -> impl Serialize + 'a {
        Diagnostics {
            roster: self,
            expected_mounts,
            verification,
        }
    }
}

fn qpc() -> i64 {
    let mut ticks = 0;
    unsafe { QueryPerformanceCounter(&mut ticks) };
    ticks
}

struct Unlike<'a>(MountDifference<'a>);
impl fmt::Display for Unlike<'_> {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (position, what) = match self.0 {
            MountDifference::NoneExpected => return out.write_str("no mounts were expected"),
            MountDifference::Missing { position, .. } => (position, "was not mounted"),
            MountDifference::Extra { position, .. } => (position, "was not expected"),
            MountDifference::Differs {
                position, folders, ..
            } => (position, differs(folders)),
        };
        write!(
            out,
            "mount {}, highest priority first, {what} (paths in content.json)",
            position + 1
        )
    }
}

fn differs(folders: Folders) -> &'static str {
    match folders {
        Folders::Unreadable => "differs as written and one of the two is no path",
        Folders::Unopened {
            observed: false,
            expected: false,
        } => "differs as written and neither path opens as a folder",
        Folders::Unopened {
            observed: false, ..
        } => "differs as written and the game's path does not open as a folder",
        Folders::Unopened { .. } => {
            "differs as written and the expected path does not open as a folder"
        }
        Folders::Different => "differs as written and names a different folder",
    }
}

fn membership(
    kind: &str,
    launcher: &[String],
    game: &[String],
    reasons: &mut String,
) -> io::Result<()> {
    if let Some(id) = launcher
        .iter()
        .enumerate()
        .find_map(|(index, id)| launcher[..index].contains(id).then_some(id))
    {
        add_reason(
            reasons,
            format_args!("launcher lists {kind} {id} more than once"),
        )?;
    }
    let launcher_only = Only {
        ids: launcher,
        absent_from: game,
    };
    let game_only = Only {
        ids: game,
        absent_from: launcher,
    };
    if launcher_only.is_empty() && game_only.is_empty() {
        return Ok(());
    }
    add_reason(
        reasons,
        format_args!(
            "launcher and in-memory {kind} membership differ (launcher only: {launcher_only}; game only: {game_only})"
        ),
    )
}

struct Only<'a> {
    ids: &'a [String],
    absent_from: &'a [String],
}
impl Only<'_> {
    fn iter(&self) -> impl Iterator<Item = &String> {
        self.ids.iter().filter(|id| !self.absent_from.contains(id))
    }
    fn is_empty(&self) -> bool {
        self.iter().next().is_none()
    }
}
impl fmt::Display for Only<'_> {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return out.write_str("none");
        }
        for (index, id) in self.iter().enumerate() {
            if index > 0 {
                out.write_str(", ")?;
            }
            out.write_str(id)?;
        }
        Ok(())
    }
}

struct CheckedText<'a>(&'a mut String);
impl fmt::Write for CheckedText<'_> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        allocation::reserve_string(self.0, text.len()).map_err(|_| fmt::Error)?;
        self.0.push_str(text);
        Ok(())
    }
}

pub fn add_reason(reasons: &mut String, args: fmt::Arguments<'_>) -> io::Result<()> {
    if !reasons.is_empty() {
        allocation::reserve_string(reasons, 2)?;
        reasons.push_str("; ");
    }
    CheckedText(reasons)
        .write_fmt(args)
        .map_err(|_| allocation::oom())
}

fn copy_strings(rows: &[String]) -> io::Result<Vec<String>> {
    let mut result = allocation::vec(rows.len())?;
    for row in rows {
        result.push(allocation::string(row)?);
    }
    Ok(result)
}

fn observed_ids(
    rows: &[cdlc_manager::Entry<Owned, Owned>],
    outcome: Outcome,
    normalize: fn(&[u8]) -> Result<&str, &'static str>,
    kind: &str,
    check_inactive: bool,
    runs: impl Fn(&cdlc_manager::Entry<Owned, Owned>) -> bool,
    reasons: &mut String,
) -> io::Result<Vec<String>> {
    let rows = &rows[..outcome.len()];
    let mut ids = allocation::vec(rows.len())?;
    for (index, row) in rows.iter().enumerate() {
        let running = runs(row);
        if !running && !check_inactive {
            continue;
        }
        match normalize(row.descriptor.as_bytes()) {
            Ok(id) => {
                if rows[..index].iter().any(|earlier| {
                    (check_inactive || runs(earlier))
                        && normalize(earlier.descriptor.as_bytes()) == Ok(id)
                }) {
                    add_reason(reasons, format_args!("duplicate in-memory {kind} ID: {id}"))?;
                }
                if running && !ids.iter().any(|known| known == id) {
                    ids.push(allocation::string(id)?);
                }
            }
            Err(reason) => add_reason(
                reasons,
                format_args!(
                    "{kind} descriptor {} has no recordable content ID: {reason}",
                    index + 1,
                ),
            )?,
        }
    }
    Ok(ids)
}

fn components(bytes: &[u8]) -> Result<impl Iterator<Item = &str>, &'static str> {
    let text = std::str::from_utf8(bytes).map_err(|_| "descriptor is not UTF-8")?;
    if text.chars().any(|c| c.is_control() || c == ':') {
        return Err("descriptor contains a control character or drive separator");
    }
    if text
        .split(['/', '\\'])
        .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err("descriptor has an empty or relative path component");
    }
    Ok(text.split(['/', '\\']))
}

fn dlc_id(bytes: &[u8]) -> Result<&str, &'static str> {
    let mut parts = components(bytes)?;
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some("dlc"), Some(id), Some(descriptor), None)
            if id.starts_with("dlc") && descriptor.ends_with(".dlc") =>
        {
            Ok(id)
        }
        _ => Err("expected dlc/<DLC folder>/<descriptor>.dlc"),
    }
}

fn mod_id(bytes: &[u8]) -> Result<&str, &'static str> {
    let mut parts = components(bytes)?;
    match (parts.next(), parts.next(), parts.next()) {
        (Some("mod"), Some(descriptor), None) if descriptor.ends_with(".mod") => Ok(descriptor),
        _ => Err("expected mod/<descriptor>.mod; remote file ID is not present in memory"),
    }
}

struct Diagnostics<'a> {
    roster: &'a Roster,
    expected_mounts: &'a [String],
    verification: &'a Verification,
}
impl Serialize for Diagnostics<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(7))?;
        map.serialize_entry("sampled", &self.roster.sampled)?;
        map.serialize_entry("hour", &self.roster.hour)?;
        map.serialize_entry("callback_ticks", &self.roster.ticks)?;
        map.serialize_entry("expected_mounts", self.expected_mounts)?;
        map.serialize_entry("sample", &Sample(&self.roster.pass))?;
        map.serialize_entry("verification", &Verified(self.verification))?;
        let pass = &self.roster.pass;
        map.serialize_entry(
            "mount_difference",
            &mount_difference(
                &pass.mounts[..pass.mount_outcome.len()],
                self.expected_mounts,
            )
            .map(Difference),
        )?;
        map.end()
    }
}
struct Verified<'a>(&'a Verification);
impl Serialize for Verified<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        let state = match self.0 {
            Verification::Unchecked => "unchecked",
            Verification::Verified { .. } => "verified",
            Verification::Unverified { reason, .. } => {
                map.serialize_entry("reason", reason)?;
                "unverified"
            }
        };
        map.serialize_entry("state", state)?;
        map.end()
    }
}
struct Sample<'a>(&'a Pass);
impl Serialize for Sample<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let pass = self.0;
        let mut map = serializer.serialize_map(Some(7))?;
        map.serialize_entry("outcomes", &NamedOutcomes(pass.outcomes()))?;
        map.serialize_entry("capacities", &NamedCapacities(pass.capacities()))?;
        map.serialize_entry(
            "dlc_filter_strings",
            &Descriptors(&pass.dlc_filter_strings[..pass.observation.dlc_filter_strings.len()]),
        )?;
        map.serialize_entry(
            "mount_paths",
            &Descriptors(&pass.mounts[..pass.mount_outcome.len()]),
        )?;
        map.serialize_entry("dlcs", &Entries(&pass.dlcs[..pass.observation.dlcs.len()]))?;
        map.serialize_entry("mods", &Entries(&pass.mods[..pass.observation.mods.len()]))?;
        map.serialize_entry(
            "requested_mods",
            &Descriptors(&pass.requested_mods[..pass.observation.requested_mods.len()]),
        )?;
        map.end()
    }
}
struct NamedOutcomes([Outcome; 5]);
impl Serialize for NamedOutcomes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(NAMES.len()))?;
        for (name, outcome) in NAMES.into_iter().zip(self.0) {
            map.serialize_entry(name, &Read(outcome))?;
        }
        map.end()
    }
}
struct NamedCapacities([usize; 5]);
impl Serialize for NamedCapacities {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(NAMES.len()))?;
        for (name, capacity) in NAMES.into_iter().zip(self.0) {
            map.serialize_entry(name, &capacity)?;
        }
        map.end()
    }
}
struct Read(Outcome);
impl Serialize for Read {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        match self.0 {
            Outcome::Rows { len } => {
                map.serialize_entry("state", "rows")?;
                map.serialize_entry("rows", &len)?;
            }
            Outcome::Truncated { len, unread_rows } => {
                map.serialize_entry("state", "truncated")?;
                map.serialize_entry("rows", &len)?;
                map.serialize_entry("unread_rows", &unread_rows)?;
            }
            Outcome::Unavailable { reason } => {
                map.serialize_entry("state", "unavailable")?;
                map.serialize_entry("reason", reason)?;
            }
        }
        map.end()
    }
}
struct Difference<'a>(MountDifference<'a>);
impl Serialize for Difference<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        let (position, observed, expected, what) = match self.0 {
            MountDifference::NoneExpected => {
                map.serialize_entry("differs", "no mounts were expected")?;
                return map.end();
            }
            MountDifference::Missing { position, expected } => {
                (position, None, Some(expected), "was not mounted")
            }
            MountDifference::Extra { position, observed } => {
                (position, Some(observed), None, "was not expected")
            }
            MountDifference::Differs {
                position,
                observed,
                expected,
                folders,
            } => (position, Some(observed), Some(expected), differs(folders)),
        };
        map.serialize_entry("mount", &(position + 1))?;
        map.serialize_entry("observed", &observed.map(Bytes))?;
        map.serialize_entry("expected", &expected)?;
        map.serialize_entry("differs", what)?;
        map.end()
    }
}
struct Bytes<'a>(&'a [u8]);
impl Serialize for Bytes<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if let Ok(text) = std::str::from_utf8(self.0) {
            return serializer.serialize_str(text);
        }
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry("invalid_utf8_bytes", self.0)?;
        map.end()
    }
}
struct Descriptors<'a>(&'a [Owned]);
impl Serialize for Descriptors<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for row in self.0 {
            seq.serialize_element(&Bytes(row.as_bytes()))?;
        }
        seq.end()
    }
}
struct Entry<'a>(&'a cdlc_manager::Entry<Owned, Owned>);
impl Serialize for Entry<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(3))?;
        map.serialize_entry("name", &Bytes(self.0.name.as_bytes()))?;
        map.serialize_entry("descriptor", &Bytes(self.0.descriptor.as_bytes()))?;
        map.serialize_entry("reported_active", &self.0.reported_active)?;
        map.end()
    }
}
struct Entries<'a>(&'a [cdlc_manager::Entry<Owned, Owned>]);
impl Serialize for Entries<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for row in self.0 {
            seq.serialize_element(&Entry(row))?;
        }
        seq.end()
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn allocation_failure_fixture() -> impl FnOnce() -> io::Result<()> {
    fixtures::allocation_failure_fixture()
}

#[cfg(any(test, feature = "test-support"))]
mod fixtures {
    use super::*;
    use replay_data::{identity::COVERAGE_POLICY_VERSION, identity::Dlc};
    use std::collections::BTreeSet;

    pub(super) fn identity(ids: &[&str]) -> Identity {
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

    pub(super) fn text(bytes: &[u8]) -> Owned {
        Owned {
            bytes: bytes.to_vec(),
            len: bytes.len(),
        }
    }

    pub(super) fn captured(descriptors: &[&[u8]]) -> Roster {
        let mut roster = Roster::with_counts(cdlc_manager::Counts {
            dlcs: descriptors.len(),
            ..Default::default()
        });
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
            pass.mounts = paths.iter().map(|path| text(path.as_bytes())).collect();
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

    pub(super) fn allocation_failure_fixture() -> impl FnOnce() -> io::Result<()> {
        let roster = captured(&[b"dlc/dlc_b/b.dlc", b"dlc/dlc_a/a.dlc"]);
        let mut verified = identity(&["dlc_a", "dlc_b"]);
        let mut refused = identity(&["dlc_a", "dlc_b"]);
        let expected = vec![
            "F:/game/dlc/dlc_b".into(),
            "F:/game/dlc/dlc_a".into(),
            "F:/game".into(),
        ];
        move || {
            roster.verify(&mut verified, &expected, &[], None)?;
            match roster.verify(
                &mut refused,
                &expected,
                &[],
                Some("unresolved root coverage"),
            ) {
                Err(error) if error.kind() == io::ErrorKind::InvalidData => Ok(()),
                Err(error) => Err(error),
                Ok(()) => Err(io::ErrorKind::InvalidData.into()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use replay_data::identity::Mod;

    fn check(roster: &Roster, identity: &mut Identity, note: Option<&str>) -> Result<(), String> {
        let expected: Vec<_> = identity
            .dlc
            .iter()
            .rev()
            .map(|layer| format!("F:/game/dlc/{}", layer.id))
            .chain(std::iter::once("F:/game".to_owned()))
            .collect();
        roster
            .verify(identity, &expected, &[], note)
            .map_err(|error| {
                if let Verification::Unverified { reason, .. } = &identity.verification {
                    reason.clone()
                } else {
                    error.to_string()
                }
            })
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

    fn excluded(roster: &mut Roster, filters: &[&[u8]]) {
        let pass = &mut roster.pass;
        pass.dlc_filter_strings = filters.iter().map(|filter| text(filter)).collect();
        pass.observation.dlc_filter_strings = Outcome::Rows { len: filters.len() };
    }

    fn mounted(roster: &mut Roster, ids: &[&str]) {
        let paths: Vec<_> = ids
            .iter()
            .rev()
            .map(|id| format!("F:/game/dlc/{id}"))
            .chain(std::iter::once("F:/game".to_owned()))
            .collect();
        roster.pass.mount_outcome = Outcome::Rows { len: paths.len() };
        roster.pass.mounts = paths.iter().map(|path| text(path.as_bytes())).collect();
    }

    #[test]
    fn a_launcher_disabled_dlc_the_map_reports_active_is_not_run_and_verifies() {
        let mut roster = captured(&[b"dlc/dlc_a/a.dlc", b"dlc/dlc_b/b.dlc", b"dlc/dlc_c/c.dlc"]);
        excluded(&mut roster, &[b"dlc/dlc_b/b.dlc", b"DLC_A", b"DLC_C"]);
        mounted(&mut roster, &["dlc_a", "dlc_c"]);
        let mut value = identity(&["dlc_a", "dlc_c"]);
        check(&roster, &mut value, None).unwrap();
        value.validate().unwrap();
        let Verification::Verified { memory, .. } = &value.verification else {
            panic!("a reduced DLC set the game honoured must verify")
        };
        assert_eq!(memory.dlc, ["dlc_a", "dlc_c"]);
        let diagnostics =
            serde_json::to_value(roster.diagnostics(&[], &value.verification)).unwrap();
        assert_eq!(
            diagnostics["sample"]["dlc_filter_strings"][0],
            "dlc/dlc_b/b.dlc"
        );
    }

    #[test]
    fn a_membership_refusal_names_each_differing_dlc_and_its_side() {
        let mut roster = captured(&[b"dlc/dlc_a/a.dlc", b"dlc/dlc_b/b.dlc"]);
        roster.pass.dlcs[1].reported_active = false;
        mounted(&mut roster, &["dlc_a"]);
        let mut value = identity(&["dlc_a", "dlc_b"]);
        let reason = check(&roster, &mut value, None).unwrap_err();
        assert!(
            reason.contains(
                "launcher and in-memory DLC membership differ (launcher only: dlc_b; game only: none)"
            ),
            "{reason}"
        );
        value.validate().unwrap();

        let mut roster = captured(&[b"dlc/dlc_a/a.dlc", b"dlc/dlc_c/c.dlc"]);
        excluded(&mut roster, &[b"dlc/dlc_b/b.dlc"]);
        let mut value = identity(&["dlc_a"]);
        let reason = check(&roster, &mut value, None).unwrap_err();
        assert!(
            reason.contains(
                "launcher and in-memory DLC membership differ (launcher only: none; game only: dlc_c)"
            ),
            "{reason}"
        );
        value.validate().unwrap();

        let mut roster = captured(&[b"dlc/dlc_a/a.dlc", b"dlc/dlc_b/b.dlc"]);
        excluded(&mut roster, &[b"dlc/dlc_b/b.dlc", b"DLC_A"]);
        mounted(&mut roster, &["dlc_a", "dlc_b"]);
        let mut value = identity(&["dlc_a", "dlc_b"]);
        let reason = check(&roster, &mut value, None).unwrap_err();
        assert!(
            reason.contains(
                "launcher and in-memory DLC membership differ (launcher only: dlc_b; game only: none)"
            ),
            "{reason}"
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
    fn unexpected_mods_and_failed_reads_remain_explicitly_unverified() {
        let mut roster = captured(&[]);
        roster.pass.observation.mods = Outcome::Rows { len: 1 };
        roster.pass.mods.resize_with(1, Default::default);
        roster.pass.mods[0].descriptor = text(b"mod/ugc_123.mod");
        roster.pass.mods[0].reported_active = true;
        let mut value = identity(&[]);
        assert!(
            check(&roster, &mut value, None)
                .unwrap_err()
                .contains("membership differ")
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
    fn mods_verify_by_their_resolved_descriptor_files_and_mount_order() {
        let mut roster = captured(&[b"dlc/dlc_a/a.dlc"]);
        let mounts = [
            "C:/mods/top",
            "C:/mods/base",
            "F:/game/dlc/dlc_a",
            "F:/game",
        ];
        roster.pass.mount_outcome = Outcome::Rows { len: mounts.len() };
        roster.pass.mounts = mounts.iter().map(|path| text(path.as_bytes())).collect();
        roster.pass.mods.resize_with(2, Default::default);
        roster.pass.observation.mods = Outcome::Rows { len: 2 };
        for (row, descriptor) in roster
            .pass
            .mods
            .iter_mut()
            .zip([b"mod/ugc_42.mod".as_slice(), b"mod/base.mod"])
        {
            row.descriptor = text(descriptor);
            row.reported_active = true;
        }
        let mut value = identity(&["dlc_a"]);
        value.mods = ["base.mod", "42"]
            .map(|id| Mod {
                id: id.into(),
                display_name: id.into(),
                declared_version: String::new(),
                hash: [0; 32],
            })
            .into();
        let expected = mounts.map(str::to_owned);
        let files = ["base.mod".to_owned(), "ugc_42.mod".to_owned()];
        roster.verify(&mut value, &expected, &files, None).unwrap();
        value.validate().unwrap();
        let Verification::Verified { memory, .. } = &value.verification else {
            panic!("a proven modded stack must verify")
        };
        assert_eq!(memory.mods, ["base.mod", "42"]);

        let other = ["base.mod".to_owned(), "ugc_43.mod".to_owned()];
        let swapped = [
            "C:/mods/base",
            "C:/mods/top",
            "F:/game/dlc/dlc_a",
            "F:/game",
        ]
        .map(str::to_owned);
        for (files, mounts, reason) in [
            (&other[..], &expected, "membership differ"),
            (
                &[][..],
                &expected,
                "0 mod descriptor files supplied for 2 recorded mods",
            ),
            (
                &files[..],
                &swapped,
                "priority order (observed 4, expected 4): mount 1, highest priority first, differs as written and ",
            ),
        ] {
            roster.verify(&mut value, mounts, files, None).unwrap_err();
            let Verification::Unverified { reason: error, .. } = &value.verification else {
                panic!()
            };
            assert!(error.contains(reason), "{error}");
            assert!(!error.contains("C:/mods"), "{error}");
            value.validate().unwrap();
        }
        roster
            .verify(&mut value, &swapped, &files, None)
            .unwrap_err();
        let diagnostics =
            serde_json::to_value(roster.diagnostics(&swapped, &value.verification)).unwrap();
        let difference = &diagnostics["mount_difference"];
        assert_eq!(difference["mount"], 1);
        assert_eq!(difference["observed"], "C:/mods/top");
        assert_eq!(difference["expected"], "C:/mods/base");
        let differs = difference["differs"].as_str().unwrap();
        assert!(differs.starts_with("differs as written and "), "{differs}");
        assert_eq!(
            serde_json::to_value(roster.diagnostics(&expected, &value.verification)).unwrap()["mount_difference"],
            serde_json::Value::Null
        );
    }

    #[test]
    fn extra_or_truncated_mounts_cannot_verify_or_leak_paths() {
        let mut roster = captured(&[]);
        roster
            .pass
            .mounts
            .push(text(b"C:/private/user/interface-override"));
        roster.pass.mount_outcome = Outcome::Rows { len: 2 };
        let mut value = identity(&[]);
        let reason = check(&roster, &mut value, None).unwrap_err();
        assert!(
            reason.contains(
                "observed 2, expected 1): mount 2, highest priority first, was not expected"
            ),
            "{reason}"
        );
        assert!(!reason.contains("private"));
        value.validate().unwrap();
        let diagnostics =
            serde_json::to_value(roster.diagnostics(&["F:/game".to_owned()], &value.verification))
                .unwrap();
        assert_eq!(
            diagnostics["sample"]["mount_paths"][1],
            "C:/private/user/interface-override"
        );
        let difference = &diagnostics["mount_difference"];
        assert_eq!(difference["mount"], 2);
        assert_eq!(difference["observed"], "C:/private/user/interface-override");
        assert_eq!(difference["expected"], serde_json::Value::Null);
        assert_eq!(difference["differs"], "was not expected");
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
        fn string(&mut self, at: usize, text: &[u8]) {
            let data = self.c_string(text);
            self.u64(at, data);
            self.u64(at + 16, text.len());
            self.u64(at + 24, text.len().max(31));
        }
        fn node(&mut self, at: usize, left: usize, parent: usize, right: usize, nil: bool) {
            use engine_native_memory::msvc_tree_node as tree;
            self.u64(at + tree::LEFT, left);
            self.u64(at + tree::PARENT, parent);
            self.u64(at + tree::RIGHT, right);
            self.bytes[at + tree::NIL] = u8::from(nil);
        }
        fn new(ids: &[String], filters: usize) -> Self {
            use engine_dlc::{
                dlc_content_entry as entry, dlc_content_map_node as content, msvc_map,
            };
            let mut live = Self {
                bytes: vec![0; 0x100_0000],
                heap: 0x80_0000,
            };
            let manager = Self::MANAGER;
            let strings = 0x500000;
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
            let mods = 0x20000;
            live.u64(manager + cdlc_manager::MODS + msvc_map::HEAD, mods);
            live.node(mods, mods, mods, mods, true);
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
    impl engine_native_memory::Memory for Live {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let root = match address {
                a if a == cdlc_manager::SINGLETON_RVA.0 => Self::MANAGER,
                a if a == cdlc_manager::SEARCH_PATH_RVA.0 => Self::ROOT,
                _ => {
                    return engine_native_memory::memory::SliceMemory {
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
    fn live_counts_and_lengths_prepare_the_complete_content_roster() {
        let ids: Vec<String> = (0..150).map(|index| format!("dlc{index:03}")).collect();
        let live = Live::new(&ids, 90);
        let counts = cdlc_manager::observe_counts(&live, 0);
        assert_eq!(
            (counts.dlcs, counts.dlc_filter_strings, counts.mounts),
            (150, 90, 151)
        );
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();

        let mut roster = Roster::with_counts(counts);
        roster.prepare(&live, 0);
        crate::test_support::with_allocations_denied(|| roster.sample(&live, 0, Some(100)));
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
        let diagnostics =
            serde_json::to_value(roster.diagnostics(&[], &value.verification)).unwrap();
        assert_eq!(diagnostics["sample"]["dlcs"].as_array().unwrap().len(), 150);
        assert!(diagnostics["sample"]["capacities"]["DLC map"].as_u64() >= Some(150));

        let mut floor = Roster::new();
        floor.sample(&live, 0, Some(100));
        assert_eq!(
            floor.pass.observation.dlcs,
            Outcome::Truncated {
                len: 0,
                unread_rows: 150
            }
        );
        let mut value = identity(&refs);
        assert!(
            check(&floor, &mut value, None)
                .unwrap_err()
                .contains("150 rows unread")
        );
        assert_eq!(
            Roster::with_counts(cdlc_manager::Counts::default())
                .pass
                .capacities(),
            [0; 5]
        );
    }
    #[test]
    fn roster_entries_and_long_native_text_are_sampled_without_allocating() {
        use engine_dlc::{dlc_content_entry as entry, dlc_content_map_node as content, msvc_map};
        const COUNT: usize = 41;
        const TEXT: usize = 5_001;
        let mut ids: Vec<String> = (0..COUNT).map(|index| format!("dlc{index:05}")).collect();
        ids[COUNT - 1] = format!("dlc{}", "x".repeat(TEXT));
        let mut live = Live::new(&ids, 90);
        let mod_name = "n".repeat(TEXT);
        let mod_descriptor = format!("mod/{}.mod", "m".repeat(TEXT));
        let requested = format!("C:/content/{mod_descriptor}");
        live.u64(Live::MANAGER + cdlc_manager::REQUESTED_MODS, 0x600000);
        live.u64(
            Live::MANAGER + cdlc_manager::REQUESTED_MODS + 8,
            (1_usize << 32) | 1,
        );
        live.string(0x600000, requested.as_bytes());
        let head = 0x20000;
        let node = 0x21000;
        live.u64(Live::MANAGER + cdlc_manager::MODS + msvc_map::COUNT, 1);
        live.node(head, node, node, node, true);
        live.node(node, head, head, head, false);
        live.string(node + content::VALUE + entry::NAME, mod_name.as_bytes());
        live.string(
            node + content::VALUE + entry::DESCRIPTOR,
            mod_descriptor.as_bytes(),
        );
        let counts = cdlc_manager::observe_counts(&live, 0);
        assert_eq!(
            (counts.dlcs, counts.mods, counts.mounts),
            (COUNT, 1, COUNT + 1)
        );
        let mut roster = Roster::with_counts(counts);
        roster.prepare(&live, 0);
        let bytes = roster.allocated_bytes();
        live.string(
            node + content::VALUE + entry::NAME,
            "z".repeat(TEXT).as_bytes(),
        );
        crate::test_support::with_allocations_denied(|| roster.sample(&live, 0, Some(100)));
        assert_eq!(roster.pass.observation.dlcs, Outcome::Rows { len: COUNT });
        assert_eq!(roster.pass.observation.mods, Outcome::Rows { len: 1 });
        assert_eq!(roster.pass.mount_outcome, Outcome::Rows { len: COUNT + 1 });
        assert_eq!(
            roster.pass.dlcs[COUNT - 1].name.as_bytes(),
            ids[COUNT - 1].to_uppercase().as_bytes()
        );
        assert_eq!(
            roster.pass.dlcs[COUNT - 1].descriptor.as_bytes(),
            format!("dlc/{0}/{0}.dlc", ids[COUNT - 1]).as_bytes()
        );
        assert_eq!(
            roster.pass.mounts[0].as_bytes(),
            format!("F:/game/dlc/{}", ids[COUNT - 1]).as_bytes()
        );
        assert_eq!(
            roster.pass.mods[0].name.as_bytes(),
            "z".repeat(TEXT).as_bytes()
        );
        assert_eq!(
            roster.pass.mods[0].descriptor.as_bytes(),
            mod_descriptor.as_bytes()
        );
        assert!(roster.pass.mods[0].reported_active);
        assert_eq!(roster.allocated_bytes(), bytes);
    }
}
