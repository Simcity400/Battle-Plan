//! things: the installed receipts, a one-shot take of the released receipts, a
//! the installed/released sides can never both hold the same site.

#![cfg(windows)]

use crate::{
    bind_detours_by_identity, next_teardown_owner_token, observe_enabled_minhook_rel5,
    observe_released_set, ExpectedRvaHook, HookTeardownAttestation, InstalledRvaHookReceipt,
    ReleasedRvaHookReceipt, SiteDetour,
};
use std::sync::Mutex;

pub struct Rel5HookRoster<const N: usize> {
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

    /// with a different one. A site whose release was already published refuses
    /// outright -- installing over released custody is exactly the state the
    /// `0..k` persisted, which is what the caller's own rollback then releases.
    pub fn observe_installed(&self, base: usize, detours: [SiteDetour; N]) -> Result<(), String> {
        let detours = bind_detours_by_identity(self.lane, &self.specs, detours)?;
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

    /// I publish a complete release", and publishing over custody this cannot
    pub fn has_complete_installed_custody(&self) -> bool {
        self.lock_installed()
            .map(|installed| installed.iter().all(Option::is_some))
            .unwrap_or(false)
    }

    /// This is the predicate a lane's `release_pending()` is built from, and it
    ///     proof it may skip the release entirely -- which would strand
    pub fn may_still_hold_installed_custody(&self) -> bool {
        self.lock_installed()
            .map(|installed| installed.iter().any(Option::is_some))
            .unwrap_or(true)
    }

    /// and it is deliberately pessimistic -- a half-released roster answers
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
    /// forged attestation, and `observe_released_set` re-reads the target's
    /// released one, so a second call after a completed release does not mint a
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

    /// target's bytes itself and there is no capture for a release receipt to
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

    #[test]
    fn target_addresses_come_from_the_declared_rvas() {
        assert_eq!(ROSTER.target(0x1_0000_0000, 0).unwrap(), 0x1_0000_1000);
        assert_eq!(ROSTER.target(0x1_0000_0000, 1).unwrap(), 0x1_0000_2000);
        assert!(ROSTER.target(0x1_0000_0000, 2).is_err());
        assert!(ROSTER.target(usize::MAX, 0).is_err());
    }

    /// failed install must leave behind, and the registry's release plan reads
    /// never looked at released custody at all despite its name -- a test that
    /// `*released = [None; N]` from `discard_all_custody` and this fails on the
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
        assert!(DISCARDED
            .lock_installed()
            .unwrap()
            .iter()
            .all(Option::is_none));
        assert!(DISCARDED
            .lock_released()
            .unwrap()
            .iter()
            .all(Option::is_none));
    }

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
    #[cfg(feature = "test-support")]
    #[test]
    fn installing_over_published_release_custody_refuses() {
        static REPUBLISH: Rel5HookRoster<2> = Rel5HookRoster::new("DTEST", SPECS);
        {
            let mut released = REPUBLISH.lock_released().unwrap();
            released[0] = Some(crate::mint_test_released_receipt(SPECS[0], 1));
        }
        let error = REPUBLISH
            .observe_installed(0x1_0000_0000, [DETOUR_FIRST, DETOUR_SECOND])
            .unwrap_err();
        assert!(
            error.contains("already released"),
            "the refusal must name the two-sided custody violation: {error}"
        );
    }

    const DETOUR_FIRST: SiteDetour = SiteDetour::new("gameplay.dtest.first", 1);
    const DETOUR_SECOND: SiteDetour = SiteDetour::new("gameplay.dtest.second", 2);

    /// `--release`. The refusal has to happen here rather than at
    /// It runs BEFORE the released-custody check and before any module read, so
    #[test]
    fn two_sites_sharing_one_detour_address_are_refused() {
        static FOLDED: Rel5HookRoster<2> = Rel5HookRoster::new("DTEST", SPECS);
        let folded = SiteDetour::new("gameplay.dtest.second", DETOUR_FIRST.address);
        let error = FOLDED
            .observe_installed(0x1_0000_0000, [DETOUR_FIRST, folded])
            .unwrap_err();
        assert!(
            error.contains("gameplay.dtest.first")
                && error.contains("gameplay.dtest.second")
                && error.contains("share detour address"),
            "the refusal must name both sites and the shared address: {error}"
        );
        assert!(
            !FOLDED.has_any_custody(),
            "a refused fold must mint no custody at all"
        );
    }

    #[test]
    fn a_null_detour_is_refused() {
        static NULLED: Rel5HookRoster<2> = Rel5HookRoster::new("DTEST", SPECS);
        let error = NULLED
            .observe_installed(
                0x1_0000_0000,
                [DETOUR_FIRST, SiteDetour::new("gameplay.dtest.second", 0)],
            )
            .unwrap_err();
        assert!(error.contains("is null"), "{error}");
        assert!(!NULLED.has_any_custody());
    }

    #[test]
    fn a_detour_naming_a_site_the_roster_does_not_have_is_refused() {
        static MISNAMED: Rel5HookRoster<2> = Rel5HookRoster::new("DTEST", SPECS);
        let error = MISNAMED
            .observe_installed(
                0x1_0000_0000,
                [DETOUR_FIRST, SiteDetour::new("gameplay.dtest.third", 2)],
            )
            .unwrap_err();
        assert!(
            error.contains("gameplay.dtest.third") && error.contains("matches 0 sites"),
            "the refusal must name the detour that belongs to no site of this \
             roster: {error}"
        );
        assert!(!MISNAMED.has_any_custody());
    }

    #[test]
    fn a_reordered_detour_table_still_binds_each_site_to_its_own_detour() {
        assert_eq!(
            crate::bind_detours_by_identity("DTEST", &SPECS, [DETOUR_SECOND, DETOUR_FIRST])
                .expect("a reordered table names the same two sites"),
            [DETOUR_FIRST.address, DETOUR_SECOND.address],
            "the bound addresses must be in SPEC order regardless of the order \
             the caller wrote them in"
        );
    }
}
