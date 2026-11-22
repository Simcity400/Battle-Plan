use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap, HashMap, HashSet};
use std::hash::Hash;

use super::{Field, Kind, Mode, Shape, Slots, Token, lex, untraced};
use crate::parser::Operator;

const MAX_VISITED_STATES: usize = 65_536;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExploredNames {
    pub names: BTreeSet<Vec<u8>>,
    pub exhausted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct SmallToken {
    kind: Kind,
    quoted: bool,
    text: u32,
}

impl SmallToken {
    fn end() -> Self {
        Self {
            kind: Kind::End,
            quoted: false,
            text: 0,
        }
    }

    fn slot() -> Self {
        Self {
            kind: Kind::Bare,
            quoted: false,
            text: 0,
        }
    }

    fn operator(self) -> bool {
        matches!(self.kind, Kind::Equals | Kind::Less | Kind::Greater)
    }

    fn as_operator(self) -> Operator {
        match self.kind {
            Kind::Less => Operator::Less,
            Kind::Greater => Operator::Greater,
            _ => Operator::Equals,
        }
    }

    fn ends_block(self) -> bool {
        matches!(self.kind, Kind::Close | Kind::End)
    }

    fn to_token(self, texts: &[&[u8]]) -> Token {
        Token {
            kind: self.kind,
            text: texts[self.text as usize].to_vec(),
            line: 0,
            quoted: self.quoted,
            byte_end: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum FrameKind<S> {
    File(S),
    Object(S),
    KeyBlock { meta: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Frame<S> {
    kind: FrameKind<S>,
    decl_name: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct State<S> {
    next: usize,
    pending: bool,
    pending_result: bool,
    current: SmallToken,
    key: SmallToken,
    operator: Operator,
    value: SmallToken,
    variables: Vec<(u32, SmallToken)>,
    stack: Vec<Frame<S>>,
}

impl<S> State<S> {
    fn new(top: S) -> Self {
        Self {
            next: 0,
            pending: false,
            pending_result: false,
            current: SmallToken::end(),
            key: SmallToken::slot(),
            operator: Operator::Equals,
            value: SmallToken::slot(),
            variables: Vec::new(),
            stack: vec![Frame {
                kind: FrameKind::File(top),
                decl_name: None,
            }],
        }
    }

    fn advance_raw(&mut self, tokens: &[SmallToken], texts: &[&[u8]]) -> bool {
        let Some(&token) = tokens.get(self.next) else {
            return false;
        };
        self.next += 1;
        if token.kind != Kind::End {
            self.current = token;
            return true;
        }
        let bytes = texts[token.text as usize];
        if !bytes.is_empty() {
            let kind = match bytes {
                b"!" | b"#" | b";" => Kind::Comment,
                _ => Kind::Bare,
            };
            self.current = SmallToken { kind, ..token };
        }
        false
    }

    fn advance(&mut self, tokens: &[SmallToken], texts: &[&[u8]]) -> bool {
        if self.pending {
            self.pending = false;
            self.pending_result
        } else {
            self.advance_raw(tokens, texts)
        }
    }

    fn peek(&mut self, tokens: &[SmallToken], texts: &[&[u8]]) -> SmallToken {
        if !self.pending {
            let result = self.advance_raw(tokens, texts);
            if !result {
                self.current.kind = Kind::End;
            }
            self.pending = true;
            self.pending_result = result;
        }
        self.current
    }

    fn peek_kind(&self, tokens: &[SmallToken]) -> Kind {
        if self.pending {
            self.current.kind
        } else {
            tokens.get(self.next).map_or(Kind::End, |token| token.kind)
        }
    }

    fn resolve(&self, token: SmallToken, texts: &[&[u8]]) -> SmallToken {
        let bytes = texts[token.text as usize];
        if token.quoted || token.kind == Kind::Code || !bytes.starts_with(b"@") {
            return token;
        }
        let hash = crate::parser::variable_hash(&bytes[1..]);
        match self.variables.binary_search_by_key(&hash, |(h, _)| *h) {
            Ok(index) => self.variables[index].1,
            Err(_) => token,
        }
    }

    fn define(&mut self, tokens: &[SmallToken], texts: &[&[u8]]) {
        let name = self.current;
        let hash = crate::parser::variable_hash(&texts[name.text as usize][1..]);
        if self
            .variables
            .binary_search_by_key(&hash, |(h, _)| *h)
            .is_ok()
        {
            return;
        }
        if !self.advance(tokens, texts) {
            return;
        }
        if self.current.kind != Kind::Equals {
            return;
        }
        let peeked = self.peek(tokens, texts);
        let value = if texts[peeked.text as usize].starts_with(b"@") {
            self.resolve(peeked, texts)
        } else {
            self.advance(tokens, texts);
            self.current
        };
        if let Err(pos) = self.variables.binary_search_by_key(&hash, |(h, _)| *h) {
            self.variables.insert(pos, (hash, value));
        }
    }

    fn key_operator_value(&mut self, tokens: &[SmallToken], texts: &[&[u8]]) -> Slots {
        loop {
            if !self.advance(tokens, texts) {
                return Slots::Ended;
            }
            if texts[self.current.text as usize].starts_with(b"@") {
                self.define(tokens, texts);
                continue;
            }
            self.key = self.current;
            if !self.advance(tokens, texts) || !self.current.operator() {
                return Slots::NoOperator;
            }
            self.operator = self.current.as_operator();
            if self.peek(tokens, texts).kind == Kind::Close {
                self.value = SmallToken::end();
            } else {
                self.advance(tokens, texts);
                self.value = self.resolve(self.current, texts);
            }
            return Slots::Fresh;
        }
    }

    fn at_loop_end(&mut self, tokens: &[SmallToken], texts: &[&[u8]]) -> bool {
        if !self.peek(tokens, texts).ends_block() {
            return false;
        }
        self.advance(tokens, texts);
        self.current = self.resolve(self.current, texts);
        true
    }

    fn skip(&mut self, tokens: &[SmallToken], texts: &[&[u8]]) {
        let mut depth = 1_usize;
        while depth > 0 {
            let end = self.peek(tokens, texts).ends_block();
            self.advance(tokens, texts);
            self.current = self.resolve(self.current, texts);
            if end {
                depth -= 1;
                if depth > 0 && !self.pending && self.next >= tokens.len() {
                    self.current.kind = Kind::End;
                    break;
                }
            } else if self.current.kind == Kind::Open {
                depth += 1;
            }
        }
    }

    fn skip_block(&mut self, tokens: &[SmallToken], texts: &[&[u8]]) {
        let mut depth = 0_u32;
        loop {
            if !self.advance(tokens, texts) {
                return;
            }
            match self.current.kind {
                Kind::Open => depth += 1,
                Kind::Close if depth == 0 => return,
                Kind::Close => depth -= 1,
                _ => {}
            }
        }
    }

    fn skip_past_embedded_end(&mut self, tokens: &[SmallToken], texts: &[&[u8]]) {
        let mut depth = 0_u32;
        loop {
            if !self.advance(tokens, texts) {
                if self.next < tokens.len() {
                    continue;
                }
                return;
            }
            match self.current.kind {
                Kind::Open => depth += 1,
                Kind::Close if depth == 0 => return,
                Kind::Close => depth -= 1,
                _ => {}
            }
        }
    }

    fn list(&mut self, tokens: &[SmallToken], texts: &[&[u8]]) {
        loop {
            let stop = self.peek(tokens, texts).ends_block();
            self.advance(tokens, texts);
            if stop {
                return;
            }
            self.value = self.resolve(self.current, texts);
        }
    }

    fn prune_dead_variables(&mut self, last_var_mention: &HashMap<u32, usize>, texts: &[&[u8]]) {
        let effective_pos = if self.pending {
            self.next.saturating_sub(1)
        } else {
            self.next
        };
        let current_hash = texts[self.current.text as usize]
            .strip_prefix(b"@")
            .map(crate::parser::variable_hash);
        self.variables.retain(|(hash, _)| {
            current_hash == Some(*hash)
                || last_var_mention
                    .get(hash)
                    .is_some_and(|&last| last >= effective_pos)
        });
    }
}

struct HeapItem<S> {
    next: usize,
    seq: usize,
    state: State<S>,
}

impl<S> PartialEq for HeapItem<S> {
    fn eq(&self, other: &Self) -> bool {
        self.next == other.next && self.seq == other.seq
    }
}

impl<S> Eq for HeapItem<S> {}

impl<S> PartialOrd for HeapItem<S> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<S> Ord for HeapItem<S> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (Reverse(self.next), Reverse(self.seq)).cmp(&(Reverse(other.next), Reverse(other.seq)))
    }
}

fn is_tag(token: SmallToken, texts: &[&[u8]]) -> bool {
    if token.quoted {
        return false;
    }
    let bytes = texts[token.text as usize];
    bytes.eq_ignore_ascii_case(b"rgb") || bytes.eq_ignore_ascii_case(b"hsv")
}

fn is_meta_key(token: SmallToken, texts: &[&[u8]]) -> bool {
    let key = crate::parser::c_string(texts[token.text as usize]);
    crate::parser::looked_up(key, token.quoted)
        && matches!(
            key.to_ascii_lowercase().as_slice(),
            b"meta_effect" | b"meta_trigger"
        )
}

pub(crate) fn recover_declared_names<S>(
    bytes: &[u8],
    mode: Mode,
    top: S,
    is_declaration: impl Fn(S) -> bool,
    dispatch: impl Fn(S, &Field<'_>) -> Shape<S>,
) -> ExploredNames
where
    S: Copy + Eq + Hash,
{
    let lexed = lex(bytes, mode);
    let mut texts: Vec<&[u8]> = vec![b""];
    let mut text_map: HashMap<&[u8], u32> = HashMap::new();
    text_map.insert(b"", 0);
    let mut tokens = Vec::with_capacity(lexed.tokens.len());
    let mut last_var_mention: HashMap<u32, usize> = HashMap::new();

    for (index, token) in lexed.tokens.iter().enumerate() {
        let slice = token.text.as_slice();
        let id = if let Some(&existing) = text_map.get(slice) {
            existing
        } else {
            let next_id = u32::try_from(texts.len()).expect("token text count fits in u32");
            texts.push(slice);
            text_map.insert(slice, next_id);
            next_id
        };
        if let Some(var) = slice.strip_prefix(b"@") {
            last_var_mention.insert(crate::parser::variable_hash(var), index);
        }
        tokens.push(SmallToken {
            kind: token.kind,
            quoted: token.quoted,
            text: id,
        });
    }

    let mut visited: HashSet<State<S>> = HashSet::new();
    let mut branch_visited: HashSet<State<S>> = HashSet::new();
    let mut heap: BinaryHeap<HeapItem<S>> = BinaryHeap::new();
    let mut seq = 0_usize;
    let mut recovered_ids: BTreeSet<u32> = BTreeSet::new();
    let mut exhausted = false;

    let enqueue = |mut state: State<S>,
                   visited: &mut HashSet<State<S>>,
                   heap: &mut BinaryHeap<HeapItem<S>>,
                   seq: &mut usize,
                   exhausted: &mut bool| {
        if *exhausted {
            return;
        }
        state.prune_dead_variables(&last_var_mention, &texts);
        if visited.contains(&state) {
            return;
        }
        if visited.len() >= MAX_VISITED_STATES {
            *exhausted = true;
            return;
        }
        let next = state.next;
        visited.insert(state.clone());
        let id = *seq;
        *seq += 1;
        heap.push(HeapItem { next, seq: id, state });
    };

    enqueue(
        State::new(top),
        &mut visited,
        &mut heap,
        &mut seq,
        &mut exhausted,
    );

    while let Some(HeapItem { mut state, .. }) = heap.pop() {
        if exhausted {
            break;
        }
        let Some(&top_frame) = state.stack.last() else {
            continue;
        };
        match top_frame.kind {
            FrameKind::File(_) => {
                if state.peek(&tokens, &texts).kind == Kind::End {
                    state.stack.pop();
                    continue;
                }
            }
            FrameKind::Object(node) => {
                if state.at_loop_end(&tokens, &texts) {
                    let popped = state.stack.pop().expect("checked top frame");
                    if is_declaration(node)
                        && let Some(name_id) = popped.decl_name
                        && name_id != 0
                    {
                        recovered_ids.insert(name_id);
                    }
                    enqueue(state, &mut visited, &mut heap, &mut seq, &mut exhausted);
                    continue;
                }
            }
            FrameKind::KeyBlock { .. } => {
                if state.at_loop_end(&tokens, &texts) {
                    state.stack.pop();
                    enqueue(state, &mut visited, &mut heap, &mut seq, &mut exhausted);
                    continue;
                }
            }
        }

        let slots = state.key_operator_value(&tokens, &texts);
        let fresh = slots == Slots::Fresh;
        let key_token = state.key.to_token(&texts);
        let value_token = state.value.to_token(&texts);
        let current_token = state.current.to_token(&texts);
        let field = Field {
            key: &key_token,
            value: &value_token,
            fresh,
            operator: state.operator,
            current: &current_token,
            effect_scope: None,
        };
        let is_name_field = match top_frame.kind {
            FrameKind::Object(node) if is_declaration(node) => field.keyword_is("name"),
            _ => false,
        };
        let shape = match top_frame.kind {
            FrameKind::File(node) | FrameKind::Object(node) => dispatch(node, &field),
            FrameKind::KeyBlock { meta } => {
                if meta && fresh && field.keyword_is("text") {
                    Shape::Skip
                } else {
                    untraced(&field)
                }
            }
        };
        let shape = match shape {
            Shape::ObjectAs(child, _) => Shape::Object(child),
            Shape::Reported { read, .. } => *read,
            other => other,
        };

        let opens = state.value.kind == Kind::Open;
        let current_open = state.current.kind == Kind::Open;
        let tagged_open =
            !opens && is_tag(state.value, &texts) && state.peek_kind(&tokens) == Kind::Open;

        match shape {
            Shape::Discard => {
                if current_open {
                    state.skip(&tokens, &texts);
                }
                enqueue(state, &mut visited, &mut heap, &mut seq, &mut exhausted);
            }
            Shape::One | Shape::Current => {
                let scalar_id = if matches!(shape, Shape::Current) {
                    state.current.text
                } else {
                    state.value.text
                };
                if opens || tagged_open {
                    state.prune_dead_variables(&last_var_mention, &texts);
                    if !branch_visited.insert(state.clone()) {
                        continue;
                    }
                    let mut skipped = state.clone();
                    if opens {
                        skipped.skip(&tokens, &texts);
                    } else {
                        skipped.advance(&tokens, &texts);
                        skipped.skip(&tokens, &texts);
                    }
                    enqueue(skipped, &mut visited, &mut heap, &mut seq, &mut exhausted);
                }
                if is_name_field {
                    state.stack.last_mut().expect("active frame").decl_name = Some(scalar_id);
                }
                enqueue(state, &mut visited, &mut heap, &mut seq, &mut exhausted);
            }
            Shape::Object(child) => {
                if opens {
                    state.stack.push(Frame {
                        kind: FrameKind::Object(child),
                        decl_name: None,
                    });
                    enqueue(state, &mut visited, &mut heap, &mut seq, &mut exhausted);
                } else {
                    state.prune_dead_variables(&last_var_mention, &texts);
                    if !branch_visited.insert(state.clone()) {
                        continue;
                    }
                    let mut entered = state.clone();
                    entered.stack.push(Frame {
                        kind: FrameKind::Object(child),
                        decl_name: None,
                    });
                    enqueue(entered, &mut visited, &mut heap, &mut seq, &mut exhausted);

                    if tagged_open {
                        state.advance(&tokens, &texts);
                        state.skip(&tokens, &texts);
                    } else if current_open {
                        state.skip(&tokens, &texts);
                    }
                    enqueue(state, &mut visited, &mut heap, &mut seq, &mut exhausted);
                }
            }
            Shape::ObjectIfOpen(child) => {
                if opens {
                    state.stack.push(Frame {
                        kind: FrameKind::Object(child),
                        decl_name: None,
                    });
                }
                enqueue(state, &mut visited, &mut heap, &mut seq, &mut exhausted);
            }
            Shape::ObjectOrSkip(child) => {
                if opens {
                    state.stack.push(Frame {
                        kind: FrameKind::Object(child),
                        decl_name: None,
                    });
                } else if current_open {
                    state.skip(&tokens, &texts);
                }
                enqueue(state, &mut visited, &mut heap, &mut seq, &mut exhausted);
            }
            Shape::List => {
                state.list(&tokens, &texts);
                enqueue(state, &mut visited, &mut heap, &mut seq, &mut exhausted);
            }
            Shape::ListIfOpen => {
                if opens {
                    state.list(&tokens, &texts);
                }
                enqueue(state, &mut visited, &mut heap, &mut seq, &mut exhausted);
            }
            Shape::ListOrSkip => {
                if opens {
                    state.list(&tokens, &texts);
                } else if current_open {
                    state.skip(&tokens, &texts);
                }
                enqueue(state, &mut visited, &mut heap, &mut seq, &mut exhausted);
            }
            Shape::Skip => {
                if current_open {
                    state.skip(&tokens, &texts);
                }
                enqueue(state, &mut visited, &mut heap, &mut seq, &mut exhausted);
            }
            Shape::SkipIfOpen => {
                if opens {
                    state.skip_block(&tokens, &texts);
                }
                enqueue(state, &mut visited, &mut heap, &mut seq, &mut exhausted);
            }
            Shape::RawSkip => {
                state.skip_block(&tokens, &texts);
                enqueue(state, &mut visited, &mut heap, &mut seq, &mut exhausted);
            }
            Shape::ByKey => {
                state.prune_dead_variables(&last_var_mention, &texts);
                if !branch_visited.insert(state.clone()) {
                    continue;
                }
                if opens && current_open {
                    let meta = is_meta_key(state.key, &texts);
                    let mut keyed = state.clone();
                    keyed.stack.push(Frame {
                        kind: FrameKind::KeyBlock { meta },
                        decl_name: None,
                    });
                    enqueue(keyed, &mut visited, &mut heap, &mut seq, &mut exhausted);

                    let mut skipped = state.clone();
                    skipped.skip(&tokens, &texts);
                    enqueue(skipped, &mut visited, &mut heap, &mut seq, &mut exhausted);

                    let mut listed = state.clone();
                    listed.list(&tokens, &texts);
                    enqueue(listed, &mut visited, &mut heap, &mut seq, &mut exhausted);

                    let mut raw_skipped = state.clone();
                    raw_skipped.skip_block(&tokens, &texts);
                    enqueue(
                        raw_skipped,
                        &mut visited,
                        &mut heap,
                        &mut seq,
                        &mut exhausted,
                    );

                    state.skip_past_embedded_end(&tokens, &texts);
                    enqueue(state, &mut visited, &mut heap, &mut seq, &mut exhausted);
                } else if opens {
                    let mut keyed = state.clone();
                    keyed.stack.push(Frame {
                        kind: FrameKind::KeyBlock { meta: false },
                        decl_name: None,
                    });
                    enqueue(keyed, &mut visited, &mut heap, &mut seq, &mut exhausted);

                    let mut listed = state.clone();
                    listed.list(&tokens, &texts);
                    enqueue(listed, &mut visited, &mut heap, &mut seq, &mut exhausted);

                    enqueue(state, &mut visited, &mut heap, &mut seq, &mut exhausted);
                } else if current_open {
                    enqueue(
                        state.clone(),
                        &mut visited,
                        &mut heap,
                        &mut seq,
                        &mut exhausted,
                    );

                    state.skip(&tokens, &texts);
                    enqueue(state, &mut visited, &mut heap, &mut seq, &mut exhausted);
                } else {
                    if tagged_open {
                        let mut skipped = state.clone();
                        skipped.advance(&tokens, &texts);
                        skipped.skip(&tokens, &texts);
                        enqueue(skipped, &mut visited, &mut heap, &mut seq, &mut exhausted);
                    }
                    enqueue(state, &mut visited, &mut heap, &mut seq, &mut exhausted);
                }
            }
            Shape::Unsettled(_) => {
                state.prune_dead_variables(&last_var_mention, &texts);
                if !branch_visited.insert(state.clone()) {
                    continue;
                }
                if current_open {
                    enqueue(
                        state.clone(),
                        &mut visited,
                        &mut heap,
                        &mut seq,
                        &mut exhausted,
                    );

                    state.skip(&tokens, &texts);
                    enqueue(state, &mut visited, &mut heap, &mut seq, &mut exhausted);
                } else if opens {
                    let mut keyed = state.clone();
                    keyed.stack.push(Frame {
                        kind: FrameKind::KeyBlock { meta: false },
                        decl_name: None,
                    });
                    enqueue(keyed, &mut visited, &mut heap, &mut seq, &mut exhausted);

                    let mut listed = state.clone();
                    listed.list(&tokens, &texts);
                    enqueue(listed, &mut visited, &mut heap, &mut seq, &mut exhausted);

                    enqueue(state, &mut visited, &mut heap, &mut seq, &mut exhausted);
                } else {
                    enqueue(state, &mut visited, &mut heap, &mut seq, &mut exhausted);
                }
            }
            _ => {
                if current_open {
                    state.skip(&tokens, &texts);
                }
                enqueue(state, &mut visited, &mut heap, &mut seq, &mut exhausted);
            }
        }
    }

    let mut names: BTreeSet<Vec<u8>> = recovered_ids
        .into_iter()
        .map(|id| texts[id as usize].to_vec())
        .collect();

    if exhausted {
        for token in &lexed.tokens {
            if token.text.is_empty() {
                continue;
            }
            if token.kind == Kind::End && matches!(token.text.as_slice(), b"!" | b"#" | b";") {
                continue;
            }
            names.insert(token.text.clone());
        }
    }

    ExploredNames { names, exhausted }
}
