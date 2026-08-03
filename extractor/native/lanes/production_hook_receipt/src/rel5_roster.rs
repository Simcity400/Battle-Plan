//! Shared receipt custody for a lane whose production sites are all MinHook
//! normal REL5 detours.
//!
//! # Why this exists
//!
//! Every lane the production hook registry owns has to hand it the same four
//! things: the installed receipts, a one-shot take of the released receipts, a
//! predicate for "did I actually let go", and a rollback that leaves neither
//! side holding custody. D45, D67, D86 and D99 each wrote that logic out by
//! hand. Three more lanes needed it on 2026-08-03 -- D85, D95 and D97, whose
//! producer hooks the shipped passive product never installed at all -- and a
//! fourth, fifth and sixth hand-written copy of a custody state machine is
//! three more places for the same class of bug to live independently.
//!
//! So this is the one implementation those three share. It is deliberately NOT
//! retrofitted onto the existing lanes: they are installed, measured and
//! shipping, and rewriting their custody to prove a refactor is a far worse
//! trade than leaving them alone.
//!
//! # What it does not do
//!
//! It owns receipts, not detours. Creating, enabling, disabling and removing
//! the patch stays with the lane, because the ordering rules around those
//! operations are lane-specific (which sites roll back together, whether a
//! trampoline may be freed, how callbacks are quiesced). What this module
//! guarantees is narrower and exact: a receipt is only ever minted from
//! [`observe_enabled_minhook_rel5`] against the lane's own declared spec, and
//! the installed/released sides can never both hold the same site.

#![cfg(windows)]

use crate::{
    next_teardown_owner_token, observe_enabled_minhook_rel5, observe_released_set,
    ExpectedRvaHook, HookTeardownAttestation, InstalledRvaHookReceipt, ReleasedRvaHookReceipt,
};
use std::sync::Mutex;

/// Custody for `N` REL5 sites belonging to one lane.
///
/// Held in a `static`; `new` is `const` so the specs are fixed at compile time
/// and no runtime path can substitute a different roster.
pub struct Rel5HookRoster<const N: usize> {
    /// Prefixes every diagnostic, so a failure names the lane that produced it.
    lane: &'static str,
    specs: [ExpectedRvaHook; N],
    installed: Mutex<[Option<InstalledRvaHookReceipt>; N]>,
    released: Mutex<[Option<ReleasedRvaHookReceipt>; N]>,
}

impl<const N: usize> Rel5HookRoster<N> {
    pub const fn new(lane: &'static str, specs: [ExpectedRvaHook; N]) -> Self {
        Self {
            lane,
            specs,
            installed: Mutex::new([None; N]),
            released: Mutex::new([None; N]),
        }
    }

    pub fn specs(&self) -> &[ExpectedRvaHook; N] {
        &self.specs
    }

    /// Runtime address of site `index`, from the module base the lane resolved.
    pub fn target(&self, base: usize, index: usize) -> Result<usize, String> {
        let expected = self
            .specs
            .get(index)
            .ok_or_else(|| format!("{} hook spec index {index} is invalid", self.lane))?;
        let rva = usize::try_from(expected.requested_rva)
            .map_err(|_| format!("{} RVA does not fit usize", expected.logical_name))?;
        base.checked_add(rva)
            .ok_or_else(|| format!("{} runtime address overflow", expected.logical_name))
    }

    /// Observe every site as an enabled REL5 patch bound to the lane's own
    /// detour, and persist the receipts.
    ///
    /// `detours[i]` is the detour address site `i` must resolve to; a relay
    /// pointing anywhere else is refused by
    /// [`observe_enabled_minhook_rel5`] rather than recorded.
    ///
    /// Idempotent for a retry: a site that already holds a receipt is
    /// re-observed and the result must be byte-identical, so a second call
    /// after a partial failure cannot quietly replace an earlier attestation
    /// with a different one. A site whose release was already published refuses
    /// outright -- installing over released custody is exactly the state the
    /// two-sided invariant exists to forbid.
    ///
    /// All-or-nothing at the roster level: a failure at site `k` leaves sites
    /// `0..k` persisted, which is what the caller's own rollback then releases.
    /// This function does not roll back detours, because it did not install
    /// them.
    pub fn observe_installed(&self, base: usize, detours: [usize; N]) -> Result<(), String> {
        for index in 0..N {
            let existing = self.installed_slot(index)?;
            if let Some(existing) = existing {
                let observed =
                    self.observe_site(base, index, detours[index], existing.teardown_owner_token)?;
                if observed != existing {
                    return Err(format!(
                        "{} installed hook receipt {} changed during retry",
                        self.lane, self.specs[index].logical_name
                    ));
                }
                continue;
            }
            if self.released_slot(index)?.is_some() {
                return Err(format!(
                    "{} hook receipt {} was already released during install",
                    self.lane, self.specs[index].logical_name
                ));
            }
            let observed =
                self.observe_site(base, index, detours[index], next_teardown_owner_token()?)?;
            let mut installed = self.lock_installed()?;
            if installed[index].is_some() {
                return Err(format!(
                    "{} installed hook receipt {} changed before publication",
                    self.lane, self.specs[index].logical_name
                ));
            }
            installed[index] = Some(observed);
        }
        Ok(())
    }

    /// The complete installed roster. Refuses unless every site holds a
    /// receipt, so a partially observed install can never be reported as an
    /// installed one.
    pub fn installed_receipts(&self) -> Result<[InstalledRvaHookReceipt; N], String> {
        let installed = self.lock_installed()?;
        let mut complete = [None; N];
        complete[..].clone_from_slice(&installed[..]);
        drop(installed);
        let mut out = Vec::with_capacity(N);
        for index in 0..N {
            out.push(complete[index].ok_or_else(|| {
                format!(
                    "{} installed hook receipt {} is absent",
                    self.lane, self.specs[index].logical_name
                )
            })?);
        }
        out.try_into()
            .map_err(|_| format!("{} installed hook roster width mismatch", self.lane))
    }

    /// True once every site holds an installed receipt.
    ///
    /// EXACT, and deliberately fail-open on a poisoned lock: this answers "may
    /// I publish a complete release", and publishing over custody this cannot
    /// read would be a worse answer than declining. For the safety question --
    /// "might a detour still be patched in" -- use
    /// [`Self::may_still_hold_installed_custody`], which fails the other way.
    pub fn has_complete_installed_custody(&self) -> bool {
        self.lock_installed()
            .map(|installed| installed.iter().all(Option::is_some))
            .unwrap_or(false)
    }

    /// True while ANY site may still hold installed custody.
    ///
    /// This is the predicate a lane's `release_pending()` is built from, and it
    /// is pessimistic in both directions [`Self::has_complete_installed_custody`]
    /// is not:
    ///
    ///   * a PARTIAL roster counts. `observe_installed` persists receipts site
    ///     by site, so a failure at site three leaves two attested detours in
    ///     the executable; "not complete" is not "not there".
    ///   * a POISONED lock counts. "I cannot read my own custody" is not
    ///     evidence that there is none, and the registry reads a `false` here as
    ///     proof it may skip the release entirely -- which would strand
    ///     executable detours in a running game.
    pub fn may_still_hold_installed_custody(&self) -> bool {
        self.lock_installed()
            .map(|installed| installed.iter().any(Option::is_some))
            .unwrap_or(true)
    }

    /// True while ANY custody is outstanding on either side.
    ///
    /// The registry's "did this lane really let go" question is answered from
    /// the lane's own `installed()` flag; this is the receipt-side companion,
    /// and it is deliberately pessimistic -- a half-released roster answers
    /// true.
    pub fn has_any_custody(&self) -> bool {
        let installed = self
            .lock_installed()
            .map(|installed| installed.iter().any(Option::is_some))
            .unwrap_or(true);
        let released = self
            .lock_released()
            .map(|released| released.iter().any(Option::is_some))
            .unwrap_or(true);
        installed || released
    }

    /// Publish the complete release for every site, atomically.
    ///
    /// `relay_freed` says whether the lane called MinHook's remove -- which
    /// frees the trampoline -- or only its disable. It is passed rather than
    /// assumed because a receipt that claims a free nobody performed is a
    /// forged attestation, and `observe_released_set` re-reads the target's
    /// bytes to check the restoration but cannot see which of the two calls the
    /// lane made.
    ///
    /// Refuses if any site is missing an installed receipt or already holds a
    /// released one, so a second call after a completed release does not mint a
    /// second set.
    pub fn publish_release(&self, relay_freed: bool) -> Result<(), String> {
        let installed = self.installed_receipts()?;
        {
            let released = self.lock_released()?;
            if released.iter().any(Option::is_some) {
                return Err(format!(
                    "{} release found partial release custody already published",
                    self.lane
                ));
            }
        }
        let attestation = HookTeardownAttestation {
            callbacks_quiesced: true,
            hook_removed: true,
            relay_freed,
        };
        let released = observe_released_set(self.specs, installed, [attestation; N])?;
        let mut installed_slot = self.lock_installed()?;
        let mut released_slot = self.lock_released()?;
        for index in 0..N {
            if installed_slot[index] != Some(installed[index]) || released_slot[index].is_some() {
                return Err(format!(
                    "{} hook receipt custody for {} changed during release",
                    self.lane, self.specs[index].logical_name
                ));
            }
        }
        for index in 0..N {
            released_slot[index] = Some(released[index]);
            installed_slot[index] = None;
        }
        Ok(())
    }

    /// Consume the release roster. One-shot: a second call reports the receipts
    /// as unavailable rather than handing the registry a second copy.
    pub fn take_released_receipts(&self) -> Result<[ReleasedRvaHookReceipt; N], String> {
        let mut released = self.lock_released()?;
        let mut out = Vec::with_capacity(N);
        for index in 0..N {
            out.push(released[index].ok_or_else(|| {
                format!(
                    "{} released hook receipt {} is absent",
                    self.lane, self.specs[index].logical_name
                )
            })?);
        }
        *released = [None; N];
        out.try_into()
            .map_err(|_| format!("{} released hook roster width mismatch", self.lane))
    }

    /// Drop every receipt on both sides.
    ///
    /// This is the ROLLBACK path, for an install that failed before the lane
    /// ever became usable. It publishes nothing: the caller has restored the
    /// target's bytes itself and there is no capture for a release receipt to
    /// belong to. Using it after a successful capture would destroy the
    /// evidence the recorder reconciles against, which is why it is named for
    /// discarding rather than for releasing.
    pub fn discard_all_custody(&self) {
        if let Ok(mut installed) = self.lock_installed() {
            *installed = [None; N];
        }
        if let Ok(mut released) = self.lock_released() {
            *released = [None; N];
        }
    }

    fn observe_site(
        &self,
        base: usize,
        index: usize,
        detour: usize,
        owner_token: u64,
    ) -> Result<InstalledRvaHookReceipt, String> {
        let expected = self.specs[index];
        observe_enabled_minhook_rel5(expected, self.target(base, index)?, detour, owner_token)
    }

    fn installed_slot(&self, index: usize) -> Result<Option<InstalledRvaHookReceipt>, String> {
        Ok(self.lock_installed()?[index])
    }

    fn released_slot(&self, index: usize) -> Result<Option<ReleasedRvaHookReceipt>, String> {
        Ok(self.lock_released()?[index])
    }

    fn lock_installed(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, [Option<InstalledRvaHookReceipt>; N]>, String> {
        self.installed
            .lock()
            .map_err(|_| format!("{} installed hook receipt custody is poisoned", self.lane))
    }

    fn lock_released(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, [Option<ReleasedRvaHookReceipt>; N]>, String> {
        self.released
            .lock()
            .map_err(|_| format!("{} released hook receipt custody is poisoned", self.lane))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MINHOOK_NORMAL_REL5_PRIMITIVE_ID;

    const PREFIX: [u8; 16] = [0x40; 16];

    const SPECS: [ExpectedRvaHook; 2] = [
        ExpectedRvaHook {
            logical_name: "gameplay.dtest.first",
            requested_rva: 0x1000,
            target_patch_len: 5,
            primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
            original_prefix: &PREFIX,
        },
        ExpectedRvaHook {
            logical_name: "gameplay.dtest.second",
            requested_rva: 0x2000,
            target_patch_len: 5,
            primitive_id: MINHOOK_NORMAL_REL5_PRIMITIVE_ID,
            original_prefix: &PREFIX,
        },
    ];

    static ROSTER: Rel5HookRoster<2> = Rel5HookRoster::new("DTEST", SPECS);

    /// An empty roster reports absence rather than an empty success.
    ///
    /// Mutation proof: make `installed_receipts` return `Ok` for a `None` slot
    /// and this fails. That matters because the registry treats a returned
    /// roster as proof the lane owns its detours.
    #[test]
    fn an_unobserved_roster_refuses_to_report_installed_receipts() {
        let error = ROSTER.installed_receipts().unwrap_err();
        assert!(
            error.contains("gameplay.dtest.first") && error.contains("absent"),
            "the refusal must name the missing site: {error}"
        );
        assert!(!ROSTER.has_complete_installed_custody());
    }

    /// The release path refuses a roster it never observed installed, so a
    /// teardown cannot mint release receipts for hooks nobody attested.
    #[test]
    fn releasing_an_unobserved_roster_refuses() {
        assert!(ROSTER.publish_release(true).is_err());
        assert!(ROSTER.take_released_receipts().is_err());
    }

    /// `target` derives from the declared RVA, never from a caller-supplied
    /// address.
    #[test]
    fn target_addresses_come_from_the_declared_rvas() {
        assert_eq!(ROSTER.target(0x1_0000_0000, 0).unwrap(), 0x1_0000_1000);
        assert_eq!(ROSTER.target(0x1_0000_0000, 1).unwrap(), 0x1_0000_2000);
        assert!(ROSTER.target(0x1_0000_0000, 2).is_err());
        assert!(ROSTER.target(usize::MAX, 0).is_err());
    }

    /// A discarded roster holds nothing on EITHER side. This is the state a
    /// failed install must leave behind, and the registry's release plan reads
    /// it.
    ///
    /// The roster is SEEDED on both sides first. An earlier version of this
    /// test discarded an already-empty roster and asserted it was still empty,
    /// which passed with `discard_all_custody` replaced by an empty function and
    /// never looked at released custody at all despite its name -- a test that
    /// could not fail.
    ///
    /// Mutation proof: delete either `*installed = [None; N]` or
    /// `*released = [None; N]` from `discard_all_custody` and this fails on the
    /// corresponding side.
    #[cfg(feature = "test-support")]
    #[test]
    fn discard_clears_both_sides() {
        static DISCARDED: Rel5HookRoster<2> = Rel5HookRoster::new("DTEST", SPECS);
        {
            let mut installed = DISCARDED.lock_installed().unwrap();
            installed[0] = Some(crate::mint_test_installed_receipt(SPECS[0], 1));
            installed[1] = Some(crate::mint_test_installed_receipt(SPECS[1], 2));
        }
        {
            let mut released = DISCARDED.lock_released().unwrap();
            released[1] = Some(crate::mint_test_released_receipt(SPECS[1], 2));
        }
        // Seeded, so the assertions below are about the discard rather than
        // about an empty starting state.
        assert!(DISCARDED.has_complete_installed_custody());
        assert!(DISCARDED.may_still_hold_installed_custody());
        assert!(DISCARDED.has_any_custody());

        DISCARDED.discard_all_custody();

        assert!(!DISCARDED.has_complete_installed_custody());
        assert!(!DISCARDED.may_still_hold_installed_custody());
        assert!(
            !DISCARDED.has_any_custody(),
            "the released side must be cleared too, which is what this test's \
             name claims and what an empty-roster version never checked"
        );
        assert!(DISCARDED.installed_receipts().is_err());
        assert!(
            DISCARDED.take_released_receipts().is_err(),
            "a discarded roster has no release to hand the registry"
        );
        // Read the stores directly as well: the accessors above could in
        // principle refuse for a reason other than emptiness.
        assert!(DISCARDED.lock_installed().unwrap().iter().all(Option::is_none));
        assert!(DISCARDED.lock_released().unwrap().iter().all(Option::is_none));
    }

    /// The width of the reported roster is the width of the declared spec list.
    /// A lane that declares two sites cannot report one.
    ///
    /// Mutation proof: make `installed_receipts` skip absent slots and return
    /// what it holds, and this fails. The registry compares the returned names
    /// against `pinned_hook_names_for_group`, so a short roster would mark a
    /// group installed while one of its detours was unattested.
    #[cfg(feature = "test-support")]
    #[test]
    fn a_partially_observed_roster_is_not_a_complete_one() {
        static PARTIAL: Rel5HookRoster<2> = Rel5HookRoster::new("DTEST", SPECS);
        {
            let mut installed = PARTIAL.lock_installed().unwrap();
            installed[0] = Some(crate::mint_test_installed_receipt(SPECS[0], 1));
        }
        assert!(!PARTIAL.has_complete_installed_custody());
        let error = PARTIAL.installed_receipts().unwrap_err();
        assert!(
            error.contains("gameplay.dtest.second"),
            "the refusal must name the site that is missing, not the one held: {error}"
        );
        assert!(PARTIAL.has_any_custody());
    }

    /// Install refuses a site whose release is already published, so custody
    /// can never be held on both sides at once.
    #[cfg(feature = "test-support")]
    #[test]
    fn installing_over_published_release_custody_refuses() {
        static REPUBLISH: Rel5HookRoster<2> = Rel5HookRoster::new("DTEST", SPECS);
        {
            let mut released = REPUBLISH.lock_released().unwrap();
            released[0] = Some(crate::mint_test_released_receipt(SPECS[0], 1));
        }
        let error = REPUBLISH.observe_installed(0x1_0000_0000, [1, 2]).unwrap_err();
        assert!(
            error.contains("already released"),
            "the refusal must name the two-sided custody violation: {error}"
        );
    }
}
