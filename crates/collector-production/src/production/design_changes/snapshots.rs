use super::*;
pub(super) mod speculative;
use std::sync::Arc;

pub(super) struct Decoded {
    pub value: rows::VariantSnapshot,
    pub definition: native::DefinitionRow<Arc<rows::VariantDesignState>>,
}
impl Decoded {
    fn allocated_bytes(&self) -> usize {
        size_of::<Decoded>()
            + 2 * size_of::<usize>()
            + replay_data::value::Value::heap_bytes(&self.value)
            + rows::NativeValue::heap_bytes(self.definition.design.as_ref())
            + self
                .definition
                .source_links
                .as_ref()
                .map_or(0, |links| links.direct.capacity() * size_of::<usize>())
    }
}
pub(super) struct Occurrence {
    index: usize,
    pub sequence: u64,
    pub hour: i32,
    pub variant: u64,
    pub value: Arc<Decoded>,
}
#[derive(Clone, Copy)]
pub(super) enum Change<'a> {
    Snapshot(&'a Occurrence),
    Row(&'a rows::VariantChangeRow),
}
pub(super) struct View<'a> {
    pub variant: u64,
    pub flags: Option<[u8; 3]>,
    pub scalar: Option<&'a rows::VariantScalarChange>,
    pub sources: Option<&'a rows::VariantSourceReferences>,
    pub organisation: Option<&'a rows::VariantOrganisationChange>,
    pub removed: bool,
    pub snapshot: Option<&'a rows::VariantSnapshot>,
}
impl<'a> Change<'a> {
    pub fn view(self) -> View<'a> {
        match self {
            Self::Snapshot(snapshot) => View {
                variant: snapshot.variant,
                flags: None,
                scalar: None,
                sources: None,
                organisation: None,
                removed: false,
                snapshot: Some(&snapshot.value.value),
            },
            Self::Row(row) => View {
                variant: row.variant,
                flags: row.flags,
                scalar: row.scalar.as_ref(),
                sources: row.sources.as_ref(),
                organisation: row.organisation.as_ref(),
                removed: row.removed,
                snapshot: row.snapshot.as_ref(),
            },
        }
    }
    pub fn try_clone(self) -> std::io::Result<rows::VariantChangeRow> {
        let row = match self {
            Self::Snapshot(snapshot) => rows::VariantChangeRow {
                sequence: snapshot.sequence,
                hour: snapshot.hour,
                variant: snapshot.variant,
                snapshot: Some(replay_data::value::Value::try_clone(&snapshot.value.value)?),
                ..Default::default()
            },
            Self::Row(row) => return rows::RowCodec::try_clone(row),
        };
        rows::RowCodec::check_key_fields(&row)?;
        Ok(row)
    }
}
#[derive(Clone, Copy)]
pub(in crate::production) struct History<'a> {
    rows: &'a [rows::VariantChangeRow],
    shared: &'a Shared,
}
impl<'a> History<'a> {
    pub(super) fn new(rows: &'a [rows::VariantChangeRow], shared: &'a Shared) -> Self {
        Self { rows, shared }
    }
    #[cfg(test)]
    pub(super) fn from_rows(rows: &'a [rows::VariantChangeRow]) -> Self {
        static EMPTY: Shared = Shared {
            entries: Vec::new(),
            heap_bytes: 0,
        };
        Self {
            rows,
            shared: &EMPTY,
        }
    }
    pub(super) fn len(self) -> usize {
        self.rows.len() + self.shared.len()
    }
    pub(super) fn is_empty(self) -> bool {
        self.len() == 0
    }
    pub(super) fn iter(self) -> impl Iterator<Item = Result<Change<'a>, &'static str>> {
        self.shared.iter(self.rows)
    }
}
#[derive(Default)]
pub(super) struct Shared {
    entries: Vec<Occurrence>,
    heap_bytes: usize,
}
impl Shared {
    pub fn clear(&mut self) {
        self.entries.clear();
        self.heap_bytes = 0;
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<Occurrence>() + self.heap_bytes
    }
    pub fn reserve(&mut self, additional: usize) -> Result<(), &'static str> {
        engine_allocation::reserve(&mut self.entries, additional)
            .map_err(|_| "variant shared snapshot allocation failed")
    }
    pub fn push(
        &mut self,
        index: usize,
        sequence: u64,
        hour: i32,
        variant: u64,
        value: Arc<Decoded>,
    ) -> Result<(), &'static str> {
        self.reserve(1)?;
        self.heap_bytes = self.heap_bytes.saturating_add(value.allocated_bytes());
        self.entries.push(Occurrence {
            index,
            sequence,
            hour,
            variant,
            value,
        });
        Ok(())
    }
    pub fn iter<'a>(
        &'a self,
        source: &'a [rows::VariantChangeRow],
    ) -> impl Iterator<Item = Result<Change<'a>, &'static str>> {
        let mut entries = self.entries.iter().peekable();
        let mut rows = source.iter();
        (0..source.len() + self.entries.len()).map(move |index| {
            if entries.peek().is_some_and(|entry| entry.index == index) {
                Ok(Change::Snapshot(entries.next().unwrap()))
            } else {
                rows.next()
                    .map(Change::Row)
                    .ok_or("variant shared snapshot index invalid")
            }
        })
    }
    #[cfg(test)]
    pub fn materialize(
        &self,
        output: &mut Vec<rows::VariantChangeRow>,
    ) -> Result<(), &'static str> {
        if !self.entries.is_empty() {
            let mut rows = engine_allocation::vec(output.len() + self.len())
                .map_err(|_| "variant shared history allocation failed")?;
            for change in self.iter(output) {
                rows.push(
                    change?
                        .try_clone()
                        .map_err(|_| "variant shared history row invalid")?,
                );
            }
            *output = rows;
        }
        Ok(())
    }
}
