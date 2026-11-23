use crate::mesh::MeshProducts as _;
use content_reader_layers::content::visual::{
    apply,
    fixtures::{Machine, never, pick},
};
use content_reader_layers::content::{
    Generation, ReadOutcome,
    fixtures::{overlaid_stack, overlay_folders},
};
use sha2::{Digest, Sha256};
use std::{fs, io, sync::Arc};

#[test]
fn a_recorded_descriptor_read_for_a_mods_unparsed_one_is_traced() {
    use std::sync::{Arc, Mutex};
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_descriptor_fallback_trace_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let recorded: &[u8] = br#"entity = { name = "plane" pdxmesh = "plane_mesh" }"#;
    let presentation = overlaid_stack(
        &root,
        &[("gfx/entities/a.asset", recorded)],
        &[(
            "gfx/entities/a.asset",
            br#"entity = { name = "plane" zeppelin } }{ = ="#,
        )],
        &[],
    );
    let reads = Arc::new(Mutex::new(Vec::new()));
    let log = reads.clone();
    let traced = presentation
        .with_read_trace(Arc::new(move |read| log.lock().unwrap().push(read)))
        .with_read_purpose("artwork");
    let roster = crate::mesh::Roster::read(&traced).unwrap();
    assert!(roster.descriptors.entities.contains_key("plane"));
    assert!(roster.unreadable.is_empty(), "{:?}", roster.unreadable);
    let reads = reads.lock().unwrap();
    let fallback: Vec<_> = reads
        .iter()
        .filter(|read| read.path == b"gfx/entities/a.asset" && read.layer == Some(0))
        .collect();
    assert_eq!(fallback.len(), 1, "{reads:?}");
    assert_eq!(fallback[0].purpose, "artwork");
    assert_eq!(
        fallback[0].outcome,
        ReadOutcome::Bytes {
            length: recorded.len() as u64,
            digest: Sha256::digest(recorded).into(),
        }
    );
    drop(reads);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn shared_products_replay_notes_to_each_documents_own_reports() {
    let root = std::env::temp_dir().join(format!("product-reports-{}", std::process::id()));
    let base = root.join("base");
    let visual = root.join("visual");
    fs::create_dir_all(base.join("gfx/world")).unwrap();
    fs::create_dir_all(visual.join("gfx/world")).unwrap();
    fs::write(base.join("gfx/world/base.mesh"), b"base").unwrap();
    fs::write(visual.join("gfx/world/visual.mesh"), b"visual").unwrap();
    let _generation = Generation::enter();
    let source = overlay_folders(&base, &visual, "Selected art", false);
    let first = source.with_own_overlay_diagnostics();
    let first_reports = first.report_uncovered_reads();
    let value = first
        .product("synthetic report product", 1, &[], |reader| {
            reader.read_uncovered("gfx/world/base.mesh")?;
            reader.read_uncovered("gfx/world/visual.mesh")?;
            reader.note_refused("synthetic model".into());
            reader.note_overlay_refusal("synthetic fallback".into());
            Ok(7_u32)
        })
        .unwrap();
    let second = source.with_own_overlay_diagnostics();
    let second_reports = second.report_uncovered_reads();
    let shared = second
        .product("synthetic report product", 1, &[], |_| -> io::Result<u32> {
            panic!("the second document must use the shared product");
        })
        .unwrap();
    assert!(Arc::ptr_eq(&value, &shared));
    assert_eq!(first_reports.warning(), second_reports.warning());
    assert!(
        second_reports
            .warning()
            .unwrap()
            .contains("gfx/world/base.mesh")
    );
    assert_eq!(
        first_reports.overlay_warning(),
        second_reports.overlay_warning()
    );
    assert_eq!(
        first_reports.overlay_refusals(),
        second_reports.overlay_refusals()
    );
    assert_eq!(first.refused(), second.refused());
    assert!(source.read_warnings().overlay_warning().is_none());
    assert!(source.refused().is_empty());
    let recorded = source
        .without_overlays()
        .unwrap()
        .with_own_overlay_diagnostics();
    let reports = recorded.report_uncovered_reads();
    recorded.read_uncovered("gfx/world/base.mesh").unwrap();
    assert!(reports.overlay_warning().is_none());
    assert!(reports.overlay_refusals().is_none());
    assert!(recorded.refused().is_empty());
    assert!(crate::mesh::parse_in(&recorded, "gfx/world/missing.mesh").is_err());
    assert!(
        reports
            .warning()
            .unwrap()
            .contains("gfx/world/missing.mesh")
    );
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn a_mods_descriptor_that_does_not_parse_reads_the_recordings_file() {
    let machine = Machine::new("descriptor-fallback");
    machine.write(
        "base/gfx/entities/a.asset",
        br#"entity = { name = "plane" pdxmesh = "plane_mesh" }"#,
    );
    machine.local("art.mod", "name=\"Art\"", "art");
    machine.write(
        "art/gfx/entities/a.asset",
        br#"entity = { name = "plane" zeppelin } }{ = ="#,
    );
    let mut layers = machine.recorded(&["base"]);
    let reads = layers.report_uncovered_reads();
    apply(
        &mut layers,
        &machine.options,
        &[pick("mod/art.mod", "Art")],
        &[],
        &never(),
    )
    .unwrap();
    let roster = layers.presentation().descriptor_roster().unwrap();
    assert!(roster.descriptors.entities.contains_key("plane"));
    assert!(roster.unreadable.is_empty(), "{:?}", roster.unreadable);
    assert!(roster.unreadable_names.contains_key("zeppelin"));
    assert!(!roster.unreadable_names.contains_key("plane"));
    let refusals = reads.overlay_refusals().unwrap();
    assert!(
        refusals.contains("descriptor gfx/entities/a.asset ("),
        "{refusals}"
    );
}
