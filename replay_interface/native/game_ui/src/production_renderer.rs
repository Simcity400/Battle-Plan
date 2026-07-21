//! Production replay UI composition.
//!
//! This is deliberately narrower than the evidence/probe compositor.  It is
//! the product entry point and therefore admits only packet surfaces whose
//! replay-owned laws are proven and their product surface authority is
//! authenticated. Omitted families are not given chrome, placeholders, or
//! partially populated rows here.

use crate::assets::AssetResolver;
use crate::binding::{
    apply_clock, clock_binding_from_frame, outliner_binding_from_frame, topbar_binding_from_frame,
};
use crate::compose::UiAssetCache;
use crate::dds::RgbaImage;
use crate::gfx::SpriteRegistry;
use crate::hud::{
    compose_authenticated_hud, HudBinding, HudSpriteStats, MinimapBinding, TooltipBinding,
};
use crate::loc::Localisation;
use crate::variant_admission::{
    DisplayMode, DisplayVariantKey, ExactKeyVariantContract, InputVariant, PixelExtent,
};
use crate::{GameUiError, Result};
use replay_core::{MapmodeProvenance, RecordedInputVariant, ValidatedContentAdmission, WorldFrame};
use std::collections::BTreeSet;
use std::sync::Mutex;

pub const PRODUCTION_UI_WIDTH: usize = 1920;
pub const PRODUCTION_UI_HEIGHT: usize = 1080;
pub const PRODUCTION_UI_MIN_GUI: PixelExtent = PixelExtent {
    width: 1280,
    height: 720,
};
pub const PRODUCTION_UI_GUI_SCALE_BITS: u32 = 0x3f80_0000;
pub const PRODUCTION_UI_LANGUAGE: &str = "l_english";

pub fn production_ui_variant_key() -> DisplayVariantKey {
    DisplayVariantKey {
        physical_viewport: PixelExtent {
            width: PRODUCTION_UI_WIDTH as u32,
            height: PRODUCTION_UI_HEIGHT as u32,
        },
        min_gui: PRODUCTION_UI_MIN_GUI,
        gui_scale_f32_bits: PRODUCTION_UI_GUI_SCALE_BITS,
        display_mode: DisplayMode::BorderlessFullscreen,
        localisation_language: PRODUCTION_UI_LANGUAGE.to_string(),
        input_variant: InputVariant::MouseKeyboard,
    }
}

const PRODUCT_HUD_SURFACES: &[(&str, &str, &str)] = &[
    ("topbar", "topbar", crate::topbar::TOPBAR_GUI),
    (
        "minimap",
        crate::hud::MINIMAP_WINDOW_NAME,
        crate::hud::MINIMAP_GUI,
    ),
    (
        "mapmodes",
        crate::hud::MAPMODES_WINDOW_NAME,
        crate::hud::MAPMODES_GUI,
    ),
    (
        "theatre_selector",
        crate::hud::OUTLINER_WINDOW_NAME,
        crate::hud::OUTLINER_GUI,
    ),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedProductHudRoute {
    consumer_prefixes: Vec<String>,
}

impl AuthenticatedProductHudRoute {
    pub fn consumer_prefixes(&self) -> impl Iterator<Item = &str> {
        self.consumer_prefixes.iter().map(String::as_str)
    }
}

/// Admit the current all-or-nothing HUD compositor only when the replay binds
/// every always-on runtime surface to the exact GUI identity this compositor
/// implements and records the compositor's full authored read-set beneath
/// those semantic surface consumers. A replay with no complete route is a
/// normal omission; a malformed claimed route is an error.
pub fn admit_authenticated_product_hud_route(
    admission: &replay_core::ValidatedContentAdmission,
) -> Result<Option<AuthenticatedProductHudRoute>> {
    let mut prefixes = Vec::new();
    let mut selected_paths = BTreeSet::new();
    let mut missing_surface = false;
    for (surface, root, expected_gui_path) in PRODUCT_HUD_SURFACES {
        let prefix = format!("hud_role/v1/{surface}/");
        let consumers = admission
            .required_consumers()
            .filter(|consumer| consumer.starts_with(&prefix))
            .collect::<Vec<_>>();
        if consumers.is_empty() {
            missing_surface = true;
            continue;
        }
        let any_gui_prefix = format!("{prefix}gui/");
        let gui_consumers = consumers
            .iter()
            .copied()
            .filter(|consumer| consumer.starts_with(&any_gui_prefix))
            .collect::<Vec<_>>();
        if gui_consumers.len() != 1 {
            return Err(GameUiError::parse(format!(
                "authenticated HUD surface {surface} has {} GUI-root consumers",
                gui_consumers.len()
            )));
        }
        let gui_prefix = format!("{prefix}gui/{root}/");
        if !gui_consumers[0].starts_with(&gui_prefix) {
            missing_surface = true;
            continue;
        }
        let gui_rows = admission
            .require_used_content_for_consumer(gui_consumers[0])
            .map_err(GameUiError::missing)?;
        if gui_rows.len() != 1
            || gui_consumers[0].strip_prefix(&gui_prefix)
                != Some(gui_rows[0].payload_sha256.as_str())
        {
            return Err(GameUiError::parse(format!(
                "authenticated HUD surface {surface} has a malformed GUI-root binding"
            )));
        }
        if gui_rows[0].normalized_game_relative_path != *expected_gui_path {
            missing_surface = true;
            continue;
        }
        for consumer in consumers {
            for content in admission
                .require_used_content_for_consumer(consumer)
                .map_err(GameUiError::missing)?
            {
                selected_paths.insert(content.normalized_game_relative_path.as_str());
            }
        }
        prefixes.push(prefix);
    }
    if missing_surface {
        return Ok(None);
    }
    for required in REQUIRED_AUTHORED_HUD_SOURCES {
        if !selected_paths.contains(required) {
            return Ok(None);
        }
    }
    Ok(Some(AuthenticatedProductHudRoute {
        consumer_prefixes: prefixes,
    }))
}

/// Authored a729 source identities consumed by this exact HUD law. These are
/// not resolver defaults: the replay must list every one as used content, and
/// a profile whose controller namespace renamed or removed one is unsupported.
pub const REQUIRED_AUTHORED_HUD_SOURCES: &[&str] = &[
    crate::topbar::TOPBAR_GUI,
    crate::hud::MINIMAP_GUI,
    crate::hud::MAPMODES_GUI,
    crate::hud::OUTLINER_GUI,
];

/// Only packets with an actual replay-app composition/input route are listed.
/// Promotion-ready semantic/canvas laws remain internal when product surface
/// activation or semantic root authority is absent.
///
/// The historical public name denotes route eligibility, not proof that every
/// extractor-side authority producer is active. In particular, XUI25 still
/// rejects until d77 carries its authenticated same-launch input-family receipt.
pub const PROMOTED_XUI_PACKETS: &[&str] = &["XUI24", "XUI25"];

/// XUI27 is the post-XUI26 producer-role extension that makes the four core
/// HUD roots routable. It is reported separately so the historical XUI01-26
/// partition remains exact.
pub const PROMOTED_EXTENSION_PACKETS: &[&str] = &["XUI27"];

/// Proven semantic/topology packets whose exact canvases are deliberately not
/// product-routed because the replay does not authenticate the live product
/// surface activation and semantic window/root binding.
pub const OMITTED_PIXEL_CANVAS_ROUTES: &[(&str, &str)] = &[
    (
        "XUI09",
        "authenticated research/doctrine surface activation and semantic GUI-root binding; the XUI09 canvas/data law itself is promotion-ready",
    ),
    (
        "XUI10",
        "authenticated focus-view surface activation and semantic GUI-root binding; the XUI10 focus presentation/canvas law itself is promotion-ready",
    ),
    (
        "XUI13",
        "authenticated designer surface activation and semantic GUI-root binding; canonical A27 catalog packaging and d78 display order are admitted",
    ),
    (
        "XUI15",
        "authenticated country-trade window activation and semantic countrytradeview root binding; the exact d43 carrier and selected-POV typed presentation are production-routed",
    ),
];

/// Successor packet outside the historical XUI01-XUI26 partition.
pub const OMITTED_EXTENSION_PACKETS: &[(&str, &str)] = &[];

/// Packet surfaces intentionally absent from the production image until their
/// authenticated carrier exists.  Kept machine-readable and terse so product
/// code and the promotion report cannot silently diverge.
pub const OMITTED_AUTHORITY_BLOCKED_PACKETS: &[(&str, &str)] = &[
    (
        "XUI01",
        "authenticated semantic HUD source/window role bindings for the selected effective content manifest",
    ),
    (
        "XUI02",
        "an admitted product panel route; omitted HUD playback must not require or parse unrelated UI content",
    ),
    ("XUI03", "authenticated controller-boundary collection rows"),
    (
        "XUI04",
        "authenticated semantic HUD element bindings for the selected effective content manifest",
    ),
    (
        "XUI05",
        "d72 is reserved pending authenticated same-Present live-matrix evidence; the TTI2 final-blob consumer is wired but unschedulable",
    ),
    (
        "XUI06",
        "runtime localisation, formatting, glyph, wrap and inline-icon runs",
    ),
    (
        "XUI07",
        "authenticated semantic HUD window and sprite bindings for the selected effective content manifest",
    ),
    ("XUI08", "active designer and peace-conference carrier"),
    ("XUI11", "evaluated politics tooltip and runtime inputs"),
    (
        "XUI12",
        "authenticated d71 countryproductionlineview item_grid materialized slot rectangles, clip, and scroll state",
    ),
    ("XUI14", "intelligence and operative producer inputs"),
    (
        "XUI16",
        "runtime text and authenticated collection ordering",
    ),
    ("XUI17", "runtime text and authenticated visible order"),
    ("XUI18", "A37 power-balance runtime presentation inputs"),
    ("XUI19", "A34 complete ordered decision-panel carrier"),
    (
        "XUI20",
        "complete combat-detail and loss presentation carriers",
    ),
    ("XUI21", "fired-event and popup runtime carrier"),
    ("XUI22", "A38 scripted-GUI engine-materialisation carrier"),
    ("XUI23", "A36 selected-context active-ability carrier"),
    (
        "XUI26",
        "recording-authenticated frontend family/source admission for the selected effective content manifest",
    ),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProductionUiRejection {
    UnsupportedDisplayVariant { width: usize, height: usize },
    MissingDisplayVariantCarrier(&'static str),
    UnsupportedInputVariant(InputVariant),
    UnsupportedExactDisplayVariant(DisplayVariantKey),
    Asset(String),
    IncompleteComposition(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionUiReport {
    pub viewport: [usize; 2],
    pub pov_tag: Option<String>,
    pub promoted_packets: Vec<String>,
    pub promoted_extension_packets: Vec<String>,
    pub omitted_authority_blocked_packets: Vec<String>,
    pub omitted_pixel_canvas_packets: Vec<String>,
    pub omitted_extension_packets: Vec<String>,
    pub drawn_elements: usize,
    pub sprite_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProductionUiOutput {
    pub image: RgbaImage,
    pub report: ProductionUiReport,
}

/// Convert the authenticated d72 TTI2 carrier into the one global the game
/// tooltip binding. Domain absence, measured-empty, POV mismatch, and display
/// mismatch all deliberately produce no shell.
pub fn tooltip_binding_from_frame(
    frame: &WorldFrame,
    pov_tag: Option<&str>,
    viewport_width: usize,
    viewport_height: usize,
) -> Option<TooltipBinding> {
    tooltip_binding_from_exact_view(
        frame.resolved_tooltip_exact(),
        pov_tag,
        viewport_width,
        viewport_height,
    )
}

pub fn tooltip_binding_from_exact_view(
    view: &replay_core::ResolvedTooltipExactView,
    pov_tag: Option<&str>,
    viewport_width: usize,
    viewport_height: usize,
) -> Option<TooltipBinding> {
    if view.status != replay_core::ViewStatus::Present {
        return None;
    }
    let exact = view.frame.as_ref()?;
    let [instance] = exact.instances.as_slice() else {
        return None;
    };
    let pov_matches = match pov_tag {
        Some(tag) => {
            instance.pov_country_or_global == tag
                || instance.pov_country_or_global == "global"
        }
        None => instance.pov_country_or_global == "global",
    };
    if !pov_matches
        || usize::try_from(instance.viewport_width_px).ok() != Some(viewport_width)
        || usize::try_from(instance.viewport_height_px).ok() != Some(viewport_height)
    {
        return None;
    }
    Some(TooltipBinding {
        x: instance.screen_rect.x,
        y: instance.screen_rect.y,
        width: usize::try_from(instance.screen_rect.width).ok()?,
        height: usize::try_from(instance.screen_rect.height).ok()?,
        text: instance.final_resolved_text.clone(),
    })
}

pub struct ProductionUiRenderer {
    resolver: AssetResolver,
    registry: SpriteRegistry,
    localisation: Localisation,
    content_admission: ValidatedContentAdmission,
    /// Decoded sprite/texture/font cache reused across composed frames. Owned
    /// alongside its exact resolver + registry pair and interior-mutable so
    /// `compose_hud_frame` keeps its `&self` signature.
    asset_cache: Mutex<UiAssetCache>,
}

impl ProductionUiRenderer {
    /// Build the production renderer from the exact effective asset resolver.
    /// Any malformed GFX source fails construction; product rendering never
    /// falls back to a probe/sample registry.
    pub fn new(
        resolver: AssetResolver,
        content_admission: &replay_core::ValidatedContentAdmission,
    ) -> Result<Self> {
        if !resolver.is_admitted_for(content_admission) {
            return Err(GameUiError::missing(
                "production UI resolver is not restricted to the admitted replay content manifest",
            ));
        }
        for source in REQUIRED_AUTHORED_HUD_SOURCES {
            content_admission
                .require_used_content(source)
                .map_err(GameUiError::missing)?;
        }
        let mut registry = SpriteRegistry::new();
        let files = resolver.list_files_recursive("interface", Some("gfx"))?;
        if files.is_empty() {
            return Err(GameUiError::missing(
                "production UI has no effective interface/*.gfx files",
            ));
        }
        for file in files {
            registry.parse_file(&file.path, &file.rel_path)?;
        }
        if registry.is_empty() {
            return Err(GameUiError::missing(
                "production UI effective GFX registry is empty",
            ));
        }
        let localisation = Localisation::load_english(&resolver)?;
        Ok(Self {
            resolver,
            registry,
            localisation,
            content_admission: content_admission.clone(),
            asset_cache: Mutex::new(UiAssetCache::default()),
        })
    }

    pub fn resolver(&self) -> &AssetResolver {
        &self.resolver
    }

    pub fn registry(&self) -> &SpriteRegistry {
        &self.registry
    }

    /// Admit the exact XUI16 occupation/garrison panel model for production
    /// playback. The panel is not part of the always-on HUD compositor, but
    /// its product route still requires every authored source consumed by the
    /// binding to belong to the replay's effective content manifest.
    pub fn admit_xui16_surface(
        &self,
        request: crate::occupation_binding::OccupationProductionSurfaceRequest<'_>,
    ) -> std::result::Result<
        crate::occupation_binding::AdmittedOccupationSurface,
        ProductionUiRejection,
    > {
        for source in [
            crate::occupation_binding::COUNTRY_OCCUPATION_GUI,
            crate::occupation_binding::GARRISON_LOG_GUI,
            crate::occupation_binding::COUNTRY_STATE_GUI,
            crate::occupation_binding::OCCUPATION_LAWS_SOURCE,
        ] {
            self.content_admission
                .require_used_content(source)
                .map_err(ProductionUiRejection::Asset)?;
        }
        crate::occupation_binding::admit_occupation_surface_from_world(request).map_err(
            |rejection| {
                ProductionUiRejection::IncompleteComposition(format!(
                    "XUI16 occupation surface rejected: {rejection:?}"
                ))
            },
        )
    }

    /// Compose the always-on, proven replay HUD.  Country-dependent content is
    /// present only for an explicit POV and measured WorldFrame values.  The
    /// topbar compositor owns the exact absent/unobtained omission law; this
    /// entry point does not insert zeros or sample values.
    pub fn compose_hud_frame(
        &self,
        frame: Option<&WorldFrame>,
        pov_tag: Option<&str>,
        playback_speed_step: usize,
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<ProductionUiOutput, ProductionUiRejection> {
        let input_variant = frame
            .and_then(WorldFrame::production_render)
            .and_then(|render| render.authority.recorded_input_variant)
            .map(|variant| match variant {
                RecordedInputVariant::MouseKeyboard => InputVariant::MouseKeyboard,
                RecordedInputVariant::Keyboard => InputVariant::Keyboard,
                RecordedInputVariant::Controller => InputVariant::Controller,
            });
        self.compose_hud_frame_for_input(
            frame,
            pov_tag,
            playback_speed_step,
            input_variant,
            viewport_width,
            viewport_height,
        )
    }

    /// Variant-explicit entry point. Production calls this only with a value
    /// decoded from the replay session; tests may supply a fixture value.
    pub fn compose_hud_frame_for_input(
        &self,
        frame: Option<&WorldFrame>,
        pov_tag: Option<&str>,
        playback_speed_step: usize,
        input_variant: Option<InputVariant>,
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<ProductionUiOutput, ProductionUiRejection> {
        if viewport_width != PRODUCTION_UI_WIDTH || viewport_height != PRODUCTION_UI_HEIGHT {
            return Err(ProductionUiRejection::UnsupportedDisplayVariant {
                width: viewport_width,
                height: viewport_height,
            });
        }
        let frame = frame.ok_or(ProductionUiRejection::MissingDisplayVariantCarrier(
            "WorldFrame",
        ))?;
        let session = frame.session_metadata();
        let display = session.display_variant.measured().ok_or(
            ProductionUiRejection::MissingDisplayVariantCarrier("session.display_variant"),
        )?;
        let language = session.localisation_language.measured().ok_or(
            ProductionUiRejection::MissingDisplayVariantCarrier("session.localisation_language"),
        )?;
        let input_variant = input_variant.ok_or(
            ProductionUiRejection::MissingDisplayVariantCarrier("session.input_variant"),
        )?;
        if input_variant != InputVariant::MouseKeyboard {
            return Err(ProductionUiRejection::UnsupportedInputVariant(
                input_variant,
            ));
        }
        let key = DisplayVariantKey::from_session_metadata(display, language, input_variant)
            .map_err(|_| {
                ProductionUiRejection::MissingDisplayVariantCarrier("invalid exact key")
            })?;
        let contract = ExactKeyVariantContract::admit(production_ui_variant_key()).map_err(|_| {
            ProductionUiRejection::MissingDisplayVariantCarrier(
                "invalid product exact-key contract",
            )
        })?;
        contract
            .select(&key)
            .map_err(|_| ProductionUiRejection::UnsupportedExactDisplayVariant(key.clone()))?;

        let mut binding = HudBinding {
            minimap: MinimapBinding::default(),
            ..HudBinding::default()
        };
        binding.mapmode = self.mapmode_binding(frame)?;
        if let Some(pov_tag) = pov_tag {
            binding.topbar = topbar_binding_from_frame(frame, pov_tag);
            apply_clock(
                &mut binding.topbar,
                &clock_binding_from_frame(frame, playback_speed_step, &self.localisation),
            );
            binding.outliner = outliner_binding_from_frame(frame, pov_tag);
        } else {
            // Clock is global replay state and remains valid in omniscient
            // observer mode. Country fields remain absent.
            apply_clock(
                &mut binding.topbar,
                &clock_binding_from_frame(frame, playback_speed_step, &self.localisation),
            );
        }
        // The d72 scheduler remains reserved pending its live matrix, but the
        // consumer is complete and fail-closed: only a selected-Present-joined
        // TTI2 exact blob for this POV and physical viewport reaches the HUD.
        binding.tooltip =
            tooltip_binding_from_frame(frame, pov_tag, viewport_width, viewport_height);

        // A poisoned lock only means a previous compose panicked after fully
        // inserted cache entries; the cache itself stays coherent.
        let mut asset_cache = self
            .asset_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let composed = compose_authenticated_hud(
            &self.resolver,
            &self.registry,
            &binding,
            &self.localisation,
            &mut asset_cache,
            viewport_width,
            viewport_height,
        )
        .map_err(|error| ProductionUiRejection::Asset(error.to_string()))?;
        drop(asset_cache);
        validate_complete_hud(&composed.sprite_stats, &composed.output.report)
            .map_err(ProductionUiRejection::IncompleteComposition)?;

        Ok(ProductionUiOutput {
            image: composed.output.image,
            report: ProductionUiReport {
                viewport: [viewport_width, viewport_height],
                pov_tag: pov_tag.map(str::to_string),
                promoted_packets: PROMOTED_XUI_PACKETS
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect(),
                promoted_extension_packets: PROMOTED_EXTENSION_PACKETS
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect(),
                omitted_authority_blocked_packets: OMITTED_AUTHORITY_BLOCKED_PACKETS
                    .iter()
                    .map(|(packet, _)| (*packet).to_string())
                    .collect(),
                omitted_pixel_canvas_packets: OMITTED_PIXEL_CANVAS_ROUTES
                    .iter()
                    .map(|(packet, _)| (*packet).to_string())
                    .collect(),
                omitted_extension_packets: OMITTED_EXTENSION_PACKETS
                    .iter()
                    .map(|(packet, _)| (*packet).to_string())
                    .collect(),
                drawn_elements: composed.output.report.drawn_elements,
                sprite_count: self.registry.len(),
            },
        })
    }

    fn mapmode_binding(
        &self,
        frame: &WorldFrame,
    ) -> std::result::Result<crate::hud::MapmodeBinding, ProductionUiRejection> {
        let Some(state) = frame
            .production_render()
            .and_then(|render| render.authenticated_ui_pass.as_ref())
            .and_then(|ui_pass| ui_pass.active_mapmode_state.as_ref())
        else {
            return Ok(crate::hud::MapmodeBinding::default());
        };
        let selected_button = u8::try_from(state.button_ordinal)
            .ok()
            .filter(|button| (1..=12).contains(button))
            .ok_or_else(|| {
                ProductionUiRejection::IncompleteComposition(
                    "active mapmode button ordinal is outside 1..=12".into(),
                )
            })?;
        let expected_button_identity = format!("mapmode_button_{selected_button}");
        if state.authored_button_identity != expected_button_identity {
            return Err(ProductionUiRejection::IncompleteComposition(format!(
                "active mapmode button identity {:?} does not match ordinal {selected_button}",
                state.authored_button_identity
            )));
        }
        let use_four_button_layout = match state.layout_root_token.as_str() {
            "3_button_mapmode" => Some(false),
            "4_button_mapmode" => Some(true),
            other => {
                return Err(ProductionUiRejection::IncompleteComposition(format!(
                    "unsupported active mapmode layout root {other:?}"
                )))
            }
        };
        if let MapmodeProvenance::EffectiveVfs { content } = &state.provenance {
            let admitted = self
                .content_admission
                .require_used_content(&content.normalized_game_relative_path)
                .map_err(ProductionUiRejection::Asset)?;
            if admitted != content {
                return Err(ProductionUiRejection::Asset(
                    "active mapmode effective-VFS provenance differs from playback admission"
                        .into(),
                ));
            }
        }
        Ok(crate::hud::MapmodeBinding {
            selected_button,
            use_four_button_layout,
        })
    }
}

fn validate_complete_hud(
    stats: &HudSpriteStats,
    report: &crate::compose::ComposeReport,
) -> std::result::Result<(), String> {
    if !stats.missing.is_empty()
        || !stats.undecodable.is_empty()
        || !report.missing_sprites.is_empty()
        || !report.undecodable_sprites.is_empty()
        || !report.missing_fonts.is_empty()
        || !report.unresolved_loc_keys.is_empty()
        || !report.unresolved_text_inputs.is_empty()
    {
        return Err(format!(
            "production HUD is incomplete: stats_missing={} stats_undecodable={} missing_sprites={} undecodable_sprites={} missing_fonts={} unresolved_loc={} unresolved_runtime_text={}; stats_missing_items={:?}; stats_undecodable_items={:?}; missing_sprite_items={:?}; undecodable_sprite_items={:?}; missing_font_items={:?}; unresolved_loc_items={:?}; unresolved_runtime_text_items={:?}",
            stats.missing.len(),
            stats.undecodable.len(),
            report.missing_sprites.len(),
            report.undecodable_sprites.len(),
            report.missing_fonts.len(),
            report.unresolved_loc_keys.len(),
            report.unresolved_text_inputs.len(),
            stats.missing,
            stats.undecodable,
            report.missing_sprites,
            report.undecodable_sprites,
            report.missing_fonts,
            report.unresolved_loc_keys,
            report.unresolved_text_inputs,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_partition_has_no_overlap_and_keeps_all_authority_blocked_surfaces_omitted() {
        let promoted = PROMOTED_XUI_PACKETS
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        let omitted = OMITTED_AUTHORITY_BLOCKED_PACKETS
            .iter()
            .map(|(packet, _)| *packet)
            .collect::<std::collections::BTreeSet<_>>();
        assert!(promoted.is_disjoint(&omitted));
        let omitted_canvas = OMITTED_PIXEL_CANVAS_ROUTES
            .iter()
            .map(|(packet, _)| *packet)
            .collect::<std::collections::BTreeSet<_>>();
        assert!(promoted.is_disjoint(&omitted_canvas));
        assert_eq!(omitted_canvas.len(), 4);
        assert_eq!(promoted, ["XUI24", "XUI25"].into_iter().collect());
        assert_eq!(omitted.len(), 20);
        assert_eq!(PROMOTED_EXTENSION_PACKETS, ["XUI27"]);
        assert!(OMITTED_EXTENSION_PACKETS.is_empty());
        assert!(OMITTED_AUTHORITY_BLOCKED_PACKETS
            .iter()
            .all(|(_, dependency)| !dependency.trim().is_empty()));
    }

    #[test]
    fn only_the_frozen_xui25_viewport_is_admitted() {
        assert_eq!([PRODUCTION_UI_WIDTH, PRODUCTION_UI_HEIGHT], [1920, 1080]);
        let rejection = ProductionUiRejection::UnsupportedDisplayVariant {
            width: 1280,
            height: 720,
        };
        assert!(matches!(
            rejection,
            ProductionUiRejection::UnsupportedDisplayVariant {
                width: 1280,
                height: 720
            }
        ));
    }
}
