use super::{Arg, Context, Tooltip, line};
use content_reader_parser::fixed5::fixed5_to_string;
use replay_data::{rows::IntelNetworkRow, rows::tables};
use viewer_native_gui::native_text::{COLOR, FLAG};

const ONE: i64 = 100_000;
const FULL_STRENGTH: i64 = 100 * ONE;
const BONUS: &str = "intel_network_state_level_bonus";
const PENALTY: &str = "intel_network_state_level_penalty";

trait Source {
    fn loc(&self, key: &str, args: &[(&str, Arg<'_>)]) -> Result<String, String>;
    fn slot(&self, raw: u32) -> Result<u32, String>;
    fn tag(&self, slot: u32) -> Result<String, String>;
    fn marked(&self, slot: u32) -> Result<String, String>;
    fn state_name(&self) -> Result<String, String>;
    fn effects(&self, token: &str, scale: i64) -> Result<String, String>;
    fn base_target(&self) -> Result<i64, String>;
}

impl Source for Context<'_> {
    fn loc(&self, key: &str, args: &[(&str, Arg<'_>)]) -> Result<String, String> {
        Context::loc(self, key, args)
    }
    fn slot(&self, raw: u32) -> Result<u32, String> {
        Context::slot(self, raw)
    }
    fn tag(&self, slot: u32) -> Result<String, String> {
        self.countries.tag(slot, "country naming operands absent")
    }
    fn marked(&self, slot: u32) -> Result<String, String> {
        self.marked_slot(slot)
    }
    fn state_name(&self) -> Result<String, String> {
        Context::state_name(self)
    }
    fn effects(&self, token: &str, scale: i64) -> Result<String, String> {
        self.scaled_static_modifiers(token, scale)
    }
    fn base_target(&self) -> Result<i64, String> {
        self.content.province_hover.network_base_target.clone()
    }
}

#[derive(Clone, Copy)]
struct Hover<'a> {
    state: u32,
    controller: u32,
    ours: u32,
    rows: &'a [IntelNetworkRow],
}

pub(super) fn append(context: &Context, out: &mut Tooltip) {
    let state = context.province.state;
    if state == 0 {
        return;
    }
    let frame = context.frame;
    let section = (|| {
        let controller = frame
            .complete::<tables::States>()
            .map_err(|e| e.to_string())?
            .iter()
            .find(|row| row.id == state)
            .ok_or("state row absent")?
            .controller;
        let rows = frame
            .complete::<tables::IntelNetworks>()
            .map_err(|e| e.to_string())?;
        text(
            context,
            &Hover {
                state,
                controller,
                ours: context.vision.ours(),
                rows,
            },
        )
    })();
    line(out, "intel network", 0, section);
}

fn highlighted(text: &str) -> String {
    format!("{COLOR}H{text}{COLOR}!")
}

fn trailing_newlines(text: &mut String, count: usize) {
    if text.is_empty() {
        return;
    }
    let held = text
        .bytes()
        .rev()
        .take(count)
        .take_while(|&b| b == b'\n')
        .count();
    text.extend(std::iter::repeat_n('\n', count - held));
}

fn indented(text: &str) -> String {
    let mut out = String::new();
    for line in text.split_inclusive('\n') {
        if line != "\n" {
            out.push_str("  ");
        }
        out.push_str(line);
    }
    out
}

fn text(context: &impl Source, hover: &Hover) -> Result<String, String> {
    let Hover {
        state,
        controller,
        ours,
        rows,
    } = *hover;
    if controller == 0 {
        return Ok(String::new());
    }
    let controller_slot = context.slot(controller)?;
    let mut node = None;
    for row in rows {
        if row.state_id == state as i32
            && context.slot(row.owner_country)? == ours
            && context.slot(row.target_country)? == controller_slot
        {
            node = Some(row);
            break;
        }
    }
    let flag = format!("{FLAG}{}", context.tag(controller_slot)?);
    let name = context.state_name()?;
    let place = [("STATE", Arg::Text(&name)), ("FLAG", Arg::Text(&flag))];
    let Some(node) = node.filter(|row| row.local_strength_raw > 0) else {
        return context.loc("NO_INTEL_NETWORK_IN_STATE", &place);
    };
    let mut text = context.loc("INTEL_NETWORK_IN_STATE", &place)?;
    if node.subnetwork_quiet != 0 {
        trailing_newlines(&mut text, 2);
        text.push_str(&context.loc("QUIET_INTEL_NETWORK_IN_STATE_DESC", &[])?);
        return Ok(text);
    }
    trailing_newlines(&mut text, 1);
    text.push_str(&highlighted(&context.loc(
        "INTEL_NETWORK_STRENGTH",
        &[("STRENGTH", Arg::Fixed5(node.local_strength_raw))],
    )?));
    text.push('\n');
    text.push_str(&highlighted(&context.loc(
        "INTEL_NETWORK_DAILY_GROWTH",
        &[("GROWTH", Arg::Fixed5(node.gain_raw))],
    )?));
    let growth = growth(context, node)?;
    if !growth.is_empty() {
        text.push('\n');
        text.push_str(&indented(&growth));
    }
    if let Some(target) = node
        .strength_target_raw
        .filter(|_| node.subnetwork_selector >= 0)
    {
        text.push('\n');
        let cap = target.clamp(0, FULL_STRENGTH);
        let colour = if cap < FULL_STRENGTH { 'R' } else { 'Y' };
        let value = format!("{COLOR}{colour}{}%{COLOR}!", fixed5_to_string(cap, 0));
        text.push_str(&highlighted(
            &context.loc("INTEL_NETWORK_STRENGTH_CAP", &[("CAP", Arg::Text(&value))])?,
        ));
        let parts = cap_parts(context, node)?;
        if !parts.is_empty() {
            text.push('\n');
            text.push_str(&indented(&parts));
        }
    }
    let scale = effect_scale(node.local_strength_raw);
    for (country, modifier) in [(None, BONUS), (Some(controller), PENALTY)] {
        trailing_newlines(&mut text, 2);
        let country = match country {
            None => context.marked(ours)?,
            Some(raw) => context.marked(context.slot(raw)?)?,
        };
        text.push_str(&context.loc("EFFECTS_FOR_COUNTRY", &[("COUNTRY", Arg::Text(&country))])?);
        text.push('\n');
        text.push_str(&context.effects(modifier, scale)?);
    }
    Ok(text)
}

fn effect_scale(strength: i64) -> i64 {
    if matches!(strength, i64::MAX | i64::MIN) || strength == i64::MIN + 1 {
        return i64::from(u32::MAX);
    }
    strength.wrapping_mul(ONE) / (FULL_STRENGTH)
}

fn growth(context: &impl Source, node: &IntelNetworkRow) -> Result<String, String> {
    let value = |key: &str, value: i64| context.loc(key, &[("VALUE", Arg::Fixed5(value))]);
    let Some(factor) = node
        .gain_factor_raw
        .filter(|_| node.subnetwork_selector >= 0)
    else {
        return value("INTEL_NETWORK_STRENGTH_GROWTH_OUT_OF_RANGE", node.gain_raw);
    };
    let gain = node.gain_raw;
    if gain < 0 {
        return value("INTEL_NETWORK_STRENGTH_GROWTH_OVER_CAP", gain);
    }
    if gain == 0 {
        let colour = if node.local_strength_raw == FULL_STRENGTH {
            'Y'
        } else {
            'R'
        };
        let text = context.loc("INTEL_NETWORK_STRENGTH_GROWTH_AT_CAP", &[])?;
        return Ok(format!("{COLOR}{colour}{text}{COLOR}!"));
    }
    let mut text = String::new();
    if node.gain_from_operatives_raw != 0 {
        text.push_str(&value(
            "INTEL_NETWORK_STRENGTH_GROWTH_FROM_OPERATIVE",
            node.gain_from_operatives_raw,
        )?);
    }
    if node.gain_from_adjacency_raw != 0 {
        text.push('\n');
        text.push_str(&value(
            "INTEL_NETWORK_STRENGTH_GROWTH_FROM_ADJACENCY",
            node.gain_from_adjacency_raw,
        )?);
    }
    if factor != ONE {
        text.push('\n');
        text.push_str(&value(
            "INTEL_NETWORK_STRENGTH_GROWTH_ENEMY_FACTOR",
            factor.wrapping_sub(ONE),
        )?);
    }
    Ok(text)
}

fn cap_parts(context: &impl Source, node: &IntelNetworkRow) -> Result<String, String> {
    let base = context.base_target()?;
    let mut text = context.loc("STAT_BASE_VALUE", &[])?;
    text.push_str(&format!("{COLOR}Y{}{COLOR}!", fixed5_to_string(base, 0)));
    for (key, part) in [
        (
            "INTEL_NETWORK_STRENGTH_CAP_FROM_OPERATIVE",
            node.target_from_operatives_raw,
        ),
        (
            "INTEL_NETWORK_STRENGTH_CAP_FROM_COUNTER_INTELLIGENCE",
            node.target_from_counterintelligence_raw,
        ),
    ] {
        let part = part.ok_or("subnetwork target parts absent")?;
        if part != 0 {
            text.push('\n');
            text.push_str(&context.loc(key, &[("VALUE", Arg::Fixed5(part))])?);
        }
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use content_reader_localisation::localisation::Localisation;

    struct Fake(Localisation);

    impl Source for Fake {
        fn loc(&self, key: &str, args: &[(&str, Arg<'_>)]) -> Result<String, String> {
            self.0
                .localise_with(key.as_bytes(), args)
                .map_err(|e| e.to_string())
        }
        fn slot(&self, raw: u32) -> Result<u32, String> {
            raw.checked_sub(1).ok_or_else(|| "unlinked".to_owned())
        }
        fn tag(&self, slot: u32) -> Result<String, String> {
            Ok(["GER", "SOV"][slot as usize].to_owned())
        }
        fn marked(&self, slot: u32) -> Result<String, String> {
            Ok(format!("<{}>", self.tag(slot)?))
        }
        fn state_name(&self) -> Result<String, String> {
            Ok("Kiev".to_owned())
        }
        fn effects(&self, token: &str, scale: i64) -> Result<String, String> {
            Ok(format!("  {token}*{scale}"))
        }
        fn base_target(&self) -> Result<i64, String> {
            Ok(9_000_000)
        }
    }

    fn fake() -> Fake {
        Fake(
            [
                ("INTEL_NETWORK_IN_STATE", "in $STATE$ $FLAG$"),
                ("NO_INTEL_NETWORK_IN_STATE", "none in $STATE$ $FLAG$"),
                ("QUIET_INTEL_NETWORK_IN_STATE_DESC", "quiet"),
                ("INTEL_NETWORK_STRENGTH", "strength $STRENGTH|0%%$"),
                ("INTEL_NETWORK_DAILY_GROWTH", "growth $GROWTH|1$"),
                ("INTEL_NETWORK_STRENGTH_CAP", "cap $CAP$"),
                ("STAT_BASE_VALUE", "base "),
                ("INTEL_NETWORK_STRENGTH_CAP_FROM_OPERATIVE", "ops $VALUE|0$"),
                (
                    "INTEL_NETWORK_STRENGTH_CAP_FROM_COUNTER_INTELLIGENCE",
                    "ci $VALUE|0$",
                ),
                (
                    "INTEL_NETWORK_STRENGTH_GROWTH_OUT_OF_RANGE",
                    "out $VALUE|2$",
                ),
                ("INTEL_NETWORK_STRENGTH_GROWTH_OVER_CAP", "over $VALUE|2$"),
                (
                    "INTEL_NETWORK_STRENGTH_GROWTH_FROM_OPERATIVE",
                    "from ops $VALUE|2$",
                ),
                (
                    "INTEL_NETWORK_STRENGTH_GROWTH_FROM_ADJACENCY",
                    "from adj $VALUE|2$",
                ),
                (
                    "INTEL_NETWORK_STRENGTH_GROWTH_ENEMY_FACTOR",
                    "law $VALUE|%0$",
                ),
                ("INTEL_NETWORK_STRENGTH_GROWTH_AT_CAP", "at cap"),
                ("EFFECTS_FOR_COUNTRY", "for $COUNTRY$:"),
            ]
            .into_iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect(),
        )
    }

    fn node() -> IntelNetworkRow {
        IntelNetworkRow {
            owner_country: 1,
            target_country: 2,
            state_id: 7,
            strength_raw: Some(7_250_000),
            subnetwork_selector: 0,
            local_strength_raw: 7_250_000,
            gain_raw: 63_750,
            gain_from_operatives_raw: 42_500,
            gain_from_adjacency_raw: 21_250,
            local_quiet: 0,
            gain_factor_raw: Some(100_000),
            strength_target_raw: Some(5_750_000),
            target_from_operatives_raw: Some(1_500_000),
            target_from_counterintelligence_raw: Some(-4_750_000),
            subnetwork_quiet: 0,
        }
    }

    fn hover(rows: &[IntelNetworkRow]) -> String {
        text(
            &fake(),
            &Hover {
                state: 7,
                controller: 2,
                ours: 0,
                rows,
            },
        )
        .unwrap()
    }

    #[test]
    fn a_growing_network_shows_strength_growth_cap_and_both_countries_effects() {
        assert_eq!(
            hover(&[node()]),
            "in Kiev \u{12}SOV\n\
             \u{11}Hstrength 72%\u{11}!\n\
             \u{11}Hgrowth 0.6\u{11}!\n\
             \x20 from ops 0.42\n\
             \x20 from adj 0.21\n\
             \u{11}Hcap \u{11}R57%\u{11}!\u{11}!\n\
             \x20 base \u{11}Y90\u{11}!\n\
             \x20 ops 15\n\
             \x20 ci -47\n\n\
             for <GER>:\n\
             \x20 intel_network_state_level_bonus*72500\n\n\
             for <SOV>:\n\
             \x20 intel_network_state_level_penalty*72500"
        );
    }

    #[test]
    fn growth_shows_decay_over_and_at_the_cap_and_the_targets_garrison_factor() {
        let with = |change: fn(&mut IntelNetworkRow)| {
            let mut row = node();
            change(&mut row);
            growth(&fake(), &row).unwrap()
        };
        assert_eq!(with(|_| {}), "from ops 0.42\nfrom adj 0.21");
        assert_eq!(
            with(|row| {
                row.gain_from_operatives_raw = 0;
                row.gain_factor_raw = Some(50_000);
            }),
            "\nfrom adj 0.21\nlaw -50%"
        );
        assert_eq!(with(|row| row.gain_raw = -250_000), "over -2.50");
        assert_eq!(with(|row| row.gain_raw = 0), "\u{11}Rat cap\u{11}!");
        assert_eq!(
            with(|row| {
                row.gain_raw = 0;
                row.local_strength_raw = 10_000_000;
            }),
            "\u{11}Yat cap\u{11}!"
        );
        assert_eq!(
            with(|row| {
                row.subnetwork_selector = -1;
                row.gain_raw = -175_000;
            }),
            "out -1.75"
        );
    }

    #[test]
    fn quiet_absent_and_other_countries_networks_and_uncontrolled_states() {
        let mut quiet = node();
        quiet.subnetwork_quiet = 1;
        assert_eq!(hover(&[quiet]), "in Kiev \u{12}SOV\n\nquiet");
        let mut empty = node();
        empty.local_strength_raw = 0;
        let mut theirs = node();
        theirs.owner_country = 2;
        assert_eq!(hover(&[empty]), "none in Kiev \u{12}SOV");
        assert_eq!(hover(&[theirs]), "none in Kiev \u{12}SOV");
        assert_eq!(hover(&[]), "none in Kiev \u{12}SOV");
        let mut full = node();
        full.strength_target_raw = Some(12_000_000);
        assert!(hover(&[full]).contains("cap \u{11}Y100%\u{11}!"));
        let section = text(
            &fake(),
            &Hover {
                state: 7,
                controller: 0,
                ours: 0,
                rows: &[node()],
            },
        );
        assert_eq!(section, Ok(String::new()));
    }

    #[test]
    fn trailing_newlines_top_up_to_the_count_and_leave_empty_text() {
        let mut text = String::new();
        trailing_newlines(&mut text, 2);
        assert_eq!(text, "");
        let mut text = "a".to_owned();
        trailing_newlines(&mut text, 2);
        assert_eq!(text, "a\n\n");
        let mut text = "a\n".to_owned();
        trailing_newlines(&mut text, 2);
        assert_eq!(text, "a\n\n");
        let mut text = "a\n\n\n".to_owned();
        trailing_newlines(&mut text, 2);
        assert_eq!(text, "a\n\n\n");
    }

    #[test]
    fn indentation_skips_empty_lines() {
        assert_eq!(indented("a\n\nb"), "  a\n\n  b");
        assert_eq!(indented("\nb"), "\n  b");
    }

    #[test]
    fn effects_scale_by_the_strengths_share_of_one_hundred() {
        assert_eq!(effect_scale(7_250_000), 72_500);
        assert_eq!(effect_scale(10_000_000), 100_000);
        assert_eq!(effect_scale(-1_234_567), -12_345);
        assert_eq!(effect_scale(i64::MAX), i64::from(u32::MAX));
        assert_eq!(effect_scale(i64::MIN + 1), i64::from(u32::MAX));
    }
}
