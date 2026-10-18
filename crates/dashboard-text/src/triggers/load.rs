//! Load the children a trigger body holds: a conjunction's keys,
//! or a conditional's limit and attached alternatives.
//! Evaluate conjunctions, disjunctions and conditional chains
//! with the shared recorded-hour evaluator.

use installed_content_core::parser::Item;

use super::{
    Eval, Scopes,
    eval::{Child, Evaluator, key_text},
    operands::shown,
};

/// The token of `if`.
const IF: u32 = 0x27C3;

/// One child a body holds: a loaded child, an `if` with the
/// alternatives attached to it, or a key whether a child is added for is
/// unknown.
pub(super) enum Unit<'i, 't> {
    One(&'i Item, Child<'t>),
    /// A conditional and its alternatives, in chain order.
    Chain(Vec<Branch<'i, 't>>),
    Unknown(String),
}

/// The key a conditional holds when an alternative is appended:
/// `if`, or an attached `else`/`else_if` (the token is set once attached),
/// or an alternative loaded inside a conditional,
/// which keeps its initial conjunction keyword.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Key {
    If,
    Else,
    ElseIf,
    Inner,
}

/// One conditional of a chain: its limit conjunction
/// and its body, the conditional's own children.
pub(super) struct Branch<'i, 't> {
    key: Key,
    pub(super) limit: Vec<Unit<'i, 't>>,
    pub(super) body: Vec<Unit<'i, 't>>,
    /// Why the conditional cannot be read as the game reads it.
    pub(super) refused: Option<String>,
}

impl<'a> Evaluator<'_, 'a> {
    /// The children added for a conjunction's keys, in
    /// order. A key that names nothing adds none. An `else` or `else_if` is
    /// a conditional attached to the last child added when that is an `if`,
    /// walking its chain from the `if`, which refuses a
    /// chain holding an attached `else`; otherwise the misplaced
    /// alternative is logged and adds none.
    pub(super) fn units<'i>(&self, items: &'i [Item]) -> Vec<Unit<'i, 'a>> {
        let mut out: Vec<Unit<'i, 'a>> = Vec::new();
        for item in items {
            if let Some(key) = alternative(item) {
                match out.last_mut() {
                    Some(Unit::Chain(chain)) if !chain.iter().any(|b| b.key == Key::Else) => {
                        chain.extend(self.conditional(item, key));
                    }
                    Some(Unit::Unknown(why)) => {
                        let why =
                            format!("whether {} follows an if is unknown: {why}", key_text(item));
                        out.push(Unit::Unknown(why));
                    }
                    _ => {}
                }
                continue;
            }
            self.load_key(item, &mut out);
        }
        out
    }

    /// Load a key other than an alternative: the child it
    /// adds, if any.
    fn load_key<'i>(&self, item: &'i Item, out: &mut Vec<Unit<'i, 'a>>) {
        match self.child(item) {
            Ok(None) => {}
            Ok(Some(Child::Coded(token, _))) if token == IF => {
                out.push(Unit::Chain(self.conditional(item, Key::If)));
            }
            Ok(Some(child)) => out.push(Unit::One(item, child)),
            Err(why) => out.push(Unit::Unknown(why)),
        }
    }

    /// Load a conditional body,
    /// with the alternatives its body attaches to itself after
    /// it, in chain order:
    /// - `limit` loads into the limit only while the limit holds no
    ///   child; a later one is recorded as an error and skipped;
    /// - `else`/`else_if` make a conditional attached to the
    ///   last child when that is an `if`, from its first alternative, else
    ///   to this conditional's own chain (which finds no
    ///   `else` key among them);
    /// - any other key loads a child of the body.
    fn conditional<'i>(&self, item: &'i Item, key: Key) -> Vec<Branch<'i, 'a>> {
        let mut branch = Branch {
            key,
            limit: Vec::new(),
            body: Vec::new(),
            refused: None,
        };
        let Some(items) = item.value.items() else {
            branch.refused = Some(format!(
                "trigger {} = {}: the conditional is not a block",
                key_text(item),
                shown(&item.value)
            ));
            return vec![branch];
        };
        let mut own = Vec::new();
        for child in items {
            if child.keyword_is("limit") {
                match presence(&branch.limit) {
                    Presence::None => {
                        // LoadBody on a value that is no block reads nothing.
                        if let Some(limit) = child.value.items() {
                            let loaded = self.units(limit);
                            branch.limit.extend(loaded);
                        }
                    }
                    Presence::Some => {}
                    Presence::Unknown(why) => branch.limit.push(Unit::Unknown(format!(
                        "whether a later limit is loaded (only an empty limit accepts one) is unknown: {why}"
                    ))),
                }
                continue;
            }
            if alternative(child).is_some() {
                let alternatives = self.conditional(child, Key::Inner);
                match branch.body.last_mut() {
                    Some(Unit::Chain(nested)) => nested.extend(alternatives),
                    Some(Unit::Unknown(why)) => {
                        let why = format!(
                            "whether {} attaches to a nested if or to its enclosing conditional is unknown: {why}",
                            key_text(child)
                        );
                        branch.body.push(Unit::Unknown(why.clone()));
                        own.push(Branch {
                            key: Key::Inner,
                            limit: Vec::new(),
                            body: Vec::new(),
                            refused: Some(why),
                        });
                    }
                    _ => own.extend(alternatives),
                }
                continue;
            }
            self.load_key(child, &mut branch.body);
        }
        let mut chain = vec![branch];
        chain.extend(own);
        chain
    }

    /// Whether a body holds a loaded child, for callers that test a
    /// conjunction's child count, including adjacency rules
    /// and the raid state selector.
    pub fn has_children(&self, items: &[Item]) -> Result<bool, String> {
        match presence(&self.units(items)) {
            Presence::None => Ok(false),
            Presence::Some => Ok(true),
            Presence::Unknown(why) => Err(why),
        }
    }

    /// A trigger body, evaluated as a conjunction.
    pub fn evaluate(&self, items: &[Item], scopes: Scopes) -> Eval {
        self.evaluate_seeded(items, scopes, &[])
    }

    /// [`Self::evaluate`] with temp variables set before the body runs, as a caller that
    /// initialises its outputs does (fixed5 values).
    pub fn evaluate_seeded(&self, items: &[Item], scopes: Scopes, seeds: &[(&[u8], i64)]) -> Eval {
        let depth = self.checking.get();
        if depth == 0 && self.description_depth.get() == 0 {
            // Begin a fresh variable-random context for the outer check.
            self.local_temps.borrow_mut().clear();
            self.clear_temp_writer();
            self.initialize_variable_random(scopes);
        }
        for (name, value) in seeds {
            self.local_temps
                .borrow_mut()
                .insert(name.to_vec(), Ok(super::variables::Value::Raw(*value)));
        }
        self.checking.set(depth + 1);
        let result = self.all(&self.units(items), scopes);
        // Closing the variable context leaves known residual values.
        self.checking.set(depth);
        result
    }

    pub(super) fn all(&self, units: &[Unit<'_, 'a>], scopes: Scopes) -> Eval {
        let mut result = Eval::True;
        for unit in units {
            let value = self.maybe(matches!(result, Eval::Unknown(_)), || {
                self.unit(unit, scopes)
            });
            result = result.and(|| value);
            if result == Eval::False {
                return result;
            }
        }
        result
    }

    /// Any of the children holds; none added is
    /// false.
    pub(super) fn any(&self, items: &[Item], scopes: Scopes) -> Eval {
        let mut result = Eval::False;
        for unit in &self.units(items) {
            let value = self.maybe(matches!(result, Eval::Unknown(_)), || {
                self.unit(unit, scopes)
            });
            result = result.or(|| value);
            if result == Eval::True {
                break;
            }
        }
        result
    }

    pub(super) fn unit(&self, unit: &Unit<'_, 'a>, scopes: Scopes) -> Eval {
        match unit {
            Unit::One(item, child) => self.child_value(item, child, scopes),
            Unit::Chain(chain) => self.branches(chain, scopes),
            Unit::Unknown(why) => Eval::unknown(why.clone()),
        }
    }

    /// Evaluate a chain: the first
    /// conditional whose limit holds checks its body (an empty limit holds,
    /// whatever the key); a chain none enters holds.
    fn branches(&self, chain: &[Branch<'_, 'a>], scopes: Scopes) -> Eval {
        let mut taken = Eval::False;
        let mut outcome = Eval::True;
        for branch in chain {
            let applies = self.maybe(matches!(taken, Eval::Unknown(_)), || {
                match &branch.refused {
                    Some(why) => Eval::unknown(why.clone()),
                    None => self.all(&branch.limit, scopes),
                }
            });
            let enter = taken.clone().negate().and(|| applies);
            let body = if enter == Eval::False {
                Eval::True
            } else {
                self.maybe(matches!(enter, Eval::Unknown(_)), || {
                    match &branch.refused {
                        Some(why) => Eval::unknown(why.clone()),
                        None => self.all(&branch.body, scopes),
                    }
                })
            };
            // outcome = (enter -> body) and the earlier outcomes.
            outcome = outcome.and(|| enter.clone().negate().or(|| body));
            taken = taken.or(|| enter);
            if taken == Eval::True {
                break;
            }
        }
        outcome
    }
}

/// The key of an alternative: `else` or `else_if`.
fn alternative(item: &Item) -> Option<Key> {
    if item.keyword_is("else") {
        Some(Key::Else)
    } else if item.keyword_is("else_if") {
        Some(Key::ElseIf)
    } else {
        None
    }
}

/// Whether a conjunction holds a child.
enum Presence {
    None,
    Some,
    Unknown(String),
}

fn presence(units: &[Unit<'_, '_>]) -> Presence {
    if units.iter().any(|unit| !matches!(unit, Unit::Unknown(_))) {
        return Presence::Some;
    }
    match units.first() {
        Some(Unit::Unknown(why)) => Presence::Unknown(why.clone()),
        _ => Presence::None,
    }
}
