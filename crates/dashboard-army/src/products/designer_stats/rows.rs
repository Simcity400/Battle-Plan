//! The designer's stat rows: its three grids' fixed rosters in the game's order, each value
//! the preview block's word formatted by the stat's own `_VALUE` localisation (or trimmed, for
//! width, manpower and training time), then the equipment grid's requirement rows, and its
//! four summary values and width text.
//!
//! A row shows its value only when its stat is [`PROVEN`]: its calculator rule is established
//! and its result matched the game's own recorded division caches. Every other row keeps its
//! label and names why its value is unavailable.

use installed_content_localisation::localisation::{Argument, Localisation};

use super::{Preview, kernel::Block};

/// The grid a row belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grid {
    Base,
    Combat,
    Equipment,
}

/// What a row reads from the preview block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operand {
    /// A stat word, formatted by its own value key.
    Stat(&'static str),
    CombatWidth,
    Manpower,
    TrainingTime,
}

/// One row of the game's fixed rosters: its grid, operand and label key (`None`: the stat's own
/// label from the executable's metadata).
pub const ROSTER: [(Grid, Operand, Option<&str>); 27] = [
    (Grid::Base, Operand::Stat("maximum_speed"), None),
    (Grid::Base, Operand::Stat("max_strength"), None),
    (Grid::Base, Operand::Stat("max_organisation"), None),
    (Grid::Base, Operand::Stat("default_morale"), None),
    (Grid::Base, Operand::Stat("recon"), None),
    (Grid::Base, Operand::Stat("suppression"), None),
    (Grid::Base, Operand::Stat("weight"), None),
    (Grid::Base, Operand::Stat("supply_consumption"), None),
    (
        Grid::Base,
        Operand::Stat("reliability"),
        Some("STAT_COMMON_RELIABILITY_AVERAGE"),
    ),
    (Grid::Base, Operand::Stat("reliability_factor"), None),
    (Grid::Base, Operand::Stat("casualty_trickleback"), None),
    (Grid::Base, Operand::Stat("experience_loss_factor"), None),
    (Grid::Combat, Operand::Stat("soft_attack"), None),
    (Grid::Combat, Operand::Stat("hard_attack"), None),
    (Grid::Combat, Operand::Stat("air_attack"), None),
    (Grid::Combat, Operand::Stat("defense"), None),
    (Grid::Combat, Operand::Stat("breakthrough"), None),
    (Grid::Combat, Operand::Stat("armor_value"), None),
    (Grid::Combat, Operand::Stat("ap_attack"), None),
    (Grid::Combat, Operand::Stat("initiative"), None),
    (Grid::Combat, Operand::Stat("entrenchment"), None),
    (
        Grid::Combat,
        Operand::Stat("equipment_capture_factor"),
        None,
    ),
    (
        Grid::Combat,
        Operand::CombatWidth,
        Some("DESIGNER_COMBATWIDTH"),
    ),
    (
        Grid::Equipment,
        Operand::Manpower,
        Some("DESIGNER_MANPOWER"),
    ),
    (
        Grid::Equipment,
        Operand::TrainingTime,
        Some("DESIGNER_TRAINING_TIME"),
    ),
    (Grid::Equipment, Operand::Stat("fuel_capacity"), None),
    (Grid::Equipment, Operand::Stat("fuel_consumption"), None),
];

/// The stats whose designer value is shown, by the owner's standard: each matched every
/// deployed-division comparison of a full run against the game's recorded caches, with nonzero
/// values among them ([`super::validation`]): directly where the deployed pass leaves the
/// calculator's value, and after rebuilding that pass from the recorded country and commander
/// modifiers where it does not. Suppression is compared with the stat block's own word, and fuel
/// capacity with its capacity word. A comparison is not attributed to the operands that built
/// each cache. The other rows ([`UNPROVEN`]) name why they are not shown.
pub const PROVEN: [&str; 23] = [
    "maximum_speed",
    "max_strength",
    "max_organisation",
    "suppression",
    "weight",
    "reliability",
    "reliability_factor",
    "experience_loss_factor",
    "soft_attack",
    "hard_attack",
    "air_attack",
    "defense",
    "breakthrough",
    "armor_value",
    "ap_attack",
    "initiative",
    "entrenchment",
    "hardness",
    "combat_width",
    "manpower",
    "training_time",
    "fuel_capacity",
    "fuel_consumption",
];

/// The roster's stats not in [`PROVEN`], each with why: its full-run comparisons do not all
/// match, match only at zero, or its deployed pass holds terms no recording has. Their calculator
/// rules are established; the validation names each one's state.
pub const UNPROVEN: [(&str, &str); 5] = [
    (
        "default_morale",
        "its deployed caches do not all match (some predate their commander's last change)",
    ),
    (
        "recon",
        "its deployed caches do not all match (some predate their commander's last change)",
    ),
    (
        "supply_consumption",
        "its deployed pass's regional weather and faction theatre terms are not recorded",
    ),
    (
        "casualty_trickleback",
        "its deployed caches have matched it only at zero",
    ),
    (
        "equipment_capture_factor",
        "its deployed pass's state capture terms are not recorded",
    ),
];

/// Why a row's value is unavailable while its operand is not proven.
fn unproven(operand: Operand) -> String {
    let lacks = match operand {
        Operand::Stat(stat) => UNPROVEN
            .iter()
            .find(|(unproven, _)| *unproven == stat)
            .map(|(_, lacks)| *lacks),
        _ => None,
    };
    match lacks {
        Some(lacks) => {
            format!("its designer value is not yet proven against the game's own division stats: {lacks}")
        }
        None => "its designer value is not yet proven against the game's own division stats".into(),
    }
}

/// Whether a row's operand is proven.
fn proven(operand: Operand) -> bool {
    match operand {
        Operand::Stat(stat) => PROVEN.contains(&stat),
        Operand::CombatWidth => PROVEN.contains(&"combat_width"),
        Operand::Manpower => PROVEN.contains(&"manpower"),
        Operand::TrainingTime => PROVEN.contains(&"training_time"),
    }
}

/// One drawn row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub grid: Grid,
    /// The label's localised text.
    pub label: Result<String, String>,
    /// The value's localised text, or why it is unavailable.
    pub value: Result<String, String>,
}

/// A fixed5 value as the game's trimmed formatter writes it: signed, up to five decimals,
/// trailing zeroes removed.
pub fn trimmed(raw: i64) -> String {
    let sign = if raw < 0 { "-" } else { "" };
    let magnitude = raw.unsigned_abs();
    let whole = magnitude / 100_000;
    let fraction = magnitude % 100_000;
    if fraction == 0 {
        return format!("{sign}{whole}");
    }
    let digits = format!("{fraction:05}");
    format!("{sign}{whole}.{}", digits.trim_end_matches('0'))
}

/// The stat's label key from the executable's metadata.
fn label_key(stat: &str) -> Result<&'static str, String> {
    installed_content_core::equipment::stat_index(stat.as_bytes())
        .and_then(|index| installed_content_core::equipment::stat_metadata().get(index))
        .map(|(_, label, _)| *label)
        .ok_or_else(|| format!("the executable's stat metadata has no {stat}"))
}

fn localise(
    localisation: &Localisation,
    key: &str,
    arguments: &[(&str, Argument<'_>)],
) -> Result<String, String> {
    localisation
        .localise_with(key.as_bytes(), arguments)
        .map_err(|why| format!("{key}: {why:?}"))
}

/// A stat word as its value key formats it.
pub fn stat_value(localisation: &Localisation, stat: &str, raw: i64) -> Result<String, String> {
    let key = format!("{}_VALUE", label_key(stat)?);
    localise(localisation, &key, &[("VALUE", Argument::Fixed5(raw))])
}

/// The rows of the three grids from `preview`. `empty` is the designer's empty-template test
/// (the leading line and support slots hold nothing): every fixed row then shows the no-stat
/// text. `tag` is the country the designer belongs to, whose own short equipment names the
/// requirement rows use (`None`: not recorded, which refuses their labels).
pub fn rows(
    localisation: &Localisation,
    preview: &Result<Preview, String>,
    empty: bool,
    tag: Option<&str>,
) -> Vec<Row> {
    let block = || -> Result<&Block, String> {
        preview
            .as_ref()
            .map_err(Clone::clone)?
            .block
            .as_ref()
            .map_err(Clone::clone)
    };
    let mut rows: Vec<Row> = ROSTER
        .iter()
        .map(|&(grid, operand, label)| {
            let label = match (label, operand) {
                (Some(key), _) => localise(localisation, key, &[]),
                (None, Operand::Stat(stat)) => {
                    label_key(stat).and_then(|key| localise(localisation, key, &[]))
                }
                (None, _) => Err("the row names no label".into()),
            };
            let value = if empty {
                localise(localisation, "DESIGNER_NO_STAT", &[])
            } else if !proven(operand) {
                Err(unproven(operand))
            } else {
                match operand {
                    Operand::Stat(stat) => block().and_then(|block| {
                        let index = installed_content_core::equipment::stat_index(stat.as_bytes())
                            .ok_or_else(|| format!("the executable's stat metadata has no {stat}"))?;
                        let raw = block.stats[index].ok_or_else(|| {
                            format!("the calculator's {stat} rule is not established")
                        })?;
                        stat_value(localisation, stat, raw)
                    }),
                    Operand::CombatWidth => block().map(|block| trimmed(block.combat_width)),
                    Operand::Manpower => preview
                        .as_ref()
                        .map(|preview| trimmed(i64::from(preview.manpower) * 100_000))
                        .map_err(Clone::clone),
                    Operand::TrainingTime => preview
                        .as_ref()
                        .map(|preview| trimmed(i64::from(preview.training_time) * 100_000))
                        .map_err(Clone::clone),
                }
            };
            Row { grid, label, value }
        })
        .collect();
    if let Ok(preview) = preview {
        for (name, amount) in &preview.requirements {
            let quantity = required_quantity(*amount);
            if quantity == 0 {
                continue;
            }
            rows.push(Row {
                grid: Grid::Equipment,
                label: tag
                    .ok_or_else(|| "the designer's country is not recorded".to_owned())
                    .and_then(|tag| short_name(localisation, name, tag)),
                value: Ok(trimmed(quantity.wrapping_mul(100_000))),
            });
        }
    }
    rows
}

/// A requirement's whole quantity as the designer shows it: a positive fraction rounds up to
/// the next whole number, a negative one truncates toward zero.
pub fn required_quantity(amount: i64) -> i64 {
    let whole = amount / 100_000;
    if amount > 0 && amount % 100_000 != 0 {
        whole + 1
    } else {
        whole
    }
}

/// An equipment type's short name for country `tag`: the country's own short name, else the
/// short name, else the country's own name, else the name.
pub fn short_name(localisation: &Localisation, name: &str, tag: &str) -> Result<String, String> {
    let short = format!("{name}_short");
    let candidates = [
        format!("{tag}_{short}"),
        short,
        format!("{tag}_{name}"),
        name.to_owned(),
    ];
    let key = candidates
        .iter()
        .find(|key| localisation.get(key).is_some())
        .unwrap_or(&candidates[3]);
    localise(localisation, key, &[])
}

/// The summary controls' values in their order (hardness, organisation, strength, speed), and
/// the width text.
pub fn summary(
    localisation: &Localisation,
    preview: &Result<Preview, String>,
) -> ([Result<String, String>; 4], Result<String, String>) {
    let block = || -> Result<&Block, String> {
        preview
            .as_ref()
            .map_err(Clone::clone)?
            .block
            .as_ref()
            .map_err(Clone::clone)
    };
    let value = |stat: &'static str| -> Result<String, String> {
        if !PROVEN.contains(&stat) {
            return Err(format!(
                "{stat} is not yet proven against the game's own division stats"
            ));
        }
        let index = installed_content_core::equipment::stat_index(stat.as_bytes())
            .ok_or_else(|| format!("the executable's stat metadata has no {stat}"))?;
        let raw = block()?.stats[index]
            .ok_or_else(|| format!("the calculator's {stat} rule is not established"))?;
        stat_value(localisation, stat, raw)
    };
    let width = if PROVEN.contains(&"combat_width") {
        block().map(|block| trimmed(block.combat_width))
    } else {
        Err("combat width is not yet proven against the game's own division stats".into())
    };
    (
        [
            value("hardness"),
            value("max_organisation"),
            value("max_strength"),
            value("maximum_speed"),
        ],
        width,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A requirement rounds a positive fraction up and truncates a negative one; whole
    /// quantities stay as they are.
    #[test]
    fn requirements_round_as_the_designer_shows_them() {
        assert_eq!(required_quantity(100 * 100_000), 100);
        assert_eq!(required_quantity(100 * 100_000 + 1), 101);
        assert_eq!(required_quantity(1), 1);
        assert_eq!(required_quantity(0), 0);
        assert_eq!(required_quantity(-150_000), -1);
        assert_eq!(required_quantity(-99_999), 0);
    }

    /// The requirement rows follow the fixed rows in the equipment database's order, each named
    /// by the country's own short name where it has one and rounded up; a zero quantity has no
    /// row; and manpower and training time stay shown when the equipment-dependent stats are
    /// unknown.
    #[test]
    fn requirement_rows_follow_the_fixed_rows() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let root = std::env::temp_dir().join(format!(
            "hoi4_designer_requirement_rows_{}",
            std::process::id()
        ));
        let file = root.join("localisation/english/designer_l_english.yml");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(
            &file,
            "\u{feff}l_english:\n GER_infantry_equipment_short:0 \"Gewehre\"\n \
             support_equipment_short:0 \"Support\"\n artillery_equipment:0 \"Guns\"\n \
             DESIGNER_MANPOWER:0 \"Manpower\"\n",
        )
        .unwrap();
        let layers =
            installed_content_core::ResolvedLayers::from_folders(&[(root.as_path(), &[][..])]).unwrap();
        let localisation = installed_content_localisation::localisation::read(&layers, "english").unwrap();
        let preview = Ok(Preview {
            block: Err("a critical type is short".into()),
            manpower: 1000,
            training_time: 90,
            requirements: vec![
                ("infantry_equipment".into(), 100 * 100_000),
                ("support_equipment".into(), 150_001),
                ("artillery_equipment".into(), 12 * 100_000),
                ("motorized_equipment".into(), 0),
            ],
        });
        let rows = rows(&localisation, &preview, false, Some("GER"));
        let extra: Vec<_> = rows[ROSTER.len()..]
            .iter()
            .map(|row| (row.label.clone().unwrap(), row.value.clone().unwrap()))
            .collect();
        assert_eq!(
            extra,
            [
                ("Gewehre".to_owned(), "100".to_owned()),
                ("Support".to_owned(), "2".to_owned()),
                ("Guns".to_owned(), "12".to_owned()),
            ]
        );
        let manpower = rows
            .iter()
            .find(|row| row.label.as_deref() == Ok("Manpower"))
            .unwrap();
        assert_eq!(manpower.value.as_deref(), Ok("1000"));
        let soft = ROSTER
            .iter()
            .position(|(_, operand, _)| *operand == Operand::Stat("soft_attack"))
            .unwrap();
        assert_eq!(rows[soft].value, Err("a critical type is short".into()));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Every roster stat is either proven or names what its proof lacks, never both, and an
    /// unproven row's reason names it.
    #[test]
    fn every_unproven_row_names_what_its_proof_lacks() {
        for (_, operand, _) in ROSTER {
            let Operand::Stat(stat) = operand else {
                continue;
            };
            let unproven_listed = UNPROVEN.iter().any(|(listed, _)| *listed == stat);
            assert_ne!(PROVEN.contains(&stat), unproven_listed, "{stat}");
            if unproven_listed {
                assert!(unproven(operand).contains(": "), "{stat}");
            }
        }
        assert!(
            UNPROVEN
                .iter()
                .all(|(stat, _)| ROSTER.iter().any(|(_, operand, _)| *operand == Operand::Stat(stat)))
        );
    }

    #[test]
    fn trimmed_drops_trailing_zeroes() {
        assert_eq!(trimmed(1_800_000), "18");
        assert_eq!(trimmed(1_850_000), "18.5");
        assert_eq!(trimmed(-12_345), "-0.12345");
        assert_eq!(trimmed(100), "0.001");
        assert_eq!(trimmed(0), "0");
    }
}
