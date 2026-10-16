//! Installation and state changes common to the output observers.
use crate::{
    hook::{Hook, TapSafety},
    output_log::Log,
};
use engine::Memory;
use std::{
    ops::Range,
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

#[derive(Clone, Copy)]
pub struct Site {
    pub rva: usize,
    pub sentinel: [u8; 16],
}
impl From<&engine::construction_ic_sites::Site> for Site {
    fn from(site: &engine::construction_ic_sites::Site) -> Self {
        Self {
            rva: site.rva,
            sentinel: site.sentinel,
        }
    }
}
impl From<&engine::line_output_sites::HookSite> for Site {
    fn from(site: &engine::line_output_sites::HookSite) -> Self {
        Self {
            rva: site.rva,
            sentinel: site.sentinel,
        }
    }
}

/// Production output retains its existing attachment-error behaviour for
/// enable failures; construction can refuse the table after a safe rollback.
#[derive(Clone, Copy)]
pub enum Failure {
    Propagate,
    Unavailable,
}
pub struct Domain<const N: usize> {
    pub sentinel_error: &'static str,
    pub failure: Failure,
    pub observation: &'static Log,
    pub base: &'static AtomicUsize,
    pub originals: &'static [AtomicUsize; N],
    pub installed: Option<&'static [AtomicBool; N]>,
    pub unhooked: &'static OnceLock<&'static str>,
}

pub fn validate_sentinels(
    memory: &impl Memory,
    base: usize,
    sites: &[Site],
    sentinel_error: &'static str,
) -> Result<(), &'static str> {
    for site in sites {
        if engine::read::<[u8; 16]>(memory, base + site.rva) != Some(site.sentinel) {
            return Err(sentinel_error);
        }
    }
    Ok(())
}

pub struct Hooks<const N: usize> {
    hooks: Vec<Hook>,
    domain: Domain<N>,
}
impl<const N: usize> Hooks<N> {
    /// Verify every sentinel and store every trampoline before publication.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(
        memory: &impl Memory,
        base: usize,
        sites: [Site; N],
        callbacks: [usize; N],
        domain: Domain<N>,
        ready: Result<(), &'static str>,
    ) -> Self {
        unsafe { Self::construct(memory, base, sites, callbacks, domain, ready, None) }
    }

    /// Bind the native flight counter and callback ranges before publication.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new_with_safety(
        memory: &impl Memory,
        base: usize,
        sites: [Site; N],
        callbacks: [usize; N],
        domain: Domain<N>,
        ready: Result<(), &'static str>,
        safety: [TapSafety; N],
    ) -> Self {
        unsafe { Self::construct(memory, base, sites, callbacks, domain, ready, Some(safety)) }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn construct(
        memory: &impl Memory,
        base: usize,
        sites: [Site; N],
        callbacks: [usize; N],
        domain: Domain<N>,
        ready: Result<(), &'static str>,
        safety: Option<[TapSafety; N]>,
    ) -> Self {
        let install = || -> Result<Vec<Hook>, &'static str> {
            validate_sentinels(memory, base, &sites, domain.sentinel_error)?;
            ready?;
            domain.base.store(base, Ordering::Relaxed);
            let mut hooks =
                engine::allocation::vec(N).map_err(|_| "output hook allocation failed")?;
            let mut safety = safety.map(IntoIterator::into_iter);
            for (index, site) in sites.iter().enumerate() {
                let hook = if let Some(safety) = safety.as_mut() {
                    unsafe {
                        Hook::new_with_safety(
                            base + site.rva,
                            callbacks[index],
                            site.sentinel,
                            safety.next().unwrap(),
                        )
                    }
                } else {
                    unsafe { Hook::new(base + site.rva, callbacks[index], site.sentinel) }
                }
                .map_err(|_| "native hook construction failed")?;
                domain.originals[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
            Ok(hooks)
        };
        let hooks = install().unwrap_or_else(|reason| {
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
            Hook::set_family_enabled_attach(&mut self.hooks, enabled)
        } else {
            Hook::set_family_enabled(&mut self.hooks, enabled)
        };
        match publication {
            Ok(true) => {}
            Ok(false) => return Ok(false),
            Err(_) if enabled && matches!(domain.failure, Failure::Unavailable) => {
                domain.observation.enabled.store(false, Ordering::SeqCst);
                let _ = domain.unhooked.set("native hook state change failed");
                if !Hook::set_family_enabled(&mut self.hooks, false)
                    .map_err(|_| "native hook rollback failed")?
                {
                    return Err("output hook rollback is still in use");
                }
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

pub fn status<'a, const N: usize>(
    observation: &Log,
    unhooked: &'a OnceLock<&'static str>,
    installed: Option<&[AtomicBool; N]>,
) -> crate::status::Object<impl crate::status::Fields + 'a> {
    // Optional fields preserve the previous status schema.
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
