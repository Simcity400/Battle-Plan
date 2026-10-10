//! Ideologies (`common/ideologies`): each ideology group, its colour and its
//! sub-ideologies. The recording names a party by its group token and a leader
//! by a sub-ideology; this joins both to the group and its colour, so a mod's
//! ideologies chart with their own colours.
//!
//! Each file has its own reader replay ([`native`]): `ideologies` opens
//! the group loop, which reads each group and its fields. Any other
//! top-level key stops that file.

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

// Default colours start white.
const WHITE: [f32; 4] = [1.0; 4];

#[derive(Clone, Debug, PartialEq)]
pub struct Ideology {
    pub token: String,
    /// The group's colour as its reads leave it (red, green, blue,
    /// alpha), or why a read the port does not follow makes it unknown.
    pub color: Result<[f32; 4], String>,
    /// Sub-ideologies (`types`), in source order.
    pub types: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ideologies {
    /// In load order.
    groups: Vec<Ideology>,
    /// Each group's frame ([`Ideologies::frame`]) and the file that first read
    /// it, by the token's ASCII case fold (`A`-`Z` joined).
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

    /// The group's frame as raw bits, used by a naval counter's
    /// `counter_ideology` (`number_groups`): its place in the last `ideologies`
    /// block of its file that listed it, or `0xFFFFFFFF` when a later block
    /// of that file reset it. `Ok(None)` when no file defines the group;
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
    /// The file loop.
    Top,
    /// The ideology-group loop.
    Groups,
    /// One ideology group and its fields.
    Group,
    /// The `types` loop: each key a sub-ideology.
    Types,
    /// One sub-ideology and its fields.
    Ideology,
    Rule(RuleNode),
    Modifier(modifiers::Node),
}

/// Dispatch one ideology-group field by its keyword.
fn group_field(field: &Field<'_>) -> Shape<Node> {
    match field.keyword().as_deref() {
        // `color` reads into the group's stored colour.
        Some(b"color") => Shape::Color,
        // `types` opens its own loop.
        Some(b"types") => Shape::Object(Node::Types),
        // `rules` opens the group's rule object; `modifiers` and
        // `faction_modifiers` each read into their own modifier object.
        Some(b"rules") => Shape::Object(Node::Rule(RuleNode::Rule)),
        Some(b"modifiers" | b"faction_modifiers") => {
            Shape::Object(Node::Modifier(modifiers::Node::Modifier))
        }
        // `dynamic_faction_names` reads a string list.
        Some(b"dynamic_faction_names") => Shape::List,
        // Fixed5 fields: `war_impact_on_world_tension`,
        // `faction_impact_on_world_tension`, `ai_ideology_wanted_units_factor`;
        // signed integer: `ai_give_core_state_control_threshold`;
        // boolean fields: `ai_democratic`, `ai_fascist`, `ai_communist`,
        // `can_be_boosted`, `ai_neutral`, `can_collaborate`,
        // `can_host_government_in_exile`.
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
        // Unknown fields are reported and skipped.
        _ => Shape::Skip,
    }
}

/// Duplicates skip their bodies; later blocks in the owning file reread the group.
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
    // One mode-0 lexer per file.
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
            // An unexpected top-level key reports and returns; the next file still loads.
            Node::Top => Shape::StopFile,
            Node::Groups => {
                // Fold once for the stored group identity.
                let key = crate::parser::c_string(&field.key.text).to_ascii_lowercase();
                if let Some(owner) = owners.get(&key)
                    && (owner != path || listed.contains(&key))
                {
                    refusals.push(format!(
                        "Ideologies in {path}: line {}: Duplicate database id `{}` refused; first defined in {}; later block and its sub-ideologies skipped.",
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
            // Each key reads one sub-ideology object.
            Node::Types => Shape::Object(Node::Ideology),
            // `color` reads the sub-ideology colour, and `can_be_randomly_selected`
            // reads a boolean. Any other key goes to its modifier object.
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

/// One `color` read updates the state earlier reads left (initially all
/// 1.0). After `{`, three components are scanned as floating-point text,
/// whatever the token kind; a failed scan keeps the component. Each
/// component, stored or not, is then divided by 255 when above 1.0.
/// Alpha is set to 1.0; a fourth component is the alpha as read, not divided.
/// A number token alone is a packed hexadecimal value, which the port
/// refuses by name; any other token, a quoted one included, is reported
/// and changes nothing.
fn read_color(color: &mut [f32; 4], item: &Item) -> Result<(), String> {
    let Some(components) = item.value.items() else {
        if item.number() {
            return Err(format!(
                "the packed colour {} (read as hexadecimal) is not converted",
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
                // Each read updates the same colour from where the last left it.
                if let Ok(state) = &mut color
                    && let Err(cause) = read_color(state, field)
                {
                    color = Err(cause);
                }
            } else if field.keyword_is("types") {
                // Every `types` loop adds its keys.
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

/// Accepted groups are numbered from 1; omitted groups reset to -1.
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

/// Every ideology group in file order: one reader per enumerated file.
/// The first file defining a group owns it.
pub fn read(layers: &ResolvedLayers) -> io::Result<(Ideologies, Vec<crate::Diagnostic>)> {
    let mut warnings = Vec::new();
    let mut result = Ideologies::default();
    for path in script_files::paths(layers, "common/ideologies/", None) {
        result.coverage.next_file();
        let (read, undecided, refused) = read_file_in(&layers.read(&path)?, &path, &result.frames);
        warnings.extend(
            refused
                .into_iter()
                .map(|message| (crate::DiagnosticKind::Refused, message)),
        );
        warnings.extend(
            read.misreads
                .iter()
                .map(|misread| (misread.kind, format!("Ideologies in {path}: {misread}."))),
        );
        // A modifier block the registry decides, or a block the port cannot
        // settle, leaves the rest of the file unknown: every group the file
        // could define or redefine is refused, with the place and why.
        let (_, unheld) = groups(&read.items);
        let stop = undecided
            .first()
            .map(|cause| (crate::DiagnosticKind::Unsettled, cause.to_string()))
            .or_else(|| {
                read.unsettled
                    .as_ref()
                    .map(|cause| (cause.kind, cause.to_string()))
            })
            .or_else(|| unheld.map(|cause| (crate::DiagnosticKind::Unsettled, cause)));
        if let Some((kind, cause)) = &stop {
            warnings.push((*kind, format!(
                "Ideologies in {path} are unknown from {cause}; every group it could declare is refused."
            )));
            result.coverage.gap(&path, cause.clone());
        }
        number_groups(&path, &read.items, &mut result.frames);
        // Each later block resets this file's omitted groups.
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
            warnings.push((
                crate::DiagnosticKind::Refused,
                format!("Ideology {}'s colour is refused: {cause}.", group.token),
            ));
        }
    }
    Ok((result, warnings))
}

#[cfg(test)]
mod tests;
