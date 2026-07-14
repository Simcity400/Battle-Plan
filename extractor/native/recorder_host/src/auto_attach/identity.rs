//! Process identity for the auto-attach coordinator.
//!
//! The whole coordinator keys on `(pid, creation_filetime)` — the single fact
//! that lets us prove "this is the same process I already attached to" and
//! "this reused PID is a *different* process" (design §2, §3). The FILETIME is
//! kept at full 100 ns precision on purpose: flooring two sub-second-apart
//! launches into one equal second is exactly the collision the design forbids
//! (mirrors the note in `modset.rs::query_process_start`).
//!
//! Nothing here reads live memory or opens a process handle — those live in
//! `sources.rs` behind the `ProcessProbe` trait, so this file (the identity
//! algebra the state machine depends on) is fully unit-testable offline.

/// Raw Windows FILETIME: 100 ns ticks since 1601-01-01 UTC. Stored raw so the
/// identity key never loses precision.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ProcessFiletime(pub u64);

impl ProcessFiletime {
    /// Whole-second unix stamp, for human-readable journal fields ONLY — never
    /// for the identity comparison (that uses the raw ticks).
    pub fn unix_seconds(self) -> i64 {
        const UNIX_EPOCH_TICKS: u64 = 116_444_736_000_000_000;
        if self.0 < UNIX_EPOCH_TICKS {
            // Pre-epoch is structurally impossible for a live process; report a
            // sentinel rather than panic. Callers surface identity errors
            // upstream (the probe rejects such values before we get here).
            return i64::MIN;
        }
        ((self.0 - UNIX_EPOCH_TICKS) / 10_000_000) as i64
    }
}

/// The identity key. Two processes are the same iff BOTH fields match. A reused
/// PID with a new creation FILETIME is a brand-new identity (design §2, gate 3).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ProcessKey {
    pub pid: u32,
    pub creation_filetime: ProcessFiletime,
}

impl ProcessKey {
    pub fn new(pid: u32, creation_filetime: u64) -> Self {
        Self {
            pid,
            creation_filetime: ProcessFiletime(creation_filetime),
        }
    }
}

/// Fully-resolved live process identity, produced by a `ProcessProbe` reading
/// the real OS handle. Every field is authority-grade evidence: the design
/// requires the live token/handle to be authority and WMI's SID/session fields
/// to be treated only as hints (design §3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub key: ProcessKey,
    /// Canonical image path from `QueryFullProcessImageNameW`.
    pub image_path: String,
    /// Windows terminal-services session id (`ProcessIdToSessionId`).
    pub session_id: u32,
    /// SHA-256 (hex) of the process token user SID. Raw SID is never stored or
    /// journalled (design §8 "Do not log raw SID"); the hash is enough to prove
    /// same-user equality.
    pub user_sid_hash: String,
}

impl ProcessIdentity {
    /// Lowercased basename of the image path, e.g. `hoi4.exe`. Windows file
    /// names are case-insensitive, so the coordinator's `hoi4.exe` gate must be
    /// too (design §2/§3).
    pub fn image_basename_lower(&self) -> String {
        let norm = self.image_path.replace('\\', "/");
        norm.rsplit('/')
            .next()
            .unwrap_or(&norm)
            .to_ascii_lowercase()
    }

    /// True iff this identity is game executable by basename.
    pub fn is_hoi4(&self) -> bool {
        self.image_basename_lower() == BATTLE_PLAN_EXE_LOWER
    }
}

/// The game executable basename we attach to. Kept as a constant (not a
/// hard-coded path) so it stays portable: the live process supplies its own
/// full path; we never assume a drive or Steam library (design §3, §7).
pub const BATTLE_PLAN_EXE_LOWER: &str = "hoi4.exe";

/// Case-insensitive `hoi4.exe` basename test over an arbitrary process name /
/// path string, for the cheap first-pass filter on a snapshot or WMI event
/// (before we spend an `OpenProcess` probe).
pub fn name_is_hoi4(name_or_path: &str) -> bool {
    let norm = name_or_path.replace('\\', "/");
    let base = norm.rsplit('/').next().unwrap_or(&norm);
    base.eq_ignore_ascii_case(BATTLE_PLAN_EXE_LOWER)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_distinguishes_reused_pid() {
        let a = ProcessKey::new(1234, 100);
        let b = ProcessKey::new(1234, 200);
        assert_ne!(a, b, "same pid, different FILETIME must be distinct");
    }

    #[test]
    fn key_same_identity_equal() {
        assert_eq!(ProcessKey::new(1234, 100), ProcessKey::new(1234, 100));
    }

    #[test]
    fn filetime_full_precision_not_floored_to_seconds() {
        // Two launches 1 tick (100 ns) apart within the same wall-clock second
        // must remain distinct — the exact sub-second collision the design bans.
        const BASE: u64 = 133_000_000_000_000_000; // arbitrary post-epoch tick
        let a = ProcessKey::new(9, BASE);
        let b = ProcessKey::new(9, BASE + 1);
        assert_ne!(a, b);
        assert_eq!(
            a.creation_filetime.unix_seconds(),
            b.creation_filetime.unix_seconds(),
            "they DO share the same whole second — proving second-precision would collide them"
        );
    }

    #[test]
    fn hoi4_name_match_is_case_and_separator_insensitive() {
        assert!(name_is_hoi4("hoi4.exe"));
        assert!(name_is_hoi4("HOI4.EXE"));
        assert!(name_is_hoi4("Hoi4.Exe"));
        assert!(name_is_hoi4(
            r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV\hoi4.exe"
        ));
        assert!(name_is_hoi4("C:/Games/hoi4.exe"));
        assert!(!name_is_hoi4("hoi4_launcher.exe"));
        assert!(!name_is_hoi4("notepad.exe"));
        assert!(!name_is_hoi4("dowser.exe"));
    }

    #[test]
    fn identity_basename_and_is_hoi4() {
        let id = ProcessIdentity {
            key: ProcessKey::new(10, 500),
            image_path: r"D:\Steam\common\the game\Hoi4.exe".to_string(),
            session_id: 1,
            user_sid_hash: "abc".to_string(),
        };
        assert_eq!(id.image_basename_lower(), "hoi4.exe");
        assert!(id.is_hoi4());
    }
}
