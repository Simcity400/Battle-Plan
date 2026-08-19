
use crate::mod_resolver::GameFileResolver;
use crate::types::MapFrameTextureBinding;
use crate::Result;

const DDPF_ALPHAPIXELS: u32 = 0x1;
const DDPF_FOURCC: u32 = 0x4;
const DDPF_RGB: u32 = 0x40;

const DDSCAPS2_CUBEMAP_ALLFACES: u32 = 0xfe00;
const DDSCAPS2_VOLUME: u32 = 0x200000;

const DDS_HEADER_LEN: usize = 128;
const CUBE_FACES: u32 = 6;
const BYTES_PER_TEXEL: usize = 4;

pub(crate) const REFLECTION_CUBE_DDS_SOURCE: &str = "map/terrain/reflection.dds";

#[derive(Debug)]
pub(crate) struct DdsCube {
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) mips: usize,
    pub(crate) faces: usize,
    pub(crate) payload: Vec<u8>,
}

fn u32le(data: &[u8], off: usize) -> Result<u32> {
    let slice = data
        .get(off..off + 4)
        .ok_or_else(|| format!("reflection.dds: truncated reading u32 at offset {off}"))?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

pub(crate) fn parse_reflection_dds(data: &[u8], path: &str) -> Result<DdsCube> {
    if data.get(0..4) != Some(b"DDS ") {
        return Err(format!("{path}: not a DDS file (missing 'DDS ' magic)").into());
    }
    if data.len() < DDS_HEADER_LEN {
        return Err(format!("{path}: truncated DDS (<{DDS_HEADER_LEN} B header)").into());
    }

    let height = u32le(data, 12)? as usize;
    let width = u32le(data, 16)? as usize;
    let mips = (u32le(data, 28)?.max(1)) as usize;
    let pf_flags = u32le(data, 80)?;
    let fourcc = &data[84..88];
    let rgb_bits = u32le(data, 88)? as usize;
    let rmask = u32le(data, 92)?;
    let gmask = u32le(data, 96)?;
    let bmask = u32le(data, 100)?;
    let amask = u32le(data, 104)?;
    let caps2 = u32le(data, 112)?;

    if fourcc == b"DX10" {
        return Err(format!(
            "{path}: reflection.dds uses a DX10 extended header — refusing to resolver-decode \
             (not the legacy uncompressed BGRA cube the captured payload records)"
        )
        .into());
    }
    if pf_flags & DDPF_FOURCC != 0 || fourcc.iter().any(|&b| b != 0) {
        return Err(format!(
            "{path}: reflection.dds is compressed/fourcc-tagged (pf_flags={pf_flags:#x}, \
             fourcc={:?}) — refusing to resolver-decode a non-uncompressed cube (would not \
             match the captured mapped-subresource payload byte-for-byte)",
            String::from_utf8_lossy(fourcc)
        )
        .into());
    }
    if pf_flags & DDPF_RGB == 0 || pf_flags & DDPF_ALPHAPIXELS == 0 || rgb_bits != 32 {
        return Err(format!(
            "{path}: expected uncompressed 32bpp RGBA DDS (DDPF_RGB|DDPF_ALPHAPIXELS), got \
             rgb_bits={rgb_bits} pf_flags={pf_flags:#x}"
        )
        .into());
    }
    // GUARD 3: BGRA channel masks (B in the low byte). This is the memory order
    if (rmask, gmask, bmask, amask) != (0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 0xff00_0000) {
        return Err(format!(
            "{path}: unexpected DDS channel masks R={rmask:#010x} G={gmask:#010x} \
             B={bmask:#010x} A={amask:#010x} (expected B8G8R8A8)"
        )
        .into());
    }
    if caps2 & DDSCAPS2_CUBEMAP_ALLFACES != DDSCAPS2_CUBEMAP_ALLFACES {
        return Err(format!("{path}: caps2 {caps2:#x} is not a 6-face cubemap").into());
    }
    if caps2 & DDSCAPS2_VOLUME != 0 {
        return Err(format!("{path}: caps2 {caps2:#x} marks a volume texture").into());
    }
    if width == 0 || height == 0 || width != height {
        return Err(
            format!("{path}: cube faces must be square nonzero, got {width}x{height}").into(),
        );
    }

    let faces = CUBE_FACES as usize;
    let face_tight = face_tight_bytes(width, height, mips)?;
    let expected_payload = face_tight
        .checked_mul(faces)
        .ok_or_else(|| format!("{path}: face payload size overflow"))?;
    let payload = &data[DDS_HEADER_LEN..];
    if payload.len() != expected_payload {
        return Err(format!(
            "{path}: reflection.dds payload {} B != {} (faces*face_tight for {width}x{height} \
             mips {mips})",
            payload.len(),
            expected_payload
        )
        .into());
    }

    Ok(DdsCube {
        width,
        height,
        mips,
        faces,
        payload: payload.to_vec(),
    })
}

fn face_tight_bytes(width: usize, height: usize, mips: usize) -> Result<usize> {
    let mut total: usize = 0;
    for level in 0..mips {
        let (w, h) = mip_dim(width, height, level);
        let mip_bytes = w
            .checked_mul(h)
            .and_then(|texels| texels.checked_mul(BYTES_PER_TEXEL))
            .ok_or("reflection.dds: mip byte size overflow")?;
        total = total
            .checked_add(mip_bytes)
            .ok_or("reflection.dds: face byte size overflow")?;
    }
    Ok(total)
}

fn mip_dim(width: usize, height: usize, level: usize) -> (usize, usize) {
    let shift = |dim: usize| (dim.checked_shr(level as u32).unwrap_or(0)).max(1);
    (shift(width), shift(height))
}

fn repack_to_captured_layout(cube: &DdsCube, resource: &MapFrameTextureBinding) -> Result<Vec<u8>> {
    if resource.width as usize != cube.width
        || resource.height as usize != cube.height
        || resource.mip_count as usize != cube.mips
    {
        return Err(format!(
            "reflection.dds {}x{} mips {} does not match captured binding {}x{} mips {}",
            cube.width, cube.height, cube.mips, resource.width, resource.height, resource.mip_count
        )
        .into());
    }
    if resource.array_size as usize != cube.faces {
        return Err(format!(
            "captured reflection cube array_size {} != {} cube faces",
            resource.array_size, cube.faces
        )
        .into());
    }

    let subs = &resource.subresources;
    let expected_subs = cube
        .faces
        .checked_mul(cube.mips)
        .ok_or("reflection cube subresource count overflow")?;
    if subs.len() != expected_subs {
        return Err(format!(
            "captured reflection cube has {} subresources, expected faces*mips = {}",
            subs.len(),
            expected_subs
        )
        .into());
    }

    let face_tight = face_tight_bytes(cube.width, cube.height, cube.mips)?;

    let mut out = vec![0u8; resource.byte_len];
    for face in 0..cube.faces {
        let face_src = face
            .checked_mul(face_tight)
            .ok_or("reflection cube face source offset overflow")?;
        let mut src_mip_off = 0usize;
        for mip in 0..cube.mips {
            let (w, h) = mip_dim(cube.width, cube.height, mip);
            let src_row_bytes = w
                .checked_mul(BYTES_PER_TEXEL)
                .ok_or("reflection cube source row byte overflow")?;
            let k = face
                .checked_mul(cube.mips)
                .and_then(|base| base.checked_add(mip))
                .ok_or("reflection cube subresource index overflow")?;
            let sub = &subs[k];

            if sub.array_slice as usize != face || sub.mip_slice as usize != mip {
                return Err(format!(
                    "reflection cube subresource {k} has array_slice={} mip_slice={}, expected \
                     face={face} mip={mip}",
                    sub.array_slice, sub.mip_slice
                )
                .into());
            }
            if sub.width as usize != w || sub.height as usize != h {
                return Err(format!(
                    "reflection cube subresource {k} dims {}x{} != mip {mip} dims {w}x{h}",
                    sub.width, sub.height
                )
                .into());
            }

            let dst_row_pitch = sub.row_pitch as usize;
            if src_row_bytes > dst_row_pitch {
                return Err(format!(
                    "reflection cube subresource {k}: tight row {src_row_bytes} > captured \
                     row_pitch {dst_row_pitch}"
                )
                .into());
            }
            let sub_needed = h
                .checked_sub(1)
                .and_then(|last_row| last_row.checked_mul(dst_row_pitch))
                .and_then(|last_off| last_off.checked_add(src_row_bytes))
                .ok_or("reflection cube subresource required byte size overflow")?;
            if sub.byte_len < sub_needed {
                return Err(format!(
                    "reflection cube subresource {k} byte_len {} < required {sub_needed} \
                     ({h} rows @ pitch {dst_row_pitch})",
                    sub.byte_len
                )
                .into());
            }

            for row in 0..h {
                let src = face_src
                    .checked_add(src_mip_off)
                    .and_then(|base| base.checked_add(row.checked_mul(src_row_bytes)?))
                    .ok_or("reflection cube source row offset overflow")?;
                let src_end = src
                    .checked_add(src_row_bytes)
                    .ok_or("reflection cube source row end overflow")?;
                let dst = sub
                    .file_offset
                    .checked_add(
                        row.checked_mul(dst_row_pitch)
                            .ok_or("reflection cube destination row offset overflow")?,
                    )
                    .ok_or("reflection cube destination offset overflow")?;
                let dst_end = dst
                    .checked_add(src_row_bytes)
                    .ok_or("reflection cube destination row end overflow")?;
                if src_end > cube.payload.len() {
                    return Err(format!(
                        "reflection cube subresource {k} row {row} source range {src}..{src_end} \
                         exceeds DDS payload {}",
                        cube.payload.len()
                    )
                    .into());
                }
                if dst_end > out.len() {
                    return Err(format!(
                        "reflection cube subresource {k} row {row} destination range \
                         {dst}..{dst_end} exceeds captured byte_len {}",
                        out.len()
                    )
                    .into());
                }
                out[dst..dst_end].copy_from_slice(&cube.payload[src..src_end]);
            }
            src_mip_off = src_mip_off
                .checked_add(
                    src_row_bytes
                        .checked_mul(h)
                        .ok_or("reflection cube mip source size overflow")?,
                )
                .ok_or("reflection cube mip source offset overflow")?;
        }
    }
    Ok(out)
}

pub(crate) fn decode_reflection_cube_payload(
    resolver: &GameFileResolver,
    resource: &MapFrameTextureBinding,
) -> Result<Vec<u8>> {
    let path = resolver
        .resolve_required(REFLECTION_CUBE_DDS_SOURCE, "water reflection cubemap DDS")
        .map_err(|e| format!("resolve {REFLECTION_CUBE_DDS_SOURCE}: {e}"))?;
    let data = std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let cube = parse_reflection_dds(&data, &path.to_string_lossy())?;
    repack_to_captured_layout(&cube, resource)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::MapFrameTextureSubresource;

    fn vanilla_reflection_binding(size: u32, mips: u32) -> MapFrameTextureBinding {
        let align = |x: usize, a: usize| (x + (a - 1)) / a * a;
        let mut subresources = Vec::new();
        let mut offset = 0usize;
        for face in 0..CUBE_FACES {
            for mip in 0..mips {
                let w = (size >> mip).max(1);
                let h = (size >> mip).max(1);
                let row_pitch = align(w as usize * BYTES_PER_TEXEL, 128);
                let depth_pitch = row_pitch * h as usize;
                subresources.push(MapFrameTextureSubresource {
                    mip_slice: mip,
                    array_slice: face,
                    file_offset: offset,
                    byte_len: depth_pitch,
                    row_pitch: row_pitch as u32,
                    depth_pitch: depth_pitch as u32,
                    width: w,
                    height: h,
                });
                offset += depth_pitch;
            }
        }
        MapFrameTextureBinding {
            stage: "ps".into(),
            slot: 8,
            declared_name: Some("ReflectionCubeMap".into()),
            is_null: false,
            view_desc: None,
            file: None,
            byte_len: offset,
            fnv1a64: None,
            dxgi_format: 91,
            width: size,
            height: size,
            mip_count: mips,
            array_size: CUBE_FACES,
            sample_count: 1,
            sample_quality: 0,
            payload_format: Some("d3d11_mapped_subresources".into()),
            subresources,
            bind_flags: 0,
            usage: 0,
            cpu_access_flags: 0,
            misc_flags: 4,
        }
    }

    fn synth_reflection_dds(size: u32, mips: u32) -> Vec<u8> {
        let mut hdr = vec![0u8; DDS_HEADER_LEN];
        hdr[0..4].copy_from_slice(b"DDS ");
        let put =
            |h: &mut [u8], off: usize, v: u32| h[off..off + 4].copy_from_slice(&v.to_le_bytes());
        put(&mut hdr, 4, 124);
        put(&mut hdr, 8, 0x0002_100f);
        put(&mut hdr, 12, size);
        put(&mut hdr, 16, size);
        put(&mut hdr, 28, mips);
        put(&mut hdr, 76, 32);
        put(&mut hdr, 80, DDPF_RGB | DDPF_ALPHAPIXELS);
        put(&mut hdr, 88, 32);
        put(&mut hdr, 92, 0x00ff_0000);
        put(&mut hdr, 96, 0x0000_ff00);
        put(&mut hdr, 100, 0x0000_00ff);
        put(&mut hdr, 104, 0xff00_0000);
        put(&mut hdr, 112, DDSCAPS2_CUBEMAP_ALLFACES);

        let face_tight = face_tight_bytes(size as usize, size as usize, mips as usize).unwrap();
        let mut payload = vec![0u8; face_tight * CUBE_FACES as usize];
        for (i, b) in payload.iter_mut().enumerate() {
            *b = (i as u32).wrapping_mul(2654435761).to_le_bytes()[0];
        }
        let mut out = hdr;
        out.extend_from_slice(&payload);
        out
    }

    fn reference_repack(dds: &[u8], resource: &MapFrameTextureBinding) -> Vec<u8> {
        let size = resource.width as usize;
        let mips = resource.mip_count as usize;
        let align = |x: usize, a: usize| (x + (a - 1)) / a * a;
        let payload = &dds[DDS_HEADER_LEN..];
        let face_tight = face_tight_bytes(size, size, mips).unwrap();
        let mut out = vec![0u8; resource.byte_len];
        for face in 0..CUBE_FACES as usize {
            let mut src_off = face * face_tight;
            for mip in 0..mips {
                let (w, h) = mip_dim(size, size, mip);
                let row = w * BYTES_PER_TEXEL;
                let rp = align(row, 128);
                let sub = &resource.subresources[face * mips + mip];
                for r in 0..h {
                    let s = src_off + r * row;
                    let d = sub.file_offset + r * rp;
                    out[d..d + row].copy_from_slice(&payload[s..s + row]);
                }
                src_off += row * h;
            }
        }
        out
    }

    #[test]
    fn repack_matches_independent_reference() {
        for (size, mips) in [(256u32, 9u32), (8, 4)] {
            let binding = vanilla_reflection_binding(size, mips);
            let dds = synth_reflection_dds(size, mips);
            let cube = parse_reflection_dds(&dds, "synthetic.dds").unwrap();
            let repacked = repack_to_captured_layout(&cube, &binding).unwrap();
            let reference = reference_repack(&dds, &binding);
            assert_eq!(repacked.len(), binding.byte_len);
            assert_eq!(
                repacked, reference,
                "repack must equal the independent tight->padded reference for {size}x{size}x{mips}"
            );
            for face in 0..CUBE_FACES as usize {
                for mip in 0..mips as usize {
                    let (w, h) = mip_dim(size as usize, size as usize, mip);
                    let sub = &binding.subresources[face * mips as usize + mip];
                    let row = w * BYTES_PER_TEXEL;
                    let rp = sub.row_pitch as usize;
                    for r in 0..h {
                        let base = sub.file_offset + r * rp;
                        assert!(
                            repacked[base + row..base + rp].iter().all(|&b| b == 0),
                            "row pad must be zero"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn rejects_dx10_header() {
        let mut dds = synth_reflection_dds(8, 4);
        dds[84..88].copy_from_slice(b"DX10");
        let err = parse_reflection_dds(&dds, "dx10.dds")
            .unwrap_err()
            .to_string();
        assert!(err.contains("DX10"), "got: {err}");
    }

    #[test]
    fn rejects_compressed_fourcc() {
        let mut dds = synth_reflection_dds(8, 4);
        dds[84..88].copy_from_slice(b"DXT5");
        dds[80..84].copy_from_slice(&DDPF_FOURCC.to_le_bytes());
        let err = parse_reflection_dds(&dds, "dxt5.dds")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("compressed") || err.contains("fourcc"),
            "got: {err}"
        );
    }

    #[test]
    fn rejects_non_bgra_masks() {
        let mut dds = synth_reflection_dds(8, 4);
        dds[92..96].copy_from_slice(&0x0000_00ffu32.to_le_bytes());
        dds[100..104].copy_from_slice(&0x00ff_0000u32.to_le_bytes());
        let err = parse_reflection_dds(&dds, "rgba.dds")
            .unwrap_err()
            .to_string();
        assert!(err.contains("channel masks"), "got: {err}");
    }

    #[test]
    fn rejects_non_cube() {
        let mut dds = synth_reflection_dds(8, 4);
        dds[112..116].copy_from_slice(&0u32.to_le_bytes());
        let err = parse_reflection_dds(&dds, "flat.dds")
            .unwrap_err()
            .to_string();
        assert!(err.contains("6-face cubemap"), "got: {err}");
    }

    #[test]
    fn rejects_volume_texture() {
        let mut dds = synth_reflection_dds(8, 4);
        dds[112..116].copy_from_slice(&(DDSCAPS2_CUBEMAP_ALLFACES | DDSCAPS2_VOLUME).to_le_bytes());
        let err = parse_reflection_dds(&dds, "vol.dds")
            .unwrap_err()
            .to_string();
        assert!(err.contains("volume"), "got: {err}");
    }

    #[test]
    fn rejects_mip_count_mismatch_against_binding() {
        let binding = vanilla_reflection_binding(8, 9);
        let dds = synth_reflection_dds(8, 4);
        let cube = parse_reflection_dds(&dds, "synthetic.dds").unwrap();
        let err = repack_to_captured_layout(&cube, &binding)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("does not match captured binding"),
            "got: {err}"
        );
    }

    #[test]
    fn hostile_mip_count_fails_closed_without_panic() {
        let mut dds = synth_reflection_dds(8, 4);
        dds[28..32].copy_from_slice(&200u32.to_le_bytes());
        let err = parse_reflection_dds(&dds, "hostile.dds")
            .unwrap_err()
            .to_string();
        assert!(err.contains("payload"), "got: {err}");
        assert_eq!(mip_dim(256, 256, 999), (1, 1));
    }

    fn fnv1a64(bytes: &[u8]) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for &b in bytes {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn gate_vanilla_reflection_dds_decodes_to_captured_payload_fnv() {
        let game_dir = replay_test_gates::require_vanilla_game_root();
        let resolver = GameFileResolver::vanilla(game_dir).expect("vanilla resolver");
        let binding = vanilla_reflection_binding(256, 9);
        assert_eq!(
            binding.subresources.len(),
            54,
            "vanilla cube binding must carry all 6 faces x 9 mips"
        );
        assert_eq!(binding.byte_len, 2_112_768, "captured layout byte length");
        let decoded = decode_reflection_cube_payload(&resolver, &binding)
            .expect("resolver-decode shipped reflection.dds");
        assert_eq!(decoded.len(), 2_112_768, "decoded byte length");
        assert_eq!(
            fnv1a64(&decoded),
            0xe267_c5bf_c5d4_99c8,
            "resolver-decoded shipped reflection.dds must reproduce the captured \
             upload payload (FNV-1a64 over all 2,112,768 bytes / 54 subresources)"
        );
        println!(
            "vanilla reflection FNV gate: shipped DDS -> captured layout reproduces \
             the captured payload (2112768 B, fnv e267c5bfc5d499c8)"
        );
    }

    #[test]
    #[cfg_attr(
        not(gate_reflection_cube_proof),
        ignore = "REFLECTION_CUBE_PROOF_DIR naming a directory with reflection.dds and cap_c.bin"
    )]
    fn resolver_decode_byte_equals_captured_payload_from_assets() {
        let asset_dir = replay_test_gates::require_dir_from_env("REFLECTION_CUBE_PROOF_DIR");
        let dds_src = asset_dir.join("reflection.dds");
        replay_test_gates::require_existing_file(&dds_src);
        let captured_path = asset_dir.join("cap_c.bin");
        replay_test_gates::require_existing_file(&captured_path);
        let captured = std::fs::read(captured_path).expect("read cap_c.bin");
        assert_eq!(captured.len(), 2_112_768, "captured payload length");
        assert_eq!(
            fnv1a64(&captured),
            0xe267_c5bf_c5d4_99c8,
            "captured payload FNV must be the declared e267c5bfc5d499c8"
        );

        let root =
            std::env::temp_dir().join(format!("hoi4_reflection_proof_{}", std::process::id()));
        let terrain_dir = root.join("map").join("terrain");
        std::fs::create_dir_all(&terrain_dir).expect("create terrain dir");
        std::fs::copy(&dds_src, terrain_dir.join("reflection.dds")).expect("copy reflection.dds");

        let resolver = GameFileResolver::vanilla(&root).expect("build vanilla resolver");
        let binding = vanilla_reflection_binding(256, 9);
        assert_eq!(binding.byte_len, 2_112_768, "captured layout byte length");

        let decoded = decode_reflection_cube_payload(&resolver, &binding)
            .expect("resolver-decode reflection cube");

        assert_eq!(decoded.len(), captured.len(), "decoded length");
        assert_eq!(
            fnv1a64(&decoded),
            0xe267_c5bf_c5d4_99c8,
            "decoded FNV checkpoint"
        );
        assert!(
            decoded == captured,
            "resolver-decoded reflection cube must be byte-identical to the captured payload"
        );

        std::fs::remove_dir_all(&root).ok();
    }
}
