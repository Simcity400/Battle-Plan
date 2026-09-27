//! Characters (`common/characters`): authored portraits, advisor roles and
//! country leader roles.
//!
//! A character may be written as several conditional `instance` alternatives
//! (for example one per DLC). No trigger is evaluated, so an authored value is
//! used only when every alternative agrees on it; alternatives that disagree
//! make it [`Authored::Ambiguous`], and definitions of the same token in several
//! files must agree the same way.

use crate::{
    ResolvedLayers,
    parser::{self, Item, Value},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

/// One authored value (a portrait sprite name) as every alternative gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Authored {
    Missing,
    Asset(String),
    /// Alternatives that do not agree, with every value any of them names.
    Ambiguous(BTreeSet<String>),
}

impl Authored {
    /// Every value this can name.
    pub fn names(&self) -> BTreeSet<String> {
        match self {
            Self::Missing => BTreeSet::new(),
            Self::Asset(name) => BTreeSet::from([name.clone()]),
            Self::Ambiguous(names) => names.clone(),
        }
    }

    /// Combines another alternative: equal stays, different becomes ambiguous.
    pub fn agree(&mut self, other: Self) {
        if *self != other {
            let mut names = self.names();
            names.extend(other.names());
            *self = Self::Ambiguous(names);
        }
    }
}

fn values<'a>(items: &'a [Item], key: &'a [u8]) -> impl Iterator<Item = &'a Value> {
    items
        .iter()
        .filter(move |item| item.key.as_deref() == Some(key))
        .map(|item| &item.value)
}

fn direct_portrait(items: &[Item], category: &[u8], size: &[u8]) -> Authored {
    let mut names = BTreeSet::new();
    for portraits in values(items, b"portraits") {
        let Some(portraits) = portraits.items() else {
            return Authored::Ambiguous(names);
        };
        for group in values(portraits, category) {
            let Some(group) = group.items() else {
                return Authored::Ambiguous(names);
            };
            for sprite in values(group, size) {
                let Some(name) = sprite.scalar_str().filter(|name| !name.is_empty()) else {
                    return Authored::Ambiguous(names);
                };
                names.insert(name.to_owned());
            }
        }
    }
    match names.len() {
        0 => Authored::Missing,
        1 => Authored::Asset(names.pop_first().unwrap()),
        _ => Authored::Ambiguous(names),
    }
}

/// A character's portrait of one category (`army`, `civilian`, …) and size
/// (`large`, `small`), agreed across its instances.
fn authored(items: &[Item], category: &[u8], size: &[u8]) -> Authored {
    let direct = direct_portrait(items, category, size);
    let mut alternatives = values(items, b"instance").map(|instance| {
        instance
            .items()
            .map(|items| authored(items, category, size))
            .unwrap_or(Authored::Ambiguous(BTreeSet::new()))
    });
    let Some(mut result) = alternatives.next() else {
        return direct;
    };
    for alternative in alternatives {
        result.agree(alternative);
    }
    // A parent portrait plus an instance without one would require a proven
    // inheritance rule. Do not silently supply that instance's missing image.
    if direct != Authored::Missing {
        result.agree(direct);
    }
    result
}

fn characters_root(document: &parser::Document) -> io::Result<Vec<(&str, &[Item])>> {
    let mut result = Vec::new();
    for root in values(&document.items, b"characters") {
        let Some(root) = root.items() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "characters is not a block",
            ));
        };
        for character in root {
            let (Some(token), Some(items)) = (&character.key, character.value.items()) else {
                continue;
            };
            let token = std::str::from_utf8(token).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "character token is not UTF-8")
            })?;
            result.push((token, items));
        }
    }
    Ok(result)
}

/// Every character's portrait of one category and size in one file.
pub fn portraits(
    input: &[u8],
    category: &[u8],
    size: &[u8],
) -> io::Result<BTreeMap<String, Authored>> {
    let (document, _) = parser::parse_unclosed(input)?;
    let mut characters = BTreeMap::<String, Authored>::new();
    for (token, items) in characters_root(&document)? {
        let portrait = authored(items, category, size);
        characters
            .entry(token.to_owned())
            .and_modify(|previous| previous.agree(portrait.clone()))
            .or_insert(portrait);
    }
    Ok(characters)
}

/// An advisor role: the idea the game appoints and the character behind it.
#[derive(Clone, Debug, PartialEq)]
pub struct AdvisorRole {
    pub character: String,
    pub slot: String,
    pub idea_token: String,
    pub traits: Vec<String>,
    pub cost: Option<f64>,
    pub ledger: Option<String>,
}

/// A country leader role for one ideology.
#[derive(Clone, Debug, PartialEq)]
pub struct LeaderRole {
    pub character: String,
    pub ideology: String,
    pub traits: Vec<String>,
}

/// What the characters files say, as far as every alternative agrees.
#[derive(Clone, Debug, Default)]
pub struct Characters {
    /// Advisor roles by the idea token the game appoints.
    pub advisors: BTreeMap<String, AdvisorRole>,
    /// Idea tokens whose alternatives or definitions disagree; left out of `advisors`.
    pub ambiguous_advisors: BTreeSet<String>,
    /// Country leader roles by (character, ideology).
    pub leaders: BTreeMap<(String, String), LeaderRole>,
    pub ambiguous_leaders: BTreeSet<(String, String)>,
    /// The name localisation key of each character, when every alternative agrees.
    pub names: BTreeMap<String, Authored>,
    /// Portraits by character, then (category, size): `civilian`/`army` and `large`/`small`.
    pub portraits: BTreeMap<String, BTreeMap<(String, String), Authored>>,
}

fn scalar<'a>(items: &'a [Item], key: &'a [u8]) -> Option<&'a str> {
    values(items, key).last()?.scalar_str()
}

fn words(items: &[Item], key: &[u8]) -> Vec<String> {
    values(items, key)
        .last()
        .and_then(Value::items)
        .map(|list| {
            list.iter()
                .filter(|item| item.key.is_none())
                .filter_map(|item| item.value.scalar_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Roles found in one block, keyed.
type RoleReader<'r, K, R> = &'r dyn Fn(&[Item]) -> Vec<(K, R)>;

/// Roles of one kind from a character and its alternatives. Each alternative's
/// roles are keyed; a key that alternatives define differently is ambiguous.
fn roles<K: Ord + Clone, R: Clone + PartialEq>(
    items: &[Item],
    role: RoleReader<'_, K, R>,
) -> (BTreeMap<K, R>, BTreeSet<K>) {
    let own = role(items);
    let alternatives: Vec<Vec<(K, R)>> = values(items, b"instance")
        .filter_map(Value::items)
        .map(role)
        .collect();
    let mut found = BTreeMap::new();
    let mut ambiguous = BTreeSet::new();
    for (key, value) in own.into_iter().chain(alternatives.into_iter().flatten()) {
        match found.get(&key) {
            Some(existing) if *existing != value => {
                ambiguous.insert(key);
            }
            Some(_) => {}
            None => {
                found.insert(key, value);
            }
        }
    }
    for key in &ambiguous {
        found.remove(key);
    }
    (found, ambiguous)
}

/// Parses one characters file.
pub fn parse(input: &[u8]) -> io::Result<Characters> {
    from_document(&parser::parse_unclosed(input)?.0)
}

fn from_document(document: &parser::Document) -> io::Result<Characters> {
    let mut result = Characters::default();
    for (token, items) in characters_root(document)? {
        let advisor = |items: &[Item]| {
            values(items, b"advisor")
                .filter_map(Value::items)
                .filter_map(|advisor| {
                    let idea = scalar(advisor, b"idea_token")?.to_owned();
                    Some((
                        idea.clone(),
                        AdvisorRole {
                            character: token.to_owned(),
                            slot: scalar(advisor, b"slot")?.to_owned(),
                            idea_token: idea,
                            traits: words(advisor, b"traits"),
                            cost: scalar(advisor, b"cost")
                                .and_then(|cost| cost.parse().ok())
                                .filter(|c: &f64| c.is_finite()),
                            ledger: scalar(advisor, b"ledger").map(str::to_owned),
                        },
                    ))
                })
                .collect::<Vec<_>>()
        };
        let (advisors, ambiguous) = roles(items, &advisor);
        merge(
            &mut result.advisors,
            &mut result.ambiguous_advisors,
            advisors,
            ambiguous,
        );
        let leader = |items: &[Item]| {
            values(items, b"country_leader")
                .filter_map(Value::items)
                .filter_map(|leader| {
                    let ideology = scalar(leader, b"ideology")?.to_owned();
                    Some((
                        (token.to_owned(), ideology.clone()),
                        LeaderRole {
                            character: token.to_owned(),
                            ideology,
                            traits: words(leader, b"traits"),
                        },
                    ))
                })
                .collect::<Vec<_>>()
        };
        let (leaders, ambiguous) = roles(items, &leader);
        merge(
            &mut result.leaders,
            &mut result.ambiguous_leaders,
            leaders,
            ambiguous,
        );
        let name = |items: &[Item]| {
            scalar(items, b"name")
                .map_or(Authored::Missing, |name| Authored::Asset(name.to_owned()))
        };
        let mut agreed = name(items);
        let mut instances = values(items, b"instance")
            .filter_map(Value::items)
            .map(name)
            .peekable();
        if instances.peek().is_some() {
            let own = agreed;
            agreed = instances.next().unwrap();
            for alternative in instances {
                agreed.agree(alternative);
            }
            if own != Authored::Missing {
                agreed.agree(own);
            }
        }
        result
            .names
            .entry(token.to_owned())
            .and_modify(|previous| previous.agree(agreed.clone()))
            .or_insert(agreed);
        let portraits = result.portraits.entry(token.to_owned()).or_default();
        for category in ["civilian", "army", "navy"] {
            for size in ["large", "small"] {
                let portrait = authored(items, category.as_bytes(), size.as_bytes());
                portraits
                    .entry((category.to_owned(), size.to_owned()))
                    .and_modify(|previous| previous.agree(portrait.clone()))
                    .or_insert(portrait);
            }
        }
    }
    Ok(result)
}

fn merge<K: Ord + Clone, R: PartialEq>(
    found: &mut BTreeMap<K, R>,
    ambiguous: &mut BTreeSet<K>,
    more: BTreeMap<K, R>,
    more_ambiguous: BTreeSet<K>,
) {
    for (key, value) in more {
        if ambiguous.contains(&key) {
            continue;
        }
        match found.get(&key) {
            Some(existing) if *existing != value => {
                found.remove(&key);
                ambiguous.insert(key);
            }
            _ => {
                found.insert(key, value);
            }
        }
    }
    for key in more_ambiguous {
        found.remove(&key);
        ambiguous.insert(key);
    }
}

/// The `common/characters` files in load order.
pub fn paths(layers: &ResolvedLayers) -> Vec<String> {
    let mut paths = layers
        .paths()
        .filter(|path| {
            let lower = path.to_ascii_lowercase();
            lower.starts_with("common/characters/") && lower.ends_with(".txt")
        })
        .map(str::to_owned)
        .collect::<Vec<_>>();
    paths.sort_unstable();
    paths
}

/// Reads every characters file. A file that cannot be parsed is left out with a
/// warning; definitions of one token in several files must agree.
pub fn read(layers: &ResolvedLayers) -> io::Result<(Characters, Vec<String>)> {
    let mut result = Characters::default();
    let mut warnings = Vec::new();
    for path in paths(layers) {
        let parsed =
            parser::parse_unclosed(&layers.read(&path)?).and_then(|(document, repaired)| {
                if repaired {
                    warnings.push(format!(
                        "{path} ends inside an open block; it was closed as the game closes it."
                    ));
                }
                from_document(&document)
            });
        match parsed {
            Ok(file) => {
                merge(
                    &mut result.advisors,
                    &mut result.ambiguous_advisors,
                    file.advisors,
                    file.ambiguous_advisors,
                );
                merge(
                    &mut result.leaders,
                    &mut result.ambiguous_leaders,
                    file.leaders,
                    file.ambiguous_leaders,
                );
                for (token, name) in file.names {
                    result
                        .names
                        .entry(token)
                        .and_modify(|previous| previous.agree(name.clone()))
                        .or_insert(name);
                }
                for (token, portraits) in file.portraits {
                    let entry = result.portraits.entry(token).or_default();
                    for (key, portrait) in portraits {
                        entry
                            .entry(key)
                            .and_modify(|previous| previous.agree(portrait.clone()))
                            .or_insert(portrait);
                    }
                }
            }
            Err(error) => warnings.push(format!("Characters unavailable in {path}: {error}")),
        }
    }
    if !result.ambiguous_advisors.is_empty() {
        warnings.push(format!(
            "{} advisor roles are defined differently by conditional alternatives and show without traits or cost.",
            result.ambiguous_advisors.len()
        ));
    }
    Ok((result, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &[u8] = br#"characters = {
        GER_schacht = { name = GER_schacht portraits = { civilian = { large = GFX_schacht_large small = GFX_idea_schacht } }
            advisor = { slot = political_advisor idea_token = GER_schacht_idea cost = 150 traits = { captain_of_industry } ledger = civilian } }
        GER_hitler = { portraits = { civilian = { large = GFX_hitler } }
            country_leader = { ideology = nazism traits = { dictator } } }
        GER_split = {
            instance = { allowed = { has_dlc = "A" } portraits = { army = { small = GFX_a } }
                advisor = { slot = army_chief idea_token = GER_split_idea traits = { a_trait } } }
            instance = { allowed = { NOT = { has_dlc = "A" } } portraits = { army = { small = GFX_b } }
                advisor = { slot = army_chief idea_token = GER_split_idea traits = { b_trait } } } }
        GER_same = {
            instance = { portraits = { army = { large = GFX_same } } }
            instance = { portraits = { army = { large = GFX_same } } } } }"#;

    #[test]
    fn advisors_and_leaders_keep_their_roles_and_traits() {
        let characters = parse(FILE).unwrap();
        let schacht = &characters.advisors["GER_schacht_idea"];
        assert_eq!(
            (
                schacht.character.as_str(),
                schacht.slot.as_str(),
                schacht.cost
            ),
            ("GER_schacht", "political_advisor", Some(150.0))
        );
        assert_eq!(schacht.traits, ["captain_of_industry"]);
        let leader = &characters.leaders[&("GER_hitler".to_owned(), "nazism".to_owned())];
        assert_eq!(leader.traits, ["dictator"]);
        assert_eq!(
            characters.names["GER_schacht"],
            Authored::Asset("GER_schacht".into())
        );
    }

    #[test]
    fn alternatives_that_disagree_are_ambiguous_and_agreeing_ones_are_kept() {
        let characters = parse(FILE).unwrap();
        assert!(!characters.advisors.contains_key("GER_split_idea"));
        assert!(characters.ambiguous_advisors.contains("GER_split_idea"));
        let split = &characters.portraits["GER_split"][&("army".to_owned(), "small".to_owned())];
        assert_eq!(
            *split,
            Authored::Ambiguous(BTreeSet::from(["GFX_a".into(), "GFX_b".into()]))
        );
        assert_eq!(
            characters.portraits["GER_same"][&("army".to_owned(), "large".to_owned())],
            Authored::Asset("GFX_same".into())
        );
        assert_eq!(
            characters.portraits["GER_schacht"][&("civilian".to_owned(), "small".to_owned())],
            Authored::Asset("GFX_idea_schacht".into())
        );
    }

    #[test]
    fn portraits_of_one_kind_match_the_full_parse() {
        let army = portraits(FILE, b"army", b"large").unwrap();
        assert_eq!(army["GER_same"], Authored::Asset("GFX_same".into()));
        assert_eq!(army["GER_schacht"], Authored::Missing);
    }

    #[test]
    fn only_agreeing_army_large_alternatives_are_usable() {
        let characters = portraits(
            br#"characters = {
                plain = { portraits = { army = { large = GFX_army small = GFX_small }
                                         civilian = { large = GFX_civilian } } }
                same = { instance = { allowed = { has_dlc = A }
                                      portraits = { army = { large = GFX_army } } }
                         instance = { portraits = { army = { large = GFX_army } } } }
                different = { instance = { portraits = { army = { large = GFX_first } } }
                              instance = { portraits = { army = { large = GFX_second } } } }
                incomplete = { instance = { portraits = { army = { large = GFX_army } } }
                               instance = { portraits = { army = { small = GFX_small } } } }
                duplicate = { portraits = { army = { large = GFX_first } } }
                duplicate = { portraits = { army = { large = GFX_second } } }
                direct = { portraits = { army = { large = "gfx/leaders/portrait.dds" } } }
                no_army = { portraits = { civilian = { large = GFX_civilian } } }
            }"#,
            b"army",
            b"large",
        )
        .unwrap();
        for token in ["plain", "same"] {
            assert_eq!(characters[token], Authored::Asset("GFX_army".to_owned()));
        }
        assert_eq!(
            characters["different"],
            Authored::Ambiguous(["GFX_first".to_owned(), "GFX_second".to_owned()].into())
        );
        assert_eq!(
            characters["incomplete"],
            Authored::Ambiguous(["GFX_army".to_owned()].into())
        );
        assert_eq!(
            characters["duplicate"],
            Authored::Ambiguous(["GFX_first".to_owned(), "GFX_second".to_owned()].into())
        );
        assert_eq!(
            characters["direct"],
            Authored::Asset("gfx/leaders/portrait.dds".to_owned())
        );
        assert_eq!(characters["no_army"], Authored::Missing);
    }
}
