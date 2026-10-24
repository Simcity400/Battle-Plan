use super::*;
pub use content_witness::{Entry, Layer, Outcome, Payload, Recorder, WITNESS_CONTRACT, Witness};

pub(super) trait Publish {
    fn publish(&self, writer: &mut Writer, key: &Key) -> io::Result<()>;
}
impl Publish for Witness {
    fn publish(&self, writer: &mut Writer, key: &Key) -> io::Result<()> {
        self.visit(|ordinal, bytes| writer.write_at(&key.section("witness"), ordinal, bytes))
    }
}

pub(super) fn verify(
    key: &Key,
    read: impl FnMut(u64) -> io::Result<Option<store::Range>>,
    check: impl FnMut() -> io::Result<()>,
) -> io::Result<()> {
    content_witness::verify(
        key.content,
        key.content_entries,
        read,
        |range| {
            let length = usize::try_from(range.len())
                .map_err(|_| invalid("content witness exceeds address space"))?;
            codec::read_exact_range(range, length)
        },
        check,
    )
}
