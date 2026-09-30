//! Ideologies (`common/ideologies`): each ideology group, its colour and its
//! sub-ideologies. The recording names a party by its group token and a leader
//! by a sub-ideology; this joins both to the group and its colour, so a mod's
//! ideologies chart with their own colours.
//!
//! Each file is read by its own CReader through the reader replay
//! ([`native`]): CIdeologyGroupDatabase::vf6 (140A38A00) reads `ideologies`
//! (0x2FD1) through its group loop (140A38190), each group a CIdeologyGroup
//! (CIdeologyGroup::vf4 14117C9C0), and reports any other top-level key.

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
use std::io;

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
        Self { groups, coverage }
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

/// One file as its reader reads it, and the misreads and refusal to note.
fn read_file(input: &[u8]) -> (native::Read, Vec<modifiers::Undecided>) {
    let mut undecided = Vec::new();
    // constructs the lexer in mode 0 (CTextLexer 1424A6370).
    let read = native::read(
        input,
        native::Mode::KeepMark,
        Node::Top,
        &mut |node, field| match node {
            Node::Top if field.keyword_is("ideologies") => Shape::Object(Node::Groups),
            // ReportUnexpectedToken (1424AD100).
            Node::Top => Shape::Skip,
            // Each key a CIdeologyGroup read through its Read (CPersistent::Read).
            Node::Groups => Shape::Object(Node::Group),
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
    (read, undecided)
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
        let mut color = Ok([1.0; 4]);
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

/// Parses one ideologies file.
pub fn parse(input: &[u8]) -> Vec<Ideology> {
    groups(&read_file(input).0.items).0
}

/// Every ideology group in load order; a group defined again replaces the earlier one in place.
pub fn read(layers: &ResolvedLayers) -> io::Result<(Ideologies, Vec<String>)> {
    let mut warnings = Vec::new();
    let mut result = Ideologies::default();
    for path in script_files::paths(layers, "common/ideologies/", None) {
        result.coverage.next_file();
        let (read, undecided) = read_file(&layers.read(&path)?);
        warnings.extend(
            read.misreads
                .iter()
                .map(|misread| format!("Ideologies in {path}: {misread}.")),
        );
        // A modifier block the registry decides, or a block the port cannot
        // settle, leaves the rest of the file unknown: every group the file
        // could define or redefine is refused, with the place and why.
        let (declared, unheld) = groups(&read.items);
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
        for group in declared {
            result.coverage.replace(&group.token);
            match result
                .groups
                .iter_mut()
                .find(|existing| existing.token == group.token)
            {
                Some(existing) => *existing = group,
                None => result.groups.push(group),
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
mod tests {
    use super::*;

    fn parsed(text: &[u8]) -> Ideologies {
        Ideologies {
            groups: parse(text),
            ..Ideologies::default()
        }
    }

    fn color(ideologies: &Ideologies, token: &str) -> Result<[f32; 4], String> {
        ideologies.get(token).unwrap().unwrap().color.clone()
    }

    /// CColor::vf3 (142237160): three `%f` components, each divided by 255
    /// when above 1.0, the alpha set to 1.0; a fourth component is the alpha
    /// as read. No `color` leaves CColor::construct's white (142236FA0).
    #[test]
    fn groups_keep_colours_and_sub_ideologies() {
        let ideologies = parsed(
            b"ideologies = { democratic = { types = { conservatism = { } liberalism = { } } color = { 0 0 255 } rules = { a = yes } } \
              fascism = { types = { nazism = { } } color = { 150 75 0 128 } } odd = { color = { 0.1 0.5 0.5 } } \
              red = { color = { 1 0 0 } } plain = { } }",
        );
        assert_eq!(color(&ideologies, "democratic"), Ok([0.0, 0.0, 1.0, 1.0]));
        assert_eq!(
            color(&ideologies, "fascism"),
            Ok([150.0 / 255.0, 75.0 / 255.0, 0.0, 128.0])
        );
        assert_eq!(color(&ideologies, "odd"), Ok([0.1, 0.5, 0.5, 1.0]));
        // 1 is not above 1.0: full red, not 1/255.
        assert_eq!(color(&ideologies, "red"), Ok([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(color(&ideologies, "plain"), Ok([1.0; 4]));
        assert_eq!(
            ideologies.group_of("liberalism").unwrap().unwrap().token,
            "democratic"
        );
        assert!(ideologies.group_of("unknown").unwrap().is_none());
    }

    /// A later `color` reads into the same CColor from where the last left
    /// it, a component `%f` cannot read keeps its value, a `{`-less form is
    /// refused by name when it is a packed number (`%x`, 1424B01A0) and
    /// otherwise changes nothing (reported), and a variable resolves first.
    #[test]
    fn colours_follow_ccolors_reads_in_order() {
        let ideologies = parsed(
            b"@r = 2 ideologies = { a = { color = { 0 0 255 } COLOR = { @r x 0.5 } } b = { color = 16711680 } \
              c = { color = { 1tail 0 0 } } }",
        );
        assert_eq!(color(&ideologies, "a"), Ok([2.0 / 255.0, 0.0, 0.5, 1.0]));
        assert!(
            color(&ideologies, "b")
                .unwrap_err()
                .contains("the packed colour 16711680")
        );
        assert_eq!(color(&ideologies, "c"), Ok([1.0, 0.0, 0.0, 1.0]));
        // The division follows the attempted store, so a component `%f`
        // cannot read is divided again when above 1.0; a quoted number is
        // no number token (0xF): reported, the colour unchanged.
        let ideologies = parsed(
            b"ideologies = { d = { color = { 510 0 0 } color = { bad 0 0 } } \
              e = { color = \"123\" } f = { color = { \"12\" 0 0 } } }",
        );
        assert_eq!(color(&ideologies, "d"), Ok([2.0 / 255.0, 0.0, 0.0, 1.0]));
        assert_eq!(color(&ideologies, "e"), Ok([1.0; 4]));
        assert_eq!(color(&ideologies, "f"), Ok([12.0 / 255.0, 0.0, 0.0, 1.0]));
    }

    /// A group whose token is not UTF-8 is a named gap, never dropped.
    #[test]
    fn a_group_that_is_not_text_stops_the_file() {
        let (read, _) = read_file(b"ideologies = { a = { } \"\xfe\" = { } b = { } }");
        let (found, cause) = groups(&read.items);
        assert_eq!(found.len(), 1);
        assert_eq!(
            cause.as_deref(),
            Some(
                "an ideology group's token is the stored text `\\xfe` is not UTF-8, which the port's text cannot hold"
            )
        );
    }

    /// CColor (142237160) reads a number token or `{`: `rgb` is reported
    /// and nothing more read, so the block after it is read as the group's
    /// fields and its `}` ends the group.
    #[test]
    fn a_tagged_colour_is_not_read_by_ccolor() {
        let (read, _) = read_file(
            b"ideologies = { fascism = { color = rgb { 150 75 0 } types = { nazism = { } } } }",
        );
        let (groups, _) = groups(&read.items);
        assert_eq!(groups[0].token, "fascism");
        assert_eq!(groups[0].color, Ok([1.0; 4]));
        assert!(groups[0].types.is_empty());
        assert!(
            read.misreads[0]
                .what
                .contains("where CColor reads a number or `{`"),
            "{:?}",
            read.misreads
        );
    }

    /// A sub-ideology's other keys go to its CModifier (14117C960 -> +0x40):
    /// a block value there is a modifier block the registry decides.
    #[test]
    fn a_sub_ideologys_other_keys_are_its_modifier() {
        let (read, undecided) = read_file(
            b"ideologies = { a = { types = { x = { political_power_factor = 0.1 odd = { @v = 1 } } } } }",
        );
        assert!(read.misreads.is_empty(), "{:?}", read.misreads);
        assert_eq!(undecided[0].name, "odd");
    }

    /// The replay's classes through this reader: a quoted key is no keyword
    /// (1424A7A90) and a bare one folds; a variable holds within its file;
    /// a block where ReadStringList or one value is read is read as the
    /// reader reads it; the end of the text closes every loop.
    #[test]
    fn the_replay_classes_hold_for_ideologies() {
        let (read, _) = read_file(
            b"@blue = 255 ideologies = { a = { \"color\" = { 1 2 3 } COLOR = { 0 0 @blue } Types = { x = { } } } \
              b = { can_be_boosted = { color = { 9 9 9 } } } }",
        );
        let (found, _) = groups(&read.items);
        assert_eq!(found[0].color, Ok([0.0, 0.0, 1.0, 1.0]));
        assert_eq!(found[0].types, ["x"]);
        // `can_be_boosted` read `{`; the block's colour is b's own and its
        // `}` ended b.
        assert_eq!(
            found[1].color,
            Ok([9.0 / 255.0, 9.0 / 255.0, 9.0 / 255.0, 1.0])
        );
        assert!(
            read.misreads[0]
                .what
                .contains("field `can_be_boosted` is a block")
        );
        let (read, _) = read_file(b"ideologies = { c = { types = { y = { } ");
        assert!(read.misreads.is_empty());
        assert_eq!(groups(&read.items).0[0].types, ["y"]);
    }
}
