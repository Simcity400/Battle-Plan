//! Commander hover text, rebuilt from recorded identities and content.
//!
//! Portrait: 1415C4AF0, which appends learning traits only for the viewing
//! country. The land helper constructs
//! modifier text in its second output string, then overwrites that string
//! with the commander's description (1415C56C5..1415C56E0). Consequently those
//! temporary modifier descriptions are deliberately not displayed here.
//! CUnitLeader+60 is a localized cache; +80 is its raw description token.
use super::{
    Selection, Values,
    binding::Bindings,
    binding::{TextIcons, builtin, token},
    modifier_text::{Block, Definitions, Refusal},
    tooltips::{hover_text, refuse_hovers, set_hover},
};
use crate::{
    document::Document,
    map::{counters::NativeWindows, perspective::Vision},
    table_access::rows,
};
use installed_content::{
    ResolvedLayers,
    defines::Defines,
    leader_traits::LeaderTraits,
    localisation::{Argument, Localisation},
    tokens::Token,
    unit_leader::{
        Adjuster, Component, Leaders, Skills, SubUnit, SubUnits, TraitEffects, TraitReference,
        Traits,
    },
};
use replay::{
    frame::DecodedFrame,
    rows::{
        CharacterRoleTraitRow, CharacterRow, CountryRow, LandCombatCommanderLearningTrait,
        LandCombatCommanderTimedTrait, LandCombatCommanderTrait, NavalCommanderInputs,
        UnitLeaderRow, tables,
    },
};
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, Mutex},
};

#[derive(Debug)]
pub(super) struct Content {
    /// `common/unit_leader` as read without the modifier registry: a block
    /// under a modifier entry is read as skipped and left a condition.
    leaders: Arc<Leaders>,
    /// The content terrain types' registered spellings, which 141001790
    /// makes adjusters of, for a re-read.
    terrain: Vec<String>,
    /// The re-read with the recorded modifier registry and the name index
    /// it used, when the content depends on it. The index is held, not its
    /// address: an evicted index stays alive here, so a new one never
    /// matches it by a reused address.
    settled: Mutex<Option<(Arc<super::modifier_text::NameIndex>, Arc<Leaders>)>>,
    /// `NCharacter.{SPECIALIST,EXPERT,GENIUS}_ADVISOR_MIN_RANK`, or the
    /// define that failed and why.
    advisor_ranks: Result<[i32; 3], String>,
    /// `NMilitary.GENERAL_PROXIMITY_{DEFAULT,CLOSE,MEDIUM}`, with which
    /// 140AD9060 and 14159EB40 turn a proximity offset into a distance title,
    /// or the define that failed and why.
    proximity: Result<[i32; 3], String>,
    /// The sub-unit database `sub_unit_modifiers` sorts by, or why the
    /// content cannot give it.
    sub_units: Result<SubUnits, String>,
    /// The country-leader traits (`common/country_leader`, loaded before
    /// the trait bodies by CBuildingDatabase::load 14018B610), which the
    leader_traits: Result<Arc<LeaderTraits>, String>,
}

/// Three `int32` defines of one category, or the first that failed.
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
    pub(super) fn sub_units(&self) -> Result<&installed_content::unit_leader::SubUnits, &str> {
        self.sub_units.as_ref().map_err(String::as_str)
    }
    /// The commander content, and the `leader_skills` the same files'
    /// readers hold, which the leader skill hover reads.
    /// `sub_units` is the one sub-unit database read
    /// (installed_content::unit_leader::sub_units) and `leader_traits` the one
    /// country-leader trait read (installed_content::leader_traits), or why each
    /// could not be read.
    pub(super) fn load(
        layers: &ResolvedLayers,
        defines: &Defines,
        sub_units: Result<SubUnits, String>,
        leader_traits: Result<Arc<LeaderTraits>, String>,
    ) -> io::Result<(Self, Skills)> {
        // 141001790 makes an adjuster of every content terrain type, found as
        // 1403BCA00 finds one.
        let (terrain, _) = installed_content::terrain::read(layers)?;
        let terrain: Vec<String> = terrain
            .categories
            .iter()
            .map(|category| String::from_utf8_lossy(&category.name).into_owned())
            .collect();
        let leaders = Leaders::load(layers, &|name| spelling(&terrain, name))?;
        let skills = leaders.skills.clone();
        // 14142E0A0 reads these authored rank thresholds. An unsupported Lua
        // expression affects only the advisor-unlock hover line.
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
                sub_units,
                leader_traits,
            },
            skills,
        ))
    }
}

/// A content terrain type as 1403BCA00 finds one (without ASCII case), by
/// its registered spelling.
fn spelling(terrain: &[String], name: &str) -> Option<String> {
    terrain
        .iter()
        .find(|known| known.eq_ignore_ascii_case(name))
        .cloned()
}

impl Content {
    /// The commander content as the game read it with the recording's
    /// modifier registry (1406042A0 finds a name through the lexer's token
    /// table, which the recorded definitions index): read again only when a
    /// block under a modifier entry makes the reading depend on it, and
    /// kept while the name index is the same. Without the definitions, the
    /// load-time reading, whose conditions the hovers then refuse by name.
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
                // Two definitions carry the name: which the token table
                // keeps is not recorded.
                Err(_) => None,
            },
        ));
        *settled = Some((identity, Arc::clone(&leaders)));
        leaders
    }

    /// Malformed `leader_traits` entries and readings the port cannot
    /// follow, counted, and each place the game's reading leaves the text.
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

/// Both hover strings through the one CTooltip setter.
fn set(values: &mut Values, key: impl Into<String>, text: Option<(String, String)>) {
    let (text, description) = match text {
        Some((text, description)) => (hover_text(&text), hover_text(&description)),
        None => (None, None),
    };
    set_hover(values, key, text, description);
}

/// 140BBA780/140BBA390 prefer TAG_key (or TAG_key_desc), with the raw
/// simulation country tag. Cosmetic and localized country names are unrelated.
/// `tag` is the country's recorded tag or why it is unavailable.
pub(super) fn country_localized(
    icons: &dyn TextIcons,
    loc: &Localisation,
    tag: Result<&str, &str>,
    key: &str,
) -> Result<String, String> {
    let text = super::text::country_localised(loc, tag, key)?;
    super::binding::displayable(icons, text)
        .map_err(|reason| format!("localisation {key} holds {reason}"))
}

fn ratio(experience: i64, required: i32) -> Option<i64> {
    super::fixed::fraction(experience, i64::from(required) * super::fixed::ONE)
}

fn learning(
    icons: &dyn TextIcons,
    loc: &Localisation,
    tag: Result<&str, &str>,
    traits: &[LandCombatCommanderLearningTrait],
    threshold: i64,
) -> Result<String, String> {
    let mut lines = String::new();
    for item in traits {
        let token = String::from_utf8_lossy(&item.token);
        let progress = ratio(item.experience, item.required_experience).ok_or_else(|| {
            format!(
                "learning trait {token}'s progress fraction: {}",
                super::fixed::TRAP
            )
        })?;
        // The native test is strictly greater; equality is not shown.
        if progress <= threshold {
            continue;
        }
        let token = std::str::from_utf8(&item.token)
            .map_err(|_| format!("learning trait token {token} is not UTF-8"))?;
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

/// 1415C4AF0's portrait hover, or the token, key or operand that failed.
pub(super) fn portrait(
    icons: &dyn TextIcons,
    loc: &Localisation,
    name: &[u8],
    description: &[u8],
    role: (i32, i32),
    progress: Option<(&[LandCombatCommanderLearningTrait], i64, Result<&str, &str>)>,
    initiative: bool,
) -> Result<(String, String), String> {
    let name =
        token(icons, loc, name).map_err(|refusal| format!("the commander's name: {refusal}"))?;
    let title = role_title(icons, loc, role.0, role.1)?;
    // 140C0A9A0 appends one space and the localized name (+0x40) to
    // GetLocalizedRoleTitle, even when an unknown category leaves it empty.
    let name = format!("{title} {name}");
    let mut text = builtin(
        icons,
        loc,
        super::tooltips::HEADER,
        &[("KEY", Argument::Text(&name))],
    )?;
    if let Some((traits, threshold, tag)) = progress {
        text.push_str(&learning(icons, loc, tag, traits, threshold)?);
    }
    if initiative {
        text.push('\n');
        text.push_str(&builtin(icons, loc, LEADER_HAS_INITIATIVE, &[])?);
    }
    let description = token(icons, loc, description)
        .map_err(|refusal| format!("the commander's description: {refusal}"))?;
    Ok((text, description))
}

/// CUnitLeader::GetLocalizedRoleTitle@140C0A7A0 selects by +0xE7C.
/// The operative branch reads CUnitLeaderSkill+0x1B8 and checks the key
/// before falling back. All these key spellings are executable literals.
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

/// An authored entry the game's reader drops, and why.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Dropped {
    /// No recorded modifier definition carries the name: 1406042A0 reports
    /// it and drops it.
    Modifier(String),
    /// The sub-unit database lacks the key: 1410079E0 reports it and drops
    /// it.
    SubUnit(String),
    /// The sub-unit is already specified: 1410079E0 keeps the first.
    Repeat(String),
}

impl std::fmt::Display for Dropped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Dropped::Modifier(name) => write!(
                f,
                "the modifier {name}, which no recorded modifier definition carries (1406042A0 reports and drops it)"
            ),
            Dropped::SubUnit(key) => write!(
                f,
                "the sub-unit {key}, which the sub-unit database lacks (1410079E0 reports and drops it)"
            ),
            Dropped::Repeat(key) => write!(
                f,
                "the sub-unit {key} a second time, which 1410079E0 drops as already specified"
            ),
        }
    }
}

/// Why a trait's effect lines cannot be rebuilt, counted per trait grid.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum EffectRefusal {
    /// A form the port cannot keep, such as text that is not UTF-8.
    Content(String),
    /// The game's parse of the trait's file leaves the text before or in
    /// the trait, or may, depending on the modifier registry.
    OutOfStep(String),
    Modifier(Refusal),
    /// `sub_unit_modifiers` needs the sub-unit database order (140AD36C0
    /// sorts by `CSubUnitDefinition+0x58C`), which the content failed to give.
    SubUnitOrder(String),
    /// The localisation refusal, naming the key and why.
    Localisation(String),
    /// A proximity offset needs the NMilitary proximity defines: why they
    /// are unavailable.
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

/// A content key: one the localisation lacks shows as itself (142230940),
/// so only output the localiser or the native text layout cannot produce refusals.
fn text(icons: &dyn TextIcons, loc: &Localisation, key: &str) -> Result<String, EffectRefusal> {
    token(icons, loc, key.as_bytes()).map_err(EffectRefusal::Localisation)
}

/// A key the executable names: it must be in the localisation, and a missing
/// or unsupported one is refused by name (the owner's rule for built-in names),
/// never shown as its own text as a content key would be.
fn named(
    icons: &dyn TextIcons,
    loc: &Localisation,
    key: &str,
    args: &[(&str, Argument<'_>)],
) -> Result<String, EffectRefusal> {
    builtin(icons, loc, key, args).map_err(EffectRefusal::Localisation)
}

/// 140FFFDD0(adjuster, 1, null, 1, 0): the key's name as a header, then the
/// movement, attack and defence lines that are not zero.
fn adjuster(
    icons: &dyn TextIcons,
    loc: &Localisation,
    adjuster: &Adjuster,
) -> Result<String, EffectRefusal> {
    if adjuster.attack == 0 && adjuster.defence == 0 && adjuster.movement == 0 {
        return Ok(String::new());
    }
    // 141000A30: the key's localisation in the highlight colour, the marks
    // appended to an unbounded string (append_bytes 140129700), outside the
    // localiser's buffer.
    let mut out = format!("\u{11}H{}\u{11}!", text(icons, loc, &adjuster.key)?);
    out.push('\n');
    out.push_str(&adjuster_lines(icons, loc, adjuster)?);
    Ok(out)
}

/// 140FFFDD0's value lines: 140FFF670 builds `STAT_ADJUSTER_<STAT>` and its
/// `_DIFF` value, each ended by a newline.
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

/// What 140AD9060 needs besides the trait itself.
struct EffectContext<'a> {
    icons: &'a dyn TextIcons,
    loc: &'a Localisation,
    definitions: Result<&'a Definitions<'a>, &'a Refusal>,
    /// The traits as read with the recording's modifier registry.
    traits: &'a Traits,
    proximity: Result<[i32; 3], &'a str>,
    /// The chosen country's tag: 140BBA630 localises for the player's
    /// country, which is the viewer's chosen country. The error says why it
    /// is unavailable.
    viewing_tag: Result<&'a str, &'a str>,
    /// The sub-unit database, from the content.
    sub_units: Result<&'a SubUnits, &'a str>,
}

/// The trait item passes indent 1 (141C5F730 -> 140AD9060): two spaces
/// before each sub-unit modifier line.
const SUB_UNIT_INDENT: &str = "  ";

/// 140AD36C0 over what 1410079E0 kept: adjusters and modifier blocks keyed
/// by sub-unit index, merged in index order, each under the sub-unit's
/// highlighted name (140AD5CD0).
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
        // 1410079E0 finds the sub-unit without case (140AB5DA0).
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
        // A sub-unit already kept is "already specified" and skipped.
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
        // Copied into the entry, then into the array (1405492D0 twice).
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
    // 140AD5CD0 names the sub-unit by its registered spelling.
    let name = |index: u32| -> Result<String, EffectRefusal> {
        let key = order
            .name(index)
            .expect("indices come from the database")
            .map_err(EffectRefusal::Content)?;
        let name = country_localized(context.icons, context.loc, context.viewing_tag, key)
            .map_err(EffectRefusal::Localisation)?;
        // 140AD5CD0 appends the marks to an unbounded string (140129700).
        Ok(format!("\u{11}H{name}\u{11}!\n"))
    };
    let mut out = String::new();
    let indices: std::collections::BTreeSet<u32> =
        adjusters.keys().chain(blocks.keys()).copied().collect();
    for index in indices {
        super::text::ensure_newlines(&mut out, 1);
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

/// 140AD9060 with the combat grid's arguments (indent 1, scale 1.0).
/// Names the definition table lacks go to `dropped`: the native reader
/// reports and drops them, and the caller counts them.
fn effects(
    context: &EffectContext<'_>,
    definition: &TraitEffects,
    dropped: &mut Vec<Dropped>,
) -> Result<String, EffectRefusal> {
    let (icons, loc) = (context.icons, context.loc);
    let mut out = String::new();
    // A trait whose content was refused is refused whole: which override
    // lines the game's reader would have kept is not known either.
    if let Some(what) = definition.unsupported.first() {
        return Err(if what.starts_with("the game's parse of ") {
            EffectRefusal::OutOfStep(what.clone())
        } else {
            EffectRefusal::Content(what.clone())
        });
    }
    // A block under a registered modifier earlier in the file leaves the
    // trait's reading unknown.
    Definitions::settle(context.definitions, &definition.conditions)
        .map_err(EffectRefusal::OutOfStep)?;
    if !definition.override_tooltips.is_empty() {
        // The early return suppresses every standard effect.
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
    // modifier, non_shared_modifier, field_marshal_modifier and
    // corps_commander_modifier, each through 14054E050.
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
            super::text::ensure_newlines(&mut out, 1);
        }
    }
    for item in &definition.adjusters {
        super::text::ensure_newlines(&mut out, 1);
        out.push_str(&adjuster(icons, loc, item)?);
    }
    // 140AD9060 appends the sub-unit text without a newline of its own.
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
        super::text::ensure_newlines(&mut out, 1);
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
            super::text::ensure_newlines(&mut out, 1);
            out.push_str(&named(icons, loc, key, &[("VAL", Argument::Int32(value))])?);
        }
    }
    if definition.proximity_offset != 0 {
        let [default, close, medium] = context
            .proximity
            .map_err(|reason| EffectRefusal::ProximityDefines(reason.into()))?;
        // 14159EB40 on the offset plus the default distance.
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
        super::text::ensure_newlines(&mut out, 1);
        let key = "trait_bonus_leader_default_proximity_offset";
        out.push_str(&named(
            icons,
            loc,
            key,
            &[("NAME", Argument::Text(&title))],
        )?);
    }
    // The reader kept only keys naming a trait, by its registered spelling.
    for (token, value) in &definition.experience_factors {
        super::text::ensure_newlines(&mut out, 1);
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
    super::text::ensure_newlines(&mut out, 1);
    Ok(out)
}

#[derive(Clone, Copy)]
struct AdvisorTrait<'a> {
    token: &'a str,
    ordinal: u32,
}

/// 140ADC880 stores the country-leader trait resolved by token (140712DD0),
/// or null when the field is absent or the token names no trait.
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
    // 140712DD0 compares tokens with A-Z folding (1424A65A0).
    // No matching trait means a null field (140ADC880).
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

/// 14142E0A0 searches roles in map order: genius first, then expert below
/// expert rank. Recorded definition ordinals preserve pointer identity.
fn selected_advisor_trait<'a>(
    definition: &TraitEffects,
    skill: i32,
    [specialist, expert, genius]: [i32; 3],
    traits: Result<&'a LeaderTraits, &str>,
    roles: Result<&[&CharacterRoleTraitRow], &str>,
) -> Result<(&'a str, Option<i32>), String> {
    let resolved = |slot: usize| advisor_trait(definition.advisor_traits[slot].as_ref(), traits);
    // 141C5F730 reads the specialist trait (+0x68) when 14142E0A0 finds none.
    let specialist_trait = || {
        resolved(0)?.ok_or_else(|| {
            "the trait's specialist_advisor_trait is null (never read, or naming no country-leader trait), which 141C5F730 falls back to"
                .to_owned()
        })
    };
    // Native rank tests retain their order even with unordered thresholds.
    // 141C5F730 tests the minimum-rank suffix independently.
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
    // The caller asks only for a trait with a slot (140ADC4B0).
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

/// 140C0B3C0 tests the first matching entry of the commander's timed-trait
/// vector. Its signed day count decides: -1 is not an expiring trait, zero
/// still is.
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
    timed_traits: &'a [LandCombatCommanderTimedTrait],
    country_tag: Result<&'a str, &'a str>,
    advisor_feature: Option<bool>,
    skill: Option<i32>,
    advisor_roles: Result<Vec<&'a CharacterRoleTraitRow>, String>,
}

/// The trait grid's hovers (141C5F730 with 1415C8950's scale 1.0), counting
/// each refusal and each entry the native readers drop.
fn assigned(
    out: &mut Bindings,
    content: &Content,
    windows: &NativeWindows,
    key: &str,
    commander: AssignedCommander<'_>,
    effect_context: &EffectContext<'_>,
) {
    let Some(grid) = out.values.get_mut(key) else {
        // presentation.rs builds every side's grid, so this is a binder
        // order fault, reported rather than dropped.
        out.warnings.push(format!(
            "Combat trait hover {key}: the trait grid is not bound; its hovers are omitted."
        ));
        return;
    };
    let (icons, loc) = (effect_context.icons, effect_context.loc);
    let mut refused: BTreeMap<String, usize> = BTreeMap::new();
    let mut dropped: BTreeMap<String, usize> = BTreeMap::new();
    let mut ignored: BTreeMap<String, usize> = BTreeMap::new();
    // Advisor unlock lines and whole hovers refused, counted by reason.
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
            // 141C5F730 names HEADER literally; exile hovers require the
            // content definition rather than the localiser's missing-key text.
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
            // The recorded token names the trait without case (1424A65A0).
            let definition = effect_context.traits.get(trait_token).ok_or_else(|| {
                format!("trait {trait_token} is not in the content's leader traits")
            })?;
            for what in &definition.ignored {
                *ignored
                    .entry(format!("trait {trait_token} has {what}"))
                    .or_default() += 1;
            }
            // 141C5F730 prefixes the description with a newline.
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
            super::text::ensure_newlines(&mut text, 1);
            // 140AD8CF0: each custom effect line, then a newline.
            for key in &definition.custom_tooltips {
                text.push_str(&token(icons, loc, key.as_bytes())?);
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
                                // 141C5F730 separates the advisor unlock from
                                // the effect/custom-tooltip lines by two newlines.
                                super::text::ensure_newlines(&mut text, 2);
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
            // Combat passes assignment context 4, so the generic trait item's
            // army-commanding inactivity warning is inapplicable (1415C8950).
            let description_key = format!("{trait_token}_desc");
            let mut description =
                country_localized(icons, loc, commander.country_tag, &description_key)?;
            // 141C5F730: a temporary trait (140C0B3C0: the first +0xDE0 entry
            // of the trait with nonnegative days) appends a newline and
            // TRAIT_WILL_EXPIRE with its days (140C07810) to the delayed text.
            if let Some(days) = timed_days(commander.timed_traits, item) {
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
            // An empty reason was counted with the effect refusals.
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
            // 141C5EF70 selects the exile template and installs the trait
            // handler on its container. 1422AF2D0 -> 14229D3A0 propagates it
            // to every child, including exile_flag (1429D71B0), with no
            // Background. Follow the authored tree, including mod children.
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
        out.warnings.push(format!(
            "Combat trait hover {key}: the hover of {what} is omitted ({count} shown)."
        ));
    }
}

/// The country's recorded tag, or why it is unavailable.
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

/// LookupCharacter (140C0EEF0), then the nonempty advisor vectors of its
/// CCharacter+0xB0 map. The recorder stores each role's map position, including
/// positions skipped for empty vectors (engine::character_inputs). A partial
/// table cannot prove that no earlier role selects another definition.
fn advisor_roles<'a>(
    leader: Option<&UnitLeaderRow>,
    characters: Result<&[CharacterRow], String>,
    traits: Result<&'a [CharacterRoleTraitRow], String>,
) -> Result<Vec<&'a CharacterRoleTraitRow>, String> {
    let leader = leader.ok_or("the leader's UnitLeaders row is unavailable")?;
    if leader.character_type == 0 && leader.character_id == 0 {
        return Err("the leader's character reference is null (140C0EEF0), which the advisor-role search dereferences".into());
    }
    let characters = characters?;
    if !characters.iter().any(|row| row.id == leader.character_id) {
        return Err(format!(
            "the leader's character {} has no Characters row, so LookupCharacter (140C0EEF0) is unresolved",
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

pub(super) fn bind(
    out: &mut Bindings,
    document: &Document,
    frame: &DecodedFrame,
    selection: Selection,
    vision: &Vision,
) -> io::Result<()> {
    let Selection::Land(id) = selection else {
        return Ok(());
    };
    let (icons, loc) = (&document.icons.combat, &document.names.localisation);
    let country_rows = rows::<tables::Countries>(frame)?;
    let countries = country_rows.rows();
    let links = rows::<tables::CountryLinks>(frame)?;
    let viewing_tag = super::text::viewing_tag(countries, vision);
    let leaders = rows::<tables::UnitLeaders>(frame)?;
    let features = rows::<tables::SimulationFeatures>(frame)?;
    let advisor_feature = if features.is_complete() {
        match features.rows() {
            [feature] => Some(feature.dlc_mask & 0x80 != 0), // 1401AE640 feature 0x25
            _ => None,
        }
    } else {
        None
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
    let definitions = Definitions::read(frame, &document.modifier_names)?;
    let content = &document.icons.combat.commander_tooltips;
    let read = content.settled(definitions.as_ref());
    let effect_context = EffectContext {
        icons,
        loc,
        definitions: definitions.as_ref(),
        traits: &read.traits,
        proximity: content.proximity.as_ref().copied().map_err(String::as_str),
        viewing_tag,
        sub_units: content.sub_units.as_ref().map_err(String::as_str),
    };
    for (index, prefix) in ["attacker", "defender"].into_iter().enumerate() {
        let side = &detail.sides[index];
        let frame_key = if index == 0 {
            "land_battle_leader_frame"
        } else {
            "land_battle_leader_frame2"
        };
        if side.commander == 0 {
            // No commander, no portrait hover (1415C4AF0 is not reached).
            refuse_hovers(&mut out.values, [frame_key]);
            continue;
        }
        let glow = format!("{prefix}_leader_initiative_overlay_glow");
        let initiative = out.values.get(&glow).is_some_and(|widget| !widget.hidden);
        // 1415C4AF0 appends the learning traits only for the viewing
        // country's commander (a country equality): a link this hour cannot
        // resolve decides neither, which matters only when there are some.
        let own_commander = crate::table_access::country_slot(&links, side.commander_country)
            .map(|slot| slot == Some(vision.ours()));
        let unknown_owner = match &own_commander {
            Err(reason) if !side.learning_traits.is_empty() => Some(format!(
                "whether the commander is the viewing country's, whose commander alone shows the learning-trait progress (1415C4AF0), is unknown: {reason}"
            )),
            // With no learning traits both answers give the same text.
            _ => None,
        };
        let progress = if own_commander == Ok(true) {
            match &document.icons.combat.land_defines {
                Ok(defines) => Some((
                    side.learning_traits.as_slice(),
                    defines.learning_threshold,
                    viewing_tag,
                )),
                Err(reason) => {
                    if !side.learning_traits.is_empty() {
                        out.warnings.push(format!(
                            "Land combat {id} {prefix} commander: the land combat defines are unavailable ({reason}), so the learning-trait progress is omitted."
                        ));
                    }
                    None
                }
            }
        } else {
            None
        };
        let tooltip = match unknown_owner.map_or_else(
            || {
                portrait(
                    icons,
                    loc,
                    &side.commander_name_token,
                    &side.commander_description_token,
                    (side.commander_category, side.commander_skill),
                    progress,
                    initiative,
                )
            },
            Err,
        ) {
            Ok(tooltip) => Some(tooltip),
            Err(reason) => {
                out.warnings.push(format!(
                    "Land combat {id} {prefix} commander portrait hover is omitted: {reason}."
                ));
                None
            }
        };
        set(&mut out.values, frame_key, tooltip.clone());
        // Only a shown glow carries the hover (1415C8950).
        if initiative {
            set(&mut out.values, glow, tooltip);
        }
        let leader = unit_leader(leaders.rows(), side.commander);
        assigned(
            out,
            content,
            &document.icons.combat.native_windows,
            &format!("{prefix}_traits_grid"),
            AssignedCommander {
                traits: &side.traits,
                timed_traits: &side.timed_traits,
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

/// 141DF3A30 installs 141DF4270 on both the icon and its skill text.
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
            // 142230940 displays a missing localisation key itself.
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
            // 141DF4270 reads the retained +F70/+F80/+F90/+FA0 definition,
            // not a fresh lookup from the displayed component level.
            // 140AD4190 constructs the null skill with an empty CModifier.
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
            // Content supplies the hidden set; keep the retained definition's
            // recorded values and custom tooltip slot order (140AD9010 -> 14054E050).
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
            block.custom_tooltips = tooltips.iter().map(|tooltip| tooltip.text.clone()).collect();
            // 140AD9010 -> 14054E050: precision 2, all categories, no
            // indentation, scale 1.0. Reuse its shared modifier formatter.
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
            // 141DF3A30 binds leader_stat_{attack,defense,maneuvering,coordination}_{icon,text}.
            set_hover(
                &mut out.values,
                format!("{prefix}/leader_stat_{stat}_{kind}"),
                text.clone(),
                None,
            );
        }
    }
}

fn refuse_naval_traits(
    out: &mut Bindings,
    windows: &NativeWindows,
    key: &str,
    traits: &[LandCombatCommanderTrait],
) {
    let Some(grid) = out.values.get_mut(key) else {
        out.warnings.push(format!(
            "Combat trait hover {key}: the trait grid is not bound; its hovers are omitted."
        ));
        return;
    };
    let shown: Vec<_> = traits
        .iter()
        .filter(|item| item.kind != 0 && item.relevant != 0)
        .collect();
    let entry_count = grid.entries.len();
    let mut refused = Vec::new();
    for (index, (entry, &item)) in grid.entries.iter_mut().zip(&shown).enumerate() {
        if item.kind != 6 {
            // 141C5EF70 installs the ordinary trait handler only on Background.
            refuse_hovers(entry, ["Background"]);
        } else {
            // Exile handlers propagate to the authored tree (1422AF2D0 / 14229D3A0).
            let template = grid
                .entry_templates
                .get(&index)
                .or(grid.entry_template.as_ref());
            let keys: Vec<_> = template
                .and_then(|name| windows.gui_type(name.as_bytes()))
                .map(|root| {
                    super::gui::elements(root)
                        .into_iter()
                        .map(|(path, _)| path)
                        .collect()
                })
                .unwrap_or_else(|| entry.keys().cloned().collect());
            refuse_hovers(entry, keys);
        }
        // 141C5F730 unconditionally queries 140C0B3C0 / 140C07810:
        // CUnitLeader+0xDE0 pointer, +0xDEC count; stride 0x10, definition +0, days +8.
        // The naval recording has no timed-trait vector, including zero-day entries.
        refused.push(format!(
            "Naval trait {} hover is omitted: the commander's timed-trait entries are not recorded",
            String::from_utf8_lossy(&item.token)
        ));
    }
    if entry_count != shown.len() {
        out.warnings.push(format!(
            "Combat trait hover {key}: the grid has {entry_count} entries for {} shown traits; the unmatched ones have no hover.",
            shown.len()
        ));
    }
    for reason in refused {
        out.refuse(reason);
    }
}

pub(super) fn bind_naval(
    out: &mut Bindings,
    document: &Document,
    frame: &DecodedFrame,
    selection: Selection,
) -> io::Result<()> {
    if let Selection::NavalResult(id) = selection {
        let details = rows::<tables::NavalCombatResultDetails>(frame)?;
        if let Some(detail) = details.rows().iter().find(|row| row.result == id) {
            for (side, label) in detail.sides.iter().zip(["attacker", "defender"]) {
                if let Some(commander) = &side.commander_context
                    && commander.commander != 0
                {
                    // 1417C71F0 / 1417C6800 bind {attacker,defender}_leader/traits_grid.
                    refuse_naval_traits(
                        out,
                        &document.icons.combat.native_windows,
                        &format!("{label}_leader/traits_grid"),
                        &commander.traits,
                    );
                }
            }
        }
        return Ok(());
    }
    let Selection::Naval(id) = selection else {
        return Ok(());
    };
    let details = rows::<tables::NavalCombatDetails>(frame)?;
    let Some(detail) = details.rows().iter().find(|row| row.combat == id) else {
        return Ok(());
    };
    let definitions = Definitions::read(frame, &document.modifier_names)?;
    let content = &document.icons.combat.commander_tooltips;
    let read = content.settled(definitions.as_ref());
    for (side, label) in detail.sides.iter().zip(["attacker", "defender"]) {
        // 1417D81B0 binds header_container/{attacker,defender}_stats.
        let prefix = format!("header_container/{label}_stats");
        if let Some(commander) = &side.commander_inputs {
            // 1417D81B0 binds header_container/traits_grid_{attacker,defender}.
            refuse_naval_traits(
                out,
                &document.icons.combat.native_windows,
                &format!("header_container/traits_grid_{label}"),
                &commander.traits,
            );
            if commander.commander != 0 {
                naval_skills(
                    out,
                    &document.icons.combat,
                    &document.names.localisation,
                    &read.skills,
                    definitions.as_ref(),
                    commander,
                    &prefix,
                );
            }
        }
    }
    Ok(())
}

// Role key literals: CUnitLeader::GetLocalizedRoleTitle@140C0A7A0.
crate::combat::text::tooltip_keys! {
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
