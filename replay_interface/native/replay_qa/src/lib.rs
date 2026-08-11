use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const DXGI_FORMAT_R16G16B16A16_FLOAT: u32 = 10;
const DXGI_FORMAT_R8G8B8A8_UNORM: u32 = 28;
const DXGI_FORMAT_B8G8R8A8_UNORM: u32 = 87;
const DXGI_FORMAT_B8G8R8A8_UNORM_SRGB: u32 = 91;

#[derive(Clone, Debug)]
pub struct ReplayQaConfig {
    pub repo_root: PathBuf,
    pub native_root: PathBuf,
    pub out_dir: PathBuf,
    pub terrain_map_frame_bundle: PathBuf,
    pub water_map_frame_bundle: PathBuf,
    pub terrain_chunk_library: PathBuf,
    pub capture_dir: PathBuf,
    pub bundle_dir: PathBuf,
    pub game_root: Option<PathBuf>,
    pub mod_root: Option<PathBuf>,
    pub water_expected_fail: bool,
    pub frame_parity_gates: Vec<FrameParityConfig>,
    pub matrix: MatrixConfig,
    pub thresholds: ThresholdConfig,
    /// Per-child-process deadline. A hung child (deadlocked map_viewer,
    /// wedged replay_app) is killed at this deadline so the harness can still
    /// record a typed gate failure instead of hanging forever. Generous by
    /// default because the first cargo build of a cold target dir is slow.
    pub process_timeout: Duration,
}

impl Default for ReplayQaConfig {
    fn default() -> Self {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let native_root = manifest_dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let repo_root = native_root
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_else(|| native_root.clone());
        let repo_capture_dir = native_root
            .join("map_viewer")
            .join("out")
            .join("captures")
            .join("session_20260702d");
        let byte_exact_machine_capture_dir =
            PathBuf::from(r"C:\Users\Example\AppData\Local\hoi4_captures\live_B_full");
        let synthesis_machine_capture_dir =
            PathBuf::from(r"C:\Users\Example\AppData\Local\hoi4_captures\live_PAN2");
        let byte_exact_capture_dir = if repo_capture_dir.join("map_frame_bundle.json").is_file() {
            repo_capture_dir
        } else if byte_exact_machine_capture_dir
            .join("map_frame_bundle.json")
            .is_file()
        {
            byte_exact_machine_capture_dir
        } else if synthesis_machine_capture_dir
            .join("map_frame_bundle.json")
            .is_file()
        {
            synthesis_machine_capture_dir.clone()
        } else {
            repo_capture_dir
        };
        let render_capture_dir = if synthesis_machine_capture_dir
            .join("terrain_chunk_library.json")
            .is_file()
            && synthesis_machine_capture_dir
                .join("map_frame_bundle.json")
                .is_file()
        {
            synthesis_machine_capture_dir
        } else {
            byte_exact_capture_dir.clone()
        };
        let terrain_map_frame_bundle = byte_exact_capture_dir.join("map_frame_bundle.json");
        let water_map_frame_bundle =
            byte_exact_capture_dir.join("map_frame_water_pass_bundle.json");
        let water_ground_truth_available =
            water_frame_ground_truth_available(&water_map_frame_bundle);
        let frame_parity_gates = default_frame_parity_gates(
            &terrain_map_frame_bundle,
            &water_map_frame_bundle,
            water_ground_truth_available,
        );
        Self {
            repo_root: repo_root.clone(),
            native_root: native_root.clone(),
            out_dir: manifest_dir.join("out"),
            terrain_map_frame_bundle,
            water_map_frame_bundle,
            terrain_chunk_library: PathBuf::from(
                r"C:\Users\Example\AppData\Local\hoi4_captures\live_PAN2\terrain_chunk_library.json",
            ),
            capture_dir: render_capture_dir,
            bundle_dir: native_root
                .join("map_viewer")
                .join("out")
                .join("map_bundle"),
            game_root: discover_game_root(),
            mod_root: Some(repo_root),
            // Water is a hard pass/fail only when the selected captured bundle
            // has enough replay provenance to make its RT a coherent reference.
            water_expected_fail: !water_ground_truth_available,
            frame_parity_gates,
            matrix: MatrixConfig::default(),
            thresholds: ThresholdConfig::default(),
            process_timeout: Duration::from_secs(30 * 60),
        }
    }
}

impl ReplayQaConfig {
    pub fn apply_config_text(&mut self, text: &str) -> Result<(), String> {
        for (line_number, raw_line) in text.lines().enumerate() {
            let line_number = line_number + 1;
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| format!("config line {line_number} is missing '='"))?;
            let key = key.trim();
            let value = config_value(value, line_number)?;
            match key {
                "terrain_map_frame_bundle" => {
                    self.set_terrain_map_frame_bundle(PathBuf::from(value));
                    if let Some(parent) = self.terrain_map_frame_bundle.parent() {
                        self.capture_dir = parent.to_path_buf();
                    }
                }
                "water_map_frame_bundle" => self.set_water_map_frame_bundle(PathBuf::from(value)),
                "terrain_chunk_library" => self.terrain_chunk_library = PathBuf::from(value),
                "capture_dir" => self.capture_dir = PathBuf::from(value),
                "bundle_dir" => self.bundle_dir = PathBuf::from(value),
                "out_dir" => self.out_dir = PathBuf::from(value),
                "game_root" => self.game_root = Some(PathBuf::from(value)),
                "mod_root" => self.mod_root = Some(PathBuf::from(value)),
                "water.expected_fail" => self.water_expected_fail = parse_bool(value)?,
                "process.timeout_secs" => {
                    let secs = value
                        .parse::<u64>()
                        .map_err(|err| format!("{key}: {err}"))?;
                    if secs == 0 {
                        return Err(format!("{key}: value must be positive"));
                    }
                    self.process_timeout = Duration::from_secs(secs);
                }
                "matrix.hours" => self.matrix.hours = parse_f64_list(value)?,
                "threshold.full_map.max_pure_black_percent" => {
                    self.thresholds.full_map.max_pure_black_percent = parse_f64(key, value)?
                }
                "threshold.full_map.min_distinct_colors" => {
                    self.thresholds.full_map.min_distinct_colors = parse_usize(key, value)?
                }
                "threshold.full_map.min_luma_stddev" => {
                    self.thresholds.full_map.min_luma_stddev = parse_f64(key, value)?
                }
                "threshold.close_up.max_pure_black_percent" => {
                    self.thresholds.close_up.max_pure_black_percent = parse_f64(key, value)?
                }
                "threshold.close_up.min_distinct_colors" => {
                    self.thresholds.close_up.min_distinct_colors = parse_usize(key, value)?
                }
                "threshold.close_up.min_luma_stddev" => {
                    self.thresholds.close_up.min_luma_stddev = parse_f64(key, value)?
                }
                "frame_parity.clear" => {
                    if parse_bool(value)? {
                        self.frame_parity_gates.clear();
                    }
                }
                _ if key.starts_with("frame_parity.") => {
                    self.apply_frame_parity_config(key, value)?
                }
                _ if key.starts_with("camera.") => self.apply_camera_config(key, value)?,
                _ => return Err(format!("unsupported config key {key:?}")),
            }
        }
        Ok(())
    }

    pub fn set_terrain_map_frame_bundle(&mut self, path: PathBuf) {
        self.terrain_map_frame_bundle = path.clone();
        for gate in &mut self.frame_parity_gates {
            if gate.name == "terrain" || gate.name == "water" {
                gate.terrain_map_frame_bundle = path.clone();
            }
            if gate.name == "terrain" {
                gate.compare_map_frame_bundle = path.clone();
            }
        }
    }

    pub fn set_water_map_frame_bundle(&mut self, path: PathBuf) {
        self.water_map_frame_bundle = path.clone();
        for gate in &mut self.frame_parity_gates {
            if gate.name == "water" {
                gate.compare_map_frame_bundle = path.clone();
                gate.water_map_frame_bundle = Some(path.clone());
            }
        }
    }

    fn apply_camera_config(&mut self, key: &str, value: &str) -> Result<(), String> {
        let rest = key
            .strip_prefix("camera.")
            .ok_or_else(|| format!("unsupported camera key {key:?}"))?;
        let (camera_name, field) = rest
            .split_once('.')
            .ok_or_else(|| format!("camera key {key:?} must be camera.<name>.<field>"))?;
        let camera = self
            .matrix
            .cameras
            .iter_mut()
            .find(|camera| camera.config_name == camera_name)
            .ok_or_else(|| format!("unknown camera {camera_name:?}"))?;
        match field {
            "zoom" => camera.zoom = Some(parse_f64(key, value)?),
            "center" => camera.center = Some(parse_pair(key, value)?),
            _ => return Err(format!("unsupported camera field {field:?}")),
        }
        Ok(())
    }

    fn apply_frame_parity_config(&mut self, key: &str, value: &str) -> Result<(), String> {
        let rest = key
            .strip_prefix("frame_parity.")
            .ok_or_else(|| format!("unsupported frame parity key {key:?}"))?;
        let (name, field) = rest.split_once('.').ok_or_else(|| {
            format!("frame parity key {key:?} must be frame_parity.<name>.<field>")
        })?;
        let index = match self
            .frame_parity_gates
            .iter()
            .position(|gate| gate.name == name)
        {
            Some(index) => index,
            None => {
                self.frame_parity_gates
                    .push(FrameParityConfig::named(name.to_string()));
                self.frame_parity_gates.len() - 1
            }
        };
        let gate = &mut self.frame_parity_gates[index];
        match field {
            "enabled" => gate.enabled = parse_bool(value)?,
            "gate" => gate.gate = value.to_string(),
            "pass" => gate.pass = value.to_string(),
            "terrain_map_frame_bundle" => gate.terrain_map_frame_bundle = PathBuf::from(value),
            "compare_map_frame_bundle" => gate.compare_map_frame_bundle = PathBuf::from(value),
            "water_map_frame_bundle" => {
                gate.water_map_frame_bundle = optional_path_value(value);
            }
            "camera_json" => gate.camera_json = optional_path_value(value),
            "hard" => gate.hard = parse_bool(value)?,
            _ => return Err(format!("unsupported frame parity field {field:?}")),
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct FrameParityConfig {
    pub name: String,
    pub gate: String,
    pub pass: String,
    pub terrain_map_frame_bundle: PathBuf,
    pub compare_map_frame_bundle: PathBuf,
    pub water_map_frame_bundle: Option<PathBuf>,
    pub camera_json: Option<PathBuf>,
    pub hard: bool,
    pub enabled: bool,
}

impl FrameParityConfig {
    fn named(name: String) -> Self {
        Self {
            gate: format!("frame parity {name} captured RT"),
            pass: name.clone(),
            name,
            terrain_map_frame_bundle: PathBuf::new(),
            compare_map_frame_bundle: PathBuf::new(),
            water_map_frame_bundle: None,
            camera_json: None,
            hard: true,
            enabled: true,
        }
    }
}

fn default_frame_parity_gates(
    terrain_map_frame_bundle: &Path,
    water_map_frame_bundle: &Path,
    water_ground_truth_available: bool,
) -> Vec<FrameParityConfig> {
    let mut gates = vec![FrameParityConfig {
        name: "terrain".to_string(),
        gate: "frame parity terrain captured RT".to_string(),
        pass: "terrain".to_string(),
        terrain_map_frame_bundle: terrain_map_frame_bundle.to_path_buf(),
        compare_map_frame_bundle: terrain_map_frame_bundle.to_path_buf(),
        water_map_frame_bundle: None,
        camera_json: None,
        hard: true,
        enabled: true,
    }];
    if water_ground_truth_available {
        gates.push(FrameParityConfig {
            name: "water".to_string(),
            gate: "frame parity water captured RT".to_string(),
            pass: "water".to_string(),
            terrain_map_frame_bundle: terrain_map_frame_bundle.to_path_buf(),
            compare_map_frame_bundle: water_map_frame_bundle.to_path_buf(),
            water_map_frame_bundle: Some(water_map_frame_bundle.to_path_buf()),
            camera_json: None,
            hard: true,
            enabled: true,
        });
    }
    gates
}

fn water_frame_ground_truth_available(water_map_frame_bundle: &Path) -> bool {
    if !water_map_frame_bundle.is_file() {
        return false;
    }
    let Ok(text) = fs::read_to_string(water_map_frame_bundle) else {
        return false;
    };
    text.contains("\"capture_moment\": \"water_pass\"")
        && text.contains("\"post_pass_output_merger\"")
        && text.contains("\"post_pass_payload\"")
        && json_has_non_null_value(&text, "last_render_target_clear")
        && json_has_non_null_value(&text, "last_depth_stencil_clear")
}

fn json_has_non_null_value(text: &str, key: &str) -> bool {
    let token = format!("\"{key}\"");
    let mut search_from = 0usize;
    while let Some(relative) = text[search_from..].find(&token) {
        let start = search_from + relative + token.len();
        let Some(colon_relative) = text[start..].find(':') else {
            return false;
        };
        let value_start = skip_json_ws(text.as_bytes(), start + colon_relative + 1);
        if text[value_start..].starts_with("null") {
            search_from = value_start + 4;
            continue;
        }
        return true;
    }
    false
}

#[derive(Clone, Debug)]
pub struct MatrixConfig {
    pub cameras: Vec<MatrixCamera>,
    pub hours: Vec<f64>,
    pub synthesize_terrain_chunks: bool,
}

impl Default for MatrixConfig {
    fn default() -> Self {
        Self {
            cameras: vec![
                MatrixCamera {
                    name: "full-map".to_string(),
                    config_name: "full_map".to_string(),
                    threshold_class: ThresholdClass::FullMap,
                    zoom: None,
                    center: None,
                },
                MatrixCamera {
                    name: "europe-close".to_string(),
                    config_name: "europe_close".to_string(),
                    threshold_class: ThresholdClass::CloseUp,
                    zoom: Some(0.85),
                    center: Some((3030.0, 760.0)),
                },
                MatrixCamera {
                    name: "pacific-close".to_string(),
                    config_name: "pacific_close".to_string(),
                    threshold_class: ThresholdClass::CloseUp,
                    zoom: Some(0.85),
                    center: Some((5150.0, 920.0)),
                },
            ],
            hours: vec![0.5, 0.9],
            synthesize_terrain_chunks: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct MatrixCamera {
    pub name: String,
    pub config_name: String,
    pub threshold_class: ThresholdClass,
    pub zoom: Option<f64>,
    pub center: Option<(f64, f64)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThresholdClass {
    FullMap,
    CloseUp,
}

#[derive(Clone, Debug)]
pub struct ThresholdConfig {
    pub full_map: ScenarioThresholds,
    pub close_up: ScenarioThresholds,
}

impl Default for ThresholdConfig {
    fn default() -> Self {
        Self {
            full_map: ScenarioThresholds {
                max_pure_black_percent: 20.0,
                min_distinct_colors: 8,
                min_luma_stddev: 1.0,
            },
            close_up: ScenarioThresholds {
                max_pure_black_percent: 30.0,
                min_distinct_colors: 8,
                min_luma_stddev: 0.5,
            },
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ScenarioThresholds {
    pub max_pure_black_percent: f64,
    pub min_distinct_colors: usize,
    pub min_luma_stddev: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PixelStats {
    pub width: u32,
    pub height: u32,
    pub mean_luma: f64,
    pub luma_stddev: f64,
    pub pure_black_percent: f64,
    pub distinct_color_count: usize,
}

pub fn compute_bmp_stats_file(path: &Path) -> Result<PixelStats, String> {
    let bytes = fs::read(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    compute_bmp_stats(&bytes)
}

pub fn compute_bmp_stats(bytes: &[u8]) -> Result<PixelStats, String> {
    if bytes.len() < 54 {
        return Err("BMP is shorter than the minimum header".to_string());
    }
    if &bytes[0..2] != b"BM" {
        return Err("BMP magic is not BM".to_string());
    }
    let pixel_offset = read_u32(bytes, 10)? as usize;
    let dib_size = read_u32(bytes, 14)?;
    if dib_size < 40 {
        return Err(format!("unsupported DIB header size {dib_size}"));
    }
    let width = read_i32(bytes, 18)?;
    let signed_height = read_i32(bytes, 22)?;
    if width <= 0 || signed_height == 0 {
        return Err(format!("invalid BMP dimensions {width}x{signed_height}"));
    }
    let width = width as u32;
    let height = signed_height.unsigned_abs();
    let bits_per_pixel = read_u16(bytes, 28)?;
    let compression = read_u32(bytes, 30)?;
    if compression != 0 {
        return Err(format!("unsupported compressed BMP type {compression}"));
    }
    if bits_per_pixel != 24 && bits_per_pixel != 32 {
        return Err(format!("unsupported BMP bit depth {bits_per_pixel}"));
    }
    // Header-supplied dimensions are untrusted: a malformed BMP must surface
    // as a typed error, never wrap the size math and index out of bounds.
    let row_bits = (width as usize)
        .checked_mul(bits_per_pixel as usize)
        .and_then(|bits| bits.checked_add(31))
        .ok_or_else(|| "BMP dimensions overflow".to_string())?;
    let row_stride = (row_bits / 32) * 4;
    let required = row_stride
        .checked_mul(height as usize)
        .and_then(|pixel_bytes| pixel_offset.checked_add(pixel_bytes))
        .ok_or_else(|| "BMP dimensions overflow".to_string())?;
    if required > isize::MAX as usize {
        return Err("BMP dimensions overflow the maximum addressable slice length".to_string());
    }
    if bytes.len() < required {
        return Err(format!(
            "BMP pixel payload is {} bytes, expected at least {required}",
            bytes.len()
        ));
    }

    let bytes_per_pixel = (bits_per_pixel / 8) as usize;
    let mut distinct = HashSet::new();
    let mut pure_black = 0usize;
    let mut luma_sum = 0.0;
    let mut luma_square_sum = 0.0;
    let pixel_count = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| "BMP dimensions overflow".to_string())?;
    for row in 0..height as usize {
        let source_row = if signed_height > 0 {
            height as usize - 1 - row
        } else {
            row
        };
        let row_start = pixel_offset + source_row * row_stride;
        for x in 0..width as usize {
            let pixel_start = row_start + x * bytes_per_pixel;
            let b = bytes[pixel_start] as u32;
            let g = bytes[pixel_start + 1] as u32;
            let r = bytes[pixel_start + 2] as u32;
            if r == 0 && g == 0 && b == 0 {
                pure_black += 1;
            }
            distinct.insert((r << 16) | (g << 8) | b);
            let luma = 0.2126 * r as f64 + 0.7152 * g as f64 + 0.0722 * b as f64;
            luma_sum += luma;
            luma_square_sum += luma * luma;
        }
    }
    let mean_luma = luma_sum / pixel_count as f64;
    let variance = (luma_square_sum / pixel_count as f64) - mean_luma * mean_luma;
    Ok(PixelStats {
        width,
        height,
        mean_luma,
        luma_stddev: variance.max(0.0).sqrt(),
        pure_black_percent: pure_black as f64 * 100.0 / pixel_count as f64,
        distinct_color_count: distinct.len(),
    })
}

pub fn evaluate_stats(stats: &PixelStats, thresholds: ScenarioThresholds) -> Vec<String> {
    let mut failures = Vec::new();
    if stats.pure_black_percent > thresholds.max_pure_black_percent {
        failures.push(format!(
            "pure-black {:.2}% exceeds {:.2}%",
            stats.pure_black_percent, thresholds.max_pure_black_percent
        ));
    }
    if stats.distinct_color_count < thresholds.min_distinct_colors {
        failures.push(format!(
            "distinct colors {} below {}",
            stats.distinct_color_count, thresholds.min_distinct_colors
        ));
    }
    if stats.luma_stddev < thresholds.min_luma_stddev {
        failures.push(format!(
            "luma stddev {:.3} below {:.3}",
            stats.luma_stddev, thresholds.min_luma_stddev
        ));
    }
    failures
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateStatus {
    Pass,
    Fail,
    ExpectedFailStillFailing,
    UnexpectedlyPassing,
}

impl GateStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::ExpectedFailStillFailing => "expected_fail_still_failing",
            Self::UnexpectedlyPassing => "unexpectedly_passing",
        }
    }
}

#[derive(Clone, Debug)]
pub struct GateRecord {
    pub gate: String,
    pub status: GateStatus,
    pub hard: bool,
    pub key_numbers: BTreeMap<String, String>,
    pub artifacts: Vec<PathBuf>,
    pub details: Vec<String>,
}

impl GateRecord {
    fn hard_pass(gate: impl Into<String>) -> Self {
        Self {
            gate: gate.into(),
            status: GateStatus::Pass,
            hard: true,
            key_numbers: BTreeMap::new(),
            artifacts: Vec::new(),
            details: Vec::new(),
        }
    }

    fn hard_fail(gate: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            gate: gate.into(),
            status: GateStatus::Fail,
            hard: true,
            key_numbers: BTreeMap::new(),
            artifacts: Vec::new(),
            details: vec![detail.into()],
        }
    }
}

#[derive(Clone, Debug)]
pub struct ToolInfo {
    pub name: String,
    pub package: String,
    pub path: PathBuf,
    pub built_by_harness: bool,
}

#[derive(Clone, Debug)]
pub struct ReplayQaReport {
    pub schema_version: u32,
    pub git_revision: Option<String>,
    pub out_dir: PathBuf,
    pub tools: Vec<ToolInfo>,
    pub gates: Vec<GateRecord>,
}

impl ReplayQaReport {
    pub fn has_hard_failures(&self) -> bool {
        self.gates
            .iter()
            .any(|gate| gate.hard && gate.status == GateStatus::Fail)
    }

    pub fn to_json(&self) -> String {
        let mut out = String::new();
        out.push_str("{\n");
        write!(
            out,
            "  \"schema_version\": {},\n  \"out_dir\": {},\n",
            self.schema_version,
            json_string(&path_string(&self.out_dir))
        )
        .unwrap();
        match &self.git_revision {
            Some(rev) => {
                writeln!(out, "  \"git_revision\": {},", json_string(rev)).unwrap();
            }
            None => out.push_str("  \"git_revision\": null,\n"),
        }
        out.push_str("  \"tools\": [\n");
        for (index, tool) in self.tools.iter().enumerate() {
            let comma = if index + 1 == self.tools.len() {
                ""
            } else {
                ","
            };
            write!(
                out,
                "    {{\"name\": {}, \"package\": {}, \"path\": {}, \"built_by_harness\": {}}}{comma}\n",
                json_string(&tool.name),
                json_string(&tool.package),
                json_string(&path_string(&tool.path)),
                tool.built_by_harness
            )
            .unwrap();
        }
        out.push_str("  ],\n  \"gates\": [\n");
        for (index, gate) in self.gates.iter().enumerate() {
            let comma = if index + 1 == self.gates.len() {
                ""
            } else {
                ","
            };
            out.push_str("    {\n");
            write!(
                out,
                "      \"gate\": {},\n      \"status\": {},\n      \"hard\": {},\n",
                json_string(&gate.gate),
                json_string(gate.status.as_str()),
                gate.hard
            )
            .unwrap();
            out.push_str("      \"key_numbers\": {");
            for (item_index, (key, value)) in gate.key_numbers.iter().enumerate() {
                if item_index > 0 {
                    out.push_str(", ");
                }
                write!(out, "{}: {}", json_string(key), json_string(value)).unwrap();
            }
            out.push_str("},\n      \"artifacts\": [");
            for (item_index, artifact) in gate.artifacts.iter().enumerate() {
                if item_index > 0 {
                    out.push_str(", ");
                }
                out.push_str(&json_string(&path_string(artifact)));
            }
            out.push_str("],\n      \"details\": [");
            for (item_index, detail) in gate.details.iter().enumerate() {
                if item_index > 0 {
                    out.push_str(", ");
                }
                out.push_str(&json_string(detail));
            }
            write!(out, "]\n    }}{comma}\n").unwrap();
        }
        out.push_str("  ]\n}\n");
        out
    }

    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str("# Replay QA Report\n\n");
        if let Some(rev) = &self.git_revision {
            writeln!(out, "Git revision: `{}`\n", md_escape(rev)).unwrap();
        }
        writeln!(out, "Output directory: `{}`\n", self.out_dir.display()).unwrap();
        out.push_str("| Gate | Status | Key numbers | Artifact paths |\n");
        out.push_str("|---|---|---|---|\n");
        for gate in &self.gates {
            let key_numbers = if gate.key_numbers.is_empty() {
                "-".to_string()
            } else {
                gate.key_numbers
                    .iter()
                    .map(|(key, value)| format!("{}={}", md_escape(key), md_escape(value)))
                    .collect::<Vec<_>>()
                    .join("; ")
            };
            let artifacts = if gate.artifacts.is_empty() {
                "-".to_string()
            } else {
                gate.artifacts
                    .iter()
                    .map(|path| md_escape(&path_string(path)))
                    .collect::<Vec<_>>()
                    .join("<br>")
            };
            writeln!(
                out,
                "| {} | {} | {} | {} |",
                md_escape(&gate.gate),
                md_escape(gate.status.as_str()),
                key_numbers,
                artifacts
            )
            .unwrap();
        }
        out.push_str("\n## Tools\n\n");
        out.push_str("| Tool | Package | Path | Built by harness |\n");
        out.push_str("|---|---|---|---|\n");
        for tool in &self.tools {
            writeln!(
                out,
                "| {} | {} | {} | {} |",
                md_escape(&tool.name),
                md_escape(&tool.package),
                md_escape(&path_string(&tool.path)),
                tool.built_by_harness
            )
            .unwrap();
        }
        out
    }
}

pub fn run_harness(config: &ReplayQaConfig) -> Result<ReplayQaReport, String> {
    fs::create_dir_all(&config.out_dir)
        .map_err(|err| format!("create {}: {err}", config.out_dir.display()))?;
    fs::create_dir_all(config.out_dir.join("logs"))
        .map_err(|err| format!("create logs dir: {err}"))?;

    let mut report = ReplayQaReport {
        schema_version: 1,
        git_revision: discover_git_revision(&config.repo_root),
        out_dir: config.out_dir.clone(),
        tools: Vec::new(),
        gates: Vec::new(),
    };

    let map_viewer = ensure_binary(config, "map_viewer", "map_viewer", None, &mut report);
    let replay_app = ensure_binary(config, "replay_app", "replay_app", None, &mut report);
    let ui_probe = ensure_binary(config, "game_ui", "ui_probe", Some("ui_probe"), &mut report);
    let map_bundle = if let Some(map_viewer) = map_viewer.as_deref() {
        ensure_map_bundle(config, map_viewer, &mut report)?
    } else {
        None
    };

    if let Some(map_viewer) = map_viewer.as_deref() {
        run_byte_exact_gates(config, map_viewer, &mut report)?;
        run_frame_parity_gates(config, map_viewer, &mut report)?;
    } else {
        report.gates.push(GateRecord::hard_fail(
            "byte-exact gates",
            "map_viewer binary is unavailable",
        ));
    }

    if let (Some(replay_app), Some(map_bundle)) = (replay_app.as_deref(), map_bundle.as_deref()) {
        run_render_matrix(config, replay_app, map_bundle, &mut report)?;
        // The recording-ingest/scrub gate was retired because production recordings refuse
        // its debug CLI surfaces. Its properties now live in
        // replay_app/tests/recording_dump_cli.rs and replay_app/src/timeline.rs.
    } else {
        report.gates.push(GateRecord::hard_fail(
            "render matrix",
            "replay_app binary or map bundle is unavailable",
        ));
    }

    if let Some(ui_probe) = ui_probe.as_deref() {
        run_ui_probe_gate(config, ui_probe, &mut report)?;
    } else {
        report.gates.push(GateRecord::hard_fail(
            "ui composition gates",
            "ui_probe binary is unavailable",
        ));
    }

    Ok(report)
}

fn run_byte_exact_gates(
    config: &ReplayQaConfig,
    map_viewer: &Path,
    report: &mut ReplayQaReport,
) -> Result<(), String> {
    let terrain_report_dir = config.out_dir.join("terrain_byte_exact");
    let terrain_args = vec![
        "render-production-map-d3d11".to_string(),
        "--terrain-map-frame-bundle".to_string(),
        path_string(&config.terrain_map_frame_bundle),
        "--parity-report".to_string(),
        path_string(&terrain_report_dir),
    ];
    let terrain = run_command_gate(
        config,
        "terrain byte-exact reference compare",
        map_viewer,
        &terrain_args,
        true,
        "terrain_byte_exact",
    )?;
    report.gates.push(terrain);

    let synthesis_report = config
        .out_dir
        .join("terrain_chunk_synthesis_validation.json");
    let synthesis_args = vec![
        "validate-terrain-chunk-synthesis".to_string(),
        "--terrain-chunk-library".to_string(),
        path_string(&config.terrain_chunk_library),
        "--report".to_string(),
        path_string(&synthesis_report),
    ];
    let mut synthesis = run_command_gate(
        config,
        "terrain chunk synthesis validation",
        map_viewer,
        &synthesis_args,
        true,
        "terrain_chunk_synthesis",
    )?;
    synthesis.artifacts.push(synthesis_report);
    report.gates.push(synthesis);

    let water_report_dir = config.out_dir.join("terrain_water_byte_exact");
    let water_args = vec![
        "render-production-map-d3d11".to_string(),
        "--terrain-map-frame-bundle".to_string(),
        path_string(&config.terrain_map_frame_bundle),
        "--water-map-frame-bundle".to_string(),
        path_string(&config.water_map_frame_bundle),
        "--parity-report".to_string(),
        path_string(&water_report_dir),
    ];
    let mut water = run_command_gate(
        config,
        "terrain+water reference compare",
        map_viewer,
        &water_args,
        !config.water_expected_fail,
        "terrain_water_byte_exact",
    )?;
    if config.water_expected_fail {
        water.hard = false;
        water.status = if water.status == GateStatus::Pass {
            GateStatus::UnexpectedlyPassing
        } else {
            GateStatus::ExpectedFailStillFailing
        };
    }
    report.gates.push(water);
    Ok(())
}

fn run_frame_parity_gates(
    config: &ReplayQaConfig,
    map_viewer: &Path,
    report: &mut ReplayQaReport,
) -> Result<(), String> {
    for parity in config
        .frame_parity_gates
        .iter()
        .filter(|parity| parity.enabled)
    {
        report
            .gates
            .push(run_frame_parity_gate(config, map_viewer, parity)?);
    }
    Ok(())
}

fn run_frame_parity_gate(
    config: &ReplayQaConfig,
    map_viewer: &Path,
    parity: &FrameParityConfig,
) -> Result<GateRecord, String> {
    let parity_dir = config
        .out_dir
        .join("frame_parity")
        .join(safe_stem(&parity.name));
    fs::create_dir_all(&parity_dir)
        .map_err(|err| format!("create {}: {err}", parity_dir.display()))?;
    let replay_raw = parity_dir.join("replay_final.raw");
    let heatmap = parity_dir.join("diff_heatmap.png");
    let viewer_report_dir = parity_dir.join("map_viewer_parity");
    let mut args = vec![
        "render-production-map-d3d11".to_string(),
        "--terrain-map-frame-bundle".to_string(),
        path_string(&parity.terrain_map_frame_bundle),
        "--out".to_string(),
        path_string(&replay_raw),
        "--parity-report".to_string(),
        path_string(&viewer_report_dir),
    ];
    if let Some(water) = parity.water_map_frame_bundle.as_ref() {
        args.push("--water-map-frame-bundle".to_string());
        args.push(path_string(water));
    }
    if let Some(camera_json) = parity.camera_json.as_ref() {
        args.push("--camera-json".to_string());
        args.push(path_string(camera_json));
    }

    let output = run_process(
        map_viewer,
        &args,
        &config.native_root,
        config.process_timeout,
    );
    let mut artifacts = write_command_logs(
        config,
        &format!("frame_parity_{}", parity.name),
        map_viewer,
        &args,
        &output,
    )?;
    artifacts.push(replay_raw.clone());
    artifacts.push(heatmap.clone());
    artifacts.push(viewer_report_dir.clone());

    let mut gate = GateRecord {
        gate: parity.gate.clone(),
        status: if output.success() {
            GateStatus::Pass
        } else {
            GateStatus::Fail
        },
        hard: parity.hard,
        key_numbers: BTreeMap::new(),
        artifacts,
        details: Vec::new(),
    };
    gate.key_numbers
        .insert("exit_code".to_string(), exit_code_string(output.exit_code));
    gate.key_numbers
        .insert("duration_ms".to_string(), output.duration_ms.to_string());
    gate.key_numbers
        .insert("pass".to_string(), parity.pass.clone());
    gate.key_numbers.insert(
        "camera".to_string(),
        parity
            .camera_json
            .as_ref()
            .map(|path| path_string(path))
            .unwrap_or_else(|| "captured_scene_cb0".to_string()),
    );
    // Do not `?` here: an unknown pass must fail THIS gate (typed, via the
    // compute_frame_parity_diff Err arm below), not abort the whole run.
    match resolve_map_frame_bundle_path(&parity.compare_map_frame_bundle, &parity.pass) {
        Ok(ground_truth) => {
            gate.key_numbers
                .insert("ground_truth".to_string(), path_string(&ground_truth));
        }
        Err(err) => {
            gate.key_numbers
                .insert("ground_truth_error".to_string(), err);
        }
    }

    match compute_frame_parity_diff(&parity.compare_map_frame_bundle, &parity.pass, &replay_raw) {
        Ok(diff) => {
            write_png_rgba(&heatmap, diff.width, diff.height, &diff.heatmap_rgba)?;
            gate.key_numbers
                .insert("width".to_string(), diff.width.to_string());
            gate.key_numbers
                .insert("height".to_string(), diff.height.to_string());
            gate.key_numbers
                .insert("dxgi_format".to_string(), diff.dxgi_format.to_string());
            gate.key_numbers
                .insert("exact_match".to_string(), diff.exact_match.to_string());
            gate.key_numbers.insert(
                "differing_pixels".to_string(),
                diff.differing_pixels.to_string(),
            );
            gate.key_numbers.insert(
                "differing_percent".to_string(),
                format!("{:.6}", diff.differing_percent),
            );
            gate.key_numbers.insert(
                "match_percent".to_string(),
                format!("{:.6}", 100.0 - diff.differing_percent),
            );
            gate.key_numbers.insert(
                "max_channel_delta".to_string(),
                format_delta(diff.max_channel_delta),
            );
            gate.key_numbers
                .insert("captured_rt".to_string(), diff.captured_file);
            gate.key_numbers
                .insert("heatmap".to_string(), path_string(&heatmap));
            if !diff.exact_match {
                gate.status = GateStatus::Fail;
                gate.details.push(format!(
                    "captured RT and replay raw differ: {} pixels ({:.6}%), max channel delta {}",
                    diff.differing_pixels,
                    diff.differing_percent,
                    format_delta(diff.max_channel_delta)
                ));
            }
        }
        Err(err) => {
            gate.status = GateStatus::Fail;
            gate.details.push(err);
        }
    }
    if !output.success() {
        gate.status = GateStatus::Fail;
        if let Some(line) =
            last_nonempty_line(&output.stderr).or_else(|| last_nonempty_line(&output.stdout))
        {
            gate.details.push(line.to_string());
        }
    }
    Ok(gate)
}

fn run_render_matrix(
    config: &ReplayQaConfig,
    replay_app: &Path,
    bundle_dir: &Path,
    report: &mut ReplayQaReport,
) -> Result<(), String> {
    let matrix_dir = config.out_dir.join("matrix");
    fs::create_dir_all(&matrix_dir).map_err(|err| format!("create matrix dir: {err}"))?;
    for camera in &config.matrix.cameras {
        for hour in &config.matrix.hours {
            let hour_label = hour_label(*hour);
            let bmp_path = matrix_dir.join(format!("{}_h{}_synth.bmp", camera.name, hour_label));
            let mut args = vec![
                "--bundle".to_string(),
                path_string(bundle_dir),
                "--capture".to_string(),
                path_string(&config.capture_dir),
                "--dump-frame".to_string(),
                path_string(&bmp_path),
                "--hour".to_string(),
                trim_float(*hour),
            ];
            if let Some(game_root) = &config.game_root {
                args.push("--game".to_string());
                args.push(path_string(game_root));
            }
            if let Some(zoom) = camera.zoom {
                args.push("--dump-zoom".to_string());
                args.push(trim_float(zoom));
            }
            if let Some((x, y)) = camera.center {
                args.push("--dump-center".to_string());
                args.push(format!("{},{}", trim_float(x), trim_float(y)));
            }
            if config.matrix.synthesize_terrain_chunks {
                args.push("--synthesize-terrain-chunks".to_string());
            }

            let gate_name = format!("render matrix {} hour {}", camera.name, trim_float(*hour));
            let mut gate = run_command_gate(
                config,
                &gate_name,
                replay_app,
                &args,
                true,
                &format!("matrix_{}_h{}", camera.name, hour_label),
            )?;
            gate.artifacts.push(bmp_path.clone());
            gate.key_numbers
                .insert("camera".to_string(), camera.name.clone());
            gate.key_numbers
                .insert("hour".to_string(), trim_float(*hour));
            if gate.status == GateStatus::Pass {
                match compute_bmp_stats_file(&bmp_path) {
                    Ok(stats) => {
                        let thresholds = match camera.threshold_class {
                            ThresholdClass::FullMap => config.thresholds.full_map,
                            ThresholdClass::CloseUp => config.thresholds.close_up,
                        };
                        let failures = evaluate_stats(&stats, thresholds);
                        push_stats_numbers(&mut gate.key_numbers, &stats);
                        if !failures.is_empty() {
                            gate.status = GateStatus::Fail;
                            gate.details.extend(failures);
                        }
                    }
                    Err(err) => {
                        gate.status = GateStatus::Fail;
                        gate.details.push(err);
                    }
                }
            }
            report.gates.push(gate);
        }
    }
    // Day/night must actually change pixels: for each camera, the dumps at
    // the first and last matrix hours must NOT be byte-identical. Guards the
    // "reports hour X but renders identically" defect class caught 2026-07-03.
    if config.matrix.hours.len() >= 2 {
        let first = hour_label(config.matrix.hours[0]);
        let last = hour_label(*config.matrix.hours.last().unwrap());
        for camera in &config.matrix.cameras {
            let gate_name = format!("day/night divergence {}", camera.name);
            let mut gate = if first == last {
                // Identical endpoints make the divergence check unsatisfiable:
                // both paths would resolve to the same file and the comparison
                // would misreport a config defect as a renderer no-op. Do not
                // silently skip either — fail typed on the config.
                GateRecord::hard_fail(
                    gate_name,
                    format!(
                        "matrix.hours first/last endpoints resolve to the same hour label \
                         {first:?}; day/night divergence is not checkable — fix matrix.hours \
                         in the config"
                    ),
                )
            } else {
                let a = matrix_dir.join(format!("{}_h{}_synth.bmp", camera.name, first));
                let b = matrix_dir.join(format!("{}_h{}_synth.bmp", camera.name, last));
                let mut gate = match (fs::read(&a), fs::read(&b)) {
                    (Ok(da), Ok(db)) if da != db => GateRecord::hard_pass(gate_name),
                    (Ok(_), Ok(_)) => GateRecord::hard_fail(
                        gate_name,
                        format!(
                            "hour {} and hour {} dumps are byte-identical — day/night is a visual no-op",
                            trim_float(config.matrix.hours[0]),
                            trim_float(*config.matrix.hours.last().unwrap())
                        ),
                    ),
                    _ => GateRecord::hard_fail(
                        gate_name,
                        "matrix dump missing for comparison".to_string(),
                    ),
                };
                gate.artifacts.push(a);
                gate.artifacts.push(b);
                gate
            };
            gate.key_numbers
                .insert("camera".to_string(), camera.name.clone());
            report.gates.push(gate);
        }
    }
    Ok(())
}

fn run_ui_probe_gate(
    config: &ReplayQaConfig,
    ui_probe: &Path,
    report: &mut ReplayQaReport,
) -> Result<(), String> {
    let ui_dir = config.out_dir.join("ui_probe");
    let mut args = vec!["--out".to_string(), path_string(&ui_dir)];
    if let Some(game_root) = &config.game_root {
        args.push("--game-root".to_string());
        args.push(path_string(game_root));
    }
    if let Some(mod_root) = &config.mod_root {
        args.push("--mod-root".to_string());
        args.push(path_string(mod_root));
    }
    let output = run_process(ui_probe, &args, &config.native_root, config.process_timeout);
    let mut artifacts = write_command_logs(config, "ui_probe", ui_probe, &args, &output)?;
    artifacts.push(ui_dir);
    let mut gate = GateRecord::hard_pass("ui composition gates");
    gate.artifacts = artifacts;
    gate.key_numbers
        .insert("exit_code".to_string(), exit_code_string(output.exit_code));
    gate.key_numbers
        .insert("duration_ms".to_string(), output.duration_ms.to_string());
    let evaluation = evaluate_ui_probe_text(&output.stdout, output.success());
    gate.key_numbers.extend(evaluation.key_numbers);
    gate.details.extend(evaluation.gate_lines);
    if !evaluation.failures.is_empty() {
        gate.status = GateStatus::Fail;
        gate.details.extend(evaluation.failures);
    }
    if !output.success() {
        gate.status = GateStatus::Fail;
        if let Some(line) = last_nonempty_line(&output.stderr) {
            gate.details.push(line.to_string());
        }
    }
    report.gates.push(gate);
    Ok(())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UiProbeEvaluation {
    pub key_numbers: BTreeMap<String, String>,
    pub gate_lines: Vec<String>,
    pub failures: Vec<String>,
}

pub fn evaluate_ui_probe_text(text: &str, exit_success: bool) -> UiProbeEvaluation {
    let mut evaluation = UiProbeEvaluation::default();
    if !exit_success {
        evaluation
            .failures
            .push("ui_probe exited nonzero".to_string());
    }
    let mut seen = HashSet::new();
    for line in text.lines() {
        if line.starts_with("gfx files=") {
            seen.insert("gfx");
            evaluation.gate_lines.push(line.to_string());
            let values = parse_kv_tokens(line);
            copy_keys(
                &mut evaluation.key_numbers,
                "gfx",
                &values,
                &["files", "sprites", "parse_failures"],
            );
            require_zero(
                &mut evaluation.failures,
                "gfx parse failures",
                &values,
                "parse_failures",
            );
        } else if line.starts_with("gui files=") {
            seen.insert("gui");
            evaluation.gate_lines.push(line.to_string());
            let values = parse_kv_tokens(line);
            copy_keys(
                &mut evaluation.key_numbers,
                "gui",
                &values,
                &["files", "windows", "elements", "parse_failures"],
            );
            require_zero(
                &mut evaluation.failures,
                "gui parse failures",
                &values,
                "parse_failures",
            );
        } else if line.starts_with("topbar sprite_refs=") {
            seen.insert("topbar");
            evaluation.gate_lines.push(line.to_string());
            let values = parse_kv_tokens(line);
            copy_keys(
                &mut evaluation.key_numbers,
                "topbar",
                &values,
                &[
                    "sprite_refs",
                    "resolved_textures",
                    "decoded_textures",
                    "missing",
                    "undecodable",
                ],
            );
            require_positive(
                &mut evaluation.failures,
                "topbar sprite references",
                &values,
                "sprite_refs",
            );
            require_positive(
                &mut evaluation.failures,
                "topbar resolved textures",
                &values,
                "resolved_textures",
            );
            require_zero(
                &mut evaluation.failures,
                "topbar missing textures",
                &values,
                "missing",
            );
            require_zero(
                &mut evaluation.failures,
                "topbar undecodable textures",
                &values,
                "undecodable",
            );
        } else if line.starts_with("topbar composition sample:") {
            seen.insert("composition_sample");
            evaluation.gate_lines.push(line.to_string());
            let values = parse_kv_tokens(line);
            copy_keys(
                &mut evaluation.key_numbers,
                "composition_sample",
                &values,
                &[
                    "drawn",
                    "missing_sprites",
                    "undecodable_sprites",
                    "missing_glyph_sets",
                ],
            );
            require_zero(
                &mut evaluation.failures,
                "sample composition missing sprites",
                &values,
                "missing_sprites",
            );
            require_zero(
                &mut evaluation.failures,
                "sample composition undecodable sprites",
                &values,
                "undecodable_sprites",
            );
            require_zero(
                &mut evaluation.failures,
                "sample composition missing glyph sets",
                &values,
                "missing_glyph_sets",
            );
        } else if line.starts_with("topbar composition empty:") {
            seen.insert("composition_empty");
            evaluation.gate_lines.push(line.to_string());
            let values = parse_kv_tokens(line);
            copy_keys(
                &mut evaluation.key_numbers,
                "composition_empty",
                &values,
                &[
                    "drawn",
                    "missing_sprites",
                    "undecodable_sprites",
                    "missing_glyph_sets",
                ],
            );
            require_zero(
                &mut evaluation.failures,
                "empty composition missing sprites",
                &values,
                "missing_sprites",
            );
            require_zero(
                &mut evaluation.failures,
                "empty composition undecodable sprites",
                &values,
                "undecodable_sprites",
            );
            require_zero(
                &mut evaluation.failures,
                "empty composition missing glyph sets",
                &values,
                "missing_glyph_sets",
            );
        } else if line.starts_with("fonts checked=") {
            seen.insert("fonts");
            evaluation.gate_lines.push(line.to_string());
            let values = parse_kv_tokens(line);
            copy_keys(
                &mut evaluation.key_numbers,
                "fonts",
                &values,
                &[
                    "checked",
                    "missing_ascii_sets",
                    "missing_topbar_sample_sets",
                ],
            );
            require_zero(
                &mut evaluation.failures,
                "font missing ASCII sets",
                &values,
                "missing_ascii_sets",
            );
            require_zero(
                &mut evaluation.failures,
                "font missing topbar sample sets",
                &values,
                "missing_topbar_sample_sets",
            );
        }
    }
    let png_count = count_png_dump_lines(text);
    evaluation
        .key_numbers
        .insert("png_dumps".to_string(), png_count.to_string());
    if png_count < 8 {
        evaluation
            .failures
            .push(format!("png dumps {png_count} below 8"));
    }
    for required in [
        "gfx",
        "gui",
        "topbar",
        "composition_sample",
        "composition_empty",
        "fonts",
    ] {
        if !seen.contains(required) {
            evaluation
                .failures
                .push(format!("ui_probe output missing {required} gate line"));
        }
    }
    evaluation
}

fn ensure_binary(
    config: &ReplayQaConfig,
    package: &str,
    binary_name: &str,
    explicit_bin: Option<&str>,
    report: &mut ReplayQaReport,
) -> Option<PathBuf> {
    let path = target_debug_binary(&config.native_root, binary_name);
    // Always rebuild: an existing-but-stale binary silently tests OLD code
    // (a stale replay_app.exe made the 2026-07-03 matrix run with the layer
    // composition missing while cargo-run invocations showed it working).
    // cargo makes an up-to-date rebuild a cheap no-op, so never skip it.
    let mut args = vec!["build".to_string(), "-p".to_string(), package.to_string()];
    if let Some(bin) = explicit_bin {
        args.push("--bin".to_string());
        args.push(bin.to_string());
    }
    let output = run_process(
        Path::new("cargo"),
        &args,
        &config.native_root,
        config.process_timeout,
    );
    let log_artifacts = write_command_logs(
        config,
        &format!("build_{binary_name}"),
        Path::new("cargo"),
        &args,
        &output,
    );
    if !output.success() {
        let mut gate = GateRecord::hard_fail(
            format!("build tool {binary_name}"),
            last_nonempty_line(&output.stderr)
                .unwrap_or("cargo build failed")
                .to_string(),
        );
        gate.key_numbers
            .insert("exit_code".to_string(), exit_code_string(output.exit_code));
        // Record the build-failure gate even when the log files could not be
        // written (full/readonly disk): dropping it would hide the compiler
        // diagnosis behind the generic "binary is unavailable" failure.
        match log_artifacts {
            Ok(mut artifacts) => gate.artifacts.append(&mut artifacts),
            Err(err) => gate.details.push(err),
        }
        report.gates.push(gate);
    }
    if output.success() && path.is_file() {
        report.tools.push(ToolInfo {
            name: binary_name.to_string(),
            package: package.to_string(),
            path: path.clone(),
            built_by_harness: true,
        });
        Some(path)
    } else {
        None
    }
}

fn ensure_map_bundle(
    config: &ReplayQaConfig,
    map_viewer: &Path,
    report: &mut ReplayQaReport,
) -> Result<Option<PathBuf>, String> {
    if config.bundle_dir.is_dir() {
        return Ok(Some(config.bundle_dir.clone()));
    }
    let harness_bundle = config.out_dir.join("map_bundle");
    if harness_bundle.is_dir() {
        return Ok(Some(harness_bundle));
    }
    let Some(game_root) = config.game_root.as_ref() else {
        report.gates.push(GateRecord::hard_fail(
            "build map bundle",
            "configured bundle is missing and no game root is available",
        ));
        return Ok(None);
    };
    let args = vec![
        "bundle".to_string(),
        "--game".to_string(),
        path_string(game_root),
        "--out".to_string(),
        path_string(&harness_bundle),
        "--force".to_string(),
    ];
    let mut gate = run_command_gate(
        config,
        "build map bundle",
        map_viewer,
        &args,
        true,
        "build_map_bundle",
    )?;
    gate.artifacts.push(harness_bundle.clone());
    gate.key_numbers
        .insert("game_root".to_string(), path_string(game_root));
    if gate.status == GateStatus::Pass && !harness_bundle.is_dir() {
        gate.status = GateStatus::Fail;
        gate.details
            .push("map_viewer exited 0 but bundle directory was not created".to_string());
    }
    let success = gate.status == GateStatus::Pass;
    report.gates.push(gate);
    Ok(success.then_some(harness_bundle))
}

fn run_command_gate(
    config: &ReplayQaConfig,
    gate_name: &str,
    program: &Path,
    args: &[String],
    hard: bool,
    log_stem: &str,
) -> Result<GateRecord, String> {
    let output = run_process(program, args, &config.native_root, config.process_timeout);
    let artifacts = write_command_logs(config, log_stem, program, args, &output)?;
    let mut gate = GateRecord {
        gate: gate_name.to_string(),
        status: if output.success() {
            GateStatus::Pass
        } else {
            GateStatus::Fail
        },
        hard,
        key_numbers: BTreeMap::new(),
        artifacts,
        details: Vec::new(),
    };
    gate.key_numbers
        .insert("exit_code".to_string(), exit_code_string(output.exit_code));
    gate.key_numbers
        .insert("duration_ms".to_string(), output.duration_ms.to_string());
    if output.success() {
        // On success the first line serves as a summary of what the tool did.
        if let Some(line) =
            first_nonempty_line(&output.stderr).or_else(|| first_nonempty_line(&output.stdout))
        {
            gate.details.push(line.to_string());
        }
    } else if let Some(line) =
        last_nonempty_line(&output.stderr).or_else(|| last_nonempty_line(&output.stdout))
    {
        gate.details.push(line.to_string());
    }
    Ok(gate)
}

#[derive(Clone, Debug)]
struct ProcessResult {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    duration_ms: u128,
}

impl ProcessResult {
    fn success(&self) -> bool {
        self.exit_code == Some(0)
    }
}

fn run_process(
    program: &Path,
    args: &[String],
    current_dir: &Path,
    timeout: Duration,
) -> ProcessResult {
    let started = Instant::now();
    let mut child = match Command::new(program)
        .args(args)
        .current_dir(current_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            return ProcessResult {
                exit_code: None,
                stdout: String::new(),
                stderr: format!("failed to spawn {}: {err}", program.display()),
                duration_ms: started.elapsed().as_millis(),
            }
        }
    };
    // Drain both pipes on dedicated threads from the start. Without concurrent
    // draining, a child that writes more than the OS pipe buffer (e.g. cargo
    // build stderr) blocks on the full pipe and the deadline below would kill
    // healthy builds.
    let stdout_drain = spawn_pipe_drain(child.stdout.take());
    let stderr_drain = spawn_pipe_drain(child.stderr.take());
    let mut failure_note = None;
    let exit_code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) => {
                if started.elapsed() >= timeout {
                    // std's kill() terminates only the direct child on
                    // Windows; grandchildren (e.g. rustc under a killed
                    // cargo) may linger. Acceptable: the harness still
                    // records the failure and proceeds.
                    let _ = child.kill();
                    let _ = child.wait();
                    failure_note = Some(format!(
                        "[replay_qa] timed out after {}s and was killed",
                        timeout.as_secs()
                    ));
                    break None;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                failure_note = Some(format!(
                    "[replay_qa] wait on {} failed: {err}",
                    program.display()
                ));
                break None;
            }
        }
    };
    let stdout = join_pipe_drain(stdout_drain);
    let mut stderr = join_pipe_drain(stderr_drain);
    if let Some(note) = failure_note {
        if !stderr.is_empty() && !stderr.ends_with('\n') {
            stderr.push('\n');
        }
        stderr.push_str(&note);
    }
    ProcessResult {
        exit_code,
        stdout,
        stderr,
        duration_ms: started.elapsed().as_millis(),
    }
}

fn spawn_pipe_drain<R: std::io::Read + Send + 'static>(
    pipe: Option<R>,
) -> Option<std::thread::JoinHandle<Vec<u8>>> {
    pipe.map(|mut pipe| {
        std::thread::spawn(move || {
            let mut buffer = Vec::new();
            let _ = pipe.read_to_end(&mut buffer);
            buffer
        })
    })
}

fn join_pipe_drain(handle: Option<std::thread::JoinHandle<Vec<u8>>>) -> String {
    let bytes = handle
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn write_command_logs(
    config: &ReplayQaConfig,
    stem: &str,
    program: &Path,
    args: &[String],
    output: &ProcessResult,
) -> Result<Vec<PathBuf>, String> {
    let logs_dir = config.out_dir.join("logs");
    fs::create_dir_all(&logs_dir).map_err(|err| format!("create logs dir: {err}"))?;
    let stem = safe_stem(stem);
    let command_path = logs_dir.join(format!("{stem}.command.txt"));
    let stdout_path = logs_dir.join(format!("{stem}.stdout.txt"));
    let stderr_path = logs_dir.join(format!("{stem}.stderr.txt"));
    let mut command_text = String::new();
    writeln!(command_text, "program: {}", program.display()).unwrap();
    writeln!(command_text, "args: {}", args.join(" ")).unwrap();
    writeln!(
        command_text,
        "exit_code: {}",
        exit_code_string(output.exit_code)
    )
    .unwrap();
    writeln!(command_text, "duration_ms: {}", output.duration_ms).unwrap();
    fs::write(&command_path, command_text)
        .map_err(|err| format!("write {}: {err}", command_path.display()))?;
    fs::write(&stdout_path, &output.stdout)
        .map_err(|err| format!("write {}: {err}", stdout_path.display()))?;
    fs::write(&stderr_path, &output.stderr)
        .map_err(|err| format!("write {}: {err}", stderr_path.display()))?;
    Ok(vec![command_path, stdout_path, stderr_path])
}

fn target_debug_binary(native_root: &Path, binary_name: &str) -> PathBuf {
    let mut path = native_root.join("target").join("debug").join(binary_name);
    if cfg!(windows) {
        path.set_extension("exe");
    }
    path
}

fn push_stats_numbers(numbers: &mut BTreeMap<String, String>, stats: &PixelStats) {
    numbers.insert("width".to_string(), stats.width.to_string());
    numbers.insert("height".to_string(), stats.height.to_string());
    numbers.insert("mean_luma".to_string(), format!("{:.2}", stats.mean_luma));
    numbers.insert(
        "pure_black_percent".to_string(),
        format!("{:.2}", stats.pure_black_percent),
    );
    numbers.insert(
        "distinct_colors".to_string(),
        stats.distinct_color_count.to_string(),
    );
    numbers.insert(
        "luma_stddev".to_string(),
        format!("{:.3}", stats.luma_stddev),
    );
}

fn discover_git_revision(repo_root: &Path) -> Option<String> {
    std::env::var("GIT_COMMIT")
        .ok()
        .or_else(|| std::env::var("SOURCE_VERSION").ok())
        .or_else(|| read_git_head(repo_root).ok())
}

fn discover_game_root() -> Option<PathBuf> {
    for key in ["BATTLE_PLAN_GAME_DIR", "BATTLE_PLAN_VANILLA"] {
        if let Some(path) = std::env::var_os(key).map(PathBuf::from) {
            if path.is_dir() {
                return Some(path);
            }
        }
    }
    let default = PathBuf::from(r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV");
    default.is_dir().then_some(default)
}

fn read_git_head(repo_root: &Path) -> Result<String, String> {
    let git_path = repo_root.join(".git");
    let git_dir = if git_path.is_dir() {
        git_path
    } else {
        let text = fs::read_to_string(&git_path).map_err(|err| err.to_string())?;
        let path = text
            .trim()
            .strip_prefix("gitdir:")
            .ok_or_else(|| ".git file has no gitdir".to_string())?
            .trim();
        let path = PathBuf::from(path);
        if path.is_absolute() {
            path
        } else {
            repo_root.join(path)
        }
    };
    let head = fs::read_to_string(git_dir.join("HEAD")).map_err(|err| err.to_string())?;
    let head = head.trim();
    if let Some(reference) = head.strip_prefix("ref: ") {
        fs::read_to_string(git_dir.join(reference))
            .map(|text| text.trim().to_string())
            .or_else(|_| read_packed_ref(&git_dir, reference))
    } else {
        Ok(head.to_string())
    }
}

fn read_packed_ref(git_dir: &Path, reference: &str) -> Result<String, String> {
    let packed = fs::read_to_string(git_dir.join("packed-refs")).map_err(|err| err.to_string())?;
    for line in packed.lines() {
        if line.starts_with('#') || line.starts_with('^') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(sha) = parts.next() else {
            continue;
        };
        let Some(name) = parts.next() else {
            continue;
        };
        if name == reference {
            return Ok(sha.to_string());
        }
    }
    Err(format!("ref {reference} not found in packed-refs"))
}

fn parse_kv_tokens(line: &str) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    for token in line.split_whitespace() {
        if let Some((key, value)) = token.split_once('=') {
            let key = key.trim_end_matches(':').to_string();
            values.insert(key, value.trim_end_matches(',').to_string());
        }
    }
    values
}

fn copy_keys(
    output: &mut BTreeMap<String, String>,
    prefix: &str,
    values: &BTreeMap<String, String>,
    keys: &[&str],
) {
    for key in keys {
        if let Some(value) = values.get(*key) {
            output.insert(format!("{prefix}_{key}"), value.clone());
        }
    }
}

fn require_zero(
    failures: &mut Vec<String>,
    label: &str,
    values: &BTreeMap<String, String>,
    key: &str,
) {
    match values
        .get(key)
        .and_then(|value| value.parse::<usize>().ok())
    {
        Some(0) => {}
        Some(value) => failures.push(format!("{label} is {value}, expected 0")),
        None => failures.push(format!("{label} is missing")),
    }
}

fn require_positive(
    failures: &mut Vec<String>,
    label: &str,
    values: &BTreeMap<String, String>,
    key: &str,
) {
    match values
        .get(key)
        .and_then(|value| value.parse::<usize>().ok())
    {
        Some(value) if value > 0 => {}
        Some(_) => failures.push(format!("{label} is 0")),
        None => failures.push(format!("{label} is missing")),
    }
}

fn count_png_dump_lines(text: &str) -> usize {
    let mut in_pngs = false;
    let mut count = 0usize;
    for line in text.lines() {
        if line == "png dumps:" {
            in_pngs = true;
            continue;
        }
        if in_pngs {
            if line.starts_with("  ") {
                if !line.trim().is_empty() && line.trim() != "<none>" {
                    count += 1;
                }
            } else {
                break;
            }
        }
    }
    count
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    bytes
        .get(offset..offset + 2)
        .and_then(|chunk| chunk.try_into().ok())
        .map(u16::from_le_bytes)
        .ok_or_else(|| format!("missing u16 at offset {offset}"))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    bytes
        .get(offset..offset + 4)
        .and_then(|chunk| chunk.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| format!("missing u32 at offset {offset}"))
}

fn read_i32(bytes: &[u8], offset: usize) -> Result<i32, String> {
    bytes
        .get(offset..offset + 4)
        .and_then(|chunk| chunk.try_into().ok())
        .map(i32::from_le_bytes)
        .ok_or_else(|| format!("missing i32 at offset {offset}"))
}

#[derive(Clone, Debug)]
pub struct FrameParityDiff {
    pub width: u32,
    pub height: u32,
    pub dxgi_format: u32,
    pub exact_match: bool,
    pub differing_pixels: usize,
    pub differing_percent: f64,
    pub max_channel_delta: f64,
    pub captured_file: String,
    pub heatmap_rgba: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CapturedRtMeta {
    bundle_json: PathBuf,
    width: u32,
    height: u32,
    dxgi_format: u32,
    payload_file: String,
    payload_byte_len: usize,
    payload_fnv1a64: String,
    file_offset: usize,
    subresource_byte_len: usize,
    row_pitch: usize,
    depth_pitch: usize,
    subresource_width: u32,
    subresource_height: u32,
}

#[derive(Clone, Debug)]
struct FrameData {
    width: u32,
    height: u32,
    dxgi_format: u32,
    bytes_per_pixel: usize,
    canonical_bytes: Vec<u8>,
}

pub fn compute_frame_parity_diff(
    captured_bundle: &Path,
    pass: &str,
    replay_raw: &Path,
) -> Result<FrameParityDiff, String> {
    let (captured, meta) = load_captured_rt_frame(captured_bundle, pass)?;
    let replay_bytes =
        fs::read(replay_raw).map_err(|err| format!("read {}: {err}", replay_raw.display()))?;
    let row_pitch = render_target_row_bytes(captured.dxgi_format, captured.width)?;
    let replay = frame_from_tight_bytes(
        captured.width,
        captured.height,
        captured.dxgi_format,
        row_pitch,
        &replay_bytes,
        &path_string(replay_raw),
    )?;
    let diff = compare_frame_data(&captured, &replay)?;
    Ok(FrameParityDiff {
        width: diff.width,
        height: diff.height,
        dxgi_format: diff.dxgi_format,
        exact_match: diff.exact_match,
        differing_pixels: diff.differing_pixels,
        differing_percent: diff.differing_percent,
        max_channel_delta: diff.max_channel_delta,
        captured_file: meta.payload_file,
        heatmap_rgba: diff.heatmap_rgba,
    })
}

fn load_captured_rt_frame(
    bundle: &Path,
    pass: &str,
) -> Result<(FrameData, CapturedRtMeta), String> {
    let meta = load_captured_rt_meta(bundle, pass)?;
    let payload_path = meta
        .bundle_json
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", meta.bundle_json.display()))?
        .join(&meta.payload_file);
    let raw = fs::read(&payload_path)
        .map_err(|err| format!("read captured RT {}: {err}", payload_path.display()))?;
    if raw.len() != meta.payload_byte_len {
        return Err(format!(
            "captured RT {} has {} bytes, bundle metadata expects {}",
            payload_path.display(),
            raw.len(),
            meta.payload_byte_len
        ));
    }
    let expected_hash = meta.payload_fnv1a64.to_ascii_lowercase();
    let actual_hash = fnv1a64_bytes(&raw);
    if actual_hash != expected_hash {
        return Err(format!(
            "captured RT {} hash mismatch: expected {}, actual {}",
            payload_path.display(),
            expected_hash,
            actual_hash
        ));
    }
    let frame = frame_from_pitched_payload(&meta, &raw)?;
    Ok((frame, meta))
}

fn load_captured_rt_meta(bundle: &Path, pass: &str) -> Result<CapturedRtMeta, String> {
    let bundle_json = resolve_map_frame_bundle_path(bundle, pass)?;
    let text = fs::read_to_string(&bundle_json)
        .map_err(|err| format!("read {}: {err}", bundle_json.display()))?;
    let post_pass = json_object_after(&text, "\"post_pass_output_merger\"")?;
    let targets = json_array_after(post_pass, "\"render_targets\"")?;
    let target = json_first_object(targets)?;
    let desc = json_object_after(target, "\"desc\"")?;
    let payload = json_object_after(target, "\"post_pass_payload\"")?;
    let subresources = json_array_after(payload, "\"subresources\"")?;
    let subresource = json_first_object(subresources)?;
    let width = json_u32_value(desc, "width")?;
    let height = json_u32_value(desc, "height")?;
    let dxgi_format = json_u32_value(desc, "format")?;
    let payload_file = json_string_value(payload, "file")?;
    let payload_byte_len = json_usize_value(payload, "byte_len")?;
    let payload_fnv1a64 = json_string_value(payload, "fnv1a64")?;
    let payload_format = json_string_value(payload, "payload_format")?;
    if payload_format != "d3d11_mapped_subresources" {
        return Err(format!(
            "{} post-pass payload_format is {}, expected d3d11_mapped_subresources",
            bundle_json.display(),
            payload_format
        ));
    }
    Ok(CapturedRtMeta {
        bundle_json,
        width,
        height,
        dxgi_format,
        payload_file,
        payload_byte_len,
        payload_fnv1a64,
        file_offset: json_usize_value(subresource, "file_offset")?,
        subresource_byte_len: json_usize_value(subresource, "byte_len")?,
        row_pitch: json_usize_value(subresource, "row_pitch")?,
        depth_pitch: json_usize_value(subresource, "depth_pitch")?,
        subresource_width: json_u32_value(subresource, "width")?,
        subresource_height: json_u32_value(subresource, "height")?,
    })
}

fn resolve_map_frame_bundle_path(path: &Path, pass: &str) -> Result<PathBuf, String> {
    if !path.is_dir() {
        // A direct bundle file is authoritative regardless of the pass name:
        // config-created gates default `pass` to the gate name, and the file
        // choice is already explicit.
        return Ok(path.to_path_buf());
    }
    let file = match pass {
        "terrain" | "terrain_active_pass" => "map_frame_bundle.json",
        "water" | "water_pass" => "map_frame_water_pass_bundle.json",
        "rivers" | "rivers_pass" => "map_frame_rivers_pass_bundle.json",
        "trees" | "trees_pass" => "map_frame_trees_pass_bundle.json",
        // Fail closed: a typo'd or not-yet-mapped pass must never silently
        // compare against the terrain bundle.
        _ => {
            return Err(format!(
                "unknown frame parity pass {pass:?} for bundle directory {}; known: \
                 terrain|terrain_active_pass|water|water_pass|rivers|rivers_pass|trees|trees_pass",
                path.display()
            ))
        }
    };
    Ok(path.join(file))
}

fn frame_from_pitched_payload(meta: &CapturedRtMeta, raw: &[u8]) -> Result<FrameData, String> {
    if meta.subresource_width != meta.width || meta.subresource_height != meta.height {
        return Err(format!(
            "{} post-pass subresource is {}x{}, descriptor is {}x{}",
            meta.bundle_json.display(),
            meta.subresource_width,
            meta.subresource_height,
            meta.width,
            meta.height
        ));
    }
    let row_bytes = render_target_row_bytes(meta.dxgi_format, meta.width)?;
    if meta.row_pitch < row_bytes {
        return Err(format!(
            "{} post-pass row_pitch {} is below row byte width {}",
            meta.bundle_json.display(),
            meta.row_pitch,
            row_bytes
        ));
    }
    if meta.depth_pitch < meta.row_pitch * meta.height as usize {
        return Err(format!(
            "{} post-pass depth_pitch {} is below row_pitch {} * height {}",
            meta.bundle_json.display(),
            meta.depth_pitch,
            meta.row_pitch,
            meta.height
        ));
    }
    if meta.subresource_byte_len < meta.depth_pitch {
        return Err(format!(
            "{} post-pass subresource byte_len {} is below depth_pitch {}",
            meta.bundle_json.display(),
            meta.subresource_byte_len,
            meta.depth_pitch
        ));
    }
    let end = meta
        .file_offset
        .checked_add(meta.depth_pitch)
        .ok_or_else(|| "captured RT byte range overflow".to_string())?;
    if end > raw.len() {
        return Err(format!(
            "{} post-pass payload range {}..{} exceeds file byte_len {}",
            meta.bundle_json.display(),
            meta.file_offset,
            end,
            raw.len()
        ));
    }
    let mut tight = Vec::with_capacity(row_bytes * meta.height as usize);
    for row in 0..meta.height as usize {
        let row_start = meta.file_offset + row * meta.row_pitch;
        let row_end = row_start + row_bytes;
        tight.extend_from_slice(&raw[row_start..row_end]);
    }
    frame_from_tight_bytes(
        meta.width,
        meta.height,
        meta.dxgi_format,
        row_bytes,
        &tight,
        &meta.payload_file,
    )
}

fn frame_from_tight_bytes(
    width: u32,
    height: u32,
    dxgi_format: u32,
    row_pitch: usize,
    bytes: &[u8],
    label: &str,
) -> Result<FrameData, String> {
    let row_bytes = render_target_row_bytes(dxgi_format, width)?;
    if row_pitch != row_bytes {
        return Err(format!(
            "{label} row_pitch {row_pitch} is not tight row byte width {row_bytes}"
        ));
    }
    let expected = row_bytes
        .checked_mul(height as usize)
        .ok_or_else(|| format!("{label} byte_len overflow"))?;
    if bytes.len() != expected {
        return Err(format!(
            "{label} has {} bytes, expected {} for {}x{} format {}",
            bytes.len(),
            expected,
            width,
            height,
            dxgi_format
        ));
    }
    let bytes_per_pixel = bytes_per_pixel(dxgi_format)?;
    let canonical_bytes = canonicalize_render_target_bytes(dxgi_format, bytes)?;
    Ok(FrameData {
        width,
        height,
        dxgi_format,
        bytes_per_pixel,
        canonical_bytes,
    })
}

fn canonicalize_render_target_bytes(dxgi_format: u32, bytes: &[u8]) -> Result<Vec<u8>, String> {
    match dxgi_format {
        DXGI_FORMAT_R16G16B16A16_FLOAT | DXGI_FORMAT_R8G8B8A8_UNORM => Ok(bytes.to_vec()),
        DXGI_FORMAT_B8G8R8A8_UNORM | DXGI_FORMAT_B8G8R8A8_UNORM_SRGB => {
            let mut out = Vec::with_capacity(bytes.len());
            for pixel in bytes.chunks_exact(4) {
                out.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
            }
            Ok(out)
        }
        other => Err(format!(
            "render target format {other} has no replay_qa frame-parity converter"
        )),
    }
}

#[derive(Clone, Debug)]
struct FrameCompare {
    width: u32,
    height: u32,
    dxgi_format: u32,
    exact_match: bool,
    differing_pixels: usize,
    differing_percent: f64,
    max_channel_delta: f64,
    heatmap_rgba: Vec<u8>,
}

fn compare_frame_data(expected: &FrameData, actual: &FrameData) -> Result<FrameCompare, String> {
    if expected.width != actual.width
        || expected.height != actual.height
        || expected.dxgi_format != actual.dxgi_format
    {
        return Err(format!(
            "frame dimensions/format differ: captured {}x{} format {}, replay {}x{} format {}",
            expected.width,
            expected.height,
            expected.dxgi_format,
            actual.width,
            actual.height,
            actual.dxgi_format
        ));
    }
    let pixel_count = expected.width as usize * expected.height as usize;
    let bpp = expected.bytes_per_pixel;
    let mut differing_pixels = 0usize;
    let mut max_channel_delta = 0.0f64;
    for index in 0..pixel_count {
        let start = index * bpp;
        let end = start + bpp;
        let expected_pixel = &expected.canonical_bytes[start..end];
        let actual_pixel = &actual.canonical_bytes[start..end];
        let delta = pixel_channel_delta(expected.dxgi_format, expected_pixel, actual_pixel)?;
        if expected_pixel != actual_pixel {
            differing_pixels += 1;
        }
        if delta.is_infinite() || delta > max_channel_delta {
            max_channel_delta = delta;
        }
    }
    // Second pass renders the heatmap directly against the now-known maximum
    // so no per-pixel f64 buffer is retained (~64 MB transient for a 4K RT
    // otherwise). pixel_channel_delta cannot newly fail here: the format was
    // already accepted in the first pass.
    let mut heatmap_rgba = Vec::with_capacity(pixel_count * 4);
    for index in 0..pixel_count {
        let start = index * bpp;
        let end = start + bpp;
        let delta = pixel_channel_delta(
            expected.dxgi_format,
            &expected.canonical_bytes[start..end],
            &actual.canonical_bytes[start..end],
        )?;
        let intensity = if delta == 0.0 {
            0
        } else if delta.is_infinite() || max_channel_delta == 0.0 || max_channel_delta.is_infinite()
        {
            255
        } else {
            ((delta / max_channel_delta) * 255.0)
                .ceil()
                .clamp(1.0, 255.0) as u8
        };
        heatmap_rgba.extend_from_slice(&[intensity, intensity / 4, 0, 255]);
    }
    let differing_percent = if pixel_count == 0 {
        0.0
    } else {
        differing_pixels as f64 * 100.0 / pixel_count as f64
    };
    Ok(FrameCompare {
        width: expected.width,
        height: expected.height,
        dxgi_format: expected.dxgi_format,
        exact_match: differing_pixels == 0,
        differing_pixels,
        differing_percent,
        max_channel_delta,
        heatmap_rgba,
    })
}

fn pixel_channel_delta(format: u32, expected: &[u8], actual: &[u8]) -> Result<f64, String> {
    match format {
        DXGI_FORMAT_R16G16B16A16_FLOAT => {
            let mut max_delta = 0.0f64;
            for channel in 0..4 {
                let offset = channel * 2;
                let a = u16::from_le_bytes([expected[offset], expected[offset + 1]]);
                let b = u16::from_le_bytes([actual[offset], actual[offset + 1]]);
                let delta = if a == b {
                    0.0
                } else {
                    let a = half_to_f32(a);
                    let b = half_to_f32(b);
                    if a.is_finite() && b.is_finite() {
                        f64::from((a - b).abs())
                    } else {
                        f64::INFINITY
                    }
                };
                if delta.is_infinite() || delta > max_delta {
                    max_delta = delta;
                }
            }
            Ok(max_delta)
        }
        DXGI_FORMAT_R8G8B8A8_UNORM
        | DXGI_FORMAT_B8G8R8A8_UNORM
        | DXGI_FORMAT_B8G8R8A8_UNORM_SRGB => Ok(expected
            .iter()
            .zip(actual.iter())
            .map(|(a, b)| a.abs_diff(*b) as f64)
            .fold(0.0, f64::max)),
        other => Err(format!("format {other} has no delta calculator")),
    }
}

fn half_to_f32(bits: u16) -> f32 {
    let sign = ((bits & 0x8000) as u32) << 16;
    let exponent = (bits >> 10) & 0x1f;
    let fraction = bits & 0x03ff;
    let out = match exponent {
        0 => {
            if fraction == 0 {
                sign
            } else {
                let mut fraction_u32 = fraction as u32;
                let mut exponent_i32 = -14;
                while (fraction_u32 & 0x0400) == 0 {
                    fraction_u32 <<= 1;
                    exponent_i32 -= 1;
                }
                fraction_u32 &= 0x03ff;
                sign | (((exponent_i32 + 127) as u32) << 23) | (fraction_u32 << 13)
            }
        }
        0x1f => sign | 0x7f80_0000 | ((fraction as u32) << 13),
        _ => sign | (((exponent as i32 - 15 + 127) as u32) << 23) | ((fraction as u32) << 13),
    };
    f32::from_bits(out)
}

fn bytes_per_pixel(format: u32) -> Result<usize, String> {
    match format {
        DXGI_FORMAT_R16G16B16A16_FLOAT => Ok(8),
        DXGI_FORMAT_R8G8B8A8_UNORM
        | DXGI_FORMAT_B8G8R8A8_UNORM
        | DXGI_FORMAT_B8G8R8A8_UNORM_SRGB => Ok(4),
        other => Err(format!("render target format {other} has no byte width")),
    }
}

fn render_target_row_bytes(format: u32, width: u32) -> Result<usize, String> {
    bytes_per_pixel(format)?
        .checked_mul(width as usize)
        .ok_or_else(|| "render target row byte width overflow".to_string())
}

fn fnv1a64_bytes(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn format_delta(value: f64) -> String {
    if value.is_infinite() {
        "inf".to_string()
    } else if value == 0.0 {
        "0".to_string()
    } else {
        format!("{value:.9}")
    }
}

fn write_png_rgba(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    let row_bytes = width as usize * 4;
    let expected = row_bytes
        .checked_mul(height as usize)
        .ok_or_else(|| "PNG byte_len overflow".to_string())?;
    if rgba.len() != expected {
        return Err(format!(
            "PNG RGBA input has {} bytes, expected {} for {}x{}",
            rgba.len(),
            expected,
            width,
            height
        ));
    }
    let mut filtered = Vec::with_capacity(expected + height as usize);
    for row in 0..height as usize {
        filtered.push(0);
        filtered.extend_from_slice(&rgba[row * row_bytes..(row + 1) * row_bytes]);
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    append_png_chunk(&mut out, b"IHDR", &ihdr);
    let zlib = zlib_store(&filtered);
    append_png_chunk(&mut out, b"IDAT", &zlib);
    append_png_chunk(&mut out, b"IEND", &[]);
    fs::write(path, out).map_err(|err| format!("write {}: {err}", path.display()))
}

fn append_png_chunk(out: &mut Vec<u8>, name: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(name);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(name.len() + data.len());
    crc_input.extend_from_slice(name);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

fn zlib_store(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut offset = 0usize;
    while offset < data.len() {
        let remaining = data.len() - offset;
        let block_len = remaining.min(65_535);
        let final_block = offset + block_len == data.len();
        out.push(if final_block { 1 } else { 0 });
        let len = block_len as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(&data[offset..offset + block_len]);
        offset += block_len;
    }
    if data.is_empty() {
        out.push(1);
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(!0u16).to_le_bytes());
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    const MOD: u32 = 65_521;
    let mut a = 1u32;
    let mut b = 0u32;
    for byte in bytes {
        a = (a + u32::from(*byte)) % MOD;
        b = (b + a) % MOD;
    }
    (b << 16) | a
}

fn json_object_after<'a>(text: &'a str, needle: &str) -> Result<&'a str, String> {
    json_value_after(text, needle, b'{')
}

fn json_array_after<'a>(text: &'a str, needle: &str) -> Result<&'a str, String> {
    json_value_after(text, needle, b'[')
}

fn json_value_after<'a>(text: &'a str, needle: &str, expected: u8) -> Result<&'a str, String> {
    let start = text
        .find(needle)
        .ok_or_else(|| format!("JSON key {needle} not found"))?;
    let after_key = start + needle.len();
    let colon = text[after_key..]
        .find(':')
        .map(|offset| after_key + offset)
        .ok_or_else(|| format!("JSON key {needle} has no ':'"))?;
    let value_start = skip_json_ws(text.as_bytes(), colon + 1);
    if text.as_bytes().get(value_start).copied() != Some(expected) {
        return Err(format!(
            "JSON key {needle} does not start with {}",
            expected as char
        ));
    }
    let value_end = balanced_json_end(text.as_bytes(), value_start)?;
    Ok(&text[value_start..value_end])
}

fn json_first_object(array: &str) -> Result<&str, String> {
    let bytes = array.as_bytes();
    let index = skip_json_ws(bytes, 1);
    if bytes.get(index).copied() == Some(b']') {
        return Err("JSON array is empty".to_string());
    }
    if bytes.get(index).copied() != Some(b'{') {
        return Err("JSON array first item is not an object".to_string());
    }
    let end = balanced_json_end(bytes, index)?;
    Ok(&array[index..end])
}

fn json_string_value(text: &str, key: &str) -> Result<String, String> {
    let token = format!("\"{key}\"");
    let start = text
        .find(&token)
        .ok_or_else(|| format!("JSON string key {key:?} not found"))?;
    let after_key = start + token.len();
    let colon = text[after_key..]
        .find(':')
        .map(|offset| after_key + offset)
        .ok_or_else(|| format!("JSON string key {key:?} has no ':'"))?;
    let mut index = skip_json_ws(text.as_bytes(), colon + 1);
    if text.as_bytes().get(index).copied() != Some(b'"') {
        return Err(format!("JSON key {key:?} is not a string"));
    }
    index += 1;
    // Accumulate raw bytes, not chars: pushing each byte as a char would
    // mojibake-decode multi-byte UTF-8 sequences (e.g. a payload filename
    // under a non-ASCII directory) byte-per-byte as Latin-1. b'"' and b'\\'
    // never occur inside a multi-byte sequence, so the scan stays correct.
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                return String::from_utf8(out)
                    .map_err(|_| format!("JSON key {key:?} string is not valid UTF-8"))
            }
            b'\\' => {
                index += 1;
                let escaped = bytes
                    .get(index)
                    .copied()
                    .ok_or_else(|| format!("JSON key {key:?} has trailing escape"))?;
                let byte = match escaped {
                    b'"' => b'"',
                    b'\\' => b'\\',
                    b'/' => b'/',
                    b'b' => 0x08,
                    b'f' => 0x0c,
                    b'n' => b'\n',
                    b'r' => b'\r',
                    b't' => b'\t',
                    other => {
                        return Err(format!(
                            "JSON key {key:?} has unsupported escape \\{}",
                            other as char
                        ))
                    }
                };
                out.push(byte);
            }
            byte => out.push(byte),
        }
        index += 1;
    }
    Err(format!("JSON key {key:?} string is unterminated"))
}

fn json_u32_value(text: &str, key: &str) -> Result<u32, String> {
    json_u64_value(text, key).and_then(|value| {
        u32::try_from(value).map_err(|_| format!("JSON key {key:?} value {value} exceeds u32"))
    })
}

fn json_usize_value(text: &str, key: &str) -> Result<usize, String> {
    json_u64_value(text, key).and_then(|value| {
        usize::try_from(value).map_err(|_| format!("JSON key {key:?} value {value} exceeds usize"))
    })
}

fn json_u64_value(text: &str, key: &str) -> Result<u64, String> {
    let token = format!("\"{key}\"");
    let start = text
        .find(&token)
        .ok_or_else(|| format!("JSON numeric key {key:?} not found"))?;
    let after_key = start + token.len();
    let colon = text[after_key..]
        .find(':')
        .map(|offset| after_key + offset)
        .ok_or_else(|| format!("JSON numeric key {key:?} has no ':'"))?;
    let index = skip_json_ws(text.as_bytes(), colon + 1);
    let end = text.as_bytes()[index..]
        .iter()
        .position(|byte| !byte.is_ascii_digit())
        .map(|offset| index + offset)
        .unwrap_or(text.len());
    if end == index {
        return Err(format!("JSON key {key:?} is not an unsigned integer"));
    }
    text[index..end]
        .parse::<u64>()
        .map_err(|err| format!("JSON key {key:?}: {err}"))
}

fn skip_json_ws(bytes: &[u8], mut index: usize) -> usize {
    while matches!(bytes.get(index), Some(b' ' | b'\n' | b'\r' | b'\t')) {
        index += 1;
    }
    index
}

fn balanced_json_end(bytes: &[u8], start: usize) -> Result<usize, String> {
    let open = bytes
        .get(start)
        .copied()
        .ok_or_else(|| "JSON value start is out of range".to_string())?;
    match open {
        b'{' | b'[' => {}
        _ => return Err("balanced_json_end requires object or array".to_string()),
    }
    let mut object_depth = 0usize;
    let mut array_depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, byte) in bytes[start..].iter().copied().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        if byte == b'"' {
            in_string = true;
        } else if byte == b'{' {
            object_depth += 1;
        } else if byte == b'[' {
            array_depth += 1;
        } else if byte == b'}' {
            object_depth = object_depth
                .checked_sub(1)
                .ok_or_else(|| "JSON object close delimiter underflow".to_string())?;
            if object_depth == 0 && array_depth == 0 {
                return Ok(start + offset + 1);
            }
        } else if byte == b']' {
            array_depth = array_depth
                .checked_sub(1)
                .ok_or_else(|| "JSON array close delimiter underflow".to_string())?;
            if object_depth == 0 && array_depth == 0 {
                return Ok(start + offset + 1);
            }
        }
    }
    Err("JSON object/array is unterminated".to_string())
}

fn parse_bool(value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(format!("invalid boolean {value:?}")),
    }
}

fn optional_path_value(value: &str) -> Option<PathBuf> {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("none") || value.eq_ignore_ascii_case("null")
    {
        None
    } else {
        Some(PathBuf::from(value))
    }
}

fn parse_f64(key: &str, value: &str) -> Result<f64, String> {
    let parsed = value
        .parse::<f64>()
        .map_err(|err| format!("{key}: {err}"))?;
    if parsed.is_finite() {
        Ok(parsed)
    } else {
        Err(format!("{key}: value must be finite"))
    }
}

fn parse_usize(key: &str, value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|err| format!("{key}: {err}"))
}

fn parse_f64_list(value: &str) -> Result<Vec<f64>, String> {
    let mut values = Vec::new();
    for item in value.split(',') {
        values.push(parse_f64("matrix.hours", item.trim())?);
    }
    if values.is_empty() {
        return Err("matrix.hours must contain at least one value".to_string());
    }
    Ok(values)
}

fn parse_pair(key: &str, value: &str) -> Result<(f64, f64), String> {
    let (x, y) = value
        .split_once(',')
        .ok_or_else(|| format!("{key}: expected x,y"))?;
    Ok((parse_f64(key, x.trim())?, parse_f64(key, y.trim())?))
}

/// Extract the value part of a `key = value` config line. A quoted value
/// keeps every character between the quotes (including '#', a legal Windows
/// filename character) and tolerates a trailing `# comment`; anything else
/// after the closing quote is a typed parse error, as is a missing closing
/// quote. An unquoted value is truncated only at the first '#' that is
/// preceded by whitespace, so paths like `C:\caps\run#2` survive intact.
fn config_value(raw_value: &str, line_number: usize) -> Result<&str, String> {
    let trimmed = raw_value.trim();
    if let Some(quote @ ('"' | '\'')) = trimmed.chars().next() {
        let closing = trimmed[1..].find(quote).ok_or_else(|| {
            format!("config line {line_number} has an unterminated {quote} quote")
        })?;
        let value = &trimmed[1..1 + closing];
        let remainder = trimmed[1 + closing + 1..].trim_start();
        if !remainder.is_empty() && !remainder.starts_with('#') {
            return Err(format!(
                "config line {line_number} has trailing text {remainder:?} after the closing quote"
            ));
        }
        return Ok(value);
    }
    let uncommented = match inline_comment_start(raw_value) {
        Some(index) => &raw_value[..index],
        None => raw_value,
    };
    Ok(uncommented.trim())
}

/// First '#' preceded by whitespace — the only place an unquoted config value
/// may carry an inline comment.
fn inline_comment_start(value: &str) -> Option<usize> {
    let bytes = value.as_bytes();
    (1..bytes.len()).find(|&index| bytes[index] == b'#' && matches!(bytes[index - 1], b' ' | b'\t'))
}

fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => write!(out, "\\u{:04x}", ch as u32).unwrap(),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn md_escape(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}

fn path_string(path: &Path) -> String {
    path.display().to_string()
}

fn safe_stem(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn first_nonempty_line(text: &str) -> Option<&str> {
    text.lines().find(|line| !line.trim().is_empty())
}

/// Failure attribution reads the LAST non-empty line: replay_app (and cargo)
/// print informational/warning lines first and the fatal error last, so the
/// first line is typically an unrelated info line. Rust's
/// "note: run with RUST_BACKTRACE=1" panic trailer is skipped so a child
/// panic still attributes to the panic message.
fn last_nonempty_line(text: &str) -> Option<&str> {
    text.lines().rev().find(|line| {
        let trimmed = line.trim();
        !trimmed.is_empty() && !trimmed.starts_with("note:")
    })
}

fn exit_code_string(exit_code: Option<i32>) -> String {
    exit_code
        .map(|code| code.to_string())
        .unwrap_or_else(|| "spawn_failed".to_string())
}

fn hour_label(hour: f64) -> String {
    trim_float(hour).replace('.', "_").replace('-', "neg")
}

fn trim_float(value: f64) -> String {
    let mut text = format!("{value:.6}");
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.push('0');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_bmp_stats_for_32_bit_pixels() {
        let bmp = bmp32(
            2,
            2,
            &[(0, 0, 0), (255, 255, 255), (255, 0, 0), (0, 0, 255)],
        );
        let stats = compute_bmp_stats(&bmp).expect("stats");
        assert_eq!(stats.width, 2);
        assert_eq!(stats.height, 2);
        assert_eq!(stats.distinct_color_count, 4);
        assert!((stats.pure_black_percent - 25.0).abs() < 0.001);
        assert!(stats.mean_luma > 80.0);
        assert!(stats.luma_stddev > 60.0);
    }

    #[test]
    fn evaluates_threshold_failures() {
        let stats = PixelStats {
            width: 2,
            height: 2,
            mean_luma: 0.0,
            luma_stddev: 0.0,
            pure_black_percent: 100.0,
            distinct_color_count: 1,
        };
        let failures = evaluate_stats(
            &stats,
            ScenarioThresholds {
                max_pure_black_percent: 20.0,
                min_distinct_colors: 8,
                min_luma_stddev: 1.0,
            },
        );
        assert_eq!(failures.len(), 3);
    }

    #[test]
    fn accepts_ui_probe_gate_lines() {
        let text = "\
game_ui probe report
gfx files=3 sprites=20 parse_failures=0 recoveries=0
gui files=2 windows=1 elements=12 parse_failures=0 recoveries=0
topbar sprite_refs=5 resolved_textures=5 decoded_textures=5 missing=0 undecodable=0
topbar composition sample: drawn=40 missing_sprites=0 undecodable_sprites=0 missing_glyph_sets=0 flag=foo.dds
topbar composition empty: drawn=20 missing_sprites=0 undecodable_sprites=0 missing_glyph_sets=0
fonts checked=hoi_18 missing_ascii_sets=0 missing_topbar_sample_sets=0
png dumps:
  a.png
  b.png
  c.png
  d.png
  e.png
  f.png
  g.png
  h.png
parser design notes:
";
        let evaluation = evaluate_ui_probe_text(text, true);
        assert!(evaluation.failures.is_empty(), "{:?}", evaluation.failures);
        assert_eq!(
            evaluation.key_numbers.get("topbar_sprite_refs"),
            Some(&"5".to_string())
        );
    }

    #[test]
    fn report_generation_outputs_json_and_markdown_rows() {
        let mut key_numbers = BTreeMap::new();
        key_numbers.insert("mean_luma".to_string(), "42.00".to_string());
        let report = ReplayQaReport {
            schema_version: 1,
            git_revision: Some("abcdef".to_string()),
            out_dir: PathBuf::from("out"),
            tools: vec![ToolInfo {
                name: "map_viewer".to_string(),
                package: "map_viewer".to_string(),
                path: PathBuf::from("target/debug/map_viewer.exe"),
                built_by_harness: false,
            }],
            gates: vec![GateRecord {
                gate: "render | matrix".to_string(),
                status: GateStatus::Pass,
                hard: true,
                key_numbers,
                artifacts: vec![PathBuf::from("out/frame.bmp")],
                details: vec!["ok".to_string()],
            }],
        };
        let json = report.to_json();
        assert!(json.contains("\"schema_version\": 1"));
        assert!(json.contains("\"render | matrix\""));
        let md = report.to_markdown();
        assert!(md.contains("render \\| matrix"));
        assert!(md.contains("mean_luma=42.00"));
    }

    #[test]
    fn pitched_bgra_payload_converts_to_tight_rgba() {
        let meta = CapturedRtMeta {
            bundle_json: PathBuf::from("bundle.json"),
            width: 2,
            height: 2,
            dxgi_format: DXGI_FORMAT_B8G8R8A8_UNORM,
            payload_file: "frame.d3d11".to_string(),
            payload_byte_len: 24,
            payload_fnv1a64: String::new(),
            file_offset: 0,
            subresource_byte_len: 24,
            row_pitch: 12,
            depth_pitch: 24,
            subresource_width: 2,
            subresource_height: 2,
        };
        let raw = vec![
            3, 2, 1, 4, 30, 20, 10, 40, 0xaa, 0xaa, 0xaa, 0xaa, 7, 6, 5, 8, 70, 60, 50, 80, 0xbb,
            0xbb, 0xbb, 0xbb,
        ];
        let frame = frame_from_pitched_payload(&meta, &raw).expect("convert frame");
        assert_eq!(frame.width, 2);
        assert_eq!(frame.height, 2);
        assert_eq!(
            frame.canonical_bytes,
            vec![1, 2, 3, 4, 10, 20, 30, 40, 5, 6, 7, 8, 50, 60, 70, 80]
        );
    }

    #[test]
    fn frame_diff_counts_pixels_and_max_channel_delta() {
        let expected = FrameData {
            width: 2,
            height: 1,
            dxgi_format: DXGI_FORMAT_R8G8B8A8_UNORM,
            bytes_per_pixel: 4,
            canonical_bytes: vec![0, 0, 0, 255, 10, 20, 30, 255],
        };
        let actual = FrameData {
            width: 2,
            height: 1,
            dxgi_format: DXGI_FORMAT_R8G8B8A8_UNORM,
            bytes_per_pixel: 4,
            canonical_bytes: vec![0, 0, 0, 255, 15, 18, 30, 245],
        };
        let diff = compare_frame_data(&expected, &actual).expect("diff");
        assert!(!diff.exact_match);
        assert_eq!(diff.differing_pixels, 1);
        assert!((diff.differing_percent - 50.0).abs() < 0.001);
        assert_eq!(diff.max_channel_delta, 10.0);
        assert_eq!(&diff.heatmap_rgba[0..4], &[0, 0, 0, 255]);
        assert_eq!(&diff.heatmap_rgba[4..8], &[255, 63, 0, 255]);
    }

    #[test]
    fn half_float_delta_decodes_channel_values() {
        assert_eq!(half_to_f32(0x3c00), 1.0);
        assert_eq!(half_to_f32(0x4000), 2.0);
        let expected = 0x3c00u16.to_le_bytes();
        let actual = 0x4000u16.to_le_bytes();
        let mut expected_pixel = Vec::new();
        let mut actual_pixel = Vec::new();
        for _ in 0..4 {
            expected_pixel.extend_from_slice(&expected);
            actual_pixel.extend_from_slice(&actual);
        }
        let delta = pixel_channel_delta(
            DXGI_FORMAT_R16G16B16A16_FLOAT,
            &expected_pixel,
            &actual_pixel,
        )
        .expect("delta");
        assert_eq!(delta, 1.0);
    }

    #[test]
    fn heatmap_png_writer_emits_png_signature_and_dimensions() {
        let path = std::env::temp_dir().join(format!(
            "replay_qa_heatmap_{}_{}.png",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ));
        let rgba = vec![0, 0, 0, 255, 255, 63, 0, 255];
        write_png_rgba(&path, 2, 1, &rgba).expect("write png");
        let bytes = fs::read(&path).expect("read png");
        let _ = fs::remove_file(&path);
        assert_eq!(&bytes[0..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&bytes[12..16], b"IHDR");
        assert_eq!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()), 2);
        assert_eq!(u32::from_be_bytes(bytes[20..24].try_into().unwrap()), 1);
    }

    #[test]
    fn bmp_stats_rejects_overflowing_dimensions_with_typed_error() {
        let mut bmp = bmp32(1, 1, &[(1, 2, 3)]);
        bmp[18..22].copy_from_slice(&i32::MAX.to_le_bytes());
        bmp[22..26].copy_from_slice(&i32::MAX.to_le_bytes());
        let err = compute_bmp_stats(&bmp).expect_err("overflowing dimensions must not panic");
        assert!(err.contains("overflow"), "{err}");
    }

    #[test]
    fn config_values_keep_hash_in_unquoted_paths() {
        let mut config = ReplayQaConfig::default();
        config
            .apply_config_text("capture_dir = C:\\caps\\run#2\n")
            .expect("hash path");
        assert_eq!(config.capture_dir, PathBuf::from(r"C:\caps\run#2"));
        config
            .apply_config_text("bundle_dir = C:\\bundles\\main # nightly bundle\n")
            .expect("inline comment");
        assert_eq!(config.bundle_dir, PathBuf::from(r"C:\bundles\main"));
    }

    #[test]
    fn config_quoted_values_keep_hash_and_reject_malformed_quoting() {
        let mut config = ReplayQaConfig::default();
        config
            .apply_config_text("capture_dir = \"C:\\caps\\run #2\" # commentary\n")
            .expect("quoted value with trailing comment");
        assert_eq!(config.capture_dir, PathBuf::from(r"C:\caps\run #2"));
        let err = config
            .apply_config_text("capture_dir = \"C:\\caps\\open\n")
            .expect_err("unterminated quote");
        assert!(err.contains("unterminated"), "{err}");
        let err = config
            .apply_config_text("capture_dir = \"C:\\caps\" trailing\n")
            .expect_err("trailing text after quote");
        assert!(err.contains("trailing text"), "{err}");
    }

    #[test]
    fn config_full_line_comments_and_blank_lines_are_ignored() {
        let mut config = ReplayQaConfig::default();
        config
            .apply_config_text("# includes an = sign\n\n  # another = comment\nout_dir = out\n")
            .expect("comment lines");
        assert_eq!(config.out_dir, PathBuf::from("out"));
    }

    #[test]
    fn config_errors_are_typed_for_missing_assignment_unknown_key_and_bad_bool() {
        let mut config = ReplayQaConfig::default();
        let err = config
            .apply_config_text("no assignment here\n")
            .expect_err("missing '='");
        assert!(err.contains("missing '='"), "{err}");
        let err = config
            .apply_config_text("unknown_key = 1\n")
            .expect_err("unknown key");
        assert!(err.contains("unsupported config key"), "{err}");
        let err = config
            .apply_config_text("water.expected_fail = perhaps\n")
            .expect_err("bad bool");
        assert!(err.contains("invalid boolean"), "{err}");
    }

    #[test]
    fn config_round_trips_scalar_families() {
        let mut config = ReplayQaConfig::default();
        config
            .apply_config_text(
                "water.expected_fail = true\n\
                 matrix.hours = 0.25, 0.75, 1\n\
                 threshold.full_map.max_pure_black_percent = 12.5\n\
                 threshold.close_up.min_distinct_colors = 4\n\
                 process.timeout_secs = 60\n",
            )
            .expect("scalar families");
        assert!(config.water_expected_fail);
        assert_eq!(config.matrix.hours, vec![0.25, 0.75, 1.0]);
        assert_eq!(config.thresholds.full_map.max_pure_black_percent, 12.5);
        assert_eq!(config.thresholds.close_up.min_distinct_colors, 4);
        assert_eq!(config.process_timeout, Duration::from_secs(60));
        let err = config
            .apply_config_text("threshold.full_map.min_luma_stddev = inf\n")
            .expect_err("non-finite threshold");
        assert!(err.contains("finite"), "{err}");
        let err = config
            .apply_config_text("process.timeout_secs = 0\n")
            .expect_err("zero timeout");
        assert!(err.contains("positive"), "{err}");
    }

    #[test]
    fn config_camera_overrides_and_errors() {
        let mut config = ReplayQaConfig::default();
        config
            .apply_config_text(
                "camera.europe_close.zoom = 0.5\ncamera.europe_close.center = 100, 200\n",
            )
            .expect("camera overrides");
        let camera = config
            .matrix
            .cameras
            .iter()
            .find(|camera| camera.config_name == "europe_close")
            .expect("europe_close camera");
        assert_eq!(camera.zoom, Some(0.5));
        assert_eq!(camera.center, Some((100.0, 200.0)));
        let err = config
            .apply_config_text("camera.mars.zoom = 1\n")
            .expect_err("unknown camera");
        assert!(err.contains("unknown camera"), "{err}");
        let err = config
            .apply_config_text("camera.europe_close.tilt = 1\n")
            .expect_err("unsupported field");
        assert!(err.contains("unsupported camera field"), "{err}");
    }

    #[test]
    fn config_frame_parity_auto_creates_gates_and_clear_is_order_sensitive() {
        let mut config = ReplayQaConfig::default();
        config
            .apply_config_text(
                "frame_parity.clear = true\n\
                 frame_parity.rivers.compare_map_frame_bundle = C:\\caps\\dir\n\
                 frame_parity.rivers.hard = false\n",
            )
            .expect("frame parity config");
        assert_eq!(config.frame_parity_gates.len(), 1);
        let gate = &config.frame_parity_gates[0];
        assert_eq!(gate.name, "rivers");
        assert_eq!(gate.pass, "rivers");
        assert_eq!(gate.gate, "frame parity rivers captured RT");
        assert_eq!(gate.compare_map_frame_bundle, PathBuf::from(r"C:\caps\dir"));
        assert!(!gate.hard);
        assert!(gate.enabled);
        // clear AFTER the re-adds wipes them again: application is line-ordered.
        config
            .apply_config_text("frame_parity.clear = true\n")
            .expect("clear");
        assert!(config.frame_parity_gates.is_empty());
    }

    #[test]
    fn frame_bundle_path_resolution_maps_known_passes_and_refuses_unknown() {
        let dir = std::env::temp_dir();
        for (pass, file) in [
            ("terrain", "map_frame_bundle.json"),
            ("terrain_active_pass", "map_frame_bundle.json"),
            ("water", "map_frame_water_pass_bundle.json"),
            ("water_pass", "map_frame_water_pass_bundle.json"),
            ("rivers", "map_frame_rivers_pass_bundle.json"),
            ("rivers_pass", "map_frame_rivers_pass_bundle.json"),
            ("trees", "map_frame_trees_pass_bundle.json"),
            ("trees_pass", "map_frame_trees_pass_bundle.json"),
        ] {
            let resolved = resolve_map_frame_bundle_path(&dir, pass).expect("known pass");
            assert_eq!(resolved, dir.join(file));
        }
        let err = resolve_map_frame_bundle_path(&dir, "watter").expect_err("unknown pass");
        assert!(err.contains("watter"), "{err}");
        // A direct (non-directory) bundle path is authoritative; the pass name
        // is not validated because the file choice is already explicit.
        let file = dir.join("direct_bundle_that_does_not_exist.json");
        assert_eq!(
            resolve_map_frame_bundle_path(&file, "watter").expect("direct file"),
            file
        );
    }

    #[test]
    fn json_mini_parser_extracts_nested_values_and_reports_typed_errors() {
        let text = r#"{"outer": {"desc": {"width": 4, "height": 2}, "items": [{"name": "a\"b\\c", "count": 7}], "path": "unïcode.bin"}}"#;
        let outer = json_object_after(text, "\"outer\"").expect("outer object");
        let desc = json_object_after(outer, "\"desc\"").expect("desc object");
        assert_eq!(json_u64_value(desc, "width").expect("width"), 4);
        assert_eq!(json_u64_value(desc, "height").expect("height"), 2);
        let items = json_array_after(outer, "\"items\"").expect("items array");
        let item = json_first_object(items).expect("first item");
        assert_eq!(json_string_value(item, "name").expect("name"), "a\"b\\c");
        assert_eq!(json_u64_value(item, "count").expect("count"), 7);
        // Multi-byte UTF-8 must round-trip exactly, not be decoded byte-per-byte.
        assert_eq!(
            json_string_value(outer, "path").expect("path"),
            "unïcode.bin"
        );
        assert!(json_object_after(text, "\"absent\"").is_err());
        assert!(json_u64_value(outer, "path").is_err());
        assert!(json_string_value(desc, "width").is_err());
        assert!(json_first_object("[]").is_err());
        assert!(balanced_json_end(b"{\"open\": [1, 2}", 0).is_err());
        assert!(balanced_json_end(b"{\"never\": 1", 0).is_err());
    }

    #[test]
    fn captured_rt_meta_parses_synthetic_bundle_and_refuses_wrong_payload_format() {
        let dir = std::env::temp_dir().join(format!(
            "replay_qa_bundle_meta_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&dir).expect("create temp bundle dir");
        let bundle_json = dir.join("map_frame_bundle.json");
        let text = r#"{
  "capture_moment": "active_pass",
  "post_pass_output_merger": {
    "render_targets": [
      {
        "desc": {"width": 2, "height": 2, "format": 87},
        "post_pass_payload": {
          "file": "frame.d3d11",
          "byte_len": 24,
          "fnv1a64": "00112233aabbccdd",
          "payload_format": "d3d11_mapped_subresources",
          "subresources": [
            {"file_offset": 0, "byte_len": 24, "row_pitch": 12, "depth_pitch": 24, "width": 2, "height": 2}
          ]
        }
      }
    ]
  }
}"#;
        fs::write(&bundle_json, text).expect("write bundle json");
        let meta = load_captured_rt_meta(&dir, "terrain").expect("parse synthetic bundle");
        assert_eq!(meta.bundle_json, bundle_json);
        assert_eq!(meta.width, 2);
        assert_eq!(meta.height, 2);
        assert_eq!(meta.dxgi_format, 87);
        assert_eq!(meta.payload_file, "frame.d3d11");
        assert_eq!(meta.payload_byte_len, 24);
        assert_eq!(meta.payload_fnv1a64, "00112233aabbccdd");
        assert_eq!(meta.file_offset, 0);
        assert_eq!(meta.subresource_byte_len, 24);
        assert_eq!(meta.row_pitch, 12);
        assert_eq!(meta.depth_pitch, 24);
        assert_eq!(meta.subresource_width, 2);
        assert_eq!(meta.subresource_height, 2);

        let wrong = text.replace("d3d11_mapped_subresources", "raw_rows");
        fs::write(&bundle_json, wrong).expect("rewrite bundle json");
        let err = load_captured_rt_meta(&dir, "terrain").expect_err("payload format refusal");
        assert!(err.contains("payload_format"), "{err}");

        let err = load_captured_rt_meta(&dir, "watter").expect_err("unknown pass refusal");
        assert!(err.contains("watter"), "{err}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn failure_detail_uses_last_error_line_and_skips_backtrace_note() {
        let text = "replay_app: POV picker available\n\
                    replay_app: fatal: recording rejected\n\
                    note: run with RUST_BACKTRACE=1\n\n";
        assert_eq!(
            last_nonempty_line(text),
            Some("replay_app: fatal: recording rejected")
        );
        assert_eq!(
            first_nonempty_line(text),
            Some("replay_app: POV picker available")
        );
        assert_eq!(last_nonempty_line("\n \n"), None);
    }

    fn bmp32(width: u32, height: u32, pixels_top_down: &[(u8, u8, u8)]) -> Vec<u8> {
        assert_eq!(pixels_top_down.len(), (width * height) as usize);
        let row_bytes = width as usize * 4;
        let image_bytes = row_bytes * height as usize;
        let file_bytes = 54 + image_bytes;
        let mut out = Vec::with_capacity(file_bytes);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&(file_bytes as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&54u32.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&(width as i32).to_le_bytes());
        out.extend_from_slice(&(height as i32).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(image_bytes as u32).to_le_bytes());
        out.extend_from_slice(&2835i32.to_le_bytes());
        out.extend_from_slice(&2835i32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        for y in (0..height as usize).rev() {
            for x in 0..width as usize {
                let (r, g, b) = pixels_top_down[y * width as usize + x];
                out.extend_from_slice(&[b, g, r, 255]);
            }
        }
        out
    }
}
