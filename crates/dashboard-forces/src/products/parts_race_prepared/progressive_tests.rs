#[cfg(feature = "slow-tests")]
use super::test_fixtures::frame;
use super::*;
#[cfg(feature = "slow-tests")]
use dashboard_base::test_fixtures::{CAMPAIGN, records};
#[cfg(feature = "slow-tests")]
use std::sync::Arc;

#[cfg(feature = "slow-tests")]
fn sampler_matches<F: Family>() {
    let frames = [frame(6, 5000), frame(7, 5000)];
    let replay =
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames.to_vec())]).unwrap());
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "force-sample".into(),
            revision: "1".into(),
        },
        "force-sample.battleplan".into(),
        replay.clone(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let mut prepared = Prepared::<F>::default();
    for frame in &frames {
        prepared
            .observe(
                frame,
                &replay,
                &source,
                &LoadContext { content: None },
                &AtomicBool::new(false),
                &dashboard_base::prepared::frame_changes::FrameChangeTracker::new()
                    .observe(frame, replay.format),
            )
            .unwrap();
        let samples = sample::<F>(frame, None).unwrap();
        assert_eq!(samples.len(), 5000 * (4 + F::REFERENCES));
        let observed = std::array::from_fn(|index| F::observed(&prepared.scan, index, frame.hour));
        assert_eq!(
            samples,
            sample_rows::<F>(&prepared.scan, &observed, None).unwrap()
        );
        // Reading open state is a publication boundary: unchanged endpoints are deferred.
        for values in prepared.values.iter_mut().chain(&mut prepared.maximum) {
            values.seal();
        }
        for ((tag, field), (identity, value)) in &samples {
            assert!(identity.is_some());
            let key = prepared.keys[&series::field_key(tag, "")];
            let accumulator = if let Some(channel) = field.strip_prefix("maximum-") {
                &prepared.maximum[F::reference(channel.parse().unwrap()).unwrap()]
            } else {
                &prepared.values[field.parse::<usize>().unwrap()]
            };
            assert_eq!(
                *value,
                accumulator
                    .state(key)
                    .and_then(|state| state.last)
                    .filter(|run| run.last_hour == frame.hour)
                    .map(|run| run.value)
            );
        }
    }
    assert_eq!(
        sample::<F>(&frames[0], Some(&[("absent".into(), "0".into())]))
            .unwrap()
            .get(&("absent".into(), "0".into())),
        Some(&(None, None))
    );
}

#[test]
#[cfg(feature = "slow-tests")]
fn air_force_progressive_sampler_matches_observer_every_key_at_5000_countries() {
    sampler_matches::<crate::products::air_force::prepared::Air>();
}
#[test]
#[cfg(feature = "slow-tests")]
fn navy_progressive_sampler_matches_observer_every_key_at_5000_countries() {
    sampler_matches::<crate::products::navy::prepared::Navy>();
}

#[test]
fn progressive_part_absence_is_zero_and_explicit_refusal_stays_unknown() {
    let totals = [Run::new(0, 5, 10, true)];
    let parts = [Run::new(0, 1, 3, true), Run::new(4, 5, 7, false)];
    let witnessed = [(0, 2, 0, 1), (4, 5, 0, 1)];
    let runs = part_runs(&totals, &parts, &witnessed);
    assert_eq!(transient_at(&runs, 2), None);
    assert_eq!(transient_at(&runs, 3).unwrap().value, 0);
    assert_eq!(transient_at(&runs, 4).unwrap().value, 7);
}
