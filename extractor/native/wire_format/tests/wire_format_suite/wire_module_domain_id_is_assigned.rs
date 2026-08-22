use std::fs;
use std::path::PathBuf;

#[test]
fn every_wire_module_domain_id_matches_its_filename_and_is_assigned() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut checked = Vec::new();

    for entry in fs::read_dir(&src).expect("wire_format src must be readable") {
        let path = entry.expect("src entry must be readable").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("source file stem");
        let Some(digits) = stem
            .strip_prefix('d')
            .filter(|rest| !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_digit()))
        else {
            continue;
        };
        let filename_id: u32 = digits.parse().expect("digit run parses");

        let source = fs::read_to_string(&path).expect("wire module must be readable");
        let declared = source
            .lines()
            .find_map(|line| {
                line.strip_prefix("pub const DOMAIN: u32 = ")
                    .and_then(|rest| rest.strip_suffix(';'))
                    .and_then(|value| value.trim().parse::<u32>().ok())
            })
            .unwrap_or_else(|| panic!("{stem}.rs declares no `pub const DOMAIN: u32 = N;`"));

        assert_eq!(
            declared, filename_id,
            "{stem}.rs declares DOMAIN {declared} but its filename claims {filename_id}"
        );

        let id = u16::try_from(filename_id)
            .unwrap_or_else(|_| panic!("{stem}.rs domain id {filename_id} is not a domain id"));
        let descriptor = compact_contract::assigned_descriptor_by_id(id).unwrap_or_else(|| {
            panic!("{stem}.rs claims domain {id}, which ASSIGNED_COMPACT_DOMAINS does not assign")
        });
        checked.push((id, descriptor.canonical_name));
    }

    assert!(
        checked.len() >= 40,
        "expected the full dNN module set, found only {} — the scan is not matching",
        checked.len()
    );
    assert!(
        checked
            .iter()
            .any(|(id, name)| *id == 99 && *name == "division_combat_stats_live"),
        "d99 lost its wire module or its descriptor name changed"
    );
}
