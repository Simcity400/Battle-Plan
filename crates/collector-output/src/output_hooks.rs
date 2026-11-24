use crate::{
    hook::{Hook, TapSafety},
    output_log::Log,
};
use engine_sites::Rva;
use std::{
    ops::Range,
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

#[derive(Clone, Copy)]
pub struct Site {
    pub rva: Rva,
}
impl From<&engine_production::construction_ic_sites::Site> for Site {
    fn from(site: &engine_production::construction_ic_sites::Site) -> Self {
        Self { rva: site.rva }
    }
}
impl From<&engine_production::line_output_sites::HookSite> for Site {
    fn from(site: &engine_production::line_output_sites::HookSite) -> Self {
        Self { rva: site.rva }
    }
}

#[derive(Clone, Copy)]
pub enum Failure {
    Propagate,
    Unavailable,
}
pub struct Domain<const N: usize> {
    pub failure: Failure,
    pub observation: &'static Log,
    pub base: &'static AtomicUsize,
    pub originals: &'static [AtomicUsize; N],
    pub installed: Option<&'static [AtomicBool; N]>,
    pub unhooked: &'static OnceLock<&'static str>,
}

pub fn locate<const N: usize>(
    base: usize,
    sites: &[Site; N],
) -> Result<[(usize, [u8; 16]); N], &'static str> {
    let mut located = [(0, [0; 16]); N];
    for (slot, site) in located.iter_mut().zip(sites) {
        *slot = engine_sites::hook(base, site.rva)
            .and_then(|code| Ok((code.address, code.sentinel::<16>()?)))
            .inspect_err(|cause| {
                crate::hook::installation::refused(
                    crate::hook::installation::Kind::Sentinel,
                    None,
                    cause,
                )
            })?;
    }
    Ok(located)
}

pub struct Hooks<const N: usize> {
    hooks: Vec<Hook>,
    domain: Domain<N>,
}
impl<const N: usize> Hooks<N> {
    /// Locate every site and store every trampoline before publication.
    ///
    /// # Safety
    /// The base must identify the authenticated executable in this process.
    /// The caller owns this family exclusively and must restore its hooks and
    /// quiesce native calls before releasing code or unloading. `callbacks[i]` is the detour for
    /// `sites[i]`, with that site's ABI and process lifetime.
    pub unsafe fn new(
        base: usize,
        sites: [Site; N],
        callbacks: [usize; N],
        domain: Domain<N>,
        ready: Result<(), &'static str>,
    ) -> Self {
        unsafe { Self::construct(base, sites, callbacks, domain, ready, None) }
    }

    /// Bind the native flight counter and callback ranges before publication.
    ///
    /// # Safety
    /// The base must identify the authenticated executable in this process.
    /// The caller owns this family exclusively and must restore its hooks and
    /// quiesce native calls before releasing code or unloading. `callbacks[i]` is the detour for
    /// `sites[i]`, with that site's ABI and process lifetime, and `safety[i]` covers that callback.
    pub unsafe fn new_with_safety(
        base: usize,
        sites: [Site; N],
        callbacks: [usize; N],
        domain: Domain<N>,
        ready: Result<(), &'static str>,
        safety: [TapSafety; N],
    ) -> Self {
        unsafe { Self::construct(base, sites, callbacks, domain, ready, Some(safety)) }
    }

    /// # Safety
    /// `base` identifies the authenticated executable in this process. The caller owns these sites
    /// exclusively and restores the hooks and quiesces native calls before releasing code or unloading.
    /// `callbacks[i]` is the detour for `sites[i]`, with that site's ABI and process lifetime, and
    /// `safety[i]`, when given, is the flight counter and stub that cover that callback.
    unsafe fn construct(
        base: usize,
        sites: [Site; N],
        callbacks: [usize; N],
        domain: Domain<N>,
        ready: Result<(), &'static str>,
        safety: Option<[TapSafety; N]>,
    ) -> Self {
        let install = || -> Result<Vec<Hook>, &'static str> {
            let located = locate(base, &sites)?;
            ready?;
            domain.base.store(base, Ordering::Relaxed);
            let mut hooks =
                engine_allocation::vec(N).map_err(|_| "output hook allocation failed")?;
            let mut safety = safety.map(IntoIterator::into_iter);
            for (index, &(address, original)) in located.iter().enumerate() {
                let hook = if let Some(safety) = safety.as_mut() {
                    unsafe {
                        Hook::new_with_safety(
                            address,
                            callbacks[index],
                            original,
                            safety.next().unwrap(),
                        )
                    }
                } else {
                    unsafe { Hook::new(address, callbacks[index], original) }
                }
                .map_err(|_| "native hook construction failed")?;
                domain.originals[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
            Ok(hooks)
        };
        let hooks = install().unwrap_or_else(|reason| {
            crate::hook::installation::refused(
                crate::hook::installation::Kind::Construction,
                None,
                reason,
            );
            let _ = domain.unhooked.set(reason);
            Vec::new()
        });
        if let Some(installed) = domain.installed {
            for value in installed {
                value.store(!hooks.is_empty(), Ordering::Relaxed);
            }
        }
        Self { hooks, domain }
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        self.set_enabled_with_prepare(enabled, None)
    }

    pub fn set_enabled_prepared(
        &mut self,
        enabled: bool,
        prepare: fn(),
    ) -> Result<bool, &'static str> {
        self.set_enabled_with_prepare(enabled, Some(prepare))
    }

    fn set_enabled_with_prepare(
        &mut self,
        enabled: bool,
        prepare: Option<fn()>,
    ) -> Result<bool, &'static str> {
        let domain = &self.domain;
        if !enabled {
            domain.observation.enabled.store(false, Ordering::SeqCst);
        }
        if let Some(prepare) = prepare {
            if !crate::hook::shared::defer_preparation(prepare) {
                prepare();
            }
        } else {
            domain.observation.invalidate();
        }
        if enabled
            && matches!(domain.failure, Failure::Unavailable)
            && domain.unhooked.get().is_some()
        {
            return Ok(true);
        }
        let publication = if matches!(domain.failure, Failure::Propagate) {
            Hook::set_family_enabled(&mut self.hooks, enabled)
        } else {
            Hook::set_family_enabled(&mut self.hooks, enabled)
        };
        match publication {
            Ok(true) => {}
            Ok(false) => return Ok(false),
            Err(reason) if enabled && matches!(domain.failure, Failure::Unavailable) => {
                domain.observation.enabled.store(false, Ordering::SeqCst);
                let _ = domain.unhooked.set("native hook state change failed");
                rollback_after_publication_error(&reason, || {
                    Hook::set_family_enabled(&mut self.hooks, false)
                })?;
                if let Some(installed) = domain.installed {
                    for value in installed {
                        value.store(false, Ordering::Relaxed);
                    }
                }
                return Ok(true);
            }
            Err(_) => return Err("native hook state change failed"),
        }
        domain
            .observation
            .enabled
            .store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}

pub fn rollback_after_publication_error(
    cause: &str,
    rollback: impl FnOnce() -> Result<bool, String>,
) -> Result<(), &'static str> {
    use crate::hook::installation::{Kind, refused};
    refused(Kind::Publication, None, cause);
    match rollback() {
        Ok(true) => Ok(()),
        Ok(false) => {
            refused(Kind::Rollback, None, "output hook rollback is still in use");
            Err("output hook rollback is still in use")
        }
        Err(cause) => {
            refused(Kind::Rollback, None, &cause);
            Err("native hook rollback failed")
        }
    }
}

pub fn status<'a, const N: usize>(
    observation: &Log,
    unhooked: &'a OnceLock<&'static str>,
    installed: Option<&[AtomicBool; N]>,
) -> crate::status::Object<impl crate::status::Fields + 'a> {
    crate::status::status!({
        "unhooked": unhooked.get(),
        "enabled": observation.enabled.load(Ordering::SeqCst),
    })
    .join(crate::status::Object(InstalledStatus(installed.map(
        |values| values.each_ref().map(|value| value.load(Ordering::Relaxed)),
    ))))
}

struct InstalledStatus<const N: usize>(Option<[bool; N]>);
impl<const N: usize> crate::status::Fields for InstalledStatus<N> {
    fn field_count(&self) -> usize {
        usize::from(self.0.is_some())
    }
    fn serialize<M: serde::ser::SerializeMap>(&self, map: &mut M) -> Result<(), M::Error> {
        if let Some(installed) = self.0 {
            map.serialize_entry("installed", &installed.as_slice())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod installation_tests {
    use super::*;
    #[test]
    fn successful_internal_rollback_still_reports_failed_publication() {
        let (result, report) = crate::hook::installation::capture(|| {
            rollback_after_publication_error("enable failed", || Ok(true))
        });
        assert!(result.is_ok());
        assert_eq!(
            report.failure.unwrap().kind,
            crate::hook::installation::Kind::Publication
        );
    }
    #[test]
    fn first_internal_rollback_error_is_fatal_even_before_family_refusal() {
        let (result, report) = crate::hook::installation::capture(|| {
            rollback_after_publication_error("enable failed", || {
                Err("initial restore failed".into())
            })
        });
        assert!(result.is_err());
        let failure = report.failure.unwrap();
        assert_eq!(failure.kind, crate::hook::installation::Kind::Rollback);
        assert!(failure.cause.contains("initial restore failed"));
    }
}
