use std::{
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopBox {
    pub kind: [u8; 4],
    pub offset: u64,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mp4Summary {
    pub boxes: Vec<TopBox>,
    pub moov_before_mdat: bool,
    pub has_avc1: bool,
    pub has_mp4a: bool,
    pub duration_ms: Option<u64>,
}

pub fn inspect_mp4(path: &Path) -> io::Result<Mp4Summary> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let mut offset = 0u64;
    let mut boxes = Vec::new();
    let mut moov_range = None;
    let mut moov_index = None;
    let mut mdat_index = None;

    while offset + 8 <= len {
        file.seek(SeekFrom::Start(offset))?;
        let mut header = [0u8; 8];
        file.read_exact(&mut header)?;
        let size32 = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
        let kind = [header[4], header[5], header[6], header[7]];
        let size = match size32 {
            0 => len - offset,
            1 => {
                if offset + 16 > len {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "truncated 64-bit MP4 box"));
                }
                let mut ext = [0u8; 8];
                file.read_exact(&mut ext)?;
                u64::from_be_bytes(ext)
            }
            other => u64::from(other),
        };
        if size < 8 || offset.saturating_add(size) > len {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid MP4 box size"));
        }
        let idx = boxes.len();
        if &kind == b"moov" {
            moov_index = Some(idx);
            moov_range = Some((offset, size));
        } else if &kind == b"mdat" && mdat_index.is_none() {
            mdat_index = Some(idx);
        }
        boxes.push(TopBox { kind, offset, size });
        offset += size;
    }

    let (has_avc1, has_mp4a, duration_ms) = if let Some((moov_offset, moov_size)) = moov_range {
        let cap = usize::try_from(moov_size).unwrap_or(0).min(16 << 20);
        let mut moov = vec![0u8; cap];
        file.seek(SeekFrom::Start(moov_offset))?;
        file.read_exact(&mut moov)?;
        let has_avc1 = moov.windows(4).any(|w| w == b"avc1");
        let has_mp4a = moov.windows(4).any(|w| w == b"mp4a");
        let duration_ms = parse_mvhd_duration_ms(&moov);
        (has_avc1, has_mp4a, duration_ms)
    } else {
        (false, false, None)
    };

    let moov_before_mdat = matches!((moov_index, mdat_index), (Some(moov), Some(mdat)) if moov < mdat);
    Ok(Mp4Summary {
        boxes,
        moov_before_mdat,
        has_avc1,
        has_mp4a,
        duration_ms,
    })
}

pub fn relocate_moov_before_mdat(path: &Path) -> io::Result<()> {
    let summary = inspect_mp4(path)?;
    if summary.moov_before_mdat {
        return Ok(());
    }
    let Some(moov) = summary.boxes.iter().find(|b| &b.kind == b"moov").cloned() else {
        return Ok(());
    };
    let Some(first_mdat) = summary.boxes.iter().find(|b| &b.kind == b"mdat").cloned() else {
        return Ok(());
    };
    if moov.offset < first_mdat.offset {
        return Ok(());
    }
    let moov_len = usize::try_from(moov.size)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "MP4 moov box exceeds addressable memory"))?;
    let mut moov_bytes = vec![0u8; moov_len];
    let file_len = {
        let mut src = File::open(path)?;
        let len = src.metadata()?.len();
        src.seek(SeekFrom::Start(moov.offset))?;
        src.read_exact(&mut moov_bytes)?;
        len
    };

    patch_chunk_offsets(&mut moov_bytes, first_mdat.offset, moov.offset, moov.size)?;

    let mut tmp_name = path.as_os_str().to_owned();
    tmp_name.push(".faststart");
    let tmp_path = std::path::PathBuf::from(tmp_name);
    let write_res = (|| -> io::Result<()> {
        let mut src = File::open(path)?;
        let mut dst = File::create(&tmp_path)?;
        copy_range(&mut src, &mut dst, 0, first_mdat.offset)?;
        dst.write_all(&moov_bytes)?;
        copy_range(&mut src, &mut dst, first_mdat.offset, moov.offset - first_mdat.offset)?;
        let moov_end = moov.offset + moov.size;
        if file_len > moov_end {
            copy_range(&mut src, &mut dst, moov_end, file_len - moov_end)?;
        }
        dst.flush()?;
        Ok(())
    })();
    if let Err(error) = write_res {
        let _ = fs::remove_file(&tmp_path);
        return Err(error);
    }
    let _ = fs::remove_file(path);
    if let Err(error) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(error);
    }
    Ok(())
}

fn copy_range(src: &mut File, dst: &mut File, start: u64, mut remaining: u64) -> io::Result<()> {
    src.seek(SeekFrom::Start(start))?;
    let mut buf = vec![0u8; 64 * 1024];
    while remaining > 0 {
        let take = usize::try_from(remaining.min(buf.len() as u64)).unwrap_or(buf.len());
        src.read_exact(&mut buf[..take])?;
        dst.write_all(&buf[..take])?;
        remaining -= take as u64;
    }
    Ok(())
}

fn patch_chunk_offsets(bytes: &mut [u8], insert_offset: u64, moov_offset: u64, delta: u64) -> io::Result<()> {
    patch_children(bytes, 0, bytes.len(), insert_offset, moov_offset, delta)
}

fn patch_children(
    bytes: &mut [u8],
    start: usize,
    end: usize,
    insert_offset: u64,
    moov_offset: u64,
    delta: u64,
) -> io::Result<()> {
    let mut pos = start;
    while pos + 8 <= end {
        let size32 = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]);
        let kind = [bytes[pos + 4], bytes[pos + 5], bytes[pos + 6], bytes[pos + 7]];
        let (header, size) = match size32 {
            0 => (8usize, end - pos),
            1 => {
                if pos + 16 > end {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "truncated 64-bit box inside moov"));
                }
                let ext = u64::from_be_bytes(bytes[pos + 8..pos + 16].try_into().unwrap());
                let size = usize::try_from(ext)
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "oversized box inside moov"))?;
                (16usize, size)
            }
            other => (8usize, other as usize),
        };
        if size < header || pos + size > end {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid child box inside moov"));
        }
        let body_start = pos + header;
        let box_end = pos + size;
        match &kind {
            b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"edts" | b"dinf" | b"udta" => {
                patch_children(bytes, body_start, box_end, insert_offset, moov_offset, delta)?;
            }
            b"stco" => {
                if body_start + 8 > box_end {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "truncated stco box"));
                }
                let count = u32::from_be_bytes(bytes[body_start + 4..body_start + 8].try_into().unwrap()) as usize;
                let entries_start = body_start + 8;
                if entries_start + count * 4 > box_end {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "truncated stco entries"));
                }
                for i in 0..count {
                    let at = entries_start + i * 4;
                    let off = u64::from(u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()));
                    if off >= insert_offset && off < moov_offset {
                        let shifted = off
                            .checked_add(delta)
                            .and_then(|v| u32::try_from(v).ok())
                            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "stco offset exceeds 32-bit range"))?;
                        bytes[at..at + 4].copy_from_slice(&shifted.to_be_bytes());
                    }
                }
            }
            b"co64" => {
                if body_start + 8 > box_end {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "truncated co64 box"));
                }
                let count = u32::from_be_bytes(bytes[body_start + 4..body_start + 8].try_into().unwrap()) as usize;
                let entries_start = body_start + 8;
                if entries_start + count * 8 > box_end {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "truncated co64 entries"));
                }
                for i in 0..count {
                    let at = entries_start + i * 8;
                    let off = u64::from_be_bytes(bytes[at..at + 8].try_into().unwrap());
                    if off >= insert_offset && off < moov_offset {
                        let shifted = off
                            .checked_add(delta)
                            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "co64 offset overflow"))?;
                        bytes[at..at + 8].copy_from_slice(&shifted.to_be_bytes());
                    }
                }
            }
            _ => {}
        }
        pos = box_end;
    }
    Ok(())
}

fn parse_mvhd_duration_ms(moov: &[u8]) -> Option<u64> {
    let pos = moov.windows(4).position(|w| w == b"mvhd")?;
    let body = moov.get(pos + 4..)?;
    let version = *body.first()?;
    if version == 0 {
        let timescale = u32::from_be_bytes(body.get(12..16)?.try_into().ok()?);
        let duration = u32::from_be_bytes(body.get(16..20)?.try_into().ok()?);
        if timescale == 0 {
            return None;
        }
        Some(u64::from(duration) * 1000 / u64::from(timescale))
    } else if version == 1 {
        let timescale = u32::from_be_bytes(body.get(20..24)?.try_into().ok()?);
        let duration = u64::from_be_bytes(body.get(24..32)?.try_into().ok()?);
        if timescale == 0 {
            return None;
        }
        Some(duration.saturating_mul(1000) / u64::from(timescale))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::inspect_mp4;
    use std::fs;

    fn make_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let size = (8 + payload.len()) as u32;
        let mut out = Vec::with_capacity(size as usize);
        out.extend_from_slice(&size.to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn distinguishes_moov_before_and_after_mdat() {
        let dir = std::env::temp_dir().join(format!("bp-mp4-boxes-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let ftyp = make_box(b"ftyp", b"isom\x00\x00\x02\x00isom");
        let mut mvhd_payload = vec![0u8; 24];
        mvhd_payload[12..16].copy_from_slice(&1000u32.to_be_bytes());
        mvhd_payload[16..20].copy_from_slice(&2500u32.to_be_bytes());
        let mut moov_body = make_box(b"mvhd", &mvhd_payload);
        moov_body.extend_from_slice(b"avc1");
        let moov = make_box(b"moov", &moov_body);
        let mdat = make_box(b"mdat", &[1, 2, 3, 4]);

        let fast = dir.join("fast.mp4");
        fs::write(&fast, [&ftyp[..], &moov[..], &mdat[..]].concat()).unwrap();
        let summary = inspect_mp4(&fast).unwrap();
        assert!(summary.moov_before_mdat);
        assert!(summary.has_avc1);
        assert!(!summary.has_mp4a);
        assert_eq!(summary.duration_ms, Some(2500));

        let slow = dir.join("slow.mp4");
        fs::write(&slow, [&ftyp[..], &mdat[..], &moov[..]].concat()).unwrap();
        let summary = inspect_mp4(&slow).unwrap();
        assert!(!summary.moov_before_mdat);

        let mdat_payload_offset = (ftyp.len() + 8) as u32;
        let mut stco_body = vec![0u8; 12];
        stco_body[4..8].copy_from_slice(&1u32.to_be_bytes());
        stco_body[8..12].copy_from_slice(&mdat_payload_offset.to_be_bytes());
        let stco = make_box(b"stco", &stco_body);
        let stbl = make_box(b"stbl", &stco);
        let minf = make_box(b"minf", &stbl);
        let mdia = make_box(b"mdia", &minf);
        let trak = make_box(b"trak", &mdia);
        let mut moov_with_stco_body = make_box(b"mvhd", &mvhd_payload);
        moov_with_stco_body.extend_from_slice(&trak);
        moov_with_stco_body.extend_from_slice(b"avc1");
        let moov_with_stco = make_box(b"moov", &moov_with_stco_body);
        let relocate = dir.join("relocate.mp4");
        fs::write(&relocate, [&ftyp[..], &mdat[..], &moov_with_stco[..]].concat()).unwrap();
        super::relocate_moov_before_mdat(&relocate).unwrap();
        let relocated = inspect_mp4(&relocate).unwrap();
        assert!(relocated.moov_before_mdat);
        let raw = fs::read(&relocate).unwrap();
        let stco_at = raw.windows(4).position(|w| w == b"stco").unwrap();
        let shifted = u32::from_be_bytes(raw[stco_at + 12..stco_at + 16].try_into().unwrap()) as usize;
        assert_eq!(&raw[shifted..shifted + 4], &[1, 2, 3, 4]);
        let _ = fs::remove_dir_all(&dir);
    }
}
