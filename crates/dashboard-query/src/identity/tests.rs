use super::*;

fn run(first: i32, last: i32, holder: Holder) -> HolderRun {
    HolderRun { first, last, holder, starts_segment: true }
}

const GER: Holder = Holder::Found { slot: 5, id: 5 };
const GER_AFTER: Holder = Holder::Found { slot: 5, id: 377 };

#[test]
fn a_tag_that_changes_holder_starts_a_new_run_and_segment() {
    let runs = merged(vec![run(100, 109, GER), run(110, 119, GER_AFTER)]);
    assert_eq!(runs, vec![run(100, 109, GER), run(110, 119, GER_AFTER)]);
    let holders = Holders { tags: [("GER".to_owned(), runs)].into(), unknown: Vec::new() };
    assert_eq!(holders.at("GER", 115).map(|run| run.holder), Some(GER_AFTER));
    assert_eq!(holders.segment_first("GER", 115), Some(110));
    assert_eq!(holders.segment_first("GER", 105), Some(100));
}

#[test]
fn the_same_holder_across_a_load_into_a_recorded_hour_is_one_run() {
    // Two stretches of runs, the second beginning the hour after the first ends.
    let runs = merged(vec![run(200, 299, GER), run(100, 199, GER)]);
    assert_eq!(runs, vec![run(100, 299, GER)]);
}

#[test]
fn hours_without_a_holder_break_the_run_and_unresolved_is_never_a_holder() {
    let runs = merged(vec![run(100, 109, GER), run(110, 114, Holder::Unresolved), run(115, 119, GER), run(130, 139, GER)]);
    assert_eq!(runs.len(), 4, "unresolved hours and a gap each break the holder's run");
    let holders = Holders { tags: [("GER".to_owned(), runs)].into(), unknown: Vec::new() };
    assert_eq!(holders.at("GER", 112).map(|run| run.holder), Some(Holder::Unresolved));
    assert!(holders.at("GER", 125).is_none(), "an hour no run holds is absent, not a holder");
    assert_eq!(holders.segment_first("GER", 117), Some(115));
    let within = holders.within("GER", 105, 132);
    assert_eq!((within.first().map(|run| run.first), within.last().map(|run| run.last)), (Some(105), Some(132)));
}
