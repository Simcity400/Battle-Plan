use super::*;
use map_display_data::landscape::{
    ClosedLayer, ColourSettings, ModeProjection, NativeLayer, RefreshDecision,
    ScriptedProjection,
};

pub fn project(mode: &MapMode, inputs: &Inputs<'_, '_>) -> io::Result<ModeProjection> {
    project_with(mode, inputs, None)
}
pub fn project_lazy<'a, 'w>(
    mode: &MapMode,
    inputs: &Inputs<'a, 'w>,
    world: &dyn Fn() -> &'a dashboard_text::triggers::World<'w>,
) -> io::Result<ModeProjection> {
    project_with(mode, inputs, Some(world))
}
fn project_with<'a, 'w>(
    mode: &MapMode,
    inputs: &Inputs<'a, 'w>,
    world: Option<&dyn Fn() -> &'a dashboard_text::triggers::World<'w>>,
) -> io::Result<ModeProjection> {
    let MapMode::Scripted(name) = mode else {
        let MapMode::Stock(stock) = mode else {
            unreachable!()
        };
        return stock_fill(*stock, inputs).map(|fill| ModeProjection::Stock { fill });
    };
    let Some(mode) = inputs.content.scripted.get(name) else {
        return Ok(ModeProjection::Stock {
            fill: refusal(vec![format!(
                "The scripted map mode {name} is not declared by this recording's content"
            )]),
        });
    };
    let forced = match colour_script::forced_refresh(name, inputs) {
        Ok(forced) => RefreshDecision::Value { forced },
        Err(reason) => RefreshDecision::Refused { reason },
    };
    let mut game_layers: Vec<(GameLayer, ClosedLayer)> = Vec::new();
    let mut close = |side,
                     layer: &content_reader_map::scripted_map_modes::Layer|
     -> io::Result<ClosedLayer> {
        match layer.kind {
            LayerType::None => Ok(ClosedLayer::Empty {}),
            LayerType::Game(game) => {
                if matches!(game, GameLayer::States | GameLayer::Diplomacy) {
                    if let Some((_, closed)) = game_layers.iter().find(|(kind, _)| *kind == game) {
                        return Ok(closed.clone());
                    }
                }
                let mut warnings = Vec::new();
                let variant = match game {
                    GameLayer::Country => Some(Variant::Country),
                    GameLayer::Players => Some(Variant::Players),
                    GameLayer::Factions => Some(Variant::Factions),
                    GameLayer::Ideology => Some(Variant::Ideology),
                    GameLayer::States | GameLayer::Diplomacy => None,
                };
                let (kind, layer) = if let Some(variant) = variant {
                    (
                        NativeLayer::Country {},
                        country_variant(variant, inputs, &mut warnings)?,
                    )
                } else if game == GameLayer::States {
                    (NativeLayer::States {}, states(inputs, &mut warnings)?)
                } else {
                    (NativeLayer::Diplomacy {}, diplomacy(inputs, &mut warnings)?)
                };
                let closed = ClosedLayer::Game {
                    kind,
                    layer,
                    warnings,
                };
                game_layers.push((game, closed.clone()));
                Ok(closed)
            }
            LayerType::Country | LayerType::State | LayerType::StateController => {
                let program = match world {
                    Some(world) => colour_script::close_lazy(name, side, layer, inputs, world)?,
                    None => colour_script::close(name, side, layer, inputs)?,
                };
                let province_count = region_table_len(inputs.map, std::iter::empty());
                Ok(ClosedLayer::Colour {
                    program,
                    settings: ColourSettings {
                        thickness: layer
                            .thickness
                            .then_some(inputs.content.defines.scripted_thickness),
                        gradient: (!layer.thickness).then_some(colour_script::FLAT_GRADIENT),
                        channel: inputs.content.defines.country_channel,
                        province_count,
                    },
                })
            }
        }
    };
    Ok(ModeProjection::Scripted {
        program: ScriptedProjection {
            mode: name.clone(),
            hour: inputs.frame.hour,
            update_daily: mode.update_daily,
            forced,
            refresh: colour_script::refresh_interval(name, inputs),
            political: inputs.content.defines.country_channel,
            layers: [close("bottom", &mode.bottom)?, close("top", &mode.top)?],
        },
    })
}
