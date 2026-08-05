//! Lane CE measurement instrument for the replay checkpoint cache.
//!
//! Feature-gated (`ce-cache-anatomy`), never built by default and never
//! packaged. It exists to answer ONE question with measurements rather than
//! assumptions: the sealed cache stores 3.12 MiB of per-hour content in
//! 77.8 KiB of container bytes, a 41.1x ratio, and this module decomposes the
//! bytes that REMAIN on disk after the container's own zstd-3 pass.
//!
//! It is a child module of [`crate::replay_cache`] so it can walk the private
//! v2 grammar directly instead of re-implementing it. It never writes a cache
//! and never changes one; every function here takes an existing cache slice.
//!
//! # Why the numbers here are disk bytes, not content bytes
//!
//! Every candidate encoding is priced by rewriting the whole cache under that
//! encoding and compressing the rewritten bytes with the production container
//! profile (`pack::configured_zstd_encoder`: level 3, windowLog 27, long
//! distance matching, content checksum). A saving that zstd-3 was already
//! taking shows up here as approximately zero, which is the point.

use super::{
    ByteReader, ReplayCacheError, ReplayHourEntry, ReplayHourIndex, INLINE_UINT_BASE, PATCH_ARRAY,
    PATCH_KEEP, PATCH_OBJECT, PATCH_REPLACE, RECORD_CHECKPOINT, RECORD_DELTA, TAG_ARRAY,
    TAG_FALSE, TAG_NEGINT, TAG_NULL, TAG_NUMBER_TOKEN, TAG_OBJECT, TAG_STRING, TAG_TRUE, TAG_UINT,
};
use std::collections::HashMap;
use std::io::Write;

/// The four byte classes the v2 record grammar decomposes into.
///
/// `Struct` carries every tag byte, every collection count and every array
/// patch gap — including the inline-integer tag bytes, whose tag IS their
/// value and therefore cannot be moved to another stream without inventing a
/// second tag. [`ClassCounts::inline_uint_bytes`] reports how much of `Struct`
/// that is, so the mixture is visible rather than hidden.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Class {
    Struct,
    Key,
    Number,
    String,
}

pub const CLASS_COUNT: usize = 4;

impl Class {
    pub fn index(self) -> usize {
        match self {
            Class::Struct => 0,
            Class::Key => 1,
            Class::Number => 2,
            Class::String => 3,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Class::Struct => "struct",
            Class::Key => "key",
            Class::Number => "number",
            Class::String => "string",
        }
    }
}

#[derive(Clone, Copy, Default, Debug)]
pub struct ClassCounts {
    pub bytes: [u64; CLASS_COUNT],
    pub inline_uint_bytes: u64,
    pub key_occurrences: u64,
    pub string_occurrences: u64,
    pub number_token_occurrences: u64,
    /// Bytes spent on `Patch::Replace` subtrees, split by what the replacement
    /// IS — and by nothing else.
    ///
    /// These counters are read off the patch alone, so they record what shape
    /// was restated, NOT why it was restated. `diff_value` reaches for a
    /// replacement in three situations that all land in the same bucket: an
    /// array whose length changed, a member absent from the previous hour
    /// (an added member is emitted as a replacement whatever its type), and a
    /// value whose type changed. Telling those apart needs the previous world,
    /// which a patch does not carry.
    ///
    /// So `replace_array_bytes` measures the cost of restating whole arrays,
    /// which is the region an array-aware encoder would work in. It is not by
    /// itself a measurement of length changes.
    pub replace_array_bytes: u64,
    pub replace_array_count: u64,
    pub replace_array_elements: u64,
    pub replace_object_bytes: u64,
    pub replace_object_count: u64,
    pub replace_scalar_bytes: u64,
    pub replace_scalar_count: u64,
}

impl ClassCounts {
    pub fn total(&self) -> u64 {
        self.bytes.iter().sum()
    }
    pub fn add(&mut self, other: &ClassCounts) {
        for (slot, value) in self.bytes.iter_mut().zip(other.bytes) {
            *slot += value;
        }
        self.inline_uint_bytes += other.inline_uint_bytes;
        self.key_occurrences += other.key_occurrences;
        self.string_occurrences += other.string_occurrences;
        self.number_token_occurrences += other.number_token_occurrences;
        self.replace_array_bytes += other.replace_array_bytes;
        self.replace_array_count += other.replace_array_count;
        self.replace_array_elements += other.replace_array_elements;
        self.replace_object_bytes += other.replace_object_bytes;
        self.replace_object_count += other.replace_object_count;
        self.replace_scalar_bytes += other.replace_scalar_bytes;
        self.replace_scalar_count += other.replace_scalar_count;
    }
}

/// A sink the grammar walk hands every byte span to, tagged with its class.
///
/// The walk is total: the spans it emits concatenate back to the record
/// payload in order, byte for byte. [`ParitySink`] is the test that says so.
pub trait SpanSink {
    fn span(&mut self, class: Class, bytes: &[u8]) -> Result<(), ReplayCacheError>;
    /// Called for a token (key / string body / non-integer number token) with
    /// its length varint already emitted separately, so a dictionary sink can
    /// see the token identity.
    fn token(&mut self, _class: Class, _body: &[u8]) -> Result<(), ReplayCacheError> {
        Ok(())
    }
    /// A sink that wants to know WHERE in the world a span sits returns its
    /// path stack here. Everything else returns `None`, and because the walk
    /// is generic over the sink the bookkeeping compiles away entirely.
    fn path_stack(&mut self) -> Option<&mut PathStack> {
        None
    }
    /// A `Patch::Replace` whose replacement is a whole array, with the exact
    /// subtree bytes it occupied (tag byte first) and the element count it
    /// restated. The spans for those bytes have ALREADY been delivered, so a
    /// sink that rewrites them must roll its own output back first.
    fn replace_array(&mut self, _raw: &[u8], _elements: u64) -> Result<(), ReplayCacheError> {
        Ok(())
    }
}

/// The object member path the walk is currently inside, maintained without
/// allocating: one shared `a/b/c` buffer plus the truncation point of each
/// enclosing member.
///
/// Members deeper than `depth_limit` still push and pop, so the stack depth
/// stays correct, but contribute no bytes — the path is a grouping key, and a
/// per-unit key would make every group a singleton.
#[derive(Default)]
pub struct PathStack {
    joined: Vec<u8>,
    marks: Vec<usize>,
    pub depth_limit: usize,
}

impl PathStack {
    pub fn new(depth_limit: usize) -> Self {
        Self {
            joined: Vec::with_capacity(256),
            marks: Vec::with_capacity(64),
            depth_limit,
        }
    }
    pub fn push(&mut self, key: &[u8]) {
        self.marks.push(self.joined.len());
        if self.marks.len() <= self.depth_limit {
            if !self.joined.is_empty() {
                self.joined.push(b'/');
            }
            self.joined.extend_from_slice(key);
        }
    }
    pub fn pop(&mut self) {
        if let Some(mark) = self.marks.pop() {
            self.joined.truncate(mark);
        }
    }
    pub fn current(&self) -> &[u8] {
        if self.joined.is_empty() {
            b"<root>"
        } else {
            &self.joined
        }
    }
    pub fn push_index(&mut self, index: u64) {
        self.marks.push(self.joined.len());
        if self.marks.len() <= self.depth_limit {
            self.joined.push(b'#');
            let mut buffer = [0u8; 20];
            let mut at = buffer.len();
            let mut value = index;
            loop {
                at -= 1;
                buffer[at] = b'0' + (value % 10) as u8;
                value /= 10;
                if value == 0 {
                    break;
                }
            }
            self.joined.extend_from_slice(&buffer[at..]);
        }
    }
    pub fn reset(&mut self) {
        self.joined.clear();
        self.marks.clear();
    }
}

/// Counts bytes per class and attributes whole-array replacements by path.
pub struct PathSink {
    pub counts: ClassCounts,
    pub stack: PathStack,
    pub totals: HashMap<Vec<u8>, (u64, u64, u64)>,
}

impl PathSink {
    pub fn new(depth_limit: usize) -> Self {
        Self {
            counts: ClassCounts::default(),
            stack: PathStack::new(depth_limit),
            totals: HashMap::new(),
        }
    }
}

impl SpanSink for PathSink {
    fn span(&mut self, class: Class, bytes: &[u8]) -> Result<(), ReplayCacheError> {
        self.counts.bytes[class.index()] += bytes.len() as u64;
        Ok(())
    }
    fn path_stack(&mut self) -> Option<&mut PathStack> {
        Some(&mut self.stack)
    }
    fn replace_array(&mut self, raw: &[u8], elements: u64) -> Result<(), ReplayCacheError> {
        let entry = match self.totals.get_mut(self.stack.current()) {
            Some(entry) => entry,
            None => self
                .totals
                .entry(self.stack.current().to_vec())
                .or_insert((0, 0, 0)),
        };
        entry.0 += raw.len() as u64;
        entry.1 += 1;
        entry.2 += elements;
        Ok(())
    }
}

/// The key bytes of the length-prefixed token that starts at `start`.
fn key_bytes(bytes: &[u8], start: usize) -> Result<&[u8], ReplayCacheError> {
    let mut cursor = Cursor {
        bytes,
        position: start,
    };
    let (len, _) = cursor.varint()?;
    let len = usize::try_from(len)
        .map_err(|_| ReplayCacheError::new("ce_anatomy: key length does not fit usize"))?;
    cursor.take(len)
}

/// Counts bytes per class and nothing else.
#[derive(Default)]
pub struct CountSink {
    pub counts: ClassCounts,
}

impl SpanSink for CountSink {
    fn span(&mut self, class: Class, bytes: &[u8]) -> Result<(), ReplayCacheError> {
        self.counts.bytes[class.index()] += bytes.len() as u64;
        Ok(())
    }
    fn token(&mut self, class: Class, _body: &[u8]) -> Result<(), ReplayCacheError> {
        match class {
            Class::Key => self.counts.key_occurrences += 1,
            Class::String => self.counts.string_occurrences += 1,
            Class::Number => self.counts.number_token_occurrences += 1,
            Class::Struct => {}
        }
        Ok(())
    }
}

/// Rebuilds the payload from the spans, so the walk can be proven total.
#[derive(Default)]
pub struct ParitySink {
    pub bytes: Vec<u8>,
}

impl SpanSink for ParitySink {
    fn span(&mut self, _class: Class, bytes: &[u8]) -> Result<(), ReplayCacheError> {
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
}

/// Collects token frequencies for the global-dictionary prototypes.
#[derive(Default)]
pub struct FrequencySink {
    pub keys: HashMap<Vec<u8>, u64>,
    pub strings: HashMap<Vec<u8>, u64>,
    pub numbers: HashMap<Vec<u8>, u64>,
}

impl SpanSink for FrequencySink {
    fn span(&mut self, _class: Class, _bytes: &[u8]) -> Result<(), ReplayCacheError> {
        Ok(())
    }
    fn token(&mut self, class: Class, body: &[u8]) -> Result<(), ReplayCacheError> {
        let table = match class {
            Class::Key => &mut self.keys,
            Class::String => &mut self.strings,
            Class::Number => &mut self.numbers,
            Class::Struct => return Ok(()),
        };
        *table.entry(body.to_vec()).or_insert(0) += 1;
        Ok(())
    }
}

/// Writes each class into its own buffer (the split-stream prototype).
#[derive(Default)]
pub struct SplitSink {
    pub streams: [Vec<u8>; CLASS_COUNT],
}

impl SplitSink {
    pub fn clear(&mut self) {
        for stream in &mut self.streams {
            stream.clear();
        }
    }
}

impl SpanSink for SplitSink {
    fn span(&mut self, class: Class, bytes: &[u8]) -> Result<(), ReplayCacheError> {
        self.streams[class.index()].extend_from_slice(bytes);
        Ok(())
    }
}

/// A dictionary that maps a token body to a compact id.
///
/// Ids are assigned by descending occurrence count with the token bytes as the
/// tie-break, so the table is a deterministic function of the cache content
/// and rebuilds identically on any machine. A token that occurs once is left
/// out: it would cost a table entry AND a reference.
pub struct TokenDictionary {
    pub order: Vec<Vec<u8>>,
    lookup: HashMap<Vec<u8>, u64>,
}

impl TokenDictionary {
    pub fn build(counts: &HashMap<Vec<u8>, u64>, min_occurrences: u64) -> Self {
        let mut entries: Vec<(&Vec<u8>, u64)> = counts
            .iter()
            .filter(|(_, count)| **count >= min_occurrences)
            .map(|(body, count)| (body, *count))
            .collect();
        entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        let order: Vec<Vec<u8>> = entries.into_iter().map(|(body, _)| body.clone()).collect();
        let lookup = order
            .iter()
            .enumerate()
            .map(|(index, body)| (body.clone(), index as u64))
            .collect();
        Self { order, lookup }
    }

    pub fn id(&self, body: &[u8]) -> Option<u64> {
        self.lookup.get(body).copied()
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Serialized table: varint entry count, then each entry as varint length
    /// plus bytes, in id order.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put_varint(&mut out, self.order.len() as u64);
        for body in &self.order {
            put_varint(&mut out, body.len() as u64);
            out.extend_from_slice(body);
        }
        out
    }

    /// Rebuild a table from the bytes an artifact carries, and nothing else.
    ///
    /// A candidate's measured size includes its table, so a verification that
    /// decodes with the in-memory table proves less than it claims: the emitted
    /// table bytes could be short or wrong and the round trip would still pass.
    /// Decoding through this is what makes the table part of what is proven
    /// sufficient.
    pub fn decode(bytes: &[u8]) -> Result<Self, ReplayCacheError> {
        let mut cursor = Cursor::new(bytes);
        let (count, _) = cursor.varint()?;
        let count = usize::try_from(count)
            .map_err(|_| ReplayCacheError::new("ce_anatomy: table count does not fit usize"))?;
        let mut order = Vec::with_capacity(count.min(4096));
        for _ in 0..count {
            let (len, _) = cursor.varint()?;
            let len = usize::try_from(len).map_err(|_| {
                ReplayCacheError::new("ce_anatomy: table entry length does not fit usize")
            })?;
            order.push(cursor.take(len)?.to_vec());
        }
        if !cursor.finished() {
            return Err(ReplayCacheError::new(
                "ce_anatomy: dictionary table has trailing bytes",
            ));
        }
        let lookup = order
            .iter()
            .enumerate()
            .map(|(index, body)| (body.clone(), index as u64))
            .collect();
        Ok(Self { order, lookup })
    }
}

/// Substitutes dictionary ids for token bodies, optionally into split streams.
///
/// A reference is `varint(id + 1)`; `varint(0)` means the token is not in the
/// table and its length-prefixed body follows inline. That keeps every token
/// expressible, so the encoding is total rather than conditional on a
/// dictionary hit.
pub struct DictionarySink<'a> {
    pub dictionaries: [Option<&'a TokenDictionary>; CLASS_COUNT],
    pub split: bool,
    pub streams: [Vec<u8>; CLASS_COUNT],
}

impl<'a> DictionarySink<'a> {
    pub fn new(dictionaries: [Option<&'a TokenDictionary>; CLASS_COUNT], split: bool) -> Self {
        Self {
            dictionaries,
            split,
            streams: Default::default(),
        }
    }

    pub fn clear(&mut self) {
        for stream in &mut self.streams {
            stream.clear();
        }
    }

    fn stream(&mut self, class: Class) -> &mut Vec<u8> {
        if self.split {
            &mut self.streams[class.index()]
        } else {
            &mut self.streams[0]
        }
    }

    /// The record payload under this encoding.
    pub fn payload(&self) -> Vec<u8> {
        if !self.split {
            return self.streams[0].clone();
        }
        let mut out = Vec::new();
        for stream in &self.streams {
            put_varint(&mut out, stream.len() as u64);
        }
        for stream in &self.streams {
            out.extend_from_slice(stream);
        }
        out
    }
}

impl SpanSink for DictionarySink<'_> {
    fn span(&mut self, class: Class, bytes: &[u8]) -> Result<(), ReplayCacheError> {
        self.stream(class).extend_from_slice(bytes);
        Ok(())
    }

    fn token(&mut self, class: Class, body: &[u8]) -> Result<(), ReplayCacheError> {
        // `span` has already appended `varint(len) || body` for this token.
        // Roll that back and re-emit as a reference when the table has it.
        let dictionary = match self.dictionaries[class.index()] {
            Some(dictionary) => dictionary,
            None => return Ok(()),
        };
        let length_bytes = varint_len(body.len() as u64);
        let emitted = length_bytes + body.len();
        let stream = if self.split {
            &mut self.streams[class.index()]
        } else {
            &mut self.streams[0]
        };
        debug_assert!(stream.len() >= emitted);
        stream.truncate(stream.len() - emitted);
        match dictionary.id(body) {
            Some(id) => put_varint(stream, id + 1),
            None => {
                put_varint(stream, 0);
                put_varint(stream, body.len() as u64);
                stream.extend_from_slice(body);
            }
        }
        Ok(())
    }
}

fn varint_len(mut value: u64) -> usize {
    let mut len = 1;
    while value >= 0x80 {
        value >>= 7;
        len += 1;
    }
    len
}

fn put_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// A cursor that hands back the exact byte span it consumed.
struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], ReplayCacheError> {
        let end = self
            .position
            .checked_add(len)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| ReplayCacheError::new("ce_anatomy: record payload is truncated"))?;
        let span = &self.bytes[self.position..end];
        self.position = end;
        Ok(span)
    }

    fn byte(&mut self) -> Result<(u8, &'a [u8]), ReplayCacheError> {
        let span = self.take(1)?;
        Ok((span[0], span))
    }

    /// Minimal base-128 varint, returning the value and the bytes it occupied.
    fn varint(&mut self) -> Result<(u64, &'a [u8]), ReplayCacheError> {
        let start = self.position;
        let mut value = 0u64;
        let mut shift = 0u32;
        loop {
            let byte = self.take(1)?[0];
            let payload = u64::from(byte & 0x7f);
            if shift >= 64 || (shift == 63 && payload > 1) {
                return Err(ReplayCacheError::new("ce_anatomy: varint overflows u64"));
            }
            value |= payload << shift;
            if byte & 0x80 == 0 {
                if byte == 0 && shift != 0 {
                    return Err(ReplayCacheError::new("ce_anatomy: varint is not minimal"));
                }
                return Ok((value, &self.bytes[start..self.position]));
            }
            shift += 7;
        }
    }

    fn finished(&self) -> bool {
        self.position == self.bytes.len()
    }
}

const MAX_WALK_DEPTH: usize = 512;

fn walk_text<S: SpanSink>(
    cursor: &mut Cursor<'_>,
    sink: &mut S,
    class: Class,
) -> Result<(), ReplayCacheError> {
    let (len, span) = cursor.varint()?;
    sink.span(class, span)?;
    let len = usize::try_from(len)
        .map_err(|_| ReplayCacheError::new("ce_anatomy: token length does not fit usize"))?;
    let body = cursor.take(len)?;
    sink.span(class, body)?;
    sink.token(class, body)?;
    Ok(())
}

fn walk_value<S: SpanSink>(
    cursor: &mut Cursor<'_>,
    sink: &mut S,
    depth: usize,
    counts: &mut ClassCounts,
) -> Result<(), ReplayCacheError> {
    if depth > MAX_WALK_DEPTH {
        return Err(ReplayCacheError::new("ce_anatomy: value exceeds depth bound"));
    }
    let (tag, span) = cursor.byte()?;
    sink.span(Class::Struct, span)?;
    if tag >= INLINE_UINT_BASE {
        counts.inline_uint_bytes += 1;
        return Ok(());
    }
    match tag {
        TAG_NULL | TAG_FALSE | TAG_TRUE => Ok(()),
        TAG_NUMBER_TOKEN => walk_text(cursor, sink, Class::Number),
        TAG_STRING => walk_text(cursor, sink, Class::String),
        TAG_UINT | TAG_NEGINT => {
            let (_, span) = cursor.varint()?;
            sink.span(Class::Number, span)
        }
        TAG_ARRAY => {
            let (count, span) = cursor.varint()?;
            sink.span(Class::Struct, span)?;
            for _ in 0..count {
                walk_value(cursor, sink, depth + 1, counts)?;
            }
            Ok(())
        }
        TAG_OBJECT => {
            let (count, span) = cursor.varint()?;
            sink.span(Class::Struct, span)?;
            for _ in 0..count {
                walk_text(cursor, sink, Class::Key)?;
                walk_value(cursor, sink, depth + 1, counts)?;
            }
            Ok(())
        }
        other => Err(ReplayCacheError::new(format!(
            "ce_anatomy: unknown value tag {other}"
        ))),
    }
}

fn walk_patch<S: SpanSink>(
    cursor: &mut Cursor<'_>,
    sink: &mut S,
    depth: usize,
    counts: &mut ClassCounts,
) -> Result<(), ReplayCacheError> {
    if depth > MAX_WALK_DEPTH {
        return Err(ReplayCacheError::new("ce_anatomy: patch exceeds depth bound"));
    }
    let (tag, span) = cursor.byte()?;
    sink.span(Class::Struct, span)?;
    match tag {
        PATCH_KEEP => Ok(()),
        PATCH_REPLACE => {
            // Classify by what the replacement is, and charge it the bytes it
            // actually occupies, before the walk consumes them.
            let start = cursor.position;
            let replacement_tag = cursor
                .bytes
                .get(start)
                .copied()
                .ok_or_else(|| ReplayCacheError::new("ce_anatomy: replacement value is missing"))?;
            let elements = if replacement_tag == TAG_ARRAY {
                let mut peek = Cursor {
                    bytes: cursor.bytes,
                    position: start + 1,
                };
                peek.varint()?.0
            } else {
                0
            };
            walk_value(cursor, sink, depth + 1, counts)?;
            let spent = (cursor.position - start) as u64;
            match replacement_tag {
                TAG_ARRAY => {
                    counts.replace_array_bytes += spent;
                    counts.replace_array_count += 1;
                    counts.replace_array_elements += elements;
                }
                TAG_OBJECT => {
                    counts.replace_object_bytes += spent;
                    counts.replace_object_count += 1;
                }
                _ => {
                    counts.replace_scalar_bytes += spent;
                    counts.replace_scalar_count += 1;
                }
            }
            if replacement_tag == TAG_ARRAY {
                sink.replace_array(&cursor.bytes[start..cursor.position], elements)?;
            }
            Ok(())
        }
        PATCH_OBJECT => {
            let (removals, span) = cursor.varint()?;
            sink.span(Class::Struct, span)?;
            for _ in 0..removals {
                walk_text(cursor, sink, Class::Key)?;
            }
            let (changes, span) = cursor.varint()?;
            sink.span(Class::Struct, span)?;
            for _ in 0..changes {
                let key_start = cursor.position;
                walk_text(cursor, sink, Class::Key)?;
                let key = key_bytes(cursor.bytes, key_start)?;
                if let Some(stack) = sink.path_stack() {
                    stack.push(key);
                }
                walk_patch(cursor, sink, depth + 1, counts)?;
                if let Some(stack) = sink.path_stack() {
                    stack.pop();
                }
            }
            Ok(())
        }
        PATCH_ARRAY => {
            let (changes, span) = cursor.varint()?;
            sink.span(Class::Struct, span)?;
            let mut previous: Option<u64> = None;
            for _ in 0..changes {
                let (gap, span) = cursor.varint()?;
                sink.span(Class::Struct, span)?;
                // The element index is part of WHERE this patch sits. Without
                // it every element of an array collapses onto one path, and a
                // per-location experiment silently compares unrelated slots.
                let element = match previous {
                    None => gap,
                    Some(previous) => previous + 1 + gap,
                };
                previous = Some(element);
                if let Some(stack) = sink.path_stack() {
                    stack.push_index(element);
                }
                walk_patch(cursor, sink, depth + 1, counts)?;
                if let Some(stack) = sink.path_stack() {
                    stack.pop();
                }
            }
            Ok(())
        }
        other => Err(ReplayCacheError::new(format!(
            "ce_anatomy: unknown patch tag {other}"
        ))),
    }
}

/// Walk one record payload, emitting every byte exactly once.
pub fn walk_record<S: SpanSink>(
    kind: u8,
    payload: &[u8],
    sink: &mut S,
) -> Result<ClassCounts, ReplayCacheError> {
    let mut counts = ClassCounts::default();
    let mut cursor = Cursor::new(payload);
    match kind {
        RECORD_CHECKPOINT => walk_value(&mut cursor, sink, 0, &mut counts)?,
        RECORD_DELTA => walk_patch(&mut cursor, sink, 0, &mut counts)?,
        other => {
            return Err(ReplayCacheError::new(format!(
                "ce_anatomy: unknown record kind {other}"
            )))
        }
    }
    if !cursor.finished() {
        return Err(ReplayCacheError::new(
            "ce_anatomy: record payload has trailing bytes",
        ));
    }
    Ok(counts)
}

/// The record header + payload view the walk needs, taken from the index.
pub struct CacheRecord<'a> {
    pub kind: u8,
    pub hour_index: i64,
    pub header: &'a [u8],
    pub payload: &'a [u8],
}

pub fn record<'a>(
    cache: &'a [u8],
    entry: &ReplayHourEntry,
) -> Result<CacheRecord<'a>, ReplayCacheError> {
    let start = usize::try_from(entry.cache_record_offset())
        .map_err(|_| ReplayCacheError::new("ce_anatomy: record offset does not fit usize"))?;
    let len = usize::try_from(entry.cache_record_len())
        .map_err(|_| ReplayCacheError::new("ce_anatomy: record length does not fit usize"))?;
    let end = start
        .checked_add(len)
        .filter(|end| *end <= cache.len())
        .ok_or_else(|| ReplayCacheError::new("ce_anatomy: record range is out of bounds"))?;
    let bytes = &cache[start..end];
    let mut reader = ByteReader::new(bytes);
    let kind = reader.u8()?;
    reader.take(3)?;
    let hour_index = reader.i64()?;
    let payload_len = usize::try_from(reader.u64()?)
        .map_err(|_| ReplayCacheError::new("ce_anatomy: payload length does not fit usize"))?;
    reader.u8()?;
    let payload = reader.take(payload_len)?;
    Ok(CacheRecord {
        kind,
        hour_index,
        header: &bytes[..21],
        payload,
    })
}

pub fn records<'a>(
    cache: &'a [u8],
    index: &'a ReplayHourIndex,
) -> impl Iterator<Item = Result<CacheRecord<'a>, ReplayCacheError>> + 'a {
    index.hours().iter().map(move |entry| record(cache, entry))
}

/// Reverse of [`DictionarySink`]: rebuild the original v2 record payload from
/// the candidate encoding.
///
/// This exists so a candidate's measured size is a size for the SAME
/// information. An encoding that shrinks the cache by losing a byte is not a
/// smaller cache, and only a decoder that returns the original payload proves
/// it did not.
pub fn decode_dictionary_payload(
    kind: u8,
    encoded: &[u8],
    dictionaries: [Option<&TokenDictionary>; CLASS_COUNT],
    split: bool,
) -> Result<Vec<u8>, ReplayCacheError> {
    let mut streams: Vec<Cursor<'_>> = Vec::with_capacity(CLASS_COUNT);
    if split {
        let mut header = Cursor::new(encoded);
        let mut lengths = [0usize; CLASS_COUNT];
        for slot in &mut lengths {
            let (value, _) = header.varint()?;
            *slot = usize::try_from(value).map_err(|_| {
                ReplayCacheError::new("ce_anatomy: split stream length does not fit usize")
            })?;
        }
        let mut at = header.position;
        for len in lengths {
            let end = at
                .checked_add(len)
                .filter(|end| *end <= encoded.len())
                .ok_or_else(|| ReplayCacheError::new("ce_anatomy: split stream is truncated"))?;
            streams.push(Cursor::new(&encoded[at..end]));
            at = end;
        }
        if at != encoded.len() {
            return Err(ReplayCacheError::new(
                "ce_anatomy: split payload has trailing bytes",
            ));
        }
    } else {
        for _ in 0..CLASS_COUNT {
            streams.push(Cursor::new(encoded));
        }
    }
    let mut out = Vec::new();
    let mut state = DecodeState {
        streams,
        split,
        dictionaries,
        out: &mut out,
    };
    match kind {
        RECORD_CHECKPOINT => state.value(0)?,
        RECORD_DELTA => state.patch(0)?,
        other => {
            return Err(ReplayCacheError::new(format!(
                "ce_anatomy: unknown record kind {other}"
            )))
        }
    }
    state.finish()?;
    Ok(out)
}

struct DecodeState<'a, 'b> {
    streams: Vec<Cursor<'a>>,
    split: bool,
    dictionaries: [Option<&'b TokenDictionary>; CLASS_COUNT],
    out: &'b mut Vec<u8>,
}

impl DecodeState<'_, '_> {
    fn stream_index(&self, class: Class) -> usize {
        if self.split {
            class.index()
        } else {
            0
        }
    }

    fn raw(&mut self, class: Class, len: usize) -> Result<(), ReplayCacheError> {
        let index = self.stream_index(class);
        let span = self.streams[index].take(len)?;
        self.out.extend_from_slice(span);
        Ok(())
    }

    fn varint(&mut self, class: Class, copy: bool) -> Result<u64, ReplayCacheError> {
        let index = self.stream_index(class);
        let (value, span) = self.streams[index].varint()?;
        if copy {
            self.out.extend_from_slice(span);
        }
        Ok(value)
    }

    fn byte(&mut self, class: Class) -> Result<u8, ReplayCacheError> {
        let index = self.stream_index(class);
        let (value, span) = self.streams[index].byte()?;
        self.out.extend_from_slice(span);
        Ok(value)
    }

    fn text(&mut self, class: Class) -> Result<(), ReplayCacheError> {
        match self.dictionaries[class.index()] {
            None => {
                let len = self.varint(class, true)?;
                let len = usize::try_from(len).map_err(|_| {
                    ReplayCacheError::new("ce_anatomy: token length does not fit usize")
                })?;
                self.raw(class, len)
            }
            Some(dictionary) => {
                let reference = self.varint(class, false)?;
                if reference == 0 {
                    let len = self.varint(class, true)?;
                    let len = usize::try_from(len).map_err(|_| {
                        ReplayCacheError::new("ce_anatomy: token length does not fit usize")
                    })?;
                    self.raw(class, len)
                } else {
                    let body = dictionary
                        .order
                        .get((reference - 1) as usize)
                        .ok_or_else(|| {
                            ReplayCacheError::new("ce_anatomy: dictionary reference is out of range")
                        })?;
                    put_varint(self.out, body.len() as u64);
                    self.out.extend_from_slice(body);
                    Ok(())
                }
            }
        }
    }

    fn value(&mut self, depth: usize) -> Result<(), ReplayCacheError> {
        if depth > MAX_WALK_DEPTH {
            return Err(ReplayCacheError::new("ce_anatomy: value exceeds depth bound"));
        }
        let tag = self.byte(Class::Struct)?;
        if tag >= INLINE_UINT_BASE {
            return Ok(());
        }
        match tag {
            TAG_NULL | TAG_FALSE | TAG_TRUE => Ok(()),
            TAG_NUMBER_TOKEN => self.text(Class::Number),
            TAG_STRING => self.text(Class::String),
            TAG_UINT | TAG_NEGINT => {
                self.varint(Class::Number, true)?;
                Ok(())
            }
            TAG_ARRAY => {
                let count = self.varint(Class::Struct, true)?;
                for _ in 0..count {
                    self.value(depth + 1)?;
                }
                Ok(())
            }
            TAG_OBJECT => {
                let count = self.varint(Class::Struct, true)?;
                for _ in 0..count {
                    self.text(Class::Key)?;
                    self.value(depth + 1)?;
                }
                Ok(())
            }
            other => Err(ReplayCacheError::new(format!(
                "ce_anatomy: unknown value tag {other}"
            ))),
        }
    }

    fn patch(&mut self, depth: usize) -> Result<(), ReplayCacheError> {
        if depth > MAX_WALK_DEPTH {
            return Err(ReplayCacheError::new("ce_anatomy: patch exceeds depth bound"));
        }
        let tag = self.byte(Class::Struct)?;
        match tag {
            PATCH_KEEP => Ok(()),
            PATCH_REPLACE => self.value(depth + 1),
            PATCH_OBJECT => {
                let removals = self.varint(Class::Struct, true)?;
                for _ in 0..removals {
                    self.text(Class::Key)?;
                }
                let changes = self.varint(Class::Struct, true)?;
                for _ in 0..changes {
                    self.text(Class::Key)?;
                    self.patch(depth + 1)?;
                }
                Ok(())
            }
            PATCH_ARRAY => {
                let changes = self.varint(Class::Struct, true)?;
                for _ in 0..changes {
                    self.varint(Class::Struct, true)?;
                    self.patch(depth + 1)?;
                }
                Ok(())
            }
            other => Err(ReplayCacheError::new(format!(
                "ce_anatomy: unknown patch tag {other}"
            ))),
        }
    }

    fn finish(&self) -> Result<(), ReplayCacheError> {
        let streams: &[Cursor<'_>] = if self.split {
            &self.streams
        } else {
            &self.streams[..1]
        };
        for stream in streams {
            if !stream.finished() {
                return Err(ReplayCacheError::new(
                    "ce_anatomy: candidate payload has trailing bytes",
                ));
            }
        }
        Ok(())
    }
}

/// A `Write` that discards bytes and counts them.
pub struct CompressedSize {
    count: u64,
}

impl Write for CompressedSize {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.count += buf.len() as u64;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Compress `feed`'s output with the PRODUCTION container encoder and return
/// the zstd frame length in bytes.
///
/// `window_log` is the only knob a caller may move, and moving it is a
/// diagnostic, not a proposal: the container's reader contract caps it at
/// [`crate::zip::ZSTD_WINDOW_LOG`], which `CompressionProfile::validate`
/// enforces on every shippable call.
///
/// `pledged` mirrors the container's own `set_pledged_src_size`. Production
/// always knows the entry length before it streams, so leaving it out here
/// would make every number in this instrument differ from the shipped entry by
/// a frame-header field.
pub fn compressed_len<F>(window_log: u32, pledged: Option<u64>, feed: F) -> std::io::Result<u64>
where
    F: FnOnce(&mut dyn Write) -> std::io::Result<()>,
{
    let profile = crate::pack::CompressionProfile {
        zstd_level: crate::pack::DEFAULT_ZSTD_COMPRESSION_LEVEL,
        zstd_window_log: window_log,
    };
    let sink = CompressedSize { count: 0 };
    let mut encoder = crate::pack::ce_production_zstd_encoder(sink, profile)?;
    encoder.set_pledged_src_size(pledged)?;
    feed(&mut encoder)?;
    let sink = encoder.finish()?;
    Ok(sink.count)
}

/// Rewrites each whole-array restatement as a byte delta against the array
/// last restated at the SAME world location.
///
/// `diff_value` restates an array in full whenever its length changed, and the
/// paths report shows that is where most delta content goes. This prices the
/// obvious repair — an array diff — without inventing an element alignment:
/// it shares the longest common prefix and suffix of the two encodings, which
/// is exactly what a positional element diff recovers for an append, a
/// truncation, or a tail edit, and nothing more. It therefore reads as a LOWER
/// bound on an array-aware encoder, not an estimate of one: a keyed or
/// LCS alignment would recover mid-array edits this does not.
///
/// The location key carries array element indices, so two different divisions'
/// equipment lists are never compared with each other.
pub struct ArrayDeltaSink {
    pub stack: PathStack,
    pub last: HashMap<Vec<u8>, Vec<u8>>,
    pub out: Vec<u8>,
    pub rewritten: u64,
    pub first_sighting: u64,
    pub raw_array_bytes: u64,
    pub delta_array_bytes: u64,
    pub shared_bytes: u64,
}

impl ArrayDeltaSink {
    pub fn new() -> Self {
        Self {
            stack: PathStack::new(usize::MAX),
            last: HashMap::new(),
            out: Vec::new(),
            rewritten: 0,
            first_sighting: 0,
            raw_array_bytes: 0,
            delta_array_bytes: 0,
            shared_bytes: 0,
        }
    }

    pub fn begin_record(&mut self) {
        self.out.clear();
        self.stack.reset();
    }
}

impl Default for ArrayDeltaSink {
    fn default() -> Self {
        Self::new()
    }
}

/// Longest common prefix and then longest common suffix of the REMAINDERS, so
/// the two counts never overlap and the split is a function of the inputs.
fn shared_edges(previous: &[u8], current: &[u8]) -> (usize, usize) {
    let limit = previous.len().min(current.len());
    let mut prefix = 0;
    while prefix < limit && previous[prefix] == current[prefix] {
        prefix += 1;
    }
    let remaining = limit - prefix;
    let mut suffix = 0;
    while suffix < remaining
        && previous[previous.len() - 1 - suffix] == current[current.len() - 1 - suffix]
    {
        suffix += 1;
    }
    (prefix, suffix)
}

impl SpanSink for ArrayDeltaSink {
    fn span(&mut self, _class: Class, bytes: &[u8]) -> Result<(), ReplayCacheError> {
        self.out.extend_from_slice(bytes);
        Ok(())
    }
    fn path_stack(&mut self) -> Option<&mut PathStack> {
        Some(&mut self.stack)
    }
    fn replace_array(&mut self, raw: &[u8], _elements: u64) -> Result<(), ReplayCacheError> {
        // The spans for the whole subtree are already in `out`. Keep the
        // TAG_ARRAY byte — it is what tells a decoder this form follows — and
        // replace the body.
        let body = &raw[1..];
        self.out.truncate(self.out.len() - body.len());
        self.raw_array_bytes += raw.len() as u64;
        let before = self.out.len();
        let key = self.stack.current();
        match self.last.get(key) {
            Some(previous) => {
                let (prefix, suffix) = shared_edges(previous, body);
                put_varint(&mut self.out, prefix as u64);
                put_varint(&mut self.out, suffix as u64);
                let middle = &body[prefix..body.len() - suffix];
                put_varint(&mut self.out, middle.len() as u64);
                self.out.extend_from_slice(middle);
                self.rewritten += 1;
                self.shared_bytes += (prefix + suffix) as u64;
            }
            None => {
                put_varint(&mut self.out, 0);
                put_varint(&mut self.out, 0);
                put_varint(&mut self.out, body.len() as u64);
                self.out.extend_from_slice(body);
                self.first_sighting += 1;
            }
        }
        self.delta_array_bytes += (self.out.len() - before) as u64 + 1;
        let key = self.stack.current().to_vec();
        match self.last.get_mut(&key) {
            Some(slot) => {
                slot.clear();
                slot.extend_from_slice(body);
            }
            None => {
                self.last.insert(key, body.to_vec());
            }
        }
        Ok(())
    }
}

/// Reverse of [`ArrayDeltaSink`], carrying the same per-location state.
pub struct ArrayDeltaDecoder {
    pub stack: PathStack,
    pub last: HashMap<Vec<u8>, Vec<u8>>,
    scratch: Vec<u8>,
}

impl ArrayDeltaDecoder {
    pub fn new() -> Self {
        Self {
            stack: PathStack::new(usize::MAX),
            last: HashMap::new(),
            scratch: Vec::new(),
        }
    }

    pub fn decode_record(&mut self, kind: u8, encoded: &[u8]) -> Result<Vec<u8>, ReplayCacheError> {
        self.stack.reset();
        let mut cursor = Cursor::new(encoded);
        let mut out = Vec::with_capacity(encoded.len());
        match kind {
            RECORD_CHECKPOINT => self.copy_value(&mut cursor, &mut out, 0)?,
            RECORD_DELTA => self.patch(&mut cursor, &mut out, 0)?,
            other => {
                return Err(ReplayCacheError::new(format!(
                    "ce_anatomy: unknown record kind {other}"
                )))
            }
        }
        if !cursor.finished() {
            return Err(ReplayCacheError::new(
                "ce_anatomy: array-delta payload has trailing bytes",
            ));
        }
        Ok(out)
    }

    fn copy_text(
        &mut self,
        cursor: &mut Cursor<'_>,
        out: &mut Vec<u8>,
    ) -> Result<Vec<u8>, ReplayCacheError> {
        let (len, span) = cursor.varint()?;
        out.extend_from_slice(span);
        let len = usize::try_from(len)
            .map_err(|_| ReplayCacheError::new("ce_anatomy: token length does not fit usize"))?;
        let body = cursor.take(len)?;
        out.extend_from_slice(body);
        Ok(body.to_vec())
    }

    fn copy_value(
        &mut self,
        cursor: &mut Cursor<'_>,
        out: &mut Vec<u8>,
        depth: usize,
    ) -> Result<(), ReplayCacheError> {
        if depth > MAX_WALK_DEPTH {
            return Err(ReplayCacheError::new("ce_anatomy: value exceeds depth bound"));
        }
        let (tag, span) = cursor.byte()?;
        out.extend_from_slice(span);
        if tag >= INLINE_UINT_BASE {
            return Ok(());
        }
        match tag {
            TAG_NULL | TAG_FALSE | TAG_TRUE => Ok(()),
            TAG_NUMBER_TOKEN | TAG_STRING => {
                self.copy_text(cursor, out)?;
                Ok(())
            }
            TAG_UINT | TAG_NEGINT => {
                let (_, span) = cursor.varint()?;
                out.extend_from_slice(span);
                Ok(())
            }
            TAG_ARRAY => {
                let (count, span) = cursor.varint()?;
                out.extend_from_slice(span);
                for _ in 0..count {
                    self.copy_value(cursor, out, depth + 1)?;
                }
                Ok(())
            }
            TAG_OBJECT => {
                let (count, span) = cursor.varint()?;
                out.extend_from_slice(span);
                for _ in 0..count {
                    self.copy_text(cursor, out)?;
                    self.copy_value(cursor, out, depth + 1)?;
                }
                Ok(())
            }
            other => Err(ReplayCacheError::new(format!(
                "ce_anatomy: unknown value tag {other}"
            ))),
        }
    }

    fn patch(
        &mut self,
        cursor: &mut Cursor<'_>,
        out: &mut Vec<u8>,
        depth: usize,
    ) -> Result<(), ReplayCacheError> {
        if depth > MAX_WALK_DEPTH {
            return Err(ReplayCacheError::new("ce_anatomy: patch exceeds depth bound"));
        }
        let (tag, span) = cursor.byte()?;
        out.extend_from_slice(span);
        match tag {
            PATCH_KEEP => Ok(()),
            PATCH_REPLACE => {
                let (value_tag, span) = cursor.byte()?;
                out.extend_from_slice(span);
                if value_tag != TAG_ARRAY {
                    return self.copy_value_after_tag(cursor, out, value_tag, depth + 1);
                }
                let (prefix, _) = cursor.varint()?;
                let (suffix, _) = cursor.varint()?;
                let (middle_len, _) = cursor.varint()?;
                let middle = cursor.take(usize::try_from(middle_len).map_err(|_| {
                    ReplayCacheError::new("ce_anatomy: array delta middle does not fit usize")
                })?)?;
                let key = self.stack.current().to_vec();
                self.scratch.clear();
                match self.last.get(&key) {
                    Some(previous) => {
                        let prefix = usize::try_from(prefix).map_err(|_| {
                            ReplayCacheError::new("ce_anatomy: array delta prefix does not fit")
                        })?;
                        let suffix = usize::try_from(suffix).map_err(|_| {
                            ReplayCacheError::new("ce_anatomy: array delta suffix does not fit")
                        })?;
                        if prefix + suffix > previous.len() {
                            return Err(ReplayCacheError::new(
                                "ce_anatomy: array delta edges exceed the previous encoding",
                            ));
                        }
                        self.scratch.extend_from_slice(&previous[..prefix]);
                        self.scratch.extend_from_slice(middle);
                        self.scratch
                            .extend_from_slice(&previous[previous.len() - suffix..]);
                    }
                    None => {
                        if prefix != 0 || suffix != 0 {
                            return Err(ReplayCacheError::new(
                                "ce_anatomy: array delta references an unseen location",
                            ));
                        }
                        self.scratch.extend_from_slice(middle);
                    }
                }
                out.extend_from_slice(&self.scratch);
                let body = std::mem::take(&mut self.scratch);
                self.last.insert(key, body);
                Ok(())
            }
            PATCH_OBJECT => {
                let (removals, span) = cursor.varint()?;
                out.extend_from_slice(span);
                for _ in 0..removals {
                    self.copy_text(cursor, out)?;
                }
                let (changes, span) = cursor.varint()?;
                out.extend_from_slice(span);
                for _ in 0..changes {
                    let key = self.copy_text(cursor, out)?;
                    self.stack.push(&key);
                    self.patch(cursor, out, depth + 1)?;
                    self.stack.pop();
                }
                Ok(())
            }
            PATCH_ARRAY => {
                let (changes, span) = cursor.varint()?;
                out.extend_from_slice(span);
                let mut previous: Option<u64> = None;
                for _ in 0..changes {
                    let (gap, span) = cursor.varint()?;
                    out.extend_from_slice(span);
                    let element = match previous {
                        None => gap,
                        Some(previous) => previous + 1 + gap,
                    };
                    previous = Some(element);
                    self.stack.push_index(element);
                    self.patch(cursor, out, depth + 1)?;
                    self.stack.pop();
                }
                Ok(())
            }
            other => Err(ReplayCacheError::new(format!(
                "ce_anatomy: unknown patch tag {other}"
            ))),
        }
    }

    fn copy_value_after_tag(
        &mut self,
        cursor: &mut Cursor<'_>,
        out: &mut Vec<u8>,
        tag: u8,
        depth: usize,
    ) -> Result<(), ReplayCacheError> {
        if tag >= INLINE_UINT_BASE {
            return Ok(());
        }
        match tag {
            TAG_NULL | TAG_FALSE | TAG_TRUE => Ok(()),
            TAG_NUMBER_TOKEN | TAG_STRING => {
                self.copy_text(cursor, out)?;
                Ok(())
            }
            TAG_UINT | TAG_NEGINT => {
                let (_, span) = cursor.varint()?;
                out.extend_from_slice(span);
                Ok(())
            }
            TAG_OBJECT => {
                let (count, span) = cursor.varint()?;
                out.extend_from_slice(span);
                for _ in 0..count {
                    self.copy_text(cursor, out)?;
                    self.copy_value(cursor, out, depth + 1)?;
                }
                Ok(())
            }
            other => Err(ReplayCacheError::new(format!(
                "ce_anatomy: unexpected replacement value tag {other}"
            ))),
        }
    }
}

impl Default for ArrayDeltaDecoder {
    fn default() -> Self {
        Self::new()
    }
}

/// DIAGNOSTIC ONLY: the same level-3 LDM encoder at a window the container's
/// reader contract does not permit.
///
/// It answers "how much of the cache's remaining redundancy is out of the
/// shipped 128 MiB window's reach", which bounds what any cross-record
/// regrouping could be worth. Nothing it measures is shippable:
/// `CompressionProfile::validate` rejects a window above
/// [`crate::zip::ZSTD_WINDOW_LOG`], so this path deliberately does not go
/// through the production encoder.
pub fn compressed_len_oversized_window<F>(
    window_log: u32,
    pledged: Option<u64>,
    feed: F,
) -> std::io::Result<u64>
where
    F: FnOnce(&mut dyn Write) -> std::io::Result<()>,
{
    let sink = CompressedSize { count: 0 };
    let mut encoder =
        zstd::stream::write::Encoder::new(sink, crate::pack::DEFAULT_ZSTD_COMPRESSION_LEVEL)?;
    encoder.long_distance_matching(true)?;
    encoder.window_log(window_log)?;
    encoder.include_checksum(true)?;
    encoder.set_pledged_src_size(pledged)?;
    feed(&mut encoder)?;
    let sink = encoder.finish()?;
    Ok(sink.count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replay_cache::{CacheBuildHour, ReplayCacheEncoder};

    fn world(hour: i64, body: &str) -> Vec<u8> {
        format!(
            r#"{{"date":"1936.1.{}","game_id":"g","hour_index":{hour},"schema_version":"1","world":{body}}}"#,
            hour + 1
        )
        .into_bytes()
    }

    fn build(bodies: &[&str], interval: u32) -> (Vec<u8>, ReplayHourIndex) {
        let source = b"H4CRfixture-source-bytes";
        let source_sha = crate::sha256::Sha256::hex(source);
        let mut encoder = ReplayCacheEncoder::new(Vec::new(), source_sha, interval).unwrap();
        for (position, body) in bodies.iter().enumerate() {
            let hour = position as i64;
            encoder
                .push(CacheBuildHour {
                    hour_index: hour,
                    date: format!("1936.1.{}", hour + 1),
                    h4cr_start_offset: position as u64 * 2,
                    h4cr_end_offset: position as u64 * 2 + 2,
                    world_json: world(hour, body),
                })
                .unwrap();
        }
        let (cache, index) = encoder
            .finish(
                "g",
                "1",
                crate::replay_cache::CacheIdentityMode::IdentityV1,
                bodies.len() as u64 * 2,
            )
            .unwrap();
        (cache, index)
    }

    /// The walk must be TOTAL: every payload byte is emitted exactly once, in
    /// order. Without this the class table is an opinion, because bytes the
    /// walk silently skipped would be attributed to nothing.
    #[test]
    fn span_walk_reproduces_every_record_payload() {
        let (cache, index) = build(
            &[
                r#"{"a":[1,2,3],"b":{"x":"one","y":1.5},"c":null}"#,
                r#"{"a":[1,9,3],"b":{"x":"two","y":1.5,"z":true},"c":null}"#,
                r#"{"a":[1,9,4],"b":{"y":-17},"c":[{"deep":[0.25,1e9]}]}"#,
            ],
            8,
        );
        let mut seen = 0;
        for record in records(&cache, &index) {
            let record = record.unwrap();
            let mut sink = ParitySink::default();
            walk_record(record.kind, record.payload, &mut sink).unwrap();
            assert_eq!(
                sink.bytes, record.payload,
                "walk did not reproduce hour {} payload",
                record.hour_index
            );
            seen += 1;
        }
        assert_eq!(seen, 3);
    }

    /// The class byte counts must sum to the payload length, or the
    /// decomposition table double counts or drops bytes.
    #[test]
    fn class_counts_sum_to_the_payload_length() {
        let (cache, index) = build(
            &[
                r#"{"a":[1,2,3],"f":0.5,"s":"hello"}"#,
                r#"{"a":[1,2,4],"f":0.5,"s":"world"}"#,
            ],
            8,
        );
        for record in records(&cache, &index) {
            let record = record.unwrap();
            let mut sink = CountSink::default();
            let counts = walk_record(record.kind, record.payload, &mut sink).unwrap();
            assert_eq!(sink.counts.total(), record.payload.len() as u64);
            assert!(counts.inline_uint_bytes <= sink.counts.bytes[Class::Struct.index()]);
        }
    }

    /// A table must survive the trip through its own serialized bytes, or a
    /// candidate's measured size includes a table its decoder never used.
    #[test]
    fn dictionary_table_rebuilds_from_its_own_bytes() {
        let mut counts: HashMap<Vec<u8>, u64> = HashMap::new();
        for (body, count) in [
            (b"alpha".to_vec(), 3u64),
            (b"".to_vec(), 5),
            (vec![0xffu8; 300], 2),
            (b"once".to_vec(), 1),
        ] {
            counts.insert(body, count);
        }
        let built = TokenDictionary::build(&counts, 2);
        assert_eq!(built.len(), 3);
        let rebuilt = TokenDictionary::decode(&built.encode()).unwrap();
        assert_eq!(built.order, rebuilt.order);
        for (index, body) in built.order.iter().enumerate() {
            assert_eq!(rebuilt.id(body), Some(index as u64));
        }
        let mut truncated = built.encode();
        truncated.pop();
        assert!(TokenDictionary::decode(&truncated).is_err());
        let mut extended = built.encode();
        extended.push(0);
        assert!(TokenDictionary::decode(&extended).is_err());
    }

    /// The array-delta rewrite must return the original payload for every
    /// record, including the length changes that are the whole point of it.
    /// Its state is carried across records, so a per-record test would not
    /// exercise the case that matters.
    #[test]
    fn array_delta_rewrite_round_trips_across_length_changes() {
        let (cache, index) = build(
            &[
                r#"{"other":1,"queue":[{"id":1,"n":10},{"id":2,"n":20}]}"#,
                // grows at the tail
                r#"{"other":1,"queue":[{"id":1,"n":10},{"id":2,"n":21},{"id":3,"n":30}]}"#,
                // shrinks from the head
                r#"{"other":2,"queue":[{"id":2,"n":21},{"id":3,"n":30}]}"#,
                // same length again: this one never reaches the array path
                r#"{"other":2,"queue":[{"id":2,"n":22},{"id":3,"n":30}]}"#,
            ],
            8,
        );
        let mut writer = ArrayDeltaSink::new();
        let mut reader = ArrayDeltaDecoder::new();
        let mut rewrote = 0;
        for record in records(&cache, &index) {
            let record = record.unwrap();
            writer.begin_record();
            walk_record(record.kind, record.payload, &mut writer).unwrap();
            let decoded = reader.decode_record(record.kind, &writer.out).unwrap();
            assert_eq!(
                decoded, record.payload,
                "array delta lost hour {}",
                record.hour_index
            );
            rewrote += 1;
        }
        assert_eq!(rewrote, 4);
        assert!(
            writer.first_sighting + writer.rewritten >= 2,
            "the fixture must actually exercise whole-array replacement"
        );
        assert!(
            writer.rewritten >= 1,
            "the second length change must be delta coded against the first"
        );
    }

    /// The shared-edge split must never let the prefix and the suffix claim the
    /// same byte, or the middle length underflows and the rebuild is wrong.
    #[test]
    fn shared_edges_never_overlap() {
        assert_eq!(shared_edges(b"aaaa", b"aaaa"), (4, 0));
        assert_eq!(shared_edges(b"aaaa", b"aa"), (2, 0));
        assert_eq!(shared_edges(b"abcz", b"abz"), (2, 1));
        assert_eq!(shared_edges(b"", b"abc"), (0, 0));
        for (previous, current) in [
            (&b"aaaa"[..], &b"aaaa"[..]),
            (b"aaaa", b"aa"),
            (b"abcz", b"abz"),
            (b"xyz", b"abc"),
        ] {
            let (prefix, suffix) = shared_edges(previous, current);
            assert!(prefix + suffix <= previous.len().min(current.len()));
        }
    }

    /// A dictionary must be a function of the counts alone: same content, same
    /// table, on any machine and in any iteration order.
    #[test]
    fn dictionary_order_is_deterministic_under_hash_order() {
        let mut counts: HashMap<Vec<u8>, u64> = HashMap::new();
        for (body, count) in [
            (b"alpha".to_vec(), 3u64),
            (b"beta".to_vec(), 3),
            (b"gamma".to_vec(), 9),
            (b"once".to_vec(), 1),
        ] {
            counts.insert(body, count);
        }
        let first = TokenDictionary::build(&counts, 2);
        // Rebuild from a map populated in a different insertion order.
        let mut shuffled: HashMap<Vec<u8>, u64> = HashMap::new();
        for (body, count) in [
            (b"once".to_vec(), 1u64),
            (b"gamma".to_vec(), 9),
            (b"beta".to_vec(), 3),
            (b"alpha".to_vec(), 3),
        ] {
            shuffled.insert(body, count);
        }
        let second = TokenDictionary::build(&shuffled, 2);
        assert_eq!(first.order, second.order);
        assert_eq!(
            first.order,
            vec![b"gamma".to_vec(), b"alpha".to_vec(), b"beta".to_vec()]
        );
        assert!(first.id(b"once").is_none());
    }

    /// The dictionary sink must produce a payload that decodes back to the
    /// original: rewriting the cache under a candidate encoding is only a
    /// measurement if the candidate is lossless.
    #[test]
    fn dictionary_sink_payload_round_trips() {
        let (cache, index) = build(
            &[
                r#"{"alpha":[1,2,3],"beta":{"alpha":"same","beta":"same"}}"#,
                r#"{"alpha":[1,2,7],"beta":{"alpha":"same","beta":"other"}}"#,
            ],
            8,
        );
        let mut frequencies = FrequencySink::default();
        for record in records(&cache, &index) {
            let record = record.unwrap();
            walk_record(record.kind, record.payload, &mut frequencies).unwrap();
        }
        let keys = TokenDictionary::build(&frequencies.keys, 2);
        let strings = TokenDictionary::build(&frequencies.strings, 2);
        assert!(!keys.is_empty());
        for record in records(&cache, &index) {
            let record = record.unwrap();
            let mut sink = DictionarySink::new([None, Some(&keys), None, Some(&strings)], true);
            walk_record(record.kind, record.payload, &mut sink).unwrap();
            let encoded = sink.payload();
            let decoded = super::decode_dictionary_payload(
                record.kind,
                &encoded,
                [None, Some(&keys), None, Some(&strings)],
                true,
            )
            .unwrap();
            assert_eq!(decoded, record.payload);
        }
    }
}
