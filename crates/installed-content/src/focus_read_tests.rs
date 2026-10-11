use super::*;
use crate::{DiagnosticKind, mod_layers_tests::Stack};
use std::sync::{
    Barrier, Condvar, Mutex,
    atomic::{AtomicUsize, Ordering},
};

fn paths(layers: &ResolvedLayers) -> Vec<(usize, String)> {
    let mut paths = layers
        .paths()
        .filter(|path| {
            (path.starts_with("common/national_focus/")
                || path.starts_with("common/continuous_focus/"))
                && path.ends_with(".txt")
        })
        .map(|path| (layers.layer_index(path).unwrap_or(0), path.to_owned()))
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn serial(
    layers: &ResolvedLayers,
    names: &KeyNames,
) -> io::Result<(Focuses, Vec<crate::Diagnostic>)> {
    let mut definitions = Focuses::new();
    let mut warnings = Vec::new();
    for (_, path) in paths(layers) {
        let database = if path.starts_with("common/national_focus/") {
            Database::National
        } else {
            Database::Continuous
        };
        match parse(&layers.read(&path)?, database, names) {
            Ok((content, notes)) => {
                definitions.extend(content);
                warnings.extend(
                    notes
                        .into_iter()
                        .map(|(kind, note)| (kind, format!("{path}: {note}"))),
                );
            }
            Err((kind, error)) => warnings.push((
                kind,
                format!("Focus definitions unavailable in {path}: {error}"),
            )),
        }
    }
    Ok((definitions, warnings))
}

#[test]
fn ordered_parallel_focus_files_keep_layer_path_and_database_precedence() -> io::Result<()> {
    let stack = Stack::new("parallel_focus_precedence");
    for (layer, path, title) in [
        (
            stack.base(),
            "common/national_focus/10-base.txt",
            "BASE_EARLY",
        ),
        (
            stack.base(),
            "common/national_focus/90-base.txt",
            "BASE_LATE",
        ),
        (
            stack.base(),
            "common/national_focus/overlay.txt",
            "BASE_HIDDEN",
        ),
        (
            stack.addon(),
            "common/national_focus/overlay.txt",
            "MOD_OVERLAY",
        ),
        (
            stack.addon(),
            "common/national_focus/00-mod.txt",
            "MOD_EARLY",
        ),
        (
            stack.addon(),
            "common/national_focus/zz-mod.txt",
            "MOD_LATE",
        ),
    ] {
        stack.write(
            &layer,
            path,
            &format!("shared_focus={{id=SYN_DUPLICATE text={title}}}"),
        );
    }
    stack.write(
        &stack.addon(),
        "common/continuous_focus/00-mod.txt",
        "continuous_focus_palette={focus={id=SYN_DUPLICATE text=CONTINUOUS}}",
    );
    let layers = stack.layers(&[]);
    let names = layers.key_names()?;
    let expected = serial(&layers, &names)?;
    assert!(expected.1.is_empty(), "{:?}", expected.1);
    assert_eq!(expected.0["SYN_DUPLICATE"].localisation, "MOD_LATE");
    for workers in [1, 2, 3, 8] {
        assert_eq!(
            read_files(
                &paths(&layers),
                &names,
                workers,
                |path| layers.read(path),
                parse
            )?,
            expected,
        );
    }
    assert_eq!(read(&layers)?, expected);
    assert_eq!(read_national(&layers)?.0, expected.0);
    Ok(())
}

#[test]
fn parallel_focus_files_isolate_variables_and_share_complete_mod_names() -> io::Result<()> {
    let stack = Stack::new("parallel_focus_variables");
    stack.write(
        &stack.addon(),
        "common/scripted_triggers/probe.txt",
        "synthetic_focus_probe={always=yes}",
    );
    for (file, prefix, id, title) in [
        ("a", "@title=FIRST", "A", "@title"),
        ("b", "@title=SECOND", "B", "@title"),
        ("c", "", "C", "@title"),
        ("d", "@title=FOURTH", "D", "@title"),
        ("e", "", "E", "LITERAL"),
    ] {
        stack.write(
            &stack.base(),
            &format!("common/national_focus/{file}.txt"),
            &format!(
                "{prefix} shared_focus={{id={id} text={title} \
                 available={{synthetic_focus_probe=yes}}}}"
            ),
        );
    }
    let layers = stack.layers(&[]);
    let names = layers.key_names()?;
    assert!(names.scripted.contains(b"synthetic_focus_probe".as_slice()));
    let expected = serial(&layers, &names)?;
    assert!(expected.1.is_empty(), "{:?}", expected.1);
    for (id, title) in [
        ("A", "FIRST"),
        ("B", "SECOND"),
        ("C", "@title"),
        ("D", "FOURTH"),
    ] {
        assert_eq!(expected.0[id].localisation, title);
    }
    for workers in [1, 2, 3] {
        let actual = read_files(
            &paths(&layers),
            &names,
            workers,
            |path| layers.read(path),
            |bytes, database, shared_names| {
                assert!(std::ptr::eq(shared_names, names.as_ref()));
                parse(bytes, database, shared_names)
            },
        )?;
        assert_eq!(actual, expected);
    }
    Ok(())
}

#[test]
fn parallel_focus_files_preserve_diagnostic_kind_message_count_order_and_prefix() -> io::Result<()>
{
    let stack = Stack::new("parallel_focus_diagnostics");
    for (file, text) in [
        ("a", "shared_focus=9 id=RECOVERED text=RECOVERED_TITLE }"),
        (
            "b",
            "focus_tree={inlay_window={id=w position={1 2}} focus={id=VECTOR}}",
        ),
        ("c", "shared_focus={text=NO_ID}"),
        ("d", "shared_focus={id=NATIVE_RECOVERY available={x @y=1}}"),
        (
            "e",
            "shared_focus={id=COUNTED completion_reward={free_operative={operative=a} free_operative={operative=b}}}",
        ),
        (
            "f",
            "shared_focus={id=PREFIX text=PREFIX_TITLE} } shared_focus={id=UNREACHED}",
        ),
        (
            "g",
            "shared_focus={id=UNSETTLED available={synthetic_prefix:target={always=yes}}}",
        ),
    ] {
        stack.write(
            &stack.base(),
            &format!("common/national_focus/{file}.txt"),
            text,
        );
    }
    let layers = stack.layers(&[]);
    let names = layers.key_names()?;
    let expected = serial(&layers, &names)?;
    assert_eq!(expected.0["RECOVERED"].localisation, "RECOVERED_TITLE");
    assert_eq!(expected.0["PREFIX"].localisation, "PREFIX_TITLE");
    assert!(expected.0.contains_key("COUNTED"));
    // Complete load-point names let the reader skip this invalid trigger.
    // The apparent variable is consumed as its missing operator, not defined.
    assert!(expected.0.contains_key("NATIVE_RECOVERY"));
    for absent in ["VECTOR", "UNSETTLED", "UNREACHED"] {
        assert!(
            !expected.0.contains_key(absent),
            "{absent}: {:?}",
            expected.0
        );
    }
    for kind in [
        DiagnosticKind::Misread,
        DiagnosticKind::Unsupported,
        DiagnosticKind::Refused,
        DiagnosticKind::Unsettled,
    ] {
        assert!(
            expected.1.iter().any(|note| note.0 == kind),
            "{kind:?}: {:?}",
            expected.1
        );
    }
    assert!(
        expected
            .1
            .iter()
            .any(|(_, note)| note.contains("free_operative") && note.contains("count: 2")),
        "{:?}",
        expected.1
    );
    for workers in [1, 2, 3, 8] {
        assert_eq!(
            read_files(
                &paths(&layers),
                &names,
                workers,
                |path| layers.read(path),
                parse
            )?,
            expected
        );
    }
    assert_eq!(read(&layers)?, expected);
    Ok(())
}

#[test]
fn a_verified_focus_read_error_never_reaches_later_files() -> io::Result<()> {
    for failed in 0..5 {
        let stack = Stack::new("parallel_focus_read_error");
        for index in 0..5 {
            stack.write(
                &stack.base(),
                &format!("common/national_focus/{index}.txt"),
                &format!("shared_focus={{id=SYN_{index}}}"),
            );
        }
        let layers = stack.layers(&[]);
        let names = layers.key_names()?;
        let paths = paths(&layers);
        stack.write(
            &stack.base(),
            &paths[failed].1,
            "shared_focus={id=CHANGED_AFTER_INVENTORY}",
        );
        for workers in [1, 2, 3] {
            let mut reached = Vec::new();
            let parsed = AtomicUsize::new(0);
            let result = read_files(
                &paths,
                &names,
                workers,
                |path| {
                    reached.push(path.to_owned());
                    layers.read(path)
                },
                |bytes, database, names| {
                    parsed.fetch_add(1, Ordering::SeqCst);
                    parse(bytes, database, names)
                },
            );
            assert!(result.is_err());
            assert_eq!(
                reached,
                paths[..=failed]
                    .iter()
                    .map(|(_, path)| path.clone())
                    .collect::<Vec<_>>()
            );
            assert_eq!(parsed.load(Ordering::SeqCst), failed / workers * workers);
        }
    }
    Ok(())
}

#[test]
fn focus_wave_parsers_overlap_and_merge_after_reverse_completion() -> io::Result<()> {
    let paths = (0..4)
        .map(|index| (0, format!("common/national_focus/{index}.txt")))
        .collect::<Vec<_>>();
    let barrier = Barrier::new(2);
    let active = AtomicUsize::new(0);
    let peak = AtomicUsize::new(0);
    let threads = Mutex::new(Vec::new());
    let completion_order = Mutex::new(Vec::new());
    let second_finished = Mutex::new([false; 2]);
    let finished = Condvar::new();
    let (definitions, notes) = read_files(
        &paths,
        &KeyNames::default(),
        2,
        |path| Ok(vec![path.as_bytes()["common/national_focus/".len()] - b'0']),
        |bytes, database, names| {
            let index = usize::from(bytes[0]);
            threads.lock().unwrap().push(std::thread::current().id());
            let count = active.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(count, Ordering::SeqCst);
            barrier.wait();
            let parsed = parse(
                format!("shared_focus={{id=DUPLICATE text=TITLE_{index}}}").as_bytes(),
                database,
                names,
            );
            let mut done = second_finished.lock().unwrap();
            if index.is_multiple_of(2) {
                while !done[index / 2] {
                    done = finished.wait(done).unwrap();
                }
            } else {
                done[index / 2] = true;
                finished.notify_all();
            }
            completion_order.lock().unwrap().push(index);
            active.fetch_sub(1, Ordering::SeqCst);
            parsed
        },
    )?;
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(definitions["DUPLICATE"].localisation, "TITLE_3");
    assert_eq!(peak.load(Ordering::SeqCst), 2);
    assert_eq!(completion_order.into_inner().unwrap(), [1, 0, 3, 2]);
    let threads = threads.into_inner().unwrap();
    assert_ne!(threads[0], threads[1]);
    assert!(threads.iter().all(|id| *id != std::thread::current().id()));
    Ok(())
}
