
use crate::{
    ResolvedLayers,
    coverage::{Coverage, Gap},
    crt::{Scanned, scan_f32, scan_x32},
    ideas::{RuleNode, rule_field},
    modifiers,
    parser::{
        Item, Unheld,
        native::{self, Field, Shape},
    },
    script_files,
};
use std::{collections::BTreeMap, io};

const WHITE: [f32; 4] = [1.0; 4];

#[derive(Clone, Debug, PartialEq)]
pub struct Ideology {
    pub token: String,
    pub color: Result<[f32; 4], String>,
    pub types: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ideologies {
    groups: Vec<Ideology>,
    frames: BTreeMap<String, (u32, script_files::ScriptPath)>,
    coverage: Coverage,
}

impl Ideologies {
    pub fn get(&self, token: &str) -> Result<Option<&Ideology>, &Gap> {
        let token = token.to_ascii_lowercase();
        match self.coverage.unknown(&token) {
            Some(gap) => Err(gap),
            None => Ok(self
                .groups
                .iter()
                .find(|group| group.token.eq_ignore_ascii_case(&token))),
        }
    }

    pub fn group_of(&self, sub_ideology: &str) -> Result<Option<&Ideology>, &Gap> {
        match self.coverage.any_gap() {
            Some(gap) => Err(gap),
            None => Ok(self.groups.iter().find(|group| {
                group
                    .types
                    .iter()
                    .any(|kind| kind.eq_ignore_ascii_case(sub_ideology))
            })),
        }
    }

    pub fn frame(&self, token: &str) -> Result<Option<u32>, String> {
        if let Some(gap) = self.coverage.any_gap() {
            return Err(gap.to_string());
        }
        Ok(self
            .frames
            .get(&token.to_ascii_lowercase())
            .map(|(frame, _)| *frame))
    }

    pub fn groups(&self) -> (&[Ideology], Option<&Gap>) {
        (&self.groups, self.coverage.any_gap())
    }

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    Top,
    Groups,
    Group,
    Types,
    Ideology,
    Rule(RuleNode),
    Modifier(modifiers::Node),
}

fn group_field(field: &Field<'_>) -> Shape<Node> {
    match field.keyword().as_deref() {
        Some(b"color") => Shape::Color,
        Some(b"types") => Shape::Object(Node::Types),
        Some(b"rules") => Shape::Object(Node::Rule(RuleNode::Rule)),
        Some(b"modifiers" | b"faction_modifiers") => {
            Shape::Object(Node::Modifier(modifiers::Node::Modifier))
        }
        Some(b"dynamic_faction_names") => Shape::List,
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
        _ => Shape::Skip,
    }
}

fn read_file_in(
    input: &[u8],
    path: &script_files::ScriptPath,
    frames: &BTreeMap<String, (u32, script_files::ScriptPath)>,
) -> (native::Read, Vec<modifiers::Undecided>, Vec<String>) {
    let mut undecided = Vec::new();
    let mut owners: BTreeMap<Vec<u8>, script_files::ScriptPath> = frames
        .iter()
        .map(|(key, (_, file))| (key.as_bytes().to_vec(), file.clone()))
        .collect();
    let mut listed = Vec::new();
    let mut refusals = Vec::new();
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
            Node::Top => Shape::StopFile,
            Node::Groups => {
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
                    owners.entry(key.clone()).or_insert_with(|| path.clone());
                    listed.push(key.clone());
                    Shape::ObjectAs(Node::Group, key)
                }
            }
            Node::Group => group_field(field),
            Node::Types => Shape::Object(Node::Ideology),
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

#[derive(Clone, Debug, PartialEq)]
pub struct ColorReads {
    color: [f32; 4],
}

impl Default for ColorReads {
    fn default() -> Self {
        Self { color: WHITE }
    }
}

impl ColorReads {
    pub fn read(&mut self, item: &Item) {
        let Some(components) = item.value.items() else {
            if item.number() {
                let value = match scan_x32(item.value.scalar().unwrap_or_default()) {
                    Scanned::Stored(value) => Some(value),
                    Scanned::End => Some(0),
                    Scanned::NoMatch => None,
                };
                if let Some(value) = value {
                    let [b, g, r, a] = value.to_le_bytes();
                    self.color = [r, g, b, a].map(|byte| f32::from(byte) / 255.0);
                }
            }
            return;
        };
        for (index, component) in components.iter().enumerate().take(4) {
            let text = component.value.scalar().unwrap_or_default();
            let scanned = scan_f32(text);
            if index == 3 {
                if let Some(alpha) = scanned {
                    self.color[3] = alpha;
                }
                continue;
            }
            if let Some(value) = scanned {
                self.color[index] = value;
            }
            if self.color[index] > 1.0 {
                self.color[index] /= 255.0;
            }
            if index == 2 {
                self.color[3] = 1.0;
            }
        }
    }

    pub fn color(&self) -> Result<[f32; 4], String> {
        Ok(self.color)
    }
}

fn groups(items: &[Item]) -> (Vec<Ideology>, Option<String>) {
    let mut groups = Vec::new();
    let roots = items.iter().filter(|item| item.keyword_is("ideologies"));
    for item in roots.flat_map(|root| root.value.items().unwrap_or_default()) {
        let mut unheld = Unheld::default();
        let token = unheld.keep("an ideology group's token", item.key_str());
        let mut color = ColorReads::default();
        let mut types: Vec<String> = Vec::new();
        for field in item.value.items().unwrap_or_default() {
            if field.keyword_is("color") {
                color.read(field);
            } else if field.keyword_is("types") {
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
                color: color.color(),
                types,
            }),
            (_, cause) => return (groups, cause.map(str::to_owned)),
        }
    }
    (groups, None)
}

fn number_groups(
    path: &script_files::ScriptPath,
    items: &[Item],
    frames: &mut BTreeMap<String, (u32, script_files::ScriptPath)>,
) {
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
            frames.insert(key.to_owned(), (place, path.clone()));
        }
    }
}

pub fn parse(input: &[u8]) -> Vec<Ideology> {
    let path = script_files::ScriptPath::new(Vec::new());
    groups(&read_file_in(input, &path, &BTreeMap::new()).0.items).0
}

pub fn read(layers: &ResolvedLayers) -> io::Result<(Ideologies, Vec<crate::Diagnostic>)> {
    layers.recorded_product("ideologies", 1, &[], read_ideologies).map(|ideologies| ideologies.as_ref().clone())
}

fn read_ideologies(layers: &ResolvedLayers) -> io::Result<(Ideologies, Vec<crate::Diagnostic>)> {
    let mut warnings = Vec::new();
    let mut result = Ideologies::default();
    for listed in script_files::paths(layers, "common/ideologies/", None)? {
        let path = listed.to_string();
        result.coverage.next_file();
        let (read, undecided, refused) = read_file_in(&listed.read(layers)?, &listed, &result.frames);
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
        number_groups(&listed, &read.items, &mut result.frames);
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
                    .is_some_and(|(_, file)| file == &listed)
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
