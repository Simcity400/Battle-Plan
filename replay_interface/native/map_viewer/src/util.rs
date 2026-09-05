use crate::*;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub(crate) fn require_dir(path: &Path, label: &str) -> Result<PathBuf> {
    if !path.is_dir() {
        return Err(format!("missing {label}: {}", path.display()).into());
    }
    Ok(path.to_path_buf())
}

pub(crate) fn require_file(path: &Path, label: &str) -> Result<PathBuf> {
    if !path.is_file() {
        return Err(format!("missing {label}: {}", path.display()).into());
    }
    Ok(path.to_path_buf())
}

#[cfg(test)]
pub(crate) fn read_latin1(path: &Path) -> Result<String> {
    let bytes = fs::read(path)?;
    Ok(latin1_bytes_to_string(&bytes))
}

pub(crate) fn latin1_bytes_to_string(bytes: &[u8]) -> String {
    bytes.iter().copied().map(char::from).collect()
}

pub(crate) fn strip_comments(text: &str) -> String {
    text.lines()
        .map(|line| line.split_once('#').map(|(head, _)| head).unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

pub(crate) fn find_assignment(text: &str, key: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let key_bytes = key.as_bytes();
    if key_bytes.is_empty() || bytes.len() < key_bytes.len() {
        return None;
    }
    let mut i = start;
    while i + key_bytes.len() <= bytes.len() {
        if &bytes[i..i + key_bytes.len()] == key_bytes {
            let before_ok = i == 0 || !is_word_byte(bytes[i - 1]);
            let after = i + key_bytes.len();
            let after_ok = after >= bytes.len() || !is_word_byte(bytes[after]);
            if before_ok && after_ok {
                let mut j = after;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == b'=' {
                    return Some(j + 1);
                }
            }
        }
        i += 1;
    }
    None
}

pub(crate) fn parse_u32_value(text: &str, key: &str) -> Option<u32> {
    let idx = find_assignment(text, key, 0)?;
    parse_u32_at(&text[idx..])
}

pub(crate) fn parse_u64_value(text: &str, key: &str) -> Option<u64> {
    let idx = find_assignment(text, key, 0)?;
    let value = &text[idx..];
    let mut seen = false;
    let mut out = 0u64;
    for ch in value.chars().skip_while(|c| c.is_whitespace()) {
        if let Some(d) = ch.to_digit(10) {
            seen = true;
            out = out.saturating_mul(10).saturating_add(u64::from(d));
        } else {
            break;
        }
    }
    seen.then_some(out)
}

pub(crate) fn parse_f32_value(text: &str, key: &str) -> Option<f32> {
    let idx = find_assignment(text, key, 0)?;
    parse_f32_at(&text[idx..])
}

pub(crate) fn parse_word_value(text: &str, key: &str) -> Option<String> {
    let idx = find_assignment(text, key, 0)?;
    parse_word_at(&text[idx..])
}

pub(crate) fn parse_token_value(text: &str, key: &str) -> Option<String> {
    let idx = find_assignment(text, key, 0)?;
    parse_token_at(&text[idx..])
}

/// `key = "value"` or `key = value`, on the line the key is written on.
///
/// Vanilla writes both - `gfx/entities/wtt_units_infantry.asset` has twenty
/// entities whose `name` and `clone` are bare tokens - and a reader that only
/// looked for the next quote read the NEXT line's quoted `pdxmesh` as the
/// entity's name. Whitespace after the `=` is skipped only on the same line.
pub(crate) fn parse_string_value(text: &str, key: &str) -> Option<String> {
    let idx = find_assignment(text, key, 0)?;
    let value = text[idx..].trim_start_matches([' ', '\t']);
    if let Some(quoted) = value.strip_prefix('"') {
        let end = quoted.find('"')?;
        return Some(quoted[..end].to_string());
    }
    let token: String = value
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != '{' && *c != '}' && *c != '"')
        .collect();
    (!token.is_empty()).then_some(token)
}

pub(crate) fn parse_quoted_value(text: &str, key: &str) -> Option<String> {
    let idx = find_assignment(text, key, 0)?;
    let value = &text[idx..];
    let first = value.find('"')?;
    let rest = &value[first + 1..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

pub(crate) fn parse_u32_at(value: &str) -> Option<u32> {
    let mut seen = false;
    let mut out = 0u32;
    for ch in value.chars().skip_while(|c| c.is_whitespace()) {
        if let Some(d) = ch.to_digit(10) {
            seen = true;
            out = out.saturating_mul(10).saturating_add(d);
        } else {
            break;
        }
    }
    seen.then_some(out)
}

pub(crate) fn parse_f32_at(value: &str) -> Option<f32> {
    let token = parse_token_at(value)?;
    token.parse::<f32>().ok()
}

pub(crate) fn parse_word_at(value: &str) -> Option<String> {
    let word: String = value
        .chars()
        .skip_while(|c| c.is_whitespace())
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    (!word.is_empty()).then_some(word)
}

pub(crate) fn parse_token_at(value: &str) -> Option<String> {
    let token: String = value
        .chars()
        .skip_while(|c| c.is_whitespace())
        .take_while(|c| !c.is_whitespace() && *c != '{' && *c != '}')
        .collect();
    (!token.is_empty()).then_some(token)
}

pub(crate) fn extract_block_after(text: &str, key: &str, start: usize) -> Option<(String, usize)> {
    let idx = find_assignment(text, key, start)?;
    let bytes = text.as_bytes();
    let mut brace = idx;
    while brace < bytes.len() && bytes[brace].is_ascii_whitespace() {
        brace += 1;
    }
    if brace >= bytes.len() || bytes[brace] != b'{' {
        return None;
    }
    let mut depth = 0i32;
    let inner_start = brace + 1;
    let mut i = brace;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((text[inner_start..i].to_string(), i + 1));
                }
            }
            _ => {}
        }
        i += 1;
    }
    Some((text[inner_start..].to_string(), text.len()))
}

pub(crate) fn parse_i32s(text: &str) -> Vec<i32> {
    let mut nums = Vec::new();
    let mut current: Option<i32> = None;
    let mut sign = 1i32;
    for ch in text.chars() {
        if ch == '-' && current.is_none() {
            sign = -1;
        } else if let Some(d) = ch.to_digit(10) {
            let value = current
                .unwrap_or(0)
                .saturating_mul(10)
                .saturating_add(d as i32);
            current = Some(value);
        } else if let Some(value) = current.take() {
            nums.push(value.saturating_mul(sign));
            sign = 1;
        } else {
            sign = 1;
        }
    }
    if let Some(value) = current {
        nums.push(value.saturating_mul(sign));
    }
    nums
}

pub(crate) fn parse_u32s(text: &str) -> Vec<u32> {
    parse_i32s(text)
        .into_iter()
        .filter_map(|n| u32::try_from(n).ok())
        .collect()
}

pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    fs::write(path, bytes)?;
    Ok(())
}

pub(crate) fn write_shader_rgba_sources(
    resolver: &GameFileResolver,
    out: &Path,
    entries: &[(&str, &str, &PathBuf)],
) -> Result<(BTreeMap<String, String>, BTreeMap<String, Dimensions>)> {
    let mut files = BTreeMap::new();
    let mut dimensions = BTreeMap::new();
    for (source, output, path) in entries {
        let image = decode_dds_rgba(&resolver.read_resolved_bytes(path)?, path)?;
        write_rgba_raw(&out.join(output), &image.pixels)?;
        files.insert((*source).to_string(), (*output).to_string());
        dimensions.insert(
            (*source).to_string(),
            Dimensions {
                width: image.width,
                height: image.height,
            },
        );
    }
    Ok((files, dimensions))
}

pub(crate) fn write_material_rgba_sources(
    resolver: &GameFileResolver,
    out: &Path,
    entries: &[RgbaSourceSpec],
) -> Result<(BTreeMap<String, String>, BTreeMap<String, Dimensions>)> {
    let mut files = BTreeMap::new();
    let mut dimensions = BTreeMap::new();
    for spec in entries {
        let levels = decode_dds_rgba_mips(&resolver.read_resolved_bytes(&spec.path)?, &spec.path)?;
        let image = levels
            .first()
            .ok_or_else(|| format!("{} contains no DDS mip levels", spec.path.display()))?;
        write_rgba_raw(&out.join(&spec.output), &image.pixels)?;
        if levels.len() > 1 {
            write_rgba_mip_chain_raw(
                &out.join(shader_rgba_mip_output_name(&spec.source)),
                &levels,
            )?;
        }
        files.insert(spec.source.clone(), spec.output.clone());
        dimensions.insert(
            spec.source.clone(),
            Dimensions {
                width: image.width,
                height: image.height,
            },
        );
    }
    Ok((files, dimensions))
}

pub(crate) fn copy_shader_source(resolver: &GameFileResolver, 
    out: &Path,
    source: &str,
    output: &str,
    path: &Path,
    files: &mut BTreeMap<String, String>,
) -> Result<()> {
    fs::write(out.join(output), resolver.read_resolved_bytes(path)?)?;
    files.insert(source.to_string(), output.to_string());
    Ok(())
}

pub(crate) fn copy_bundle_source(resolver: &GameFileResolver, 
    out: &Path,
    source: &str,
    output: &str,
    path: &Path,
    files: &mut BTreeMap<String, String>,
) -> Result<()> {
    fs::write(out.join(output), resolver.read_resolved_bytes(path)?)?;
    files.insert(source.to_string(), output.to_string());
    Ok(())
}

pub(crate) fn write_u32_raw(path: &Path, values: &[u32]) -> Result<()> {
    let mut file = fs::File::create(path)?;
    let mut buf = Vec::with_capacity(values.len() * 4);
    for value in values {
        buf.extend_from_slice(&value.to_le_bytes());
    }
    file.write_all(&buf)?;
    Ok(())
}

pub(crate) fn write_rgba_mip_chain_raw(path: &Path, levels: &[RgbaImage]) -> Result<()> {
    let mut values = Vec::new();
    for level in levels {
        values.extend_from_slice(&level.pixels);
    }
    write_rgba_raw(path, &values)
}

pub(crate) fn write_u8_raw(path: &Path, values: &[u8]) -> Result<()> {
    fs::write(path, values)?;
    Ok(())
}

pub(crate) fn write_rgba_raw(path: &Path, values: &[u8]) -> Result<()> {
    if values.len() % 4 != 0 {
        return Err(format!("{} RGBA data is not 4-byte aligned", path.display()).into());
    }
    fs::write(path, values)?;
    Ok(())
}

pub(crate) fn read_u32_raw(path: &Path, expected_len: usize) -> Result<Vec<u32>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?.read_to_end(&mut bytes)?;
    if bytes.len() != expected_len * 4 {
        return Err(format!(
            "{} has {} bytes, expected {}",
            path.display(),
            bytes.len(),
            expected_len * 4
        )
        .into());
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes(chunk.try_into().expect("chunk size checked")))
        .collect())
}

pub(crate) fn read_rgba_raw(path: &Path, width: usize, height: usize) -> Result<RgbaImage> {
    let expected_len = checked_rgba_byte_len(width, height)?;
    let pixels = fs::read(path)?;
    if pixels.len() != expected_len {
        return Err(format!(
            "{} has {} bytes, expected {} for {}x{} RGBA",
            path.display(),
            pixels.len(),
            expected_len,
            width,
            height
        )
        .into());
    }
    Ok(RgbaImage {
        width,
        height,
        pixels,
    })
}

pub(crate) fn read_rgba32f_raw(path: &Path, width: usize, height: usize) -> Result<Rgba32fImage> {
    let expected_len = checked_rgba32f_byte_len(width, height)?;
    let bytes = fs::read(path)?;
    if bytes.len() != expected_len {
        return Err(format!(
            "{} has {} bytes, expected {} for {}x{} R32G32B32A32_FLOAT",
            path.display(),
            bytes.len(),
            expected_len,
            width,
            height
        )
        .into());
    }
    let pixels = bytes
        .chunks_exact(16)
        .map(|chunk| {
            [
                f32::from_le_bytes(chunk[0..4].try_into().expect("chunk size checked")),
                f32::from_le_bytes(chunk[4..8].try_into().expect("chunk size checked")),
                f32::from_le_bytes(chunk[8..12].try_into().expect("chunk size checked")),
                f32::from_le_bytes(chunk[12..16].try_into().expect("chunk size checked")),
            ]
        })
        .collect::<Vec<_>>();
    if pixels.iter().flatten().any(|value| !value.is_finite()) {
        return Err(format!(
            "production terrain renderer blocked: {} contains non-finite R32G32B32A32_FLOAT values",
            path.display()
        )
        .into());
    }
    Ok(Rgba32fImage {
        width,
        height,
        pixels,
    })
}

pub(crate) fn distinct_count(values: &[u32]) -> usize {
    let mut values = values.to_vec();
    values.sort_unstable();
    values.dedup();
    values.len()
}

pub(crate) fn visual_input_hashes(resolver: &GameFileResolver, 
    entries: &[(&str, &PathBuf)],
) -> Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for (name, path) in entries {
        out.insert((*name).to_string(), fnv1a64_bytes(&resolver.read_resolved_bytes(path)?));
    }
    Ok(out)
}

pub(crate) fn read_rgba_mip_chain_raw(
    path: &Path,
    base_width: usize,
    base_height: usize,
    mip_count: u32,
) -> Result<RgbaMipChain> {
    read_rgba_mip_chain_raw_with_color_space(
        path,
        base_width,
        base_height,
        mip_count,
        RgbaColorSpace::Linear,
    )
}

pub(crate) fn read_rgba_mip_chain_raw_with_color_space(
    path: &Path,
    base_width: usize,
    base_height: usize,
    mip_count: u32,
    color_space: RgbaColorSpace,
) -> Result<RgbaMipChain> {
    let level_count = mip_count.max(1) as usize;
    let mut expected_bytes = 0usize;
    for level in 0..level_count {
        let (width, height) = mip_dimensions(base_width, base_height, level);
        let level_bytes = width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| {
                format!(
                    "{} mip-chain dimensions overflow expected byte length",
                    path.display()
                )
            })?;
        expected_bytes = expected_bytes.checked_add(level_bytes).ok_or_else(|| {
            format!(
                "{} mip-chain dimensions overflow expected byte length",
                path.display()
            )
        })?;
    }
    let pixels = fs::read(path)?;
    if pixels.len() != expected_bytes {
        return Err(format!(
            "{} has {} bytes, expected {}",
            path.display(),
            pixels.len(),
            expected_bytes
        )
        .into());
    }
    let mut levels = Vec::with_capacity(level_count);
    let mut offset = 0usize;
    for level in 0..level_count {
        let (width, height) = mip_dimensions(base_width, base_height, level);
        let len = width * height * 4;
        levels.push(RgbaImage {
            width,
            height,
            pixels: pixels[offset..offset + len].to_vec(),
        });
        offset += len;
    }
    Ok(RgbaMipChain {
        levels,
        color_space,
    })
}

pub(crate) fn mip_dimensions(
    base_width: usize,
    base_height: usize,
    level: usize,
) -> (usize, usize) {
    ((base_width >> level).max(1), (base_height >> level).max(1))
}

pub(crate) fn visual_input_dimensions(
    entries: &[(&str, usize, usize)],
) -> BTreeMap<String, Dimensions> {
    entries
        .iter()
        .map(|(name, width, height)| {
            (
                (*name).to_string(),
                Dimensions {
                    width: *width,
                    height: *height,
                },
            )
        })
        .collect()
}

pub(crate) fn fnv1a64_file(path: &Path) -> Result<String> {
    let mut hash = 0xcbf29ce484222325u64;
    let mut file = fs::File::open(path)?;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        for b in &buf[..n] {
            hash ^= u64::from(*b);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    Ok(format!("{hash:016x}"))
}

pub(crate) fn fnv1a64_bytes(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    use std::fs;

    #[test]
    fn rgba32f_runtime_reader_rejects_nonfinite_values() {
        let root = temp_test_dir("rgba32f_nonfinite");
        fs::create_dir_all(&root).expect("create temp dir");
        let path = root.join("bad.r32g32b32a32_float");
        fs::write(&path, le_f32_bytes(&[0.0, f32::NAN, 1.0, 2.0])).expect("write float texture");
        let err = read_rgba32f_raw(&path, 1, 1).expect_err("NaN float texture must fail closed");
        assert!(err.to_string().contains("non-finite"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rgba_mip_chain_roundtrips_exact_level_dimensions() {
        let root = temp_test_dir("rgba_mip_chain");
        fs::create_dir_all(&root).expect("create temp mip dir");
        let path = root.join("terrain_diffuse_atlas0_mips.rgba");
        let chain = vec![
            RgbaImage {
                width: 2,
                height: 2,
                pixels: vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
            },
            RgbaImage {
                width: 1,
                height: 1,
                pixels: vec![17, 18, 19, 20],
            },
        ];
        write_rgba_mip_chain_raw(&path, &chain).expect("write mip chain");
        let read = read_rgba_mip_chain_raw(&path, 2, 2, 2).expect("read mip chain");
        assert_eq!(read.levels.len(), 2);
        assert_eq!((read.levels[0].width, read.levels[0].height), (2, 2));
        assert_eq!((read.levels[1].width, read.levels[1].height), (1, 1));
        assert_eq!(read.levels[0].pixels, chain[0].pixels);
        assert_eq!(read.levels[1].pixels, chain[1].pixels);
        let _ = fs::remove_dir_all(&root);
    }
}
