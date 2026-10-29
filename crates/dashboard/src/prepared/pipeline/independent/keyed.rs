//! The keyed families prepared on their own ([`dashboard_keyed`]): air
//! missions and air statistics, regional weather, minefields, naval
//! supremacy, doctrine mastery, occupation reserve manpower and intel network
//! coverage. Each is a family of its own, with its own registry entry,
//! coverage, refusals and catalogue; none reads another's state and the
//! hourly walk does not run them. A branch's scan loads from the catalogue
//! its last flush published, its blocks fold from the pre-pass facts
//! ([`dashboard_keyed::block`]), and each flush writes its series and its
//! catalogue together, which is also its continuation.

use super::{Block, Branch, Load, Observed, Own};
use crate::prepared::prepass::{self, Folded};
use dashboard_keyed::{Observer, Scan, block};
use replay::container::Replay;
use std::{any::Any, io, marker::PhantomData, sync::atomic::AtomicBool};

/// One keyed family: its observer and the product queries its coverage
/// gates.
pub(in crate::prepared::pipeline) struct Keyed<O> {
    timing: &'static str,
    gates: &'static [&'static str],
    observer: PhantomData<fn() -> O>,
}

impl<O> Keyed<O> {
    pub(in crate::prepared::pipeline) const fn new(
        timing: &'static str,
        gates: &'static [&'static str],
    ) -> Self {
        Self {
            timing,
            gates,
            observer: PhantomData,
        }
    }
}

impl<O: Observer> Own for Keyed<O> {
    fn family(&self) -> &'static prepass::Family {
        O::family()
    }

    fn timing(&self) -> &'static str {
        self.timing
    }

    fn gates(&self) -> &'static [&'static str] {
        self.gates
    }

    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        // A branch it refused is never read again; one it has not committed
        // starts new.
        let scan = match load.saved {
            Some(_) => crate::prepared::series::read_saved_catalogue::<Scan<O>>(
                load.store,
                load.key,
                load.saved,
                O::NAME,
                branch,
            )?
            .map(|(_, scan)| scan),
            None => None,
        };
        Ok(Box::new(KeyedBranch {
            scan: Box::new(scan.unwrap_or_default()),
            reached: false,
        }))
    }
}

struct KeyedBranch<O> {
    scan: Box<Scan<O>>,
    /// Whether the block it goes through gave it an hour.
    reached: bool,
}

impl<O: Observer> Branch for KeyedBranch<O> {
    fn begin_block(&mut self) {
        self.reached = false;
    }

    fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        self.scan
            .observer
            .bind(block.source, block.context.content.as_ref(), check)?;
        let (folded, reached) = block::fold(
            &mut self.scan,
            facts,
            block.replay,
            block.index,
            observed,
            check,
        )?;
        self.reached |= reached.is_some();
        Ok((folded, reached))
    }

    fn observe_block(
        &mut self,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        self.scan
            .observer
            .bind(block.source, block.context.content.as_ref(), check)?;
        let reached = block::observe_block(
            &mut self.scan,
            block.replay,
            block.index,
            None,
            observed,
            check,
        )?;
        self.reached |= reached.is_some();
        Ok(reached)
    }

    /// A block that gave no hour ends the observation.
    fn end_block(&mut self, _: &Replay, _: u64) {
        if !self.reached {
            self.scan.unreadable_block();
        }
    }

    fn pending(&self) -> usize {
        self.scan.buffered_bytes()
    }

    fn flush(
        &mut self,
        writer: &mut store::Writer,
        _: &Replay,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.scan.flush(writer, branch, cancelled)
    }

    fn save_state(&self, _: &mut store::Writer, _: u64) -> io::Result<()> {
        Ok(())
    }
}

use crate::products::{
    air_missions, air_statistics, doctrine_mastery, intel_coverage, minefields, naval_refits,
    naval_repairs, naval_supremacy, occupation_reserve, style_runs,
};

pub(in crate::prepared::pipeline) static AIR_MISSIONS: Keyed<air_missions::AirMissionsObserver> =
    Keyed::new("independent.air-missions", &["air-missions"]);
pub(in crate::prepared::pipeline) static AIR_STATISTICS: Keyed<
    air_statistics::AirStatisticsObserver,
> = Keyed::new("independent.air-statistics", &["air-statistics"]);
pub(in crate::prepared::pipeline) use super::weather::WEATHER;
pub(in crate::prepared::pipeline) static MINEFIELDS: Keyed<minefields::MinefieldsObserver> =
    Keyed::new("independent.minefields", &["minefields"]);
pub(in crate::prepared::pipeline) static NAVAL_SUPREMACY: Keyed<
    naval_supremacy::NavalSupremacyObserver,
> = Keyed::new("independent.naval-supremacy", &["naval-supremacy"]);
pub(in crate::prepared::pipeline) static DOCTRINE_MASTERY: Keyed<
    doctrine_mastery::DoctrineMasteryObserver,
> = Keyed::new("independent.doctrine-mastery", &["doctrine-mastery"]);
pub(in crate::prepared::pipeline) static OCCUPATION_RESERVE: Keyed<
    occupation_reserve::OccupationReserveObserver,
> = Keyed::new(
    "independent.occupation-reserve",
    &["occupation-reserve-manpower"],
);
pub(in crate::prepared::pipeline) static INTEL_COVERAGE: Keyed<
    intel_coverage::IntelCoverageObserver,
> = Keyed::new(
    "independent.intel-network-coverage",
    &["intel-network-coverage"],
);
pub(in crate::prepared::pipeline) static NAVAL_REPAIRS: Keyed<naval_repairs::RepairsObserver> =
    Keyed::new("independent.naval-repairs", &["naval-repairs"]);
pub(in crate::prepared::pipeline) static NAVAL_REFITS: Keyed<naval_refits::RefitsObserver> =
    Keyed::new("independent.naval-refits", &["naval-refits"]);
pub(in crate::prepared::pipeline) static COUNTRY_STYLES: Keyed<style_runs::StylesObserver> =
    Keyed::new(
        "independent.country-styles",
        &["country-styles", "country-colours"],
    );
