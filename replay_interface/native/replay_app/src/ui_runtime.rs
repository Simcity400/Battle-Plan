//! D3D product adapter for the proven game_ui compositor.

use game_ui::assets::AssetResolver;
use game_ui::gfx::SpriteRegistry;
use game_ui::power_balance::PowerBalanceRuntimeSupplement;
use game_ui::variant_admission::{DisplayVariantKey, InputVariant, PixelExtent};
use replay_core::{RecordedInputVariant, WorldFrame};

use crate::d3d::Gfx;
use crate::text::ImageRenderer;
use crate::Result;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectedPresentUiOutcome {
    Executed {
        operation_count: usize,
        m02_draw_count: usize,
    },
    Omitted(String),
}

/// Exact selected-Present UI-pass adapter. Unlike the legacy whole-HUD image,
/// this executes the portable operation frontier in place and therefore can
/// interleave M02 at its authenticated global UI ordinal.
pub struct ProductSelectedPresentRuntime {
    pass_identity_sha256: String,
    variant_key: Option<DisplayVariantKey>,
    resolver: AssetResolver,
    registry: SpriteRegistry,
    image: ImageRenderer,
    m02: map_viewer::NativeM02OverlayExecutor,
    power_balance_runtime: Vec<PowerBalanceRuntimeSupplement>,
    power_balance_runtime_error: Option<String>,
}

impl ProductSelectedPresentRuntime {
    pub fn new(
        gfx: &Gfx,
        resolver: &map_viewer::GameFileResolver,
        content_admission: &replay_core::ValidatedContentAdmission,
        pass: &replay_core::AuthenticatedUiPass,
    ) -> Result<Self> {
        let assets =
            game_ui::selected_present::admitted_asset_resolver(resolver, content_admission, pass)?;
        let mut registry = SpriteRegistry::new();
        for file in assets.list_files_recursive("interface", Some("gfx"))? {
            registry.parse_file(&file.path, &file.rel_path)?;
        }
        let m02 = unsafe {
            map_viewer::NativeM02OverlayExecutor::new(gfx.device.as_raw(), gfx.context.as_raw())?
        };
        Ok(Self {
            pass_identity_sha256: pass.pass_identity_sha256.clone(),
            variant_key: None,
            resolver: assets,
            registry,
            image: ImageRenderer::new(gfx)?,
            m02,
            power_balance_runtime: Vec::new(),
            power_balance_runtime_error: None,
        })
    }

    pub fn is_for_pass(&self, pass: &replay_core::AuthenticatedUiPass) -> bool {
        self.pass_identity_sha256 == pass.pass_identity_sha256
    }

    /// Last typed A37/A25 join prepared by the production UI runtime for the
    /// selected POV. Failure remains feature-local and cannot suppress the
    /// independently authenticated selected-Present pass.
    pub fn power_balance_runtime(
        &self,
    ) -> std::result::Result<&[PowerBalanceRuntimeSupplement], &str> {
        match self.power_balance_runtime_error.as_deref() {
            Some(error) => Err(error),
            None => Ok(&self.power_balance_runtime),
        }
    }

    pub fn execute(
        &mut self,
        gfx: &Gfx,
        game_resolver: &map_viewer::GameFileResolver,
        prepared_frame: &map_viewer::PreparedProductionD3dFrame,
        world_frame: &WorldFrame,
        pov_tag: Option<&str>,
        runtime_text: Option<&replay_core::RuntimeTextCatalog>,
    ) -> Result<SelectedPresentUiOutcome> {
        self.power_balance_runtime.clear();
        self.power_balance_runtime_error = None;
        if let Some(country_tag) = pov_tag {
            match crate::power_balance_runtime::power_balance_runtime_supplements(
                world_frame,
                country_tag,
            ) {
                Ok(runtime) => self.power_balance_runtime = runtime,
                Err(error) => self.power_balance_runtime_error = Some(error),
            }
        }
        let Some(pass) = prepared_frame.authenticated_ui_pass.as_ref() else {
            return Ok(SelectedPresentUiOutcome::Omitted(
                "complete authenticated selected-Present UI-pass carrier is absent".into(),
            ));
        };
        if pass.target.prepared_target != replay_core::PreparedRenderTarget::SwapchainBackBuffer {
            return Err(
                "authenticated selected-Present UI pass does not target SwapchainBackBuffer".into(),
            );
        }
        let variant_key =
            match exact_variant_admission_key(world_frame, pass, gfx.width, gfx.height) {
                Ok(key) => key,
                Err(reason) => return Ok(SelectedPresentUiOutcome::Omitted(reason)),
            };
        if self
            .variant_key
            .as_ref()
            .is_some_and(|admitted| admitted != &variant_key)
        {
            return Ok(SelectedPresentUiOutcome::Omitted(
                "XUI25 exact UI variant changed without a new authenticated selected-Present pass"
                    .into(),
            ));
        }
        self.variant_key = Some(variant_key);

        let playback_context = game_ui::selected_present::SelectedPresentPlaybackContext {
            frame_hour: world_frame.hour_index(),
            pov_country_or_global: pov_tag.unwrap_or("global"),
        };

        let prepared = match game_ui::selected_present::prepare_selected_present_ui_pass(
            &self.resolver,
            &self.registry,
            pass,
            prepared_frame.m02_combat.as_ref(),
            runtime_text,
            playback_context,
        ) {
            Ok(prepared) => prepared,
            Err(error)
                if matches!(
                    error.kind,
                    game_ui::ErrorKind::Missing
                        | game_ui::ErrorKind::UnsupportedDdsFormat(_)
                        | game_ui::ErrorKind::UnsupportedTextureExtension(_)
                ) =>
            {
                return Ok(SelectedPresentUiOutcome::Omitted(format!(
                    "selected-Present UI feature unavailable: {}",
                    error.message
                )));
            }
            Err(error) => return Err(error.into()),
        };
        let rtv = gfx.rtv()?.as_raw();
        let has_m02 = prepared.operations.iter().any(|operation| {
            matches!(
                operation,
                game_ui::selected_present::PreparedUiPassOperation::M02 { .. }
            )
        });
        let mut m02_plan = if has_m02 {
            Some(unsafe {
                self.m02.prepare(
                    game_resolver,
                    rtv,
                    prepared_frame
                        .m02_combat
                        .as_ref()
                        .ok_or("prepared UI pass lost its M02 carrier")?,
                )?
            })
        } else {
            None
        };

        // Exact pipeline commands are also fully preflighted before the first
        // selected-Present operation writes the app target.
        for operation in &prepared.operations {
            if let game_ui::selected_present::PreparedUiPassOperation::ExactPipeline { command } =
                operation
            {
                let replay_core::HudCommandExecution::ExactPipeline { draw } = &command.execution
                else {
                    return Err("prepared exact UI command changed execution family".into());
                };
                unsafe {
                    self.m02.preflight_exact_ui_draw(
                        game_resolver,
                        rtv,
                        &draw.exact_draw,
                        &command.ordered_effective_vfs_used_content,
                    )?;
                }
            }
        }

        let mut m02_draw_count = 0usize;
        for operation in &prepared.operations {
            match operation {
                game_ui::selected_present::PreparedUiPassOperation::M02 { output, .. } => {
                    let plan = m02_plan
                        .as_mut()
                        .ok_or("selected-Present M02 operation has no prepared plan")?;
                    unsafe {
                        self.m02.execute_next(game_resolver, rtv, plan, output)?;
                    }
                    m02_draw_count += 1;
                }
                game_ui::selected_present::PreparedUiPassOperation::ExactPipeline { command } => {
                    let replay_core::HudCommandExecution::ExactPipeline { draw } =
                        &command.execution
                    else {
                        return Err("prepared exact UI command changed execution family".into());
                    };
                    unsafe {
                        self.m02
                            .execute_exact_ui_draw(game_resolver, rtv, &draw.exact_draw)?;
                    }
                }
                game_ui::selected_present::PreparedUiPassOperation::InterpretedPaint {
                    image,
                    ..
                } => {
                    // The layer was fully rendered during the prepare
                    // preflight; upload the proven pixels instead of decoding
                    // and rasterizing the command a second time.
                    self.image.set_rgba(gfx, image)?;
                    self.image.draw(gfx)?;
                }
            }
        }
        if let Some(plan) = m02_plan {
            map_viewer::NativeM02OverlayExecutor::finish(plan)?;
        }
        Ok(SelectedPresentUiOutcome::Executed {
            operation_count: prepared.operations.len(),
            m02_draw_count,
        })
    }
}

fn exact_variant_admission_key(
    frame: &WorldFrame,
    pass: &replay_core::AuthenticatedUiPass,
    client_width: u32,
    client_height: u32,
) -> std::result::Result<DisplayVariantKey, String> {
    let Some(render) = frame.production_render() else {
        return Err("selected-Present UI pass lacks the current production frame".into());
    };
    let Some(carried_pass) = render.authenticated_ui_pass.as_ref() else {
        return Err("selected-Present UI pass is absent from the current production frame".into());
    };
    if carried_pass.pass_identity_sha256 != pass.pass_identity_sha256
        || carried_pass.update_index != pass.update_index
    {
        return Err(
            "selected-Present UI pass does not belong to the current production frame".into(),
        );
    }
    let session = frame.session_metadata();
    exact_variant_key(
        session.display_variant.measured(),
        session.localisation_language.measured().map(String::as_str),
        render.authority.recorded_input_variant,
        PixelExtent {
            width: pass.target.width,
            height: pass.target.height,
        },
        PixelExtent {
            width: client_width,
            height: client_height,
        },
    )
}

fn exact_variant_key(
    display: Option<&replay_core::world::DisplayVariantMetadata>,
    language: Option<&str>,
    recorded_input: Option<RecordedInputVariant>,
    target_extent: PixelExtent,
    client_extent: PixelExtent,
) -> std::result::Result<DisplayVariantKey, String> {
    let Some(display) = display else {
        return Err("XUI25 exact UI variant lacks session.display_variant".into());
    };
    let Some(language) = language else {
        return Err("XUI25 exact UI variant lacks session.localisation_language".into());
    };
    let Some(recorded_input) = recorded_input else {
        return Err("XUI25 exact UI variant lacks recorded input family".into());
    };
    let input = match recorded_input {
        RecordedInputVariant::MouseKeyboard => InputVariant::MouseKeyboard,
        RecordedInputVariant::Keyboard => InputVariant::Keyboard,
        RecordedInputVariant::Controller => InputVariant::Controller,
    };
    let key = match DisplayVariantKey::from_session_metadata(display, language, input) {
        Ok(key) => key,
        Err(error) => return Err(format!("XUI25 exact UI variant is unsupported: {error:?}")),
    };
    if key.physical_viewport != target_extent || key.physical_viewport != client_extent {
        return Err(format!(
            "XUI25 exact UI variant mismatch: recorded {:?}, pass {:?}, client {:?}",
            key.physical_viewport, target_extent, client_extent
        ));
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_route_source_has_no_flattened_product_ui_fallback() {
        let source = include_str!("ui_runtime.rs");
        let legacy_type = ["struct Product", "UiRuntime"].concat();
        let legacy_compose = ["compose_hud", "_frame("].concat();
        assert!(!source.contains(&legacy_type));
        assert!(!source.contains(&legacy_compose));
    }

    fn display() -> replay_core::world::DisplayVariantMetadata {
        replay_core::world::DisplayVariantMetadata {
            physical_width: 1920,
            physical_height: 1080,
            min_gui_width: 1280,
            min_gui_height: 720,
            gui_scale_f32_bits: 1.0f32.to_bits(),
            refresh_rate_hz: 60,
            max_refresh_rate_hz: Some(60),
            fullscreen: false,
            borderless: true,
            display_mode: "borderless_fullscreen".into(),
            settings_file_sha256: "11".repeat(32),
        }
    }

    #[test]
    fn xui25_exact_admission_checks_every_recorded_dimension() {
        let extent = PixelExtent {
            width: 1920,
            height: 1080,
        };
        assert!(exact_variant_key(
            Some(&display()),
            Some("l_english"),
            Some(RecordedInputVariant::MouseKeyboard),
            extent,
            extent,
        )
        .is_ok());

        let mut variants = Vec::new();
        let mut value = display();
        value.physical_width += 1;
        variants.push(value);
        let mut value = display();
        value.min_gui_width += 1;
        variants.push(value);
        let mut value = display();
        value.gui_scale_f32_bits += 1;
        variants.push(value);
        let mut value = display();
        value.display_mode = "windowed".into();
        variants.push(value);
        for value in variants {
            // The selected-Present pass is the producer-authenticated exact
            // envelope. A display mutation must either change its physical
            // extent or remain represented in the exact key without being
            // silently defaulted.
            let result = exact_variant_key(
                Some(&value),
                Some("l_english"),
                Some(RecordedInputVariant::MouseKeyboard),
                extent,
                extent,
            );
            if value.physical_width != extent.width {
                assert!(result.is_err());
            } else {
                let key = result.unwrap();
                assert_eq!(key.min_gui.width, value.min_gui_width);
                assert_eq!(key.gui_scale_f32_bits, value.gui_scale_f32_bits);
                assert_eq!(
                    key.display_mode,
                    if value.display_mode == "windowed" {
                        game_ui::variant_admission::DisplayMode::Windowed
                    } else {
                        game_ui::variant_admission::DisplayMode::BorderlessFullscreen
                    }
                );
            }
        }

        let english = exact_variant_key(
            Some(&display()),
            Some("l_english"),
            Some(RecordedInputVariant::MouseKeyboard),
            extent,
            extent,
        )
        .unwrap();
        let french = exact_variant_key(
            Some(&display()),
            Some("l_french"),
            Some(RecordedInputVariant::MouseKeyboard),
            extent,
            extent,
        )
        .unwrap();
        assert_ne!(english, french);
    }

    #[test]
    fn xui25_missing_language_display_input_and_unsupported_input_omit_locally() {
        let extent = PixelExtent {
            width: 1920,
            height: 1080,
        };
        for result in [
            exact_variant_key(
                None,
                Some("l_english"),
                Some(RecordedInputVariant::MouseKeyboard),
                extent,
                extent,
            ),
            exact_variant_key(
                Some(&display()),
                None,
                Some(RecordedInputVariant::MouseKeyboard),
                extent,
                extent,
            ),
            exact_variant_key(Some(&display()), Some("l_english"), None, extent, extent),
            exact_variant_key(
                Some(&display()),
                Some("l_english"),
                Some(RecordedInputVariant::Controller),
                extent,
                extent,
            ),
        ] {
            assert!(result.is_err());
        }
    }
}
