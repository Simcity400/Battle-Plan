use std::fs;
use std::path::{Path, PathBuf};

const MANIFEST_ALLOW_LIST: &str =
    "# production-compact-authority-allow=d54:battleplan_links,d81:rail_replay,compact_status_unknown_flags_v1,compact_status_is_failure";

const ADMITTED_PRODUCTION_LINES: [(&str, usize); 4] = [
    ("use compact_contract::rail_replay as compact;", 1),
    ("use compact_contract::battleplan_links as links;", 1),
    (
        "let unknown = compact_contract::compact_status_unknown_flags_v1(decoded.status_flags);",
        1,
    ),
    (
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

    let mut counts = vec![0usize; ADMITTED_PRODUCTION_LINES.len()];
    let mut violations = Vec::new();

    for path in files {
        let relative = path
            .strip_prefix(&manifest_dir)
            .expect("collected source must remain below manifest directory");
        let source = fs::read_to_string(&path).expect("Rust library source must be readable");
        for (line_index, line) in source.lines().enumerate() {
            let code = line.split_once("//").map_or(line, |(code, _)| code);
            if !code.contains("compact_contract") {
                continue;
            }

            if relative != Path::new("src").join("d81.rs")
                && relative != Path::new("src").join("d54.rs") {
                violations.push(format!(
                    "{}:{}: compact_contract is not admitted in this library module",
                    relative.display(),
                    line_index + 1
                ));
                continue;
            }

            let code = code.trim();
            let mut admitted = false;
            for (index, (allowed_line, _)) in ADMITTED_PRODUCTION_LINES.iter().enumerate() {
                if code == *allowed_line {
                    counts[index] += 1;
                    admitted = true;
                }
            }
            if !admitted {
                violations.push(format!(
                    "{}:{}: compact_contract symbol is outside the reviewed d81 allow-list",
                    relative.display(),
                    line_index + 1
                ));
            }
        }
    }

    for ((line, expected), actual) in ADMITTED_PRODUCTION_LINES.iter().zip(counts) {
        assert_eq!(
            actual, *expected,
            "reviewed production compact authority occurrence count changed for `{line}`"
        );
    }
    assert!(
        violations.is_empty(),
        "unreviewed production compact_contract dependency use:\n{}",
        violations.join("\n")
    );
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
