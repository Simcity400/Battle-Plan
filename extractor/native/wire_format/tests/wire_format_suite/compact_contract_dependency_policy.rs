use std::fs;
use std::path::{Path, PathBuf};

const MANIFEST_ALLOW_LIST: &str =
    "# production-compact-authority-allow=d26:division_equipment,d54:battleplan_links,d54:owner_colour,d81:rail_replay,compact_status_unknown_flags_v1,compact_status_is_failure";

const ADMITTED_PRODUCTION_LINES: [(&str, &str, usize); 6] = [
    (
        "src/d26.rs",
        "use compact_contract::division_equipment as allocation;",
        1,
    ),
    (
        "src/d54.rs",
        "use compact_contract::battleplan_links as links;",
        1,
    ),
    (
        "src/d54.rs",
        "use compact_contract::owner_colour::{self, OwnerColourRow};",
        1,
    ),
    ("src/d81.rs", "use compact_contract::rail_replay as compact;", 1),
    (
        "src/d81.rs",
        "let unknown = compact_contract::compact_status_unknown_flags_v1(decoded.status_flags);",
        1,
    ),
    (
        "src/d81.rs",
        "if compact_contract::compact_status_is_failure(decoded.status_flags) {",
        1,
    ),
];

#[test]
fn production_compact_contract_dependency_is_exactly_allow_listed() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest = fs::read_to_string(manifest_dir.join("Cargo.toml"))
        .expect("wire_format Cargo.toml must be readable");
    assert!(
        manifest.lines().any(|line| line == MANIFEST_ALLOW_LIST),
        "Cargo.toml must carry the exact reviewed production compact authority allow-list"
    );

    let src = manifest_dir.join("src");
    let mut files = Vec::new();
    collect_library_sources(&src, &mut files);
    files.sort();

    let sources: Vec<_> = files
        .into_iter()
        .map(|path| {
            let relative = path
                .strip_prefix(&manifest_dir)
                .expect("collected source must remain below manifest directory")
                .to_path_buf();
            let source = fs::read_to_string(&path).expect("Rust library source must be readable");
            (relative, source)
        })
        .collect();
    let violations = dependency_violations(&sources);
    assert!(
        violations.is_empty(),
        "unreviewed production compact_contract dependency use:\n{}",
        violations.join("\n")
    );

    for module in ["src/d26.rs", "src/d54.rs", "src/d81.rs"] {
        for extra in [
            "use compact_contract::UnreviewedAuthority;",
            "let extra = compact_contract::unreviewed_predicate();",
        ] {
            let mut mutated = sources.clone();
            let (_, source) = mutated
                .iter_mut()
                .find(|(path, _)| path.as_path() == Path::new(module))
                .expect("admitted module must exist");
            source.push_str(&format!("\n{extra}\n"));
            let violations = dependency_violations(&mutated);
            assert_eq!(violations.len(), 1, "{module}: {extra}");
            assert!(violations[0].contains("outside the reviewed module/line allow-list"));
        }
    }
    for (module, line, _) in ADMITTED_PRODUCTION_LINES {
        let mut duplicated = sources.clone();
        let (_, source) = duplicated
            .iter_mut()
            .find(|(path, _)| path.as_path() == Path::new(module))
            .expect("admitted module must exist");
        source.push_str(&format!("\n{line}\n"));
        let violations = dependency_violations(&duplicated);
        assert_eq!(violations.len(), 1, "duplicate {module}: {line}");
        assert!(violations[0].contains("occurrence count changed"));

        let mut relocated = sources.clone();
        let (_, source) = relocated
            .iter_mut()
            .find(|(path, _)| path.as_path() == Path::new(module))
            .expect("admitted module must exist");
        *source = source
            .lines()
            .filter(|candidate| candidate.trim() != line)
            .collect::<Vec<_>>()
            .join("\n");
        let wrong_module = if module == "src/d26.rs" {
            "src/d54.rs"
        } else {
            "src/d26.rs"
        };
        let (_, source) = relocated
            .iter_mut()
            .find(|(path, _)| path.as_path() == Path::new(wrong_module))
            .expect("other admitted module must exist");
        source.push_str(&format!("\n{line}\n"));
        let violations = dependency_violations(&relocated);
        assert_eq!(violations.len(), 2, "relocated {module}: {line}");
        assert!(violations
            .iter()
            .any(|v| v.contains("occurrence count changed")));
        assert!(violations
            .iter()
            .any(|v| v.contains("outside the reviewed module/line allow-list")));
    }
}

fn dependency_violations(sources: &[(PathBuf, String)]) -> Vec<String> {
    let mut counts = vec![0usize; ADMITTED_PRODUCTION_LINES.len()];
    let mut violations = Vec::new();
    for (relative, source) in sources {
        for (line_index, line) in source.lines().enumerate() {
            let code = line.split_once("//").map_or(line, |(code, _)| code);
            if !code.contains("compact_contract") {
                continue;
            }

            let code = code.trim();
            let mut admitted = false;
            for (index, (allowed_path, allowed_line, _)) in
                ADMITTED_PRODUCTION_LINES.iter().enumerate()
            {
                if relative.as_path() == Path::new(allowed_path) && code == *allowed_line {
                    counts[index] += 1;
                    admitted = true;
                }
            }
            if !admitted {
                violations.push(format!(
                    "{}:{}: compact_contract symbol is outside the reviewed module/line allow-list",
                    relative.display(),
                    line_index + 1
                ));
            }
        }
    }
    for ((path, line, expected), actual) in ADMITTED_PRODUCTION_LINES.iter().zip(counts) {
        if actual != *expected {
            violations.push(format!(
                "reviewed production compact authority occurrence count changed for `{path}: {line}`: expected {expected}, got {actual}"
            ));
        }
    }
    violations
}

fn collect_library_sources(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("wire_format src directory must be readable") {
        let entry = entry.expect("wire_format src entry must be readable");
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().and_then(|name| name.to_str()) != Some("bin") {
                collect_library_sources(&path, files);
            }
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
            files.push(path);
        }
    }
}
