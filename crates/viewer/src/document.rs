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
// rivers
use crate::map::rivers::{self, RiverAssets, RiverImage, RiverMesh};
// names
use crate::map::{
    camera::Camera,
    labels::{self, LabelContent, LabelReport},
    name_pass::{self, NameContent, PlacementInputs, Placements},
    sprites::{Batch, MapIcons},
};
// counters
use crate::map::counters::{self, CounterContent, CounterInputs, CounterReport};

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
    // names
    /// Fonts and map-icon sprites, uploaded once by the scene.
    pub icons: MapIcons,
    /// Province adjacency, province bounds and the localisation.
    pub names: NameContent,
    /// The label window layout and the unit-stack anchors.
    pub labels: LabelContent,
    // rivers
    /// `map/terrain/RiverSurface_*`.
    pub river_assets: RiverAssets,
    /// The ribbons traced from `map/rivers.bmp`.
    pub river_mesh: RiverMesh,
    /// What the trace found, including the rivers the engine reports and drops.
    pub river_trace: rivers::Trace,
    /// Wall time of the river trace and widening, for reporting.
    pub river_build: Duration,
    // counters
    /// The counter, base and combat anchors from `map/unitstacks.txt` and
    /// `map/buildings.txt`.
    pub counters: CounterContent,
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
        // names
        let icons = MapIcons::load(&content.layers)?;
        let names = NameContent::load(&content.layers, &map)?;
        let labels = LabelContent::load(&content.layers, &map, assets.heightmap_texels())?;
        // counters
        let counters = CounterContent::load(&content.layers, &map)?;
        warnings.extend(icons.counters.warnings.iter().cloned());
        if icons.counters.flags.is_empty() {
            warnings.push(
                "No flag of the small set resolved, so unit counters carry no country flag.".into(),
            );
        }
        if !counters.missing_standstill.is_empty() {
            warnings.push(format!(
                "{} provinces have no Standstill unit stack in map/unitstacks.txt; a unit counter in one of them is not drawn.",
                counters.missing_standstill.len()
            ));
        }
        warnings.push(
            "Unit counters carry no division-type icon: no division-template table is recorded. The organisation and strength bars are filled by each stack's mean over its own maximum, because the template maxima the game divides by are not recorded either.".into(),
        );
        warnings.push(
            "Map icons are placed independently of each other; the game's icon de-overlap and camera-height grouping are not reproduced, so counters of nearby provinces overlap here.".into(),
        );
        if !labels.centroid_fallbacks.is_empty() {
            warnings.push(format!(
                "{} provinces have no victory-point unit stack and their label is anchored at the province centroid instead.",
                labels.centroid_fallbacks.len()
            ));
        }
        warnings.push(
            "Country names come from the recorded ruling party's ideology group; cosmetic tags and dynamic country names are not recorded, so a country under one differs from the game.".into(),
        );
        warnings.push(
            "Victory-point labels come from the static localisation; runtime renames (set_province_name, which turns Danzig into Gdańsk) are not recorded.".into(),
        );
        warnings.push(
            "Fonts are read from gfx/fonts, which is outside the content coverage policy and is not hash-verified.".into(),
        );
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
        // rivers
        let river_assets = RiverAssets::load(&content.layers)?;
        let started = Instant::now();
        let (river_mesh, river_trace) = rivers::build(
            &RiverImage {
                width: assets.rivers.width,
                height: assets.rivers.height,
                indices: &assets.rivers.indices,
            },
            &heights,
        )?;
        let river_build = started.elapsed();
        if river_trace.unreachable > 0 {
            warnings.push(format!(
                "{} river pixels of map/rivers.bmp are not reachable from any source or junction marker and are not drawn; the game does not draw them either.",
                river_trace.unreachable
            ));
        }
        if !river_trace.too_small.is_empty() {
            warnings.push(format!(
                "{} rivers of map/rivers.bmp are shorter than {} pixels; the game reports these as \"Too small river found at\" and draws none of them.",
                river_trace.too_small.len(),
                rivers::MIN_RIVER_POINTS
            ));
        }
        Ok(Self {
            playback: Playback::new(replay)?,
            map,
            assets,
            warnings,
            source_path: path.to_path_buf(),
            // borders
            border_assets,
            border_meshes,
            straits,
            border_trace,
            // names
            icons,
            names,
            labels,
            // rivers
            river_assets,
            river_mesh,
            river_trace,
            river_build,
            // counters
            counters,
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

    // names
    /// Places the country name layer, and the state layer when `states`.
    pub fn place_names(&self, states: bool) -> io::Result<Placements> {
        name_pass::place(
            &PlacementInputs {
                map: &self.map,
                heightmap: &self.assets.heightmap,
                content: &self.names,
                font: &self.icons.fonts.map,
                states,
            },
            self.frame()?,
        )
    }

    // counters
    /// Builds this frame's whole screen-space icon layer, in priority order.
    ///
    /// `00_graphics.lua:141-160` sorts map icons by `DEFAULT_PRIORITY_*`: air
    /// and naval bases 3, victory points and capitals 5, unit stacks 10, land
    /// and naval combat 20. The sprite batch draws in push order, so pushing
    /// the four groups in that order is what produces the engine's overlap.
    pub fn overlay_batch(
        &self,
        camera: Camera,
        viewport: [u32; 2],
    ) -> io::Result<(Batch, LabelReport, CounterReport)> {
        let frame = self.frame()?;
        let icons = counters::build(
            &CounterInputs {
                content: &self.counters,
                assets: &self.icons.counters,
                fonts: &self.icons.fonts,
                map: &self.map,
                camera,
                viewport,
            },
            frame,
        )?;
        let mut batch = Batch::default();
        batch.quads.extend(icons.bases.quads);
        let label_report = labels::build(
            &labels::LabelInputs {
                content: &self.labels,
                localisation: &self.names.localisation,
                font: &self.icons.fonts.label,
                sheet: &self.icons.victory_points,
                map: &self.map,
                camera,
                viewport,
            },
            frame,
            &mut batch,
        )?;
        batch.quads.extend(icons.counters.quads);
        batch.quads.extend(icons.combats.quads);
        Ok((batch, label_report, icons.report))
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
