//! A replay and the verified local game content used to draw its current hour.

use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use installed_content::{
    ResolveOptions,
    map::{self, MapData},
};
use replay::{
    container::{Replay, SeekResult},
    frame::DecodedFrame,
    identity::Verification,
};

use crate::{
    map::textures::MapAssets,
    // borders
    map::{
        border_pass::{BorderAssets, StraitVertex, Topology, strait_quads},
        borders::{self, BorderMeshes, HeightField, ProvinceMap},
    },
    playback::Playback,
};

pub struct Document {
    pub playback: Playback,
    pub map: MapData,
    /// The game images and textures the map passes draw from.
    pub assets: MapAssets,
    /// Persistent content verification and presentation limitations.
    pub warnings: Vec<String>,
    pub source_path: PathBuf,
    // borders
    /// `map/terrain/border_*_0.dds` and `strait.dds`.
    pub border_assets: BorderAssets,
    /// The traced border strips. Every land edge sits in the `Province` bucket until the hour's
    /// political regions re-bucket it; see `map::border_pass`.
    pub border_meshes: BorderMeshes,
    /// The strait ribbons from `map/adjacencies.csv`.
    pub straits: Vec<StraitVertex>,
    /// Wall time of the border trace, for reporting.
    pub border_trace: Duration,
}

impl Document {
    pub fn load(path: &Path, options: &ResolveOptions, allow_unverified: bool) -> io::Result<Self> {
        let replay = Replay::open(fs::read(path)?)?;
        let mut warnings = Vec::new();
        check_verification(
            "replay header",
            &replay.identity.verification,
            allow_unverified,
            &mut warnings,
        )?;
        for session in &replay.sessions {
            check_verification(
                &format!("session {}", session.id),
                &session.identity.verification,
                allow_unverified,
                &mut warnings,
            )?;
        }
        // Replay::open requires every session to have the same content identity.
        // The recorded layer ids and hashes, rather than launcher state, select it.
        let content = installed_content::resolve(&replay.identity, options)?;
        let map = map::load(&content.layers)?;
        let (terrain, default_map) = installed_content::terrain::read(&content.layers)?;
        let assets = MapAssets::load(&content.layers, &terrain, &default_map, &mut warnings)?;
        warnings.push(
            "Country colors come from the game's static content; runtime and cosmetic color overrides were not captured.".into(),
        );
        // borders
        let border_assets = BorderAssets::load(&content.layers)?;
        let states = installed_content::states::read(&content.layers)?;
        let regions = installed_content::regions::read(&content.layers)?;
        let adjacencies = installed_content::adjacencies::read(&content.layers)?;
        let topology = Topology::new(&map, &states, &regions, &adjacencies)?;
        let heights = HeightField {
            width: assets.heightmap.width,
            height: assets.heightmap.height,
            values: assets.heightmap_texels(),
        };
        let straits = strait_quads(&adjacencies, &heights, border_assets.strait_aspect())?;
        let started = Instant::now();
        let border_meshes = borders::build(
            &ProvinceMap {
                width: map.width,
                height: map.height,
                ids: &map.province_ids,
            },
            &|a, b| topology.trace_class(a, b),
            &heights,
        )?;
        let border_trace = started.elapsed();
        if !border_meshes.one_pixel_provinces.is_empty() {
            warnings.push(format!(
                "{} provinces cover a single pixel of map/provinces.bmp; the game reports these too and no border is traced through them.",
                border_meshes.one_pixel_provinces.len()
            ));
        }
        Ok(Self {
            playback: Playback::new(replay)?,
            map,
            assets,
            warnings,
            source_path: path.to_path_buf(),
            border_assets,
            border_meshes,
            straits,
            border_trace,
        })
    }

    // political fill
    /// The decoded frame of the hour on screen, or a named error for a recorded gap.
    pub fn frame(&self) -> io::Result<&DecodedFrame> {
        match self.playback.current() {
            SeekResult::Frame(frame) => Ok(frame),
            SeekResult::Gap {
                branch,
                hour,
                reason,
            } => Err(invalid(format!(
                "branch {branch}, hour {hour} is a gap: {reason}"
            ))),
        }
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn check_verification(
    context: &str,
    state: &Verification,
    allow_unverified: bool,
    warnings: &mut Vec<String>,
) -> io::Result<()> {
    let reason = match state {
        Verification::Verified { .. } => return Ok(()),
        Verification::Unverified { reason, .. } => reason.as_str(),
        Verification::Unchecked => "the collector did not verify the loaded content roster",
    };
    if !allow_unverified {
        return Err(invalid(format!(
            "{context} has unverified content: {reason}; --allow-unverified-content permits viewing with a persistent warning"
        )));
    }
    warnings.push(format!("UNVERIFIED CONTENT — {context}: {reason}"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::identity::Roster;

    #[test]
    fn every_unverified_identity_requires_override_and_keeps_its_reason() {
        let mut warnings = Vec::new();
        let state = Verification::Unverified {
            reason: "DLC roster differs".into(),
            launcher: Roster::default(),
            memory: Roster::default(),
        };
        assert!(check_verification("session 2", &state, false, &mut warnings).is_err());
        check_verification("session 2", &state, true, &mut warnings).unwrap();
        assert!(warnings[0].contains("session 2: DLC roster differs"));
        assert!(
            check_verification("session 3", &Verification::Unchecked, false, &mut warnings)
                .is_err()
        );
        check_verification("session 3", &Verification::Unchecked, true, &mut warnings).unwrap();
        assert_eq!(warnings.len(), 2);
    }
}
