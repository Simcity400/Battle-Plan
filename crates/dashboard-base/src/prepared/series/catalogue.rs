//! Catalogue sections hold a compressed base text and then patches, each
//! holding only what changed since the text before it, so a flush writes what
//! its block changed rather than the whole catalogue.
//!
//! Range 0 is the base. Ranges 1.. are patches, in order, each from the text
//! the ranges before it give to the next text; ranges past the last patch are
//! empty. A flush whose text is unchanged writes nothing. Otherwise it writes
//! one patch, after merging its newest patches into it while the newest one
//! is less than twice the size of the merged patch, so every patch is at
//! least twice the size of the next one and a chain holds a logarithmic
//! number of them. Once the patches would outweigh the base, the flush writes
//! a new base instead and empties the patch ranges. Every rewritten byte was
//! preceded by at least as many written bytes of change, so bytes written
//! stay proportional to what changed, and a reader reads at most about twice
//! the base.
//!
//! A text is serde_json's compact output. A patch edits its containers
//! (objects and arrays) structurally: each container edit lists, in order,
//! runs of earlier children kept, earlier children edited in place and new
//! children's text, so reconstruction reproduces the new text byte for byte.
//! Each patch records its result's length and hash, which every reader
//! checks; a writer applies each patch before writing it and writes a base
//! instead if the patch does not reproduce its text.
use super::{compress, decompress, invalid};
use crate::prepared_chart::{take, var};
use std::{
    borrow::Cow,
    collections::HashMap,
    io::{self, Read},
};

/// First byte of a base range; the compressed text follows.
const BASE: u8 = b'B';
/// First byte of a patch range: the lengths of the text it applies to and of
/// its result, the result's hash, then the compressed edits.
const PATCH: u8 = b'P';
const HASH: usize = 16;
const LEVEL: i32 = 3;
/// Keep `count` earlier children from `start`.
const KEEP: u128 = 0;
/// Edit the earlier container child `index` (an object member's value).
const EDIT: u128 = 1;
/// `count` new children, their text joined by commas.
const NEW: u128 = 2;

const MALFORMED: &str = "Prepared catalogue text is malformed";
const UNPAIRED: &str = "Prepared catalogue patch does not fit its text";

/// Publishes `text` as the current catalogue of `section`.
pub(super) fn write<W: crate::prepared::pages::Pages + ?Sized>(
    writer: &mut W,
    section: &str,
    text: &[u8],
) -> io::Result<()> {
    Published::read(writer, section)?.write(writer, text)
}

/// The last publication, reused only while every live range and its
/// terminating slot keep their immutable identities in the current writer.
/// Other publishers, abandoned transactions and store replacement invalidate
/// it. Restoration starts with no cache.
#[derive(Default)]
pub(super) struct Cached(Option<Published>);
struct Published {
    section: String,
    stored: Vec<Vec<u8>>,
    text: Vec<u8>,
    ids: Option<Vec<Option<store::RangeId>>>,
}
impl Cached {
    pub(super) fn write<W: crate::prepared::pages::Pages + ?Sized>(
        &mut self,
        writer: &mut W,
        section: &str,
        text: &[u8],
    ) -> io::Result<()> {
        // A failed publication forgets the cache, including partial writes.
        let mut previous = self.0.take().filter(|held| held.section == section);
        if let Some(held) = &previous {
            let ids = writer.range_ids(section, held.stored.len() as u64 + 1)?;
            if ids.is_none() || ids != held.ids {
                previous = None;
            }
        }
        let mut held = match previous {
            Some(held) => held,
            None => Published::read(writer, section)?,
        };
        let changed = held.stored.is_empty() || held.text != text;
        held.write(writer, text)?;
        if changed || held.ids.is_none() {
            held.ids = writer.range_ids(section, held.stored.len() as u64 + 1)?;
        }
        self.0 = held.ids.is_some().then_some(held);
        Ok(())
    }
}
impl Published {
    fn read<W: crate::prepared::pages::Pages + ?Sized>(
        writer: &mut W,
        section: &str,
    ) -> io::Result<Self> {
        let mut stored = Vec::new();
        while let Some(bytes) = writer.read_at(section, stored.len() as u64)? {
            if bytes.is_empty() {
                break;
            }
            stored.push(bytes);
        }
        let current = if stored.is_empty() {
            Vec::new()
        } else {
            let origin = decode_base(&stored[0])?;
            let patches = stored[1..]
                .iter()
                .map(|bytes| Patch::decode(bytes))
                .collect::<io::Result<Vec<_>>>()?;
            rebuild(&origin, &patches)?.into_owned()
        };
        Ok(Self {
            section: section.to_owned(),
            stored,
            text: current,
            ids: None,
        })
    }
    fn write<W: crate::prepared::pages::Pages + ?Sized>(
        &mut self,
        writer: &mut W,
        text: &[u8],
    ) -> io::Result<()> {
        if self.stored.is_empty() {
            let range = base(text)?;
            writer.write_at(&self.section, 0, &range)?;
            #[cfg(any(test, feature = "test-support"))]
            writer.note_framing(&self.section, 0, framing(&range));
            self.stored.push(range);
        } else {
            if self.text == text {
                return Ok(());
            }
            let stored = &self.stored;
            let newest = stored.len() - 1;
            let next = patch(&self.text, text)?;
            let range = match next {
                Some(range) => {
                    let mut merged = newest;
                    let mut size = range.len();
                    while merged > 0 && stored[merged].len() < 2 * size {
                        size += stored[merged].len();
                        merged -= 1;
                    }
                    let chain = size + stored[1..=merged].iter().map(Vec::len).sum::<usize>();
                    if chain >= stored[0].len() {
                        None
                    } else if merged == newest {
                        Some((merged + 1, range))
                    } else {
                        let origin = decode_base(&stored[0])?;
                        let patches = stored[1..=merged]
                            .iter()
                            .map(|bytes| Patch::decode(bytes))
                            .collect::<io::Result<Vec<_>>>()?;
                        patch(&rebuild(&origin, &patches)?, text)?.map(|range| (merged + 1, range))
                    }
                }
                None => None,
            };
            let (slot, range) = match range {
                Some(written) => written,
                None => (0, base(text)?),
            };
            writer.write_at(&self.section, slot as u64, &range)?;
            #[cfg(any(test, feature = "test-support"))]
            writer.note_framing(&self.section, slot as u64, framing(&range));
            for stale in slot + 1..=newest {
                writer.write_at(&self.section, stale as u64, &[])?;
            }
            self.stored.truncate(slot);
            self.stored.push(range);
        }
        self.text.clear();
        self.text.extend_from_slice(text);
        Ok(())
    }
}

#[cfg(any(test, feature = "test-support"))]
fn framing(range: &[u8]) -> usize {
    let mut payload = &range[1..];
    if range[0] == PATCH {
        take(&mut payload).expect("written catalogue source length");
        take(&mut payload).expect("written catalogue result length");
        payload = &payload[HASH..];
    } else {
        assert_eq!(range[0], BASE);
    }
    range.len() - payload.len() + crate::prepared::pages::framing::compressed(payload)
}

/// The current text of a catalogue whose ranges `open` returns by ordinal.
pub(super) fn read(
    open: impl FnMut(u64) -> io::Result<Option<store::Range>>,
) -> io::Result<Option<Vec<u8>>> {
    let Some(stored) = ranges(open)? else {
        return Ok(None);
    };
    let origin = decode_base(&stored[0])?;
    let patches = stored[1..]
        .iter()
        .map(|bytes| Patch::decode(bytes))
        .collect::<io::Result<Vec<_>>>()?;
    if patches.is_empty() {
        return Ok(Some(origin));
    }
    Ok(Some(rebuild(&origin, &patches)?.into_owned()))
}

/// The identities of a catalogue's base and patch ranges, which change
/// whenever its text does.
pub(super) fn stamps(
    mut open: impl FnMut(u64) -> io::Result<Option<store::Range>>,
) -> io::Result<Option<Vec<store::RangeId>>> {
    let mut stamps = Vec::new();
    while let Some(range) = open(stamps.len() as u64)? {
        if range.is_empty() {
            break;
        }
        stamps.push(range.id());
    }
    Ok((!stamps.is_empty()).then_some(stamps))
}

/// The bytes of the base and patch ranges, up to the first missing or empty
/// one; None without a base.
fn ranges(
    mut open: impl FnMut(u64) -> io::Result<Option<store::Range>>,
) -> io::Result<Option<Vec<Vec<u8>>>> {
    let mut stored = Vec::new();
    while let Some(mut range) = open(stored.len() as u64)? {
        if range.is_empty() {
            break;
        }
        let mut bytes = Vec::with_capacity(usize::try_from(range.len()).unwrap_or(0));
        range.read_to_end(&mut bytes)?;
        stored.push(bytes);
    }
    Ok((!stored.is_empty()).then_some(stored))
}

fn base(text: &[u8]) -> io::Result<Vec<u8>> {
    let mut range = vec![BASE];
    range.extend(compress(text, LEVEL)?);
    Ok(range)
}

fn decode_base(bytes: &[u8]) -> io::Result<Vec<u8>> {
    match bytes.split_first() {
        Some((&BASE, compressed)) => decompress(compressed),
        _ => Err(invalid("Prepared catalogue has no base range")),
    }
}

fn digest(text: &[u8]) -> [u8; HASH] {
    let mut hash = [0; HASH];
    hash.copy_from_slice(&blake3::hash(text).as_bytes()[..HASH]);
    hash
}

fn length(value: u128) -> io::Result<usize> {
    usize::try_from(value).map_err(|_| invalid(UNPAIRED))
}

struct Patch {
    source: usize,
    result: usize,
    hash: [u8; HASH],
    edits: Vec<u8>,
}

impl Patch {
    fn decode(bytes: &[u8]) -> io::Result<Self> {
        let Some((&PATCH, mut rest)) = bytes.split_first() else {
            return Err(invalid("Prepared catalogue range is not a patch"));
        };
        let source = length(take(&mut rest)?)?;
        let result = length(take(&mut rest)?)?;
        if rest.len() < HASH {
            return Err(invalid(UNPAIRED));
        }
        let (hash, compressed) = rest.split_at(HASH);
        Ok(Self {
            source,
            result,
            hash: hash.try_into().expect("hash length checked above"),
            edits: decompress(compressed)?,
        })
    }
}

/// The text `patches` make from `origin`, checked against the last one's
/// length and hash.
fn rebuild<'a>(origin: &'a [u8], patches: &[Patch]) -> io::Result<Cow<'a, [u8]>> {
    let Some(last) = patches.last() else {
        return Ok(Cow::Borrowed(origin));
    };
    let mut expected = origin.len();
    let mut root = vec![Node::Raw(origin)];
    for patch in patches {
        if patch.source != expected {
            return Err(invalid(UNPAIRED));
        }
        expected = patch.result;
        let mut edits = patch.edits.as_slice();
        root = apply(root, false, &mut edits)?;
        if !edits.is_empty() || root.len() != 1 {
            return Err(invalid(UNPAIRED));
        }
    }
    let mut text = Vec::with_capacity(last.result);
    emit(&root[0], &mut text);
    if text.len() != last.result || digest(&text) != last.hash {
        return Err(invalid(
            "Prepared catalogue patches do not reproduce their text",
        ));
    }
    Ok(Cow::Owned(text))
}

/// A patch range from `from` to `to`, or None if its edits would not
/// reproduce `to` exactly (the caller then writes a base).
fn patch(from: &[u8], to: &[u8]) -> io::Result<Option<Vec<u8>>> {
    let mut ops = Vec::new();
    change(&mut ops, 0, from, (0, from.len()), to, (0, to.len()), 0)?;
    let edits = encode(&ops, to);
    let mut cursor = edits.as_slice();
    let reproduced = apply(vec![Node::Raw(from)], false, &mut cursor)
        .ok()
        .and_then(|root| {
            let [node] = root.as_slice() else {
                return None;
            };
            let mut text = Vec::with_capacity(to.len());
            emit(node, &mut text);
            Some(text)
        });
    let exact = cursor.is_empty() && reproduced.as_deref() == Some(to);
    debug_assert!(exact, "a catalogue patch does not reproduce its text");
    if !exact {
        return Ok(None);
    }
    let mut range = vec![PATCH];
    var(&mut range, from.len() as u128);
    var(&mut range, to.len() as u128);
    range.extend_from_slice(&digest(to));
    range.extend(compress(&edits, LEVEL)?);
    Ok(Some(range))
}

/// One container edit step. Spans index the new container's text.
enum Op {
    Keep {
        start: usize,
        count: usize,
    },
    Edit {
        index: usize,
        edits: Vec<u8>,
    },
    New {
        count: usize,
        start: usize,
        end: usize,
    },
}

fn keep(ops: &mut Vec<Op>, index: usize) {
    if let Some(Op::Keep { start, count }) = ops.last_mut()
        && *start + *count == index
    {
        *count += 1;
        return;
    }
    ops.push(Op::Keep {
        start: index,
        count: 1,
    });
}

/// A new child; consecutive new children share one run of text.
fn add(ops: &mut Vec<Op>, (start, end): (usize, usize)) {
    if let Some(Op::New {
        count,
        end: previous,
        ..
    }) = ops.last_mut()
        && *previous + 1 == start
    {
        *count += 1;
        *previous = end;
        return;
    }
    ops.push(Op::New {
        count: 1,
        start,
        end,
    });
}

fn encode(ops: &[Op], new: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    var(&mut bytes, ops.len() as u128);
    for op in ops {
        match op {
            Op::Keep { start, count } => {
                var(&mut bytes, KEEP);
                var(&mut bytes, *start as u128);
                var(&mut bytes, *count as u128);
            }
            Op::Edit { index, edits } => {
                var(&mut bytes, EDIT);
                var(&mut bytes, *index as u128);
                bytes.extend_from_slice(edits);
            }
            Op::New { count, start, end } => {
                var(&mut bytes, NEW);
                var(&mut bytes, *count as u128);
                var(&mut bytes, (end - start) as u128);
                bytes.extend_from_slice(&new[*start..*end]);
            }
        }
    }
    bytes
}

/// The step for earlier child `index` (span `old_span` of `old`) becoming
/// `new_span` of `new`; a member's value starts `value` bytes into its span.
fn change(
    ops: &mut Vec<Op>,
    index: usize,
    old: &[u8],
    (old_start, old_end): (usize, usize),
    new: &[u8],
    (new_start, new_end): (usize, usize),
    value: usize,
) -> io::Result<()> {
    if old[old_start..old_end] == new[new_start..new_end] {
        keep(ops, index);
        return Ok(());
    }
    let before = &old[old_start + value..old_end];
    let after = &new[new_start + value..new_end];
    if let (Some(&open), Some(&next)) = (before.first(), after.first())
        && open == next
        && matches!(open, b'{' | b'[')
    {
        let edits = edits(before, after)?;
        if edits.len() < new_end - new_start {
            ops.push(Op::Edit { index, edits });
            return Ok(());
        }
    }
    add(ops, (new_start, new_end));
    Ok(())
}

/// The encoded edits from container `old` to container `new` of the same kind.
fn edits(old: &[u8], new: &[u8]) -> io::Result<Vec<u8>> {
    let before = children(old)?;
    let after = children(new)?;
    let mut ops = Vec::new();
    if new[0] == b'{' {
        let mut members = HashMap::with_capacity(before.len());
        for (index, &(start, end)) in before.iter().enumerate() {
            members.insert(&old[start..string_end(old, start, end)?], index);
        }
        for &(start, end) in &after {
            let key = string_end(new, start, end)?;
            match members.remove(&new[start..key]) {
                Some(index) => change(
                    &mut ops,
                    index,
                    old,
                    before[index],
                    new,
                    (start, end),
                    key - start + 1,
                )?,
                None => add(&mut ops, (start, end)),
            }
        }
    } else {
        let same = |earlier: usize, later: usize| {
            old[before[earlier].0..before[earlier].1] == new[after[later].0..after[later].1]
        };
        let common = before.len().min(after.len());
        let prefix = (0..common).take_while(|&index| same(index, index)).count();
        let suffix = (0..common - prefix)
            .take_while(|&back| same(before.len() - 1 - back, after.len() - 1 - back))
            .count();
        for index in 0..after.len() - suffix {
            if index < before.len() - suffix {
                change(&mut ops, index, old, before[index], new, after[index], 0)?;
            } else {
                add(&mut ops, after[index]);
            }
        }
        for index in before.len() - suffix..before.len() {
            keep(&mut ops, index);
        }
    }
    Ok(encode(&ops, new))
}

/// A reconstruction tree: unchanged text stays a borrowed span.
enum Node<'a> {
    Raw(&'a [u8]),
    Array(Vec<Node<'a>>),
    Object(Vec<Node<'a>>),
    /// An object member whose value was edited.
    Member(&'a [u8], Box<Node<'a>>),
}

/// The children a container's encoded edits make from its `children`.
fn apply<'a>(
    children: Vec<Node<'a>>,
    object: bool,
    edits: &mut &'a [u8],
) -> io::Result<Vec<Node<'a>>> {
    let steps = length(take(edits)?)?;
    let mut earlier: Vec<Option<Node<'a>>> = children.into_iter().map(Some).collect();
    let mut result = Vec::with_capacity(earlier.len());
    for _ in 0..steps {
        match take(edits)? {
            KEEP => {
                let start = length(take(edits)?)?;
                let count = length(take(edits)?)?;
                let kept = start
                    .checked_add(count)
                    .and_then(|end| earlier.get_mut(start..end))
                    .ok_or_else(|| invalid(UNPAIRED))?;
                for child in kept {
                    result.push(child.take().ok_or_else(|| invalid(UNPAIRED))?);
                }
            }
            EDIT => {
                let index = length(take(edits)?)?;
                let child = earlier
                    .get_mut(index)
                    .and_then(Option::take)
                    .ok_or_else(|| invalid(UNPAIRED))?;
                result.push(if object {
                    let (key, value) = match child {
                        Node::Raw(member) => {
                            let end = string_end(member, 0, member.len())?;
                            if member.get(end) != Some(&b':') {
                                return Err(invalid(MALFORMED));
                            }
                            (&member[..end], Node::Raw(&member[end + 1..]))
                        }
                        Node::Member(key, value) => (key, *value),
                        Node::Array(_) | Node::Object(_) => return Err(invalid(UNPAIRED)),
                    };
                    Node::Member(key, Box::new(edit(value, edits)?))
                } else {
                    edit(child, edits)?
                });
            }
            NEW => {
                let count = length(take(edits)?)?;
                let len = length(take(edits)?)?;
                let all: &'a [u8] = edits;
                if all.len() < len {
                    return Err(invalid(UNPAIRED));
                }
                let (text, rest) = all.split_at(len);
                *edits = rest;
                let spans = run(text, 0, len, object)?;
                if spans.len() != count {
                    return Err(invalid(UNPAIRED));
                }
                result.extend(
                    spans
                        .into_iter()
                        .map(|(start, end)| Node::Raw(&text[start..end])),
                );
            }
            _ => return Err(invalid(UNPAIRED)),
        }
    }
    Ok(result)
}

fn edit<'a>(node: Node<'a>, edits: &mut &'a [u8]) -> io::Result<Node<'a>> {
    match node {
        Node::Raw(text) => {
            let object = text.first() == Some(&b'{');
            let children = children(text)?
                .into_iter()
                .map(|(start, end)| Node::Raw(&text[start..end]))
                .collect();
            let children = apply(children, object, edits)?;
            Ok(if object {
                Node::Object(children)
            } else {
                Node::Array(children)
            })
        }
        Node::Array(children) => Ok(Node::Array(apply(children, false, edits)?)),
        Node::Object(children) => Ok(Node::Object(apply(children, true, edits)?)),
        Node::Member(..) => Err(invalid(UNPAIRED)),
    }
}

fn emit(node: &Node, text: &mut Vec<u8>) {
    let (open, close, children) = match node {
        Node::Raw(raw) => {
            text.extend_from_slice(raw);
            return;
        }
        Node::Member(key, value) => {
            text.extend_from_slice(key);
            text.push(b':');
            emit(value, text);
            return;
        }
        Node::Array(children) => (b'[', b']', children),
        Node::Object(children) => (b'{', b'}', children),
    };
    text.push(open);
    for (index, child) in children.iter().enumerate() {
        if index > 0 {
            text.push(b',');
        }
        emit(child, text);
    }
    text.push(close);
}

/// The spans of a container's children within `text`: values, or members
/// (`"key":value`) of an object.
fn children(text: &[u8]) -> io::Result<Vec<(usize, usize)>> {
    let (object, close) = match text.first() {
        Some(b'{') => (true, b'}'),
        Some(b'[') => (false, b']'),
        _ => return Err(invalid(MALFORMED)),
    };
    if text.len() < 2 || text[text.len() - 1] != close {
        return Err(invalid(MALFORMED));
    }
    run(text, 1, text.len() - 1, object)
}

/// The spans of the comma-separated values, or members, in `text[from..to]`.
fn run(text: &[u8], from: usize, to: usize, object: bool) -> io::Result<Vec<(usize, usize)>> {
    let mut spans = Vec::new();
    if from == to {
        return Ok(spans);
    }
    let mut at = from;
    loop {
        let start = at;
        if object {
            at = string_end(text, at, to)?;
            if at >= to || text[at] != b':' {
                return Err(invalid(MALFORMED));
            }
            at += 1;
        }
        at = value_end(text, at, to)?;
        spans.push((start, at));
        if at == to {
            return Ok(spans);
        }
        if text[at] != b',' {
            return Err(invalid(MALFORMED));
        }
        at += 1;
    }
}

/// The end of the string starting at `text[at]`, within `text[..to]`.
fn string_end(text: &[u8], at: usize, to: usize) -> io::Result<usize> {
    if at >= to || text[at] != b'"' {
        return Err(invalid(MALFORMED));
    }
    let mut index = at + 1;
    while index < to {
        match text[index] {
            b'"' => return Ok(index + 1),
            b'\\' => index += 2,
            _ => index += 1,
        }
    }
    Err(invalid(MALFORMED))
}

/// The end of the value starting at `text[at]`, within `text[..to]`.
fn value_end(text: &[u8], at: usize, to: usize) -> io::Result<usize> {
    if at >= to {
        return Err(invalid(MALFORMED));
    }
    match text[at] {
        b'"' => string_end(text, at, to),
        b'{' | b'[' => {
            let mut depth = 0_usize;
            let mut index = at;
            while index < to {
                match text[index] {
                    b'"' => {
                        index = string_end(text, index, to)?;
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return Ok(index + 1);
                        }
                    }
                    _ => {}
                }
                index += 1;
            }
            Err(invalid(MALFORMED))
        }
        b',' | b']' | b'}' => Err(invalid(MALFORMED)),
        _ => {
            let mut index = at;
            while index < to && !matches!(text[index], b',' | b']' | b'}') {
                index += 1;
            }
            Ok(index)
        }
    }
}

#[cfg(test)]
mod tests;
