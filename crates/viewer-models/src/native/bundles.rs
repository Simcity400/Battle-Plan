use std::collections::HashMap;

pub(super) const RUN: usize = 64;

pub(super) const STILL: u32 = 8;

pub(super) struct Run<B> {
    pub start: usize,
    pub ids: Vec<u64>,
    pub reference: u32,
    pub still: u32,
    pub bundle: Option<B>,
}

impl<B> Run<B> {
    pub fn end(&self) -> usize {
        self.start + self.ids.len()
    }
}

pub(super) fn plan<B>(old: Vec<Run<B>>, draws: &[(u64, u32)]) -> Vec<Run<B>> {
    let mut old: HashMap<u64, Run<B>> = old
        .into_iter()
        .filter_map(|run| run.ids.first().copied().map(|id| (id, run)))
        .collect();
    let mut runs = Vec::new();
    let mut at = 0;
    while at < draws.len() {
        if let Some(run) = old.remove(&draws[at].0) {
            let end = at + run.ids.len();
            if end <= draws.len()
                && draws[at..end]
                    .iter()
                    .map(|(id, _)| *id)
                    .eq(run.ids.iter().copied())
            {
                runs.push(Run { start: at, ..run });
                at = end;
                continue;
            }
        }
        let reference = draws[at].1;
        let mut end = at + 1;
        while end < draws.len()
            && end - at < RUN
            && draws[end].1 == reference
            && !old.contains_key(&draws[end].0)
        {
            end += 1;
        }
        runs.push(Run {
            start: at,
            ids: draws[at..end].iter().map(|(id, _)| *id).collect(),
            reference,
            still: 0,
            bundle: None,
        });
        at = end;
    }
    runs
}

#[cfg(test)]
mod tests {
    use crate::native::bundles::*;

    fn draws(ids: impl IntoIterator<Item = u64>) -> Vec<(u64, u32)> {
        ids.into_iter().map(|id| (id, 0)).collect()
    }

    fn recorded(runs: &mut [Run<u64>]) {
        for run in runs {
            run.bundle.get_or_insert(run.ids[0]);
        }
    }

    #[test]
    fn runs_end_at_their_size_and_at_a_change_of_stencil_reference() {
        let mut list = draws(1..=150);
        list[100].1 = 7;
        let runs = plan::<u64>(Vec::new(), &list);
        let spans: Vec<_> = runs.iter().map(|run| (run.start, run.end(), run.reference)).collect();
        assert_eq!(spans, [(0, 64, 0), (64, 100, 0), (100, 101, 7), (101, 150, 0)]);
    }

    #[test]
    fn a_splice_records_again_only_the_runs_its_figures_changed() {
        let mut runs = plan::<u64>(Vec::new(), &draws(1..=192));
        recorded(&mut runs);
        let spliced: Vec<u64> = (1..=69).chain([500]).chain(73..=192).collect();
        let runs = plan(runs, &draws(spliced.iter().copied()));
        let kept: Vec<_> = runs.iter().map(|run| (run.start, run.ids.len(), run.bundle)).collect();
        assert_eq!(
            kept,
            [(0, 64, Some(1)), (64, 62, None), (126, 64, Some(129))],
            "the first and last runs keep their bundles; the changed one is recorded again"
        );
        let mut again = runs;
        recorded(&mut again);
        let again = plan(again, &draws(spliced.iter().copied()));
        assert!(again.iter().all(|run| run.bundle.is_some()));
    }

    #[test]
    fn a_kept_run_keeps_the_frames_it_stood_still_and_a_new_one_starts_again() {
        let mut runs = plan::<u64>(Vec::new(), &draws(1..=128));
        for run in &mut runs {
            run.still = bundles_still();
        }
        let spliced: Vec<u64> = (1..=64).chain([500]).chain(66..=128).collect();
        let runs = plan(runs, &draws(spliced));
        let still: Vec<_> = runs.iter().map(|run| run.still).collect();
        assert_eq!(still, [STILL, 0], "the unchanged run counts on; the changed one waits again");
    }

    fn bundles_still() -> u32 {
        STILL
    }

    #[test]
    fn a_run_whose_draws_are_parted_is_recorded_again() {
        let mut runs = plan::<u64>(Vec::new(), &draws(1..=10));
        recorded(&mut runs);
        let reordered: Vec<u64> = (1..=5).chain([20]).chain(6..=10).collect();
        let runs = plan(runs, &draws(reordered));
        assert!(runs.iter().all(|run| run.bundle.is_none()));
        assert_eq!(runs.iter().map(|run| run.ids.len()).sum::<usize>(), 11);
    }
}
