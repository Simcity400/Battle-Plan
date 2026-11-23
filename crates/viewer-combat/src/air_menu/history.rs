use super::{Chart, Content, DecodedFrame, State, Vision, bind_hover, loc, text};
use crate::binding::Bindings;
use content_reader_localisation::localisation::Argument;
use engine_executable::executable_tables::auxiliary::AIR_GRAPH_DISPLAYS;
use replay_data::rows::{AirMenuWindow, tables};

const LIMIT: i64 = 0x53e2_d620_fb40;

fn scale(value: i64, channel: usize) -> i64 {
    value.wrapping_mul(if channel == 0 || channel == 41 {
        1_000
    } else {
        100_000
    }) / 100_000
}

fn fixed15(value: i64) -> i64 {
    (value % 100_000).wrapping_mul(0x4000_0000) / 0xc350_0000
        + (value / 100_000).wrapping_mul(0x8000)
}

fn total(window: &AirMenuWindow, channel: usize) -> i64 {
    let capacity = window.records.len();
    let count = if window.wrapped {
        capacity
    } else {
        window.current as usize
    };
    let start = if window.wrapped {
        window.current as usize
    } else {
        0
    };
    (0..count).fold(0_i64, |sum, at| {
        sum.wrapping_add(window.records[(start + at) % capacity][channel])
            .clamp(-LIMIT, LIMIT)
    })
}

fn samples(window: &AirMenuWindow, range: usize, channel: usize) -> Vec<i64> {
    let capacity = window.records.len();
    let omit = if range == 0 { 2 } else { 0 };
    let count = capacity.saturating_sub(omit);
    let filled = if window.wrapped {
        capacity
    } else {
        window.current as usize
    }
    .saturating_sub(omit)
    .min(count);
    let start = if window.wrapped {
        window.current as usize
    } else {
        0
    };
    let mut values = vec![0; count - filled];
    values.extend((1..=filled).map(|at| {
        scale(
            window.records[(start + at) % capacity][channel].clamp(-LIMIT, LIMIT),
            channel,
        )
    }));
    values
}

fn window(
    frame: &DecodedFrame,
    ours: u32,
    region: u32,
    range: usize,
) -> Result<Option<AirMenuWindow>, String> {
    let rows = frame
        .complete::<tables::AirNavalPatrolHistories>()
        .map_err(|e| e.to_string())?;
    if rows
        .binary_search_by_key(&(ours, u32::MAX, 0, u32::MAX), |row| {
            (row.country, row.region, row.level, row.index)
        })
        .is_err()
    {
        return Err("the viewing country's air history availability was not recorded".into());
    }
    let first = rows.partition_point(|row| (row.country, row.region) < (ours, region));
    let end = rows.partition_point(|row| (row.country, row.region) <= (ours, region));
    let region_rows = &rows[first..end];
    if region_rows.is_empty() {
        return Ok(None);
    }
    let at = region_rows
        .binary_search_by_key(&(range as u8, u32::MAX), |row| (row.level, row.index))
        .map_err(|_| "air menu history range is unavailable")?;
    let marker = &region_rows[at];
    let state = marker
        .window
        .ok_or("air menu history ring metadata is unavailable")?;
    state.validate().map_err(|error| error.to_string())?;
    if !state.complete {
        return Err("air menu history channels were unreadable".into());
    }
    let mut window = AirMenuWindow {
        current: state.current,
        wrapped: state.wrapped,
        records: Vec::new(),
    };
    window
        .records
        .try_reserve_exact(state.capacity as usize)
        .map_err(|error| error.to_string())?;
    window.records.resize(
        state.capacity as usize,
        [0; replay_data::rows::AIR_MENU_CHANNELS],
    );
    let start = region_rows.partition_point(|row| row.level < range as u8);
    for row in &region_rows[start..at] {
        let slot = window
            .records
            .get_mut(row.index as usize)
            .ok_or("air menu history slot exceeds its ring")?;
        *slot = row.channels()?;
    }
    Ok(Some(window))
}

fn tooltip(
    content: &Content,
    window: Option<&AirMenuWindow>,
    channel: usize,
) -> Result<String, String> {
    let loss = matches!(
        channel,
        3 | 10 | 17 | 24 | 30 | 40 | 44 | 51 | 58 | 65 | 71 | 81
    );
    let mut text = String::new();
    let reasons = if loss { 7 } else { 1 };
    for reason in 0..reasons {
        if reason == 6 && matches!(channel, 24 | 65) {
            continue;
        }
        let key = match reason {
            0 if !loss => "REASON_NONE_TOTAL_DETAILED",
            0 => "REASON_NONE_TOTAL",
            1 => "REASON_AIR_COMBAT",
            2 => "REASON_LAND_COMBAT",
            3 => "REASON_ACCIDENT",
            4 => "REASON_BOMBED",
            5 => "REASON_ARMORED_TRAINS",
            _ => "REASON_SUICIDE",
        };
        let field = channel + reason;
        text.push_str(&loc(
            content,
            key,
            &[(
                "SUM",
                Argument::Fixed5(window.map_or(0, |window| scale(total(window, field), field))),
            )],
        )?);
        text.push('\n');
    }
    Ok(text)
}

pub(super) fn bind(
    content: &Content,
    frame: &DecodedFrame,
    vision: &Vision,
    region: u32,
    state: &State,
    out: &mut Bindings,
) {
    for (range, name) in ["days", "months", "years"].into_iter().enumerate() {
        out.values
            .entry(format!("details_wnd/btn_{name}"))
            .or_default()
            .frame = Some(if range == state.range { 1 } else { 0 });
    }
    text(
        out,
        "details_wnd/x_axis_type",
        loc(
            content,
            [
                "LINECHART_DETAILS_DAYS",
                "LINECHART_DETAILS_MONTHS",
                "LINECHART_DETAILS_YEARS",
            ][state.range],
            &[],
        ),
    );
    let window = window(frame, vision.ours(), region, state.range);
    let metadata = air_graph();
    let mut graphs = Vec::new();
    let mut maximum = 0_i64;
    for display in 0..AIR_GRAPH_DISPLAYS {
        let path = format!("details_wnd/channel_{}", display + 1);
        let channel = metadata
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|metadata| {
                let channel = metadata.channels[display] as usize;
                if channel < replay_data::rows::AIR_MENU_CHANNELS {
                    Ok(channel)
                } else {
                    Err("air graph channel is outside the supported history schema".into())
                }
            });
        let selected = state.channels & (1 << display) != 0;
        out.values
            .entry(format!("{path}/btn_channel_{}", display + 1))
            .or_default()
            .frame = Some(if selected { 1 } else { 0 });
        match &metadata {
            Ok(metadata) => {
                out.values
                    .entry(format!("{path}/color_block"))
                    .or_default()
                    .color = Some(if selected {
                    metadata.colors[display]
                } else {
                    [0., 0., 0., 1.]
                })
            }
            Err(why) => out.withhold(&[&format!("{path}/color_block")], why.clone()),
        }
        let label = channel.as_ref().map_err(Clone::clone).and_then(|&channel| {
            window.as_ref().map_err(Clone::clone).and_then(|window| {
                loc(
                    content,
                    &format!("AIRVIEW_DETAILS_CHANNEL_{}", display + 1),
                    &[(
                        "VAL",
                        Argument::Fixed5(
                            window
                                .as_ref()
                                .map_or(0, |window| scale(total(window, channel), channel)),
                        ),
                    )],
                )
            })
        });
        let tooltip = channel.as_ref().map_err(Clone::clone).and_then(|&channel| {
            window
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|window| tooltip(content, window.as_ref(), channel))
        });
        let button = format!("{path}/btn_channel_{}", display + 1);
        text(out, &button, label);
        bind_hover(out, &button, tooltip, Ok(String::new()));
        if selected
            && let (Ok(Some(window)), Ok(metadata), Ok(channel)) = (&window, &metadata, &channel)
        {
            let values = samples(window, state.range, *channel);
            let max = values.iter().map(|v| v.wrapping_abs()).max().unwrap_or(0);
            maximum = maximum.max(max);
            let divisor = fixed15(max);
            let color = metadata.colors[display];
            graphs.push(
                values
                    .into_iter()
                    .map(|value| {
                        let value = fixed15(value);
                        let normalized = if divisor > 0 {
                            value.wrapping_shl(15) / divisor
                        } else {
                            value
                        };
                        [
                            color[0],
                            color[1],
                            color[2],
                            (normalized as f32 / 32768.) / metadata.scale,
                        ]
                    })
                    .collect(),
            );
        }
    }
    for (name, value) in [
        ("max_value", maximum),
        ("avg_value", maximum / 2),
        ("min_value", 0),
    ] {
        text(
            out,
            &format!("details_wnd/{name}"),
            window.as_ref().map_err(Clone::clone).and_then(|_| {
                loc(
                    content,
                    "AIRVIEW_GRAPH_VALUE",
                    &[("VALUE", Argument::Fixed5(value))],
                )
            }),
        );
        if maximum < 1 && name != "min_value" {
            out.values
                .entry(format!("details_wnd/{name}"))
                .or_default()
                .hidden = true;
        }
    }
    text(out, "details_wnd/min_x_value", Ok("0".into()));
    text(
        out,
        "details_wnd/max_x_value",
        window
            .as_ref()
            .map(|w| w.as_ref().map_or(0, |w| w.records.len()).to_string())
            .map_err(Clone::clone),
    );
    if matches!(window, Ok(None)) {
        for name in ["min_x_value", "max_x_value", "x_axis_type"] {
            out.values
                .entry(format!("details_wnd/{name}"))
                .or_default()
                .hidden = true;
        }
    }
    if state.details && window.is_ok() && metadata.is_ok() {
        out.air_chart = Some(Chart { graphs });
    }
}

pub(crate) fn draw(
    root: &content_reader_gui::gui::native::Element,
    layout: &viewer_native_gui::native_gui::Layout,
    origin: [i32; 2],
    resources: &impl crate::gui::CombatResources,
    chart: &Chart,
    drawn: &mut crate::Drawn,
) {
    let Some(element) = root.path_named("details_wnd/linechart") else {
        *drawn
            .refused
            .entry("air history chart control is absent from the content".into())
            .or_default() += 1;
        return;
    };
    let Some(region) = layout
        .regions
        .iter()
        .find(|region| region.path == element.path)
    else {
        *drawn
            .refused
            .entry("air history chart has no native layout region".into())
            .or_default() += 1;
        return;
    };
    if crate::gui::region_rect(region, origin).is_none() {
        return;
    }
    let [x, y, width, height] = region.rect;
    let rect = [x + origin[0] as f32, y + origin[1] as f32, width, height];
    let result = (|| {
        let icon = element.icon().ok_or("air history chart is not an icon")?;
        let (effect, half_width) = resources.line_chart(&icon.sprite)?;
        for points in &chart.graphs {
            let first = drawn.batch.quads.len();
            viewer_native_gui::native_shader::draw::line_chart(
                effect.clone(),
                rect,
                half_width,
                points,
                &mut drawn.batch,
            )?;
            for (index, quad) in drawn.batch.quads.iter_mut().enumerate().skip(first) {
                quad.layer = crate::window_layer(index);
                if let Some(exact) = &mut quad.exact {
                    exact.scissor = region.clip.map(|clip| {
                        [
                            clip[0] + origin[0],
                            clip[1] + origin[1],
                            clip[2] + origin[0],
                            clip[3] + origin[1],
                        ]
                    });
                }
            }
        }
        Ok::<_, String>(())
    })();
    if let Err(why) = result {
        *drawn
            .refused
            .entry(format!("air history chart: {why}"))
            .or_default() += 1;
    }
}

struct AirGraph {
    channels: &'static [u32; AIR_GRAPH_DISPLAYS],
    colors: &'static [[f32; 4]; AIR_GRAPH_DISPLAYS],
    scale: f32,
}

fn air_graph() -> Result<AirGraph, String> {
    use engine_executable::executable_tables::{auxiliary_part, is_initialized};
    if !is_initialized() {
        return Err("air graph palette needs the game's executable tables".into());
    }
    let withheld = |withheld: &engine_executable::availability::Withheld| withheld.to_string();
    Ok(AirGraph {
        channels: auxiliary_part(|tables| &tables.air_graph_channels).map_err(withheld)?,
        colors: auxiliary_part(|tables| &tables.air_graph_colors).map_err(withheld)?,
        scale: *auxiliary_part(|tables| &tables.air_graph_scale).map_err(withheld)?,
    })
}
