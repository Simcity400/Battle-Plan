use super::*;
use crate::products::land_battles::warnings::Warnings;

type Model = BTreeMap<(i32, String), usize>;

/// The pseudo-random additions both forms receive: hour, reason and count.
fn additions(seed: u64, count: usize) -> Vec<(i32, String, usize)> {
    let mut state = seed;
    let mut next = move || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        state >> 33
    };
    (0..count)
        .map(|_| {
            let hour = 100 + (next() % 40) as i32;
            let reason = format!("Reason {}", next() % 4);
            // Mostly one, as warnings count reads, with some larger and some zero.
            let added = [1, 1, 1, 1, 2, 5, 0][(next() % 7) as usize];
            (hour, reason, added)
        })
        .collect()
}

fn hourly(model: &Model) -> Vec<(i32, &str, usize)> {
    model
        .iter()
        .map(|((hour, reason), count)| (*hour, reason.as_str(), *count))
        .collect()
}

/// What the census reported from its hourly map: each reason's first hour and total.
fn totals(model: &Model) -> Vec<(&str, i32, usize)> {
    let mut summary: BTreeMap<&str, (i32, usize)> = BTreeMap::new();
    for ((hour, reason), count) in model {
        summary.entry(reason).or_insert((*hour, 0)).1 += count;
    }
    summary
        .into_iter()
        .map(|(reason, (hour, count))| (reason, hour, count))
        .collect()
}

fn between(model: &Model, first: i32, last: i32) -> Model {
    model
        .iter()
        .filter(|((hour, _), _)| (first..=last).contains(hour))
        .map(|(key, count)| (key.clone(), *count))
        .collect()
}

/// The warnings an hourly map holds, added hour by hour in key order.
fn ordered(model: &Model) -> Warnings {
    let mut warnings = Warnings::default();
    for ((hour, reason), count) in model {
        warnings.add(*hour, reason.as_str(), *count);
    }
    warnings
}

#[test]
fn warning_runs_report_exactly_the_hourly_counts_they_replace() {
    for seed in 1..=24 {
        let mut warnings = Warnings::default();
        let mut model = Model::new();
        for (hour, reason, count) in additions(seed, 300) {
            warnings.add(hour, reason.as_str(), count);
            *model.entry((hour, reason)).or_default() += count;
            assert_eq!(warnings.hourly(), hourly(&model), "seed {seed}");
        }
        // Runs are canonical: any order of the same counts holds the same runs.
        assert_eq!(warnings, ordered(&model), "seed {seed}");
        assert_eq!(warnings.totals().collect::<Vec<_>>(), totals(&model));
        assert!(warnings.len() <= model.len());
        for (first, last) in [
            (90, 99),
            (100, 139),
            (95, 104),
            (117, 117),
            (120, 131),
            (131, 120),
        ] {
            let clipped = warnings.between(first, last);
            let expected = between(&model, first, last);
            assert_eq!(
                clipped.hourly(),
                hourly(&expected),
                "seed {seed} {first}..={last}"
            );
            assert_eq!(clipped, ordered(&expected));
            assert_eq!(clipped.totals().collect::<Vec<_>>(), totals(&expected));
        }
        let restored: Warnings =
            serde_json::from_slice(&serde_json::to_vec(&warnings).unwrap()).unwrap();
        assert_eq!(restored, warnings);

        let mut kept = warnings.clone();
        kept.retain_from(121);
        assert_eq!(kept, ordered(&between(&model, 121, i32::MAX)));
        // Two halves merged in either order rebuild the whole.
        for (early, late) in [(110, 111), (125, 126)] {
            let (low, high) = (
                warnings.between(i32::MIN, early),
                warnings.between(late, i32::MAX),
            );
            let mut merged = low.clone();
            merged.merge(high.clone());
            assert_eq!(merged, warnings);
            let mut merged = high;
            merged.merge(low);
            assert_eq!(merged, warnings);
        }
        // Merging overlapping counts adds them hour by hour.
        let other_model: Model = additions(seed + 100, 120).into_iter().fold(
            Model::new(),
            |mut model, (hour, reason, count)| {
                *model.entry((hour, reason)).or_default() += count;
                model
            },
        );
        let mut merged = warnings.clone();
        merged.merge(ordered(&other_model));
        let mut sum = model.clone();
        for (key, count) in other_model {
            *sum.entry(key).or_default() += count;
        }
        assert_eq!(merged, ordered(&sum), "seed {seed}");
    }
}

#[test]
fn warning_runs_hold_a_steady_reason_once() {
    let mut warnings = Warnings::default();
    for hour in 100..10_100 {
        for _ in 0..3 {
            warnings.add(
                hour,
                "Equipment variant 7: name not recorded or content unavailable",
                1,
            );
        }
        warnings.add(
            hour,
            "Participant country reference 99 unresolved",
            1 + (hour % 2) as usize,
        );
    }
    // One run for the steady count; every other hour changes the alternating one.
    assert_eq!(warnings.len(), 1 + 10_000);
    assert_eq!(
        warnings.totals().collect::<Vec<_>>(),
        [
            (
                "Equipment variant 7: name not recorded or content unavailable",
                100,
                30_000
            ),
            ("Participant country reference 99 unresolved", 100, 15_000),
        ]
    );
}
