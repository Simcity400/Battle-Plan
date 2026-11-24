use content_reader_layers::ResolvedLayers;
use content_reader_defines::defines::Defines;
use content_reader_military::leader_traits::LeaderTraits;
use content_reader_parser::tokens::Token;
use content_reader_key_names::unit_leader::{Adjuster, Component, Leaders, Skills, TraitEffects, TraitReference, Traits};
use content_reader_script::sub_units::{SubUnit, SubUnits};
use content_reader_localisation::{localisation::Argument, localisation::Localisation};
use replay_data::{
    rows::CharacterRoleTraitRow, rows::CharacterRow, rows::CountryRow,
    rows::LandCombatCommanderLearningTrait, rows::LandCombatCommanderTimedTrait,
    rows::LandCombatCommanderTrait, rows::LandCombatLearningProgress, rows::LandCombatSideInputs,
    rows::NavalCommanderInputs, rows::UnitLeaderRow, rows::tables,
};
use replay_frame::frame::DecodedFrame;
use std::collections::BTreeSet;
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, Mutex},
};
use dashboard_text::table_access::country_slot;
use viewer_foundation::tooltip_text::country_localised;
use viewer_foundation::tooltip_text::displayable;
use viewer_foundation::tooltip_text::ensure_newlines;
use viewer_foundation::tooltip_text::tooltip_keys;
use viewer_foundation::tooltip_text::viewing_tag as tooltip_text_viewing_tag;
use super::content::Content as StaticContent;
use viewer_native_gui::native_windows::NativeWindows;
use viewer_foundation::perspective::Vision;
use dashboard_text::table_access::rows;
use {
    super::{
        Selection, Values,
        binding::Bindings,
        modifier_text::{Block, Definitions, Refusal},
        tooltips::{hover_text, refuse_hovers, set_hover},
    },
    viewer_foundation::tooltip_text::{TextIcons, builtin, token},
};

#[derive(Debug)]
pub(super) struct Content {
    leaders: Arc<Leaders>,
    terrain: Vec<String>,
    settled: Mutex<Option<(Arc<super::modifier_text::NameIndex>, Arc<Leaders>)>>,
    advisor_ranks: Result<[i32; 3], String>,
    proximity: Result<[i32; 3], String>,
    sub_units: Result<Arc<SubUnits>, String>,
    leader_traits: Result<Arc<LeaderTraits>, String>,
}

fn int_defines(defines: &Defines, category: &str, names: [&str; 3]) -> Result<[i32; 3], String> {
    let mut values = [0; 3];
    for (value, name) in values.iter_mut().zip(names) {
        *value = defines
            .int32(category, name)
            .map_err(|error| format!("{category}.{name} is unavailable ({error})"))?;
    }
    Ok(values)
}

impl Content {
    pub(super) fn sub_units(&self) -> Result<&content_reader_script::sub_units::SubUnits, &str> {
        self.sub_units.as_deref().map_err(String::as_str)
    }
    pub(super) fn shared_sub_units(&self) -> Result<Arc<SubUnits>, String> {
        self.sub_units.clone()
    }

    pub(super) fn load(
        layers: &ResolvedLayers,
        defines: &Defines,
        sub_units: Result<SubUnits, String>,
        leader_traits: Result<Arc<LeaderTraits>, String>,
    ) -> io::Result<(Self, Skills)> {
        let (terrain, _) = content_reader_key_names::terrain::read(layers)?;
        let terrain: Vec<String> = terrain
            .categories
            .iter()
            .map(|category| String::from_utf8_lossy(&category.name).into_owned())
            .collect();
        let leaders = Leaders::load(layers, &|name| spelling(&terrain, name))?;
        let skills = leaders.skills.clone();
        let advisor_ranks = int_defines(
            defines,
            "NCharacter",
            [
                "SPECIALIST_ADVISOR_MIN_RANK",
                "EXPERT_ADVISOR_MIN_RANK",
                "GENIUS_ADVISOR_MIN_RANK",
            ],
        );
        let proximity = int_defines(
            defines,
            "NMilitary",
            [
                "GENERAL_PROXIMITY_DEFAULT",
                "GENERAL_PROXIMITY_CLOSE",
                "GENERAL_PROXIMITY_MEDIUM",
            ],
        );
        Ok((
            Self {
                leaders: Arc::new(leaders),
                terrain,
                settled: Mutex::new(None),
                advisor_ranks,
                proximity,
                sub_units: sub_units.map(Arc::new),
                leader_traits,
            },
            skills,
        ))
    }
}

fn spelling(terrain: &[String], name: &str) -> Option<String> {
    terrain
        .iter()
        .find(|known| known.eq_ignore_ascii_case(name))
        .cloned()
}

impl Content {
    pub(super) fn settled(&self, definitions: Result<&Definitions<'_>, &Refusal>) -> Arc<Leaders> {
        let Ok(definitions) = definitions else {
            return Arc::clone(&self.leaders);
        };
        if !self.leaders.conditional() {
            return Arc::clone(&self.leaders);
        }
        let identity = definitions.identity();
        let mut settled = self
            .settled
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some((known, leaders)) = settled.as_ref()
            && Arc::ptr_eq(known, &identity)
        {
            return Arc::clone(leaders);
        }
        let leaders = Arc::new(self.leaders.reread(
            &|name| spelling(&self.terrain, name),
            &|name| match definitions.index(&String::from_utf8_lossy(name)) {
                Ok(index) => Some(index.is_some()),
                Err(_) => None,
            },
        ));
        *settled = Some((identity, Arc::clone(&leaders)));
        leaders
    }

    pub(super) fn warnings(&self) -> Vec<String> {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for what in &self.leaders.traits.unsupported {
            *counts.entry(what).or_default() += 1;
        }
        counts
            .into_iter()
            .map(|(what, count)| {
                format!("Commander trait content: {count} malformed entries are refused ({what}).")
            })
            .chain(self.leaders.misreads.iter().map(|what| {
                format!("Commander trait content: {what}; the hovers follow the game's reading.")
            }))
            .chain(
                self.sub_units
                    .iter()
                    .flat_map(|sub_units| sub_units.warnings().iter().cloned()),
            )
            .collect()
    }
}

fn set(values: &mut Values, key: impl Into<String>, text: Option<(String, String)>) {
    let (text, description) = match text {
        Some((text, description)) => (hover_text(&text), hover_text(&description)),
        None => (None, None),
    };
    set_hover(values, key, text, description);
}

pub(super) fn country_localized(
    icons: &dyn TextIcons,
    loc: &Localisation,
    tag: Result<&str, &str>,
    key: &str,
) -> Result<String, String> {
    let text = country_localised(loc, tag, key)?;
    displayable(icons, text).map_err(|reason| format!("localisation {key} holds {reason}"))
}

fn ratio(experience: i64, required: i32) -> Option<i64> {
    super::fixed::fraction(experience, i64::from(required) * super::fixed::ONE)
}

#[derive(Clone, Copy)]
pub(super) enum Learning<'a> {
    Raw(&'a [LandCombatCommanderLearningTrait], i64),
    Shown(&'a [LandCombatLearningProgress]),
}

fn learning_row(
    lines: &mut String,
    icons: &dyn TextIcons,
    loc: &Localisation,
    tag: Result<&str, &str>,
    token: &[u8],
    progress: i64,
) -> Result<(), String> {
    let token = std::str::from_utf8(token).map_err(|_| {
        format!(
            "learning trait token {} is not UTF-8",
            String::from_utf8_lossy(token)
        )
    })?;
    let name = country_localized(icons, loc, tag, token)?;
    lines.push_str(&builtin(
        icons,
        loc,
        LEARNING_TRAITS,
        &[
            ("NAME", Argument::Text(&name)),
            ("PERC", Argument::Fixed5(progress)),
        ],
    )?);
    lines.push('\n');
    Ok(())
}

fn learning(
    icons: &dyn TextIcons,
    loc: &Localisation,
    tag: Result<&str, &str>,
    traits: Learning<'_>,
) -> Result<String, String> {
    let mut lines = String::new();
    match traits {
        Learning::Raw(traits, threshold) => {
            for item in traits {
                let progress =
                    ratio(item.experience, item.required_experience).ok_or_else(|| {
                        format!(
                            "learning trait {}'s progress fraction: {}",
                            String::from_utf8_lossy(&item.token),
                            super::fixed::TRAP
                        )
                    })?;
                if progress <= threshold {
                    continue;
                }
                learning_row(&mut lines, icons, loc, tag, &item.token, progress)?;
            }
        }
        Learning::Shown(traits) => {
            for item in traits {
                learning_row(&mut lines, icons, loc, tag, &item.token, item.progress)?;
            }
        }
    }
    if lines.is_empty() {
        return Ok(lines);
    }
    let title = builtin(icons, loc, LEARNING_TRAITS_TITLE, &[])?;
    Ok(format!(
        "\n{}{lines}",
        builtin(
            icons,
            loc,
            super::tooltips::HEADER,
            &[("KEY", Argument::Text(&title))]
        )?
    ))
}

pub(super) fn portrait(
    icons: &dyn TextIcons,
    loc: &Localisation,
    name: &[u8],
    description: &[u8],
    role: (i32, i32),
    progress: Option<(Learning<'_>, Result<&str, &str>)>,
    initiative: bool,
) -> Result<(String, String), String> {
    let text = portrait_text(icons, loc, name, role, progress, initiative)?;
    let description = token(icons, loc, description)
        .map_err(|refusal| format!("the commander's description: {refusal}"))?;
    Ok((text, description))
}

fn portrait_text(
    icons: &dyn TextIcons,
    loc: &Localisation,
    name: &[u8],
    role: (i32, i32),
    progress: Option<(Learning<'_>, Result<&str, &str>)>,
    initiative: bool,
) -> Result<String, String> {
    let name =
        token(icons, loc, name).map_err(|refusal| format!("the commander's name: {refusal}"))?;
    let title = role_title(icons, loc, role.0, role.1)?;
    let name = format!("{title} {name}");
    let mut text = builtin(
        icons,
        loc,
        super::tooltips::HEADER,
        &[("KEY", Argument::Text(&name))],
    )?;
    if let Some((traits, tag)) = progress {
        text.push_str(&learning(icons, loc, tag, traits)?);
    }
    if initiative {
        text.push('\n');
        text.push_str(&builtin(icons, loc, LEADER_HAS_INITIATIVE, &[])?);
    }
    Ok(text)
}

const LAND_LEARNING_UNRECORDED: &str = "the viewing country's commander shows its learning-trait progress (learning rows), which this recording does not record";

const LAND_LEARNING_UNREAD: &str = "the viewing country's commander shows its learning-trait progress above the live LEADER_TRAITS_XP_SHOW, which could not be read at capture";

const LAND_DESCRIPTION_UNRECORDED: &str =
    "the commander's description token is not recorded in this recording";

const LAND_TIMED_TRAITS_UNRECORDED: &str =
    "LandCombatDetails.side_inputs (timed traits) not recorded";

#[expect(clippy::too_many_arguments, reason = "the text resources and each recorded fact of one commander's portrait")]
fn land_portrait(
    icons: &dyn TextIcons,
    loc: &Localisation,
    name: &[u8],
    role: (i32, i32),
    own_commander: Result<bool, String>,
    learning: Result<&[LandCombatLearningProgress], &str>,
    tag: Result<&str, &str>,
    initiative: bool,
) -> Result<String, String> {
    match (own_commander, learning) {
        (Ok(false), _) => portrait_text(icons, loc, name, role, None, initiative),
        (Ok(true), Ok(traits)) => portrait_text(
            icons,
            loc,
            name,
            role,
            Some((Learning::Shown(traits), tag)),
            initiative,
        ),
        (Err(_), Ok([])) => portrait_text(icons, loc, name, role, None, initiative),
        (Ok(true), Err(reason)) => Err(reason.to_owned()),
        (Err(reason), _) => Err(format!(
            "whether the commander is the viewing country's, whose commander alone shows the learning-trait progress, is unknown: {reason}"
        )),
    }
}

fn role_title(
    icons: &dyn TextIcons,
    loc: &Localisation,
    category: i32,
    skill: i32,
) -> Result<String, String> {
    let key = match category {
        0 => FM_LEADER,
        1 => CC_LEADER,
        2 => NAVY_LEADER,
        3 => {
            let key = format!("operative_skill_{skill}");
            if loc.get(&key).is_some() {
                return builtin(icons, loc, &key, &[]);
            }
            OPERATIVE
        }
        _ => return Ok(String::new()),
    };
    builtin(icons, loc, key, &[])
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Dropped {
    Modifier(String),
    SubUnit(String),
    Repeat(String),
}

impl std::fmt::Display for Dropped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Dropped::Modifier(name) => write!(
                f,
                "the modifier {name}, which no recorded modifier definition carries (the game reports and drops it)"
            ),
            Dropped::SubUnit(key) => write!(
                f,
                "the sub-unit {key}, which the sub-unit database lacks (the game reports and drops it)"
            ),
            Dropped::Repeat(key) => write!(
                f,
                "the sub-unit {key} a second time, which the game drops as already specified"
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum EffectRefusal {
    Content(String),
    OutOfStep(String),
    Modifier(Refusal),
    SubUnitOrder(String),
    Localisation(String),
    ProximityDefines(String),
}

impl std::fmt::Display for EffectRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EffectRefusal::Content(what) => write!(f, "unsupported content ({what})"),
            EffectRefusal::OutOfStep(reason) => {
                write!(
                    f,
                    "the game's reading of the content is unknown here: {reason}"
                )
            }
            EffectRefusal::Modifier(refusal) => write!(f, "{refusal}"),
            EffectRefusal::SubUnitOrder(reason) => {
                write!(f, "the sub-unit order sub_unit_modifiers needs ({reason})")
            }
            EffectRefusal::Localisation(reason) => f.write_str(reason),
            EffectRefusal::ProximityDefines(reason) => {
                write!(f, "a proximity offset needs {reason}")
            }
        }
    }
}

fn text(icons: &dyn TextIcons, loc: &Localisation, key: &str) -> Result<String, EffectRefusal> {
    token(icons, loc, key.as_bytes()).map_err(EffectRefusal::Localisation)
}

fn named(
    icons: &dyn TextIcons,
    loc: &Localisation,
    key: &str,
    args: &[(&str, Argument<'_>)],
) -> Result<String, EffectRefusal> {
    builtin(icons, loc, key, args).map_err(EffectRefusal::Localisation)
}

fn adjuster(
    icons: &dyn TextIcons,
    loc: &Localisation,
    adjuster: &Adjuster,
) -> Result<String, EffectRefusal> {
    if adjuster.attack == 0 && adjuster.defence == 0 && adjuster.movement == 0 {
        return Ok(String::new());
    }
    let mut out = format!("\u{11}H{}\u{11}!", text(icons, loc, &adjuster.key)?);
    out.push('\n');
    out.push_str(&adjuster_lines(icons, loc, adjuster)?);
    Ok(out)
}

fn adjuster_lines(
    icons: &dyn TextIcons,
    loc: &Localisation,
    adjuster: &Adjuster,
) -> Result<String, EffectRefusal> {
    let mut out = String::new();
    for (stat, value) in [
        ("MOVEMENT", adjuster.movement),
        ("ATTACK", adjuster.attack),
        ("DEFENCE", adjuster.defence),
    ] {
        if value == 0 {
            continue;
        }
        let key = format!("STAT_ADJUSTER_{stat}");
        out.push_str(&named(icons, loc, &key, &[])?);
        let diff = format!("{key}_DIFF");
        out.push_str(&named(
            icons,
            loc,
            &diff,
            &[("VALUE", Argument::Fixed5(value))],
        )?);
        out.push('\n');
    }
    Ok(out)
}

struct EffectContext<'a> {
    icons: &'a dyn TextIcons,
    loc: &'a Localisation,
    definitions: Result<&'a Definitions<'a>, &'a Refusal>,
    traits: &'a Traits,
    proximity: Result<[i32; 3], &'a str>,
    viewing_tag: Result<&'a str, &'a str>,
    sub_units: Result<&'a SubUnits, &'a str>,
}

const SUB_UNIT_INDENT: &str = "  ";

fn sub_units(
    context: &EffectContext<'_>,
    definition: &TraitEffects,
    dropped: &mut Vec<Dropped>,
) -> Result<String, EffectRefusal> {
    if definition.sub_units.is_empty() {
        return Ok(String::new());
    }
    let order = context
        .sub_units
        .map_err(|reason| EffectRefusal::SubUnitOrder(reason.into()))?;
    let definitions = context
        .definitions
        .map_err(|refusal| EffectRefusal::Modifier(refusal.clone()))?;
    let mut adjusters: BTreeMap<u32, &Adjuster> = BTreeMap::new();
    let mut blocks: BTreeMap<u32, Block> = BTreeMap::new();
    for entry in &definition.sub_units {
        let key = &entry.adjuster.key;
        let index = match order.get(key) {
            SubUnit::Index(index) => index,
            SubUnit::Unknown => {
                dropped.push(Dropped::SubUnit(key.clone()));
                continue;
            }
            SubUnit::Uncertain(reason) => {
                return Err(EffectRefusal::SubUnitOrder(format!(
                    "sub-unit {key}'s index is unknown: {reason}"
                )));
            }
        };
        if adjusters.contains_key(&index) || blocks.contains_key(&index) {
            dropped.push(Dropped::Repeat(key.clone()));
            continue;
        }
        let adjuster = &entry.adjuster;
        if adjuster.attack != 0 || adjuster.defence != 0 || adjuster.movement != 0 {
            adjusters.insert(index, adjuster);
        }
        let (mut block, names) =
            Block::build(definitions, &entry.block).map_err(EffectRefusal::Modifier)?;
        dropped.extend(names.into_iter().map(Dropped::Modifier));
        block.custom_tooltips = entry
            .block
            .tooltip_table()
            .copied()
            .copied()
            .keys()
            .map(|key| key.as_bytes().to_vec())
            .collect();
        if block.kept(definitions).map_err(EffectRefusal::Modifier)? {
            blocks.insert(index, block);
        }
    }
    let name = |index: u32| -> Result<String, EffectRefusal> {
        let key = order
            .name(index)
            .expect("indices come from the database")
            .map_err(EffectRefusal::Content)?;
        let name = country_localized(context.icons, context.loc, context.viewing_tag, key)
            .map_err(EffectRefusal::Localisation)?;
        Ok(format!("\u{11}H{name}\u{11}!\n"))
    };
    let mut out = String::new();
    let indices: BTreeSet<u32> = adjusters.keys().chain(blocks.keys()).copied().collect();
    for index in indices {
        ensure_newlines(&mut out, 1);
        out.push_str(&name(index)?);
        if let Some(adjuster) = adjusters.get(&index) {
            out.push_str(&adjuster_lines(context.icons, context.loc, adjuster)?);
        }
        if let Some(block) = blocks.get(&index) {
            out.push_str(
                &block
                    .describe(
                        definitions,
                        context.icons,
                        context.loc,
                        super::fixed::ONE,
                        SUB_UNIT_INDENT,
                        u32::MAX,
                    )
                    .map_err(EffectRefusal::Modifier)?,
            );
        }
    }
    Ok(out)
}

fn effects(
    context: &EffectContext<'_>,
    definition: &TraitEffects,
    dropped: &mut Vec<Dropped>,
) -> Result<String, EffectRefusal> {
    let (icons, loc) = (context.icons, context.loc);
    let mut out = String::new();
    if let Some(what) = definition.unsupported.first() {
        return Err(if what.starts_with("the game's parse of ") {
            EffectRefusal::OutOfStep(what.clone())
        } else {
            EffectRefusal::Content(what.clone())
        });
    }
    Definitions::settle(context.definitions, &definition.conditions)
        .map_err(EffectRefusal::OutOfStep)?;
    if !definition.override_tooltips.is_empty() {
        for token in &definition.override_tooltips {
            out.push_str(&text(icons, loc, token)?);
            out.push('\n');
        }
        return Ok(out);
    }
    let authored = definition.blocks.iter().any(|block| {
        !block.values.is_empty() || !block.tooltips().is_empty() || !block.children.is_empty()
    });
    let definitions = if authored {
        Some(
            context
                .definitions
                .map_err(|refusal| EffectRefusal::Modifier(refusal.clone()))?,
        )
    } else {
        None
    };
    for (index, authored) in definition.blocks.iter().enumerate() {
        if let Some(definitions) = definitions {
            let (block, names) =
                Block::build(definitions, authored).map_err(EffectRefusal::Modifier)?;
            dropped.extend(names.into_iter().map(Dropped::Modifier));
            out.push_str(
                &block
                    .describe(definitions, icons, loc, super::fixed::ONE, "", u32::MAX)
                    .map_err(EffectRefusal::Modifier)?,
            );
        }
        if index < 3 {
            ensure_newlines(&mut out, 1);
        }
    }
    for item in &definition.adjusters {
        ensure_newlines(&mut out, 1);
        out.push_str(&adjuster(icons, loc, item)?);
    }
    out.push_str(&sub_units(context, definition, dropped)?);
    let [
        attack,
        defense,
        logistics,
        planning,
        maneuvering,
        coordination,
    ] = definition.skills;
    if attack != 0
        && attack == defense
        && ((attack == logistics && attack == planning)
            || (attack == maneuvering && attack == coordination))
    {
        ensure_newlines(&mut out, 1);
        out.push_str(&named(
            icons,
            loc,
            "trait_bonus_all",
            &[("VAL", Argument::Int32(attack))],
        )?);
    } else {
        for (key, value) in [
            ("trait_bonus_attack", attack),
            ("trait_bonus_defense", defense),
            ("trait_bonus_planning", planning),
            ("trait_bonus_logistics", logistics),
            ("trait_bonus_maneuvering", maneuvering),
            ("trait_bonus_coordination", coordination),
        ] {
            if value == 0 {
                continue;
            }
            ensure_newlines(&mut out, 1);
            out.push_str(&named(icons, loc, key, &[("VAL", Argument::Int32(value))])?);
        }
    }
    if definition.proximity_offset != 0 {
        let [default, close, medium] = context
            .proximity
            .map_err(|reason| EffectRefusal::ProximityDefines(reason.into()))?;
        let distance = definition.proximity_offset.wrapping_add(default);
        let title = if distance < 1 {
            LEADER_PROXIMITY_FRONTLINE_TITLE
        } else if distance <= close {
            LEADER_PROXIMITY_CLOSE_TITLE
        } else if distance <= medium {
            LEADER_PROXIMITY_MEDIUM_TITLE
        } else {
            LEADER_PROXIMITY_FAR_TITLE
        };
        let title = named(icons, loc, title, &[])?;
        ensure_newlines(&mut out, 1);
        let key = "trait_bonus_leader_default_proximity_offset";
        out.push_str(&named(
            icons,
            loc,
            key,
            &[("NAME", Argument::Text(&title))],
        )?);
    }
    for (token, value) in &definition.experience_factors {
        ensure_newlines(&mut out, 1);
        let name = country_localized(icons, loc, context.viewing_tag, token)
            .map_err(EffectRefusal::Localisation)?;
        out.push_str(&named(
            icons,
            loc,
            "trait_xp_factor",
            &[
                ("NAME", Argument::Text(&name)),
                ("VAL", Argument::Fixed5(*value)),
            ],
        )?);
    }
    ensure_newlines(&mut out, 1);
    Ok(out)
}

#[derive(Clone, Copy)]
struct AdvisorTrait<'a> {
    token: &'a str,
    ordinal: u32,
}

fn advisor_trait<'a>(
    reference: Option<&TraitReference>,
    traits: Result<&'a LeaderTraits, &str>,
) -> Result<Option<AdvisorTrait<'a>>, String> {
    let name = match reference {
        None | Some(TraitReference::Null(_)) => return Ok(None),
        Some(TraitReference::Name(name)) => name,
    };
    let traits = traits.map_err(|reason| {
        format!("advisor trait {name}: the country-leader traits are unavailable ({reason})")
    })?;
    match traits.by_token(&Token::c_str(name.as_bytes())) {
        Ok(Some(found)) => {
            let ordinal = traits
                .first_ordinal(&found.token)
                .map_err(|gap| {
                    format!("advisor trait {name}'s definition ordinal is unknown ({gap})")
                })?
                .ok_or_else(|| format!("advisor trait {name} has no content definition ordinal"))?;
            Ok(Some(AdvisorTrait {
                token: &found.token,
                ordinal,
            }))
        }
        Ok(None) => Ok(None),
        Err(gap) => Err(format!(
            "advisor trait {name}: whether it is a country-leader trait is unknown ({gap})"
        )),
    }
}

fn selected_advisor_trait<'a>(
    definition: &TraitEffects,
    skill: i32,
    [specialist, expert, genius]: [i32; 3],
    traits: Result<&'a LeaderTraits, &str>,
    roles: Result<&[&CharacterRoleTraitRow], &str>,
) -> Result<(&'a str, Option<i32>), String> {
    let resolved = |slot: usize| advisor_trait(definition.advisor_traits[slot].as_ref(), traits);
    let specialist_trait = || {
        resolved(0)?.ok_or_else(|| {
            "the trait's specialist_advisor_trait is null (never read, or naming no country-leader trait), which the game falls back to"
                .to_owned()
        })
    };
    let minimum_rank = (skill < specialist).then_some(specialist);
    let selected = if skill >= genius {
        resolved(2)?
    } else if skill < expert && skill < specialist {
        None
    } else {
        let roles = roles.map_err(str::to_owned)?;
        let genius_trait = resolved(2)?;
        let expert_trait = resolved(1)?;
        let default = if skill >= expert { expert_trait } else { None };
        roles
            .iter()
            .find_map(|role| {
                genius_trait
                    .filter(|found| role.definitions.contains(&found.ordinal))
                    .or_else(|| {
                        expert_trait.filter(|found| {
                            skill < expert && role.definitions.contains(&found.ordinal)
                        })
                    })
            })
            .or(default)
    };
    let selected = match selected {
        Some(found) => found,
        None => specialist_trait()?,
    };
    Ok((selected.token, minimum_rank))
}

fn advisor_line(
    icons: &dyn TextIcons,
    loc: &Localisation,
    definition: &TraitEffects,
    trait_token: &str,
    minimum_rank: Option<i32>,
) -> Result<String, String> {
    let slot_key = definition
        .advisor_slot
        .as_deref()
        .ok_or_else(|| "the trait names no advisor slot".to_owned())?;
    let slot = token(icons, loc, slot_key.as_bytes())?;
    let name = token(icons, loc, trait_token.as_bytes())?;
    let mut text = builtin(
        icons,
        loc,
        TRAIT_UNLOCKS_ADVISOR_ROLE,
        &[
            ("ADVISORSLOT", Argument::Text(&slot)),
            ("ADVISORNAME", Argument::Text(&name)),
        ],
    )?;
    if let Some(rank) = minimum_rank {
        text.push_str(&builtin(
            icons,
            loc,
            TRAIT_UNLOCKED_BY_MIN_RANK,
            &[("RANK", Argument::Int32(rank))],
        )?);
        text.push('\n');
    }
    Ok(text)
}

fn timed_days(
    timed: &[LandCombatCommanderTimedTrait],
    item: &LandCombatCommanderTrait,
) -> Option<i32> {
    timed
        .iter()
        .find(|row| {
            row.token_id == item.token_id && row.definition_ordinal == item.definition_ordinal
        })
        .and_then(|row| (row.remaining_days >= 0).then_some(row.remaining_days))
}

struct AssignedCommander<'a> {
    traits: &'a [LandCombatCommanderTrait],
    context: i32,
    timed_traits: Result<&'a [LandCombatCommanderTimedTrait], String>,
    country_tag: Result<&'a str, &'a str>,
    advisor_feature: Option<bool>,
    skill: Option<i32>,
    advisor_roles: Result<Vec<&'a CharacterRoleTraitRow>, String>,
}

fn assigned(
    out: &mut Bindings,
    content: &Content,
    windows: &NativeWindows,
    key: &str,
    commander: AssignedCommander<'_>,
    effect_context: &EffectContext<'_>,
) {
    let Some(grid) = out.values.get_mut(key) else {
        out.warnings.push(format!(
            "Combat trait hover {key}: the trait grid is not bound; its hovers are omitted."
        ));
        return;
    };
    let (icons, loc) = (effect_context.icons, effect_context.loc);
    let mut refused: BTreeMap<String, usize> = BTreeMap::new();
    let mut dropped: BTreeMap<String, usize> = BTreeMap::new();
    let mut ignored: BTreeMap<String, usize> = BTreeMap::new();
    let mut advisor: BTreeMap<String, usize> = BTreeMap::new();
    let mut absent: BTreeMap<String, usize> = BTreeMap::new();
    let shown: Vec<_> = commander
        .traits
        .iter()
        .filter(|item| item.kind != 0 && item.relevant != 0)
        .collect();
    let entry_count = grid.entries.len();
    for (index, (entry, &item)) in grid.entries.iter_mut().zip(&shown).enumerate() {
        let label = String::from_utf8_lossy(&item.token).into_owned();
        let result = (|| -> Result<(String, String), String> {
            let timed = commander
                .timed_traits
                .as_ref()
                .copied()
                .map_err(Clone::clone)?;
            if item.kind == 6 && loc.get(super::tooltips::HEADER).is_none() {
                return Err(format!(
                    "native exile tooltip key {} is not defined in the content localisation",
                    super::tooltips::HEADER
                ));
            }
            let trait_token = std::str::from_utf8(&item.token)
                .map_err(|_| format!("recorded trait token {label} is not UTF-8"))?;
            let name = country_localized(icons, loc, commander.country_tag, trait_token)
                .map_err(|reason| format!("trait {trait_token}'s name: {reason}"))?;
            let mut text = builtin(
                icons,
                loc,
                super::tooltips::HEADER,
                &[("KEY", Argument::Text(&name))],
            )?;
            let definition = effect_context.traits.get(trait_token).ok_or_else(|| {
                format!("trait {trait_token} is not in the content's leader traits")
            })?;
            for what in &definition.ignored {
                *ignored
                    .entry(format!("trait {trait_token} has {what}"))
                    .or_default() += 1;
            }
            text.push('\n');
            let mut names = Vec::new();
            match effects(effect_context, definition, &mut names) {
                Ok(lines) => text.push_str(&lines),
                Err(refusal) => {
                    *refused
                        .entry(format!(
                            "trait {trait_token}'s effect lines are refused: {refusal}"
                        ))
                        .or_default() += 1;
                    return Err(String::new());
                }
            }
            for name in names {
                *dropped
                    .entry(format!("trait {trait_token} authors {name}"))
                    .or_default() += 1;
            }
            ensure_newlines(&mut text, 1);
            for key in &definition.custom_tooltips {
                text.push_str(&token(icons, loc, key.as_bytes())?);
                text.push('\n');
            }
            if commander.context != 0 && commander.context != 4 && definition.army_only {
                text.push_str(&builtin(
                    icons,
                    loc,
                    "TRAIT_NOT_ACTIVE_WHEN_COMMANDING_AN_ARMY",
                    &[],
                )?);
                text.push('\n');
            }
            if definition.advisor_slot.is_some() && commander.advisor_feature != Some(false) {
                match (
                    commander.advisor_feature,
                    commander.skill,
                    &content.advisor_ranks,
                ) {
                    (Some(true), Some(skill), Ok(ranks)) => {
                        let line = selected_advisor_trait(
                            definition,
                            skill,
                            *ranks,
                            content.leader_traits.as_deref().map_err(String::as_str),
                            commander.advisor_roles.as_deref().map_err(String::as_str),
                        )
                        .and_then(|(advisor_trait, minimum_rank)| {
                            advisor_line(icons, loc, definition, advisor_trait, minimum_rank)
                        });
                        match line {
                            Ok(line) => {
                                ensure_newlines(&mut text, 2);
                                text.push_str(&line);
                            }
                            Err(reason) => {
                                *advisor
                                    .entry(format!("trait {trait_token}: {reason}"))
                                    .or_default() += 1
                            }
                        }
                    }
                    (feature, skill, ranks) => {
                        let reason = if feature.is_none() {
                            "the simulation DLC mask is unavailable".to_owned()
                        } else if skill.is_none() {
                            "the leader's UnitLeaders skill row is unavailable".to_owned()
                        } else if let Err(reason) = ranks {
                            format!("the advisor rank define {reason}")
                        } else {
                            "the advisor feature is not enabled".to_owned()
                        };
                        *advisor
                            .entry(format!("trait {trait_token}: {reason}"))
                            .or_default() += 1;
                    }
                }
            }
            let description_key = format!("{trait_token}_desc");
            let mut description =
                country_localized(icons, loc, commander.country_tag, &description_key)?;
            if let Some(days) = timed_days(timed, item) {
                description.push('\n');
                description.push_str(&builtin(
                    icons,
                    loc,
                    TRAIT_WILL_EXPIRE,
                    &[("DAYS", Argument::Int32(days))],
                )?);
            }
            Ok((text, description))
        })();
        let result = match result {
            Ok(result) => Some(result),
            Err(reason) => {
                if !reason.is_empty() {
                    *absent
                        .entry(format!("trait {label}: {reason}"))
                        .or_default() += 1;
                }
                None
            }
        };
        if item.kind == 6 {
            let template = grid
                .entry_templates
                .get(&index)
                .or(grid.entry_template.as_ref());
            if let Some(template) = template
                && let Some(root) = windows.gui_type(template.as_bytes())
            {
                let mut pending = vec![(String::new(), root)];
                while let Some((path, element)) = pending.pop() {
                    for child in element.nested() {
                        let child_path = if path.is_empty() {
                            child.name().into_owned()
                        } else {
                            format!("{path}/{}", child.name())
                        };
                        pending.push((child_path, child));
                    }
                    set(entry, path, result.clone());
                }
            } else {
                let keys: Vec<_> = entry.keys().cloned().collect();
                refuse_hovers(entry, keys);
                *absent
                    .entry(format!(
                        "trait {label}: exile entry template {} is not in the combat interface files",
                        template.map_or("<unbound>", String::as_str)
                    ))
                    .or_default() += 1;
            }
        } else {
            set(entry, "Background", result);
        }
    }
    if entry_count != shown.len() {
        out.warnings.push(format!(
            "Combat trait hover {key}: the grid has {entry_count} entries for {} shown traits; the unmatched ones have no hover.",
            shown.len()
        ));
    }
    for (what, count) in refused {
        out.warnings
            .push(format!("Combat trait hover {key}: {what} ({count} shown)."));
    }
    for (what, count) in dropped {
        out.warnings.push(format!(
            "Combat trait hover {key}: {what}; the hover omits it as well ({count} shown)."
        ));
    }
    for (what, count) in ignored {
        out.warnings.push(format!(
            "Combat trait hover {key}: {what}; the game's reader ignores it, and so does the hover ({count} shown)."
        ));
    }
    for (what, count) in advisor {
        out.warnings.push(format!(
            "Combat trait hover {key}: the advisor unlock line of {what} is omitted ({count} shown)."
        ));
    }
    for (what, count) in absent {
        for _ in 0..count {
            out.refuse(format!(
                "Combat trait hover {key}: the hover of {what} is omitted"
            ));
        }
    }
}

fn country_tag(countries: &[CountryRow], country: u32) -> Result<&str, &'static str> {
    let row = countries
        .iter()
        .find(|row| row.id == country)
        .ok_or("the commander's country has no Countries row")?;
    std::str::from_utf8(&row.tag).map_err(|_| "the commander's country tag is not UTF-8")
}

fn unit_leader(leaders: &[UnitLeaderRow], reference: u64) -> Option<&UnitLeaderRow> {
    leaders
        .iter()
        .find(|row| ((u64::from(row.id) << 32) | u64::from(row.reference_type)) == reference)
}

fn advisor_roles<'a>(
    leader: Option<&UnitLeaderRow>,
    characters: Result<&[CharacterRow], String>,
    traits: Result<&'a [CharacterRoleTraitRow], String>,
) -> Result<Vec<&'a CharacterRoleTraitRow>, String> {
    let leader = leader.ok_or("the leader's UnitLeaders row is unavailable")?;
    if leader.character_type == 0 && leader.character_id == 0 {
        return Err(
            "the leader's character reference is null, which the advisor-role search dereferences"
                .into(),
        );
    }
    let characters = characters?;
    if !characters.iter().any(|row| row.id == leader.character_id) {
        return Err(format!(
            "the leader's character {} has no Characters row, so character lookup is unresolved",
            leader.character_id
        ));
    }
    let mut roles: Vec<_> = traits?
        .iter()
        .filter(|row| {
            row.character == leader.character_id && row.kind == CharacterRoleTraitRow::ADVISOR
        })
        .collect();
    roles.sort_by_key(|row| row.role);
    Ok(roles)
}

pub(super) struct BindContent<'a> {
    pub icons: &'a dyn TextIcons,
    pub loc: &'a Localisation,
    pub commanders: &'a Content,
    pub windows: &'a NativeWindows,
    pub modifier_names: &'a super::modifier_text::NameIndexCache,
}

pub(super) fn bind(
    out: &mut Bindings,
    content: BindContent<'_>,
    frame: &DecodedFrame,
    selection: Selection,
    vision: &Vision,
) -> io::Result<()> {
    let (icons, loc) = (content.icons, content.loc);
    let country_rows = rows::<tables::Countries>(frame)?;
    let countries = country_rows.rows();
    let links = rows::<tables::CountryLinks>(frame)?;
    let viewing_tag = tooltip_text_viewing_tag(countries, vision);
    let leaders = rows::<tables::UnitLeaders>(frame)?;
    let features = rows::<tables::SimulationFeatures>(frame)?;
    let advisor_feature = if features.is_complete() {
        match features.rows() {
            [feature] => Some(feature.dlc_mask & 0x80 != 0),
            _ => None,
        }
    } else {
        None
    };
    let definitions = Definitions::read(frame, content.modifier_names)?;
    let read = content.commanders.settled(definitions.as_ref());
    let effect_context = EffectContext {
        icons,
        loc,
        definitions: definitions.as_ref(),
        traits: &read.traits,
        proximity: content
            .commanders
            .proximity
            .as_ref()
            .copied()
            .map_err(String::as_str),
        viewing_tag,
        sub_units: content
            .commanders
            .sub_units
            .as_deref()
            .map_err(String::as_str),
    };
    let naval =
        |out: &mut Bindings, key: &str, commander: &replay_data::rows::NavalCommanderInputs| {
            if commander.commander == 0 {
                return;
            }
            let leader = unit_leader(leaders.rows(), commander.commander);
            assigned(
                out,
                content.commanders,
                content.windows,
                key,
                AssignedCommander {
                    traits: &commander.traits,
                    context: 2,
                    timed_traits: commander
                        .timed_traits
                        .require("NavalCommanderInputs.timed_traits")
                        .map(Vec::as_slice),
                    country_tag: country_tag(countries, commander.country),
                    advisor_feature,
                    skill: leader
                        .filter(|row| row.skill_status != 0)
                        .map(|row| row.skill_level),
                    advisor_roles: advisor_roles(
                        leader,
                        frame
                            .complete::<tables::Characters>()
                            .map_err(|error| error.to_string()),
                        frame
                            .complete::<tables::CharacterRoleTraits>()
                            .map_err(|error| error.to_string()),
                    ),
                },
                &effect_context,
            );
        };
    let id = match selection {
        Selection::AirRegion(_) | Selection::State(_) | Selection::NavalRegion(_) => return Ok(()),
        Selection::Naval(id) => {
            let details = rows::<tables::NavalCombatDetails>(frame)?;
            if let Some(detail) = details.rows().iter().find(|row| row.combat == id) {
                for (side, prefix) in detail.sides.iter().zip(["attacker", "defender"]) {
                    if let Some(commander) = &side.commander_inputs {
                        naval(
                            out,
                            &format!("header_container/traits_grid_{prefix}"),
                            commander,
                        );
                    }
                }
            }
            return Ok(());
        }
        Selection::NavalResult(id) => {
            let details = rows::<tables::NavalCombatResultDetails>(frame)?;
            if let Some(detail) = details.rows().iter().find(|row| row.result == id) {
                for (side, prefix) in detail.sides.iter().zip(["attacker", "defender"]) {
                    if let Some(commander) = &side.commander_context {
                        naval(out, &format!("{prefix}_leader/traits_grid"), commander);
                    }
                }
            }
            return Ok(());
        }
        Selection::Land(id) => id,
    };
    let details = rows::<tables::LandCombatDetails>(frame)?;
    let Some(detail) = details.rows().iter().find(|row| row.combat == id) else {
        out.warnings.push(format!(
            "Land combat {id}: no LandCombatDetails row, so the commander portrait and trait hovers are omitted."
        ));
        refuse_hovers(
            &mut out.values,
            ["land_battle_leader_frame", "land_battle_leader_frame2"],
        );
        return Ok(());
    };
    let side_inputs = detail.side_inputs.require("LandCombatDetails.side_inputs");
    for (index, prefix) in ["attacker", "defender"].into_iter().enumerate() {
        let side = &detail.sides[index];
        let inputs: Result<&LandCombatSideInputs, &str> = side_inputs
            .as_ref()
            .map(|sides| &sides[index])
            .map_err(String::as_str);
        let frame_key = if index == 0 {
            "land_battle_leader_frame"
        } else {
            "land_battle_leader_frame2"
        };
        if side.commander == 0 {
            refuse_hovers(&mut out.values, [frame_key]);
            continue;
        }
        let glow = format!("{prefix}_leader_initiative_overlay_glow");
        let initiative = out.values.get(&glow).is_some_and(|widget| !widget.hidden);
        let own_commander =
            country_slot(&links, side.commander_country).map(|slot| slot == Some(vision.ours()));
        let learning = inputs
            .map_err(|reason| format!("{LAND_LEARNING_UNRECORDED}: {reason}"))
            .and_then(|inputs| {
                inputs
                    .learning_traits
                    .as_deref()
                    .ok_or_else(|| LAND_LEARNING_UNREAD.to_owned())
            });
        let text = match land_portrait(
            icons,
            loc,
            &side.commander_name_token,
            (side.commander_category, side.commander_skill),
            own_commander,
            learning.as_deref().map_err(String::as_str),
            viewing_tag,
            initiative,
        ) {
            Ok(text) => hover_text(&text),
            Err(reason) => {
                out.warnings.push(format!(
                    "Land combat {id} {prefix} commander portrait hover is omitted: {reason}."
                ));
                None
            }
        };
        let description = text.as_ref().and_then(|_| {
            let description = inputs
                .map_err(|reason| format!("{LAND_DESCRIPTION_UNRECORDED}: {reason}"))
                .and_then(|inputs| {
                    token(icons, loc, &inputs.description_token)
                        .map_err(|refusal| format!("the commander's description: {refusal}"))
                });
            match description {
                Ok(description) => hover_text(&description),
                Err(reason) => {
                    out.refuse(format!(
                        "Land combat {id} {prefix} commander portrait hover's delayed description is omitted: {reason}"
                    ));
                    None
                }
            }
        });
        set_hover(
            &mut out.values,
            frame_key,
            text.clone(),
            description.clone(),
        );
        if initiative {
            set_hover(&mut out.values, glow, text, description);
        }
        let leader = unit_leader(leaders.rows(), side.commander);
        assigned(
            out,
            content.commanders,
            content.windows,
            &format!("{prefix}_traits_grid"),
            AssignedCommander {
                traits: &side.traits,
                context: 4,
                timed_traits: inputs
                    .map(|inputs| inputs.timed_traits.as_slice())
                    .map_err(|reason| format!("{LAND_TIMED_TRAITS_UNRECORDED}: {reason}")),
                country_tag: country_tag(countries, side.commander_country),
                advisor_feature,
                skill: leader
                    .filter(|row| row.skill_status != 0)
                    .map(|row| row.skill_level),
                advisor_roles: advisor_roles(
                    leader,
                    frame
                        .complete::<tables::Characters>()
                        .map_err(|error| error.to_string()),
                    frame
                        .complete::<tables::CharacterRoleTraits>()
                        .map_err(|error| error.to_string()),
                ),
            },
            &effect_context,
        );
    }
    Ok(())
}

fn naval_skills(
    out: &mut Bindings,
    icons: &dyn TextIcons,
    loc: &Localisation,
    skills: &Skills,
    definitions: Result<&Definitions<'_>, &Refusal>,
    commander: &NavalCommanderInputs,
    prefix: &str,
) {
    let levels = Definitions::settle(definitions, skills.component_conditions());
    for (index, (component, stat)) in [
        (Component::Attack, "attack"),
        (Component::Defense, "defense"),
        (Component::Maneuvering, "maneuvering"),
        (Component::Coordination, "coordination"),
    ]
    .into_iter()
    .enumerate()
    {
        let text = (|| -> Result<String, String> {
            let key = super::tooltips::LEADER_SKILL_DESC;
            let mut text = builtin(
                icons,
                loc,
                key,
                &[("S", Argument::Int32(commander.skills[index + 1]))],
            )?;
            text.push('\n');
            let selected = &commander.skill_definitions[index + 1];
            if selected.present == 0 {
                return Err("the selected component skill definition is null".into());
            }
            if selected.valid == 0 {
                return Ok(text);
            }
            levels.clone()?;
            let authored = skills
                .component_modifiers(component, selected.level, selected.category)?
                .ok_or_else(|| {
                    format!(
                        "the content has no {stat} skill definition at level {} in category {}",
                        selected.level, selected.category
                    )
                })?;
            if let Some(reason) = authored.unheld.cause() {
                return Err(format!("the {stat} skill modifier: {reason}"));
            }
            let definitions = definitions.map_err(ToString::to_string)?;
            let (mut block, dropped) =
                Block::build(definitions, authored).map_err(|reason| reason.to_string())?;
            for name in dropped {
                out.refuse(format!("Naval commander skill {prefix}/{stat}: content modifier {name} has no recorded definition; the native reader drops it"));
            }
            block.values = commander
                .modifiers
                .iter()
                .filter(|modifier| modifier.source == index as u8 + 1)
                .map(|modifier| (modifier.id, modifier.value))
                .collect();
            let mut tooltips: Vec<_> = commander
                .tooltips
                .iter()
                .filter(|tooltip| tooltip.source == index as u8 + 1)
                .collect();
            tooltips.sort_by_key(|tooltip| tooltip.bucket);
            block.custom_tooltips = tooltips
                .iter()
                .map(|tooltip| tooltip.text.clone())
                .collect();
            text.push_str(
                &block
                    .describe(definitions, icons, loc, super::fixed::ONE, "", u32::MAX)
                    .map_err(|reason| reason.to_string())?,
            );
            Ok(text)
        })();
        let text = match text {
            Ok(text) => hover_text(&text),
            Err(reason) => {
                out.refuse(format!(
                    "Naval commander skill {prefix}/{stat} hover is omitted: {reason}"
                ));
                None
            }
        };
        for kind in ["icon", "text"] {
            set_hover(
                &mut out.values,
                format!("{prefix}/leader_stat_{stat}_{kind}"),
                text.clone(),
                None,
            );
        }
    }
}

pub(super) fn bind_naval(
    out: &mut Bindings,
    static_content: &StaticContent,
    frame: &DecodedFrame,
    selection: Selection,
) -> io::Result<()> {
    let Selection::Naval(id) = selection else {
        return Ok(());
    };
    let details = rows::<tables::NavalCombatDetails>(frame)?;
    let Some(detail) = details.rows().iter().find(|row| row.combat == id) else {
        return Ok(());
    };
    let definitions = Definitions::read(frame, &static_content.modifier_names)?;
    let content = &static_content.assets.commander_tooltips;
    let read = content.settled(definitions.as_ref());
    for (side, label) in detail.sides.iter().zip(["attacker", "defender"]) {
        let prefix = format!("header_container/{label}_stats");
        if let Some(commander) = &side.commander_inputs
            && commander.commander != 0
        {
            naval_skills(
                out,
                static_content.assets.as_ref(),
                &static_content.localisation,
                &read.skills,
                definitions.as_ref(),
                commander,
                &prefix,
            );
        }
    }
    Ok(())
}

tooltip_keys! {
    FM_LEADER = "fm_leader",
    CC_LEADER = "cc_leader",
    NAVY_LEADER = "navy_leader",
    OPERATIVE = "OPERATIVE",
    LEARNING_TRAITS = "LEARNING_TRAITS",
    LEARNING_TRAITS_TITLE = "LEARNING_TRAITS_TITLE",
    LEADER_HAS_INITIATIVE = "LEADER_HAS_INITIATIVE",
    LEADER_PROXIMITY_FRONTLINE_TITLE = "LEADER_PROXIMITY_FRONTLINE_TITLE",
    LEADER_PROXIMITY_CLOSE_TITLE = "LEADER_PROXIMITY_CLOSE_TITLE",
    LEADER_PROXIMITY_MEDIUM_TITLE = "LEADER_PROXIMITY_MEDIUM_TITLE",
    LEADER_PROXIMITY_FAR_TITLE = "LEADER_PROXIMITY_FAR_TITLE",
    TRAIT_UNLOCKS_ADVISOR_ROLE = "TRAIT_UNLOCKS_ADVISOR_ROLE",
    TRAIT_UNLOCKED_BY_MIN_RANK = "TRAIT_UNLOCKED_BY_MIN_RANK",
    TRAIT_WILL_EXPIRE = "TRAIT_WILL_EXPIRE",
}

#[cfg(test)]
mod tests;
