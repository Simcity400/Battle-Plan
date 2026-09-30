//! Named static modifiers from `common/modifiers`, read as the engine loads them.
//!
//! `1406007E0` enumerates the directory with `Vfs_EnumerateRecursive@1424C7630`
//! and passes zero as its recursion argument (`14060085B`-`14060086F`, filter
//! `.txt` at `142700814`); `1424C5170` enters subdirectories only when that
//! argument is nonzero. This is [`ResolvedLayers::directory_files`]: the merged
//! directory of the content stack, `replace_path` honoured, files in byte order
//! of their names, no nested files.
//!
//! Each file gets its own lexer in mode 0 (`CTextLexer::constructor@1424A63A0`
//! with `R8D` zeroed at `140600944`, called at `140600950`) and its own CReader
//! (`1424A9D30`), read through the reader replay ([`native`]), so `@name`
//! definitions are the file's own and a reference resolves through the
//! reader's table (`1424AC830`, `1424AB540`). `BindStaticModifierSlots@140600CF0`
//! reads the file until its end: every top-level key (`ReadKeyOperatorValue@1424AC5E0`)
//! constructs a new `CStaticModifier` named by the key's text up to its NUL
//! (`140549860`), pushed to the front of its hash bucket ("Duplicate modifier
//! ID" for a repeat), and reads it through `CPersistent::Read@1424A9730` (vtable
//! `+0x18`) with `CStaticModifier::vf4@1406039B0` ([`modifiers::Node::Targeted`]).
//! `FindStaticModifierOrNullObject@140550940` compares the name bytes exactly and
//! returns the first bucket match, so the last definition wins. An absent name
//! binds the null object, whose accumulator is empty.
//!
//! Inside a definition, CModifier's dispatch (`1406031A0`) hands every other
//! key to `1406042A0`, which [`modifiers`] models: the modifier is found by the
//! key's token without case (the lexer folds a bare key, `1424A7A90`; a quoted
//! one goes through `FindTokenFromSsoString@1424A7060`), its destination is
//! zeroed and `CReader::ReadFixed5@1424ABB10` (`read_fixed5@1424B0480`) converts
//! the value, so a value it cannot convert stores 0; a repeated key replaces
//! the earlier value. `hidden_modifier` entries are stored in the same
//! accumulator. The consumer (`14195EEC0`) reads the definition's own
//! accumulator (`+0x10`, `modifier_accumulator::value@140551820`), so an
//! `added_modifier` child is not read, and an absent key is zero.
//!
//! Refused by file, modifier and key, never read as zero or replaced with the
//! shipped value:
//! - a file whose reading the port cannot follow ([`modifiers::Undecided`]: a
//!   block under a modifier name, which the content alone cannot say the game
//!   reads as a value or skips; or [`native::Read::unsettled`]): the rest of
//!   that file could define any name, so a request stays refused unless a
//!   later file defines its modifier;
//! - a definition with a `key` or `tag` field: `1406039B0` renames the static
//!   modifier (`+0x1A8`) after `140600CF0` bucketed it under its first name, so
//!   which lookup finds it depends on the database's bucket layout, which the
//!   port does not model. The requests for either name are refused;
//! - a requested definition holding text the port cannot keep
//!   ([`modifiers::Modifiers::unheld`]).

use crate::{
    ResolvedLayers,
    modifiers::{self, Undecided},
    parser::{
        self, Item,
        native::{self, Shape},
    },
};
use std::io;

/// The native directory `1406007E0` is called with.
pub const DIRECTORY: &str = "common/modifiers";

/// One requested modifier value of one named static modifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request<'a> {
    pub modifier: &'a str,
    pub key: &'a str,
}

/// A requested value the reader cannot convert exactly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// The content path of the effective definition, or of the file whose
    /// reading stops.
    pub file: String,
    pub modifier: String,
    pub key: String,
    pub reason: String,
}

/// A requested value: raw fixed5 (zero when the modifier or key is absent),
/// or the named refusal.
pub type Resolved = Result<i64, Refusal>;

/// What [`read`] gives.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Values {
    /// Each request's value, in request order.
    pub values: Vec<Resolved>,
    /// Each place the game's reading leaves the text ([`native::Read::misreads`]),
    /// as `path: line N: what`; the values follow the game's reading.
    pub notes: Vec<String>,
}

/// Reads each request, in request order, from the effective definitions of
/// the content stack.
pub fn read(layers: &ResolvedLayers, requests: &[Request<'_>]) -> io::Result<Values> {
    let mut files = Vec::new();
    for path in layers.directory_files(DIRECTORY, ".txt")? {
        let bytes = layers.read_uncovered(&path)?;
        files.push((path, bytes));
    }
    Ok(resolve(
        files
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice())),
        requests,
    ))
}

/// One file as `140600CF0` reads it: its definitions up to the stop, and the
/// stop.
struct File {
    definitions: Vec<Item>,
    stop: Option<String>,
    notes: Vec<String>,
}

#[derive(Clone, Copy)]
enum Node {
    /// `140600CF0`'s file loop.
    Top,
    Modifier(modifiers::Node),
}

fn read_file(bytes: &[u8]) -> File {
    let mut undecided: Vec<Undecided> = Vec::new();
    // Each definition's position among those read, when it began.
    let mut entries = 0_usize;
    let mut stop: Option<(usize, String)> = None;
    let read = native::read(
        bytes,
        native::Mode::KeepMark,
        Node::Top,
        &mut |node, field| {
            let shape = match node {
                // Every key is a CStaticModifier read by CPersistent::Read,
                // whatever its value.
                Node::Top => {
                    entries += 1;
                    Shape::Object(Node::Modifier(modifiers::Node::Targeted))
                }
                Node::Modifier(modifier) => {
                    modifiers::shape(modifier, field, modifiers::UNKNOWN, &mut undecided)
                        .map(Node::Modifier)
                }
            };
            if stop.is_none()
                && let Some(first) = undecided.first()
            {
                stop = Some((entries.saturating_sub(1), first.to_string()));
            }
            shape
        },
    );
    if stop.is_none()
        && let Some(unsettled) = &read.unsettled
    {
        stop = Some((entries.saturating_sub(1), unsettled.to_string()));
    }
    let known = stop.as_ref().map_or(read.items.len(), |(at, _)| *at);
    File {
        definitions: read.items.into_iter().take(known).collect(),
        stop: stop.map(|(_, reason)| reason),
        notes: read.misreads.iter().map(ToString::to_string).collect(),
    }
}

/// [`read`] over files already in native order.
fn resolve<'a>(
    files: impl IntoIterator<Item = (&'a str, &'a [u8])>,
    requests: &[Request<'_>],
) -> Values {
    let mut values: Vec<Resolved> = vec![Ok(0); requests.len()];
    let mut notes = Vec::new();
    for (path, bytes) in files {
        let file = read_file(bytes);
        let refuse = |request: &Request<'_>, reason: String| Refusal {
            file: path.to_owned(),
            modifier: request.modifier.to_owned(),
            key: request.key.to_owned(),
            reason,
        };
        for definition in &file.definitions {
            let name = parser::c_string(definition.key.as_deref().unwrap_or_default());
            let items = definition.value.items().unwrap_or_default();
            let renamed: Vec<&[u8]> = items
                .iter()
                .filter(|item| item.keyword_is("key") || item.keyword_is("tag"))
                .map(|item| item.value.scalar().unwrap_or_default())
                .collect();
            for (value, request) in values.iter_mut().zip(requests) {
                let modifier = request.modifier.as_bytes();
                if !renamed.is_empty() && (name == modifier || renamed.contains(&modifier)) {
                    *value = Err(refuse(
                        request,
                        format!(
                            "definition {} has a key or tag field, which renames it after it is bucketed under its first name (1406039B0, 140600CF0); the bucket layout that decides the lookup is not modelled",
                            String::from_utf8_lossy(name)
                        ),
                    ));
                } else if name == modifier {
                    *value = stored(items, request.key).map_err(|reason| refuse(request, reason));
                }
            }
        }
        if let Some(stop) = &file.stop {
            for (value, request) in values.iter_mut().zip(requests) {
                *value = Err(refuse(
                    request,
                    format!("{stop}; any definition the rest of the file makes is unknown"),
                ));
            }
        }
        notes.extend(file.notes.iter().map(|note| format!("{path}: {note}")));
    }
    Values { values, notes }
}

/// The requested key's value in one definition's accumulator.
fn stored(items: &[Item], key: &str) -> Result<i64, String> {
    let definition = modifiers::targeted(items).modifiers;
    if let Some(cause) = definition.unheld.cause() {
        return Err(format!(
            "the definition holds text the port cannot keep: {cause}"
        ));
    }
    let name = key.to_ascii_lowercase();
    Ok(definition
        .values
        .iter()
        .find(|modifier| modifier.name == name)
        .map_or(0, |modifier| modifier.value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::Path};

    const SCREENING: [Request<'static>; 2] = [
        Request {
            modifier: "screening_bonus",
            key: "sortie_efficiency",
        },
        Request {
            modifier: "capital_screening_bonus",
            key: "fighter_sortie_efficiency",
        },
    ];

    fn resolved(files: &[(&str, &str)]) -> Vec<Resolved> {
        resolve(
            files.iter().map(|(path, text)| (*path, text.as_bytes())),
            &SCREENING,
        )
        .values
    }

    fn first(text: &str) -> Resolved {
        resolved(&[("common/modifiers/a.txt", text)])
            .into_iter()
            .next()
            .unwrap()
    }

    #[test]
    fn the_last_definition_and_the_last_value_win_and_absence_is_zero() {
        assert_eq!(resolved(&[]), [Ok(0), Ok(0)]);
        let values = resolved(&[
            (
                "common/modifiers/a.txt",
                "screening_bonus = { sortie_efficiency = 0.5 }
                 capital_screening_bonus = { fighter_sortie_efficiency = 0.25 }",
            ),
            (
                "common/modifiers/b.txt",
                // A later definition replaces the whole earlier one; its key
                // is then absent. A repeated key keeps its last value. The
                // name is compared exactly (140550940).
                "capital_screening_bonus = { sortie_efficiency = 1 }
                 screening_bonus = { sortie_efficiency = 0.1 sortie_efficiency = -0.02 }
                 Screening_Bonus = { sortie_efficiency = 9 }",
            ),
        ]);
        assert_eq!(values, [Ok(-2_000), Ok(0)]);
    }

    /// 1424AB540 resolves a reference through the file's own table before
    /// ReadFixed5 converts it.
    #[test]
    fn a_variable_reads_its_definitions_value() {
        assert_eq!(
            first("@bonus = 0.5 screening_bonus = { sortie_efficiency = @bonus }"),
            Ok(50_000)
        );
        // A definition inside the block is the same file's table.
        assert_eq!(
            first("screening_bonus = { @b = 0x10 sortie_efficiency = @b }"),
            Ok(1_600_000)
        );
        // Each file has its own reader, so another file's definition is
        // unknown here and the reference does not convert.
        let values = resolved(&[
            ("common/modifiers/a.txt", "@bonus = 0.5"),
            (
                "common/modifiers/b.txt",
                "screening_bonus = { sortie_efficiency = @bonus }",
            ),
        ]);
        assert_eq!(values[0], Ok(0));
    }

    /// 1406042A0 finds the modifier by the key's token: the lexer folds a
    /// bare key (1424A7A90) and FindTokenFromSsoString (1424A7060) looks a
    /// quoted one up without case.
    #[test]
    fn a_modifier_key_is_found_whatever_its_case() {
        assert_eq!(
            first("screening_bonus = { Sortie_Efficiency = 0.1 }"),
            Ok(10_000)
        );
        assert_eq!(
            first("screening_bonus = { sortie_efficiency = 0.1 \"SORTIE_EFFICIENCY\" = 0.2 }"),
            Ok(20_000)
        );
    }

    /// 1406042A0 zeroes the destination before ReadFixed5, so a value it
    /// cannot convert stores 0, replacing an earlier value of the key.
    #[test]
    fn a_value_that_does_not_convert_stores_zero() {
        for text in ["yes", ".5", "@undefined", "[?x]"] {
            assert_eq!(
                first(&format!(
                    "screening_bonus = {{ sortie_efficiency = {text} }}"
                )),
                Ok(0),
                "{text}"
            );
        }
        assert_eq!(
            first("screening_bonus = { sortie_efficiency = 0.5 sortie_efficiency = yes }"),
            Ok(0)
        );
    }

    /// The accumulator 14195EEC0 reads holds `hidden_modifier` entries but
    /// not an `added_modifier` child.
    #[test]
    fn hidden_values_count_and_a_child_modifier_does_not() {
        assert_eq!(
            first("screening_bonus = { hidden_modifier = { sortie_efficiency = 0.3 } }"),
            Ok(30_000)
        );
        assert_eq!(
            first("screening_bonus = { added_modifier = { sortie_efficiency = 0.3 } }"),
            Ok(0)
        );
    }

    #[test]
    fn an_unfollowed_reading_or_a_rename_is_refused_by_file_modifier_and_key() {
        // A block under a modifier name: read as its value when the name is
        // registered, skipped otherwise; content alone does not say.
        let values = resolved(&[(
            "common/modifiers/x.txt",
            "capital_screening_bonus = { fighter_sortie_efficiency = 0.2 }
             screening_bonus = { sortie_efficiency = { value = 1 } }",
        )]);
        for (value, request) in values.iter().zip(&SCREENING) {
            let refusal = value.as_ref().unwrap_err();
            assert_eq!(
                (
                    refusal.file.as_str(),
                    refusal.modifier.as_str(),
                    refusal.key.as_str()
                ),
                ("common/modifiers/x.txt", request.modifier, request.key)
            );
            assert!(
                refusal
                    .reason
                    .contains("sortie_efficiency has a block value")
                    && refusal
                        .reason
                        .ends_with("the rest of the file makes is unknown"),
                "{}",
                refusal.reason
            );
        }
        // A later file's definition supersedes a refused one.
        let values = resolved(&[
            (
                "common/modifiers/x.txt",
                "screening_bonus = { sortie_efficiency = { value = 1 } }",
            ),
            (
                "common/modifiers/y.txt",
                "screening_bonus = { sortie_efficiency = 0.1 }",
            ),
        ]);
        assert_eq!(values[0], Ok(10_000));
        // A rename, under either name.
        for text in [
            "screening_bonus = { key = other sortie_efficiency = 0.1 }",
            "other = { tag = screening_bonus sortie_efficiency = 0.1 }",
        ] {
            let refusal = first(text).unwrap_err();
            assert!(refusal.reason.contains("renames it"), "{text}");
        }
    }

    #[test]
    fn values_convert_with_the_native_fixed5_reader() {
        for (text, raw) in [
            ("0", 0),
            ("1", 100_000),
            ("+0.5", 50_000),
            ("-0.5", -50_000),
            ("0.00001", 1),
            ("-1.25", -125_000),
            ("0.010", 1_000),
            ("1.", 100_000),
            // %lli radix prefixes: a leading zero is octal, 0x hexadecimal.
            ("010", 800_000),
            ("-010.5", -850_000),
            ("0x10", 1_600_000),
            // Fraction digits past the fifth are truncated.
            ("0.123456", 12_345),
            ("-0.000019", -1),
        ] {
            let text = format!("screening_bonus = {{ sortie_efficiency = {text} }}");
            assert_eq!(first(&text), Ok(raw), "{text}");
        }
    }

    fn write(root: &Path, files: &[(&str, &str)]) {
        for (path, text) in files {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
    }

    #[test]
    fn the_stack_is_read_without_nested_files_and_honours_replacement() {
        let root = std::env::temp_dir().join(format!(
            "hoi4_installed_content_static_modifiers_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        let (base, over, replacing) = (root.join("base"), root.join("over"), root.join("rep"));
        write(
            &base,
            &[
                (
                    "common/modifiers/00_static_modifiers.txt",
                    "screening_bonus = { sortie_efficiency = 0.1 }
                     capital_screening_bonus = { fighter_sortie_efficiency = 0.2 }",
                ),
                // The engine does not enter subdirectories.
                (
                    "common/modifiers/nested/zz.txt",
                    "screening_bonus = { sortie_efficiency = 0.9 }",
                ),
            ],
        );
        write(
            &over,
            &[
                // The same path replaces the lower layer's whole file.
                (
                    "common/modifiers/00_static_modifiers.txt",
                    "capital_screening_bonus = { fighter_sortie_efficiency = 0.25 }",
                ),
                (
                    "common/modifiers/zz_mod.txt",
                    "screening_bonus = { sortie_efficiency = 0.3 }",
                ),
                (
                    "common/modifiers/zz_mod/zzz.txt",
                    "screening_bonus = { sortie_efficiency = 0.7 }",
                ),
            ],
        );
        write(
            &replacing,
            &[
                (
                    "common/modifiers/only.txt",
                    "capital_screening_bonus = { fighter_sortie_efficiency = 0.4 }",
                ),
                (
                    "common/modifiers/deeper/zz.txt",
                    "screening_bonus = { sortie_efficiency = 0.8 }",
                ),
            ],
        );
        let stack = |layers: &[(&Path, &[&str])]| ResolvedLayers::from_folders(layers).unwrap();
        let none: &[&str] = &[];
        let replaced: &[&str] = &["common/modifiers"];
        let values = |layers: &ResolvedLayers| read(layers, &SCREENING).unwrap().values;
        let layers = stack(&[(&base, none)]);
        assert_eq!(values(&layers), [Ok(10_000), Ok(20_000)]);
        let layers = stack(&[(&base, none), (&over, none)]);
        assert_eq!(values(&layers), [Ok(30_000), Ok(25_000)]);
        // replace_path hides every lower file of the directory.
        let layers = stack(&[(&base, none), (&over, none), (&replacing, replaced)]);
        assert_eq!(values(&layers), [Ok(0), Ok(40_000)]);
        fs::remove_dir_all(&root).unwrap();
    }
}
