//! The game content native backends read: each piece loaded once, by the first reader to ask,
//! on loader threads before the run starts. No worker of the run waits for content, so content
//! that is slow or fails cannot hold the run's pool; a piece that fails fails only the families
//! reading it, each by name.

use crate::{Source, config::LoadContext, products::style_runs};
use dashboard_equipment_stockpile::products::equipment_stockpile;
use dashboard_forces::products::supply_health;
use dashboard_politics_ledger::products::politics_ledger;
use dashboard_text::{catalogue::Catalogue, metadata::Extents};
use dashboard_unit_leaders::products::unit_leaders;
use std::{
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex, OnceLock, PoisonError},
};

/// A piece of content: loaded by the first reader to ask; every reader asking meanwhile waits
/// for that load, and every reader shares its value or its error.
pub(crate) struct Once<T> {
    cell: OnceLock<Result<T, Arc<io::Error>>>,
}

impl<T> Once<T> {
    pub(crate) const fn new() -> Self {
        Self {
            cell: OnceLock::new(),
        }
    }

    pub(crate) fn get(&self, load: impl FnOnce() -> io::Result<T>) -> io::Result<&T> {
        match self.cell.get_or_init(|| load().map_err(Arc::new)) {
            Ok(value) => Ok(value),
            Err(error) => Err(io::Error::new(error.kind(), error.to_string())),
        }
    }
}

/// The politics content the ledger accounts with, or why the recorded stack lacks it.
pub(crate) type Politics =
    Result<Option<Arc<dashboard_base::products::politics_content::PoliticsContent>>, String>;

/// Every piece of content the native backends read from the recording's content stack.
pub(crate) struct Content<'s> {
    pub(crate) source: &'s Source,
    pub(crate) context: &'s LoadContext,
    /// Each branch's hours, as the native recording records them.
    pub(crate) extents: Extents,
    leaders: Once<Arc<unit_leaders::Binding>>,
    supply: Once<supply_health::Configured>,
    styles: Once<style_runs::Configured>,
    equipment: Once<Option<Arc<Catalogue>>>,
    politics: Once<Politics>,
    stockpile: Once<equipment_stockpile::prepared::choices::Content>,
}

impl<'s> Content<'s> {
    pub(crate) fn new(source: &'s Source, context: &'s LoadContext, extents: Extents) -> Self {
        Self {
            source,
            context,
            extents,
            leaders: Once::new(),
            supply: Once::new(),
            styles: Once::new(),
            equipment: Once::new(),
            politics: Once::new(),
            stockpile: Once::new(),
        }
    }

    /// The unit leaders' traits, names and portraits, with what of them could not be read.
    pub(crate) fn leaders(&self) -> io::Result<&Arc<unit_leaders::Binding>> {
        self.leaders.get(|| {
            Ok(Arc::new(unit_leaders::Binding::load(
                self.source,
                self.context,
            )))
        })
    }

    /// Supply's defines, or why the content refused them, with its warnings.
    pub(crate) fn supply(&self) -> io::Result<&supply_health::Configured> {
        self.supply
            .get(|| supply_health::Configured::read(self.source, self.context.content.as_ref()))
    }

    /// The recorded mod stack's flag folders country styles choose flags in, or why they
    /// cannot be read.
    pub(crate) fn styles(&self) -> io::Result<&style_runs::Configured> {
        self.styles
            .get(|| style_runs::Configured::read(self.source, self.context.content.as_ref()))
    }

    /// The equipment catalogue division equipment names its variants from; none where the
    /// recorded stack gives none, which its replies refuse by name.
    pub(crate) fn equipment(&self) -> io::Result<&Option<Arc<Catalogue>>> {
        self.equipment.get(|| {
            Ok(dashboard_army::products::army_content::content(self.source, self.context).ok())
        })
    }

    pub(crate) fn politics(&self) -> io::Result<&Politics> {
        self.politics.get(|| {
            Ok(politics_ledger::prepared_content(
                self.source,
                self.context.content.as_ref(),
            ))
        })
    }

    /// The stockpile's equipment choices.
    pub(crate) fn stockpile(&self) -> io::Result<&equipment_stockpile::prepared::choices::Content> {
        self.stockpile.get(|| {
            Ok(equipment_stockpile::prepared::Prepared::content(
                self.source,
                self.context.content.as_ref(),
            ))
        })
    }
}

/// `loads` run on at most `threads` threads at once, each once, in any order: what each made,
/// in their order. A load that panics made the error its panic says.
pub(crate) fn load_all<T: Send>(
    threads: usize,
    loads: Vec<Box<dyn FnOnce() -> io::Result<T> + Send + '_>>,
) -> Vec<io::Result<T>> {
    let count = loads.len();
    let queue = Mutex::new(loads.into_iter().enumerate());
    let made: Mutex<Vec<Option<io::Result<T>>>> =
        Mutex::new(std::iter::repeat_with(|| None).take(count).collect());
    std::thread::scope(|scope| {
        for _ in 0..threads.clamp(1, count.max(1)) {
            scope.spawn(|| {
                loop {
                    let next = queue.lock().unwrap_or_else(PoisonError::into_inner).next();
                    let Some((at, load)) = next else {
                        break;
                    };
                    let loaded = catch_unwind(AssertUnwindSafe(load)).unwrap_or_else(|panic| {
                        let said = panic
                            .downcast_ref::<&str>()
                            .map(|said| (*said).to_owned())
                            .or_else(|| panic.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "Loading content panicked".to_owned());
                        Err(io::Error::other(said))
                    });
                    made.lock().unwrap_or_else(PoisonError::into_inner)[at] = Some(loaded);
                }
            });
        }
    });
    made.into_inner()
        .unwrap_or_else(PoisonError::into_inner)
        .into_iter()
        .map(|loaded| loaded.unwrap_or_else(|| Err(io::Error::other("A load did not run"))))
        .collect()
}
