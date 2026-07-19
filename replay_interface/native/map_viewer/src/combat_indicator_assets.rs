//! C3 files-first decode of the on-map combat indicators ("battle bubbles").
//!
//! This is the same treatment C1 gave unit counters (`unit_counter_assets.rs`):
//! decode the shipped asset/layout chain for the land/naval combat map icons
//! from interface `*.gfx` + `*.gui` files, resolve every referenced sprite to a
//! real `.dds` with a resolution gate, and expose the layout element tree via the
//! existing `.gui` parser. The **screen-space visual law** (rendered quad size,
//! world/province anchor, zoom scaling, depth ordering, frame/colour selection,
//! progress-bar fill mapping) stays CAPTURE-gated — see `COMBAT_INDICATOR_LAW.md`
//! and the CAP row in the production plan.
//!
//! No product renderer should draw combat indicators from this module yet; it is
//! the FILE half of C3. All shipped-file reads go through `GameFileResolver`
//! keyed by the recording modset (`MOD_COMPATIBILITY.md`), reusing the C1
//! parser/DDS-resolution helpers verbatim (no forked parser).

use crate::unit_counter_assets::{
    container_layout_from_text, parse_bitmap_fonts, parse_progress_bar_assets,
    sprite_inventory_from_resolver, BitmapFontBinding, CounterSpriteAsset, GuiContainerLayout,
    GuiSize, ProgressBarAsset,
};
use crate::{read_latin1, GameFileResolver, Result};
use std::collections::{BTreeMap, BTreeSet};

pub const COMBAT_INDICATOR_MAPICONS_GFX_SOURCE: &str = "interface/mapicons.gfx";
pub const COMBAT_INDICATOR_MAPICONS_GUI_SOURCE: &str = "interface/mapicons.gui";
pub const COMBAT_INDICATOR_LANDCOMBAT_GFX_SOURCE: &str = "interface/landcombat.gfx";
pub const COMBAT_INDICATOR_NAVALCOMBAT_GFX_SOURCE: &str = "interface/navalcombat.gfx";
pub const COMBAT_INDICATOR_UNITVIEW_GFX_SOURCE: &str = "interface/unitview.gfx";
pub const COMBAT_INDICATOR_NAVAL_LEADER_MODULE_GFX_SOURCE: &str =
    "interface/building_roster/naval_leader_module.gfx";
pub const COMBAT_INDICATOR_CORE_GFX_SOURCE: &str = "interface/core.gfx";

/// Land combat map-icon sprites declared in `mapicons.gfx`.
const LAND_COMBAT_MAPICONS_GFX_SPRITES: &[&str] = &[
    "GFX_mapicon_land_combat",
    "GFX_mapicon_land_combat_frame",
    "GFX_mapicon_land_combat_bombardment_overlay",
    "GFX_mapicon_land_combat_urban",
    "GFX_battle_indicator_direction",
    "GFX_landbattle_strip",
];

/// Land combat sprite declared in `landcombat.gfx` and referenced by the on-map
/// land combat icon (`air_superiority_icon` control).
const LAND_COMBAT_LANDCOMBAT_GFX_SPRITES: &[&str] = &["GFX_battle_modifier_strip"];

/// Naval combat map-icon sprites declared in `mapicons.gfx`.
const NAVAL_COMBAT_MAPICONS_GFX_SPRITES: &[&str] = &[
    "GFX_mapicon_naval_combat",
    "GFX_naval_combat_result",
    "GFX_naval_combat_result_portstrikes",
    "GFX_naval_combat_result_navalstrikes",
    "GFX_naval_combat_result_convoys",
    "GFX_mapicon_naval_battle_result",
    "GFX_mapicon_naval_battle_result_text",
    "GFX_naval_combat_icon",
    "GFX_naval_combat_results_icon",
];

/// Naval combat frame declared in `building_roster/naval_leader_module.gfx` and
/// referenced by the on-map naval combat icon (`frame_btn` control).
const NAVAL_COMBAT_NAVAL_LEADER_GFX_SPRITES: &[&str] = &["GFX_naval_hq_combat_frame"];

/// Combat progress bars: naval attacker bar (`navalcombat.gfx`) + the small land
/// unit combat progress bar (`unitview.gfx`) reused by the naval on-map icon.
const NAVALCOMBAT_PROGRESS_BAR_NAMES: &[&str] = &["GFX_navalcombat_bar_progress_attacker_154"];
const UNITVIEW_PROGRESS_BAR_NAMES: &[&str] = &["GFX_land_unit_combat_progressbar"];

/// Font used by both the land combat `combat_progress` textbox and the naval
/// fleet-status combat count textboxes.
const COMBAT_INDICATOR_FONT_NAMES: &[&str] = &["hoi_16mbs"];

/// On-map combat containers in `mapicons.gui`.
const COMBAT_INDICATOR_GUI_CONTAINERS: &[&str] = &[
    "land_combat_mapicon",
    "naval_combat_mapicon",
    "naval_combat_results_mapicon",
    "ongoing_combat_container",
    "combat_results_container",
];

#[derive(Clone, Debug, PartialEq)]
pub struct CombatIndicatorAssetManifest {
    /// Land combat bubble sprites (frame, backing, bombardment overlay, urban
    /// variant, direction indicator, animated battle strip, air-superiority
    /// modifier strip). Resolved DDS with frame rects.
    pub land_combat_sprites: BTreeMap<String, CounterSpriteAsset>,
    /// Naval combat bubble sprites (icon, result variants, HQ frame, small
    /// fleet-status combat/result icons). Resolved DDS with frame rects.
    pub naval_combat_sprites: BTreeMap<String, CounterSpriteAsset>,
    /// Shipped combat sprite references that failed to resolve through the
    /// active resolver (reported, never silently remapped). Expected empty on a
    /// clean vanilla install.
    pub unresolved_sprites: Vec<crate::unit_counter_assets::UnresolvedSpriteAsset>,
    /// Combat progress bars (naval attacker bar + land unit combat bar).
    pub progress_bars: BTreeMap<String, ProgressBarAsset>,
    /// On-map combat container layouts + their named child controls.
    pub gui_layouts: CombatIndicatorGuiLayouts,
    /// Font bindings used by the combat textboxes.
    pub fonts: Vec<BitmapFontBinding>,
}

impl CombatIndicatorAssetManifest {
    pub fn land_combat_sprite(&self, name: &str) -> Option<&CounterSpriteAsset> {
        self.land_combat_sprites.get(name)
    }

    pub fn naval_combat_sprite(&self, name: &str) -> Option<&CounterSpriteAsset> {
        self.naval_combat_sprites.get(name)
    }

    /// Total resolved combat sprite definitions across land + naval groups.
    pub fn resolved_sprite_count(&self) -> usize {
        self.land_combat_sprites.len() + self.naval_combat_sprites.len()
    }

    /// Distinct resolved combat sprite texture files.
    pub fn unique_sprite_texture_count(&self) -> usize {
        self.land_combat_sprites
            .values()
            .chain(self.naval_combat_sprites.values())
            .map(|asset| asset.texture_file.as_str())
            .collect::<BTreeSet<_>>()
            .len()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CombatIndicatorGuiLayouts {
    pub land_combat_mapicon: GuiContainerLayout,
    pub naval_combat_mapicon: GuiContainerLayout,
    pub naval_combat_results_mapicon: GuiContainerLayout,
    pub ongoing_combat_container: GuiContainerLayout,
    pub combat_results_container: GuiContainerLayout,
}

impl CombatIndicatorGuiLayouts {
    pub fn container(&self, name: &str) -> Option<&GuiContainerLayout> {
        match name {
            "land_combat_mapicon" => Some(&self.land_combat_mapicon),
            "naval_combat_mapicon" => Some(&self.naval_combat_mapicon),
            "naval_combat_results_mapicon" => Some(&self.naval_combat_results_mapicon),
            "ongoing_combat_container" => Some(&self.ongoing_combat_container),
            "combat_results_container" => Some(&self.combat_results_container),
            _ => None,
        }
    }
}

/// Which on-map combat container a placement refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CombatIndicatorKind {
    Land,
    Naval,
    NavalResults,
}

impl CombatIndicatorKind {
    pub fn container_name(self) -> &'static str {
        match self {
            CombatIndicatorKind::Land => "land_combat_mapicon",
            CombatIndicatorKind::Naval => "naval_combat_mapicon",
            CombatIndicatorKind::NavalResults => "naval_combat_results_mapicon",
        }
    }
}

/// FILE-derived placement stub keyed by province.
///
/// The container declared size and the file `position` are FILE facts. The
/// world/province anchor point, rendered quad size, zoom-scaling, depth ordering
/// against terrain, and the mapping of combat state onto frame/colour/progress
/// are all CAPTURE-gated (see `COMBAT_INDICATOR_LAW.md`). Those fields stay
/// `None`/unset until CAP11 evidence lands; this stub only exposes the FILE part.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CombatIndicatorPlacement {
    pub province_id: u32,
    pub kind: CombatIndicatorKind,
    /// Declared container size from the `.gui` (`size = { width height }`).
    /// FILE fact; NOT the rendered screen quad (capture-gated).
    pub declared_size: Option<GuiSize>,
    /// Screen-space rendered quad size (CAPTURE-gated; always `None` here).
    pub screen_quad_size: Option<GuiSize>,
    /// Whether the world/province anchor law is known (CAPTURE-gated; always
    /// `false` here — the file gives no province anchor).
    pub anchor_law_known: bool,
}

impl CombatIndicatorAssetManifest {
    /// Build a FILE-only placement stub for a province + combat kind. The
    /// screen-space fields are intentionally left unresolved (capture-gated).
    pub fn placement_stub(
        &self,
        province_id: u32,
        kind: CombatIndicatorKind,
    ) -> CombatIndicatorPlacement {
        let declared_size = self
            .gui_layouts
            .container(kind.container_name())
            .and_then(|container| container.size);
        CombatIndicatorPlacement {
            province_id,
            kind,
            declared_size,
            screen_quad_size: None,
            anchor_law_known: false,
        }
    }
}

pub fn combat_indicator_assets_from_resolver(
    resolver: &GameFileResolver,
) -> Result<CombatIndicatorAssetManifest> {
    let (land_sprites, mut unresolved) = land_combat_indicator_sprite_inventory(resolver)?;
    let (naval_sprites, naval_unresolved) = naval_combat_indicator_sprite_inventory(resolver)?;
    unresolved.extend(naval_unresolved);
    Ok(CombatIndicatorAssetManifest {
        land_combat_sprites: land_sprites,
        naval_combat_sprites: naval_sprites,
        unresolved_sprites: unresolved,
        progress_bars: combat_indicator_progress_bars_from_resolver(resolver)?,
        gui_layouts: combat_indicator_gui_layouts_from_resolver(resolver)?,
        fonts: combat_indicator_fonts_from_resolver(resolver)?,
    })
}

pub fn land_combat_indicator_sprites_from_resolver(
    resolver: &GameFileResolver,
) -> Result<BTreeMap<String, CounterSpriteAsset>> {
    Ok(land_combat_indicator_sprite_inventory(resolver)?.0)
}

pub fn naval_combat_indicator_sprites_from_resolver(
    resolver: &GameFileResolver,
) -> Result<BTreeMap<String, CounterSpriteAsset>> {
    Ok(naval_combat_indicator_sprite_inventory(resolver)?.0)
}

type SpriteInventory = (
    BTreeMap<String, CounterSpriteAsset>,
    Vec<crate::unit_counter_assets::UnresolvedSpriteAsset>,
);

fn land_combat_indicator_sprite_inventory(resolver: &GameFileResolver) -> Result<SpriteInventory> {
    let mut assets = BTreeMap::new();
    let mut unresolved = Vec::new();
    collect_named_sprites(
        resolver,
        COMBAT_INDICATOR_MAPICONS_GFX_SOURCE,
        LAND_COMBAT_MAPICONS_GFX_SPRITES,
        &mut assets,
        &mut unresolved,
    )?;
    collect_named_sprites(
        resolver,
        COMBAT_INDICATOR_LANDCOMBAT_GFX_SOURCE,
        LAND_COMBAT_LANDCOMBAT_GFX_SPRITES,
        &mut assets,
        &mut unresolved,
    )?;
    Ok((assets, unresolved))
}

fn naval_combat_indicator_sprite_inventory(resolver: &GameFileResolver) -> Result<SpriteInventory> {
    let mut assets = BTreeMap::new();
    let mut unresolved = Vec::new();
    collect_named_sprites(
        resolver,
        COMBAT_INDICATOR_MAPICONS_GFX_SOURCE,
        NAVAL_COMBAT_MAPICONS_GFX_SPRITES,
        &mut assets,
        &mut unresolved,
    )?;
    collect_named_sprites(
        resolver,
        COMBAT_INDICATOR_NAVAL_LEADER_MODULE_GFX_SOURCE,
        NAVAL_COMBAT_NAVAL_LEADER_GFX_SPRITES,
        &mut assets,
        &mut unresolved,
    )?;
    Ok((assets, unresolved))
}

fn collect_named_sprites(
    resolver: &GameFileResolver,
    source_file: &str,
    names: &[&str],
    assets: &mut BTreeMap<String, CounterSpriteAsset>,
    unresolved: &mut Vec<crate::unit_counter_assets::UnresolvedSpriteAsset>,
) -> Result<()> {
    let wanted = names.iter().copied().collect::<BTreeSet<_>>();
    let inventory = sprite_inventory_from_resolver(resolver, source_file, |def| {
        wanted.contains(def.name.as_str()) && def.texture_file.is_some()
    })?;
    for asset in inventory.resolved {
        assets.insert(asset.name.clone(), asset);
    }
    unresolved.extend(inventory.unresolved);
    Ok(())
}

pub fn combat_indicator_progress_bars_from_resolver(
    resolver: &GameFileResolver,
) -> Result<BTreeMap<String, ProgressBarAsset>> {
    let mut bars = BTreeMap::new();
    collect_named_progress_bars(
        resolver,
        COMBAT_INDICATOR_NAVALCOMBAT_GFX_SOURCE,
        NAVALCOMBAT_PROGRESS_BAR_NAMES,
        &mut bars,
    )?;
    collect_named_progress_bars(
        resolver,
        COMBAT_INDICATOR_UNITVIEW_GFX_SOURCE,
        UNITVIEW_PROGRESS_BAR_NAMES,
        &mut bars,
    )?;
    Ok(bars)
}

fn collect_named_progress_bars(
    resolver: &GameFileResolver,
    source_file: &str,
    names: &[&str],
    bars: &mut BTreeMap<String, ProgressBarAsset>,
) -> Result<()> {
    let path = resolver.resolve_required(source_file, source_file)?;
    let text = read_latin1(&path)?;
    let wanted = names.iter().copied().collect::<BTreeSet<_>>();
    for bar in parse_progress_bar_assets(&text, source_file)
        .into_iter()
        .filter(|bar| wanted.contains(bar.name.as_str()))
    {
        bars.insert(bar.name.clone(), bar);
    }
    Ok(())
}

pub fn combat_indicator_gui_layouts_from_resolver(
    resolver: &GameFileResolver,
) -> Result<CombatIndicatorGuiLayouts> {
    let path = resolver.resolve_required(COMBAT_INDICATOR_MAPICONS_GUI_SOURCE, "map icons gui")?;
    let text = read_latin1(&path)?;
    let container = |name: &str| -> Result<GuiContainerLayout> {
        container_layout_from_text(&text, name, COMBAT_INDICATOR_MAPICONS_GUI_SOURCE)
    };
    Ok(CombatIndicatorGuiLayouts {
        land_combat_mapicon: container("land_combat_mapicon")?,
        naval_combat_mapicon: container("naval_combat_mapicon")?,
        naval_combat_results_mapicon: container("naval_combat_results_mapicon")?,
        ongoing_combat_container: container("ongoing_combat_container")?,
        combat_results_container: container("combat_results_container")?,
    })
}

pub fn combat_indicator_fonts_from_resolver(
    resolver: &GameFileResolver,
) -> Result<Vec<BitmapFontBinding>> {
    let path = resolver.resolve_required(COMBAT_INDICATOR_CORE_GFX_SOURCE, "core gfx")?;
    let text = read_latin1(&path)?;
    let wanted = COMBAT_INDICATOR_FONT_NAMES
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    Ok(parse_bitmap_fonts(&text, COMBAT_INDICATOR_CORE_GFX_SOURCE)
        .into_iter()
        .filter(|font| wanted.contains(font.name.as_str()))
        .collect())
}

/// Number of GUI containers this module gates (for tests / callers).
pub fn combat_indicator_gui_container_count() -> usize {
    COMBAT_INDICATOR_GUI_CONTAINERS.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unit_counter_assets::{GuiPoint, PixelRect};
    use std::fs;
    use std::path::{Path, PathBuf};

    const VANILLA_GAME_DIR: &str = r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV";

    fn vanilla_resolver() -> Option<GameFileResolver> {
        let root = Path::new(VANILLA_GAME_DIR);
        if !root.is_dir() {
            eprintln!("skipping real-install combat indicator test; missing {VANILLA_GAME_DIR}");
            return None;
        }
        Some(GameFileResolver::vanilla(root).expect("vanilla resolver"))
    }

    fn write_fixture(root: &Path, rel: &str, bytes: &[u8]) {
        let path = slash_join(root, rel);
        fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture parent");
        fs::write(path, bytes).expect("write fixture");
    }

    fn slash_join(root: &Path, rel: &str) -> PathBuf {
        let mut out = root.to_path_buf();
        for part in rel.split('/') {
            out.push(part);
        }
        out
    }

    fn write_test_dds(root: &Path, rel: &str, width: u32, height: u32) {
        let mut bytes = vec![0u8; 128 + width as usize * height as usize * 4];
        bytes[0..4].copy_from_slice(b"DDS ");
        bytes[4..8].copy_from_slice(&124u32.to_le_bytes());
        bytes[8..12].copy_from_slice(&0x0002_100Fu32.to_le_bytes());
        bytes[12..16].copy_from_slice(&height.to_le_bytes());
        bytes[16..20].copy_from_slice(&width.to_le_bytes());
        bytes[20..24].copy_from_slice(&(width * 4).to_le_bytes());
        bytes[28..32].copy_from_slice(&1u32.to_le_bytes());
        bytes[76..80].copy_from_slice(&32u32.to_le_bytes());
        bytes[80..84].copy_from_slice(&0x41u32.to_le_bytes());
        bytes[88..92].copy_from_slice(&32u32.to_le_bytes());
        bytes[92..96].copy_from_slice(&0x00ff_0000u32.to_le_bytes());
        bytes[96..100].copy_from_slice(&0x0000_ff00u32.to_le_bytes());
        bytes[100..104].copy_from_slice(&0x0000_00ffu32.to_le_bytes());
        bytes[104..108].copy_from_slice(&0xff00_0000u32.to_le_bytes());
        bytes[108..112].copy_from_slice(&0x1000u32.to_le_bytes());
        write_fixture(root, rel, &bytes);
    }

    #[test]
    fn combat_indicator_sprite_reader_uses_mod_resolver_overrides() {
        let root = crate::test_support::temp_test_dir("combat_indicator_mod_override");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        let gfx = br#"
            spriteTypes = {
                spriteType = {
                    name = "GFX_mapicon_land_combat_frame"
                    texturefile = "gfx/interface/mapicons/mapicon_land_combat_frame.dds"
                    noOfFrames = 2
                }
            }
        "#;
        // The land inventory also reads landcombat.gfx (battle modifier strip);
        // provide a minimal one in both roots so the multi-file read resolves.
        let landcombat_gfx = br#"
            spriteTypes = {
                spriteType = {
                    name = "GFX_battle_modifier_strip"
                    texturefile = "gfx/interface/landcombat/battle_modifier_strip.dds"
                    noOfFrames = 10
                }
            }
        "#;
        write_fixture(&vanilla, COMBAT_INDICATOR_MAPICONS_GFX_SOURCE, gfx);
        write_fixture(&mod_root, COMBAT_INDICATOR_MAPICONS_GFX_SOURCE, gfx);
        write_fixture(
            &vanilla,
            COMBAT_INDICATOR_LANDCOMBAT_GFX_SOURCE,
            landcombat_gfx,
        );
        write_fixture(
            &mod_root,
            COMBAT_INDICATOR_LANDCOMBAT_GFX_SOURCE,
            landcombat_gfx,
        );
        write_test_dds(
            &vanilla,
            "gfx/interface/mapicons/mapicon_land_combat_frame.dds",
            76,
            37,
        );
        write_test_dds(
            &mod_root,
            "gfx/interface/mapicons/mapicon_land_combat_frame.dds",
            152,
            37,
        );
        for root in [&vanilla, &mod_root] {
            write_test_dds(
                root,
                "gfx/interface/landcombat/battle_modifier_strip.dds",
                250,
                25,
            );
        }

        let resolver =
            GameFileResolver::with_mod_paths(&vanilla, vec![mod_root]).expect("resolver");
        let sprites =
            land_combat_indicator_sprites_from_resolver(&resolver).expect("decode sprites");

        let frame = sprites
            .get("GFX_mapicon_land_combat_frame")
            .expect("frame sprite");
        assert_eq!(frame.texture_width, 152);
        assert_eq!(frame.no_of_frames, 2);
        assert_eq!(frame.frame_width, 76);
        // Overrides hit: both gfx files + both dds textures.
        assert_eq!(resolver.stats().overridden_file_count, 4);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn vanilla_land_combat_sprites_resolve_and_frame_split() {
        let Some(resolver) = vanilla_resolver() else {
            return;
        };
        let manifest = combat_indicator_assets_from_resolver(&resolver).expect("manifest");

        // N/N resolution gate: every declared land sprite resolves, none unresolved.
        assert_eq!(manifest.land_combat_sprites.len(), 7);
        assert!(manifest.unresolved_sprites.is_empty());

        let backing = manifest
            .land_combat_sprite("GFX_mapicon_land_combat")
            .expect("land combat backing");
        assert_eq!(
            backing.texture_file,
            "gfx/interface/mapicons/mapicon_land_combat.dds"
        );
        assert_eq!(backing.no_of_frames, 1);
        assert_eq!((backing.texture_width, backing.texture_height), (38, 37));

        let frame = manifest
            .land_combat_sprite("GFX_mapicon_land_combat_frame")
            .expect("land combat frame");
        assert_eq!(frame.no_of_frames, 2);
        assert_eq!((frame.texture_width, frame.texture_height), (76, 37));
        assert_eq!((frame.frame_width, frame.frame_height), (38, 37));
        assert_eq!(
            frame.frame(1).expect("frame 1").rect,
            PixelRect {
                x: 38,
                y: 0,
                width: 38,
                height: 37
            }
        );

        let direction = manifest
            .land_combat_sprite("GFX_battle_indicator_direction")
            .expect("direction");
        assert_eq!(
            (direction.texture_width, direction.texture_height),
            (70, 70)
        );

        let strip = manifest
            .land_combat_sprite("GFX_landbattle_strip")
            .expect("landbattle strip");
        assert_eq!(strip.no_of_frames, 20);
        assert_eq!((strip.texture_width, strip.texture_height), (1020, 44));
        assert_eq!((strip.frame_width, strip.frame_height), (51, 44));

        let modifier = manifest
            .land_combat_sprite("GFX_battle_modifier_strip")
            .expect("battle modifier strip");
        assert_eq!(modifier.source_file, COMBAT_INDICATOR_LANDCOMBAT_GFX_SOURCE);
        assert_eq!(modifier.no_of_frames, 10);
        assert_eq!((modifier.texture_width, modifier.texture_height), (250, 25));
        assert_eq!((modifier.frame_width, modifier.frame_height), (25, 25));
    }

    #[test]
    fn vanilla_naval_combat_sprites_resolve_and_frame_split() {
        let Some(resolver) = vanilla_resolver() else {
            return;
        };
        let manifest = combat_indicator_assets_from_resolver(&resolver).expect("manifest");

        assert_eq!(manifest.naval_combat_sprites.len(), 10);

        let naval = manifest
            .naval_combat_sprite("GFX_mapicon_naval_combat")
            .expect("naval combat");
        assert_eq!(naval.no_of_frames, 2);
        assert_eq!((naval.texture_width, naval.texture_height), (80, 33));
        assert_eq!((naval.frame_width, naval.frame_height), (40, 33));

        let result = manifest
            .naval_combat_sprite("GFX_naval_combat_result")
            .expect("naval combat result");
        assert_eq!(result.no_of_frames, 3);
        assert_eq!((result.frame_width, result.frame_height), (35, 33));

        let battle_result = manifest
            .naval_combat_sprite("GFX_mapicon_naval_battle_result")
            .expect("naval battle result");
        assert_eq!(battle_result.no_of_frames, 2);
        assert_eq!(
            (battle_result.frame_width, battle_result.frame_height),
            (33, 33)
        );

        let result_text = manifest
            .naval_combat_sprite("GFX_mapicon_naval_battle_result_text")
            .expect("naval battle result text");
        assert_eq!(result_text.dds_format, "DXT5");
        assert_eq!(
            (result_text.texture_width, result_text.texture_height),
            (17, 17)
        );

        let hq_frame = manifest
            .naval_combat_sprite("GFX_naval_hq_combat_frame")
            .expect("naval hq combat frame");
        assert_eq!(
            hq_frame.source_file,
            COMBAT_INDICATOR_NAVAL_LEADER_MODULE_GFX_SOURCE
        );
        assert_eq!((hq_frame.texture_width, hq_frame.texture_height), (53, 43));

        // Full inventory sanity: 17 resolved combat sprites, 12 unique textures.
        assert_eq!(manifest.resolved_sprite_count(), 17);
    }

    #[test]
    fn vanilla_combat_progress_bars_decode() {
        let Some(resolver) = vanilla_resolver() else {
            return;
        };
        let bars = combat_indicator_progress_bars_from_resolver(&resolver).expect("bars");
        assert_eq!(bars.len(), 2);

        let naval = bars
            .get("GFX_navalcombat_bar_progress_attacker_154")
            .expect("naval attacker bar");
        assert_eq!(
            naval.size,
            Some(GuiSize {
                width: 154,
                height: 14
            })
        );
        assert_eq!(
            naval.texture_file1,
            "gfx/interface/navalcombat/bar_progress_loose_157x14.dds"
        );
        assert_eq!(
            naval.texture_file2,
            "gfx/interface/navalcombat/bar_progress_win_157x14.dds"
        );

        let land = bars
            .get("GFX_land_unit_combat_progressbar")
            .expect("land combat bar");
        assert_eq!(
            land.size,
            Some(GuiSize {
                width: 40,
                height: 5
            })
        );
    }

    #[test]
    fn vanilla_combat_gui_layouts_and_placement_stub_decode() {
        let Some(resolver) = vanilla_resolver() else {
            return;
        };
        let layouts = combat_indicator_gui_layouts_from_resolver(&resolver).expect("layouts");

        assert_eq!(
            layouts.land_combat_mapicon.size.expect("land size"),
            GuiSize {
                width: 35,
                height: 34
            }
        );
        assert_eq!(
            layouts.naval_combat_mapicon.size.expect("naval size"),
            GuiSize {
                width: 33,
                height: 43
            }
        );
        assert_eq!(
            layouts
                .naval_combat_results_mapicon
                .size
                .expect("naval results size"),
            GuiSize {
                width: 19,
                height: 24
            }
        );

        // Land combat container child controls (the file-derived layout tree).
        let bg = layouts
            .land_combat_mapicon
            .control("bg_btn")
            .expect("bg_btn");
        assert_eq!(
            bg.quad_texture_sprite.as_deref(),
            Some("GFX_mapicon_land_combat")
        );
        assert_eq!(bg.position, Some(GuiPoint { x: -2, y: -2 }));

        let progress = layouts
            .land_combat_mapicon
            .control("combat_progress")
            .expect("combat_progress textbox");
        assert_eq!(progress.font.as_deref(), Some("hoi_16mbs"));
        assert_eq!(progress.max_width, Some(30));

        let air_sup = layouts
            .land_combat_mapicon
            .control("air_superiority_icon")
            .expect("air superiority icon");
        assert_eq!(
            air_sup.sprite_type.as_deref(),
            Some("GFX_battle_modifier_strip")
        );

        // Naval combat progress icons.
        let naval_progress = layouts
            .naval_combat_mapicon
            .control("progress")
            .expect("naval progress");
        assert_eq!(
            naval_progress.sprite_type.as_deref(),
            Some("GFX_navalcombat_bar_progress_attacker_154")
        );

        // Placement stub: FILE size present, screen-space fields capture-gated.
        let manifest = combat_indicator_assets_from_resolver(&resolver).expect("manifest");
        let stub = manifest.placement_stub(4242, CombatIndicatorKind::Land);
        assert_eq!(stub.province_id, 4242);
        assert_eq!(
            stub.declared_size,
            Some(GuiSize {
                width: 35,
                height: 34
            })
        );
        assert_eq!(stub.screen_quad_size, None);
        assert!(!stub.anchor_law_known);

        assert_eq!(combat_indicator_gui_container_count(), 5);
    }

    #[test]
    fn vanilla_combat_fonts_decode() {
        let Some(resolver) = vanilla_resolver() else {
            return;
        };
        let fonts = combat_indicator_fonts_from_resolver(&resolver).expect("fonts");
        assert!(fonts.iter().any(|font| {
            font.name == "hoi_16mbs"
                && font
                    .font_files
                    .iter()
                    .any(|file| file == "gfx/fonts/hoi_16mbs")
        }));
    }
}
