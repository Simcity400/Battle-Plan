use super::*;

fn run(first_hour: i32, last_hour: i32, value: i128, break_before: bool) -> Run {
    Run {
        first_hour,
        last_hour,
        value,
        break_before,
    }
}

#[test]
fn military_aid_intervals_preserve_empty_unknown_and_segment_boundaries() {
    let proof = [run(0, 2, 1, true), run(4, 6, 1, true)];
    let values = [run(1, 2, 2, false), run(4, 4, 1, true)];
    let runs = project(&values, [&proof, &proof, &proof], 0, 6);
    assert_eq!(
        runs.iter()
            .map(|run| (
                run.first_hour,
                run.last_hour,
                run.divisions,
                run.starts_segment
            ))
            .collect::<Vec<_>>(),
        [
            (0, 0, Some(0.0), true),
            (1, 2, Some(2.0), false),
            (3, 3, None, false),
            (4, 4, Some(1.0), true),
            (5, 6, Some(0.0), false)
        ]
    );
}

#[test]
fn military_aid_refuses_air() {
    assert!(
        serde_json::from_value::<VolunteersExpeditionariesParams>(
            serde_json::json!({"branchId":"1","kind":"air","tags":null})
        )
        .is_err()
    );
}
