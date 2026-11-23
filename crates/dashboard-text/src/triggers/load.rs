use crate::triggers::Failure;

use content_reader_parser::parser::Item;

use super::{
    Eval, Scopes,
    eval::{Child, Evaluator, key_text},
    operands::shown,
};

const IF: u32 = 0x27C3;

pub(super) enum Unit<'i, 't> {
    One(&'i Item, Child<'t>),
    Chain(Vec<Branch<'i, 't>>),
    Unknown(Failure),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Key {
    If,
    Else,
    ElseIf,
    Inner,
}

pub(super) struct Branch<'i, 't> {
    key: Key,
    pub(super) limit: Vec<Unit<'i, 't>>,
    pub(super) body: Vec<Unit<'i, 't>>,
    pub(super) refused: Option<Failure>,
}

impl<'a> Evaluator<'_, 'a> {
    pub(super) fn units<'i>(&self, items: &'i [Item]) -> Vec<Unit<'i, 'a>> {
        let mut out: Vec<Unit<'i, 'a>> = Vec::new();
        for item in items {
            if let Some(key) = alternative(item) {
                match out.last_mut() {
                    Some(Unit::Chain(chain)) if !chain.iter().any(|b| b.key == Key::Else) => {
                        chain.extend(self.conditional(item, key));
                    }
                    Some(Unit::Unknown(why)) => {
                        let why = why.clone().context(|why| {
                            format!("whether {} follows an if is unknown: {why}", key_text(item))
                        });
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

    fn conditional<'i>(&self, item: &'i Item, key: Key) -> Vec<Branch<'i, 'a>> {
        let mut branch = Branch {
            key,
            limit: Vec::new(),
            body: Vec::new(),
            refused: None,
        };
        let Some(items) = item.value.items() else {
            branch.refused = Some(Failure::from(format!(
                "trigger {} = {}: the conditional is not a block",
                key_text(item),
                shown(&item.value)
            )));
            return vec![branch];
        };
        let mut own = Vec::new();
        for child in items {
            if child.keyword_is("limit") {
                match presence(&branch.limit) {
                    Presence::None => {
                        if let Some(limit) = child.value.items() {
                            let loaded = self.units(limit);
                            branch.limit.extend(loaded);
                        }
                    }
                    Presence::Some => {}
                    Presence::Unknown(why) => branch.limit.push(Unit::Unknown(why.context(|why| format!(
                        "whether a later limit is loaded (only an empty limit accepts one) is unknown: {why}"
                    )))),
                }
                continue;
            }
            if alternative(child).is_some() {
                let alternatives = self.conditional(child, Key::Inner);
                match branch.body.last_mut() {
                    Some(Unit::Chain(nested)) => nested.extend(alternatives),
                    Some(Unit::Unknown(why)) => {
                        let why = why.clone().context(|why| format!(
                            "whether {} attaches to a nested if or to its enclosing conditional is unknown: {why}",
                            key_text(child)
                        ));
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

    pub fn has_children(&self, items: &[Item]) -> Result<bool, Failure> {
        match presence(&self.units(items)) {
            Presence::None => Ok(false),
            Presence::Some => Ok(true),
            Presence::Unknown(why) => Err(why),
        }
    }

    pub(super) fn with_variable_context<T>(&self, scopes: Scopes, run: impl FnOnce() -> T) -> T {
        let depth = self.checking.get();
        if depth == 0 && self.description_depth.get() == 0 {
            self.local_temps.borrow_mut().clear();
            self.clear_temp_writer();
            self.initialize_variable_random(scopes);
        }
        self.checking.set(depth + 1);
        let result = run();
        self.checking.set(depth);
        result
    }

    pub fn evaluate(&self, items: &[Item], scopes: Scopes) -> Eval {
        self.evaluate_seeded(items, scopes, &[])
    }

    pub fn evaluate_seeded(&self, items: &[Item], scopes: Scopes, seeds: &[(&[u8], i64)]) -> Eval {
        let depth = self.evaluations.get();
        if depth == 0 {
            self.diverged.replace(None);
        }
        self.evaluations.set(depth + 1);
        let value = self.with_variable_context(scopes, || {
            for (name, value) in seeds {
                self.local_temps
                    .borrow_mut()
                    .insert(name.to_vec(), Ok(super::variables::Value::Raw(*value)));
            }
            self.all(&self.units(items), scopes)
        });
        self.evaluations.set(depth);
        match self.divergence() {
            Some(why) => {
                if let Eval::Unknown(reasons) = &value {
                    for failure in reasons {
                        failure.consumed();
                    }
                }
                Eval::Unknown(std::collections::BTreeSet::from([why]))
            }
            None => value,
        }
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

    pub(super) fn branches(&self, chain: &[Branch<'_, 'a>], scopes: Scopes) -> Eval {
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
            outcome = outcome.and(|| enter.clone().negate().or(|| body));
            taken = taken.or(|| enter);
            if taken == Eval::True {
                break;
            }
        }
        outcome
    }
}

fn alternative(item: &Item) -> Option<Key> {
    if item.keyword_is("else") {
        Some(Key::Else)
    } else if item.keyword_is("else_if") {
        Some(Key::ElseIf)
    } else {
        None
    }
}

enum Presence {
    None,
    Some,
    Unknown(Failure),
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
