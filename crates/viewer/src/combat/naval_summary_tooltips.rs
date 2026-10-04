//! Dynamic hovers of the active naval combat summary (1417D4980).

use super::{
    binding::{Bindings, TextIcons, builtin},
    tooltips::{hover_text, set_hover},
};
use crate::{document::Document, map::tables::rows};
use installed_content::localisation::Localisation;
use replay::{frame::DecodedFrame, rows::tables};
use std::io;

// `1417D4980` chooses these executable keys for paired stat text and
// background widgets. Its second string comes from `1417D1A90`, which reads
// side-level damage history no current-member census can reconstruct.
super::text::tooltip_keys! {
    COMBAT_STAT_LIGHT_GUNS_INFO = "combat_stat_light_guns_info",
    COMBAT_STAT_HEAVY_GUNS_INFO = "combat_stat_heavy_guns_info",
    COMBAT_STAT_TORPEDOES_INFO = "combat_stat_torpedoes_info",
    COMBAT_STAT_DEPTH_CHARGES_INFO = "combat_stat_depth_charges_info",
    SCREENING_EFFICIENCY_INFO = "screening_efficiency_info",
}
const WEAPON_HOVERS: [(&str, &str); 4] = [
    ("stat_light_gun", COMBAT_STAT_LIGHT_GUNS_INFO),
    ("stat_heavy_gun", COMBAT_STAT_HEAVY_GUNS_INFO),
    ("stat_torpedo", COMBAT_STAT_TORPEDOES_INFO),
    ("stat_depth_charges", COMBAT_STAT_DEPTH_CHARGES_INFO),
];

fn header(
    out: &mut Bindings,
    icons: &dyn TextIcons,
    loc: &Localisation,
    id: u64,
    side: &str,
    keys: &[String],
    name: &str,
) {
    let text = match builtin(icons, loc, name, &[]) {
        Ok(text) => hover_text(&text),
        Err(reason) => {
            out.refuse(format!(
                "Naval combat {id} {side} {name} hover is unavailable: {reason}"
            ));
            None
        }
    };
    for key in keys {
        set_hover(&mut out.values, key.clone(), text.clone(), None);
    }
}

fn side_hovers(out: &mut Bindings, icons: &dyn TextIcons, loc: &Localisation, id: u64, side: &str) {
    let path = format!("header_container/{side}_wnd");
    for (widget, key) in WEAPON_HOVERS {
        let pair = [format!("{path}/{widget}"), format!("{path}/{widget}_bg")];
        header(out, icons, loc, id, side, &pair, key);
        out.refuse(format!(
            "Naval combat {id} {side} {widget} damage breakdown is unavailable: CNavalCombatant+0x168 has no recorded 6x6 damage-history matrix and the opposing CNavalCombatant+0x158 original-health denominator is not recorded (1417D1A90, 14160E9C0, 1416084E0)"
        ));
    }
    // `1417D4980` emits this header for each screening icon; `14160A7B0`
    // builds the optional explanation from the category census and modifier
    // descriptions. The latter require the ordered native accumulator tree.
    for widget in [
        "capital_screening_efficiency",
        "carrier_screening_efficiency",
    ] {
        header(
            out,
            icons,
            loc,
            id,
            side,
            &[format!("{side}_wnd_{widget}")],
            SCREENING_EFFICIENCY_INFO,
        );
        out.refuse(format!(
            "Naval combat {id} {side} {widget} breakdown is unavailable: ordered modifier-accumulator child labels and per-key contributions are not recorded (14160A7B0, 14054D0F0)"
        ));
    }
    // `1417D4980` appends `141609460`'s positioning description to its
    // heading itself. Withhold the whole text if that body cannot be made.
    for widget in ["stat_positioning", "stat_positioning_bg"] {
        set_hover(&mut out.values, format!("{path}/{widget}"), None, None);
    }
    out.refuse(format!(
        "Naval combat {id} {side} positioning hover is unavailable: the ordered location, doctrine and admiral modifier-accumulator descriptions consumed by 141609460 are not recorded (1417D4980, 1415B4BB0, 14054D0F0)"
    ));
}

/// Called after authored GUI hovers, so native dynamic stat callbacks own
/// their widgets even when a historical operand is absent.
pub(super) fn bind(
    out: &mut Bindings,
    document: &Document,
    frame: &DecodedFrame,
    id: u64,
) -> io::Result<()> {
    let details = rows::<tables::NavalCombatDetails>(frame)?;
    let Some(detail) = details.rows().iter().find(|row| row.combat == id) else {
        return Ok(());
    };
    let icons = &document.icons.combat;
    let loc = &document.names.localisation;
    for (side, name) in ["left", "right"].into_iter().enumerate() {
        // `1417D4980` returns false before selecting a stat when both
        // CCombatant primary country references are empty.
        if !detail.sides[side].summary.countries.is_empty() {
            side_hovers(out, icons, loc, id, name);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::binding::NoIcons;

    #[test]
    fn native_stat_headers_pair_text_and_background_and_name_missing_history() {
        let loc: Localisation = [
            (
                "combat_stat_light_guns_info".to_owned(),
                "Light guns".to_owned(),
            ),
            (
                "screening_efficiency_info".to_owned(),
                "Screening".to_owned(),
            ),
        ]
        .into_iter()
        .collect();
        let mut out = Bindings::default();
        side_hovers(&mut out, &NoIcons, &loc, 42, "left");
        let text = &out.values["header_container/left_wnd/stat_light_gun"];
        let background = &out.values["header_container/left_wnd/stat_light_gun_bg"];
        assert_eq!(text.tooltip.as_deref(), Some("Light guns"));
        assert_eq!(background.tooltip, text.tooltip);
        assert!(background.hover_owned);
        assert!(background.tooltip_description.is_none());
        assert_eq!(
            out.values["left_wnd_capital_screening_efficiency"]
                .tooltip
                .as_deref(),
            Some("Screening")
        );
        assert!(out.values["header_container/left_wnd/stat_positioning"].hover_owned);
        assert!(
            out.values["header_container/left_wnd/stat_positioning"]
                .tooltip
                .is_none()
        );
        assert_eq!(out.refused().len(), 7);
        assert!(
            out.refused()
                .keys()
                .any(|reason| reason.contains("6x6 damage-history matrix"))
        );
    }
}
