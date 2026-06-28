use std::path::{Path, PathBuf};

use crate::{decode_png_rgba8, save_png};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PngImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelCounts {
    pub r: u64,
    pub g: u64,
    pub b: u64,
    pub a: u64,
}

impl ChannelCounts {
    fn zero() -> Self {
        Self {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        }
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "r": self.r,
            "g": self.g,
            "b": self.b,
            "a": self.a,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PixelMismatch {
    pub x: u32,
    pub y: u32,
    pub left_rgba: [u8; 4],
    pub right_rgba: [u8; 4],
    pub abs_delta_rgba: [u8; 4],
}

impl PixelMismatch {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "x": self.x,
            "y": self.y,
            "left_rgba": self.left_rgba,
            "right_rgba": self.right_rgba,
            "abs_delta_rgba": self.abs_delta_rgba,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualDiffReport {
    pub width: u32,
    pub height: u32,
    pub total_pixels: u64,
    pub changed_pixels: u64,
    pub channel_changed_pixels: ChannelCounts,
    pub max_channel_delta: u8,
    pub sum_abs_channel_delta: u64,
    pub first_mismatch: Option<PixelMismatch>,
}

impl VisualDiffReport {
    pub fn identical(&self) -> bool {
        self.changed_pixels == 0
    }

    pub fn to_json_value(
        &self,
        left_path: &Path,
        right_path: &Path,
        diff_path: &Path,
    ) -> serde_json::Value {
        serde_json::json!({
            "version": 1,
            "kind": "exact_png_visual_diff",
            "status": if self.identical() { "identical" } else { "different" },
            "comparison": {
                "mode": "exact_rgba8",
                "tolerance": 0,
            },
            "left": {
                "path": left_path.display().to_string(),
                "width": self.width,
                "height": self.height,
            },
            "right": {
                "path": right_path.display().to_string(),
                "width": self.width,
                "height": self.height,
            },
            "diff": {
                "path": diff_path.display().to_string(),
                "image": "magenta changed-pixel mask; black means byte-identical pixel",
            },
            "total_pixels": self.total_pixels,
            "changed_pixels": self.changed_pixels,
            "unchanged_pixels": self.total_pixels - self.changed_pixels,
            "channel_changed_pixels": self.channel_changed_pixels.to_json(),
            "max_channel_delta": self.max_channel_delta,
            "sum_abs_channel_delta": self.sum_abs_channel_delta,
            "first_mismatch": self.first_mismatch.as_ref().map(PixelMismatch::to_json),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VisualDiffError {
    MissingInput {
        path: PathBuf,
    },
    InputIsDirectory {
        path: PathBuf,
    },
    ReadInput {
        path: PathBuf,
        message: String,
    },
    DecodeInput {
        path: PathBuf,
        message: String,
    },
    OutputOverwritesInput {
        output_path: PathBuf,
        input_path: PathBuf,
    },
    SizeMismatch {
        left_path: PathBuf,
        left_width: u32,
        left_height: u32,
        right_path: PathBuf,
        right_width: u32,
        right_height: u32,
    },
    WriteDiff {
        path: PathBuf,
        message: String,
    },
}

impl std::fmt::Display for VisualDiffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingInput { path } => {
                write!(f, "input PNG not found: {}", path.display())
            }
            Self::InputIsDirectory { path } => {
                write!(f, "input PNG is a directory: {}", path.display())
            }
            Self::ReadInput { path, message } => {
                write!(f, "read input PNG '{}' failed: {message}", path.display())
            }
            Self::DecodeInput { path, message } => {
                write!(f, "decode input PNG '{}' failed: {message}", path.display())
            }
            Self::OutputOverwritesInput {
                output_path,
                input_path,
            } => write!(
                f,
                "output diff PNG '{}' would overwrite input '{}'",
                output_path.display(),
                input_path.display()
            ),
            Self::SizeMismatch {
                left_path,
                left_width,
                left_height,
                right_path,
                right_width,
                right_height,
            } => write!(
                f,
                "size mismatch: '{}' is {}x{}, '{}' is {}x{}",
                left_path.display(),
                left_width,
                left_height,
                right_path.display(),
                right_width,
                right_height
            ),
            Self::WriteDiff { path, message } => {
                write!(f, "write diff PNG '{}' failed: {message}", path.display())
            }
        }
    }
}

impl std::error::Error for VisualDiffError {}

pub fn load_png_rgba8_file(path: &Path) -> Result<PngImage, VisualDiffError> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => {
            return Err(VisualDiffError::InputIsDirectory {
                path: path.to_path_buf(),
            });
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(VisualDiffError::MissingInput {
                path: path.to_path_buf(),
            });
        }
        Err(e) => {
            return Err(VisualDiffError::ReadInput {
                path: path.to_path_buf(),
                message: e.to_string(),
            });
        }
    }

    let bytes = std::fs::read(path).map_err(|e| VisualDiffError::ReadInput {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;
    let (width, height, rgba) =
        decode_png_rgba8(&bytes).map_err(|e| VisualDiffError::DecodeInput {
            path: path.to_path_buf(),
            message: e,
        })?;
    Ok(PngImage {
        width,
        height,
        rgba,
    })
}

pub fn diff_png_files(
    left_path: &Path,
    right_path: &Path,
    diff_path: &Path,
) -> Result<VisualDiffReport, VisualDiffError> {
    let left = load_png_rgba8_file(left_path)?;
    let right = load_png_rgba8_file(right_path)?;
    reject_output_overwrites_input(left_path, right_path, diff_path)?;
    let (report, diff_rgba) = diff_images(left_path, &left, right_path, &right)?;
    save_png(diff_path, report.width, report.height, &diff_rgba).map_err(|e| {
        VisualDiffError::WriteDiff {
            path: diff_path.to_path_buf(),
            message: e.to_string(),
        }
    })?;
    Ok(report)
}

fn reject_output_overwrites_input(
    left_path: &Path,
    right_path: &Path,
    diff_path: &Path,
) -> Result<(), VisualDiffError> {
    let Ok(diff_canon) = std::fs::canonicalize(diff_path) else {
        return Ok(());
    };
    for input_path in [left_path, right_path] {
        if let Ok(input_canon) = std::fs::canonicalize(input_path) {
            if diff_canon == input_canon {
                return Err(VisualDiffError::OutputOverwritesInput {
                    output_path: diff_path.to_path_buf(),
                    input_path: input_path.to_path_buf(),
                });
            }
        }
    }
    Ok(())
}

pub fn diff_images(
    left_path: &Path,
    left: &PngImage,
    right_path: &Path,
    right: &PngImage,
) -> Result<(VisualDiffReport, Vec<u8>), VisualDiffError> {
    if left.width != right.width || left.height != right.height {
        return Err(VisualDiffError::SizeMismatch {
            left_path: left_path.to_path_buf(),
            left_width: left.width,
            left_height: left.height,
            right_path: right_path.to_path_buf(),
            right_width: right.width,
            right_height: right.height,
        });
    }

    let total_pixels = (left.width as u64) * (left.height as u64);
    let mut report = VisualDiffReport {
        width: left.width,
        height: left.height,
        total_pixels,
        changed_pixels: 0,
        channel_changed_pixels: ChannelCounts::zero(),
        max_channel_delta: 0,
        sum_abs_channel_delta: 0,
        first_mismatch: None,
    };
    let mut diff = vec![0u8; left.rgba.len()];

    for (idx, (l, r)) in left
        .rgba
        .chunks_exact(4)
        .zip(right.rgba.chunks_exact(4))
        .enumerate()
    {
        let delta = [
            l[0].abs_diff(r[0]),
            l[1].abs_diff(r[1]),
            l[2].abs_diff(r[2]),
            l[3].abs_diff(r[3]),
        ];
        let changed = delta.iter().any(|d| *d != 0);
        let out = idx * 4;
        if changed {
            report.changed_pixels += 1;
            diff[out..out + 4].copy_from_slice(&[255, 0, 255, 255]);
            if delta[0] != 0 {
                report.channel_changed_pixels.r += 1;
            }
            if delta[1] != 0 {
                report.channel_changed_pixels.g += 1;
            }
            if delta[2] != 0 {
                report.channel_changed_pixels.b += 1;
            }
            if delta[3] != 0 {
                report.channel_changed_pixels.a += 1;
            }
            for d in delta {
                report.max_channel_delta = report.max_channel_delta.max(d);
                report.sum_abs_channel_delta += d as u64;
            }
            if report.first_mismatch.is_none() {
                let x = (idx as u32) % left.width;
                let y = (idx as u32) / left.width;
                report.first_mismatch = Some(PixelMismatch {
                    x,
                    y,
                    left_rgba: [l[0], l[1], l[2], l[3]],
                    right_rgba: [r[0], r[1], r[2], r[3]],
                    abs_delta_rgba: delta,
                });
            }
        } else {
            diff[out..out + 4].copy_from_slice(&[0, 0, 0, 255]);
        }
    }

    Ok((report, diff))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(test_name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "battle_plan_visual_diff_{test_name}_{}_{}",
            std::process::id(),
            nanos
        ))
    }

    fn write_image(path: &Path, width: u32, height: u32, rgba: &[u8]) {
        save_png(path, width, height, rgba).expect("write test png");
    }

    #[test]
    fn identical_tiny_pngs_write_empty_mask_and_exact_metadata() {
        let dir = temp_dir("identical");
        let left = dir.join("left.png");
        let right = dir.join("right.png");
        let diff = dir.join("diff.png");
        let rgba = [
            10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 200, 100, 110, 120, 0,
        ];
        write_image(&left, 2, 2, &rgba);
        write_image(&right, 2, 2, &rgba);

        let report = diff_png_files(&left, &right, &diff).expect("diff identical");
        assert!(report.identical());
        assert_eq!(report.changed_pixels, 0);
        assert_eq!(report.max_channel_delta, 0);
        assert_eq!(report.sum_abs_channel_delta, 0);
        assert!(report.first_mismatch.is_none());

        let out = load_png_rgba8_file(&diff).expect("load diff");
        assert_eq!(out.width, 2);
        assert_eq!(out.height, 2);
        assert_eq!(
            out.rgba,
            vec![0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255]
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn changed_tiny_pngs_report_exact_pixel_and_channel_stats() {
        let dir = temp_dir("changed");
        let left = dir.join("left.png");
        let right = dir.join("right.png");
        let diff = dir.join("diff.png");
        let left_rgba = [
            1, 2, 3, 255, 10, 20, 30, 255, 50, 60, 70, 80, 90, 100, 110, 120,
        ];
        let right_rgba = [
            1, 2, 3, 255, 11, 20, 30, 200, 50, 61, 80, 80, 90, 100, 110, 120,
        ];
        write_image(&left, 2, 2, &left_rgba);
        write_image(&right, 2, 2, &right_rgba);

        let report = diff_png_files(&left, &right, &diff).expect("diff changed");
        assert!(!report.identical());
        assert_eq!(report.total_pixels, 4);
        assert_eq!(report.changed_pixels, 2);
        assert_eq!(report.channel_changed_pixels.r, 1);
        assert_eq!(report.channel_changed_pixels.g, 1);
        assert_eq!(report.channel_changed_pixels.b, 1);
        assert_eq!(report.channel_changed_pixels.a, 1);
        assert_eq!(report.max_channel_delta, 55);
        assert_eq!(report.sum_abs_channel_delta, 67);
        assert_eq!(
            report.first_mismatch,
            Some(PixelMismatch {
                x: 1,
                y: 0,
                left_rgba: [10, 20, 30, 255],
                right_rgba: [11, 20, 30, 200],
                abs_delta_rgba: [1, 0, 0, 55],
            })
        );

        let out = load_png_rgba8_file(&diff).expect("load diff");
        assert_eq!(
            out.rgba,
            vec![0, 0, 0, 255, 255, 0, 255, 255, 255, 0, 255, 255, 0, 0, 0, 255]
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn size_mismatch_fails_closed_without_writing_diff() {
        let dir = temp_dir("size_mismatch");
        let left = dir.join("left.png");
        let right = dir.join("right.png");
        let diff = dir.join("diff.png");
        write_image(&left, 1, 1, &[0, 0, 0, 255]);
        write_image(&right, 2, 1, &[0, 0, 0, 255, 0, 0, 0, 255]);

        let err = diff_png_files(&left, &right, &diff).unwrap_err();
        assert!(matches!(err, VisualDiffError::SizeMismatch { .. }));
        assert!(!diff.exists(), "mismatched input must not write a diff");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_input_fails_closed_without_writing_diff() {
        let dir = temp_dir("missing");
        std::fs::create_dir_all(&dir).unwrap();
        let left = dir.join("missing.png");
        let right = dir.join("right.png");
        let diff = dir.join("diff.png");
        write_image(&right, 1, 1, &[0, 0, 0, 255]);

        let err = diff_png_files(&left, &right, &diff).unwrap_err();
        assert_eq!(err, VisualDiffError::MissingInput { path: left.clone() });
        assert!(!diff.exists(), "missing input must not write a diff");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn output_must_not_overwrite_either_input() {
        let dir = temp_dir("overwrite_input");
        let left = dir.join("left.png");
        let right = dir.join("right.png");
        write_image(&left, 1, 1, &[10, 20, 30, 255]);
        write_image(&right, 1, 1, &[10, 20, 31, 255]);

        let err = diff_png_files(&left, &right, &left).unwrap_err();
        assert_eq!(
            err,
            VisualDiffError::OutputOverwritesInput {
                output_path: left.clone(),
                input_path: left.clone(),
            }
        );
        let still_left = load_png_rgba8_file(&left).expect("left still readable");
        assert_eq!(still_left.rgba, vec![10, 20, 30, 255]);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
