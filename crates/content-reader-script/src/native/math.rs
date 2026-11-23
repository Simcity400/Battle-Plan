
use super::{Item, Kind, Reader, Slots, Value};

#[derive(Default)]
struct State {
    error: bool,
    operands: usize,
}

impl Reader<'_> {
    pub(super) fn math_expression(&mut self) -> Value {
        self.math_operand(&mut State::default())
    }

    fn math_operand(&mut self, state: &mut State) -> Value {
        if self.value.kind != Kind::Open {
            if state.operands >= 127 {
                self.math_error(state, "math operand index exceeds 126");
            } else {
                state.operands += 1;
            }
            return Value::Scalar(self.value.text.clone());
        }
        let mut first = self.math_key();
        if self.unsettled.is_some() {
            return Value::Block(Vec::new());
        }
        if first.keyword_is("value") {
            first.value = self.math_operand(state);
        } else {
            self.math_error(state, "math expression must start with value");
            self.skip_current_block();
        }
        let mut items = vec![first];
        self.math_tail(state, &mut items);
        Value::Block(items)
    }

    fn math_key(&mut self) -> Item {
        let slots = self.key_operator_value();
        if self.unsettled.is_some() {
            return Item {
                key: None,
                key_quoted: false,
                operator: self.operator,
                value: Value::Scalar(Vec::new()),
                kind: Kind::End,
            };
        }
        if slots == Slots::NoOperator {
            self.misread(
                self.key.line,
                format!(
                    "math key {} has no operator, so its reader takes the previous value {}",
                    self.key.shown(),
                    self.value.shown()
                ),
            );
        }
        Item {
            key: Some(self.key.text.clone()),
            key_quoted: self.key.kind == Kind::Quoted,
            operator: self.operator,
            value: Value::Scalar(self.value.text.clone()),
            kind: self.value.kind,
        }
    }

    fn math_error(&mut self, state: &mut State, why: &str) {
        state.error = true;
        self.misread(self.key.line, why.into());
        self.skip_reported();
    }

    fn math_tail(&mut self, state: &mut State, items: &mut Vec<Item>) {
        while self.unsettled.is_none() {
            if self.at_loop_end() {
                break;
            }
            if state.error {
                self.skip_block();
                break;
            }
            let mut item = self.math_key();
            if self.unsettled.is_some() {
                break;
            }
            let key = item.keyword().map(|key| key.folded());

            match key.as_deref() {
                Some(
                    b"add"
                    | b"subtract"
                    | b"multiply"
                    | b"divide"
                    | b"min"
                    | b"max"
                    | b"greater_than"
                    | b"less_than"
                    | b"greater_than_or_equals"
                    | b"less_than_or_equals"
                    | b"equals"
                    | b"not_equals"
                    | b"xor"
                    | b"pow"
                    | b"root"
                    | b"mod"
                    | b"log"
                    | b"atan2"
                    | b"and"
                    | b"or",
                ) => {
                    item.value = self.math_operand(state);
                }
                Some(b"not" | b"round" | b"sin" | b"cos" | b"tan" | b"atan") => {
                    if self.value.c_str() != b"yes" {
                        self.math_error(state, "math unary operator requires true");
                    }
                }
                Some(b"clamp") => item.value = self.math_arguments(state, &["min", "max"]),

                Some(b"lerp") => item.value = self.math_arguments(state, &["to", "alpha"]),
                Some(b"if") => {
                    item.value = self.math_branch(state);
                    items.push(item);
                    while self.peek().is_keyword("else_if") && self.unsettled.is_none() {
                        let mut branch = self.math_key();
                        if self.unsettled.is_some() {
                            break;
                        }
                        branch.value = self.math_branch(state);
                        items.push(branch);
                    }
                    if self.unsettled.is_none() && self.peek().is_keyword("else") {
                        let mut branch = self.math_key();
                        if self.unsettled.is_some() {
                            break;
                        }
                        let mut body = Vec::new();
                        self.math_tail(state, &mut body);
                        branch.value = Value::Block(body);
                        items.push(branch);
                    }
                    continue;
                }
                Some(b"every_collection") => {
                    let mut name = self.math_key();
                    if self.unsettled.is_some() {
                        break;
                    }
                    let mut body = Vec::new();
                    if !name.keyword_is("named_collection") {
                        self.math_error(state, "math collection must start with named_collection");
                        self.skip_current_block();
                    } else if state.operands >= 127 {
                        self.math_error(state, "math operand index exceeds 126");
                    } else {
                        state.operands += 1;
                        name.value = Value::Scalar(self.value.text.clone());
                        body.push(name);
                        self.math_tail(state, &mut body);
                    }
                    item.value = Value::Block(body);
                }
                _ => {
                    self.math_error(state, "invalid math operator");
                    self.skip_current_block();
                }
            }
            items.push(item);
        }
    }

    fn math_branch(&mut self, state: &mut State) -> Value {
        let mut limit = self.math_key();
        if self.unsettled.is_some() {
            return Value::Block(Vec::new());
        }
        let mut items = Vec::new();
        if limit.keyword_is("limit") {
            limit.value = self.math_operand(state);
            items.push(limit);
            self.math_tail(state, &mut items);
        } else {
            self.math_error(state, "math if/else_if must start with limit");
            self.skip_current_block();
            items.push(limit);
        }
        Value::Block(items)
    }

    fn math_arguments(&mut self, state: &mut State, names: &[&str; 2]) -> Value {
        let mut items = Vec::new();
        for name in names {
            let mut item = self.math_key();
            if self.unsettled.is_some() {
                return Value::Block(items);
            }
            if !item.keyword_is(name) {
                self.math_error(state, "unexpected math argument");
                self.skip_block();
                items.push(item);
                return Value::Block(items);
            }
            item.value = self.math_operand(state);
            items.push(item);
            if self.unsettled.is_some() {
                return Value::Block(items);
            }
        }
        self.advance();
        self.current = self.resolve(self.current());
        Value::Block(items)
    }
}
