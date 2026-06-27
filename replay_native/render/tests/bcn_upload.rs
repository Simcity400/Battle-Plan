//! Integration test: the BCn direct-upload path and the CPU transcode fallback
//! both succeed and produce a non-blank texture.
//!
//! Part 1 always runs (synthetic DXT5). Part 2 runs only if a real the game install
//! is resolvable (skips gracefully otherwise so CI without the game still
//! passes), and checks that direct-BCn upload and CPU-decode of the SAME real
//! DXT5 file both render a non-empty result.

use assets::dds::{self, DdsFormat};
use assets::resolver::GameResolver;
use render::Renderer;

/// A synthetic single-block (4x4) DXT5 DDS: solid white colour, full alpha.
fn synth_dxt5_white() -> Vec<u8> {
    let mut v = vec![0u8; 128];
    v[0..4].copy_from_slice(b"DDS ");
    v[4..8].copy_from_slice(&124u32.to_le_bytes());
    v[8..12].copy_from_slice(&0x0008_1007u32.to_le_bytes());
    v[12..16].copy_from_slice(&4u32.to_le_bytes()); // height
    v[16..20].copy_from_slice(&4u32.to_le_bytes()); // width
    v[76..80].copy_from_slice(&32u32.to_le_bytes());
    v[80..84].copy_from_slice(&0x4u32.to_le_bytes()); // DDPF_FOURCC
    v[84..88].copy_from_slice(b"DXT5");
    let mut block = [0u8; 16];
    block[0] = 255; // a0
    block[1] = 0; // a1 (a0>a1 path); indices all 0 => alpha=255
    let white: u16 = 0xFFFF;
    block[8..10].copy_from_slice(&white.to_le_bytes());
    block[10..12].copy_from_slice(&white.to_le_bytes());
    v.extend_from_slice(&block);
    v
}

fn renderer() -> Option<Renderer> {
    // Some CI boxes have no GPU/adapter; skip rather than fail.
    Renderer::new(16, 16).ok()
}

#[test]
fn bcn_direct_upload_synthetic_dxt5() {
    let Some(mut r) = renderer() else {
        eprintln!("no wgpu adapter; skipping");
        return;
    };
    let dds = synth_dxt5_white();
    assert_eq!(dds::parse_header(&dds).unwrap().format, DdsFormat::Dxt5);

    let tex = r.load_dds_bytes(&dds, "synthetic-dxt5").expect("load dxt5");
    assert_eq!((tex.width, tex.height), (4, 4));
    // On a BC-capable adapter this is the direct path; otherwise CPU fallback.
    if r.bc_supported {
        assert!(
            tex.gpu_native,
            "BC-capable adapter should take the direct path"
        );
    }
    // Draw it and confirm the target is non-blank.
    r.draw_sprite(&tex, 0.0, 0.0, 16.0, 16.0);
    let px = r.finish([0.0, 0.0, 0.0, 1.0]);
    let white_px = px
        .chunks_exact(4)
        .filter(|p| p[0] > 200 && p[1] > 200 && p[2] > 200)
        .count();
    assert!(
        white_px > 100,
        "expected a white block, got {white_px} white px"
    );
}

#[test]
fn real_dxt5_both_paths_agree() {
    let Some(mut r) = renderer() else {
        return;
    };
    let resolver = GameResolver::new(None, None);
    if !resolver.vanilla_exists() {
        eprintln!("game install not resolvable; skipping real-asset comparison");
        return;
    }
    // A small block-aligned real DXT5 file.
    let Some(path) = resolver.resolve("gfx/interface/closebutton_16.dds") else {
        eprintln!("closebutton_16.dds not found; skipping");
        return;
    };
    let bytes = std::fs::read(&path).unwrap();
    let hdr = dds::parse_header(&bytes).unwrap();
    assert_eq!(hdr.format, DdsFormat::Dxt5, "expected a DXT5 test fixture");

    // CPU-decode path: prove the fallback handles a REAL BC3 file.
    let cpu = dds::decode(&bytes).expect("cpu decode real dxt5");
    assert_eq!((cpu.width, cpu.height), (hdr.width, hdr.height));
    let cpu_visible = cpu.rgba.chunks_exact(4).filter(|p| p[3] > 16).count();
    assert!(
        cpu_visible > 0,
        "real DXT5 CPU-decode produced no visible pixels"
    );

    // GPU path: upload + draw + readback, confirm non-blank.
    let tex = r.load_dds(&path).expect("gpu load real dxt5");
    r.draw_sprite(&tex, 0.0, 0.0, hdr.width as f32, hdr.height as f32);
    let out = r.finish([0.0, 0.0, 0.0, 1.0]);
    let gpu_visible = out
        .chunks_exact(4)
        .filter(|p| p[0] > 8 || p[1] > 8 || p[2] > 8)
        .count();
    assert!(gpu_visible > 0, "real DXT5 GPU render was blank");
    eprintln!(
        "real dxt5 {}x{}: gpu_native={} cpu_visible={} gpu_visible={}",
        hdr.width, hdr.height, tex.gpu_native, cpu_visible, gpu_visible
    );
}
