use super::{Content, DecodedFrame, State, Vision, bind_hover, equipment::Sources, loc, text};
use crate::{
    Values,
    binding::{Bindings, SPRITE},
};
use content_reader_localisation::localisation::Argument;
use replay_data::rows::tables;
use replay_derive::air_region_derive::{self as derive, EquipmentPools, Item, Retained};

const DAMAGE: [&str; 11] = [
    "TO_BUILDINGS",
    "TO_SHIPS",
    "TO_DIVISIONS",
    "DISRUPTED_PLANES",
    "FROM_PLANES",
    "FROM_AA",
    "TO_LOGISTICS",
    "TO_RAILWAY_GUNS",
    "",
    "",
    "FROM_TRAINS",
];

fn retained<'a>(
    content: &Content,
    frame: &'a DecodedFrame,
    vision: &Vision,
    region: u32,
    state: &State,
) -> Result<Retained<'a>, String> {
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(|e| e.to_string())?;
    let rows = frame
        .complete::<tables::AirRegionCombat>()
        .map_err(|e| e.to_string())?;
    let Some(row) = rows.iter().find(|row| {
        row.region == region
            && links
                .iter()
                .any(|link| link.id == row.country && link.canonical_slot == vision.ours())
    }) else {
        return Ok(Retained {
            own: &[],
            foreign: &[],
        });
    };
    let pass = state.pass.as_ref().map_err(Clone::clone)?;
    let timeout = *content.assets.air_timeout.as_ref().map_err(Clone::clone)?;
    Ok(derive::retained_through(row, *pass, timeout))
}

fn damage(
    content: &Content,
    sources: &Sources<'_>,
    state: &State,
    retained: &Result<Retained<'_>, String>,
    out: &mut Bindings,
    item: Item,
) -> Result<Values, String> {
    let amount = match item.cause {
        0 | 1 | 2 | 6 | 7 => content_reader_parser::fixed5::fixed5_to_string(item.amount, 2),
        3 => loc(
            content,
            "AIRWING_DISRUPTED_VAL",
            &[("VAL", Argument::Fixed5(item.amount))],
        )?,
        _ => ((item
            .amount
            .wrapping_add(if item.amount < 0 { -50_000 } else { 50_000 })
            / 100_000) as i32)
            .to_string(),
    };
    let mut values = Values::new();
    values.entry("count_txt".into()).or_default().text = Some(format!(
        "\u{11}{}{amount}\u{11}!",
        if item.ours() { 'G' } else { 'R' }
    ));
    values.entry("damage_type".into()).or_default().frame = Some(item.cause);
    let tooltip = loc(
        content,
        &format!(
            "AIR_VIEW_DAMAGE_{}_BY_{}",
            DAMAGE[item.cause as usize],
            if item.ours() { "US" } else { "ENEMY" }
        ),
        &[],
    )
    .and_then(|tooltip| {
        if item.cause == 4 {
            sources.damage_tooltip(
                tooltip,
                *retained.as_ref().map_err(Clone::clone)?,
                state,
                item.flag,
            )
        } else {
            Ok(tooltip)
        }
    });
    let tooltip = match tooltip {
        Ok(tooltip) => crate::tooltips::hover_text(&tooltip),
        Err(why) => {
            out.refuse(why);
            None
        }
    };
    for key in ["damage_type", "count_txt"] {
        crate::tooltips::set_hover(&mut values, key, tooltip.clone(), None);
    }
    Ok(values)
}

fn grid(
    content: &Content,
    sources: &Sources<'_>,
    state: &State,
    retained: &Result<Retained<'_>, String>,
    out: &mut Bindings,
    key: &str,
    items: Result<&[Item], String>,
) {
    let items = items.and_then(|items| {
        items
            .iter()
            .map(|&item| damage(content, sources, state, retained, out, item))
            .collect::<Result<Vec<_>, _>>()
    });
    match items {
        Ok(items) => {
            let widget = out
                .values
                .entry(format!("air_combat_wnd/{key}"))
                .or_default();
            widget.entry_template = Some("air_combat_damage_received".into());
            widget.entries = items;
        }
        Err(why) => out.withhold(&[&format!("air_combat_wnd/{key}")], why),
    }
}

fn pools_count(content: &Content, pools: &EquipmentPools) -> Result<Option<String>, String> {
    let counts = pools.each_ref().map(|pool| pool.total_raw);
    if counts.iter().all(|count| *count < 1) {
        return Ok(None);
    }
    let mut text = String::new();
    for (index, key) in [
        "PLANE_COUNTS_IN_REGION",
        "PLANE_COUNTS_PASSING_THROUGH_REGION",
    ]
    .into_iter()
    .enumerate()
    {
        if counts[index] > 0 {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&loc(
                content,
                key,
                &[("COUNTS", Argument::Int32((counts[index] / 100_000) as i32))],
            )?);
        }
    }
    Ok(Some(text))
}

fn equipment(
    content: &Content,
    sources: &Sources<'_>,
    state: &State,
    retained: &Result<Retained<'_>, String>,
    out: &mut Bindings,
    key: &str,
    pools: Result<EquipmentPools, String>,
    enemy: bool,
    fighter: bool,
) {
    let path = format!("air_combat_wnd/{key}");
    let count_key = format!("{path}s_count");
    match pools
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|pools| pools_count(content, pools))
    {
        Ok(Some(value)) => text(out, &count_key, Ok(value)),
        Ok(None) => out.values.entry(count_key.clone()).or_default().hidden = true,
        Err(why) => text(out, &count_key, Err(why)),
    }
    bind_hover(
        out,
        &count_key,
        loc(
            content,
            if fighter || enemy != state.enemy {
                "AIR_VIEW_AIRPLANES_IN_COMBAT_FIGHTERS"
            } else {
                "AIR_VIEW_AIRPLANES_IN_COMBAT"
            },
            &[],
        ),
        Ok(String::new()),
    );
    let picture_count = if fighter { 2 } else { 3 };
    let pictures = pools
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|pools| sources.pictures(pools, fighter));
    for at in 0..picture_count {
        let key = format!(
            "{path}_equipment_icon{}",
            if at == 0 {
                String::new()
            } else {
                format!("_{}", at + 1)
            }
        );
        match &pictures {
            Ok(pictures) => match &pictures[at] {
                Some(sprite) => out.bind(&key, SPRITE, Ok(sprite.clone())),
                None => out.values.entry(key).or_default().hidden = true,
            },
            Err(why) => out.withhold(&[&key], why.clone()),
        }
    }
    let tooltip = retained
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|&retained| sources.tooltip(retained, enemy, fighter, state.enemy));
    bind_hover(
        out,
        &format!("{path}_equipment_icon"),
        tooltip,
        Ok(String::new()),
    );
}

pub(super) fn bind(
    content: &Content,
    frame: &DecodedFrame,
    vision: &Vision,
    region: u32,
    state: &State,
    out: &mut Bindings,
) {
    let retained = retained(content, frame, vision, region, state);
    let sources = Sources::new(content, frame);
    for enemy in [false, true] {
        let side = if enemy { "enemy" } else { "our" };
        let grids = retained
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|&retained| {
                derive::filtered_grids(retained, frame.hour, enemy, state.enemy, state.missions)
                    .map_err(str::to_owned)
            });
        grid(
            content,
            &sources,
            state,
            &retained,
            out,
            &format!("{side}_fighters_damage_list"),
            grids
                .as_ref()
                .map(|g| g.air.as_slice())
                .map_err(Clone::clone),
        );
        grid(
            content,
            &sources,
            state,
            &retained,
            out,
            &format!("{side}_bombers_damage_list"),
            grids
                .as_ref()
                .map(|g| g.ground.as_slice())
                .map_err(Clone::clone),
        );
        for (fighter, missions, classification) in [
            (true, 5, 0),
            (
                false,
                if enemy == state.enemy {
                    state.missions
                } else {
                    5
                },
                1,
            ),
        ] {
            let pools = retained
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|&retained| sources.pools(retained, enemy, missions, classification));
            equipment(
                content,
                &sources,
                state,
                &retained,
                out,
                &format!("{side}_{}", if fighter { "fighter" } else { "bomber" }),
                pools,
                enemy,
                fighter,
            );
        }
    }
}

pub(crate) fn initial_side(
    content: &Content,
    frame: &DecodedFrame,
    vision: &Vision,
    region: u32,
    state: &State,
) -> Result<bool, String> {
    let retained = retained(content, frame, vision, region, state)?;
    let sources = Sources::new(content, frame);
    let own = sources.pools(retained, false, state.missions & !0x4000, 1)?;
    let enemy = sources.pools(retained, true, state.missions, 1)?;
    Ok((own[0].total_raw / 100_000) as i32 == 0 && (enemy[0].total_raw / 100_000) as i32 >= 1)
}
