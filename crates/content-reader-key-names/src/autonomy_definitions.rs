use content_reader_parser::{fields::invalid, parser::Item};
use content_reader_script::{
    idea_fields,
    key_names::KeyNames,
    modifiers,
    native::{self, Shape},
};
use std::io;

pub fn read_definitions(input: &[u8], names: &KeyNames) -> (Vec<Item>, Option<String>) {
    #[derive(Clone, Copy)]
    enum Node {
        File,
        State,
        Modifier(modifiers::Node),
        Rule(idea_fields::RuleNode),
        Mtth,
    }
    let mut undecided = Vec::new();
    let mut started = 0_usize;
    let mut stop: Option<(usize, String)> = None;
    let read = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::Persistent(Node::File),
        Some(names),
        &mut |node, field| match node {
            Node::File
                if field.keyword_is("autonomy_state") || field.keyword_is("autonomous_state") =>
            {
                started += 1;
                Shape::Object(Node::State)
            }
            Node::File => Shape::Skip,
            Node::State => match field.keyword().as_deref() {
                Some(
                    b"id"
                    | b"default"
                    | b"cost"
                    | b"is_puppet"
                    | b"min_freedom_level"
                    | b"manpower_influence"
                    | b"peace_conference_initial_freedom"
                    | b"use_overlord_color",
                ) => Shape::One,
                Some(
                    b"allowed"
                    | b"can_take_level"
                    | b"can_lose_level"
                    | b"ai_overlord_wants_garrison",
                ) => Shape::Trigger,
                Some(b"modifier") => Shape::Object(Node::Modifier(modifiers::Node::Modifier)),
                Some(b"rule") => Shape::Object(Node::Rule(idea_fields::RuleNode::Rule)),
                Some(b"allowed_levels_filter") => Shape::List,
                Some(b"ai_subject_wants_higher" | b"ai_overlord_wants_lower") => {
                    Shape::Object(Node::Mtth)
                }
                Some(b"use_for_peace_conference_weight") => Shape::ObjectIfOpen(Node::Mtth),
                _ => Shape::Skip,
            },
            Node::Modifier(node) => {
                let shape = modifiers::shape(node, field, modifiers::UNKNOWN, &mut undecided)
                    .map(Node::Modifier);
                if stop.is_none()
                    && let Some(first) = undecided.first()
                {
                    stop = Some((started.saturating_sub(1), first.to_string()));
                }
                shape
            }
            Node::Rule(node) => idea_fields::rule_field(node, field).map(Node::Rule),
            Node::Mtth => native::mean_time_to_happen(field),
        },
    );
    if stop.is_none()
        && let Some(cause) = &read.unsettled
    {
        stop = Some((started.saturating_sub(1), cause.to_string()));
    }
    let kept = stop.as_ref().map_or(usize::MAX, |(kept, _)| *kept);
    let definitions = read
        .items
        .into_iter()
        .filter(|item| item.keyword_is("autonomy_state") || item.keyword_is("autonomous_state"))
        .take(kept)
        .collect();
    (definitions, stop.map(|(_, cause)| cause))
}

pub fn settled_definitions(input: &[u8], names: &KeyNames) -> io::Result<Vec<Item>> {
    match read_definitions(input, names) {
        (definitions, None) => Ok(definitions),
        (_, Some(cause)) => Err(invalid(format!("autonomy definitions: {cause}"))),
    }
}
