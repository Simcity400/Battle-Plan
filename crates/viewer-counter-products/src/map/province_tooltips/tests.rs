use super::*;

#[test]
fn thermal_identifier_notes_reach_province_content_warnings() {
    let content = Content {
        members: Default::default(),
        impassable: Default::default(),
        neighbors: Ok(Default::default()),
        temperature: Ok(installed_content::weather::TemperatureRanges {
            ranges: Default::default(),
            notes: vec!["fixture modifier: unsupported identifier field (1 modifier)".into()],
        }),
        pass: Err("unread".into()),
        derived: Default::default(),
        units: Err("unused".into()),
        border: Err("unused".into()),
        air_log_timeout: Err("unused".into()),
        occupation: Err("unused".into()),
        network_base_target: Err("unused".into()),
    };
    assert!(content.warnings().iter().any(|warning| warning ==
        "fixture modifier: unsupported identifier field (1 modifier)"));
    assert!(
        content.warnings().iter().any(|warning| warning.contains(
            "Province temperatures cannot be derived from the recorded content (unread)"
        ))
    );
}

#[test]
fn native_multiline_indentation_keeps_gaps_and_refusals_out_of_text() {
    let mut tooltip = Tooltip::default();
    line(
        &mut tooltip,
        "location",
        0,
        Ok("Forest in Test state".into()),
    );
    line(&mut tooltip, "owner", 1, Ok("Owner\n\nController".into()));
    line(
        &mut tooltip,
        "missing input",
        1,
        Err("operand unavailable".into()),
    );
    line(&mut tooltip, "vp", 1, Ok("Victory point: 1".into()));
    assert_eq!(
        tooltip.title,
        "Forest in Test state\n    Owner\n\n    Controller\n    Victory point: 1"
    );
    assert_eq!(
        tooltip.refused["province missing input: operand unavailable"],
        1
    );
    assert!(tooltip.body.is_empty());
}

/// A tooltip built while its air roster history was still being read is held as pending:
/// asking again without an answer keeps it, waiting returns once the worker answers, and
/// the next ask rebuilds it with what the history gave and nothing pending.
#[test]
fn a_tooltip_built_with_a_pending_history_rebuilds_once_answered() {
    let replay = air_region::tests::recording(80, &[]);
    let mut holder = Holder::<u32>::default();
    let built = std::sync::atomic::AtomicUsize::new(0);
    let ask = |holder: &mut Holder<u32>| {
        holder.get(
            7,
            true,
            &replay,
            || {},
            |rosters| {
                built.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let mut out = Tooltip::default();
                match rosters.visiting_pass(1, 127, 5) {
                    Some(Ok(pass)) => out.title = format!("pass {pass:?}"),
                    Some(Err(reason)) => out.refuse("pass", reason),
                    None => {}
                }
                out
            },
        )
    };
    let first = ask(&mut holder);
    assert!(first.title.is_empty());
    let pending = holder.pending();
    if pending {
        holder.wait();
    }
    let answered = ask(&mut holder);
    assert_eq!(answered.title, "pass Some(72)");
    assert!(!holder.pending());
    // The first build always asks the worker, so it was pending; waiting made the
    // second ask rebuild instead of returning the held text.
    assert!(pending);
    assert_eq!(built.load(std::sync::atomic::Ordering::Relaxed), 2);
    // Nothing pending and nothing new answered: the held tooltip is returned as built.
    let held = ask(&mut holder);
    assert!(Arc::ptr_eq(&held, &answered));
    assert_eq!(built.load(std::sync::atomic::Ordering::Relaxed), 2);
    holder.wait();
}
