use super::*;
use index::{IndexCursor, IndexRoot};
use pages::Pages;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Root {
    pub index: IndexRoot,
    pub last: Option<i32>,
}
impl Resident for Root {
    fn heap_bytes(&self) -> usize {
        0
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Entry {
    pub first: i32,
    pub last: i32,
    pub payload: u64,
}
impl Resident for Entry {
    fn heap_bytes(&self) -> usize {
        0
    }
}
pub(super) fn append(
    writer: &mut store::Writer,
    section: &str,
    root: &mut Root,
    entry: Entry,
    page_bytes: usize,
) -> io::Result<()> {
    if root.last.is_some_and(|last| entry.first <= last) || entry.first > entry.last {
        return Err(invalid("Private fact directory did not advance"));
    }
    root.index = index::put(
        writer,
        section,
        root.index,
        &entry.last,
        entry,
        page_bytes,
    )?;
    root.last = Some(entry.last);
    Ok(())
}
pub(super) fn cursor<R: ReadPages>(
    cache: &mut Pages<'_, R>,
    section: String,
    root: Root,
    first: i32,
) -> io::Result<IndexCursor<i32, Entry>> {
    IndexCursor::new(cache, section, root.index, Some(&first), None)
}
