use super::independent;
use crate::{
    Source,
    metadata::check_cancel,
    products::{LoadContext, production_lines},
};
use std::io;

pub(super) fn missing(
    source: &Source,
    store: &store::Store,
    key: &store::SourceKey,
    context: &LoadContext,
) -> io::Result<Vec<u64>> {
    let mut branches = Vec::new();
    for branch in source
        .metadata()
        .branches
        .iter()
        .filter_map(|branch| branch.id.parse::<u64>().ok())
    {
        if !production_lines::prepared::current(store, key, source, context, branch)? {
            branches.push(branch);
        }
    }
    Ok(branches)
}

pub(super) fn jobs<'a>(
    phase: &'a independent::Phase<'a>,
) -> io::Result<Vec<independent::Beside<'a>>> {
    let branches = missing(phase.source, phase.store, phase.key, phase.context)?;
    if branches.is_empty() {
        return Ok(Vec::new());
    }
    #[cfg(test)]
    let stall = TEST_STALL.with(|stall| stall.borrow().clone());
    Ok(vec![independent::Beside {
        name: "production-lines",
        run: Box::new(
            move |publish: &mut independent::Publish<'_>, _timings: &mut independent::Timings| {
                #[cfg(test)]
                if let Some(stall) = stall {
                    stall();
                }
                let icons = production_lines::preparation_icons(phase.source, phase.context);
                let countries = phase.source.metadata().countries;
                for branch in branches {
                    check_cancel(phase.cancelled)?;
                    let mut prepared = production_lines::prepared::Prepared::resume(
                        phase.store,
                        phase.key,
                        phase.replay,
                        branch,
                        icons.as_deref(),
                    )?
                    .unwrap_or_else(|| {
                        production_lines::prepared::Prepared::new(
                            phase.replay,
                            branch,
                            countries.iter().map(|country| country.tag.clone()),
                            icons.as_deref(),
                        )
                    });
                    let next = prepared.next_block(phase.replay)?;
                    let mut reconcile = true;
                    if let Some(&position) = phase.last_blocks.get(&branch)
                        && position < next
                    {
                        let coverage = super::Coverage {
                            schema: super::SCHEMA,
                            first_hour: phase.ranges[&branch].0,
                            last_hour: phase.replay.index[position].last_hour,
                            complete: true,
                        };
                        publish.production(branch, coverage, &mut |writer| {
                            prepared.reconcile_tags(
                                writer,
                                phase.replay,
                                countries.iter().map(|country| country.tag.as_str()),
                                icons.as_deref(),
                                phase.cancelled,
                            )
                        })?;
                        continue;
                    }
                    for (position, block) in phase
                        .replay
                        .index
                        .iter()
                        .enumerate()
                        .filter(|(position, block)| *position >= next && block.branch == branch)
                    {
                        let coverage = super::Coverage {
                            schema: super::SCHEMA,
                            first_hour: phase.ranges[&branch].0,
                            last_hour: block.last_hour,
                            complete: phase.last_blocks.get(&branch) == Some(&position),
                        };
                        publish.production(branch, coverage, &mut |writer| {
                            if reconcile {
                                prepared.reconcile_tags(
                                    writer,
                                    phase.replay,
                                    countries.iter().map(|country| country.tag.as_str()),
                                    icons.as_deref(),
                                    phase.cancelled,
                                )?;
                                reconcile = false;
                            }
                            prepared.advance_block(
                                writer,
                                phase.replay,
                                position,
                                icons.as_deref(),
                                phase.cancelled,
                            )?;
                            prepared.publish(
                                writer,
                                phase.replay,
                                block,
                                icons.as_deref(),
                                phase.cancelled,
                            )
                        })?;
                    }
                }
                Ok(())
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
