//! What a preparation accepted, and the gates its queries read: the coverage a product waits
//! for, and the holes and refusals of the families and jobs supplying it, from their pages as
//! the publication a query pinned holds them.

use super::driver::{plan, registry};
use super::{Section, State};
use crate::{Source, metadata::check_cancel, products::ContentOptions};
use dashboard_base::prepared::{
    coverage::{Refusal, WALK},
    gates,
    roots::{Frontier, Held},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::AtomicBool,
};

pub use super::coverage::Coverage;
pub(crate) use super::coverage::SCHEMA;
pub(crate) use dashboard_base::prepared::coverage::Page as CoveragePage;

/// What a preparation accepted.
#[derive(Clone, Debug)]
pub struct Accepted {
    pub sections: Vec<Section>,
    pub bytes: u64,
    /// The hours of each branch every family published alike: the controls' timeline.
    pub coverage: BTreeMap<String, Coverage>,
    /// Each family's and job's coverage, by branch.
    pub families: BTreeMap<String, BTreeMap<String, Coverage>>,
}

pub fn sections(state: State, done: usize, total: Option<usize>) -> Vec<Section> {
    registry::STATUS
        .iter()
        .map(|&(id, label)| Section {
            id: id.into(),
            label: label.into(),
            state,
            done: done.to_string(),
            total: total.map(|value| value.to_string()),
            error: None,
        })
        .collect()
}

/// A family's coverage page on `branch` as its publication `frontier` leaves
/// it: the hours it processed, complete once nothing more of the branch is
/// the family's to prepare, with its refusal of the branch.
pub(crate) fn write_family_page(
    writer: &mut store::Writer,
    family: &str,
    branch: u64,
    frontier: &Frontier,
) -> io::Result<()> {
    let mut refused = BTreeMap::new();
    if let Some(end) = &frontier.end
        && end.held == Held::Refused
    {
        refused.insert(
            family.to_owned(),
            Refusal {
                hour: end.hour,
                error: end.reason.clone(),
                from: end.from.clone(),
            },
        );
    }
    let page = CoveragePage {
        coverage: Coverage {
            schema: SCHEMA,
            first_hour: frontier.first_hour,
            last_hour: frontier.last_hour,
            complete: frontier.terminal(),
        },
        refused,
    };
    writer.write_at(
        &registry::coverage_section(family, branch),
        0,
        serde_json::to_vec(&page)?.as_slice(),
    )
}

/// The gates of `source`'s store the query this thread runs pinned, else its
/// current ones; None while none is decoded, when every gate is pending.
fn current_gates(
    source: &Source,
    content: Option<&ContentOptions>,
) -> io::Result<Option<std::sync::Arc<gates::Published>>> {
    let (store, key) = super::files(source, content)?;
    Ok(gates::current(&store, &key))
}

/// Whether `source`'s store, as the query this thread runs pinned it, holds a preparation's
/// publication.
pub(crate) fn native_published(
    source: &Source,
    content: Option<&ContentOptions>,
) -> io::Result<bool> {
    Ok(current_gates(source, content)?.is_some_and(|published| published.plan().is_some()))
}

/// Whether `gate`, `name`'s on its branch, was published under the stamp the plan gives `name`
/// now: by its root's identity; a page published with no root (a refusal) has no stamp.
fn stamped(gate: &gates::Gate, name: &str) -> io::Result<bool> {
    Ok(match gate.publication()? {
        Some(root) => Some(root.identity.schema) == plan::native().stamp(name),
        None => true,
    })
}

/// The coverage page the family or job `name` published for `branch`, as `published` holds
/// it, while it is of the version the plan gives `name`: a page of another version is pending
/// until its family is prepared again.
fn own_published<'p>(
    published: &'p gates::Published,
    name: &str,
    branch: u64,
) -> io::Result<Option<&'p CoveragePage>> {
    let Some(gate) = published.gate(branch, name) else {
        return Ok(None);
    };
    let Some(page) = gate.page()? else {
        return Ok(None);
    };
    Ok(stamped(gate, name)?.then_some(page))
}

/// The coverage page the family or job gating `query` published for its branch.
fn family_page<'p>(
    published: &'p gates::Published,
    branch: u64,
    query: &str,
) -> io::Result<Option<(&'static str, &'p CoveragePage)>> {
    let Some(family) = plan::native().gating(query) else {
        return Ok(None);
    };
    Ok(own_published(published, family, branch)?.map(|page| (family, page)))
}

/// The coverage every one of `families` has on `branch` ([`plan::Plan::joint`]): through the
/// least of their last hours, complete when every one is. None until every one has accepted
/// some.
fn joint_coverage(
    published: &gates::Published,
    branch: u64,
    families: &[registry::FamilyId],
) -> io::Result<Option<Coverage>> {
    let mut joint: Option<Coverage> = None;
    for (position, family) in families.iter().map(|family| family.name()).enumerate() {
        let extent = own_short(
            family,
            own_published(published, family, branch)?.map(|page| page.coverage.clone()),
        );
        let Some(extent) = extent else {
            // The families after it wait too, as far as this reply knows.
            for family in &families[position + 1..] {
                super::coverage::note_short(family.name());
            }
            return Ok(None);
        };
        joint = Some(match joint {
            None => extent,
            Some(held) => Coverage {
                schema: SCHEMA,
                first_hour: held.first_hour.max(extent.first_hour),
                last_hour: held.last_hour.min(extent.last_hour),
                complete: held.complete && extent.complete,
            },
        });
    }
    Ok(joint)
}

/// The coverage `product`'s queries wait for on `branch`: that of the families whose pages its
/// replies read together ([`plan::Plan::joint`]), else that of the family or job gating it
/// ([`plan::Plan::gating`]); None before any is accepted, and for a product nothing supplies.
/// Read from the gates the query pinned, for the hours it declared
/// ([`super::coverage::product_coverage`]).
pub(crate) fn product_coverage(
    source: &Source,
    content: Option<&ContentOptions>,
    branch: u64,
    product: &str,
    cancelled: &AtomicBool,
) -> io::Result<Option<Coverage>> {
    install_gates();
    super::coverage::product_coverage(source, content, branch, product, cancelled)
}

/// [`product_coverage`] as `published` holds it.
pub(crate) fn published_coverage(
    published: &gates::Published,
    branch: u64,
    product: &str,
) -> io::Result<Option<Coverage>> {
    let plan = plan::native();
    if let Some(families) = plan.joint(product) {
        return joint_coverage(published, branch, families);
    }
    let Some(gating) = plan.gating(product) else {
        super::coverage::note_short(WALK);
        return Ok(None);
    };
    Ok(match family_page(published, branch, product)? {
        Some((family, page)) => own_short(family, Some(page.coverage.clone())),
        None => {
            super::coverage::note_short(gating);
            None
        }
    })
}

/// The hours of `product`'s suppliers on `branch` through `through` (the
/// branch's end when None) that their publications will not answer, in hour
/// order: each supplier's first hole, from which no reply needing earlier
/// observations answers, and every hour after its branch ended for it.
pub(crate) fn published_holes(
    published: &gates::Published,
    branch: u64,
    product: &str,
    through: Option<i32>,
) -> io::Result<Vec<super::coverage::Hole>> {
    let mut holes = Vec::new();
    for family in suppliers(product) {
        let Some(gate) = published.gate(branch, family) else {
            continue;
        };
        let Some(root) = gate.publication()? else {
            continue;
        };
        // A publication of another version is pending as a whole ([`stamped`]).
        if !stamped(gate, family)? {
            continue;
        }
        let unanswered =
            dashboard_base::prepared::roots::holes(root, i32::MIN, through.unwrap_or(i32::MAX));
        holes.extend(
            unanswered
                .iter()
                .filter_map(|interval| super::coverage::Hole::of(family, interval)),
        );
    }
    holes.sort_by(|a, b| {
        (a.first_hour, a.last_hour, &a.family).cmp(&(b.first_hour, b.last_hour, &b.family))
    });
    Ok(holes)
}

/// Every family and job whose pages `product`'s replies read, in name order: those it waits
/// for, and those listing it among their queries.
fn suppliers(product: &str) -> BTreeSet<&'static str> {
    let plan = plan::native();
    let mut suppliers: BTreeSet<&'static str> = gate(product)
        .into_iter()
        .filter(|&entry| entry != WALK)
        .collect();
    suppliers.extend(
        plan.families()
            .iter()
            .filter(|family| family.queries.contains(&product))
            .map(|family| family.id.name()),
    );
    suppliers
}

/// What `product`'s queries read on `branch` through `through`, as
/// `published` holds it: the coverage they wait for, ending before the first
/// hole there, and the holes.
pub(crate) fn gated(
    published: &gates::Published,
    branch: u64,
    product: &str,
    through: Option<i32>,
) -> io::Result<super::coverage::Ranges> {
    let holes = published_holes(published, branch, product, through)?;
    let mut coverage = published_coverage(published, branch, product)?;
    if let (Some(coverage), Some(first)) = (coverage.as_mut(), holes.first()) {
        coverage.last_hour = coverage
            .last_hour
            .min(first.first_hour.saturating_sub(1))
            .max(coverage.first_hour.saturating_sub(1));
        coverage.complete = false;
    }
    Ok(super::coverage::Ranges { coverage, holes })
}

/// `extent`, the coverage of `family`, noted as short for the query collecting what it waits
/// for when absent or incomplete ([`super::coverage::waiting`]).
fn own_short(family: &'static str, extent: Option<Coverage>) -> Option<Coverage> {
    if !extent.as_ref().is_some_and(|extent| extent.complete) {
        super::coverage::note_short(family);
    }
    extent
}

/// The status entries `product`'s replies wait for, as [`product_coverage`] chooses its
/// coverage: each family of a joint product, or the family or job gating it. A product nothing
/// supplies waits for the preparation's shared coverage ([`WALK`]). A pending reply that
/// consulted no coverage names these.
pub(crate) fn gate(product: &str) -> Vec<&'static str> {
    let plan = plan::native();
    let mut entries: Vec<_> = match (plan.joint(product), plan.gating(product)) {
        (Some(families), _) => families.iter().map(|family| family.name()).collect(),
        (None, Some(family)) => vec![family],
        (None, None) => vec![WALK],
    };
    entries.sort_unstable();
    entries.dedup();
    entries
}

/// Publication identities of every declared supplier of a query: its gates
/// and the families whose refusals it reads, as the pinned gates hold them.
/// A replacement, including a refusal or Clear followed by preparation,
/// changes its identity.
pub(crate) fn query_pages(
    published: &gates::Published,
    branch: u64,
    product: &str,
) -> io::Result<Vec<(String, Option<[u64; 3]>)>> {
    Ok(pages_of(published, branch, suppliers(product)))
}

/// [`query_pages`] of a query's own gates alone, without the families it reads through
/// another's supply.
pub(crate) fn gate_pages(
    published: &gates::Published,
    branch: u64,
    product: &str,
) -> io::Result<Vec<(String, Option<[u64; 3]>)>> {
    Ok(pages_of(
        published,
        branch,
        gate(product).into_iter().filter(|&entry| entry != WALK),
    ))
}

/// The publication identity of each of `entries` on `branch`.
fn pages_of(
    published: &gates::Published,
    branch: u64,
    entries: impl IntoIterator<Item = &'static str>,
) -> Vec<(String, Option<[u64; 3]>)> {
    entries
        .into_iter()
        .map(|name| {
            let id = published.gate(branch, name).and_then(gates::Gate::id);
            (registry::coverage_section(name, branch), id)
        })
        .collect()
}

/// The coverage page of every family and job on `branch`: every publication to the branch
/// replaces one of them.
pub(crate) fn published_pages(
    published: &gates::Published,
    branch: u64,
) -> Vec<(String, Option<[u64; 3]>)> {
    let names: BTreeSet<&'static str> = plan::native()
        .families()
        .iter()
        .map(|family| family.id.name())
        .chain(registry::JOBS.iter().map(|job| job.name))
        .collect();
    pages_of(published, branch, names)
}

/// The gates readers below the dashboard ask ([`super::coverage::Gates`]):
/// [`product_coverage`] and [`refusal`].
struct Gates;

impl super::coverage::Gates for Gates {
    fn refusal(
        &self,
        source: &Source,
        content: Option<&ContentOptions>,
        branch: u64,
        query: &str,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        refusal(source, content, branch, query, cancelled)
    }

    fn ranges(
        &self,
        source: &Source,
        content: Option<&ContentOptions>,
        branch: u64,
        product: &str,
        through: Option<i32>,
        cancelled: &AtomicBool,
    ) -> io::Result<super::coverage::Ranges> {
        check_cancel(cancelled)?;
        match current_gates(source, content)? {
            Some(published) => gated(&published, branch, product, through),
            None => {
                for entry in gate(product) {
                    super::coverage::note_short(entry);
                }
                Ok(super::coverage::Ranges::default())
            }
        }
    }
}

/// Installs the pipeline's [`Gates`] for readers below the dashboard; the
/// pipeline, the query route and the service install them before reading. The
/// right to declare queries is the query route's ([`super::query::query`]).
pub fn install_gates() -> &'static super::coverage::Entry {
    static GATES: Gates = Gates;
    static ENTRY: std::sync::OnceLock<super::coverage::Entry> = std::sync::OnceLock::new();
    let entry = ENTRY.get_or_init(|| {
        super::coverage::install_gates(&GATES).expect("the pipeline installs the process's gates")
    });
    dashboard_base::products::country_styles::install_style_runs(
        &crate::products::style_runs::PREPARED_STYLES,
    );
    entry
}

/// Every prepared query of a refused family on this branch, and every query reading one,
/// answers with that family's refusal exactly as its product route refused, rather than with
/// the family's earlier prepared data: its gated queries, and every query the registry lists as
/// reading it, refuse from the hour its page names.
pub(crate) fn refusal(
    source: &Source,
    content: Option<&ContentOptions>,
    branch: u64,
    query: &str,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    check_cancel(cancelled)?;
    let Some(published) = current_gates(source, content)? else {
        return Ok(());
    };
    let through = super::coverage::declared().through;
    match refusal_from(&published, branch, query)? {
        Some((hour, error)) if through.is_none_or(|through| through >= hour) => {
            Err(io::Error::new(io::ErrorKind::InvalidData, error))
        }
        _ => Ok(()),
    }
}

/// The refusal `query` answers on `branch` with when it asks a refused
/// family's hours ([`super::query::query`]), as `published` holds the
/// refusals: the earliest refusing hour of the families it reads, with the
/// error of the first refusing one, its gate's then the others' in registry
/// order.
pub(crate) fn refusal_from(
    published: &gates::Published,
    branch: u64,
    query: &str,
) -> io::Result<Option<(i32, String)>> {
    let plan = plan::native();
    let mut found: Option<(i32, String)> = None;
    let mut note = |refusal: &Refusal| {
        found = Some(match found.take() {
            Some((hour, error)) => (hour.min(refusal.hour), error),
            None => (refusal.hour, refusal.error.clone()),
        });
    };
    if let Some((family, page)) = family_page(published, branch, query)?
        && let Some(refusal) = page.refused.get(family)
    {
        note(refusal);
    }
    for family in plan
        .families()
        .iter()
        .filter(|family| family.queries.contains(&query))
        .map(|family| family.id.name())
    {
        if let Some(refusal) =
            own_published(published, family, branch)?.and_then(|page| page.refused.get(family))
        {
            note(refusal);
        }
    }
    Ok(found)
}

/// Restore completed preparation directly from input/index metadata. A cached
/// source does not wait for a background worker or reconstruct a historical frame.
pub fn restore(
    source: &Source,
    content: Option<&ContentOptions>,
    cancelled: &AtomicBool,
) -> io::Result<Option<Accepted>> {
    super::driver::restore(source, content, cancelled)
}

pub fn run(
    source: &Source,
    content: Option<&ContentOptions>,
    cancelled: &AtomicBool,
    accepted: impl FnMut(&Accepted),
) -> io::Result<Accepted> {
    super::driver::prepare(source, content, cancelled, accepted)
}
