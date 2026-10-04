//! Ideologies (`common/ideologies`): each ideology group, its colour and its
//! sub-ideologies. The recording names a party by its group token and a leader
//! by a sub-ideology; this joins both to the group and its colour, so a mod's
//! ideologies chart with their own colours.
//!
//! Each file is read by its own CReader through the reader replay
//! ([`native`]): CIdeologyGroupDatabase::vf6 (140A38A00) reads `ideologies`
//! (0x2FD1) through its group loop (140A38190), each group a CIdeologyGroup
//! (CIdeologyGroup::vf4 14117C9C0), and stops that file at any other top-level key.

use crate::{
    ResolvedLayers,
    coverage::{Coverage, Gap},
    crt::scan_f32,
    ideas::{RuleNode, rule_field},
    modifiers,
    parser::{
        Item, Unheld,
        native::{self, Field, Shape},
    },
    script_files,
};
use std::{collections::BTreeMap, io};

// CColor::construct@142236FA0.
const WHITE: [f32; 4] = [1.0; 4];

#[derive(Clone, Debug, PartialEq)]
pub struct Ideology {
    pub token: String,
    /// The group's CColor (+0x460) as its reads leave it (red, green, blue,
    /// alpha), or why a read the port does not follow makes it unknown.
    pub color: Result<[f32; 4], String>,
    /// Sub-ideologies (`types`), in source order.
    pub types: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ideologies {
    /// In load order.
    groups: Vec<Ideology>,
    /// Each group's `CIdeologyGroup+0x40` ([`Ideologies::frame`]) and the file that first read
    /// it, by the token's lexer fold (`A`-`Z` joined, `1424A65A0`).
    frames: BTreeMap<String, (u32, String)>,
    coverage: Coverage,
}

impl Ideologies {
    /// A group by token: None when no file defines it, the gap when a file
    /// that could define or redefine it stops short.
    pub fn get(&self, token: &str) -> Result<Option<&Ideology>, &Gap> {
        match self.coverage.unknown(token) {
            Some(gap) => Err(gap),
            None => Ok(self.groups.iter().find(|group| group.token == token)),
        }
    }

    /// The group a sub-ideology belongs to; any file that stops short could
    /// declare it.
    pub fn group_of(&self, sub_ideology: &str) -> Result<Option<&Ideology>, &Gap> {
        match self.coverage.any_gap() {
            Some(gap) => Err(gap),
            None => Ok(self
                .groups
                .iter()
                .find(|group| group.types.iter().any(|kind| kind == sub_ideology))),
        }
    }

    /// A group's `CIdeologyGroup+0x40` as its raw bits, the frame `1418C4B50` sets on a naval
    /// counter's `counter_ideology` (`number_groups`): its place in the last `ideologies` block
    /// of its file that listed it, or `0xFFFFFFFF` (the constructor's, `14117C030`) when a later
    /// block of that file reset it (`140A38FE0`). `Ok(None)` when no file defines the group;
    /// `Err` when any file stops short (it could define the group first or shift its place).
    pub fn frame(&self, token: &str) -> Result<Option<u32>, String> {
        if let Some(gap) = self.coverage.any_gap() {
            return Err(gap.to_string());
        }
        Ok(self
            .frames
            .get(&token.to_ascii_lowercase())
            .map(|(frame, _)| *frame))
    }

    /// Every group read, in load order, and the gap that may hide more.
    pub fn groups(&self) -> (&[Ideology], Option<&Gap>) {
        (&self.groups, self.coverage.any_gap())
    }

    /// A database of these groups, with a file stopping short where `gap`
    /// names one, for other modules' tests.
    #[cfg(test)]
    pub(crate) fn from_groups(groups: Vec<Ideology>, gap: Option<(&str, &str)>) -> Self {
        let mut coverage = Coverage::default();
        if let Some((path, cause)) = gap {
            coverage.gap(path, cause);
        }
        Self {
            groups,
            frames: BTreeMap::new(),
            coverage,
        }
    }
}

/// Which reader a field goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    /// 140A38A00's file loop.
    Top,
    /// 140A38190's group loop.
    Groups,
    /// CIdeologyGroup::vf4 (14117C9C0).
    Group,
    /// `types`' own loop (140A37930): each key a CIdeology.
    Types,
    /// CIdeology::vf4 (14117C960).
    Ideology,
    Rule(RuleNode),
    Modifier(modifiers::Node),
}

/// CIdeologyGroup::vf4 (14117C9C0) per key (keyword ids from
/// register_authored_keys 140082CB0).
fn group_field(field: &Field<'_>) -> Shape<Node> {
    match field.keyword().as_deref() {
        // `color` 0x56: its CColor (+0x460) through ReadObjectFromParser.
        Some(b"color") => Shape::Color,
        // `types` 0x300C: its own loop.
        Some(b"types") => Shape::Object(Node::Types),
        // `rules` 0x317B its CRule (+0x60); `modifiers` 0x31E6 and
        // `faction_modifiers` 0x344A its CModifiers (+0x480, +0x540), each
        // through ReadObjectFromParser.
        Some(b"rules") => Shape::Object(Node::Rule(RuleNode::Rule)),
        Some(b"modifiers" | b"faction_modifiers") => {
            Shape::Object(Node::Modifier(modifiers::Node::Modifier))
        }
        // `dynamic_faction_names` 0x3500 ReadStringList.
        Some(b"dynamic_faction_names") => Shape::List,
        // ReadFixed5: `war_impact_on_world_tension` 0x34E3,
        // `faction_impact_on_world_tension` 0x34E4,
        // `ai_ideology_wanted_units_factor` 0x3133; ReadScriptSignedInt32:
        // `ai_give_core_state_control_threshold` 0x27DF; read_bool_field:
        // `ai_democratic` 0x3334, `ai_fascist` 0x3335, `ai_communist` 0x3336,
        // `can_be_boosted` 0x3475, `ai_neutral` 0x3491, `can_collaborate`
        // 0x379E, `can_host_government_in_exile` 0x3AB8.
        Some(
            b"war_impact_on_world_tension"
            | b"faction_impact_on_world_tension"
            | b"ai_ideology_wanted_units_factor"
            | b"ai_give_core_state_control_threshold"
            | b"ai_democratic"
            | b"ai_fascist"
            | b"ai_communist"
            | b"can_be_boosted"
            | b"ai_neutral"
            | b"can_collaborate"
            | b"can_host_government_in_exile",
        ) => Shape::One,
        // Serializable_ReportUnexpectedToken (1424A9CE0).
        _ => Shape::Skip,
    }
}

/// 140A38190: duplicates skip their bodies; later blocks in the owning file reread the group.
fn read_file_in(
    input: &[u8],
    path: &str,
    frames: &BTreeMap<String, (u32, String)>,
) -> (native::Read, Vec<modifiers::Undecided>, Vec<String>) {
    let mut undecided = Vec::new();
    let mut owners: BTreeMap<Vec<u8>, String> = frames
        .iter()
        .map(|(key, (_, file))| (key.as_bytes().to_vec(), file.clone()))
        .collect();
    let mut listed = Vec::new();
    let mut refusals = Vec::new();
    // 1401886F0: one mode-0 lexer per file.
    let read = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::File(Node::Top),
        None,
        &mut |node, field| match node {
            Node::Top if field.keyword_is("ideologies") => {
                listed.clear();
                Shape::Object(Node::Groups)
            }
            // 140A38A00 reports (1424AD100) then returns; the next file still loads.
            Node::Top => Shape::StopFile,
            Node::Groups => {
                // AddOrFindDynamicToken@1424A6500 folds once for the stored group identity.
                let key = crate::parser::c_string(&field.key.text).to_ascii_lowercase();
                if let Some(owner) = owners.get(&key)
                    && (owner != path || listed.contains(&key))
                {
                    refusals.push(format!(
                        "Ideologies in {path}: line {}: Duplicate database id `{}` refused (140A38190); first defined in {}; later block and its sub-ideologies skipped.",
                        field.key.line, String::from_utf8_lossy(&key), owner,
                    ));
                    Shape::Discard
                } else {
                    owners.entry(key.clone()).or_insert_with(|| path.to_owned());
                    listed.push(key.clone());
                    Shape::ObjectAs(Node::Group, key)
                }
            }
            Node::Group => group_field(field),
            // Each key a CIdeology read through its Read (CPersistent::Read).
            Node::Types => Shape::Object(Node::Ideology),
            // `color` 0x56 its CColor (+0x100), `can_be_randomly_selected`
            // 0x4CCF read_bool_field; any other key goes to its CModifier
            // (+0x40, CIdeology::CIdeology 14117BEE0) through the modifier's
            // ReadToken (vtable +0x20, 1406039A0 -> 1406031A0).
            Node::Ideology => match field.keyword().as_deref() {
                Some(b"color") => Shape::Color,
                Some(b"can_be_randomly_selected") => Shape::One,
                _ => modifiers::shape(
                    modifiers::Node::Modifier,
                    field,
                    modifiers::UNKNOWN,
                    &mut undecided,
                )
                .map(Node::Modifier),
            },
            Node::Rule(rule) => rule_field(rule, field).map(Node::Rule),
            Node::Modifier(modifier) => {
                modifiers::shape(modifier, field, modifiers::UNKNOWN, &mut undecided)
                    .map(Node::Modifier)
            }
        },
    );
    (read, undecided, refusals)
}

/// One `color` read into a CColor (CColor::vf3 142237160), from the state
/// the earlier reads left (CColor::construct 142236FA0: all 1.0). After
/// `{`, each of three components is read by `%f` (read_float_token
/// 1424B0320, whatever the token's kind; a failed scan leaves the
/// component) and then, stored or not, divided by 255 when above 1.0
/// (14270E270, 1427200C8), the alpha is set to 1.0, and a fourth component
/// is the alpha as read, not divided. The lexer's current token as a
/// number token (0xC) alone is a packed value read by `%x` (1424B01A0),
/// which the port refuses by name; any other token, a quoted one included,
/// is reported and changes nothing.
fn read_color(color: &mut [f32; 4], item: &Item) -> Result<(), String> {
    let Some(components) = item.value.items() else {
        if item.number() {
            return Err(format!(
                "the packed colour {} (read by `%x`, 1424B01A0) is not converted",
                item.value.scalar().unwrap_or_default().escape_ascii()
            ));
        }
        return Ok(());
    };
    for (index, component) in components.iter().enumerate().take(4) {
        let text = component.value.scalar().unwrap_or_default();
        let scanned = scan_f32(text);
        if index == 3 {
            if let Some(alpha) = scanned {
                color[3] = alpha;
            }
            continue;
        }
        if let Some(value) = scanned {
            color[index] = value;
        }
        if color[index] > 1.0 {
            color[index] /= 255.0;
        }
        if index == 2 {
            color[3] = 1.0;
        }
    }
    Ok(())
}

/// The groups of one file's `ideologies` blocks, up to the first whose
/// stored text (its token or a sub-ideology's) the port cannot hold, and
/// that one's cause.
fn groups(items: &[Item]) -> (Vec<Ideology>, Option<String>) {
    let mut groups = Vec::new();
    let roots = items.iter().filter(|item| item.keyword_is("ideologies"));
    for item in roots.flat_map(|root| root.value.items().unwrap_or_default()) {
        let mut unheld = Unheld::default();
        let token = unheld.keep("an ideology group's token", item.key_str());
        let mut color = Ok(WHITE);
        let mut types: Vec<String> = Vec::new();
        for field in item.value.items().unwrap_or_default() {
            if field.keyword_is("color") {
                // Each read goes into the one CColor, from where the last
                // left it.
                if let Ok(state) = &mut color
                    && let Err(cause) = read_color(state, field)
                {
                    color = Err(cause);
                }
            } else if field.keyword_is("types") {
                // Every `types` loop adds its keys (140A37930).
                for kind in field
                    .value
                    .items()
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|kind| unheld.keep("a sub-ideology's token", kind.key_str()))
                {
                    if !types.iter().any(|known| known == kind) {
                        types.push(kind.to_owned());
                    }
                }
            }
        }
        match (token, unheld.cause()) {
            (Some(token), None) => groups.push(Ideology {
                token: token.to_owned(),
                color,
                types,
            }),
            (_, cause) => return (groups, cause.map(str::to_owned)),
        }
    }
    (groups, None)
}

/// 140A38A00 numbers accepted groups from 1; 140A38FE0 resets omitted groups to 14117C030's -1.
fn number_groups(path: &str, items: &[Item], frames: &mut BTreeMap<String, (u32, String)>) {
    for block in items
        .iter()
        .filter(|item| item.keyword_is("ideologies"))
        .map(|root| root.value.items().unwrap_or_default())
    {
        let listed: Vec<_> = block
            .iter()
            .map_while(|group| group.key_str().ok())
            .collect();
        for (key, (frame, file)) in frames.iter_mut() {
            if file == path && !listed.contains(&key.as_str()) {
                *frame = u32::MAX;
            }
        }
        for (place, key) in (1_u32..).zip(listed) {
            frames.insert(key.to_owned(), (place, path.to_owned()));
        }
    }
}

/// Parses one ideologies file.
pub fn parse(input: &[u8]) -> Vec<Ideology> {
    groups(&read_file_in(input, "", &BTreeMap::new()).0.items).0
}

/// Every ideology group in native file order (1401886F0): one CReader per
/// enumerated file. The first file defining a group owns it (140A38190).
pub fn read(layers: &ResolvedLayers) -> io::Result<(Ideologies, Vec<String>)> {
    let mut warnings = Vec::new();
    let mut result = Ideologies::default();
    for path in script_files::paths(layers, "common/ideologies/", None) {
        result.coverage.next_file();
        let (read, undecided, refused) = read_file_in(&layers.read(&path)?, &path, &result.frames);
        warnings.extend(refused);
        warnings.extend(
            read.misreads
                .iter()
                .map(|misread| format!("Ideologies in {path}: {misread}.")),
        );
        // A modifier block the registry decides, or a block the port cannot
        // settle, leaves the rest of the file unknown: every group the file
        // could define or redefine is refused, with the place and why.
        let (_, unheld) = groups(&read.items);
        let stop = undecided
            .first()
            .map(ToString::to_string)
            .or_else(|| read.unsettled.as_ref().map(ToString::to_string))
            .or(unheld);
        if let Some(cause) = &stop {
            warnings.push(format!(
                "Ideologies in {path} are unknown from {cause}; every group it could declare is refused."
            ));
            result.coverage.gap(&path, cause.clone());
        }
        number_groups(&path, &read.items, &mut result.frames);
        // 140A38FE0 resets this file's groups omitted by each later block.
        for root in read
            .items
            .iter()
            .filter(|item| item.keyword_is("ideologies"))
        {
            let (block, _) = groups(std::slice::from_ref(root));
            for existing in &mut result.groups {
                if result
                    .frames
                    .get(&existing.token)
                    .is_some_and(|(_, file)| file == &path)
                    && !block.iter().any(|group| group.token == existing.token)
                {
                    existing.color = Ok(WHITE);
                    existing.types.clear();
                }
            }
            for group in block {
                result.coverage.replace(&group.token);
                match result
                    .groups
                    .iter_mut()
                    .find(|existing| existing.token == group.token)
                {
                    Some(existing) => {
                        existing.color = group.color;
                        existing.types = group.types;
                    }
                    None => result.groups.push(group),
                }
            }
        }
    }
    for group in &result.groups {
        if let Err(cause) = &group.color {
            warnings.push(format!(
                "Ideology {}'s colour is refused: {cause}.",
                group.token
            ));
        }
    }
    Ok((result, warnings))
}

#[cfg(test)]
mod tests;
