//! The backend preparing each family, checked against the registry once, and
//! everything that follows from it: what the hourly walk observes, what each
//! product waits for and refuses with, which writer publishes a section, and
//! the version stamp a family's publications are prepared under. A row
//! recording's plan ([`rows`]) prepares families in the hourly walk or on their
//! own; a native recording's ([`native`]) prepares each by its native backend or
//! refuses it by name.

use super::registry::{self, Descriptor, FamilyId, Runner};
use replay_data::rows::TableId;
use std::{
    collections::{HashMap, HashSet},
    io,
    sync::{Arc, LazyLock},
};

/// What prepares a family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Backend {
    /// The hourly walk, hour by hour.
    Walk,
    /// A row implementation of its own beside the walk, with coverage pages
    /// of its own ([`crate::prepared::pipeline::independent`]).
    Own,
    /// The segment driver's operators and emit over a native recording's segments
    /// ([`super::native`]).
    Segments,
    /// Refused by name on every branch: a family no native backend prepares, of a native
    /// recording, or one only a native backend prepares, of a row recording.
    Unavailable,
}

/// Who publishes a section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Owner {
    Family(FamilyId),
    Job(&'static str),
    /// The hourly walk's own coverage and branch state.
    Walk,
    /// The commit coordinator's checkpoint, bindings and metadata.
    Coordinator,
}

/// A writer's publisher.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lane {
    Walk,
    /// A family prepared on its own, or a job beside the families, by name.
    Own(&'static str),
}

pub(crate) struct Plan {
    /// In registry order: the order a query shared by several answers their
    /// refusals in.
    families: Vec<&'static Descriptor>,
    backends: HashMap<&'static str, Backend>,
    /// The families prepared on their own, in lane order.
    own: Vec<FamilyId>,
    hourly: HashMap<&'static str, bool>,
    gates: HashMap<&'static str, &'static str>,
    stems: HashMap<&'static str, Owner>,
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

/// Refuses `families` naming a publisher twice, referring to a family they lack, or keeping
/// no private read they declare.
fn checked(families: &[&'static Descriptor]) -> io::Result<()> {
    let named = |name: &str| {
        families
            .iter()
            .copied()
            .find(|family| family.id.name() == name)
    };
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
        .copied()
        .flat_map(|family| {
            family
                .observers
                .iter()
                .chain(family.private.iter().flat_map(|read| read.readers))
        })
        .chain(registry::QUERIES.iter().flat_map(|query| query.suppliers));
    for id in references {
        if named(id.name()).is_none() {
            return Err(invalid(format!("Unknown prepared family {:?}", id.name())));
        }
    }
    for family in families {
        if family.private.is_some_and(|read| read.kept.is_empty()) {
            return Err(invalid(format!(
                "{:?} keeps no private read",
                family.id.name()
            )));
        }
    }
    Ok(())
}

impl Plan {
    /// `families`, with those `own` names prepared on their own in that
    /// order and every other the walk can prepare in the walk. `rows` names
    /// the families with a row implementation of their own.
    pub(super) fn new(
        families: Vec<&'static Descriptor>,
        own: &[&str],
        rows: &[&'static str],
    ) -> io::Result<Self> {
        let named = |name: &str| {
            families
                .iter()
                .copied()
                .find(|family| family.id.name() == name)
        };
        checked(&families)?;
        for &row in rows {
            if named(row).is_none() {
                return Err(invalid(format!(
                    "A row implementation prepares the unknown family {row:?}"
                )));
            }
        }
        let mut lanes = Vec::with_capacity(own.len());
        for &name in own {
            let family =
                named(name).ok_or_else(|| invalid(format!("Unknown prepared family {name:?}")))?;
            if lanes.contains(&family.id) {
                return Err(invalid(format!("{name:?} is selected twice")));
            }
            if !rows.contains(&family.id.name()) {
                return Err(invalid(format!(
                    "{name:?} has no row implementation of its own"
                )));
            }
            lanes.push(family.id);
        }
        let backends = families
            .iter()
            .filter_map(|family| {
                let backend = if lanes.contains(&family.id) {
                    Backend::Own
                } else if family.walk {
                    Backend::Walk
                } else if family.segments {
                    Backend::Unavailable
                } else {
                    return None;
                };
                Some((family.id.name(), backend))
            })
            .collect();
        Self::planned(families, lanes, backends)
    }

    /// A native recording's plan of `families`: each prepared on its own by its native backend
    /// when `native` names it, and refused by name on every branch otherwise.
    pub(super) fn native(families: Vec<&'static Descriptor>, native: &[&str]) -> io::Result<Self> {
        checked(&families)?;
        for (at, &name) in native.iter().enumerate() {
            if !families.iter().any(|family| family.id.name() == name) {
                return Err(invalid(format!(
                    "A native backend prepares the unknown family {name:?}"
                )));
            }
            if native[..at].contains(&name) {
                return Err(invalid(format!("{name:?} has two native backends")));
            }
        }
        let lanes = families.iter().map(|family| family.id).collect();
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
        Self::planned(families, lanes, backends)
    }

    /// `families` prepared as `backends` says, those of `lanes` on their own in that order.
    fn planned(
        families: Vec<&'static Descriptor>,
        lanes: Vec<FamilyId>,
        backends: HashMap<&'static str, Backend>,
    ) -> io::Result<Self> {
        let registered = |id: FamilyId| {
            backends
                .get(id.name())
                .is_some_and(|backend| *backend != Backend::Walk)
        };
        // A family prepared on its own stays in the walk, read under its
        // readers' names, until every reader makes its own read.
        let hourly = families
            .iter()
            .map(|family| {
                let walked = !registered(family.id)
                    || family.private.is_some_and(|read| {
                        read.readers.is_empty() || read.readers.iter().any(|&id| !registered(id))
                    });
                (family.id.name(), walked)
            })
            .collect();
        let mut gates = HashMap::new();
        let gating = registry::JOBS
            .iter()
            .filter(|job| job.runner == Runner::Beside)
            .flat_map(|job| job.gates.iter().map(move |&gate| (gate, job.name)))
            .chain(
                families
                    .iter()
                    .filter(|family| registered(family.id))
                    .flat_map(|family| {
                        family
                            .gates
                            .iter()
                            .map(move |&gate| (gate, family.id.name()))
                    }),
            );
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
            own: lanes,
            hourly,
            gates,
            stems,
        })
    }

    /// Ordinary preparation: every family with a row implementation of its
    /// own prepared on its own, every other in the hourly walk.
    fn rows() -> io::Result<Self> {
        let rows = crate::prepared::pipeline::independent::row_names();
        let plan = Self::new(registry::FAMILIES.iter().collect(), &rows, &rows)?;
        match plan
            .families
            .iter()
            .find(|family| !plan.backends.contains_key(family.id.name()))
        {
            Some(family) => Err(invalid(format!(
                "No backend prepares {:?}",
                family.id.name()
            ))),
            None => Ok(plan),
        }
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

    /// The families prepared on their own, in lane order.
    #[cfg(test)]
    pub(crate) fn own(&self) -> &[FamilyId] {
        &self.own
    }

    /// Whether `family` is prepared on its own.
    pub(crate) fn registered(&self, family: &str) -> bool {
        self.backend(family) == Some(Backend::Own)
    }

    /// Whether the walk leaves `family` to another backend: its own rows or the segment driver,
    /// or a native recording's plan refuses it on pages of its own.
    pub(crate) fn apart(&self, family: &str) -> bool {
        self.backend(family)
            .is_some_and(|backend| backend != Backend::Walk)
    }

    /// Whether the hourly walk observes, flushes, saves and loads `family`:
    /// it prepares it, or keeps it for a reader under that reader's names
    /// ([`Self::kept_as`]).
    pub(crate) fn hourly(&self, family: &str) -> bool {
        self.hourly.get(family).copied().unwrap_or(true)
    }

    /// The name the hourly walk keeps `section` under: a reader's own, while
    /// the family publishing it is prepared on its own; `section` otherwise.
    pub(crate) fn kept_as(&self, section: &'static str) -> &'static str {
        self.own
            .iter()
            .filter_map(|id| self.family(id.name())?.private)
            .flat_map(|read| read.kept)
            .find(|(published, _)| *published == section)
            .map_or(section, |&(_, kept)| kept)
    }

    /// The family prepared on its own, or job beside the families, whose
    /// coverage `product`'s queries wait for, and whose refusals they answer
    /// with.
    pub(crate) fn gating(&self, product: &str) -> Option<&'static str> {
        self.gates.get(product).copied()
    }

    /// The families whose coverage `product`'s queries wait for together,
    /// while any of them is prepared on its own.
    pub(crate) fn joint(&self, product: &str) -> Option<&'static [FamilyId]> {
        registry::QUERIES
            .iter()
            .find(|query| query.product == product)
            .map(|query| query.suppliers)
            .filter(|suppliers| suppliers.iter().any(|id| self.apart(id.name())))
    }

    /// `name`'s version stamp: its own version, and those of the families
    /// whose same-hour state it observes or reads privately, through theirs,
    /// under the layout every family shares. A store keeps a family's
    /// publications only under its stamp; None for a family the plan lacks.
    pub(crate) fn stamp(&self, name: &str) -> Option<u64> {
        let mut hash = blake3::Hasher::new();
        hash.update(&crate::prepared::coverage::SCHEMA.to_le_bytes());
        let mut seen = vec![self.family(name)?.id];
        let mut at = 0;
        while let Some(&id) = seen.get(at) {
            at += 1;
            let family = self.family(id.name())?;
            hash.update(id.name().as_bytes());
            hash.update(&[0]);
            hash.update(&family.version.to_le_bytes());
            for supplier in &self.families {
                let supplies = supplier.observers.contains(&id)
                    || supplier
                        .private
                        .is_some_and(|read| read.readers.contains(&id));
                if supplies && !seen.contains(&supplier.id) {
                    seen.push(supplier.id);
                }
            }
        }
        Some(digest(&hash))
    }

    /// The hourly walk's stamp: that of every family it prepares and the
    /// version of every job it runs. They share its cursor and branch state,
    /// so a change of any prepares them all again.
    pub(crate) fn walk_stamp(&self) -> u64 {
        let mut hash = blake3::Hasher::new();
        hash.update(&crate::prepared::coverage::SCHEMA.to_le_bytes());
        for family in &self.families {
            if self.backend(family.id.name()) == Some(Backend::Walk)
                && let Some(stamp) = self.stamp(family.id.name())
            {
                hash.update(family.id.name().as_bytes());
                hash.update(&stamp.to_le_bytes());
            }
        }
        for job in registry::JOBS
            .iter()
            .filter(|job| job.runner == Runner::Walk)
        {
            hash.update(job.name.as_bytes());
            hash.update(&job.version.to_le_bytes());
        }
        digest(&hash)
    }

    /// The families prepared on their own, in name order.
    pub(crate) fn names(&self) -> Vec<&'static str> {
        let mut names: Vec<_> = self.own.iter().map(|id| id.name()).collect();
        names.sort_unstable();
        names
    }

    /// Whether the walk observes a family ordinary preparation prepares on
    /// its own.
    pub(crate) fn walks_own(&self) -> bool {
        rows().own.iter().any(|id| self.hourly(id.name()))
    }

    /// The tables the hourly walk's stream decodes every hour: those of the
    /// families it prepares and of the jobs it runs.
    pub(crate) fn walk_tables(&self) -> Vec<TableId> {
        let reads: Vec<&[TableId]> = self
            .families
            .iter()
            .copied()
            .filter(|family| self.backend(family.id.name()) == Some(Backend::Walk))
            .flat_map(|family| family.walk_tables.iter().copied())
            .chain(
                registry::JOBS
                    .iter()
                    .filter(|job| job.runner == Runner::Walk)
                    .flat_map(|job| job.tables.iter().copied()),
            )
            .collect();
        TableId::ALL
            .into_iter()
            .filter(|table| reads.iter().any(|tables| tables.contains(table)))
            .collect()
    }

    /// Who publishes `section`: by its structured name, a private fact
    /// section by its column's namespace, any other by its own; None for a
    /// name no publisher registers.
    pub(crate) fn owner(&self, section: &str) -> Option<Owner> {
        if registry::COORDINATOR.contains(&section) {
            return Some(Owner::Coordinator);
        }
        if registry::walk_owned(section) {
            return Some(Owner::Walk);
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

    /// Whether `lane` may write a section `owner` publishes.
    pub(crate) fn writes(&self, lane: Lane, owner: Owner) -> bool {
        match (lane, owner) {
            (_, Owner::Coordinator) | (Lane::Own(_), Owner::Walk) => false,
            (Lane::Walk, Owner::Walk) => true,
            (Lane::Walk, Owner::Family(family)) => {
                self.backend(family.name()) == Some(Backend::Walk)
            }
            (Lane::Walk, Owner::Job(job)) => {
                registry::job(job).is_some_and(|job| job.runner == Runner::Walk)
            }
            (Lane::Own(name), Owner::Family(family)) => family.name() == name,
            (Lane::Own(name), Owner::Job(job)) => job == name,
        }
    }

    /// Refuses every write of `lane` to a section another publisher owns or
    /// no publisher registers.
    pub(crate) fn guard(&'static self, lane: Lane) -> store::SectionGuard {
        Arc::new(move |section: &str| match self.owner(section) {
            Some(owner) if self.writes(lane, owner) => Ok(()),
            owner => Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                match owner {
                    Some(owner) => {
                        format!("{lane:?} may not write {section}, which {owner:?} publishes")
                    }
                    None => {
                        format!("{lane:?} may not write {section}, which no publisher registers")
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

static ROWS: LazyLock<Plan> =
    LazyLock::new(|| Plan::rows().expect("the family registry and its backends agree"));

static NATIVE: LazyLock<Plan> = LazyLock::new(|| {
    Plan::native(registry::FAMILIES.iter().collect(), &super::native::names())
        .expect("the family registry and its native backends agree")
});

/// The plan of ordinary preparation.
pub(crate) fn rows() -> &'static Plan {
    &ROWS
}

/// The plan of a native recording's preparation, and of the queries of what it published:
/// the registry's, unless a test selects another on this thread.
pub(crate) fn native() -> &'static Plan {
    #[cfg(test)]
    {
        if let Some(plan) = TEST_NATIVE.with(std::cell::Cell::get) {
            return plan;
        }
    }
    &NATIVE
}

/// The plan preparation and queries follow: ordinary preparation's, unless a
/// test selects another on this thread.
pub(crate) fn current() -> &'static Plan {
    #[cfg(test)]
    {
        if TEST_HOURLY.with(std::cell::Cell::get) {
            return &HOURLY;
        }
        if let Some(plan) = TEST_PLAN.with(std::cell::Cell::get) {
            return plan;
        }
    }
    &ROWS
}

/// Every family the walk can prepare in the walk, none on its own.
#[cfg(test)]
static HOURLY: LazyLock<Plan> = LazyLock::new(|| {
    Plan::new(registry::FAMILIES.iter().collect(), &[], &[]).expect("an hourly plan")
});

#[cfg(test)]
thread_local! {
    static TEST_PLAN: std::cell::Cell<Option<&'static Plan>> = const { std::cell::Cell::new(None) };
    static TEST_HOURLY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static TEST_NATIVE: std::cell::Cell<Option<&'static Plan>> =
        const { std::cell::Cell::new(None) };
}

/// The plan tests selected on a thread, to install on another.
#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) struct TestSelection {
    plan: Option<&'static Plan>,
    hourly: bool,
}

#[cfg(test)]
pub(crate) fn test_selection() -> TestSelection {
    TestSelection {
        plan: TEST_PLAN.with(std::cell::Cell::get),
        hourly: TEST_HOURLY.with(std::cell::Cell::get),
    }
}

/// Select on this thread what `selection` selected on another.
#[cfg(test)]
pub(crate) fn install_test(selection: TestSelection) {
    TEST_PLAN.with(|plan| plan.set(selection.plan));
    TEST_HOURLY.with(|hourly| hourly.set(selection.hourly));
}

/// Whether a test prepares every family hour by hour on this thread, over
/// any plan it selected.
#[cfg(test)]
pub(crate) fn test_hourly() -> bool {
    TEST_HOURLY.with(std::cell::Cell::get)
}

/// Run `run` with every family the walk can prepare in the walk.
#[cfg(test)]
pub(crate) fn with_test_hourly<T>(run: impl FnOnce() -> T) -> T {
    crate::prepared::with_test_cell(&TEST_HOURLY, true, run)
}

/// A plan of the registry's families with `replaced` standing in for those
/// of their names and added after the others, and `own` (with row
/// implementations of their own) prepared on their own in that order.
#[cfg(test)]
pub(crate) fn test_plan(
    replaced: &[&'static Descriptor],
    own: &[&'static str],
) -> io::Result<&'static Plan> {
    let mut families: Vec<&'static Descriptor> = registry::FAMILIES
        .iter()
        .map(|family| {
            replaced
                .iter()
                .copied()
                .find(|replacement| replacement.id == family.id)
                .unwrap_or(family)
        })
        .collect();
    families.extend(
        replaced
            .iter()
            .copied()
            .filter(|replacement| registry::family_named(replacement.id.name()).is_none()),
    );
    Ok(Box::leak(Box::new(Plan::new(families, own, own)?)))
}

/// Run `run` with `plan` selected on this thread.
#[cfg(test)]
pub(crate) fn with_test_plan<T>(plan: &'static Plan, run: impl FnOnce() -> T) -> T {
    crate::prepared::with_test_cell(&TEST_PLAN, Some(plan), run)
}

/// A native plan of the registry's families and `added`, those `native` names prepared by
/// their native backends and every other refused by name.
#[cfg(test)]
pub(crate) fn test_native(
    added: &[&'static Descriptor],
    native: &[&'static str],
) -> io::Result<&'static Plan> {
    let families = registry::FAMILIES
        .iter()
        .chain(added.iter().copied())
        .collect();
    Ok(Box::leak(Box::new(Plan::native(families, native)?)))
}

/// Run `run` with `plan` as the native plan on this thread.
#[cfg(test)]
pub(crate) fn with_test_native<T>(plan: &'static Plan, run: impl FnOnce() -> T) -> T {
    crate::prepared::with_test_cell(&TEST_NATIVE, Some(plan), run)
}
