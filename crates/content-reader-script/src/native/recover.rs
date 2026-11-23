use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap, HashSet};
use std::hash::Hash;

use super::{Field, Kind, Mode, Shape, Slots, Token, lex, untraced};
use content_reader_parser::parser::Operator;

const MAX_AMBIGUITY_WORK: usize = 65_536;
const MAX_FRONTIER_STATES: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NamePolicy {
    FirstExact,
    LastKeyword,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryShape<S> {
    Exact(Shape<S>),
    ObjectOrSkip(S),
    OneOrSkip,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExploredNames {
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
        let hash = content_reader_parser::parser::variable_hash(&bytes[1..]);
        match self.variables.binary_search_by_key(&hash, |(h, _)| *h) {
            Ok(index) => self.variables[index].1,
            Err(_) => token,
        }
    }

    fn define(&mut self, tokens: &[SmallToken], texts: &[&[u8]]) {
        let name = self.current;
        let hash = content_reader_parser::parser::variable_hash(&texts[name.text as usize][1..]);
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

    fn balanced(&mut self, tokens: &[SmallToken], texts: &[&[u8]]) -> bool {
        let mut depth = 1_usize;
        while depth > 0 {
            let raw_token = self.peek(tokens, texts);
            self.advance(tokens, texts);
            if raw_token.kind == Kind::End {
                self.current = self.resolve(raw_token, texts);
                return !has_tokens_after_end(tokens, self.next, texts);
            }
            match raw_token.kind {
                Kind::Close => {
                    depth -= 1;
                }
                Kind::Open => {
                    depth += 1;
                }
                _ => {
                    let token = self.resolve(raw_token, texts);
                    if matches!(token.kind, Kind::Open | Kind::Close) {
                        return false;
                    }
                    if self.peek(tokens, texts).operator() {
                        self.advance(tokens, texts);
                        if self.peek(tokens, texts).kind == Kind::Close {
                            return false;
                        }
                        self.advance(tokens, texts);
                        let raw_value = self.current;
                        let value = self.resolve(raw_value, texts);
                        if raw_value.kind == Kind::End
                            && has_tokens_after_end(tokens, self.next, texts)
                        {
                            return false;
                        }
                        if raw_value.kind != value.kind
                            && matches!(value.kind, Kind::Open | Kind::Close)
                        {
                            return false;
                        }
                        if value.kind == Kind::Open {
                            depth += 1;
                        }
                    } else {
                        self.value = token;
                    }
                }
            }
        }
        true
    }

    fn prune_dead_variables(&mut self, last_var_mention: &HashMap<u32, usize>, texts: &[&[u8]]) {
        let effective_pos = if self.pending {
            self.next.saturating_sub(1)
        } else {
            self.next
        };
        let current_hash = texts[self.current.text as usize]
            .strip_prefix(b"@")
            .map(content_reader_parser::parser::variable_hash);
        self.variables.retain(|(hash, _)| {
            current_hash == Some(*hash)
                || last_var_mention
                    .get(hash)
                    .is_some_and(|&last| last >= effective_pos)
        });
    }
}

fn has_tokens_after_end(tokens: &[SmallToken], next: usize, texts: &[&[u8]]) -> bool {
    tokens[next.min(tokens.len())..]
        .iter()
        .any(|token| token.kind != Kind::End || !texts[token.text as usize].is_empty())
}

fn scan_keyless_eligible(
    tokens: &[SmallToken],
    ends: &[usize],
    start: usize,
    texts: &[&[u8]],
) -> bool {
    let start = start.min(tokens.len());
    let end = ends
        .get(ends.partition_point(|&at| at < start))
        .map_or(tokens.len(), |&at| at);
    let slice = &tokens[start..end];
    let mut at = 0;
    let mut keyless = false;
    while let Some(&token) = slice.get(at) {
        match token.kind {
            Kind::Close => break,
            Kind::Open => {
                keyless = true;
                let (next_at, has_def) = scan_block_has_def(slice, at, texts);
                if has_def {
                    return false;
                }
                at = next_at;
                continue;
            }
            _ => {}
        }
        let operator = slice.get(at + 1).is_some_and(|next| next.operator());
        if !operator {
            keyless = true;
            at += 1;
            continue;
        }
        if texts[token.text as usize].starts_with(b"@") {
            return false;
        }
        at += 2;
        match slice.get(at).map(|value| value.kind) {
            Some(Kind::Open) => {
                let (next_at, has_def) = scan_block_has_def(slice, at, texts);
                if has_def {
                    return false;
                }
                at = next_at;
            }
            Some(Kind::Close) | None => break,
            Some(_) => at += 1,
        }
    }
    keyless
}

fn scan_block_has_def(tokens: &[SmallToken], mut at: usize, texts: &[&[u8]]) -> (usize, bool) {
    let mut depth = 0_usize;
    let mut has_def = false;
    while let Some(&token) = tokens.get(at) {
        if depth > 0
            && !has_def
            && texts[token.text as usize].starts_with(b"@")
            && tokens.get(at + 1).is_some_and(|next| next.operator())
        {
            has_def = true;
        }
        at += 1;
        match token.kind {
            Kind::Open => depth += 1,
            Kind::Close => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
    }
    (at, has_def)
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
    let key = content_reader_parser::parser::c_string(texts[token.text as usize]);
    content_reader_parser::parser::looked_up(key, token.quoted)
        && matches!(
            key.to_ascii_lowercase().as_slice(),
            b"meta_effect" | b"meta_trigger"
        )
}

fn apply_decl_name<S>(
    policy: Option<NamePolicy>,
    frame: &mut Frame<S>,
    key: SmallToken,
    scalar_id: u32,
    is_scalar: bool,
    texts: &[&[u8]],
) {
    let key_bytes = texts[key.text as usize];
    match policy {
        Some(NamePolicy::LastKeyword) => {
            if is_scalar {
                let c_key = content_reader_parser::parser::c_string(key_bytes);
                if content_reader_parser::parser::looked_up(c_key, key.quoted) && c_key.eq_ignore_ascii_case(b"name")
                {
                    frame.decl_name = Some(scalar_id);
                }
            }
        }
        Some(NamePolicy::FirstExact) => {
            if key_bytes == b"name" && frame.decl_name.is_none() {
                frame.decl_name = Some(if is_scalar { scalar_id } else { 0 });
            }
        }
        None => {}
    }
}

struct Frontier<S> {
    visited: BTreeMap<usize, HashSet<State<S>>>,
    branch_visited: BTreeMap<usize, HashSet<State<S>>>,
    heap: BinaryHeap<HeapItem<S>>,
    frontier_states: usize,
    ambiguity_work: usize,
    seq: usize,
    exhausted: bool,
}

impl<S: Copy + Eq + Hash> Frontier<S> {
    fn new() -> Self {
        Self {
            visited: BTreeMap::new(),
            branch_visited: BTreeMap::new(),
            heap: BinaryHeap::new(),
            frontier_states: 0,
            ambiguity_work: 0,
            seq: 0,
            exhausted: false,
        }
    }

    fn retire_before(&mut self, cursor: usize) {
        while let Some((&next, _)) = self.visited.first_key_value() {
            if next < cursor {
                if let Some((_, bucket)) = self.visited.pop_first() {
                    self.frontier_states -= bucket.len();
                }
            } else {
                break;
            }
        }
        while let Some((&next, _)) = self.branch_visited.first_key_value() {
            if next < cursor {
                self.branch_visited.pop_first();
            } else {
                break;
            }
        }
    }

    fn mark_branch(&mut self, state: &State<S>) -> bool {
        self.branch_visited
            .entry(state.next)
            .or_default()
            .insert(state.clone())
    }

    fn enqueue(
        &mut self,
        mut state: State<S>,
        last_var_mention: &HashMap<u32, usize>,
        texts: &[&[u8]],
        charge_ambiguity: bool,
    ) {
        if self.exhausted {
            return;
        }
        state.prune_dead_variables(last_var_mention, texts);
        let next = state.next;
        let bucket = self.visited.entry(next).or_default();
        if !bucket.insert(state.clone()) {
            return;
        }
        self.frontier_states += 1;
        if charge_ambiguity {
            self.ambiguity_work += 1;
        }
        if self.frontier_states >= MAX_FRONTIER_STATES || self.ambiguity_work >= MAX_AMBIGUITY_WORK
        {
            self.exhausted = true;
            return;
        }
        let seq = self.seq;
        self.seq += 1;
        self.heap.push(HeapItem { next, seq, state });
    }
}

pub fn recover_declared_names<S>(
    bytes: &[u8],
    mode: Mode,
    top: S,
    declaration_name: impl Fn(S) -> Option<NamePolicy>,
    dispatch: impl Fn(S, &Field<'_>) -> RecoveryShape<S>,
) -> ExploredNames
where
    S: Copy + Eq + Hash,
{
    let lexed = lex(bytes, mode);
    let mut texts: Vec<&[u8]> = vec![b""];
    let mut text_map: HashMap<&[u8], u32> = HashMap::new();
    text_map.insert(b"", 0);
    let mut tokens = Vec::with_capacity(lexed.tokens.len());
    let mut ends = Vec::new();
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
        if token.kind == Kind::End {
            ends.push(index);
        }
        if let Some(var) = slice.strip_prefix(b"@") {
            last_var_mention.insert(content_reader_parser::parser::variable_hash(var), index);
        }
        tokens.push(SmallToken {
            kind: token.kind,
            quoted: token.quoted,
            text: id,
        });
    }

    let mut frontier = Frontier::new();
    let mut recovered_ids: BTreeSet<u32> = BTreeSet::new();

    frontier.enqueue(State::new(top), &last_var_mention, &texts, false);

    while let Some(HeapItem {
        next: cursor,
        mut state,
        ..
    }) = frontier.heap.pop()
    {
        if frontier.exhausted {
            break;
        }
        frontier.retire_before(cursor);
        let concurrent = !frontier.heap.is_empty();
        if concurrent {
            frontier.ambiguity_work += 1;
            if frontier.ambiguity_work >= MAX_AMBIGUITY_WORK {
                frontier.exhausted = true;
                break;
            }
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
                    if declaration_name(node).is_some()
                        && let Some(name_id) = popped.decl_name
                        && name_id != 0
                    {
                        recovered_ids.insert(name_id);
                    }
                    frontier.enqueue(state, &last_var_mention, &texts, false);
                    continue;
                }
            }
            FrameKind::KeyBlock { .. } => {
                if state.at_loop_end(&tokens, &texts) {
                    state.stack.pop();
                    frontier.enqueue(state, &last_var_mention, &texts, false);
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
        let decl_policy = match top_frame.kind {
            FrameKind::Object(node) => declaration_name(node),
            FrameKind::File(_) | FrameKind::KeyBlock { .. } => None,
        };
        let shape = match top_frame.kind {
            FrameKind::File(node) | FrameKind::Object(node) => dispatch(node, &field),
            FrameKind::KeyBlock { meta } => {
                if meta && fresh && field.keyword_is("text") {
                    RecoveryShape::Exact(Shape::Skip)
                } else {
                    RecoveryShape::Exact(untraced(&field))
                }
            }
        };
        let shape = match shape {
            RecoveryShape::Exact(Shape::ObjectAs(child, _)) => {
                RecoveryShape::Exact(Shape::Object(child))
            }
            RecoveryShape::Exact(Shape::Reported { read, .. }) => RecoveryShape::Exact(*read),
            other => other,
        };

        let opens = state.value.kind == Kind::Open;
        let current_open = state.current.kind == Kind::Open;
        let tagged_open =
            !opens && is_tag(state.value, &texts) && state.peek_kind(&tokens) == Kind::Open;

        match shape {
            RecoveryShape::Exact(Shape::Discard) => {
                if current_open {
                    state.skip(&tokens, &texts);
                }
                frontier.enqueue(state, &last_var_mention, &texts, false);
            }
            RecoveryShape::Exact(Shape::One | Shape::Current) => {
                let is_current = matches!(shape, RecoveryShape::Exact(Shape::Current));
                let scalar_id = if is_current {
                    state.current.text
                } else {
                    state.value.text
                };
                let key = state.key;
                apply_decl_name(
                    decl_policy,
                    state.stack.last_mut().expect("active frame"),
                    key,
                    scalar_id,
                    true,
                    &texts,
                );
                frontier.enqueue(state, &last_var_mention, &texts, false);
            }
            RecoveryShape::OneOrSkip => {
                let scalar_id = state.value.text;
                let key = state.key;
                if opens || tagged_open {
                    state.prune_dead_variables(&last_var_mention, &texts);
                    if !frontier.mark_branch(&state) {
                        continue;
                    }
                    let mut skipped = state.clone();
                    if opens {
                        skipped.skip(&tokens, &texts);
                    } else {
                        skipped.advance(&tokens, &texts);
                        skipped.skip(&tokens, &texts);
                    }
                    apply_decl_name(
                        decl_policy,
                        skipped.stack.last_mut().expect("active frame"),
                        key,
                        0,
                        false,
                        &texts,
                    );
                    frontier.enqueue(skipped, &last_var_mention, &texts, true);
                }
                apply_decl_name(
                    decl_policy,
                    state.stack.last_mut().expect("active frame"),
                    key,
                    scalar_id,
                    true,
                    &texts,
                );
                frontier.enqueue(state, &last_var_mention, &texts, false);
            }
            RecoveryShape::Exact(Shape::Object(child)) => {
                let key = state.key;
                apply_decl_name(
                    decl_policy,
                    state.stack.last_mut().expect("active frame"),
                    key,
                    0,
                    false,
                    &texts,
                );
                state.stack.push(Frame {
                    kind: FrameKind::Object(child),
                    decl_name: None,
                });
                frontier.enqueue(state, &last_var_mention, &texts, false);
            }
            RecoveryShape::ObjectOrSkip(child) => {
                state.prune_dead_variables(&last_var_mention, &texts);
                if !frontier.mark_branch(&state) {
                    continue;
                }
                let key = state.key;
                let mut entered = state.clone();
                apply_decl_name(
                    decl_policy,
                    entered.stack.last_mut().expect("active frame"),
                    key,
                    0,
                    false,
                    &texts,
                );
                entered.stack.push(Frame {
                    kind: FrameKind::Object(child),
                    decl_name: None,
                });
                frontier.enqueue(entered, &last_var_mention, &texts, false);

                if opens {
                    let mut skipped = state.clone();
                    skipped.skip(&tokens, &texts);
                    apply_decl_name(
                        decl_policy,
                        skipped.stack.last_mut().expect("active frame"),
                        key,
                        0,
                        false,
                        &texts,
                    );
                    frontier.enqueue(skipped, &last_var_mention, &texts, true);

                    state.skip_block(&tokens, &texts);
                    apply_decl_name(
                        decl_policy,
                        state.stack.last_mut().expect("active frame"),
                        key,
                        0,
                        false,
                        &texts,
                    );
                    frontier.enqueue(state, &last_var_mention, &texts, true);
                } else {
                    if tagged_open {
                        state.advance(&tokens, &texts);
                        state.skip(&tokens, &texts);
                    } else if current_open {
                        state.skip(&tokens, &texts);
                    }
                    let scalar_id = state.value.text;
                    apply_decl_name(
                        decl_policy,
                        state.stack.last_mut().expect("active frame"),
                        key,
                        scalar_id,
                        !tagged_open && !current_open,
                        &texts,
                    );
                    frontier.enqueue(state, &last_var_mention, &texts, true);
                }
            }
            RecoveryShape::Exact(Shape::ObjectIfOpen(child)) => {
                let key = state.key;
                let scalar_id = state.value.text;
                apply_decl_name(
                    decl_policy,
                    state.stack.last_mut().expect("active frame"),
                    key,
                    scalar_id,
                    !opens,
                    &texts,
                );
                if opens {
                    state.stack.push(Frame {
                        kind: FrameKind::Object(child),
                        decl_name: None,
                    });
                }
                frontier.enqueue(state, &last_var_mention, &texts, false);
            }
            RecoveryShape::Exact(Shape::ObjectOrSkip(child)) => {
                let key = state.key;
                let scalar_id = state.value.text;
                apply_decl_name(
                    decl_policy,
                    state.stack.last_mut().expect("active frame"),
                    key,
                    scalar_id,
                    !opens && !current_open,
                    &texts,
                );
                if opens {
                    state.stack.push(Frame {
                        kind: FrameKind::Object(child),
                        decl_name: None,
                    });
                } else if current_open {
                    state.skip(&tokens, &texts);
                }
                frontier.enqueue(state, &last_var_mention, &texts, false);
            }
            RecoveryShape::Exact(Shape::List) => {
                let key = state.key;
                apply_decl_name(
                    decl_policy,
                    state.stack.last_mut().expect("active frame"),
                    key,
                    0,
                    false,
                    &texts,
                );
                state.list(&tokens, &texts);
                frontier.enqueue(state, &last_var_mention, &texts, false);
            }
            RecoveryShape::Exact(Shape::ListIfOpen) => {
                let key = state.key;
                let scalar_id = state.value.text;
                apply_decl_name(
                    decl_policy,
                    state.stack.last_mut().expect("active frame"),
                    key,
                    scalar_id,
                    !opens,
                    &texts,
                );
                if opens {
                    state.list(&tokens, &texts);
                }
                frontier.enqueue(state, &last_var_mention, &texts, false);
            }
            RecoveryShape::Exact(Shape::ListOrSkip) => {
                let key = state.key;
                let scalar_id = state.value.text;
                apply_decl_name(
                    decl_policy,
                    state.stack.last_mut().expect("active frame"),
                    key,
                    scalar_id,
                    !opens && !current_open,
                    &texts,
                );
                if opens {
                    state.list(&tokens, &texts);
                } else if current_open {
                    state.skip(&tokens, &texts);
                }
                frontier.enqueue(state, &last_var_mention, &texts, false);
            }
            RecoveryShape::Exact(Shape::Skip) => {
                let key = state.key;
                let scalar_id = state.value.text;
                apply_decl_name(
                    decl_policy,
                    state.stack.last_mut().expect("active frame"),
                    key,
                    scalar_id,
                    !current_open,
                    &texts,
                );
                if current_open {
                    state.skip(&tokens, &texts);
                }
                frontier.enqueue(state, &last_var_mention, &texts, false);
            }
            RecoveryShape::Exact(Shape::SkipIfOpen) => {
                let key = state.key;
                let scalar_id = state.value.text;
                apply_decl_name(
                    decl_policy,
                    state.stack.last_mut().expect("active frame"),
                    key,
                    scalar_id,
                    !opens,
                    &texts,
                );
                if opens {
                    state.skip_block(&tokens, &texts);
                }
                frontier.enqueue(state, &last_var_mention, &texts, false);
            }
            RecoveryShape::Exact(Shape::RawSkip) => {
                let key = state.key;
                apply_decl_name(
                    decl_policy,
                    state.stack.last_mut().expect("active frame"),
                    key,
                    0,
                    false,
                    &texts,
                );
                state.skip_block(&tokens, &texts);
                frontier.enqueue(state, &last_var_mention, &texts, false);
            }
            RecoveryShape::Exact(Shape::ByKey) => {
                state.prune_dead_variables(&last_var_mention, &texts);
                if !frontier.mark_branch(&state) {
                    continue;
                }
                let key = state.key;
                let scalar_id = state.value.text;
                if opens && current_open {
                    let meta = is_meta_key(state.key, &texts);
                    let scan_start = if state.pending {
                        state.next.saturating_sub(1)
                    } else {
                        state.next
                    };
                    let balanced_eligible =
                        !meta && scan_keyless_eligible(&tokens, &ends, scan_start, &texts);
                    apply_decl_name(
                        decl_policy,
                        state.stack.last_mut().expect("active frame"),
                        key,
                        0,
                        false,
                        &texts,
                    );
                    let mut keyed = state.clone();
                    keyed.stack.push(Frame {
                        kind: FrameKind::KeyBlock { meta },
                        decl_name: None,
                    });
                    frontier.enqueue(keyed, &last_var_mention, &texts, false);

                    let mut skipped = state.clone();
                    skipped.skip(&tokens, &texts);
                    frontier.enqueue(skipped, &last_var_mention, &texts, true);

                    let mut listed = state.clone();
                    listed.list(&tokens, &texts);
                    frontier.enqueue(listed, &last_var_mention, &texts, true);

                    if balanced_eligible {
                        let mut balanced = state.clone();
                        if balanced.balanced(&tokens, &texts) {
                            frontier.enqueue(balanced, &last_var_mention, &texts, true);
                        }
                    }

                    let mut raw_skipped = state.clone();
                    raw_skipped.skip_block(&tokens, &texts);
                    frontier.enqueue(raw_skipped, &last_var_mention, &texts, true);

                    state.skip_past_embedded_end(&tokens, &texts);
                    frontier.enqueue(state, &last_var_mention, &texts, true);
                } else if opens {
                    let mut keyed = state.clone();
                    apply_decl_name(
                        decl_policy,
                        keyed.stack.last_mut().expect("active frame"),
                        key,
                        0,
                        false,
                        &texts,
                    );
                    keyed.stack.push(Frame {
                        kind: FrameKind::KeyBlock { meta: false },
                        decl_name: None,
                    });
                    frontier.enqueue(keyed, &last_var_mention, &texts, true);

                    let mut listed = state.clone();
                    apply_decl_name(
                        decl_policy,
                        listed.stack.last_mut().expect("active frame"),
                        key,
                        0,
                        false,
                        &texts,
                    );
                    listed.list(&tokens, &texts);
                    frontier.enqueue(listed, &last_var_mention, &texts, true);

                    apply_decl_name(
                        decl_policy,
                        state.stack.last_mut().expect("active frame"),
                        key,
                        scalar_id,
                        true,
                        &texts,
                    );
                    frontier.enqueue(state, &last_var_mention, &texts, false);
                } else if current_open {
                    let mut one = state.clone();
                    apply_decl_name(
                        decl_policy,
                        one.stack.last_mut().expect("active frame"),
                        key,
                        scalar_id,
                        true,
                        &texts,
                    );
                    frontier.enqueue(one, &last_var_mention, &texts, false);

                    apply_decl_name(
                        decl_policy,
                        state.stack.last_mut().expect("active frame"),
                        key,
                        0,
                        false,
                        &texts,
                    );
                    state.skip(&tokens, &texts);
                    frontier.enqueue(state, &last_var_mention, &texts, true);
                } else {
                    if tagged_open {
                        let mut skipped = state.clone();
                        apply_decl_name(
                            decl_policy,
                            skipped.stack.last_mut().expect("active frame"),
                            key,
                            0,
                            false,
                            &texts,
                        );
                        skipped.advance(&tokens, &texts);
                        skipped.skip(&tokens, &texts);
                        frontier.enqueue(skipped, &last_var_mention, &texts, true);
                    }
                    apply_decl_name(
                        decl_policy,
                        state.stack.last_mut().expect("active frame"),
                        key,
                        scalar_id,
                        true,
                        &texts,
                    );
                    frontier.enqueue(state, &last_var_mention, &texts, false);
                }
            }
            RecoveryShape::Exact(Shape::Unsettled(_)) => {
                state.prune_dead_variables(&last_var_mention, &texts);
                if !frontier.mark_branch(&state) {
                    continue;
                }
                let key = state.key;
                let scalar_id = state.value.text;
                if current_open {
                    let mut one = state.clone();
                    apply_decl_name(
                        decl_policy,
                        one.stack.last_mut().expect("active frame"),
                        key,
                        scalar_id,
                        true,
                        &texts,
                    );
                    frontier.enqueue(one, &last_var_mention, &texts, false);

                    apply_decl_name(
                        decl_policy,
                        state.stack.last_mut().expect("active frame"),
                        key,
                        0,
                        false,
                        &texts,
                    );
                    state.skip(&tokens, &texts);
                    frontier.enqueue(state, &last_var_mention, &texts, true);
                } else if opens {
                    let mut keyed = state.clone();
                    apply_decl_name(
                        decl_policy,
                        keyed.stack.last_mut().expect("active frame"),
                        key,
                        0,
                        false,
                        &texts,
                    );
                    keyed.stack.push(Frame {
                        kind: FrameKind::KeyBlock { meta: false },
                        decl_name: None,
                    });
                    frontier.enqueue(keyed, &last_var_mention, &texts, true);

                    let mut listed = state.clone();
                    apply_decl_name(
                        decl_policy,
                        listed.stack.last_mut().expect("active frame"),
                        key,
                        0,
                        false,
                        &texts,
                    );
                    listed.list(&tokens, &texts);
                    frontier.enqueue(listed, &last_var_mention, &texts, true);

                    apply_decl_name(
                        decl_policy,
                        state.stack.last_mut().expect("active frame"),
                        key,
                        scalar_id,
                        true,
                        &texts,
                    );
                    frontier.enqueue(state, &last_var_mention, &texts, false);
                } else {
                    apply_decl_name(
                        decl_policy,
                        state.stack.last_mut().expect("active frame"),
                        key,
                        scalar_id,
                        true,
                        &texts,
                    );
                    frontier.enqueue(state, &last_var_mention, &texts, false);
                }
            }
            RecoveryShape::Exact(_) => {
                let key = state.key;
                let scalar_id = state.value.text;
                apply_decl_name(
                    decl_policy,
                    state.stack.last_mut().expect("active frame"),
                    key,
                    scalar_id,
                    !current_open,
                    &texts,
                );
                if current_open {
                    state.skip(&tokens, &texts);
                }
                frontier.enqueue(state, &last_var_mention, &texts, false);
            }
        }
    }

    let mut names: BTreeSet<Vec<u8>> = recovered_ids
        .into_iter()
        .map(|id| texts[id as usize].to_vec())
        .collect();

    if frontier.exhausted {
        for token in &lexed.tokens {
            if !token.text.is_empty() {
                names.insert(token.text.clone());
            }
        }
    }

    ExploredNames {
        names,
        exhausted: frontier.exhausted,
    }
}
