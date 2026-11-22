use super::*;
pub(super) fn fill(name: &str, inputs: &Inputs) -> io::Result<ModeFill> {
    let Some(mode) = inputs.content.scripted.get(name) else {
        return Ok(refusal(vec![format!(
            "The scripted map mode {name} is not declared by this recording's content"
        )]));
    };
    let mut warnings = Vec::new();
    let previous = inputs.colour.filter(|state| state.mode == name);
    let scheduled = colour_oracle::refreshes(previous, mode.update_daily, inputs.frame.hour);
    let forced = if scheduled {
        Ok(false)
    } else {
        colour_oracle::forced_refresh(name, inputs)
    };
    let refresh = scheduled || forced.as_ref().is_ok_and(|value| *value);
    let mut kept: [map_display_data::landscape::LayerState; 2] = Default::default();
    let mut coloured = false;
    let political = inputs.content.defines.country_channel;
    let mut draw = Draw::of([Source::Empty; 2], political);
    let mut held: Vec<(GameLayer, Source)> = Vec::new();
    for (channel, (side, layer)) in [("bottom", &mode.bottom), ("top", &mode.top)]
        .into_iter()
        .enumerate()
    {
        draw.channels[channel] = match layer.kind {
            LayerType::None => Source::Empty,
            LayerType::Game(game) => {
                let variant = match game {
                    GameLayer::Country => Some(Variant::Country),
                    GameLayer::Players => Some(Variant::Players),
                    GameLayer::Factions => Some(Variant::Factions),
                    GameLayer::Ideology => Some(Variant::Ideology),
                    GameLayer::States | GameLayer::Diplomacy => None,
                };
                match variant {
                    Some(variant) => {
                        let country = country_variant(variant, inputs, &mut warnings)?;
                        let source = built(&country, Source::Country);
                        if channel == 1 && draw.channels[0] == Source::Country {
                            draw.channels[0] = source;
                        }
                        draw.country = country;
                        source
                    }
                    None if held.iter().any(|(layer, _)| *layer == game) => held
                        .iter()
                        .find_map(|&(layer, source)| (layer == game).then_some(source))
                        .unwrap_or(Source::Empty),
                    None => {
                        let layer = if game == GameLayer::States {
                            states(inputs, &mut warnings)?
                        } else {
                            diplomacy(inputs, &mut warnings)?
                        };
                        let (slot, source) = if draw.layer.is_none() {
                            (&mut draw.layer, Source::Mode)
                        } else {
                            (&mut draw.second, Source::Second)
                        };
                        let source = built(&layer, source);
                        *slot = layer;
                        held.push((game, source));
                        source
                    }
                }
            }
            LayerType::Country | LayerType::State | LayerType::StateController => {
                coloured = true;
                let borders = if let Err(why) = &forced {
                    warnings.push(why.clone());
                    map_display_data::landscape::LayerState {
                        borders: BTreeMap::new(),
                        refusals: vec![why.clone()],
                        refused: true,
                    }
                } else {
                    colour_oracle::borders(
                        name,
                        side,
                        layer,
                        inputs,
                        previous.map(|state| &state.layers[channel]),
                        refresh,
                        &mut warnings,
                    )
                };
                let built_layer = (!borders.refused)
                    .then(|| colour_oracle::layer(layer, &borders.borders, inputs));
                kept[channel] = borders;
                let (slot, source) = if draw.layer.is_none() {
                    (&mut draw.layer, Source::Mode)
                } else {
                    (&mut draw.second, Source::Second)
                };
                let source = built(&built_layer, source);
                *slot = built_layer;
                source
            }
        };
    }
    Ok(ModeFill {
        draw: Some(draw),
        warnings,
        colour: coloured.then(|| {
            Arc::new(ColourState {
                mode: name.to_owned(),
                layers: kept,
            })
        }),
    })
}
