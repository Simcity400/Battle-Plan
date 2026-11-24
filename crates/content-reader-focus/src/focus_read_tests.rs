use content_reader_key_names::key_name_products::KeyNamesExt as _;
use super::*;
use crate::{DiagnosticKind, test_support::Stack};
use std::sync::{
    Barrier, Condvar, Mutex,
    atomic::{AtomicUsize, Ordering},
};

fn paths(layers: &ResolvedLayers) -> Vec<(usize, ScriptPath)> {
    database_paths(layers, true).unwrap()
}

fn serial(
    layers: &ResolvedLayers,
    names: &KeyNames,
) -> io::Result<(Focuses, Vec<crate::Diagnostic>)> {
    let mut definitions = Focuses::default();
    let mut warnings = Vec::new();
    for (_, path) in paths(layers) {
        let database = if path.as_bytes().starts_with(b"common/national_focus/") {
            Database::National
        } else {
            Database::Continuous
        };
        match parse(&path.read(layers)?, database, names) {
            Ok((content, notes)) => {
                definitions.read_file(database, content);
                warnings.extend(
                    notes
                        .into_iter()
                        .map(|(kind, note)| (kind, format!("{path}: {note}"))),
                );
            }
            Err((kind, error)) => {
                warnings.push((
                    kind,
                    format!("Focus definitions unavailable in {path}: {error}"),
                ));
                definitions.refuse_file(database, &path.to_string(), error.to_string());
            }
        }
    }
    Ok((definitions, warnings))
}

#[test]
fn ordered_parallel_focus_files_keep_layer_path_and_database_precedence() -> io::Result<()> {
    content_reader_script::test_game::enter();
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
    assert_eq!(
        expected
            .0
            .get_in(Database::National, "SYN_DUPLICATE")
            .unwrap()
            .unwrap()
            .localisation,
        "MOD_LATE"
    );
    assert_eq!(
        expected
            .0
            .get_in(Database::Continuous, "SYN_DUPLICATE")
            .unwrap()
            .unwrap()
            .localisation,
        "CONTINUOUS"
    );
    assert_eq!(expected.0.get("SYN_DUPLICATE"), Err(Unknown::Ambiguous));
    for workers in [1, 2, 3, 8] {
        assert_eq!(
            read_files(
                &paths(&layers),
                &names,
                workers,
                |path| path.read(&layers),
                parse
            )?,
            expected,
        );
    }
    assert_eq!(read(&layers)?, expected);
    let (national, _) = read_national(&layers)?;
    assert_eq!(national.national, expected.0.national);
    assert_eq!(national.continuous, Store::default());
    Ok(())
}

#[test]
fn parallel_focus_files_isolate_variables_and_share_complete_mod_names() -> io::Result<()> {
    content_reader_script::test_game::enter();
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
        assert_eq!(expected.0.get(id).unwrap().unwrap().localisation, title);
    }
    for workers in [1, 2, 3] {
        let actual = read_files(
            &paths(&layers),
            &names,
            workers,
            |path| path.read(&layers),
            |bytes, database, shared_names| {
                assert!(std::ptr::eq(shared_names, &raw const names.names));
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
    content_reader_script::test_game::enter();
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
    let defined = &expected.0.national.definitions;
    assert_eq!(defined["RECOVERED"].localisation, "RECOVERED_TITLE");
    assert_eq!(defined["PREFIX"].localisation, "PREFIX_TITLE");
    assert!(defined.contains_key("COUNTED"));
    assert!(defined.contains_key("NATIVE_RECOVERY"));
    for absent in ["VECTOR", "UNSETTLED", "UNREACHED"] {
        assert!(!defined.contains_key(absent), "{absent}: {defined:?}");
    }
    for id in ["RECOVERED", "PREFIX", "VECTOR"] {
        assert!(
            matches!(
                expected.0.get(id),
                Err(Unknown::Gap(gap)) if gap.path == "common/national_focus/g.txt"
            ),
            "{id}"
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
                |path| path.read(&layers),
                parse
            )?,
            expected
        );
    }
    assert_eq!(read(&layers)?, expected);
    Ok(())
}

#[test]
fn a_refused_file_leaves_what_it_could_redefine_unknown() -> io::Result<()> {
    content_reader_script::test_game::enter();
    let stack = Stack::new("focus_refused_override");
    for (file, text) in [
        (
            "a",
            "shared_focus={id=SYN text=OLD} shared_focus={id=KEPT text=KEPT_OLD}",
        ),
        ("b", "shared_focus={id=SYN text=NEW} shared_focus={text=NO_ID}"),
        ("c", "shared_focus={id=KEPT text=KEPT_NEW}"),
    ] {
        stack.write(
            &stack.base(),
            &format!("common/national_focus/{file}.txt"),
            text,
        );
    }
    let layers = stack.layers(&[]);
    let (focuses, warnings) = read(&layers)?;
    assert!(
        warnings
            .iter()
            .any(|(_, warning)| warning.contains("common/national_focus/b.txt")),
        "{warnings:?}"
    );
    for id in ["SYN", "ABSENT"] {
        assert!(
            matches!(
                focuses.get(id),
                Err(Unknown::Gap(gap)) if gap.path == "common/national_focus/b.txt"
            ),
            "{id}"
        );
    }
    assert_eq!(
        focuses.get("KEPT").unwrap().unwrap().localisation,
        "KEPT_NEW"
    );
    assert_eq!(
        focuses
            .known()
            .map(|definition| definition.localisation.as_str())
            .collect::<Vec<_>>(),
        ["KEPT_NEW"]
    );
    assert_eq!(focuses.ids().collect::<Vec<_>>(), ["KEPT", "SYN"]);
    Ok(())
}

#[test]
fn a_verified_focus_read_error_never_reaches_later_files() -> io::Result<()> {
    content_reader_script::test_game::enter();
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
            &paths[failed].1.to_string(),
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
                    reached.push(path.clone());
                    path.read(&layers)
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
    content_reader_script::test_game::enter();
    let paths = (0..4)
        .map(|index| {
            (
                0,
                ScriptPath::new(format!("common/national_focus/{index}.txt").into_bytes()),
            )
        })
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
    assert_eq!(
        definitions.get("DUPLICATE").unwrap().unwrap().localisation,
        "TITLE_3"
    );
    assert_eq!(peak.load(Ordering::SeqCst), 2);
    assert_eq!(completion_order.into_inner().unwrap(), [1, 0, 3, 2]);
    let threads = threads.into_inner().unwrap();
    assert_ne!(threads[0], threads[1]);
    assert!(threads.iter().all(|id| *id != std::thread::current().id()));
    Ok(())
}

#[test]
fn focus_folders_load_whatever_case_their_layer_spells() -> io::Result<()> {
    content_reader_script::test_game::enter();
    let stack = Stack::new("focus_folder_case");
    stack.write(
        &stack.addon(),
        "Common/National_Focus/a.txt",
        "shared_focus={id=NATIONAL text=NATIONAL_TITLE}",
    );
    stack.write(
        &stack.addon(),
        "common/Continuous_Focus/b.txt",
        "continuous_focus_palette={focus={id=CONTINUOUS text=CONTINUOUS_TITLE}}",
    );
    let layers = stack.layers(&[]);
    let (focuses, warnings) = read(&layers)?;
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        focuses.get("NATIONAL").unwrap().unwrap().localisation,
        "NATIONAL_TITLE"
    );
    assert_eq!(
        focuses.get("CONTINUOUS").unwrap().unwrap().localisation,
        "CONTINUOUS_TITLE"
    );
    let (national, _) = read_national(&layers)?;
    assert!(national.get("NATIONAL").unwrap().is_some());
    assert!(national.get("CONTINUOUS").unwrap().is_none());
    Ok(())
}

#[test]
fn a_refused_file_hides_only_its_own_databases_focuses() {
    let mut focuses = Focuses::default();
    focuses.read_file(
        Database::National,
        Definitions::from([("NATIONAL".to_owned(), FocusDefinition::default())]),
    );
    let continuous = "common/continuous_focus/refused.txt";
    focuses.refuse_file(Database::Continuous, continuous, "refused".to_owned());
    assert!(
        focuses
            .get_in(Database::National, "NATIONAL")
            .is_ok_and(|found| found.is_some())
    );
    assert!(
        focuses
            .get_in(Database::National, "UNDEFINED")
            .is_ok_and(|found| found.is_none())
    );
    for id in ["NATIONAL", "UNDEFINED"] {
        assert!(
            matches!(
                focuses.get_in(Database::Continuous, id),
                Err(gap) if gap.path == continuous
            ),
            "{id}"
        );
        assert!(
            matches!(
                focuses.get(id),
                Err(Unknown::Gap(gap)) if gap.path == continuous
            ),
            "{id}"
        );
    }
    assert_eq!(focuses.known().count(), 1);
    assert_eq!(focuses.known_in(Database::National).count(), 1);
    assert_eq!(focuses.known_in(Database::Continuous).count(), 0);
    let national = "common/national_focus/refused.txt";
    focuses.refuse_file(Database::National, national, "refused".to_owned());
    assert!(matches!(
        focuses.get_in(Database::National, "NATIONAL"),
        Err(gap) if gap.path == national
    ));
    assert!(matches!(
        focuses.get("NATIONAL"),
        Err(Unknown::Gap(gap)) if gap.path == national
    ));
}

#[test]
fn each_database_keeps_its_own_definition_of_a_shared_id() {
    let titled = |title: &str| FocusDefinition {
        localisation: title.to_owned(),
        ..FocusDefinition::default()
    };
    let mut focuses = Focuses::default();
    focuses.read_file(
        Database::National,
        Definitions::from([
            ("SHARED".to_owned(), titled("NATIONAL")),
            ("SAME".to_owned(), titled("SAME")),
        ]),
    );
    focuses.read_file(
        Database::Continuous,
        Definitions::from([
            ("SHARED".to_owned(), titled("CONTINUOUS")),
            ("SAME".to_owned(), titled("SAME")),
            ("ONLY".to_owned(), titled("ONLY")),
        ]),
    );
    let title = |database, id| {
        focuses
            .get_in(database, id)
            .unwrap()
            .map(|definition| definition.localisation.as_str())
    };
    assert_eq!(title(Database::National, "SHARED"), Some("NATIONAL"));
    assert_eq!(title(Database::Continuous, "SHARED"), Some("CONTINUOUS"));
    assert_eq!(title(Database::National, "ONLY"), None);
    assert_eq!(focuses.get("SHARED"), Err(Unknown::Ambiguous));
    assert_eq!(focuses.get("SAME").unwrap().unwrap().localisation, "SAME");
    assert_eq!(focuses.get("ONLY").unwrap().unwrap().localisation, "ONLY");
    assert_eq!(
        focuses.ids().collect::<Vec<_>>(),
        ["ONLY", "SAME", "SHARED"]
    );
    assert_eq!(
        focuses.ids_in(Database::National).collect::<Vec<_>>(),
        ["SAME", "SHARED"]
    );
    assert_eq!(
        focuses.ids_in(Database::Continuous).collect::<Vec<_>>(),
        ["ONLY", "SAME", "SHARED"]
    );
    assert_eq!(focuses.known().count(), 5);
}
