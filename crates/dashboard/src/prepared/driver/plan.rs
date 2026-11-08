//! The backend preparing each family, checked against the registry once, and everything that
//! follows from it: what each product waits for and refuses with, which publisher writes a
//! section, and the version stamp a family's publications are prepared under. Each family is
//! prepared by its native backend ([`super::native`]) or refused by name.

use super::registry::{self, Descriptor, FamilyId};
use std::{
    collections::{HashMap, HashSet},
    io,
    sync::{Arc, LazyLock},
};

/// What prepares a family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Backend {
    /// The segment driver's operators and emit over a native recording's segments
    /// ([`super::native`]).
    Segments,
    /// Refused by name on every branch: no native backend prepares it.
    Unavailable,
}

/// Who publishes a section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Owner {
    Family(FamilyId),
    Job(&'static str),
    /// The commit coordinator's bindings and metadata.
    Coordinator,
}

pub(crate) struct Plan {
    /// In registry order: the order a query shared by several answers their
    /// refusals in.
    families: Vec<&'static Descriptor>,
    backends: HashMap<&'static str, Backend>,
    gates: HashMap<&'static str, &'static str>,
    stems: HashMap<&'static str, Owner>,
    /// The recording-wide jobs a native recording's run prepares among the families.
    jobs: Vec<&'static str>,
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

/// Refuses `families` naming a publisher twice, or referring to a family they lack.
fn checked(families: &[&'static Descriptor]) -> io::Result<()> {
    let mut spellings = HashSet::new();
    let names = families
        .iter()
        .copied()
        .flat_map(|family| std::iter::once(family.id.name()).chain(family.aliases.iter().copied()))
        .chain(registry::JOBS.iter().map(|job| job.name));
    for name in names {
        if !spellings.insert(name) {
            return Err(invalid(format!("{name:?} names two prepared publishers")));
        }
    }
    let references = families
        .iter()
        .flat_map(|family| family.observers)
        .chain(registry::QUERIES.iter().flat_map(|query| query.suppliers));
    for id in references {
        if !families.iter().any(|family| family.id == *id) {
            return Err(invalid(format!("Unknown prepared family {:?}", id.name())));
        }
    }
    Ok(())
}

impl Plan {
    /// The plan of `families`: each prepared by its native backend when `native` names it, and
    /// refused by name on every branch otherwise. `native` may also name a recording-wide job,
    /// which the native run prepares among the families.
    pub(super) fn new(families: Vec<&'static Descriptor>, native: &[&str]) -> io::Result<Self> {
        checked(&families)?;
        for (at, &name) in native.iter().enumerate() {
            let known = registry::job(name).is_some()
                || families.iter().any(|family| family.id.name() == name);
            if !known {
                return Err(invalid(format!(
                    "A native backend prepares the unknown family {name:?}"
                )));
            }
            if native[..at].contains(&name) {
                return Err(invalid(format!("{name:?} has two native backends")));
            }
        }
        let backends = families
            .iter()
            .map(|family| {
                let backend = if native.contains(&family.id.name()) {
                    Backend::Segments
                } else {
                    Backend::Unavailable
                };
                (family.id.name(), backend)
            })
            .collect();
        let jobs = registry::JOBS
            .iter()
            .filter(|job| native.contains(&job.name))
            .map(|job| job.name)
            .collect();
        let mut gates = HashMap::new();
        let gating = registry::JOBS
            .iter()
            .flat_map(|job| job.gates.iter().map(move |&gate| (gate, job.name)))
            .chain(families.iter().flat_map(|family| {
                family
                    .gates
                    .iter()
                    .map(move |&gate| (gate, family.id.name()))
            }));
        for (gate, publisher) in gating {
            if let Some(other) = gates.insert(gate, publisher) {
                return Err(invalid(format!(
                    "{other:?} and {publisher:?} both gate {gate:?}"
                )));
            }
        }
        let mut stems = HashMap::new();
        let owned = families
            .iter()
            .copied()
            .flat_map(|family| {
                family
                    .sections
                    .iter()
                    .map(move |&stem| (stem, Owner::Family(family.id)))
            })
            .chain(registry::JOBS.iter().flat_map(|job| {
                job.sections
                    .iter()
                    .map(move |&stem| (stem, Owner::Job(job.name)))
            }));
        for (stem, owner) in owned {
            if let Some(other) = stems.insert(stem, owner) {
                return Err(invalid(format!(
                    "{other:?} and {owner:?} both own the sections {stem:?}"
                )));
            }
        }
        Ok(Self {
            families,
            backends,
            gates,
            stems,
            jobs,
        })
    }

    pub(crate) fn families(&self) -> &[&'static Descriptor] {
        &self.families
    }

    pub(crate) fn family(&self, name: &str) -> Option<&'static Descriptor> {
        self.families
            .iter()
            .copied()
            .find(|family| family.id.name() == name)
    }

    pub(crate) fn backend(&self, name: &str) -> Option<Backend> {
        self.backends.get(name).copied()
    }

    /// The recording-wide jobs a native recording's run prepares among the families.
    pub(crate) fn jobs(&self) -> &[&'static str] {
        &self.jobs
    }

    /// The family or job whose coverage `product`'s queries wait for, and whose refusals they
    /// answer with.
    pub(crate) fn gating(&self, product: &str) -> Option<&'static str> {
        self.gates.get(product).copied()
    }

    /// The families whose coverage `product`'s queries wait for together.
    pub(crate) fn joint(&self, product: &str) -> Option<&'static [FamilyId]> {
        registry::QUERIES
            .iter()
            .find(|query| query.product == product)
            .map(|query| query.suppliers)
    }

    /// `name`'s version stamp: its own version, and those of the families whose same-hour state
    /// it observes, through theirs, under the layout every family shares. A store keeps a
    /// family's publications only under its stamp; None for a name the plan lacks. A
    /// recording-wide job observes none: its stamp is its own version's.
    pub(crate) fn stamp(&self, name: &str) -> Option<u64> {
        let mut hash = blake3::Hasher::new();
        hash.update(&crate::prepared::coverage::SCHEMA.to_le_bytes());
        hash.update(&dashboard_text::catalogue::CONTENT_DIAGNOSTICS_FORMAT.to_le_bytes());
        hash.update(&dashboard_base::prepared::roots::STATE_FORMAT.to_le_bytes());
        let Some(family) = self.family(name) else {
            let job = registry::job(name)?;
            hash.update(job.name.as_bytes());
            hash.update(&[0]);
            hash.update(&job.version.to_le_bytes());
            return Some(digest(&hash));
        };
        let mut seen = vec![family.id];
        let mut at = 0;
        while let Some(&id) = seen.get(at) {
            at += 1;
            let family = self.family(id.name())?;
            hash.update(id.name().as_bytes());
            hash.update(&[0]);
            hash.update(&family.version.to_le_bytes());
            for supplier in &self.families {
                if supplier.observers.contains(&id) && !seen.contains(&supplier.id) {
                    seen.push(supplier.id);
                }
            }
        }
        Some(digest(&hash))
    }

    /// Who publishes `section`: by its structured name, a private fact
    /// section by its column's namespace, any other by its own; None for a
    /// name no publisher registers.
    pub(crate) fn owner(&self, section: &str) -> Option<Owner> {
        if registry::COORDINATOR.contains(&section) {
            return Some(Owner::Coordinator);
        }
        if let Some(name) = registry::coverage_owner(section) {
            return self.publisher(name);
        }
        if let Some((name, _)) = dashboard_base::prepared::roots::owner(section) {
            return self.publisher(name);
        }
        if let Some(column) = dashboard_base::prepared::facts::column(section) {
            return self.namespace(column);
        }
        if let Some(family) =
            registry::continuation_owner(section).and_then(|name| self.family(name))
        {
            return Some(Owner::Family(family.id));
        }
        self.namespace(section)
    }

    /// The publisher of the longest registered namespace `name` equals or
    /// extends at a `-`, `/` or `.`.
    fn namespace(&self, name: &str) -> Option<Owner> {
        let mut stem = name;
        loop {
            if let Some(&owner) = self.stems.get(stem) {
                return Some(owner);
            }
            stem = &stem[..stem.rfind(|separator: char| matches!(separator, '-' | '/' | '.'))?];
        }
    }

    fn publisher(&self, name: &str) -> Option<Owner> {
        self.family(name)
            .map(|family| Owner::Family(family.id))
            .or_else(|| registry::job(name).map(|job| Owner::Job(job.name)))
    }

    /// Whether the publisher `name` may write a section `owner` publishes.
    pub(crate) fn writes(&self, name: &str, owner: Owner) -> bool {
        match owner {
            Owner::Coordinator => false,
            Owner::Family(family) => family.name() == name,
            Owner::Job(job) => job == name,
        }
    }

    /// Refuses every write of the publisher `name` to a section another publisher owns or no
    /// publisher registers.
    pub(crate) fn guard(&'static self, name: &'static str) -> store::SectionGuard {
        Arc::new(move |section: &str| match self.owner(section) {
            Some(owner) if self.writes(name, owner) => Ok(()),
            owner => Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                match owner {
                    Some(owner) => {
                        format!("{name} may not write {section}, which {owner:?} publishes")
                    }
                    None => {
                        format!("{name} may not write {section}, which no publisher registers")
                    }
                },
            )),
        })
    }
}

fn digest(hash: &blake3::Hasher) -> u64 {
    let mut bytes = [0; 8];
    bytes.copy_from_slice(&hash.finalize().as_bytes()[..8]);
    u64::from_le_bytes(bytes)
}

static NATIVE: LazyLock<Plan> = LazyLock::new(|| {
    Plan::new(registry::FAMILIES.iter().collect(), &super::native::names())
        .expect("the family registry and its native backends agree")
});

/// The plan preparation and queries follow: the registry's, unless a test selects another on
/// this thread.
pub(crate) fn native() -> &'static Plan {
    #[cfg(test)]
    {
        if let Some(plan) = TEST_NATIVE.with(std::cell::Cell::get) {
            return plan;
        }
    }
    &NATIVE
}

#[cfg(test)]
thread_local! {
    static TEST_NATIVE: std::cell::Cell<Option<&'static Plan>> =
        const { std::cell::Cell::new(None) };
}

/// A plan of the registry's families with `added` standing in for those of their names and
/// added after the others, those `native` names prepared by their native backends and every
/// other refused by name.
#[cfg(test)]
pub(crate) fn test_native(
    added: &[&'static Descriptor],
    native: &[&'static str],
) -> io::Result<&'static Plan> {
    let replaced = |family: &'static Descriptor| {
        added
            .iter()
            .copied()
            .find(|added| added.id == family.id)
            .unwrap_or(family)
    };
    let families = registry::FAMILIES
        .iter()
        .map(replaced)
        .chain(
            added
                .iter()
                .copied()
                .filter(|added| registry::family_named(added.id.name()).is_none()),
        )
        .collect();
    Ok(Box::leak(Box::new(Plan::new(families, native)?)))
}

/// Run `run` with `plan` as the plan on this thread.
#[cfg(test)]
pub(crate) fn with_test_native<T>(plan: &'static Plan, run: impl FnOnce() -> T) -> T {
    crate::prepared::with_test_cell(&TEST_NATIVE, Some(plan), run)
}
