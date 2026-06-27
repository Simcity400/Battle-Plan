//! Smoke test: the CPU fallback decodes the REAL rare-format files the M1 audit
//! found in the game install (16-bit RGB555 portrait + RXGB normal swizzle).
//! Skips gracefully if no install is resolvable (CI without the game).

use assets::dds;
use assets::resolver::GameResolver;

#[test]
fn real_rgb555_and_rxgb_decode() {
    let r = GameResolver::new(None, None);
    if !r.vanilla_exists() {
        eprintln!("no game install; skipping real-edge-format decode");
        return;
    }

    if let Some(p) = r
        .resolve("gfx/interface/portraits/portraits_Africa/Portrait_Africa_Generic_1.dds")
        .or_else(|| find_any(&r, "Portrait_Africa_Generic_1.dds"))
    {
        let b = std::fs::read(&p).unwrap();
        let h = dds::parse_header(&b).unwrap();
        assert_eq!(
            h.format,
            dds::DdsFormat::Rgb555,
            "expected a 16-bit RGB555 portrait"
        );
        let img = dds::decode(&b).expect("rgb555 decode");
        assert_eq!((img.width, img.height), (h.width, h.height));
        let visible = img
            .rgba
            .chunks_exact(4)
            .filter(|p| p[0] > 4 || p[1] > 4 || p[2] > 4)
            .count();
        assert!(visible > 100, "rgb555 portrait decoded blank");
        eprintln!("RGB555 {}x{} visible={}", h.width, h.height, visible);
    } else {
        eprintln!("RGB555 portrait not found; skipping that half");
    }

    if let Some(p) = r
        .resolve("gfx/interface/swe_rearnament.dds")
        .or_else(|| find_any(&r, "swe_rearnament.dds"))
    {
        let b = std::fs::read(&p).unwrap();
        let h = dds::parse_header(&b).unwrap();
        assert_eq!(h.format, dds::DdsFormat::Rxgb);
        let img = dds::decode(&b).expect("rxgb decode");
        let visible = img
            .rgba
            .chunks_exact(4)
            .filter(|p| p[0] > 4 || p[1] > 4 || p[2] > 4)
            .count();
        assert!(visible > 100, "rxgb decoded blank");
        eprintln!("RXGB {}x{} visible={}", h.width, h.height, visible);
    } else {
        eprintln!("RXGB file not found; skipping that half");
    }
}

/// Recursive search under the vanilla gfx/ tree for a basename.
fn find_any(r: &GameResolver, name: &str) -> Option<std::path::PathBuf> {
    fn walk(dir: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
        for e in std::fs::read_dir(dir).ok()?.flatten() {
            let p = e.path();
            if p.is_dir() {
                if let Some(f) = walk(&p, name) {
                    return Some(f);
                }
            } else if p.file_name().and_then(|s| s.to_str()) == Some(name) {
                return Some(p);
            }
        }
        None
    }
    walk(&r.vanilla.join("gfx"), name)
}
