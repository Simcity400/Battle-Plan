//! Slot-3 readers outside the default ReadScript and one-value paths.
//! Addresses name executable grammar, never content from a vanilla roster.

use crate::{
    key_names::KeyNames,
    parser::native::{Field, Shape},
};

#[derive(Clone, Copy)]
enum LoopSource {
    Inline,
    ClassKey,
}

impl LoopSource {
    fn address(self, value: u64, key: u64) -> u64 {
        match self {
            Self::Inline => value,
            Self::ClassKey => key,
        }
    }
}

#[derive(Clone, Copy)]
enum Form {
    ScalarOrKeys(LoopSource),
    StringOrKeys(LoopSource),
    Keys(LoopSource),
    Ideas,
    Localization,
}

enum ScalarRead {
    One,
    Block,
    Deferred,
}

// The embedded CBoundLocalization has its own Read, independent of the
// containing effect's slot-4 reader (141384F50 -> 142390E40).
const BOUND_LOCALIZATION_READ: u64 = 0x1423912A0;

// A loop uses either the value reader itself or the class's slot-4 reader.
const VALUE_READERS: &[(u64, Form)] = &[
    (0x1403A89E0, Form::ScalarOrKeys(LoopSource::ClassKey)), // randomize
    (0x1403A9810, Form::Ideas), // raw resolved tokens, not ReadKeyOperatorValue
    (0x1403A9B40, Form::Keys(LoopSource::ClassKey)), // ReadScript thunk
    (0x1403A9C70, Form::ScalarOrKeys(LoopSource::Inline)), // puppet
    (0x1403AB080, Form::ScalarOrKeys(LoopSource::ClassKey)), // focus tree
    (0x1403AB2C0, Form::ScalarOrKeys(LoopSource::Inline)), // character name
    (0x1403AB590, Form::ScalarOrKeys(LoopSource::Inline)), // nationality
    (0x1403AC0D0, Form::Keys(LoopSource::Inline)), // swap ideas
    (0x1403ACCD0, Form::ScalarOrKeys(LoopSource::Inline)), // decision tooltip
    (0x1403ACF40, Form::ScalarOrKeys(LoopSource::Inline)), // white peace
    (0x14047CCE0, Form::Keys(LoopSource::ClassKey)), // highest scored temp
    (0x14047D130, Form::Keys(LoopSource::ClassKey)), // highest scored
    (0x14047D580, Form::Keys(LoopSource::ClassKey)), // sorted scored temp
    (0x14047D9D0, Form::Keys(LoopSource::ClassKey)), // sorted scored
    (0x14047DE20, Form::Keys(LoopSource::ClassKey)), // supply vehicles temp
    (0x14047E6C0, Form::Keys(LoopSource::ClassKey)), // modulo temp
    (0x14047EB10, Form::Keys(LoopSource::ClassKey)), // modulo
    (0x14047EF60, Form::StringOrKeys(LoopSource::ClassKey)), // random temp
    (0x14047F3E0, Form::StringOrKeys(LoopSource::ClassKey)), // random variable
    (0x141384F50, Form::Localization), // embedded CBoundLocalization
    (0x141384F60, Form::ScalarOrKeys(LoopSource::ClassKey)), // event
    (0x141384F80, Form::ScalarOrKeys(LoopSource::ClassKey)), // flag
    (0x141385260, Form::Keys(LoopSource::ClassKey)), // modify flag
    (0x141385770, Form::ScalarOrKeys(LoopSource::ClassKey)), // assignment
];

pub(super) fn shape(value: u64, key: u64, field: &Field<'_>) -> Option<Shape<()>> {
    let form = VALUE_READERS
        .iter()
        .find(|(address, _)| *address == value)?
        .1;
    let (source, scalar) = match form {
        Form::Ideas => {
            // The scalar takes the current lexer token; the raw array
            // resolves tokens without writing the retained value slot.
            return Some(if field.opens() {
                Shape::RawList
            } else {
                Shape::Current
            });
        }
        Form::Localization => {
            return Some(if field.opens() {
                Shape::EffectValueKeys(BOUND_LOCALIZATION_READ)
            } else {
                Shape::One
            });
        }
        Form::ScalarOrKeys(source) => (source, ScalarRead::One),
        Form::StringOrKeys(source) => (source, string_scalar(field)),
        Form::Keys(source) => (source, ScalarRead::Block),
    };
    let reader = source.address(value, key);
    // Shared slots retain named, counted refusals for class grammars
    // absent from the own-key table, on both scalar and block forms.
    own_reader(reader)?;
    if field.opens() {
        return Some(Shape::EffectValueKeys(reader));
    }
    match scalar {
        ScalarRead::One => Some(Shape::One),
        ScalarRead::Deferred => None,
        ScalarRead::Block => Some(Shape::Reported {
            report: format!(
                "effect value reader {value:#X} requires a block; native RecordError (1424ACF90)"
            ),
            read: Box::new(Shape::Skip),
        }),
    }
}

fn string_scalar(field: &Field<'_>) -> ScalarRead {
    use crate::parser::native::Kind;
    use std::{collections::BTreeSet, sync::LazyLock};
    static KEYWORDS: LazyLock<BTreeSet<Vec<u8>>> = LazyLock::new(|| {
        include_str!("../gui/native/keywords.tsv")
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .map(|(_, key)| key.to_ascii_lowercase().into_bytes())
            .collect()
    });
    // Token 0xF selects scalar assignment. Bare names may already be
    // dynamically interned by the recorded content stack; the static
    // catalogue cannot settle that distinction or a stale scalar route.
    if !field.fresh {
        ScalarRead::Deferred
    } else if field.value.kind == Kind::Quoted {
        ScalarRead::One
    } else if field.value.kind == Kind::Bare
        && !field.value.is_number()
        && !KEYWORDS.contains(&field.value.c_str().to_ascii_lowercase())
    {
        ScalarRead::Deferred
    } else {
        ScalarRead::Block
    }
}

#[derive(Clone, Copy)]
enum OwnReader {
    Keys(&'static [&'static str]),
    EffectFallback(&'static [&'static str]),
    One,
    Color,
    Localization,
    Randomize,
    Event(&'static [&'static str]),
}

// Ordinary accepted fields take one string/bool/scoped-variable token;
// unknowns use ReportUnexpectedToken/RecordError, including its skip.
const OWN_KEYS: &[(u64, OwnReader)] = &[
    // Thunks -> 141398390 / 141398350.
    (0x140301200, OwnReader::Keys(&["trait", "show_modifiers"])),
    (0x140301210, OwnReader::Keys(&["policy", "show_modifiers"])),
    (
        0x1403A9C70,
        OwnReader::Keys(&["target", "end_civil_wars", "end_wars"]),
    ),
    (0x1403AB2C0, OwnReader::Keys(&["name", "character"])),
    (
        0x1403AB590,
        OwnReader::Keys(&["target_country", "character"]),
    ),
    (
        0x1403AC0D0,
        OwnReader::Keys(&["days", "remove_idea", "add_idea", "add_days"]),
    ),
    (
        0x1403ACCD0,
        OwnReader::Keys(&["decision", "show_effect_tooltip", "show_modifiers"]),
    ),
    (0x1403ACF40, OwnReader::Keys(&["tooltip", "message", "tag"])),
    // Case-sensitive distribution with mutable allowed fields. Factories
    // 14033A5D0 / 14033A660 choose uniform (+0x130 = 0).
    (0x1403ADE70, OwnReader::Randomize),
    // Remaining keys tail-call CEffect::ReadEffect (1405345D0).
    (
        0x1403AF020,
        OwnReader::EffectFallback(&["ideology", "popularity"]),
    ),
    // Unknown keys return without reporting or skipping.
    (0x1403B34B0, OwnReader::One),
    (
        0x1403B4480,
        OwnReader::Keys(&[
            "target",
            "autonomous_state",
            "autonomy_state",
            "freedom_level",
            "release_non_owned_controlled",
            "force_change_controller_for_non_ally_controlled",
        ]),
    ),
    (
        0x1403B4CC0,
        OwnReader::Keys(&[
            "target",
            "autonomous_state",
            "autonomy_state",
            "freedom_level",
            "end_civil_wars",
            "end_wars",
        ]),
    ),
    (
        0x1403B4F80,
        OwnReader::Keys(&["tree", "keep_completed", "copy_completed_from"]),
    ),
    (0x140480F40, OwnReader::Keys(&["var", "scorer"])),
    (0x140480F80, OwnReader::Keys(&["array", "scores", "scorer"])),
    (0x140480FD0, OwnReader::Keys(&["var", "type", "need"])),
    // -> 140480E00: every field is an operand pair, including shorthand.
    (0x140481020, OwnReader::One),
    (
        0x140481030,
        OwnReader::Keys(&["min", "max", "var", "integer"]),
    ),
    (0x14049E7D0, OwnReader::Keys(&["target", "value"])),
    // Scalar fields or CColor; unknown keys return without an error skip.
    (0x1404B1340, OwnReader::Color),
    // random blocks read a CRandomEffect through the enclosing names.
    (
        0x141385790,
        OwnReader::Event(&[
            "id",
            "tooltip",
            "trigger_for",
            "recipient",
            "months",
            "days",
            "hours",
            "random_hours",
            "random_days",
            "random",
            "set_from",
            "set_root",
            "set_from_from",
            "originator",
        ]),
    ),
    (
        0x1413859E0,
        OwnReader::Event(&[
            "id",
            "tooltip",
            "trigger_for",
            "months",
            "days",
            "hours",
            "random_hours",
            "random_days",
            "random",
            "set_from_from",
            "originator",
        ]),
    ),
    (0x141385A10, OwnReader::Keys(&["flag", "value", "days"])),
    (0x141385E80, OwnReader::Keys(&["flag", "value", "days"])),
    (BOUND_LOCALIZATION_READ, OwnReader::Localization),
];

fn own_reader(address: u64) -> Option<OwnReader> {
    OWN_KEYS
        .iter()
        .find(|(reader, _)| *reader == address)
        .map(|(_, reader)| *reader)
}

pub(crate) struct Shaper<'n> {
    address: u64,
    reader: OwnReader,
    names: Option<&'n KeyNames>,
    distribution: u8,
}

impl<'n> Shaper<'n> {
    pub(crate) fn new(address: u64, names: Option<&'n KeyNames>) -> Self {
        Self {
            address,
            reader: own_reader(address).expect("ported effect value key loop"),
            names,
            distribution: 0,
        }
    }

    pub(crate) fn field(&mut self, field: &Field<'_>) -> Shape<()> {
        match self.reader {
            OwnReader::One => Shape::One,
            OwnReader::Color => {
                if field.keyword_is("color") {
                    Shape::Color
                } else {
                    Shape::One
                }
            }
            OwnReader::Localization => {
                if !field.keyword_is("localization_key") && field.opens() {
                    Shape::EffectValueKeys(self.address)
                } else {
                    Shape::One
                }
            }
            OwnReader::Randomize => self.randomize(field),
            OwnReader::Event(_) if field.keyword_is("random") && field.opens() => {
                super::shape(self.names, field)
            }
            OwnReader::Keys(keys) | OwnReader::Event(keys) => {
                if keys.iter().any(|key| field.keyword_is(key)) {
                    Shape::One
                } else {
                    Shape::Skip
                }
            }
            OwnReader::EffectFallback(keys) => {
                if keys.iter().any(|key| field.keyword_is(key)) {
                    Shape::One
                } else {
                    super::shape(self.names, field)
                }
            }
        }
    }

    fn randomize(&mut self, field: &Field<'_>) -> Shape<()> {
        // strcmp 14253D9F8; literals at 142766390 / 398 / 3A0.
        if field.keyword_is("distribution") {
            let value = match field.value.c_str() {
                b"uniform" => 0,
                b"poisson" => 1,
                b"binomial" => 2,
                _ => return Shape::Skip,
            };
            self.distribution = value;
            Shape::One
        } else if field.keyword_is("min")
            || field.keyword_is("var")
            || (field.keyword_is("max") && self.distribution != 1)
            || (field.keyword_is("lambda") && self.distribution == 1)
        {
            Shape::One
        } else {
            Shape::Skip
        }
    }
}

#[cfg(test)]
mod tests;
