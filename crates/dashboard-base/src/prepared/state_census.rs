//! Opt-in preparation accounting; serialization bytes are not allocation or process-memory bytes.
use serde::Serialize;
use std::{
    fmt::{Display, Write as _},
    io,
    sync::LazyLock,
};

static ON: LazyLock<bool> =
    LazyLock::new(|| std::env::var_os("BATTLE_PLAN_PREPARE_STATE_CENSUS").or_else(|| std::env::var_os("HOI4_PREPARE_STATE_CENSUS")).is_some());

/// Whether the census is on.
pub fn on() -> bool {
    *ON
}

/// The bytes of `value`'s JSON, counted as they are written and never held.
pub fn json_bytes<T: Serialize + ?Sized>(value: &T) -> io::Result<u64> {
    struct Count(u64);
    impl io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0 += bytes.len() as u64;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    serde_json::to_writer(&mut count, value)?;
    Ok(count.0)
}

/// One census line: its kind, then its fields as `name value` pairs.
#[derive(Default)]
pub struct Line(String);

impl Line {
    pub fn new(kind: &str) -> Self {
        Self(format!("PROFILE state-census {kind}"))
    }

    pub fn field(mut self, name: impl Display, value: impl Display) -> Self {
        let _ = write!(self.0, " {name} {value}");
        self
    }

    /// `value`'s JSON bytes ([`json_bytes`]), or `-` when it has none alone.
    pub fn json<T: Serialize + ?Sized>(self, name: impl Display, value: &T) -> Self {
        match json_bytes(value) {
            Ok(bytes) => self.field(name, bytes),
            Err(_) => self.field(name, "-"),
        }
    }

    pub fn print(self) {
        eprintln!("{}", self.0);
    }
}

pub struct Marks(Option<Marked>);

struct Marked {
    line: Line,
    first: (u64, i128),
    last: (u64, i128),
}

/// A writer's new range bytes and its change in published range bytes.
fn counts(writer: &::store::Writer) -> (u64, i128) {
    (writer.bytes_written(), writer.payload_bytes_change())
}

/// The bytes written and replaced between two counts.
fn between(from: (u64, i128), to: (u64, i128)) -> (u64, i128) {
    let written = to.0.saturating_sub(from.0);
    (written, i128::from(written) - (to.1 - from.1))
}

impl Marks {
    pub fn start(writer: &::store::Writer, line: impl FnOnce() -> Line) -> Self {
        Self::when(on(), writer, line)
    }

    fn when(enabled: bool, writer: &::store::Writer, line: impl FnOnce() -> Line) -> Self {
        Self(enabled.then(|| {
            let first = counts(writer);
            Marked {
                line: line(),
                first,
                last: first,
            }
        }))
    }

    /// What `writer` wrote since the last mark, as the part `part`.
    pub fn mark(&mut self, part: &str, writer: &::store::Writer) {
        if let Some(marked) = &mut self.0 {
            let now = counts(writer);
            let (written, replaced) = between(marked.last, now);
            marked.line = std::mem::take(&mut marked.line)
                .field(format_args!("{part}_written"), written)
                .field(format_args!("{part}_replaced"), replaced);
            marked.last = now;
        }
    }

    pub fn print(mut self, part: &str, writer: &::store::Writer) {
        self.mark(part, writer);
        if let Some(marked) = self.0 {
            let (written, replaced) = between(marked.first, marked.last);
            marked
                .line
                .field("written", written)
                .field("replaced", replaced)
                .print();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_sizes_count_encoded_bytes_without_retaining_a_serialized_copy() {
        let value = serde_json::json!({"countries": [1, 2], "name": "test"});
        assert_eq!(json_bytes(&value).unwrap(), 33);
        assert_eq!(
            Line::new("held").json("bytes", &value).0,
            "PROFILE state-census held bytes 33"
        );
        let invalid = std::collections::BTreeMap::from([(vec![1, 2], 3)]);
        assert!(json_bytes(&invalid).is_err());
        assert_eq!(
            Line::new("held").json("bytes", &invalid).0,
            "PROFILE state-census held bytes -"
        );
    }

    #[test]
    fn written_and_replaced_sizes_are_separate_including_net_shrinkage() {
        assert_eq!(between((10, 5), (30, 15)), (20, 10));
        assert_eq!(between((30, 15), (33, 8)), (3, 10));
        assert_eq!(between((33, 8), (33, 8)), (0, 0));
    }

    #[test]
    fn a_disabled_census_neither_serializes_state_nor_marks_writes() {
        let fixture = super::super::series::tests::Fixture::new();
        let writer = fixture.writer();
        let mut marks = Marks::when(false, &writer, || panic!("disabled census sampled state"));
        marks.mark("pages", &writer);
        assert!(marks.0.is_none());
    }
}
