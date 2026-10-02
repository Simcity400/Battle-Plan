//! Installation and state changes common to the output observers.
use crate::{hook::Hook, output_log::Log};
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
    pub name: &'static str,
    pub rva: usize,
    pub sentinel: [u8; 16],
}
impl From<&engine::construction_ic_sites::Site> for Site {
    fn from(site: &engine::construction_ic_sites::Site) -> Self {
        Self {
            name: site.name,
            rva: site.rva,
            sentinel: site.sentinel,
        }
    }
}
impl From<&engine::line_output_sites::HookSite> for Site {
    fn from(site: &engine::line_output_sites::HookSite) -> Self {
        Self {
            name: site.name,
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
    pub name: &'static str,
    pub failure: Failure,
    pub observation: &'static Log,
    pub base: &'static AtomicUsize,
    pub originals: &'static [AtomicUsize; N],
    pub installed: Option<&'static [AtomicBool; N]>,
    pub unhooked: &'static OnceLock<String>,
}

pub fn validate_sentinels(
    memory: &impl Memory,
    base: usize,
    sites: &[Site],
    name: &str,
) -> Result<(), String> {
    for site in sites {
        if engine::read::<[u8; 16]>(memory, base + site.rva) != Some(site.sentinel) {
            return Err(format!("{name} {} sentinel differs", site.name));
        }
    }
    Ok(())
}

pub struct Hooks<const N: usize> {
    hooks: Vec<Hook>,
    sites: [Site; N],
    domain: Domain<N>,
}
impl<const N: usize> Hooks<N> {
    /// All sentinels precede creation. Site order is enable order; disable and
    /// rollback run in reverse, so a paired observer never loses its context.
    pub unsafe fn new(
        memory: &impl Memory,
        base: usize,
        sites: [Site; N],
        callbacks: [usize; N],
        domain: Domain<N>,
        ready: Result<(), String>,
    ) -> Self {
        let install = || -> Result<Vec<Hook>, String> {
            validate_sentinels(memory, base, &sites, domain.name)?;
            ready?;
            domain.base.store(base, Ordering::Relaxed);
            let mut hooks = Vec::new();
            for (index, site) in sites.iter().enumerate() {
                let hook = unsafe { Hook::new(base + site.rva, callbacks[index], site.sentinel) }
                    .map_err(|reason| match domain.failure {
                    Failure::Propagate => reason,
                    Failure::Unavailable => format!(
                        "{} {} detour creation failed: {reason}",
                        domain.name, site.name
                    ),
                })?;
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
        Self {
            hooks,
            sites,
            domain,
        }
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        let domain = &self.domain;
        if !enabled {
            domain.observation.enabled.store(false, Ordering::SeqCst);
        }
        domain.observation.invalidate();
        if enabled
            && matches!(domain.failure, Failure::Unavailable)
            && domain.unhooked.get().is_some()
        {
            return Ok(true);
        }
        let mut order: Vec<_> = self.hooks.iter_mut().enumerate().collect();
        if !enabled {
            order.reverse();
        }
        let mut failure = None;
        for (index, hook) in order {
            match hook.set_enabled(enabled) {
                Ok(true) => {}
                Ok(false) => return Ok(false),
                Err(reason) => match domain.failure {
                    Failure::Propagate => return Err(reason),
                    Failure::Unavailable if enabled => {
                        failure = Some(format!("{}: {reason}", self.sites[index].name));
                        break;
                    }
                    Failure::Unavailable => {
                        return Err(format!(
                            "{} {} detour disable failed: {reason}",
                            domain.name, self.sites[index].name
                        ));
                    }
                },
            }
        }
        if let Some(reason) = failure {
            domain.observation.enabled.store(false, Ordering::SeqCst);
            let _ = domain
                .unhooked
                .set(format!("{} detour enable failed: {reason}", domain.name));
            // Continue attaching only after all partial patches are restored.
            for hook in self.hooks.iter_mut().rev() {
                if !hook.set_enabled(false)? {
                    return Err(format!("{} detour rollback is still in use", domain.name));
                }
            }
            if let Some(installed) = domain.installed {
                for value in installed {
                    value.store(false, Ordering::Relaxed);
                }
            }
            return Ok(true);
        }
        domain
            .observation
            .enabled
            .store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        Ok(true)
    }
    pub fn executable_ranges(&self) -> Vec<Range<usize>> {
        self.hooks
            .iter()
            .flat_map(Hook::executable_ranges)
            .collect()
    }
}

pub fn status(
    observation: &Log,
    unhooked: &OnceLock<String>,
    installed: Option<&[AtomicBool]>,
) -> serde_json::Value {
    let mut status = serde_json::json!({ "unhooked": unhooked.get(), "enabled": observation.enabled.load(Ordering::SeqCst) });
    if let Some(installed) = installed {
        status["installed"] = serde_json::json!(
            installed
                .iter()
                .map(|value| value.load(Ordering::Relaxed))
                .collect::<Vec<_>>()
        );
    }
    status
}
