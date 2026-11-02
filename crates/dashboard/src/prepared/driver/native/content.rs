//! The game content native backends read: each piece loaded once, by the first reader to ask,
//! on loader threads before the run starts. No worker of the run waits for content, so content
//! that is slow or fails cannot hold the run's pool; a piece that fails fails only the families
//! reading it, each by name.

use crate::{
    Source,
    config::LoadContext,
    products::{intelligence_prepared, ships_sunk, style_runs},
};
use dashboard_base::products::{equipment_icon::IconContent, production_lines};
use dashboard_equipment_stockpile::products::equipment_stockpile;
use dashboard_forces::products::supply_health;
use dashboard_game_score::products::game_score;
use dashboard_occupation::products::occupied_states;
use dashboard_politics_ledger::products::politics_ledger;
use dashboard_special_projects::products::special_projects;
use dashboard_text::{catalogue::Catalogue, metadata::Extents};
use dashboard_unit_leaders::products::unit_leaders;
use std::{
    any::{Any, TypeId},
    collections::HashMap,
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex, OnceLock, PoisonError, atomic::AtomicBool},
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
            Err(error) => Err(match crate::refusal(error) {
                Some(text) => crate::refused(text),
                None => io::Error::new(error.kind(), error.to_string()),
            }),
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
    keyed: Mutex<HashMap<TypeId, Arc<Once<Arc<dyn Any + Send + Sync>>>>>,
    leaders: Once<Arc<unit_leaders::Binding>>,
    supply: Once<supply_health::Configured>,
    styles: Once<style_runs::Configured>,
    equipment: Once<Option<Arc<Catalogue>>>,
    politics: Once<Politics>,
    stockpile: Once<equipment_stockpile::emit::Content>,
    naval: Once<ships_sunk::PreparedContent>,
    territory: Once<crate::products::territory::Configured>,
    score: Once<game_score::ScoreContent>,
    air: Once<Option<Arc<installed_content_core::equipment::Database>>>,
    land: Once<Land>,
    factions: Once<Option<Arc<Catalogue>>>,
    intelligence: Once<intelligence_prepared::Content>,
    production_icons: Once<Option<Arc<IconContent>>>,
    projects: Once<special_projects::facts::Binding>,
    occupation: Once<occupied_states::prepared::Binding>,
    casualties: Once<crate::products::war_casualties::prepared::PreparedContent>,
    divisions: Once<crate::products::division_log::Names>,
    navy: Once<Option<Arc<installed_content_core::unit_leader::SubUnits>>>,
    raids: Once<Option<Arc<crate::products::raids::Content>>>,
    names: Once<Names>,
}

/// The localisation countries are named in, or why their names are their tags.
pub(crate) type Names = Result<Arc<installed_content_localisation::localisation::Localisation>, String>;

/// The catalogue land battles name equipment designs from, and why it is missing.
pub(crate) type Land = (Option<Arc<Catalogue>>, Option<String>);

impl<'s> Content<'s> {
    pub(crate) fn new(source: &'s Source, context: &'s LoadContext, extents: Extents) -> Self {
        Self {
            source,
            context,
            extents,
            keyed: Mutex::new(HashMap::new()),
            leaders: Once::new(),
            supply: Once::new(),
            styles: Once::new(),
            equipment: Once::new(),
            politics: Once::new(),
            stockpile: Once::new(),
            naval: Once::new(),
            territory: Once::new(),
            score: Once::new(),
            air: Once::new(),
            land: Once::new(),
            factions: Once::new(),
            intelligence: Once::new(),
            production_icons: Once::new(),
            projects: Once::new(),
            occupation: Once::new(),
            casualties: Once::new(),
            divisions: Once::new(),
            navy: Once::new(),
            raids: Once::new(),
            names: Once::new(),
        }
    }

    pub(crate) fn keyed<O: dashboard_keyed::Observer>(&self) -> io::Result<Arc<O::Content>> {
        let cell = self
            .keyed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(TypeId::of::<O>())
            .or_insert_with(|| Arc::new(Once::new()))
            .clone();
        let content = cell.get(|| {
            O::content(self.source, self.context.content.as_ref(), &mut || Ok(()))
                .map(|content| Arc::new(content) as Arc<dyn Any + Send + Sync>)
        })?;
        Arc::clone(content)
            .downcast::<O::Content>()
            .map_err(|_| io::Error::other("Keyed content type mismatch"))
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

    /// The naming content, defines' multipliers and sort hints game score reads.
    pub(crate) fn score(&self) -> io::Result<&game_score::ScoreContent> {
        self.score
            .get(|| Ok(game_score::ScoreContent::load(self.source, self.context)))
    }

    /// The equipment catalogue land battles name designs from, or why it is missing.
    pub(crate) fn land(&self) -> io::Result<&Land> {
        self.land.get(|| {
            Ok(crate::products::land_battles::battle_content(
                self.source,
                self.context,
            ))
        })
    }

    /// The catalogue faction states name rules, upgrades, programmes and goals by; none where
    /// the recording names no content, which names them from its own tokens alone.
    pub(crate) fn factions(&self) -> io::Result<&Option<Arc<Catalogue>>> {
        self.factions.get(|| {
            Ok(self
                .context
                .content
                .as_ref()
                .and_then(|options| crate::catalogue::catalogue(self.source, options).ok()))
        })
    }

    /// The icon content production lines choose each produced variant's icon by, as their row
    /// preparation loads it; none without the game's content.
    pub(crate) fn production_icons(&self) -> io::Result<&Option<Arc<IconContent>>> {
        self.production_icons.get(|| {
            Ok(production_lines::preparation_icons(
                self.source,
                self.context,
            ))
        })
    }

    /// The recorded content special projects are named and drawn with.
    pub(crate) fn projects(&self) -> io::Result<&special_projects::facts::Binding> {
        self.projects
            .get(|| special_projects::facts::Binding::load(self.source, self.context))
    }

    /// The equipment types the stockpile classifies variants and logistics tokens by.
    pub(crate) fn stockpile(&self) -> io::Result<&equipment_stockpile::emit::Content> {
        self.stockpile.get(|| {
            Ok(equipment_stockpile::emit::Content::load(
                self.source,
                self.context.content.as_ref(),
            ))
        })
    }

    /// The catalogue and names sunk ships and naval battles are named with, or why the
    /// recorded stack gives none.
    pub(crate) fn naval(&self) -> io::Result<&ships_sunk::PreparedContent> {
        self.naval
            .get(|| Ok(ships_sunk::prepared_content(self.source, self.context)))
    }
}

impl Content<'_> {
    /// The map's areas and the strategic regions territory measures and attributes with.
    pub(crate) fn territory(&self) -> io::Result<&crate::products::territory::Configured> {
        self.territory.get(|| {
            crate::products::territory::Configured::read(
                self.source,
                self.context,
                &AtomicBool::new(false),
            )
        })
    }
}

impl Content<'_> {
    /// The equipment types air names its parts by; none where the recorded stack gives none,
    /// whose parts then stay the recorded variants'.
    pub(crate) fn air(&self) -> io::Result<&Option<Arc<installed_content_core::equipment::Database>>> {
        self.air.get(|| {
            Ok(self.context.content.as_ref().and_then(|options| {
                let catalogue = crate::catalogue::catalogue(self.source, options).ok()?;
                dashboard_base::products::equipment::units(self.source, &catalogue).ok()
            }))
        })
    }
}

impl Content<'_> {
    /// The sub-units navy names its ship classes' parts by; none where the recorded stack gives
    /// none, whose parts then stay the recorded classes'.
    pub(crate) fn navy(&self) -> io::Result<&Option<Arc<installed_content_core::unit_leader::SubUnits>>> {
        self.navy.get(|| {
            Ok(self.context.content.as_ref().and_then(|options| {
                let catalogue = crate::catalogue::catalogue(self.source, options).ok()?;
                crate::products::navy::content_units(&catalogue, &mut Vec::new())
            }))
        })
    }
}

impl Content<'_> {
    /// The intelligence observers' operative content, catalogue and cipher defines, each with
    /// why the recorded stack refused it.
    pub(crate) fn intelligence(&self) -> io::Result<&intelligence_prepared::Content> {
        self.intelligence
            .get(|| intelligence_prepared::Content::load(self.source, self.context))
    }
}

impl Content<'_> {
    /// The occupation laws, state names and garrison content the recorded stack gives, or the
    /// warning naming why it gives none.
    pub(crate) fn occupation(&self) -> io::Result<&occupied_states::prepared::Binding> {
        self.occupation.get(|| {
            Ok(occupied_states::prepared::Binding::load(
                self.source,
                self.context,
            ))
        })
    }
}

impl Content<'_> {
    /// The unknown-attacker title and campaign-name content casualties present with, or why
    /// the recorded stack refused it.
    pub(crate) fn casualties(
        &self,
    ) -> io::Result<&crate::products::war_casualties::prepared::PreparedContent> {
        self.casualties.get(|| {
            crate::products::war_casualties::prepared::prepared_content(self.source, self.context)
        })
    }
}

impl Content<'_> {
    /// The catalogue the division log names divisions and templates from, its name groups
    /// read, or why it is unavailable, which the log's pages name.
    pub(crate) fn divisions(&self) -> io::Result<&crate::products::division_log::Names> {
        self.divisions.get(|| {
            let names = crate::products::division_log::names_content(self.source, self.context);
            if let Some(catalogue) = &names.0 {
                crate::products::division_log::division_name_groups(self.source, catalogue);
            }
            Ok(names)
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

impl Content<'_> {
    /// The raid types, names and target texts the recorded catalogue gives raids; none where it
    /// gives none, as the walk read them.
    pub(crate) fn raids(&self) -> io::Result<&Option<Arc<crate::products::raids::Content>>> {
        self.raids.get(|| {
            Ok(
                crate::products::raids::prepared_content(self.source, self.context)
                    .ok()
                    .and_then(|(content, _)| content),
            )
        })
    }
}

impl Content<'_> {
    /// The user's game language's localisation of the recorded stack, which the land-owner
    /// catalogue names countries in, or why it shows their tags.
    pub(crate) fn names(&self) -> io::Result<&Names> {
        self.names.get(|| {
            Ok(dashboard_base::content::name_localisation(
                self.source,
                self.context.content.as_ref(),
            ))
        })
    }
}
