//! Production line scans kept between sessions. Each branch's whole production
//! scan of the recording's player country, and of every country a request read
//! whole and queued for keeping, is kept under sections of its own
//! (`production_lines::write_saved`), one commit per scan: beside the families
//! ([`independent::Beside`]) in their group commits, by the source's
//! preparation alone: through its writer, admission, game guard, cancellation
//! and Clear. A scan a request reads whole after the source's preparation has
//! finished waits for the source's next preparation in this session, or is
//! scanned again later. A scan kept for the branch's current last block is not
//! read again, and one that cannot be read is not tried again this session. The
//! production products read a kept scan where it is current, and never write
//! one.

use super::independent;
use crate::{
    Source,
    metadata::check_cancel,
    products::{LoadContext, production_lines},
};
use std::{collections::BTreeSet, io, sync::atomic::AtomicBool};

/// Countries whose scan could not be read this session: not tried again.
#[derive(Default)]
struct Unkeepable(BTreeSet<(u64, String)>);

/// The countries whose scans a preparation keeps now: by branch, the player's
/// and every one a request read whole, without a scan kept for the branch's
/// current last block, and none that could not be read this session.
pub(super) fn missing(
    source: &Source,
    store: &store::Store,
    key: &store::SourceKey,
    context: &LoadContext,
    queued: &BTreeSet<(u64, String)>,
) -> io::Result<Vec<(u64, String)>> {
    let mut wanted = wanted(source);
    wanted.extend(queued.iter().cloned());
    let unkeepable = source
        .state::<Unkeepable>()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .0
        .clone();
    let mut missing = Vec::new();
    for (branch, tag) in wanted {
        if !unkeepable.contains(&(branch, tag.clone()))
            && !production_lines::saved_current(store, key, source, context, branch, &tag)?
        {
            missing.push((branch, tag));
        }
    }
    Ok(missing)
}

pub(super) fn wanted(source: &Source) -> BTreeSet<(u64, String)> {
    let metadata = source.metadata();
    let player = metadata
        .player_country_id
        .as_ref()
        .and_then(|id| metadata.countries.iter().find(|country| &country.id == id))
        .map(|country| country.tag.clone());
    let mut wanted: BTreeSet<(u64, String)> = metadata
        .branches
        .iter()
        .filter_map(|branch| branch.id.parse::<u64>().ok())
        .flat_map(|branch| player.clone().map(|tag| (branch, tag)))
        .collect();
    wanted.extend(production_lines::read_whole(source));
    wanted
}

/// A scan to keep: its branch and the tag it is for.
type WantedScan = (u64, String);

/// Keeps each of `wanted` through `commit`, then the scans requests read whole
/// meanwhile, one at a time (a scan a request holds is not read again). A scan
/// that cannot be read is not kept, nor tried again this session: its products
/// scan at request time.
// Keep the borrowed registration callback and its publication error contract explicit.
#[allow(clippy::type_complexity)]
fn keep_each(
    source: &Source,
    context: &LoadContext,
    cancelled: &AtomicBool,
    mut wanted: Vec<WantedScan>,
    register: &mut dyn FnMut(&[WantedScan]) -> io::Result<()>,
    commit: &mut dyn FnMut(&production_lines::SavedScan) -> io::Result<()>,
) -> io::Result<()> {
    let mut tried: BTreeSet<WantedScan> = BTreeSet::new();
    loop {
        for (branch, tag) in std::mem::take(&mut wanted) {
            check_cancel(cancelled)?;
            tried.insert((branch, tag.clone()));
            match production_lines::saved_scan(source, branch, &tag, context, cancelled) {
                Ok(saved) => commit(&saved)?,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
                Err(_) => {
                    source
                        .state::<Unkeepable>()
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .0
                        .insert((branch, tag.clone()));
                }
            }
            production_lines::kept(source, branch, &tag);
        }
        wanted = production_lines::read_whole(source)
            .into_iter()
            .filter(|read| !tried.contains(read))
            .collect();
        if wanted.is_empty() {
            return Ok(());
        }
        register(&wanted)?;
    }
}

/// The production scans to keep, as work beside the families: one thread
/// reading each in turn, each published in a commit of its own as soon as it is
/// read ([`keep_each`]).
pub(super) fn jobs<'a>(
    phase: &'a independent::Phase<'a>,
    queued: &BTreeSet<(u64, String)>,
) -> io::Result<Vec<independent::Beside<'a>>> {
    let wanted = missing(phase.source, phase.store, phase.key, phase.context, queued)?;
    if wanted.is_empty() {
        return Ok(Vec::new());
    }
    #[cfg(test)]
    let stall = TEST_STALL.with(|stall| stall.borrow().clone());
    Ok(vec![independent::Beside {
        name: "production lines",
        first_view: false,
        run: Box::new(
            move |publish: &mut independent::Publish<'_>, _timings: &mut independent::Timings| {
                #[cfg(test)]
                if let Some(stall) = stall {
                    stall();
                }
                let publish = std::cell::RefCell::new(publish);
                keep_each(
                    phase.source,
                    phase.context,
                    phase.cancelled,
                    wanted,
                    &mut |wanted| publish.borrow_mut().requested(wanted),
                    &mut |saved| {
                        publish
                            .borrow_mut()
                            .commit(&mut |writer| production_lines::write_saved(writer, saved))
                    },
                )
            },
        ),
    }])
}

/// Hold production side work independently of the forward publishers.
#[cfg(test)]
type Stall = Option<std::sync::Arc<dyn Fn() + Send + Sync>>;

#[cfg(test)]
thread_local! {
    static TEST_STALL: std::cell::RefCell<Stall> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(super) fn with_test_stall<T>(
    stall: impl Fn() + Send + Sync + 'static,
    run: impl FnOnce() -> T,
) -> T {
    struct Reset(Stall);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_STALL.with(|value| *value.borrow_mut() = self.0.take());
        }
    }
    let _reset = Reset(TEST_STALL.with(|value| value.replace(Some(std::sync::Arc::new(stall)))));
    run()
}
