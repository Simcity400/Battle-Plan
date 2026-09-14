//! A replay and the verified local game content used to draw its current hour.

use std::{
    collections::{BTreeMap, BTreeSet},
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
    rows::tables as row_tables,
};

use crate::{
    map::textures::MapAssets,
    // borders
    map::{
        border_pass::{BorderAssets, StraitVertex, Topology, strait_quads},
        borders::{self, BorderMeshes, HeightField, ProvinceMap},
        demilitarized::{self, Zone},
        political::Regions,
    },
    playback::Playback,
};
// rivers
use crate::map::rivers::{self, RiverAssets, RiverImage, RiverMesh};
// trees
use crate::map::mapframe::{self, FrameAssets, FrameGeometry};
use crate::map::trees::{self, TreeAssets, TreeField, TreeGeometry, TreeImage};
// close zoom
use crate::map::map_buildings::MapBuildingAssets;
use crate::map::railways::{self as rail_layer, Bridges, RailAssets, RailMesh};
use crate::map::sprawl::{self, Sprawl};
use crate::map::unit_models::{ModelReport, UnitModelAssets};
// names
use crate::map::{
    camera::Camera,
    labels::{self, LabelContent, LabelReport},
    name_pass::{self, NameContent, PlacementInputs, Placements},
    sprites::{Batch, MapIcons},
};
// counters
use crate::map::counters::{self, CounterContent, CounterInputs, CounterReport};
// weather
use crate::map::perspective;
use crate::map::tables::{self, Limitations};
use crate::map::weather::{self, SnowMud};

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
    // borders: the demilitarised-zone bucket bank
    /// State ids to treat as demilitarised **in addition to** the recorded ones, so an hour whose
    /// `StateFlags` are short can still be scored against a reference pair. Empty unless
    /// `--dump --demilitarized` sets it; see [`Document::demilitarized_zone`] and
    /// `map::demilitarized`.
    pub demilitarized_override: BTreeSet<u32>,
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
    // the map's table frame
    /// The three band meshes of `map/ambient_object.txt` and their textures.
    pub frame_assets: FrameAssets,
    /// The band geometry, already in world space.
    pub frame_geometry: FrameGeometry,
    // trees
    /// The canopy meshes, their textures and the `tree_*` season blocks.
    pub tree_assets: TreeAssets,
    /// The canopy geometry, flattened over every LOD of every tree mesh.
    pub tree_geometry: TreeGeometry,
    /// Every canopy instance on the map, bucketed by tile.
    pub tree_field: TreeField,
    /// Wall time of the instance generation, for reporting.
    pub tree_build: Duration,
    // close zoom
    /// The city sprawl: its mesh library and its placed houses.
    pub sprawl: Sprawl,
    /// How long generating the sprawl took.
    pub sprawl_build: Duration,
    /// The map-building slot table, types and meshes; the placed buildings are
    /// per hour and come from [`Document::map_building_field`].
    pub map_buildings: MapBuildingAssets,
    /// `gfx/models/supply/railroad.dds`.
    pub rail_assets: RailAssets,
    /// The traced rail network of `map/railways.txt`.
    pub rail_mesh: RailMesh,
    /// How long tracing the rail network took.
    pub rail_build: Duration,
    /// The railway bridges, which are meshes rather than strip.
    pub bridges: Bridges,
    /// The land division entities, the sub-unit keys and the graphical
    /// cultures; the placed figures are per hour and come from
    /// [`Document::unit_model_field`].
    pub unit_models: UnitModelAssets,
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
            "Unit counters are drawn for one country: a stack it cannot see carries the game's own question-mark icon, a question mark in place of its count and no bars. Which stacks those are is decided from the recorded faction, war, subject and UnitIntel tables; the engine also reveals a stack standing in a province the viewing country has fully uncovered, and no table records that, so a stack the game drew in full may carry a question mark here. --perspective omniscient draws every stack in full instead.".into(),
        );
        warnings.push(
            "A stack of several divisions takes its unit-type icon from its oldest division's template. Which member of a stack the engine's counter holds is decided by the icon object's own construction, which this pass does not reproduce, so a stack of mixed templates may draw a different member's glyph than the game does.".into(),
        );
        warnings.push(
            "Unit counters are gathered into the engine's camera-height groups. The engine's own grouping relation is the co-belligerent byte it sets for each country on its own side of a war it is in, and the recorded WarRelations table is pairwise, so two countries count as co-belligerents here when they share an ongoing enemy and are not at war with each other; a war whose two sides share no third country would keep counters apart that the game gathers.".into(),
        );
        if !labels.centroid_fallbacks.is_empty() {
            warnings.push(format!(
                "{} provinces have no victory-point unit stack and their label is anchored at the province centroid instead.",
                labels.centroid_fallbacks.len()
            ));
        }
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
            &|province| topology.is_water(province),
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
        // the map's table frame
        let frame_assets = FrameAssets::load(&content.layers, &mut warnings)?;
        let frame_geometry = mapframe::geometry(&frame_assets);
        // trees
        let started = Instant::now();
        let tree_field = trees::generate(
            &TreeImage {
                width: assets.trees.width,
                height: assets.trees.height,
                indices: &assets.trees.indices,
            },
            &heights,
        )?;
        let tree_build = started.elapsed();
        let tree_assets = TreeAssets::load(
            &content.layers,
            &tree_field.palettes,
            // `TreeMaskTexture` is generated from the river bitmap; see
            // `map::trees::tree_mask`.
            &assets.rivers,
            &mut warnings,
        )?;
        let tree_geometry = trees::geometry(&tree_assets);
        warnings.push(
            "Tree meshes and their textures are read from gfx/entities and gfx/models, which are outside the content coverage policy and are not hash-verified.".into(),
        );
        // close zoom
        let started = Instant::now();
        let (cities, city_styles) = installed_content::cities::read(&content.layers)?;
        let sprawl_plan = sprawl::generate(&cities, &city_styles, &assets.spawn_city, &heights)?;
        let sprawl = Sprawl::load(&content.layers, &sprawl_plan, &mut warnings)?;
        let sprawl_build = started.elapsed();
        let started = Instant::now();
        let railways = installed_content::railways::read(&content.layers)?;
        let anchors =
            rail_layer::province_anchors(&installed_content::positions::read_unit_stacks(&content.layers)?);
        let water = rail_layer::WaterMask {
            heights: &heights,
            rivers: &assets.rivers,
        };
        let rail_mesh = rail_layer::build(&railways, &anchors, &heights, &water);
        let rail_build = started.elapsed();
        let rail_assets = RailAssets::load(&content.layers)?;
        let bridges = Bridges::load(&content.layers, &rail_mesh, &mut warnings)?;
        let unit_models = UnitModelAssets::load(&content.layers, &mut warnings)?;
        warnings.push(
            "A division's model follows the engine's own ladder from its recorded template: the priority sub-unit's key, then its common/units sprite, each tried with the owner's tag, then its graphical culture, then bare. Four levels of that ladder need things no table records and are skipped: the equipment model number (so GER_infantry_2_entity and the other numbered models are never reached), the division designer's explicit model, the cosmetic-tag prefix and the _desert and _snow terrain variants. The figures stand in their bind pose: no animation is sampled and no skinning matrix is bound, so a rifleman stands in the T-pose his mesh is authored in rather than in an idle frame. What the entity attaches at a skeleton node is drawn, at the joint the animation its default state plays rests that joint at, which is what puts a truck and a rifleman on a motorized rig and a gun and a crew on an artillery one; the rifle is therefore in the hand the idle pose holds it in rather than in the T-pose hand beneath it. A standing figure's facing is a hash, because nothing records the heading the engine last turned it to. Ships are not drawn, because a ship's entity follows its hull and refit and the recorded Ships table names neither."
                .into(),
        );
        if rail_mesh.unanchored > 0 {
            warnings.push(format!(
                "{} of {} map/railways.txt segments name a province with no map/unitstacks.txt anchor and are not drawn.",
                rail_mesh.unanchored,
                railways.len()
            ));
        }
        warnings.push(
            "Railways are the authored 1936 network of map/railways.txt: no recorded table carries the network as it stands at an hour, so rails built during the campaign are missing and rails lost are still drawn. Trains are dynamic and unrecorded and are not drawn at all."
                .into(),
        );
        let map_buildings = MapBuildingAssets::load(
            &content.layers,
            &crate::map::map_buildings::ProvinceMap {
                width: map.width,
                height: map.height,
                ids: &map.province_ids,
            },
            &mut warnings,
        )?;
        warnings.push(
            "Map buildings are drawn healthy: the replay records healthy_level and partial_health but not the scale the engine's destroyed and damaged thresholds are measured on, and the factory smoke is a particle system that is not ported."
                .into(),
        );
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
            // borders: the zone is the recording's; `--dump --demilitarized` only adds to it.
            demilitarized_override: BTreeSet::new(),
            // names
            icons,
            names,
            labels,
            // rivers
            river_assets,
            river_mesh,
            river_trace,
            river_build,
            // the map's table frame
            frame_assets,
            frame_geometry,
            // counters
            counters,
            // trees
            tree_assets,
            tree_geometry,
            tree_field,
            tree_build,
            // close zoom
            sprawl,
            sprawl_build,
            map_buildings,
            rail_assets,
            rail_mesh,
            rail_build,
            bridges,
            unit_models,
        })
    }

    // close zoom
    /// This hour's placed map buildings.
    pub fn map_building_field(
        &self,
    ) -> io::Result<(
        crate::map::models::ModelField,
        crate::map::map_buildings::MapBuildingSummary,
    )> {
        self.map_buildings.field(self.frame()?)
    }

    // close zoom
    /// This hour's placed unit models.
    ///
    /// The perspective is the counter layer's: the engine hides a division's
    /// 150 rather than the counter's 250.
    pub fn unit_model_field(
        &self,
        perspective: &perspective::Choice,
    ) -> io::Result<(crate::map::models::ModelField, ModelReport)> {
        let content = &self.counters;
        let definitions = &self.map.definitions;
        self.unit_models.field(
            self.frame()?,
            &self.vision(perspective)?,
            &|province, slot| content.anchor(province, slot),
            &|province| {
                !matches!(
                    definitions.get(&province).map(|it| it.kind),
                    Some(map::ProvinceKind::Sea) | Some(map::ProvinceKind::Lake)
                )
            },
        )
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

    /// What the hour on screen could not draw, one line per degraded table.
    ///
    /// The map passes degrade on their own; this is the single list of what
    /// that cost, so the window's warning area and a `--dump` report the same
    /// thing. A core table's gap is not here: it is the frame error above.
    pub fn limitations(&self, perspective: &perspective::Choice) -> io::Result<Limitations> {
        let frame = self.frame()?;
        let mut out = tables::limitations(frame);
        // counters: which country the map is drawn for. Stated on every frame,
        // degraded or not, because every counter on it depends on the answer.
        let vision = self.vision(perspective)?;
        out.note_perspective(&self.perspective_state(perspective, &vision)?);
        for table in vision.truncated() {
            out.note_field(
                &format!("{table} (perspective)"),
                "the rows this hour did not reach are read as no relation and no detection at \
                 all, so a stack among them carries the game's question mark where the game drew \
                 it in full",
            );
        }
        // names: `Countries.name` is the engine's own rendered name, so an empty one is a country
        // whose name the recording holds but the engine never built. The name layer falls back to
        // the ideology-group key for it, which a cosmetic or dynamic name would not match.
        let unnamed = name_pass::unnamed_countries(&self.map, frame)?;
        if !unnamed.is_empty() {
            out.note_field(
                "Countries.name",
                &format!(
                    "{} {} ({}{}) on the map record no name, so theirs is built from the \
                     <TAG>_<ideology> localisation key instead and will differ from the game under \
                     a cosmetic tag or a dynamic name",
                    unnamed.len(),
                    if unnamed.len() == 1 {
                        "country"
                    } else {
                        "countries"
                    },
                    unnamed
                        .iter()
                        .take(8)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", "),
                    if unnamed.len() > 8 { ", …" } else { "" },
                ),
            );
        }
        // names: bytes the port cannot lay out as text. The province keeps its static name rather
        // than being drawn with replacement marks; see `map::labels`.
        let unreadable = labels::unreadable_overrides(frame)?;
        if !unreadable.is_empty() {
            out.note_field(
                "ProvinceNames.name",
                &format!(
                    "the recorded name of {} {} ({}) is not text this port can lay out, so {} \
                     keeps the static localised name of its province",
                    unreadable.len(),
                    if unreadable.len() == 1 {
                        "province"
                    } else {
                        "provinces"
                    },
                    unreadable
                        .iter()
                        .take(8)
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(", "),
                    if unreadable.len() == 1 { "it" } else { "each" },
                ),
            );
        }
        Ok(out)
    }

    // counters: the viewing country
    /// What this hour's counters can see, for the limitation lines and the header.
    pub fn vision(&self, perspective: &perspective::Choice) -> io::Result<perspective::Vision> {
        let frame = self.frame()?;
        let links: BTreeMap<u32, u32> = tables::core::<row_tables::CountryLinks>(frame)?
            .iter()
            .map(|link| (link.id, link.canonical_slot))
            .collect();
        let tags: BTreeMap<u32, String> = tables::core::<row_tables::Countries>(frame)?
            .iter()
            .map(|country| {
                (
                    country.canonical_slot,
                    String::from_utf8_lossy(&country.tag).into_owned(),
                )
            })
            .collect();
        perspective::Vision::read(frame, &links, &tags, perspective)
    }

    /// The perspective in the shape `map::tables` words it in.
    fn perspective_state(
        &self,
        perspective: &perspective::Choice,
        vision: &perspective::Vision,
    ) -> io::Result<tables::Perspective> {
        let Some(ours) = vision.ours() else {
            let reason = match vision.fallback() {
                Some(perspective::Fallback::Requested) | None => {
                    "the render was asked for the omniscient view".to_owned()
                }
                Some(perspective::Fallback::NoPlayer) => {
                    "this hour records no human player to draw it for".to_owned()
                }
                Some(perspective::Fallback::UnknownTag(tag)) => {
                    format!("this hour records no country {tag} to draw it for")
                }
                Some(perspective::Fallback::MissingTable(table)) => format!(
                    "table {table} is unavailable for this hour and an unavailable table is not \
                     evidence that it is empty"
                ),
            };
            return Ok(tables::Perspective::Omniscient { reason });
        };
        let tag = tables::core::<row_tables::Countries>(self.frame()?)?
            .iter()
            .find(|country| country.canonical_slot == ours)
            .map(|country| String::from_utf8_lossy(&country.tag).into_owned())
            .unwrap_or_else(|| format!("country slot {ours}"));
        Ok(tables::Perspective::Country {
            tag,
            requested: matches!(perspective, perspective::Choice::Tag(_)),
        })
    }

    // borders: the demilitarised-zone bucket bank
    /// This hour's demilitarised zone, for `map::demilitarized` and the border pass.
    ///
    /// The recorded set is the hour's own `StateFlags` joined to its `States`
    /// ([`crate::map::demilitarized::recorded_states`]). The `--dump --demilitarized` override
    /// only ever *adds* to it, so an hour that records the flag is drawn from the recording
    /// whatever the command line says.
    pub fn demilitarized_zone(&self, regions: &Regions) -> io::Result<Zone> {
        let mut states = demilitarized::recorded_states(self.frame()?)?;
        states.extend(self.demilitarized_override.iter().copied());
        Ok(Zone::new(&self.map, regions, &states))
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
        perspective: &perspective::Choice,
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
                perspective,
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

    // trees
    /// `vSeasonColumn` and `vSeasonLerp` for the hour on screen.
    pub fn tree_season(&self) -> trees::Season {
        self.tree_assets.seasons.at_hour(self.playback.hour())
    }

    // weather
    /// Builds this hour's `SnowMudData` grid from its weather tables.
    ///
    /// The `perm_snow` mask and the province image are static content, so the
    /// hour is the only input that moves; the window goes through
    /// `map::weather::WeatherCache` instead, which skips an hour that changed
    /// no weather row.
    pub fn snow_mud(&self) -> io::Result<SnowMud> {
        weather::build(&self.map, &self.assets.perm_snow, self.frame()?)
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
