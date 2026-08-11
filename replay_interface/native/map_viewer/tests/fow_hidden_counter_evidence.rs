//! Wave-9 hidden-counter evidence gate.
//!
//! CAPTURE source: `%LOCALAPPDATA%/hoi4_renderdoc/mined/hoi4_frame69592`
//! (FoW ON) and `hoi4_frame71288` (FoW OFF). Draw families are identified by
//! the bound resource descriptors documented in `lane_reports/w9-fow-producer`:
//! land base 198x27, land overlay 30x20, naval base 222x36. WVP xy lives at
//! bytes 48..56 of buffer 647577 CB-write blobs; the corrected evidence chunks
//! are 35813/35824 and 36070/36081, not their following draw chunks.

/// The crate's gate table, included so this test resolves the evidence store
/// through EXACTLY the candidate list the build-time probe used.
#[allow(dead_code)]
mod corpus {
    include!("../test_gates.rs");
}

use flate2::read::{DeflateDecoder, GzDecoder};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::error::Error;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

type TestResult<T> = Result<T, Box<dyn Error>>;

#[derive(Debug)]
struct CounterFrameEvidence {
    land_bases: usize,
    land_overlays: usize,
    naval_bases: usize,
    land_wvp_xy: BTreeSet<[u8; 8]>,
}

fn le_u16(bytes: &[u8], offset: usize) -> TestResult<u16> {
    Ok(u16::from_le_bytes(
        bytes
            .get(offset..offset + 2)
            .ok_or("ZIP structure ended before u16 field")?
            .try_into()?,
    ))
}

fn le_u32(bytes: &[u8], offset: usize) -> TestResult<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or("ZIP structure ended before u32 field")?
            .try_into()?,
    ))
}

fn read_zip_member(path: &Path, wanted: &str) -> TestResult<Vec<u8>> {
    let bytes = fs::read(path)?;
    let eocd = bytes
        .windows(4)
        .rposition(|window| window == [0x50, 0x4b, 0x05, 0x06])
        .ok_or("CB blob ZIP has no end-of-central-directory record")?;
    let entries = usize::from(le_u16(&bytes, eocd + 10)?);
    let mut cursor = usize::try_from(le_u32(&bytes, eocd + 16)?)?;
    for _ in 0..entries {
        if le_u32(&bytes, cursor)? != 0x0201_4b50 {
            return Err("CB blob ZIP central-directory signature drifted".into());
        }
        let method = le_u16(&bytes, cursor + 10)?;
        let compressed_len = usize::try_from(le_u32(&bytes, cursor + 20)?)?;
        let uncompressed_len = usize::try_from(le_u32(&bytes, cursor + 24)?)?;
        let name_len = usize::from(le_u16(&bytes, cursor + 28)?);
        let extra_len = usize::from(le_u16(&bytes, cursor + 30)?);
        let comment_len = usize::from(le_u16(&bytes, cursor + 32)?);
        let local = usize::try_from(le_u32(&bytes, cursor + 42)?)?;
        let name = std::str::from_utf8(
            bytes
                .get(cursor + 46..cursor + 46 + name_len)
                .ok_or("CB blob ZIP member name is truncated")?,
        )?;
        if name == wanted {
            if le_u32(&bytes, local)? != 0x0403_4b50 {
                return Err("CB blob ZIP local-header signature drifted".into());
            }
            let local_name_len = usize::from(le_u16(&bytes, local + 26)?);
            let local_extra_len = usize::from(le_u16(&bytes, local + 28)?);
            let data_start = local + 30 + local_name_len + local_extra_len;
            let compressed = bytes
                .get(data_start..data_start + compressed_len)
                .ok_or("CB blob ZIP member payload is truncated")?;
            let mut out = Vec::with_capacity(uncompressed_len);
            match method {
                0 => out.extend_from_slice(compressed),
                8 => {
                    DeflateDecoder::new(compressed).read_to_end(&mut out)?;
                }
                other => return Err(format!("CB blob ZIP uses unsupported method {other}").into()),
            }
            if out.len() != uncompressed_len {
                return Err(format!(
                    "CB blob ZIP member {wanted} decoded {} bytes, expected {uncompressed_len}",
                    out.len()
                )
                .into());
            }
            return Ok(out);
        }
        cursor += 46 + name_len + extra_len + comment_len;
    }
    Err(format!("CB blob ZIP lacks required member {wanted}").into())
}

fn counter_frame_evidence(frame_dir: &Path) -> TestResult<CounterFrameEvidence> {
    let resources: Value = serde_json::from_slice(&fs::read(frame_dir.join("resources.json"))?)?;
    let reader = BufReader::new(GzDecoder::new(fs::File::open(
        frame_dir.join("events.jsonl.gz"),
    )?));
    let mut current_srv = None;
    let mut latest_cb_write = HashMap::<u64, u64>::new();
    let mut current_cb = None;
    let mut evidence = CounterFrameEvidence {
        land_bases: 0,
        land_overlays: 0,
        naval_bases: 0,
        land_wvp_xy: BTreeSet::new(),
    };
    for line in reader.lines() {
        let event: Value = serde_json::from_str(&line?)?;
        match event.get("c").and_then(Value::as_str) {
            Some("PSSetShaderResources")
                if event.get("slot").and_then(Value::as_u64) == Some(0) =>
            {
                current_srv = event
                    .get("ids")
                    .and_then(Value::as_array)
                    .and_then(|ids| ids.first())
                    .and_then(Value::as_u64);
            }
            Some("Unmap") => {
                if let (Some(id), Some(chunk)) = (
                    event.get("id").and_then(Value::as_u64),
                    event.get("i").and_then(Value::as_u64),
                ) {
                    latest_cb_write.insert(id, chunk);
                }
            }
            Some("VSSetConstantBuffers1")
                if event.get("slot").and_then(Value::as_u64) == Some(0) =>
            {
                current_cb = event
                    .get("ids")
                    .and_then(Value::as_array)
                    .and_then(|ids| ids.first())
                    .and_then(Value::as_u64);
            }
            Some("Draw")
                if event.get("n").and_then(Value::as_u64) == Some(4)
                    && event.get("vs").and_then(Value::as_u64) == Some(157)
                    && event.get("ps").and_then(Value::as_u64) == Some(158) =>
            {
                let view_key = current_srv
                    .ok_or("counter draw has no PS slot-0 SRV")?
                    .to_string();
                let texture_key = resources
                    .get(&view_key)
                    .and_then(|row| row.get("of"))
                    .and_then(Value::as_u64)
                    .ok_or("counter SRV has no underlying texture id")?
                    .to_string();
                let texture = resources
                    .get(&texture_key)
                    .ok_or("counter texture is absent from resources.json")?;
                let dims = (
                    texture.get("w").and_then(Value::as_u64),
                    texture.get("h").and_then(Value::as_u64),
                );
                match dims {
                    (Some(198), Some(27)) => {
                        evidence.land_bases += 1;
                        let cb = current_cb.ok_or("land counter draw has no bound VS CB0")?;
                        let chunk = latest_cb_write
                            .get(&cb)
                            .copied()
                            .ok_or("land counter draw has no preceding CB write")?;
                        let blob = read_zip_member(
                            &frame_dir.join("cb_blobs.zip"),
                            &format!("{chunk}_{cb}.bin"),
                        )?;
                        evidence.land_wvp_xy.insert(
                            blob.get(48..56)
                                .ok_or("land counter CB is too short for WVP xy")?
                                .try_into()?,
                        );
                    }
                    (Some(30), Some(20)) => evidence.land_overlays += 1,
                    (Some(222), Some(36)) => evidence.naval_bases += 1,
                    _ => {}
                }
            }
            _ => {}
        }
    }
    Ok(evidence)
}

// Was a static `#[ignore]`, so it never ran under the standing `Mode Final`
// gate even on a machine that holds the mined store.
#[test]
#[cfg_attr(not(gate_renderdoc_mined), ignore = "the mined RenderDoc evidence store (set BATTLE_PLAN_RENDERDOC_MINED, else %LOCALAPPDATA%\\hoi4_renderdoc\\mined)")]
fn wave9_hidden_counter_gate_reads_draws_and_wvp_cb_writes() {
    let root = corpus::renderdoc_mined_root()
        .expect("the mined RenderDoc evidence store is absent, but this test was compiled as runnable");
    let on = counter_frame_evidence(&root.join("hoi4_frame69592"))
        .expect("analyze FoW-ON counter evidence");
    let off = counter_frame_evidence(&root.join("hoi4_frame71288"))
        .expect("analyze FoW-OFF counter evidence");

    // CAPTURE `w9-fow-producer.md:63-66`.
    assert_eq!((off.land_bases, on.land_bases), (57, 47));
    assert_eq!((off.land_overlays, on.land_overlays), (57, 47));
    assert_eq!((off.naval_bases, on.naval_bases), (1, 0));
    assert_eq!(off.land_wvp_xy.intersection(&on.land_wvp_xy).count(), 47);
    assert_eq!(off.land_wvp_xy.difference(&on.land_wvp_xy).count(), 10);
    assert_eq!(on.land_wvp_xy.difference(&off.land_wvp_xy).count(), 0);

    // Corrected CAPTURE attribution: bytes are in the CB-write members, not
    // the following draw chunks 35818 and 36075.
    let first = read_zip_member(
        &root.join("hoi4_frame71288").join("cb_blobs.zip"),
        "35813_647577.bin",
    )
    .expect("corrected CB-write chunk 35813");
    assert_eq!(
        &first[48..56],
        &[0xab, 0xaa, 0x4a, 0x3e, 0x5b, 0xb0, 0x85, 0x3e]
    );
    let second = read_zip_member(
        &root.join("hoi4_frame71288").join("cb_blobs.zip"),
        "36070_647577.bin",
    )
    .expect("corrected CB-write chunk 36070");
    assert_eq!(
        &second[48..56],
        &[0x78, 0x77, 0xd7, 0xbd, 0xe3, 0x38, 0x0e, 0x3e]
    );
}
