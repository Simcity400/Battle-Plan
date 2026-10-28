use super::{Block, Branch, Load, Observed, Own};
use crate::prepared::prepass::{self, Folded};
use dashboard_base::prepared_map::{
    pages::{self, BlockPages, PageFold},
    selected_tables,
};
use replay::container::Replay;
use std::{any::Any, io, sync::atomic::AtomicBool};

pub(in crate::prepared::pipeline) struct MapPages;
pub(in crate::prepared::pipeline) static MAP_PAGES: MapPages = MapPages;

impl Own for MapPages {
    fn family(&self) -> &'static prepass::Family {
        &pages::FAMILY
    }
    fn timing(&self) -> &'static str {
        "independent.map_pages"
    }
    fn gates(&self) -> &'static [&'static str] {
        &[]
    }
    fn paced(&self) -> bool {
        true
    }
    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        let committed = load
            .read(&pages::progress_section(branch))?
            .map(|file| serde_json::from_reader(io::BufReader::new(file)))
            .transpose()?;
        let mut fold = PageFold::new(branch, selected_tables(), &pages::STRIDES, committed)?;
        fold.resume(|section| load.read(section))?;
        Ok(Box::new(State { fold }))
    }
}

/// The groups of map tables a block's pages are produced in at once: one per four CPUs
/// preparation uses, as the family's own thread is one chain among many. Tests produce whole.
fn groups() -> usize {
    if cfg!(test) {
        return 1;
    }
    (super::super::parallelism() / 4).clamp(1, 8)
}

/// One branch's page fold.
struct State {
    fold: PageFold,
}

impl State {
    fn fold_block(
        &mut self,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
        produce: impl FnOnce(usize, &mut dyn FnMut() -> io::Result<()>) -> io::Result<BlockPages>,
    ) -> io::Result<Option<i32>> {
        let folded = (|| -> io::Result<_> {
            let ordinal = block
                .replay
                .block_position(block.index)
                .ok_or_else(|| invalid("map page block is not in the recording"))?;
            let pages = produce(ordinal, &mut *check)?;
            let reached = pages.reached();
            Ok((reached, self.fold.fold(ordinal, pages)?))
        })();
        match folded {
            Ok((reached, None)) => Ok(reached),
            Ok((_, Some((hour, reason)))) => {
                observed(hour, Err(invalid(&reason)))?;
                Ok(Some(hour))
            }
            Err(error) => {
                let hour = block.index.first_hour;
                observed(hour, Err(error))?;
                Ok(Some(hour))
            }
        }
    }
}

fn invalid(reason: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason.to_owned())
}

impl Branch for State {
    fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        let pages = facts
            .downcast::<BlockPages>()
            .map_err(|_| io::Error::other("Map page pre-pass facts have another family"))?;
        let reached = self.fold_block(block, observed, check, |_, _| Ok(*pages))?;
        Ok((Folded::Fast, reached))
    }
    fn observe_block(
        &mut self,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        self.fold_block(block, observed, check, |ordinal, _| {
            BlockPages::produce_grouped(
                block.replay,
                ordinal,
                selected_tables(),
                &pages::STRIDES,
                groups(),
                block.cancelled,
            )
        })
    }
    fn publish(&mut self, writer: &mut store::Writer, _: u64, _: &AtomicBool) -> io::Result<()> {
        self.fold.publish(writer)
    }
    fn pending(&self) -> usize {
        self.fold.pending()
    }
    fn flush(
        &mut self,
        _: &mut store::Writer,
        _: &Replay,
        _: u64,
        _: &AtomicBool,
    ) -> io::Result<()> {
        Ok(())
    }
    fn save_state(&self, writer: &mut store::Writer, branch: u64) -> io::Result<()> {
        super::super::json(writer, &pages::progress_section(branch), self.fold.pages())?;
        self.fold.save_states(writer)
    }
}
