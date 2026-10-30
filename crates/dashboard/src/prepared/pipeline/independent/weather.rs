use super::{Block, Branch, Load, Observed, Own};
use crate::prepared::prepass::{self, Folded};
use crate::products::regional_weather::block;
use dashboard_keyed::{Observer, Scan};
use replay::container::Replay;
use std::{any::Any, io, sync::atomic::AtomicBool};

pub(in crate::prepared::pipeline) struct Weather;
pub(in crate::prepared::pipeline) static WEATHER: Weather = Weather;

impl Own for Weather {
    fn family(&self) -> &'static prepass::Family {
        &block::FAMILY
    }
    fn timing(&self) -> &'static str {
        "independent.regional-weather"
    }
    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        let scan = match load.saved {
            Some(_) => crate::prepared::series::read_saved_catalogue::<Scan<block::Observer>>(
                load.store,
                load.key,
                load.saved,
                block::Observer::NAME,
                branch,
            )?
            .map(|(_, scan)| scan),
            None => None,
        };
        Ok(Box::new(State {
            scan: Box::new(scan.unwrap_or_default()),
            reached: false,
        }))
    }
}

struct State {
    scan: Box<Scan<block::Observer>>,
    reached: bool,
}
impl Branch for State {
    fn begin_block(&mut self) {
        self.reached = false;
    }
    fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        at: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        self.scan
            .observer
            .bind_checked(at.source, at.context.content.as_ref(), check)?;
        let (folded, reached) =
            block::fold(&mut self.scan, facts, at.replay, at.index, observed, check)?;
        self.reached |= reached.is_some();
        Ok((folded, reached))
    }
    fn observe_block(
        &mut self,
        at: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        self.scan
            .observer
            .bind_checked(at.source, at.context.content.as_ref(), check)?;
        let result = dashboard_keyed::block::observe_block(
            &mut self.scan,
            at.replay,
            at.index,
            None,
            observed,
            check,
        );
        if result.is_err() {
            self.scan.observer.retire();
        }
        let reached = result?;
        self.reached |= reached.is_some();
        Ok(reached)
    }
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
        let result = self.scan.flush(writer, branch, cancelled);
        if result.is_err() {
            self.scan.observer.retire();
        }
        result
    }
    fn save_state(&self, _: &mut store::Writer, _: u64) -> io::Result<()> {
        Ok(())
    }
}
