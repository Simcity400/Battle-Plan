//! A product recording dump executes the same XMAP22 packet-native composition used by windowed playback.
//! It selects one timeline hour before publishing the final BMP and its
//! content-bound composition receipt.
//!
//! Retained bundle-based and explicit evidence helpers remain non-product
//! screen-free tooling; they may render/read back evidence without authorizing
//! product-map output.
//!
//! GATE INPUT: xmap00's shipping_help foundation-gate test requires the first
//! line of this header verbatim in the file's prelude; do not delete it
//! without re-anchoring that test.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use map_viewer::GameFileResolver;
use sha2::{Digest, Sha256};

use crate::bundle::MapBundle;
use crate::camera::Camera;
use crate::d3d::Gfx;
use crate::overlay::Overlay;
use crate::render::{validate_day_night_hour, MapRenderer, POLITICAL_COLOR_BLOCKER_BADGE};
use crate::timeline::{build_provider, resolve_dump_hour, DumpHourRequest, OwnerProvider};
use crate::{Result, TimelineMode};

const DUMP_WIDTH: u32 = 1408;
const DUMP_HEIGHT: u32 = 512;
static IMMUTABLE_PUBLISH_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) fn verify_xmap22_dump(path: &Path) -> Result<()> {
    let receipt_path = xmap22_receipt_path(path);
    let receipt_bytes = std::fs::read(&receipt_path).map_err(|error| {
        format!(
            "read XMAP22 composition receipt {}: {error}",
            receipt_path.display()
        )
    })?;
    let receipt = crate::app::ProductMapCompositionReceipt::decode_json(
        &receipt_bytes,
        &format!("XMAP22 composition receipt {}", receipt_path.display()),
    )?;
    receipt.validate()?;
    let (width, height, pixels) = read_xmap22_bmp(path)?;
    if width != receipt.viewport_width || height != receipt.viewport_height {
        return Err(format!(
            "XMAP22 BMP dimensions {width}x{height} disagree with receipt {}x{}",
            receipt.viewport_width, receipt.viewport_height
        )
        .into());
    }
    let observed = snapshot::sha256::Sha256::hex(&pixels);
    if observed != receipt.final_pixels_sha256 {
        return Err(format!(
            "XMAP22 BMP pixels hash to {observed}, expected {}",
            receipt.final_pixels_sha256
        )
        .into());
    }
    println!(
        "verified XMAP22 dump: {} (receipt {})",
        path.display(),
        receipt.deterministic_sha256
    );
    Ok(())
}

pub(crate) const FRAME_DUMP_REFUSAL_SCHEMA: &str = "hoi4.frame-dump-refusal.v1";

/// The per-family account of a dump that published no frame. A dump that seals
/// publishes the XMAP22 composition receipt instead, whose
/// `mandatory_family_execution_census` is the same account one grain richer.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FrameDumpRefusalAccount {
    pub schema: String,
    pub foundation_envelope_sha256: String,
    pub envelope: map_viewer::FoundationRenderEnvelope,
}

impl FrameDumpRefusalAccount {
    pub(crate) fn from_envelope(envelope: &map_viewer::FoundationRenderEnvelope) -> Result<Self> {
        let account = Self {
            schema: FRAME_DUMP_REFUSAL_SCHEMA.into(),
            foundation_envelope_sha256: envelope.deterministic_sha256()?,
            envelope: envelope.clone(),
        };
        account.validate()?;
        Ok(account)
    }

    pub(crate) fn decode_json(bytes: &[u8]) -> Result<Self> {
        let value: Self = serde_json::from_slice(bytes)?;
        // `deny_unknown_fields` does not reach inside an internally tagged
        // enum's variants, so a stray field in a disposition would be dropped
        // and two different files would admit as one account. Comparing the
        // submitted tree against the re-encoded decode closes that.
        let submitted: serde_json::Value = serde_json::from_slice(bytes)?;
        if submitted != serde_json::to_value(&value)? {
            return Err(
                "frame-dump refusal account carries fields outside its admitted schema".into(),
            );
        }
        value.validate()?;
        Ok(value)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.schema != FRAME_DUMP_REFUSAL_SCHEMA {
            return Err(format!("frame-dump refusal account schema {}", self.schema).into());
        }
        // The envelope's own law is where "twenty families, in family order" is
        // enforced; the digest binds this account to that envelope and no other.
        self.envelope.validate()?;
        if self.foundation_envelope_sha256 != self.envelope.deterministic_sha256()? {
            return Err("frame-dump refusal account digest is not its own envelope's".into());
        }
        Ok(())
    }

    pub(crate) fn families_total(&self) -> usize {
        self.envelope.families.len()
    }

    pub(crate) fn families_refusing(&self) -> usize {
        self.envelope.gaps().count()
    }

    /// The refusing families, in family order, as one line each.
    pub(crate) fn refusal_lines(&self) -> Vec<String> {
        self.envelope
            .gaps()
            .map(|(family, gap)| {
                format!(
                    "{family:?} ({}) refuses: {} [{}]",
                    family.owner_packet(),
                    gap.reason,
                    gap.gap_id
                )
            })
            .collect()
    }
}

fn frame_dump_refusal_path(path: &Path) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(".refusal.json");
    PathBuf::from(value)
}

/// A dump has exactly one outcome.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DumpOutcome {
    Frame,
    Refusal,
}

impl DumpOutcome {
    fn token(self) -> &'static str {
        match self {
            Self::Frame => "frame",
            Self::Refusal => "refusal",
        }
    }

    /// The FIRST artifact this outcome publishes, and therefore the one name
    /// that settles whether the outcome happened. Every other artifact a dump
    /// writes - the XMAP22 receipt, the H2 sidecars - follows it, which
    /// `the_dump_path_publishes_the_account_and_claims_its_outcome` holds each
    /// route to, so no second name has to be remembered here.
    fn first_artifact(self, out: &Path) -> PathBuf {
        match self {
            Self::Frame => out.to_path_buf(),
            Self::Refusal => frame_dump_refusal_path(out),
        }
    }

    fn other(self) -> Self {
        match self {
            Self::Frame => Self::Refusal,
            Self::Refusal => Self::Frame,
        }
    }
}

const DUMP_OUTCOME_CLAIM_SUFFIX: &str = ".outcome";

fn dump_outcome_claim_path(out: &Path) -> PathBuf {
    let mut path = out.as_os_str().to_os_string();
    path.push(DUMP_OUTCOME_CLAIM_SUFFIX);
    PathBuf::from(path)
}

/// Both outcome paths claim the dump here before publishing anything of their
/// own, so a frame and a refusal never end up beside each other.
///
/// Two things arbitrate, because one cannot do both jobs. The other outcome's
/// first artifact is ASKED ABOUT BY NAME rather than found by folding directory
/// entries: whether two names are one file is the filesystem's judgement, not a
/// case table's, and `try_exists` turns a question it cannot answer into an
/// error instead of an absence. The claim file then settles two runs racing at
/// one path, because `hard_link` refuses a name that already exists.
///
/// The claim is PERMANENT and nothing removes it, so what a path admits is
/// settled once and for all: ONE outcome, for ever. It is not spent by the
/// claim alone - the SAME outcome may be attempted again, which is what lets a
/// dump killed in its 45-minute admission be re-run without inventing a new
/// path. What the second attempt may publish is then each artifact's own
/// business, and every one of them refuses to be republished with different
/// bytes.
fn claim_dump_outcome(out: &Path, outcome: DumpOutcome) -> Result<()> {
    let published = outcome.other().first_artifact(out);
    if published.try_exists().map_err(|error| {
        format!(
            "cannot establish whether {} was already published: {error}",
            published.display()
        )
    })? {
        return Err(format!(
            "this dump cannot publish {}: {} is the other outcome, already published",
            outcome.token(),
            published.display()
        )
        .into());
    }
    write_create_new_or_equal(
        &dump_outcome_claim_path(out),
        format!("{}\n", outcome.token()).as_bytes(),
        "dump outcome claim",
    )
    .map_err(|error| -> Box<dyn std::error::Error> {
        format!("this dump cannot publish {}: {error}", outcome.token()).into()
    })
}

fn publish_frame_dump_refusal_account(
    out: &Path,
    hour: i64,
    envelope: &map_viewer::FoundationRenderEnvelope,
) -> Result<(PathBuf, FrameDumpRefusalAccount)> {
    if envelope.hour_index != hour {
        return Err(format!(
            "the account on hand is hour {}, not the hour {hour} this dump asked for",
            envelope.hour_index
        )
        .into());
    }
    // Built and validated before the outcome is claimed, so an envelope this
    // account cannot describe leaves the dump's outcome unclaimed.
    let account = FrameDumpRefusalAccount::from_envelope(envelope)?;
    let mut bytes = serde_json::to_vec_pretty(&account)?;
    bytes.push(b'\n');
    claim_dump_outcome(out, DumpOutcome::Refusal)?;
    let path = frame_dump_refusal_path(out);
    write_create_new_or_equal(&path, &bytes, "frame-dump refusal account")?;
    Ok((path, account))
}

/// A composition that did not seal still owes the reader an account. The
/// original error stays first and unaltered; a sink failure never replaces it.
fn frame_dump_refusal_error(
    out: &Path,
    hour: i64,
    envelope: Option<&map_viewer::FoundationRenderEnvelope>,
    error: Box<dyn std::error::Error>,
) -> Box<dyn std::error::Error> {
    let Some(envelope) = envelope else {
        return error;
    };
    match publish_frame_dump_refusal_account(out, hour, envelope) {
        Ok((path, account)) => {
            let mut message = format!(
                "{error}\nno frame was published; the per-family account is {}\n\
                 {} of {} families refused this hour:",
                path.display(),
                account.families_refusing(),
                account.families_total()
            );
            for line in account.refusal_lines() {
                message.push_str("\n  ");
                message.push_str(&line);
            }
            message.into()
        }
        Err(sink) => {
            format!("{error} (the per-family account could not be written: {sink})").into()
        }
    }
}

fn xmap22_receipt_path(path: &Path) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(".xmap22.json");
    PathBuf::from(value)
}

fn read_xmap22_bmp(path: &Path) -> Result<(u32, u32, Vec<u8>)> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("read XMAP22 BMP {}: {error}", path.display()))?;
    if bytes.len() < 54 || &bytes[..2] != b"BM" {
        return Err("XMAP22 dump is not a complete BMP".into());
    }
    let u16_at = |offset: usize| u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
    let u32_at = |offset: usize| {
        u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ])
    };
    let i32_at = |offset: usize| {
        i32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ])
    };
    let pixel_offset = usize::try_from(u32_at(10))?;
    let width_i32 = i32_at(18);
    let height_i32 = i32_at(22);
    if u32_at(14) != 40
        || width_i32 <= 0
        || height_i32 <= 0
        || u16_at(26) != 1
        || u16_at(28) != 32
        || u32_at(30) != 0
    {
        return Err("XMAP22 dump is not the canonical 32-bit bottom-up BI_RGB BMP".into());
    }
    let width = u32::try_from(width_i32)?;
    let height = u32::try_from(height_i32)?;
    let row_bytes = usize::try_from(width)?
        .checked_mul(4)
        .ok_or("XMAP22 BMP row size overflow")?;
    let image_bytes = row_bytes
        .checked_mul(usize::try_from(height)?)
        .ok_or("XMAP22 BMP image size overflow")?;
    let canonical_header = canonical_bmp_header(
        width,
        height,
        u32::try_from(image_bytes).map_err(|_| "XMAP22 BMP image exceeds u32")?,
    )?;
    if bytes[..54] != canonical_header {
        return Err("XMAP22 BMP header differs from the canonical producer header".into());
    }
    let end = pixel_offset
        .checked_add(image_bytes)
        .ok_or("XMAP22 BMP extent overflow")?;
    if end != bytes.len() || u32_at(2) as usize != bytes.len() || pixel_offset != 54 {
        return Err("XMAP22 BMP byte extent is noncanonical or truncated".into());
    }
    let mut pixels = Vec::with_capacity(image_bytes);
    for y in (0..usize::try_from(height)?).rev() {
        let start = pixel_offset + y * row_bytes;
        pixels.extend_from_slice(&bytes[start..start + row_bytes]);
    }
    Ok((width, height, pixels))
}

#[derive(Debug)]
pub(crate) struct ProductionEvidenceSinkError {
    sink: &'static str,
    dump: PathBuf,
    source: Box<dyn std::error::Error>,
}

impl ProductionEvidenceSinkError {
    fn wrap(
        sink: &'static str,
        dump: &Path,
    ) -> impl FnOnce(Box<dyn std::error::Error>) -> Box<dyn std::error::Error> {
        let dump = dump.to_path_buf();
        move |source| Box::new(Self { sink, dump, source }) as Box<dyn std::error::Error>
    }

    #[cfg(test)]
    pub(crate) fn sink(&self) -> &'static str {
        self.sink
    }
}

impl std::fmt::Display for ProductionEvidenceSinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "frame-dump evidence sink '{}' failed after the frame BMP {} was already published; \
             the dump fails closed with its completion evidence withheld: {}",
            self.sink,
            self.dump.display(),
            self.source
        )
    }
}

impl std::error::Error for ProductionEvidenceSinkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

fn snow_mud_h2_receipt_path(out: &Path) -> PathBuf {
    let mut path = out.as_os_str().to_os_string();
    path.push(".snow-mud-h2-receipt.json");
    PathBuf::from(path)
}

fn write_snow_mud_h2_receipt(
    out: &Path,
    receipt: &replay_core::SnowMudReplayReadbackReceipt,
) -> Result<PathBuf> {
    write_snow_mud_h2_receipt_inner(out, receipt).map_err(ProductionEvidenceSinkError::wrap(
        "SnowMudData H2 receipt",
        out,
    ))
}

fn write_snow_mud_h2_receipt_inner(
    out: &Path,
    receipt: &replay_core::SnowMudReplayReadbackReceipt,
) -> Result<PathBuf> {
    let path = snow_mud_h2_receipt_path(out);
    let mut bytes = serde_json::to_vec_pretty(receipt)?;
    bytes.push(b'\n');
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => {
            file.write_all(&bytes)?;
            file.sync_all()?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if std::fs::read(&path)? != bytes {
                return Err(format!(
                    "SnowMudData H2 receipt path already exists with different bytes: {}",
                    path.display()
                )
                .into());
            }
        }
        Err(error) => return Err(error.into()),
    }
    Ok(path)
}

fn snow_mud_h2_payload_path(out: &Path) -> PathBuf {
    let mut path = out.as_os_str().to_os_string();
    path.push(".snow-mud-h2.tight-bgra8");
    PathBuf::from(path)
}

fn write_snow_mud_h2_evidence(
    out: &Path,
    receipt: &replay_core::SnowMudReplayReadbackReceipt,
    tight_readback_bytes: &[u8],
) -> Result<(PathBuf, PathBuf)> {
    if tight_readback_bytes.len() as u64 != receipt.verified_tight_byte_count
        || format!("{:x}", Sha256::digest(tight_readback_bytes)) != receipt.readback_sha256
    {
        return Err("SnowMudData H2 sidecar bytes differ from the receipt".into());
    }
    let receipt_path = snow_mud_h2_receipt_path(out);
    let payload_path = snow_mud_h2_payload_path(out);
    let mut receipt_bytes = serde_json::to_vec_pretty(receipt)?;
    receipt_bytes.push(b'\n');
    write_create_new_or_equal(
        &payload_path,
        tight_readback_bytes,
        "SnowMudData H2 payload",
    )?;
    write_create_new_or_equal(&receipt_path, &receipt_bytes, "SnowMudData H2 receipt")?;
    Ok((receipt_path, payload_path))
}

fn occupation_secondary_h2_receipt_path(out: &Path) -> PathBuf {
    let mut path = out.as_os_str().to_os_string();
    path.push(".occupation-secondary-h2-receipt.json");
    PathBuf::from(path)
}

fn occupation_secondary_h2_payload_path(out: &Path) -> PathBuf {
    let mut path = out.as_os_str().to_os_string();
    path.push(".occupation-secondary-h2.tight-bgra8");
    PathBuf::from(path)
}

fn write_occupation_secondary_h2_evidence(
    out: &Path,
    receipt: &replay_core::OccupationSecondaryReplayReadbackReceipt,
    tight_readback_bytes: &[u8],
) -> Result<(PathBuf, PathBuf)> {
    write_occupation_secondary_h2_evidence_inner(out, receipt, tight_readback_bytes).map_err(
        ProductionEvidenceSinkError::wrap("occupation secondary H2 evidence", out),
    )
}

fn write_occupation_secondary_h2_evidence_inner(
    out: &Path,
    receipt: &replay_core::OccupationSecondaryReplayReadbackReceipt,
    tight_readback_bytes: &[u8],
) -> Result<(PathBuf, PathBuf)> {
    if tight_readback_bytes.len() as u64 != receipt.verified_tight_byte_count
        || format!("{:x}", Sha256::digest(tight_readback_bytes)) != receipt.readback_sha256
    {
        return Err("occupation secondary H2 sidecar bytes differ from the receipt".into());
    }
    let receipt_path = occupation_secondary_h2_receipt_path(out);
    let payload_path = occupation_secondary_h2_payload_path(out);
    let mut receipt_bytes = serde_json::to_vec_pretty(receipt)?;
    receipt_bytes.push(b'\n');
    write_create_new_or_equal(&payload_path, tight_readback_bytes, "occupation H2 payload")?;
    write_create_new_or_equal(&receipt_path, &receipt_bytes, "occupation H2 receipt")?;
    Ok((receipt_path, payload_path))
}

pub(crate) fn write_create_new_or_equal(path: &Path, bytes: &[u8], label: &str) -> Result<()> {
    if path.exists() {
        if std::fs::read(path)? == bytes {
            return Ok(());
        }
        return Err(format!(
            "{label} path already exists with different bytes: {}",
            path.display()
        )
        .into());
    }

    let parent = path
        .parent()
        .ok_or_else(|| format!("{label} path has no parent: {}", path.display()))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("{label} path has no file name: {}", path.display()))?
        .to_string_lossy();
    let (temporary, mut file) = (0..64)
        .find_map(|_| {
            let counter = IMMUTABLE_PUBLISH_COUNTER.fetch_add(1, Ordering::Relaxed);
            let candidate = parent.join(format!(
                ".{file_name}.{}.{}.tmp",
                std::process::id(),
                counter
            ));
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&candidate)
            {
                Ok(file) => Some(Ok((candidate, file))),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(error) => Some(Err(error)),
            }
        })
        .transpose()?
        .ok_or_else(|| format!("could not allocate atomic {label} staging file"))?;

    let staged = (|| -> Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    })();
    drop(file);
    let result = staged.and_then(|()| match std::fs::hard_link(&temporary, path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if std::fs::read(path)? == bytes {
                Ok(())
            } else {
                Err(format!(
                    "{label} path was concurrently published with different bytes: {}",
                    path.display()
                )
                .into())
            }
        }
        Err(error) => Err(format!("atomically publish {label} {}: {error}", path.display()).into()),
    });
    let _ = std::fs::remove_file(&temporary);
    result
}

struct FrameKeying {
    owners: std::sync::Arc<std::collections::BTreeMap<u32, String>>,
    controllers: std::sync::Arc<std::collections::BTreeMap<u32, String>>,
    world_frame: Option<std::sync::Arc<replay_core::WorldFrame>>,
}

fn load_frame_keying(provider: &dyn OwnerProvider, hour: i64) -> Result<FrameKeying> {
    Ok(FrameKeying {
        owners: provider.owners_at(hour)?,
        controllers: provider.controllers_at(hour)?,
        world_frame: provider.world_frame_at(hour)?,
    })
}

pub fn dump_frame(
    bundle: Option<&MapBundle>,
    mode: &TimelineMode,
    hour: Option<i64>,
    nth_hour: Option<usize>,
    out: &Path,
    _game_resolver: Option<&GameFileResolver>,
    product_paths: &crate::install_paths::ProductPaths,
    dump_zoom: Option<f64>,
    dump_center: Option<(f64, f64)>,
    dump_hdr: Option<&Path>,
    capture_dir: Option<&Path>,
    postfx_enabled: bool,
    postfx_restore_full: bool,
    day_night_hour_override: Option<f32>,
    water_full: bool,
) -> Result<()> {
    reject_recording_dump_debug_bundle(mode, bundle.is_some())?;
    match bundle {
        Some(bundle) => dump_debug_frame(
            bundle,
            mode,
            hour,
            out,
            product_paths,
            dump_zoom,
            dump_center,
            dump_hdr,
            capture_dir,
            postfx_enabled,
            postfx_restore_full,
            day_night_hour_override,
            water_full,
        ),
        None => dump_production_recording_frame(
            mode,
            hour,
            nth_hour,
            out,
            product_paths,
            dump_zoom,
            dump_center,
            dump_hdr,
            capture_dir,
            day_night_hour_override,
        ),
    }
}

fn reject_recording_dump_debug_bundle(mode: &TimelineMode, has_bundle: bool) -> Result<()> {
    if matches!(mode, TimelineMode::Recording { .. }) && has_bundle {
        return Err(crate::blockers::reason(crate::blockers::BlockerId::RecordingBundle).into());
    }
    Ok(())
}

fn dump_debug_frame(
    bundle: &MapBundle,
    mode: &TimelineMode,
    hour: Option<i64>,
    out: &Path,
    product_paths: &crate::install_paths::ProductPaths,
    dump_zoom: Option<f64>,
    dump_center: Option<(f64, f64)>,
    dump_hdr: Option<&Path>,
    capture_dir: Option<&Path>,
    postfx_enabled: bool,
    postfx_restore_full: bool,
    day_night_hour_override: Option<f32>,
    water_full: bool,
) -> Result<()> {
    let (provider, label) = build_provider(Some(bundle), mode)?;
    let (game_resolver, _content_admission) =
        crate::content_compatibility::bind_provider_content(provider.as_ref(), product_paths)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
    let hours = provider.hours();
    if hours.is_empty() {
        return Err("timeline provider has no hours".into());
    }
    let hour = match hour {
        Some(hour) if matches!(mode, TimelineMode::Recording { .. }) => {
            resolve_dump_hour(hours, DumpHourRequest::Canonical(hour))?
        }
        Some(hour) => hour,
        None => hours[0],
    };
    let keying = load_frame_keying(provider.as_ref(), hour)?;

    let gfx = Gfx::create_offscreen(DUMP_WIDTH, DUMP_HEIGHT)?;
    if let Some(day_night_hour) = day_night_hour_override {
        validate_day_night_hour(day_night_hour)?;
    }
    let mut renderer = MapRenderer::new(
        &gfx,
        bundle,
        bundle.max_state_id(),
        capture_dir,
        Some(&game_resolver),
        provider
            .content_manifest()
            .ok_or("portable playback has no admitted content manifest")?
            .effective_vfs_manifest_sha256
            .as_str(),
        postfx_enabled,
        postfx_restore_full,
        water_full,
    )?;
    let mut overlay = Overlay::create(&gfx, Some(&game_resolver));
    let owners = keying.owners;
    let controllers = keying.controllers;
    let frame = keying.world_frame;
    renderer.update_state_politics(
        &gfx,
        bundle,
        frame.as_deref(),
        &owners,
        &controllers,
        &bundle.country_colors,
    )?;
    let mut camera = Camera::fit(
        bundle.width,
        bundle.height,
        f64::from(DUMP_WIDTH),
        f64::from(DUMP_HEIGHT),
    );
    if let Some(zoom) = dump_zoom {
        camera.zoom = zoom;
    }
    if let Some((x, y)) = dump_center {
        camera.center_x = x;
        camera.center_y = y;
    }
    camera.normalize(f64::from(DUMP_HEIGHT));
    let day_night_hour = day_night_hour_override;
    renderer.render(&gfx, &camera, day_night_hour, None)?;
    if let Some(hdr_path) = dump_hdr {
        renderer.dump_hdr_mainscene(&gfx, hdr_path)?;
    }
    let hour_label = provider.hour_label(hour)?;
    let hour_index = match hours.binary_search(&hour) {
        Ok(index) => index,
        Err(0) => 0,
        Err(index) => index - 1,
    };
    let line2 =
        format!("{label} | {POLITICAL_COLOR_BLOCKER_BADGE} | prov - state - owner - controller -");
    overlay.draw(&gfx, &hour_label, &line2, hour_index, hours.len())?;
    claim_dump_outcome(out, DumpOutcome::Frame)?;
    let pixels = gfx.read_target()?;
    write_bmp(out, DUMP_WIDTH, DUMP_HEIGHT, &pixels)?;
    let snow_mud_h2_receipt_path = renderer
        .snow_mud_h2_receipt()
        .map(|receipt| write_snow_mud_h2_receipt(out, receipt))
        .transpose()?;
    let day_night_label = match day_night_hour {
        Some(hour) => format!("{hour:.6}"),
        None => "off".to_string(),
    };
    println!(
        "wrote frame dump: {} ({label}, {}, {DUMP_WIDTH}x{DUMP_HEIGHT}, zoom {:.3}, center {:.1},{:.1}, daynight {day_night_label}, postfx {}, restore_full {}, {} controlled states, \
         overlay text {})",
        out.display(),
        hour_label,
        camera.zoom,
        camera.center_x,
        camera.center_y,
        if postfx_enabled { "on" } else { "off" },
        if postfx_restore_full { "on" } else { "off" },
        controllers.len(),
        if overlay.has_text() { "on" } else { "off" }
    );
    if let Some(path) = snow_mud_h2_receipt_path {
        println!(
            "wrote SnowMudData H2 immutable-upload/readback receipt: {}",
            path.display()
        );
    }
    if let Some(stats) = renderer.captured_pass_stats() {
        let stats_day_night = match stats.last_day_night_hour {
            Some(hour) => format!("{hour:.6}"),
            None => "off".to_string(),
        };
        println!(
            "captured pass: session {}, terrain {}x{} format {}, water {}, rivers {}, trees {}, postfx {}, restore_full {}, restore_evidence {}, daynight {stats_day_night}, last render {:.2} ms",
            stats.session_dir.display(),
            stats.target_width,
            stats.target_height,
            stats.target_format,
            if stats.has_water { "on" } else { "off" },
            if stats.has_rivers { "on" } else { "off" },
            if stats.has_trees { "on" } else { "off" },
            if stats.postfx_enabled { "on" } else { "off" },
            if stats.postfx_restore_full { "on" } else { "off" },
            if stats.restore_evidence_found { "found" } else { "none" },
            stats.last_render_ms.unwrap_or(0.0)
        );
        println!(
            "captured pass submitted-stream omissions: particles {}, entities {}, weather {}",
            if stats.particle_stream_omitted {
                "omitted"
            } else {
                "none"
            },
            if stats.entity_stream_omitted {
                "omitted"
            } else {
                "none"
            },
            if stats.weather_stream_omitted {
                "omitted"
            } else {
                "none"
            },
        );
        if let Some(water_gap) = &stats.water_gap {
            println!("captured pass water gap: {water_gap}");
        }
        for gap in &stats.layer_gaps {
            println!("captured pass layer gap: {gap}");
        }
    }
    Ok(())
}

/// Wall clock per phase of one frame dump, so where a dump spends its time is
/// measured rather than guessed.
struct DumpPhaseLog {
    start: std::time::Instant,
    last: std::time::Instant,
}

impl DumpPhaseLog {
    fn new() -> Self {
        let now = std::time::Instant::now();
        Self {
            start: now,
            last: now,
        }
    }

    fn mark(&mut self, phase: &str) {
        let now = std::time::Instant::now();
        eprintln!(
            "PHASE {phase}: {} ms (elapsed {} ms)",
            now.duration_since(self.last).as_millis(),
            now.duration_since(self.start).as_millis()
        );
        self.last = now;
    }
}

fn dump_production_recording_frame(
    mode: &TimelineMode,
    hour: Option<i64>,
    nth_hour: Option<usize>,
    out: &Path,
    product_paths: &crate::install_paths::ProductPaths,
    dump_zoom: Option<f64>,
    dump_center: Option<(f64, f64)>,
    dump_hdr: Option<&Path>,
    capture_dir: Option<&Path>,
    day_night_hour_override: Option<f32>,
) -> Result<()> {
    let mut phases = DumpPhaseLog::new();
    let provider = match mode {
        TimelineMode::Recording {
            path,
            logical_stream_id,
            timeline,
        } => {
            let source =
                crate::recording_open::open_product_recording_segment(path, *logical_stream_id)
                    .map_err(|error| format!("open recording {}: {error}", path.display()))?;
            let (provider, _label) = crate::timeline::build_recording_provider(
                None,
                source,
                *timeline,
                &path.display().to_string(),
            )?;
            provider
        }
        TimelineMode::Static1936 => return Err(
            "bundle-less --dump-frame is a production recording command and requires --recording"
                .into(),
        ),
    };
    phases.mark("recording_open");
    reject_production_dump_debug_inputs(
        dump_zoom,
        dump_center,
        dump_hdr,
        capture_dir,
        day_night_hour_override,
    )?;

    let (game_resolver, content_admission) =
        crate::content_compatibility::bind_provider_content(provider.as_ref(), product_paths)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
    phases.mark("content_admission");
    let content_manifest = provider
        .content_manifest()
        .ok_or("portable playback has no admitted content manifest after binding")?;
    let hours = provider.hours();
    let selected_hour = match (hour, nth_hour) {
        (Some(hour), None) => resolve_dump_hour(hours, DumpHourRequest::Canonical(hour))?,
        (None, Some(ordinal)) => resolve_dump_hour(hours, DumpHourRequest::Ordinal(ordinal))?,
        (None, None) => *hours.first().ok_or("timeline provider has no hours")?,
        (Some(_), Some(_)) => {
            return Err("competing canonical and ordinal dump-hour selectors".into())
        }
    };
    let frame = provider
        .world_frame_at(selected_hour)?
        .ok_or_else(|| map_viewer::refuse_missing_xmap00_authority("world_frame"))?;
    phases.mark("hour_decode");
    let (map_width, map_height) =
        map_viewer::foundation_map_dimensions(&game_resolver, &content_admission)?;
    let camera = map_viewer::ViewerMapCameraState::from_admitted_content(
        &game_resolver,
        &content_admission,
        map_width,
        map_height,
        DUMP_WIDTH,
        DUMP_HEIGHT,
        map_viewer::DEFAULT_VIEWER_CAMERA_SPEED_SETTING,
    )?;
    let viewer = map_viewer::ViewerOwnedMapState {
        viewport_width: DUMP_WIDTH,
        viewport_height: DUMP_HEIGHT,
        input_revision: camera.input_revision,
        camera: Some(camera),
        observer_country_id: None,
        map_mode_id: map_viewer::ViewerMapModeSelection::default().viewer_token(),
        map_mode_selection_epoch: 0,
        selected_object_ids: Vec::new(),
        animation_time_f32_bits: 0.0f32.to_bits(),
        frame_delta_seconds_f32_bits: 0.0f32.to_bits(),
    };
    let (terrain_options, render_settings_pin) =
        crate::install_paths::playback_render_settings(
            product_paths,
            &content_manifest.required_settings,
        )?;
    crate::app::announce_game_file_ledger(
        &content_admission,
        provider.product_admission(),
        &render_settings_pin,
        product_paths,
        &game_resolver.mounted_layer_roots(),
        &content_manifest.localisation_language,
    );
    let mut composition = crate::app::ProductMapCompositionSession::start_for_headless(
        &game_resolver,
        &content_admission,
        content_manifest,
        terrain_options,
        DUMP_WIDTH,
        DUMP_HEIGHT,
        provider.foundation_content(),
    )?;
    phases.mark("render_setup");
    let ui_profile = crate::install_paths::playback_ui_profile_from_paths(
        product_paths,
        &game_resolver,
        crate::win32::actual_client_input_variant(),
    )?;
    let xmap15_selection = map_viewer::xmap15_viewer_selection(&frame, None);
    let executed = composition.execute(
        &game_resolver,
        &content_admission,
        content_manifest,
        &frame,
        &viewer,
        &xmap15_selection,
        ui_profile.localisation_language(),
        0.0,
        false,
        false,
        ui_profile.key().gui_scale(),
    );
    phases.mark("render_execute");
    let receipt = match executed {
        Ok(receipt) => receipt,
        Err(error) => {
            return Err(frame_dump_refusal_error(
                out,
                selected_hour,
                composition.last_foundation_envelope(),
                error,
            ))
        }
    };
    claim_dump_outcome(out, DumpOutcome::Frame)?;
    let pixels = composition.read_target()?;
    if snapshot::sha256::Sha256::hex(&pixels) != receipt.final_pixels_sha256 {
        return Err("XMAP22 headless readback changed after the sealed composition".into());
    }
    write_bmp(out, DUMP_WIDTH, DUMP_HEIGHT, &pixels)?;
    phases.mark("readback_and_write");
    let receipt_path = xmap22_receipt_path(out);
    let mut receipt_bytes = serde_json::to_vec_pretty(&receipt)?;
    receipt_bytes.push(b'\n');
    write_create_new_or_equal(&receipt_path, &receipt_bytes, "XMAP22 composition receipt")?;
    println!(
        "wrote frame dump: {} ({DUMP_WIDTH}x{DUMP_HEIGHT}, hour {selected_hour}), receipt {}",
        out.display(),
        receipt_path.display()
    );
    Ok(())
}

fn reject_production_dump_debug_inputs(
    dump_zoom: Option<f64>,
    dump_center: Option<(f64, f64)>,
    dump_hdr: Option<&Path>,
    capture_dir: Option<&Path>,
    day_night_hour_override: Option<f32>,
) -> Result<()> {
    use crate::blockers::{reason, BlockerId};
    if dump_zoom.is_some() {
        return Err(reason(BlockerId::RecordingDumpZoom).into());
    }
    if dump_center.is_some() {
        return Err(reason(BlockerId::RecordingDumpCenter).into());
    }
    if dump_hdr.is_some() {
        return Err(reason(BlockerId::RecordingDumpHdr).into());
    }
    if capture_dir.is_some() {
        return Err(reason(BlockerId::RecordingCapture).into());
    }
    if day_night_hour_override.is_some() {
        return Err(reason(BlockerId::RecordingDayNightHour).into());
    }
    Ok(())
}

fn write_bmp(path: &Path, width: u32, height: u32, pixels: &[u8]) -> Result<()> {
    let row_bytes = width as usize * 4;
    if pixels.len() != row_bytes * height as usize {
        return Err(format!(
            "pixel buffer is {} bytes, expected {}",
            pixels.len(),
            row_bytes * height as usize
        )
        .into());
    }
    let image_bytes = pixels.len() as u32;
    let header = canonical_bmp_header(width, height, image_bytes)?;
    let file_bytes = 54 + image_bytes;
    let mut out = Vec::with_capacity(file_bytes as usize);
    out.extend_from_slice(&header);
    for y in (0..height as usize).rev() {
        out.extend_from_slice(&pixels[y * row_bytes..(y + 1) * row_bytes]);
    }
    write_create_new_or_equal(path, &out, "headless BGRA BMP")
}

fn canonical_bmp_header(width: u32, height: u32, image_bytes: u32) -> Result<[u8; 54]> {
    let file_bytes = 54u32
        .checked_add(image_bytes)
        .ok_or("XMAP22 BMP file size overflow")?;
    let width = i32::try_from(width).map_err(|_| "XMAP22 BMP width exceeds i32")?;
    let height = i32::try_from(height).map_err(|_| "XMAP22 BMP height exceeds i32")?;
    let mut out = Vec::with_capacity(54);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&file_bytes.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&image_bytes.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.try_into()
        .map_err(|_| "XMAP22 canonical BMP header length is not 54".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use snapshot::{pack, PackEntry, PackIdentity};

    fn account_digest(seed: u8) -> String {
        snapshot::sha256::Sha256::hex(&[seed])
    }

    fn account_envelope(
        overrides: &[(map_viewer::MapVisualFamily, map_viewer::FamilyDisposition)],
        omit: Option<map_viewer::MapVisualFamily>,
    ) -> map_viewer::FoundationRenderEnvelope {
        let families = map_viewer::MapVisualFamily::ALL
            .into_iter()
            .filter(|family| Some(*family) != omit)
            .map(|family| map_viewer::FamilyCoverageRow {
                family,
                owner_packet: family.owner_packet().into(),
                disposition: overrides
                    .iter()
                    .find(|(over, _)| *over == family)
                    .map(|(_, disposition)| disposition.clone())
                    .unwrap_or(map_viewer::FamilyDisposition::Present {
                        payload_schema: "hoi4.test.v1".into(),
                        producer_registration_sha256: account_digest(1),
                        payload_sha256: account_digest(2),
                        ordered_operations: Vec::new(),
                        ordered_used_content: Vec::new(),
                    }),
            })
            .collect();
        map_viewer::FoundationRenderEnvelope {
            schema: map_viewer::FOUNDATION_RENDER_SCHEMA.into(),
            hour_index: 17_030_998,
            content_manifest_sha256: account_digest(3),
            product_modset_sha256: account_digest(4),
            effective_vfs_manifest_sha256: account_digest(5),
            foundation_content_operation_sha256: account_digest(6),
            families,
            element_diagnostics: Vec::new(),
        }
    }

    fn refusing_labels() -> (map_viewer::MapVisualFamily, map_viewer::FamilyDisposition) {
        (
            map_viewer::MapVisualFamily::LabelsMapText,
            map_viewer::FamilyDisposition::Unavailable {
                gap: map_viewer::FoundationGap {
                    gap_id: "XMAP17.unbound_runtime_variable".into(),
                    first_unavailable_hour: 17_030_998,
                    last_unavailable_hour: 17_030_998,
                    witness_sha256: account_digest(7),
                    reason: "no operand binds OVERLORDADJ".into(),
                },
            },
        )
    }

    /// kills: an account that says less than the preparation knew, or that
    /// reads as an account it is not.
    #[test]
    fn the_refusal_account_covers_every_family_and_names_each_refusal() {
        let total = map_viewer::MapVisualFamily::ALL.len();
        let vouched = FrameDumpRefusalAccount::from_envelope(&account_envelope(&[], None)).unwrap();
        assert_eq!(vouched.families_total(), total);
        assert_eq!(vouched.families_refusing(), 0);
        assert!(vouched.refusal_lines().is_empty());

        let deferred = (
            map_viewer::MapVisualFamily::GradingPostFx,
            map_viewer::FamilyDisposition::DeferredToDraw {
                producer_registration_sha256: account_digest(8),
                draw_contract_id: "XMAP10.postfx".into(),
                reason: "answered per frame".into(),
            },
        );
        let refused = account_envelope(&[refusing_labels(), deferred], None);
        let account = FrameDumpRefusalAccount::from_envelope(&refused).unwrap();
        assert_eq!(account.families_total(), total);
        assert_eq!(account.families_refusing(), 1);
        assert_eq!(account.envelope, refused);
        assert_eq!(
            account.refusal_lines(),
            vec![concat!(
                "LabelsMapText (XMAP17) refuses: no operand binds OVERLORDADJ",
                " [XMAP17.unbound_runtime_variable]"
            )
            .to_string()]
        );
        // A deferral is not a refusal and never appears as one.
        assert!(!account.refusal_lines().iter().any(|line| line.contains("XMAP10")));

        // An envelope that is not twenty families in family order is refused by
        // the envelope's own law, which this account is required to run.
        assert!(FrameDumpRefusalAccount::from_envelope(&account_envelope(
            &[],
            Some(map_viewer::MapVisualFamily::Trees)
        ))
        .is_err());

        // A file carrying anything outside the schema is refused rather than
        // normalised into an account that is not what was written.
        let bytes = serde_json::to_vec(&account).unwrap();
        let decoded = FrameDumpRefusalAccount::decode_json(&bytes).unwrap();
        assert_eq!(decoded, account);
        let mut tree: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        tree["envelope"]["families"][0]["disposition"]["smuggled"] =
            serde_json::Value::from("field");
        assert!(
            FrameDumpRefusalAccount::decode_json(&serde_json::to_vec(&tree).unwrap())
                .unwrap_err()
                .to_string()
                .contains("outside its admitted schema")
        );

        // Each forgery agrees with itself and disagrees with its own envelope.
        let forge = |edit: &dyn Fn(&mut FrameDumpRefusalAccount)| {
            let mut forged = account.clone();
            edit(&mut forged);
            forged.validate().unwrap_err().to_string()
        };
        assert!(
            forge(&|forged| forged.envelope.hour_index += 1).contains("is not its own envelope's"),
            "an account for another hour must not read as this one's"
        );
        assert!(
            forge(&|forged| forged.envelope.effective_vfs_manifest_sha256 = account_digest(9))
                .contains("is not its own envelope's"),
            "an account for another install must not read as this one's"
        );
        assert!(forge(&|forged| forged.schema = "hoi4.frame-dump-refusal.v2".into())
            .contains("frame-dump refusal account schema"));
    }

    /// kills: publishing a refusal account beside a published frame or a frame
    /// beside a published refusal; overwriting one run's account with another's;
    /// and losing the composition error when there is no account to publish.
    #[test]
    fn a_dump_publishes_one_outcome_and_its_account_is_immutable() {
        let root = std::env::temp_dir().join(format!(
            "battle-plan-wb-refusal-{}-{}",
            std::process::id(),
            IMMUTABLE_PUBLISH_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let out = root.join("frame.bmp");
        let refused = account_envelope(&[refusing_labels()], None);

        // No envelope means no account, and the composition error survives whole.
        let hour = 17_030_998i64;
        let bare = frame_dump_refusal_error(&out, hour, None, "the composition failed".into());
        assert_eq!(bare.to_string(), "the composition failed");
        assert!(!frame_dump_refusal_path(&out).exists());

        // An account for another hour is not this dump's outcome and is never
        // published as one; the composition error survives that refusal.
        let wrong_hour = frame_dump_refusal_error(
            &out,
            hour + 1,
            Some(&refused),
            "XMAP22 cannot seal this frame".into(),
        )
        .to_string();
        assert!(wrong_hour.contains("not the hour 17030999 this dump asked for"), "{wrong_hour}");
        assert!(wrong_hour.starts_with("XMAP22 cannot seal this frame"), "{wrong_hour}");
        assert!(!frame_dump_refusal_path(&out).exists());

        let reported = frame_dump_refusal_error(
            &out,
            hour,
            Some(&refused),
            "XMAP22 cannot seal this frame".into(),
        )
        .to_string();
        assert!(reported.starts_with("XMAP22 cannot seal this frame"), "{reported}");
        assert!(reported.contains("1 of 20 families refused this hour"), "{reported}");
        assert!(reported.contains("XMAP17.unbound_runtime_variable"), "{reported}");
        assert!(reported.contains("no operand binds OVERLORDADJ"), "{reported}");

        let path = frame_dump_refusal_path(&out);
        let written =
            FrameDumpRefusalAccount::decode_json(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(written.envelope, refused);

        // Republishing the same account is the same account; a different one is
        // refused rather than overwriting the first.
        publish_frame_dump_refusal_account(&out, hour, &refused).unwrap();
        assert!(
            publish_frame_dump_refusal_account(&out, hour, &account_envelope(&[], None))
                .unwrap_err()
                .to_string()
                .contains("already exists with different bytes")
        );

        // A dump has one outcome, in either order: a published refusal stops a
        // frame, and a published frame stops a refusal.
        assert!(claim_dump_outcome(&out, DumpOutcome::Frame)
            .unwrap_err()
            .to_string()
            .contains("this dump cannot publish frame"));
        let sealed = root.join("sealed.bmp");
        claim_dump_outcome(&sealed, DumpOutcome::Frame).unwrap();
        write_bmp(&sealed, 1, 1, &[0, 0, 0, 255]).unwrap();
        assert!(publish_frame_dump_refusal_account(&sealed, hour, &refused)
            .unwrap_err()
            .to_string()
            .contains("this dump cannot publish refusal"));
        assert!(!frame_dump_refusal_path(&sealed).exists());

        // An artifact published without a claim - by a run that predates the
        // claim, or one killed between publishing and claiming - still settles
        // the outcome, because the claim is not the only arbiter. Each outcome
        // is settled by the ONE artifact it publishes first; everything else a
        // dump writes follows that one, which the wiring guard holds each route
        // to.
        for (name, blocked) in [
            ("bare.bmp", DumpOutcome::Refusal),
            ("refused.bmp", DumpOutcome::Frame),
        ] {
            let path = root.join(name);
            let published = blocked.other().first_artifact(&path);
            std::fs::write(&published, b"published\n").unwrap();
            let error = claim_dump_outcome(&path, blocked).unwrap_err().to_string();
            assert!(error.contains("is the other outcome, already published"), "{error}");
            assert!(error.contains(&published.display().to_string()), "{error}");
            assert!(
                !dump_outcome_claim_path(&path).exists(),
                "a refused claim was still taken"
            );
        }

        // What a path admits is settled once and for all. The SAME outcome may
        // be attempted again - a dump killed in its admission is re-run at the
        // path it already claimed - and the other outcome never may.
        let settled = root.join("settled.bmp");
        claim_dump_outcome(&settled, DumpOutcome::Frame).unwrap();
        claim_dump_outcome(&settled, DumpOutcome::Frame).unwrap();
        assert!(dump_outcome_claim_path(&settled).exists());
        assert!(claim_dump_outcome(&settled, DumpOutcome::Refusal)
            .unwrap_err()
            .to_string()
            .contains("this dump cannot publish refusal"));
        // ...and a second attempt at the same outcome still cannot publish
        // different bytes over what the first one left.
        write_bmp(&settled, 1, 1, &[0, 0, 0, 255]).unwrap();
        assert!(write_bmp(&settled, 1, 1, &[255, 255, 255, 255])
            .unwrap_err()
            .to_string()
            .contains("already exists with different bytes"));

        // The two outcomes name two different first artifacts, and each asks
        // about the other's by name rather than deciding which directory
        // entries look like its own.
        let named = root.join("named.bmp");
        assert_eq!(DumpOutcome::Frame.first_artifact(&named), named);
        assert_eq!(
            DumpOutcome::Refusal.first_artifact(&named),
            frame_dump_refusal_path(&named)
        );
        assert_eq!(DumpOutcome::Frame.other(), DumpOutcome::Refusal);
        assert_eq!(DumpOutcome::Refusal.other(), DumpOutcome::Frame);

        // A neighbour that merely starts with the same name is nobody's
        // artifact and settles nothing.
        let bounded = root.join("bounded.bmp");
        for neighbour in ["bounded.bmp2", "bounded.bmpx", "bounded.bmp-backup"] {
            std::fs::write(root.join(neighbour), b"not ours\n").unwrap();
        }
        claim_dump_outcome(&bounded, DumpOutcome::Refusal).unwrap();

        // Absence has to be established, not assumed: a path whose parent
        // directory is missing has nothing published at it, and a question the
        // filesystem cannot answer is an error rather than an absence.
        let missing = root.join("no-such-directory").join("frame.bmp");
        claim_dump_outcome(&missing, DumpOutcome::Frame).unwrap_err();
        std::fs::remove_dir_all(&root).ok();
    }

    /// One function's body, found by BALANCING BRACES rather than by a sentinel
    /// in the text after it, so an unrelated edit nearby cannot move it.
    fn function_body<'a>(source: &'a str, signature: &str) -> &'a str {
        let after = source
            .split_once(signature)
            .unwrap_or_else(|| panic!("{signature} must exist"))
            .1;
        let open = after
            .find('{')
            .unwrap_or_else(|| panic!("{signature} has no body"));
        let body = &after[open + 1..];
        let bytes = body.as_bytes();
        let (mut depth, mut index) = (1usize, 0usize);
        let (mut in_string, mut in_char, mut escaped, mut in_comment) =
            (false, false, false, false);
        while index < bytes.len() {
            let byte = bytes[index];
            if in_comment {
                if byte == b'\n' {
                    in_comment = false;
                }
            } else if escaped {
                escaped = false;
            } else if byte == b'\\' && (in_string || in_char) {
                escaped = true;
            } else if in_string {
                in_string = byte != b'"';
            } else if in_char {
                in_char = byte != b'\'';
            } else if byte == b'"' {
                in_string = true;
            } else if byte == b'\'' && bytes.get(index + 1) != Some(&b'a') {
                // A lifetime (`'a`) is not a character literal. The bodies this
                // walks carry no other apostrophe outside a string.
                in_char = true;
            } else if byte == b'/' && bytes.get(index + 1) == Some(&b'/') {
                in_comment = true;
            } else if byte == b'{' {
                depth += 1;
            } else if byte == b'}' {
                depth -= 1;
                if depth == 0 {
                    return &body[..index];
                }
            }
            index += 1;
        }
        panic!("{signature} body is not brace-balanced");
    }

    /// The wiring the helpers above cannot reach on their own: only a real dump
    /// of a real capture executes it, so it is pinned at the source.
    ///
    /// kills: a dump that returns its composition error without publishing the
    /// account; either frame publisher losing its outcome claim or publishing
    /// anything before it; and a foundation session that stops recording the
    /// envelope a refusing hour leaves behind, or records it too late.
    #[test]
    fn the_dump_path_publishes_the_account_and_claims_its_outcome() {
        let source = crate::source_scan::headless_production();
        let flatten = |body: &str| body.split_whitespace().collect::<Vec<_>>().join(" ");
        let production = flatten(function_body(&source, "fn dump_production_recording_frame("));
        assert!(
            production.contains(concat!(
                "Err(error) => { return Err(frame_dump_refusal_error( out, selected_hour,",
                " composition.last_foundation_envelope(), error, )) }"
            )),
            "the dump no longer publishes an account for a composition it could not seal"
        );

        for (route, frame) in [
            ("the production dump", &production),
            (
                "the debug dump",
                &flatten(function_body(&source, "fn dump_debug_frame(")),
            ),
        ] {
            assert_eq!(
                frame
                    .matches("claim_dump_outcome(out, DumpOutcome::Frame)?;")
                    .count(),
                1,
                "{route} publishes a frame without claiming the outcome exactly once"
            );
            let claim = frame
                .find("claim_dump_outcome(out, DumpOutcome::Frame)?;")
                .expect("the claim was just counted");
            for published in [
                "read_target()?",
                "write_bmp(out, DUMP_WIDTH, DUMP_HEIGHT, &pixels)?;",
            ] {
                let at = frame
                    .find(published)
                    .unwrap_or_else(|| panic!("{route} no longer publishes {published}"));
                assert!(claim < at, "{route} publishes {published} before its claim");
            }
            // Every artifact derived from `out` follows THE BMP, not merely the
            // claim: `DumpOutcome::first_artifact` names one artifact per
            // outcome and asks about the other's, which only settles the
            // question while nothing this route writes can appear before it.
            // `--dump-hdr` is not among them: its path is the caller's, not one
            // this dump derives.
            let bmp = frame
                .find("write_bmp(out, DUMP_WIDTH, DUMP_HEIGHT, &pixels)?;")
                .expect("the BMP write was just required");
            for sidecar in [
                "write_snow_mud_h2_receipt(",
                "write_occupation_secondary_h2_evidence(",
                "xmap22_receipt_path(out)",
            ] {
                if let Some(at) = frame.find(sidecar) {
                    assert!(bmp < at, "{route} publishes {sidecar} before the frame it belongs to");
                }
            }
        }

        // Each preparation clears the previous hour's account as its FIRST
        // statement and records between the family loop and the playback
        // admission that rejects a refusing hour.
        let session = crate::source_scan::validate_production();
        for method in [
            "pub(crate) fn prepare_with_xmap12_inputs(",
            "pub(crate) fn prepare_with_xmap12_and_xmap15_inputs(",
        ] {
            let raw = function_body(&session, method);
            let prepare = flatten(raw);
            let at = |needle: &str| {
                prepare
                    .find(needle)
                    .unwrap_or_else(|| panic!("{method} no longer has: {needle}"))
            };
            let first = raw
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty() && !line.starts_with("//"))
                .unwrap_or_else(|| panic!("{method} has no statements"));
            assert_eq!(
                first, "self.last_candidate_envelope = None;",
                "{method} does something before clearing the previous hour's account"
            );
            let recorded = at("self.last_candidate_envelope = Some(candidate.envelope.clone());");
            assert!(
                at("let candidate = self.adapters.prepare_with_xmap12") < recorded
                    && recorded < at(".admit_allowing_xmap16_unavailable("),
                "{method} records the envelope outside the window it exists in"
            );
        }
    }

    #[test]
    fn xmap22_bmp_decoder_round_trips_top_down_bgra_and_rejects_trailing_bytes() {
        let root = std::env::temp_dir().join(format!(
            "battle-plan-xmap22-bmp-{}-{}",
            std::process::id(),
            IMMUTABLE_PUBLISH_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("frame.bmp");
        let pixels = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        write_bmp(&path, 2, 2, &pixels).unwrap();
        let (width, height, decoded) =
            portable_test_fs_retry::with_read_guard(&path, || read_xmap22_bmp(&path))
                .expect("hold the freshly written BMP open across the scanner's window")
                .unwrap();
        assert_eq!((width, height), (2, 2));
        assert_eq!(decoded, pixels);

        let mut corrupt = portable_test_fs_retry::read(&path).unwrap();
        corrupt.push(0);
        let corrupt_path = root.join("corrupt.bmp");
        std::fs::write(&corrupt_path, corrupt).unwrap();
        assert!(
            portable_test_fs_retry::with_read_guard(&corrupt_path, || read_xmap22_bmp(
                &corrupt_path
            ))
            .expect("hold the corrupt fixture open across the scanner's window")
            .is_err()
        );

        for offset in [6usize, 34, 38, 42, 46, 50] {
            let mut corrupt = portable_test_fs_retry::read(&path).unwrap();
            corrupt[offset] ^= 1;
            let corrupt_path = root.join(format!("corrupt-header-{offset}.bmp"));
            std::fs::write(&corrupt_path, corrupt).unwrap();
            assert!(
                portable_test_fs_retry::with_read_guard(&corrupt_path, || read_xmap22_bmp(
                    &corrupt_path
                ))
                .expect("hold the corrupt fixture open across the scanner's window")
                .is_err(),
                "offset {offset}"
            );
        }
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn immutable_authority_publication_is_atomic_idempotent_and_no_replace() {
        let root = std::env::temp_dir().join(format!(
            "battle-plan-receipt-publish-{}-{}",
            std::process::id(),
            IMMUTABLE_PUBLISH_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("receipt.json");
        write_create_new_or_equal(&path, b"complete receipt bytes\n", "test receipt").unwrap();
        write_create_new_or_equal(&path, b"complete receipt bytes\n", "test receipt").unwrap();
        assert!(write_create_new_or_equal(&path, b"different\n", "test receipt").is_err());
        assert_eq!(portable_test_fs_retry::read(&path).unwrap(), b"complete receipt bytes\n");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn post_publication_h2_evidence_sink_failure_is_typed() {
        let root = std::env::temp_dir().join(format!(
            "battle-plan-evidence-sink-{}-{}",
            std::process::id(),
            IMMUTABLE_PUBLISH_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let out = root.join("frame.bmp");
        std::fs::write(&out, b"published bmp bytes").unwrap();
        std::fs::write(snow_mud_h2_receipt_path(&out), b"different\n").unwrap();
        let snow_receipt = replay_core::SnowMudReplayReadbackReceipt {
            schema: "test".to_string(),
            process_start_nonce: 1,
            hook_epoch_nonce: 1,
            producer_ordinal: 1,
            producer_game_hour_raw: 0,
            display_game_hour_raw: 0,
            selected_candidate_id: 1,
            selected_qpc_ticks: 1,
            selected_present_sha256: String::new(),
            snow_mud_fade_f32_bits: 0,
            snow_mud_fade_consumer_count: 0,
            snow_mud_fade_consumer_set_sha256: String::new(),
            observation_sha256: String::new(),
            payload_sha256: String::new(),
            readback_sha256: String::new(),
            width: 1,
            height: 1,
            source_row_pitch: 4,
            mapped_row_pitch: 4,
            verified_row_count: 1,
            verified_tight_byte_count: 4,
            replaced_binding_count: 1,
            receipt_sha256: String::new(),
        };
        let error = write_snow_mud_h2_receipt(&out, &snow_receipt)
            .expect_err("a colliding snow/mud receipt with different bytes must refuse");
        let typed = error
            .downcast_ref::<ProductionEvidenceSinkError>()
            .expect("the snow/mud sink failure must be the typed classification");
        assert_eq!(typed.sink(), "SnowMudData H2 receipt");
        assert!(typed.to_string().contains("already published"), "{typed}");
        assert!(
            std::error::Error::source(typed).is_some(),
            "the underlying collision error must be preserved as the source"
        );
        assert_eq!(portable_test_fs_retry::read(&out).unwrap(), b"published bmp bytes");

        let payload = b"bgra";
        std::fs::write(occupation_secondary_h2_payload_path(&out), b"other\n").unwrap();
        let occupation_receipt = replay_core::OccupationSecondaryReplayReadbackReceipt {
            schema: "test".to_string(),
            target_binding_identity: String::new(),
            upload_order: 1,
            selected_present_sha256: String::new(),
            payload_sha256: String::new(),
            readback_sha256: format!("{:x}", Sha256::digest(payload)),
            width: 1,
            height: 1,
            source_row_pitch: 4,
            mapped_row_pitch: 4,
            verified_row_count: 1,
            verified_tight_byte_count: payload.len() as u64,
            terrain_t6_binding_count: 1,
            water_t3_binding_count: 0,
            receipt_sha256: String::new(),
        };
        let error = write_occupation_secondary_h2_evidence(&out, &occupation_receipt, payload)
            .expect_err("a colliding occupation payload with different bytes must refuse");
        let typed = error
            .downcast_ref::<ProductionEvidenceSinkError>()
            .expect("the occupation sink failure must be the typed classification");
        assert_eq!(typed.sink(), "occupation secondary H2 evidence");
        assert!(typed.to_string().contains("already published"), "{typed}");

        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn snow_mud_h2_evidence_publishes_raw_plane_with_receipt_and_refuses_mismatched_bytes() {
        let bytes: Vec<u8> = (0..64u8).collect();
        let receipt = replay_core::SnowMudReplayReadbackReceipt {
            schema: replay_core::SNOW_MUD_H2_READBACK_SCHEMA.into(),
            process_start_nonce: 1,
            hook_epoch_nonce: 2,
            producer_ordinal: 3,
            producer_game_hour_raw: 4,
            display_game_hour_raw: 4,
            selected_candidate_id: 5,
            selected_qpc_ticks: 6,
            selected_present_sha256: "11".repeat(32),
            snow_mud_fade_f32_bits: 0,
            snow_mud_fade_consumer_count: 1,
            snow_mud_fade_consumer_set_sha256: "22".repeat(32),
            observation_sha256: "33".repeat(32),
            payload_sha256: "44".repeat(32),
            readback_sha256: format!("{:x}", Sha256::digest(&bytes)),
            width: 4,
            height: 4,
            source_row_pitch: 16,
            mapped_row_pitch: 16,
            verified_row_count: 4,
            verified_tight_byte_count: bytes.len() as u64,
            replaced_binding_count: 1,
            receipt_sha256: String::new(),
        };
        let root = std::env::temp_dir().join(format!(
            "battle-plan-snow-mud-h2-evidence-{}-{}",
            std::process::id(),
            IMMUTABLE_PUBLISH_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let out = root.join("hour-00000000.bmp");

        let mut wrong = bytes.clone();
        wrong[0] ^= 0xff;
        assert!(write_snow_mud_h2_evidence(&out, &receipt, &wrong).is_err());
        assert!(!snow_mud_h2_receipt_path(&out).exists());
        assert!(!snow_mud_h2_payload_path(&out).exists());

        let (receipt_path, payload_path) =
            write_snow_mud_h2_evidence(&out, &receipt, &bytes).unwrap();
        assert_eq!(portable_test_fs_retry::read(&payload_path).unwrap(), bytes);
        let written: serde_json::Value =
            serde_json::from_slice(&portable_test_fs_retry::read(&receipt_path).unwrap()).unwrap();
        assert_eq!(
            written["readback_sha256"],
            serde_json::Value::from(receipt.readback_sha256.as_str())
        );
        write_snow_mud_h2_evidence(&out, &receipt, &bytes).unwrap();

        let mut altered = receipt.clone();
        altered.replaced_binding_count += 1;
        assert!(write_snow_mud_h2_evidence(&out, &altered, &bytes).is_err());
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    fn packed_political_keying_recording() -> std::path::PathBuf {
        let mut states = serde_json::Map::new();
        for state_id in 1..=1_081u32 {
            let owner = if state_id <= 216 { "FRA" } else { "GER" };
            let controller = if state_id <= 8 { "GER" } else { owner };
            states.insert(
                state_id.to_string(),
                serde_json::json!({ "owner": owner, "controller": controller }),
            );
        }
        let mod_identity = serde_json::json!({
            "modset_id": "vanilla",
            "source": "headless_keying_test",
            "load_order": [],
            "mods": [],
        });
        let snapshot = serde_json::json!({
            "schema_version": "1",
            "game_id": "headless_1081_keying",
            "date": "1936.1.1.13",
            "hour_index": 16_959_373,
            "save_version": null,
            "provenance": {
                // A null channel is NOT the "no collector" shape. The fold
                // always writes this channel as an object carrying a
                // `domain_problems` array (`world_fold.rs:9290-9294, :9744`),
                // so `rederive_collector_domain_problems` reads a present
                // non-object as a MALFORMED envelope and fails closed --
                // every problem-backed typed view is downgraded, owners
                // included. A fixture that then asserts 1081 measured owners
                // is asserting against a recording shape no capture can
                // produce. Say what this fixture means instead: the collector
                // captured nothing here, and it declares no problem.
                "collector": {
                    "captured": false,
                    "domains": [],
                    "domain_problems": [],
                },
                "memory": null,
                "mod": mod_identity,
                "save": null,
                "notes": [],
            },
            "countries": {},
            "states": states,
            "provinces": {},
            "divisions": [],
            "combats": [],
            "diplomacy_world": {},
            "map": {},
            "designs": { "division_templates": [], "equipment_variants": {} },
            "order_of_battle": {
                "army_groups": [], "armies": [], "fleets": [], "navy_theaters": [],
                "air_wings": [], "theatres": [], "ships": [], "by_country": {},
            },
            "mio_live": {}, "market_live": {}, "special_projects": {},
            "diplomacy_live": {}, "naval_air": {}, "supply": {},
            "strategic_air": {}, "strategic_operatives": {}, "raids": {},
            "world_events": {}, "world": {}, "weather": {}, "visibility": {},
            "naval_combats": [], "models": {}, "intel_ledger": {},
        });
        let index = serde_json::json!({
            "game_id": "headless_1081_keying",
            "schema_version": "1",
            "snapshots": [{
                "hour_index": 16_959_373,
                "date": "1936.1.1.13",
                "file": "000016959373.snapshot.json",
            }],
        });
        let entries = vec![
            PackEntry {
                path: "payload/headless_1081_keying/index.json".to_string(),
                bytes: serde_json::to_vec(&index).expect("serialize index"),
                role: "index",
                hour_index: None,
            },
            PackEntry {
                path: "payload/headless_1081_keying/000016959373.snapshot.json".to_string(),
                bytes: serde_json::to_vec(&snapshot).expect("serialize snapshot"),
                role: "snapshot",
                hour_index: Some(16_959_373),
            },
        ];
        let bytes = pack(
            "headless_1081_keying",
            &entries,
            &PackIdentity {
                schema_version: Some("1".to_string()),
                mod_identity: Some(mod_identity),
                ..Default::default()
            },
            true,
        )
        .expect("pack keying recording");
        let path = crate::bundle::tests::unique_temp_dir("recording_keying")
            .join("headless_1081_keying.battleplan");
        std::fs::create_dir_all(path.parent().expect("recording parent"))
            .expect("create recording parent");
        std::fs::write(&path, bytes).expect("write keying recording");
        path
    }

    #[test]
    fn recording_keying_seam_receives_all_owners_and_controller_overrides_without_d3d() {
        let path = packed_political_keying_recording();
        let source = portable_test_fs_retry::with_read_guard(&path, || {
            replay_core::open_recording(&path)
        })
        .expect("hold the recording open across the scanner's window")
        .expect("open packed keying recording");
        let timeline = replay_core::WorldTimeline::open(source).expect("keying timeline");
        let provider = crate::timeline::RecordingTimeline::new_with_policy(timeline, None);
        let keying = load_frame_keying(&provider, 16_959_373).expect("headless keying frame");

        assert_eq!(keying.owners.len(), 1_081);
        assert_eq!(keying.controllers.len(), 1_081);
        assert_eq!(
            keying
                .controllers
                .iter()
                .filter(|(state_id, controller)| keying.owners.get(state_id) != Some(controller))
                .count(),
            8,
            "controller overrides must survive the real pre-render seam"
        );
        assert_eq!(
            keying
                .world_frame
                .as_ref()
                .expect("typed world frame")
                .hour_index(),
            16_959_373
        );
        drop(provider);
        portable_test_fs_retry::remove_dir_all(path.parent().expect("recording parent"))
            .expect("remove keying recording");
    }

    #[test]
    fn production_recording_dump_rejects_every_debug_substitute() {
        use crate::blockers::{reason, BlockerId};
        assert!(reject_production_dump_debug_inputs(None, None, None, None, None).is_ok());
        let err = reject_production_dump_debug_inputs(Some(1.0), None, None, None, None)
            .expect_err("zoom");
        assert_eq!(err.to_string(), reason(BlockerId::RecordingDumpZoom));
        let err = reject_production_dump_debug_inputs(None, Some((1.0, 2.0)), None, None, None)
            .expect_err("center");
        assert_eq!(err.to_string(), reason(BlockerId::RecordingDumpCenter));
        let err = reject_production_dump_debug_inputs(
            None,
            None,
            Some(Path::new("scene.hdr")),
            None,
            None,
        )
        .expect_err("hdr");
        assert_eq!(err.to_string(), reason(BlockerId::RecordingDumpHdr));
        let err =
            reject_production_dump_debug_inputs(None, None, None, Some(Path::new("capture")), None)
                .expect_err("capture");
        assert_eq!(err.to_string(), reason(BlockerId::RecordingCapture));
        let err = reject_production_dump_debug_inputs(None, None, None, None, Some(0.5))
            .expect_err("day/night");
        assert_eq!(err.to_string(), reason(BlockerId::RecordingDayNightHour));
    }

    #[test]
    fn production_recording_dump_rejects_debug_bundle_route() {
        let mode = TimelineMode::Recording {
            path: "portable.battleplan".into(),
            logical_stream_id: None,
            timeline: replay_core::TimelineConfig::default(),
        };
        assert!(reject_recording_dump_debug_bundle(&mode, false).is_ok());
        let err = reject_recording_dump_debug_bundle(&mode, true).expect_err("bundle");
        assert_eq!(
            err.to_string(),
            crate::blockers::reason(crate::blockers::BlockerId::RecordingBundle),
            "bundle rejection is the registry row's reason verbatim"
        );
        assert!(reject_recording_dump_debug_bundle(&TimelineMode::Static1936, true).is_ok());
    }

    #[test]
    fn headless_product_dump_executes_the_shared_xmap22_composition() {
        let (topology, schedule) =
            crate::validate::product_foundation_assigned_stage_plan().unwrap();
        assert_eq!(topology.ordered_stages().len(), 22);
        assert_eq!(topology.ordered_edges().len(), 18);
        schedule
            .require(
                map_viewer::XMAP06_EARLY_STAGE_NAME,
                map_viewer::XMAP06_EARLY_OPERATION_NAME,
            )
            .expect("headless product foundation includes XMAP06");
        schedule
            .require(
                map_viewer::XMAP08_RIVER_STAGE_NAME,
                map_viewer::XMAP08_RIVER_OPERATION_ID,
            )
            .expect("headless product foundation includes XMAP08 rivers");
        schedule
            .require(
                map_viewer::XMAP08_STRAIT_STAGE_NAME,
                map_viewer::XMAP08_STRAIT_OPERATION_ID,
            )
            .expect("headless product foundation includes XMAP08 straits");
        schedule
            .require(
                map_viewer::XMAP11_TREE_STAGE_NAME,
                map_viewer::XMAP11_TREE_OPERATION_NAME,
            )
            .expect("headless product foundation includes XMAP11 trees");
        schedule
            .require(
                map_viewer::XMAP17_EXECUTE_STAGE,
                map_viewer::XMAP17_EXECUTE_OPERATION,
            )
            .expect("headless product foundation includes XMAP17 labels");
        schedule
            .require(
                map_viewer::XMAP09_PREPARE_STAGE_NAME,
                map_viewer::XMAP09_PREPARE_OPERATION_NAME,
            )
            .expect("headless product foundation includes XMAP09");
        schedule
            .require(
                map_viewer::XMAP19_ASSIGNED_STAGE_NAME,
                map_viewer::XMAP19_ASSIGNED_OPERATION_NAME,
            )
            .expect("headless product foundation includes XMAP19");
        schedule
            .require(
                map_viewer::XMAP07_ASSIGNED_STAGE_NAME,
                map_viewer::XMAP07_ASSIGNED_OPERATION_NAME,
            )
            .expect("headless product foundation includes XMAP07");
        schedule
            .require(
                map_viewer::XMAP13_UNIT_ENTITY_STAGE_NAME,
                map_viewer::XMAP13_UNIT_ENTITY_OPERATION_NAME,
            )
            .expect("headless product foundation includes XMAP13 entities");
        schedule
            .require(
                map_viewer::XMAP13_COUNTER_QUAD_STAGE_NAME,
                map_viewer::XMAP13_COUNTER_QUAD_OPERATION_NAME,
            )
            .expect("headless product foundation includes XMAP13 counters");
        schedule
            .require(
                map_viewer::XMAP14_COMBAT_STAGE_NAME,
                map_viewer::XMAP14_COMBAT_OPERATION_NAME,
            )
            .expect("headless product foundation includes XMAP14 combat indicators");
        schedule
            .require(
                map_viewer::XMAP15_ASSIGNED_STAGE_NAME,
                map_viewer::XMAP15_ASSIGNED_OPERATION_NAME,
            )
            .expect("headless product foundation includes XMAP15 orders");
        schedule
            .require(
                map_viewer::XMAP20_ASSIGNED_STAGE_NAME,
                map_viewer::XMAP20_ASSIGNED_OPERATION_NAME,
            )
            .expect("headless product foundation includes XMAP20");
        schedule
            .require(
                map_viewer::XMAP16_RAIL_STAGE_NAME,
                map_viewer::XMAP16_RAIL_OPERATION_ID,
            )
            .expect("headless product foundation includes XMAP16");
        schedule
            .require(
                map_viewer::XMAP21_PREPARE_STAGE_NAME,
                map_viewer::XMAP21_PREPARE_OPERATION_NAME,
            )
            .expect("headless product foundation includes XMAP21 prepare");
        schedule
            .require(
                map_viewer::XMAP21_EXECUTE_STAGE_NAME,
                map_viewer::XMAP21_EXECUTE_OPERATION_NAME,
            )
            .expect("headless product foundation includes XMAP21 execute");

        let source = crate::source_scan::headless_production();
        let start = source
            .find("fn dump_production_recording_frame(")
            .expect("production recording dump function");
        let end = source[start..]
            .find("fn reject_production_dump_debug_inputs(")
            .expect("next headless function boundary");
        let production = &source[start..start + end];
        let composition = production
            .find("ProductMapCompositionSession::start_for_headless(")
            .expect("shared XMAP22 product compositor");
        let ui_profile = production
            .find("playback_ui_profile_from_paths(")
            .expect("presentation profile load");
        let execute = production
            .find("composition.execute(")
            .expect("shared XMAP22 execution");
        assert!(production[..composition].contains("foundation_map_dimensions("));
        assert!(
            composition < ui_profile,
            "foundation admission must precede UI profile loading"
        );
        assert!(
            ui_profile < execute,
            "UI profile must be available before composition execution"
        );
        assert!(production[composition..].contains("write_bmp("));
        assert!(production[composition..].contains("xmap22_receipt_path(out)"));
        assert_eq!(
            xmap22_receipt_path(Path::new("frame.bmp")),
            PathBuf::from("frame.bmp.xmap22.json")
        );
        for forbidden in [
            "production_render",
            "selected_present",
            "ProductObserverUiRuntime",
        ] {
            assert!(
                !production[composition..].contains(forbidden),
                "headless product composition consumed {forbidden}"
            );
        }
    }
}
