use super::*;

type Answer = Result<Vec<u8>, (io::ErrorKind, String)>;
fn answer(result: io::Result<Vec<u8>>) -> Answer {
    result.map_err(|error| (error.kind(), error.to_string()))
}

#[test]
fn warm_memo_keeps_fixed_bytes_errors_loads_reports_and_changed_stack_checks() {
    let root = std::env::temp_dir().join(format!("read-memo-exact-{}", std::process::id()));
    let base = root.join("base");
    let overlay = root.join("overlay");
    fs::create_dir_all(base.join("map")).unwrap();
    fs::create_dir_all(base.join("gfx/world")).unwrap();
    fs::create_dir_all(overlay.join("gfx/world")).unwrap();
    fs::write(base.join("map/covered.txt"), b"covered").unwrap();
    fs::write(base.join("gfx/world/fallback.mesh"), b"fallback").unwrap();
    fs::write(base.join("gfx/world/Case.mesh"), b"base case").unwrap();
    fs::write(overlay.join("gfx/world/Case.mesh"), b"higher case").unwrap();
    let archive = root.join("synthetic.zip");
    let bytes = zip_records(
        &[],
        &[
            Record {
                name: b"map/covered-zip.txt",
                host: 3,
                flags: 0,
                deflate: false,
                data: b"verified archive",
            },
            Record {
                name: b"gfx/world/stored.mesh",
                host: 3,
                flags: 0,
                deflate: false,
                data: b"stored",
            },
            Record {
                name: b"gfx/world/deflate.mesh",
                host: 3,
                flags: 0,
                deflate: true,
                data: &[7; 12000],
            },
            Record {
                name: b"gfx/world/raw-\xff.mesh",
                host: 3,
                flags: 0,
                deflate: false,
                data: b"raw name",
            },
        ],
    );
    fs::write(&archive, &bytes).unwrap();
    let encrypted = root.join("encrypted.zip");
    fs::write(
        &encrypted,
        include_bytes!("../testdata/zipcrypto_stored.zip"),
    )
    .unwrap();
    let make = || {
        let mut layers = ResolvedLayers::from_folders(&[(&base, &[]), (&overlay, &[])]).unwrap();
        layers
            .layers
            .push(archive_layer("synthetic", &archive, &[b"gfx/hidden"]));
        layers
            .layers
            .push(archive_layer("encrypted", &encrypted, &[]));
        layers.finish().unwrap();
        layers
    };
    let new = {
        let mut layers = make();
        let reports = layers.report_uncovered_reads();
        let first = LOADS.with(std::cell::Cell::get);
        let mut out = Vec::new();
        let mut loads = Vec::new();
        for _ in 0..3 {
            for path in [
                b"map/covered.txt".as_slice(),
                b"map/covered-zip.txt",
                b"gfx/world/fallback.mesh",
                b"gfx/world/Case.mesh",
                b"gfx/world/CASE.mesh",
                b"gfx/world/stored.mesh",
                b"gfx/world/deflate.mesh",
                b"gfx/world/raw-\xff.mesh",
                b"gfx/world/missing.mesh",
                b"gfx/hidden/blocked.mesh",
                b"gfx/interface/e.dds$secret",
                b"gfx/interface/e.dds$wrong",
                b"gfx//world/Case.mesh",
                b"gfx\\world\\Case.mesh",
                b"../unsafe",
                b"gfx/world/.",
            ] {
                out.push(answer(layers.read_uncovered_path(path)));
                loads.push(LOADS.with(std::cell::Cell::get) - first);
            }
        }
        // A stale hit must still rescan; a memoized miss still verifies above it.
        fs::remove_file(base.join("gfx/world/fallback.mesh")).unwrap();
        out.push(answer(layers.read_uncovered("gfx/world/fallback.mesh")));
        fs::write(base.join("gfx/world/fallback.mesh"), b"fallback").unwrap();
        fs::write(base.join("map/covered.txt"), b"changed").unwrap();
        out.push(answer(layers.read("map/covered.txt")));
        fs::write(base.join("map/covered.txt"), b"covered").unwrap();
        // A different held mount roster cannot answer from the old memo.
        let higher = layers.layers.remove(1);
        out.push(answer(layers.read_uncovered("gfx/world/Case.mesh")));
        layers.layers.insert(1, higher);
        out.push(answer(layers.read_uncovered("gfx/world/Case.mesh")));
        // Covered central-directory changes are not waived by a warm memo.
        fs::write(&archive, raw_zip(&[(b"map/changed.txt", 0, b"changed")])).unwrap();
        out.push(answer(layers.read_uncovered("gfx/world/stored.mesh")));
        fs::write(&archive, &bytes).unwrap();
        (
            out,
            loads,
            LOADS.with(std::cell::Cell::get) - first,
            reports.warning(),
            reports.overlay_warning(),
            reports.overlay_refusals(),
        )
    };
    assert_eq!(new.0.len(), 53);
    for round in 1..3 {
        assert_eq!(&new.0[round * 16..(round + 1) * 16], &new.0[..16]);
    }
    assert_eq!(new.0[0], Ok(b"covered".to_vec()));
    assert_eq!(new.0[1], Ok(b"verified archive".to_vec()));
    assert_eq!(new.0[2], Ok(b"fallback".to_vec()));
    assert_eq!(new.0[5], Ok(b"stored".to_vec()));
    assert_eq!(new.0[7], Ok(b"raw name".to_vec()));
    assert!(
        new.0[8]
            .as_ref()
            .is_err_and(|(kind, _)| *kind == io::ErrorKind::NotFound)
    );
    assert_eq!(new.0[50], Ok(b"base case".to_vec()));
    assert_eq!(new.0[51], Ok(b"higher case".to_vec()));
    assert_eq!(new.0[3], Ok(b"higher case".to_vec()));
    assert_eq!(new.0[6], Ok(vec![7; 12000]));
    assert!(new.0.iter().any(|r| {
        r.as_ref()
            .is_err_and(|(kind, _)| *kind == io::ErrorKind::NotFound)
    }));
    assert!(new.0.iter().any(|r| {
        r.as_ref()
            .is_err_and(|(_, text)| text.contains("changed since verification"))
    }));
    assert!(new.2 > 0);
    fs::remove_dir_all(root).unwrap();
}
