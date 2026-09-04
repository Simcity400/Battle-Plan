//! 2. **Glyph atlas mip chain is authored + shipped, not runtime-generated.**
//!    uploaded as-is, not regenerated. Closes the "glyph atlas mips" residual

const FONT_FNT: &str = "gfx/fonts/hoi_mapfont4.fnt";
const FONT_DDS: &str = "gfx/fonts/hoi_mapfont4.dds";

fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.split_whitespace().find_map(|tok| {
        tok.split_once('=')
            .filter(|(k, _)| *k == key)
            .map(|(_, v)| v.trim_matches('"'))
    })
}

/// (1) The advance table is the `.fnt` `xadvance`; the font ships no kerning.
#[test]
#[cfg_attr(not(gate_hoi4_install), ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path")]
fn glyph_advance_table_is_fnt_xadvance_with_no_kerning() {
    let fnt = replay_test_gates::require_vanilla_game_root().join(FONT_FNT);
    let text = std::fs::read_to_string(&fnt).expect("read shipped map font");

    // The declared char count matches the parsed `char` lines.
    let declared = text
        .lines()
        .find_map(|l| l.strip_prefix("chars count=").map(|c| c.trim()))
        .and_then(|c| c.parse::<usize>().ok())
        .expect("`.fnt` declares `chars count=`");
    let char_lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("char "))
        .collect();
    assert_eq!(
        declared,
        char_lines.len(),
        "parsed char lines must match declared count"
    );
    assert_eq!(declared, 575, "vanilla hoi_mapfont4 ships 575 glyph rects");

    // NO kerning table anywhere in the shipped font — the whole point: the
    // advance is purely additive `xadvance`, matching the closed tracking law.
    let kerning_lines = text
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("kerning") || l.starts_with("kernings"))
        .count();
    assert_eq!(
        kerning_lines, 0,
        "hoi_mapfont4.fnt must ship ZERO kerning pairs (advance = xadvance only)"
    );

    // Every glyph carries a parseable `xadvance` — this IS the advance table.
    // Space (id 32) carries a nonzero advance with a near-zero rect (the engine
    // advances the pen over the space but emits no quad).
    let advance_of = |id: u32| -> Option<i32> {
        char_lines
            .iter()
            .find(|l| field(l, "id").and_then(|v| v.parse::<u32>().ok()) == Some(id))
            .and_then(|l| field(l, "xadvance").and_then(|v| v.parse::<i32>().ok()))
    };
    let xoffset_of = |id: u32| -> Option<i32> {
        char_lines
            .iter()
            .find(|l| field(l, "id").and_then(|v| v.parse::<u32>().ok()) == Some(id))
            .and_then(|l| field(l, "xoffset").and_then(|v| v.parse::<i32>().ok()))
    };
    assert_eq!(advance_of(32), Some(20), "space xadvance = 20 font px");
    assert_eq!(advance_of(b'V' as u32), Some(47), "V xadvance = 47 font px");
    assert_eq!(advance_of(b'I' as u32), Some(33), "I xadvance = 33 font px");

    // The apparent "V->I negative kerning" residual is the per-glyph `xoffset`
    // delta, NOT a kerning term: BMFont places each quad at `pen + xoffset`, so
    // the quad-origin-to-quad-origin distance differs from the pen advance by
    // `xoffset[next] - xoffset[prev]`. For V->I that is I(1) - V(-3) = 4 font
    // px; at the GAPS_4 SOVIETUNION scale ~0.0751 wu/px that is ~0.30 wu, which
    // is the measured "<=0.4 wu, largest V->I" residual attributed to
    // kerning. There is no kerning pair for (V,I) — the font has none at all.
    let v_off = xoffset_of(b'V' as u32).expect("V xoffset");
    let i_off = xoffset_of(b'I' as u32).expect("I xoffset");
    assert_eq!(v_off, -3, "V xoffset = -3");
    assert_eq!(i_off, 1, "I xoffset = 1");
    assert_eq!(
        i_off - v_off,
        4,
        "the V->I 'kerning'-looking residual is the xoffset delta (4 font px)"
    );

    // Bound the placement-vs-advance residual family over the alphabet the
    // engine actually draws map labels in: uppercase A-Z (the 23 decoded
    // engine labels are all caps). Every A-Z xoffset lies in [-3, 3], so the
    // per-glyph placement term is <= 3 font px and any consecutive-pair delta
    // <= 6 font px (~0.45 wu at the label scale) — it is NOT a hidden kerning
    // table masquerading as advance. (Six extended/Cyrillic glyphs, ids
    // 1157-1161 etc., carry |xoffset| up to 35, but those never appear in a
    // Latin country name — reported, not asserted.)
    let max_az_xoffset = char_lines
        .iter()
        .filter(|l| {
            field(l, "id")
                .and_then(|v| v.parse::<u32>().ok())
                .map(|id| (b'A' as u32..=b'Z' as u32).contains(&id))
                .unwrap_or(false)
        })
        .filter_map(|l| field(l, "xoffset").and_then(|v| v.parse::<i32>().ok()))
        .map(i32::abs)
        .max()
        .expect("A-Z glyphs present");
    let max_all_xoffset = char_lines
        .iter()
        .filter_map(|l| field(l, "xoffset").and_then(|v| v.parse::<i32>().ok()))
        .map(i32::abs)
        .max()
        .unwrap_or(0);
    eprintln!("xoffset: max over A-Z = {max_az_xoffset} font px (whole font {max_all_xoffset})");
    assert!(
        max_az_xoffset <= 3,
        "max A-Z |xoffset| = {max_az_xoffset} font px; label placement residual must stay bounded"
    );
}

fn le_u32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

/// (2) The atlas ships a complete 4-level mip chain (fmt 87, BGRA); the engine
/// uploads it verbatim rather than runtime-generating it.
#[test]
#[cfg_attr(not(gate_hoi4_install), ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path")]
fn glyph_atlas_ships_complete_mip_chain_not_runtime_generated() {
    let dds_path = replay_test_gates::require_vanilla_game_root().join(FONT_DDS);
    let f = std::fs::read(&dds_path).expect("read shipped map font atlas");
    assert_eq!(&f[0..4], b"DDS ", "magic");

    let height = le_u32(&f, 12) as usize;
    let width = le_u32(&f, 16) as usize;
    let mip_count = le_u32(&f, 28);
    assert_eq!((width, height), (2048, 1024), "atlas page dimensions");
    assert_eq!(mip_count, 4, "hoi_mapfont4.dds ships a 4-level mip chain");

    // Pixel format = uncompressed 32-bit B8G8R8A8 (== DXGI fmt 87 UNORM, the
    // engine's frame4700 SRV binding; NAME2b: not the sRGB view).
    let pf_flags = le_u32(&f, 80);
    let fourcc = &f[84..88];
    let rgb_bit_count = le_u32(&f, 88);
    let (mask_r, mask_g, mask_b, mask_a) = (
        le_u32(&f, 92),
        le_u32(&f, 96),
        le_u32(&f, 100),
        le_u32(&f, 104),
    );
    assert_eq!(pf_flags & 0x41, 0x41, "DDPF_RGB | DDPF_ALPHAPIXELS");
    assert_eq!(
        fourcc,
        [0, 0, 0, 0],
        "uncompressed (no FourCC / DX10 header)"
    );
    assert_eq!(rgb_bit_count, 32, "32 bpp");
    assert_eq!(
        (mask_r, mask_g, mask_b, mask_a),
        (0x00ff0000, 0x0000ff00, 0x000000ff, 0xff000000),
        "BGRA channel masks (B8G8R8A8)"
    );

    // File size == 128-B header + exactly four mips of the declared chain.
    let mut expected = 128usize;
    let (mut w, mut h) = (width, height);
    let mut dims = Vec::new();
    for _ in 0..mip_count {
        expected += w * h * 4;
        dims.push((w, h));
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    assert_eq!(
        dims,
        vec![(2048, 1024), (1024, 512), (512, 256), (256, 128)],
        "mip dimension ladder"
    );
    assert_eq!(
        f.len(),
        expected,
        "file carries a complete 4-mip chain (128 + sum of mip sizes)"
    );

    // REFUTE runtime `GenerateMips`: a 2x2 box average of mip 0 in stored
    // (UNORM) space does NOT reproduce the shipped mip 1, so the engine cannot
    // be regenerating the chain with a box filter — it must upload the shipped
    // (offline higher-order authored) bytes. If a future change made the box
    // filter suddenly match, the atlas-mip assumption changed and this must be
    // revisited.
    let mip0 = &f[128..128 + 2048 * 1024 * 4];
    let mip1 = &f[128 + 2048 * 1024 * 4..128 + 2048 * 1024 * 4 + 1024 * 512 * 4];
    let (sw, dw, dh) = (2048usize, 1024usize, 512usize);
    let mut sum_abs: u64 = 0;
    let mut exact: u64 = 0;
    for y in 0..dh {
        for x in 0..dw {
            for c in 0..4 {
                let s = |yy: usize, xx: usize| mip0[((yy * sw + xx) * 4) + c] as i32;
                let avg = ((s(2 * y, 2 * x)
                    + s(2 * y, 2 * x + 1)
                    + s(2 * y + 1, 2 * x)
                    + s(2 * y + 1, 2 * x + 1)
                    + 2)
                    / 4) as u8;
                let target = mip1[((y * dw + x) * 4) + c];
                let d = (avg as i32 - target as i32).unsigned_abs() as u64;
                sum_abs += d;
                if d == 0 {
                    exact += 1;
                }
            }
        }
    }
    let n = (dw * dh * 4) as f64;
    let mean_abs = sum_abs as f64 / n;
    let exact_frac = exact as f64 / n;
    eprintln!(
        "box(mip0)->mip1: mean|d|={mean_abs:.3}/255 exact={:.1}%  (runtime box REFUTED)",
        exact_frac * 100.0
    );
    assert!(
        mean_abs > 3.0,
        "box downsample of mip0 unexpectedly reproduces shipped mip1 \
         (mean|d|={mean_abs:.3}); the shipped-not-generated premise needs review"
    );
}
