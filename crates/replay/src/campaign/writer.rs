//! Changes to a campaign's recording that no capture is writing: one more revision added to it
//! ([`revise`]), the revisions after one cut away ([`cut_back`]), and a recording of records
//! alone ([`create`]). A capture under way writes
//! through [`super::live::LiveWriter`], which holds the recording's newest root; nothing else
//! adds to a recording while one does.

use super::{Revision, records::Records};
use crate::{
    columnar_file::{FileWriter, Settings},
    columnar_frames::schema as tables,
    format::FormatVersion,
    identity::IdentityCodec,
};
use replay_columnar::{
    container::{ContainerWriter, Root, SectionKey, Tags, schema},
    envelope::{self, Kind},
};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom},
    path::Path,
};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

/// What one revision adds to a recording no capture is writing.
#[derive(Default)]
pub struct Addition<'a> {
    /// Records, added as one extent of their own.
    pub records: Option<&'a Records>,
    /// A game facts pack.
    pub facts: Option<&'a [u8]>,
}

/// Adds one revision to the recording at `path`, whose newest revision is `before` with root
/// `root`: `addition`, saved or not. The revision leaves the resume state of a capture under way
/// as the root before it names it. A failure cuts the file back to `before`.
pub fn revise(
    path: &Path,
    (root, before): (&Root, Revision),
    addition: &Addition<'_>,
    saved: bool,
    level: i32,
) -> io::Result<Revision> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    check_revision(&mut file, before)?;
    if file.seek(SeekFrom::End(0))? != before.length {
        return Err(invalid(
            "The recording grew after it was opened; nothing added",
        ));
    }
    let written = (|| {
        let mut container = ContainerWriter::resume(file, root.clone(), before, level, None)?;
        container.section(SectionKey::RECORDS, schema::RECORDS)?;
        container.section(SectionKey::FACTS, schema::FACTS)?;
        if let Some(pack) = addition.facts {
            // Stored as it is: a pack is compressed within, and read at its offsets.
            let extent = container.put(Kind::Facts, Tags::default(), pack.len() as u64, pack)?;
            container.append(SectionKey::FACTS, extent)?;
        }
        if let Some(records) = addition.records.filter(|records| !records.is_empty()) {
            let bytes = records.encode(FormatVersion::CURRENT)?;
            let extent = container.put_zstd(Kind::Controls, Tags::default(), &bytes, level)?;
            container.append(SectionKey::RECORDS, extent)?;
        }
        let revision = container.commit_carrying(saved, |file| file.sync_data())?;
        container.into_inner().sync_all()?;
        Ok(revision)
    })();
    if written.is_err() {
        // Readers keep the revision they opened; the torn tail goes.
        cut_back(path, before)?;
    }
    written
}

/// Creates at `path` the recording of `campaign` that holds `records` alone, not saved: what a
/// capture that lost every hour of the campaign leaves to say. Its content and capture mode are
/// its first session's. A failure leaves no file.
pub fn create(path: &Path, campaign: &[u8], records: &Records, level: i32) -> io::Result<Revision> {
    let session = records
        .sessions
        .first()
        .ok_or_else(|| invalid("A recording holds a session"))?;
    let tables = tables()?;
    let mut content = Vec::new();
    session.identity.encode(&mut content)?;
    let identity = envelope::Identity {
        schema: envelope::schema_digest(&tables),
        campaign: campaign.to_vec(),
        content,
        capture_mode: session.capture_mode.byte(),
    };
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)?;
    let written = (|| {
        let mut writer = FileWriter::create(file, identity, tables.len(), Settings { level }, None)?;
        let bytes = records.encode(FormatVersion::CURRENT)?;
        let container = writer.container();
        let extent = container.put_zstd(Kind::Controls, Tags::default(), &bytes, level)?;
        container.append(SectionKey::RECORDS, extent)?;
        let (file, revision, _) = writer.finish_runs(&[], false, |file| file.sync_data())?;
        file.sync_all()?;
        Ok(revision)
    })();
    if written.is_err() {
        std::fs::remove_file(path)?;
    }
    written
}

/// Whether the file at `path` holds `revision`: its footer there names that root.
pub fn holds_revision(path: &Path, revision: Revision) -> io::Result<bool> {
    let mut file = File::open(path)?;
    if file.seek(SeekFrom::End(0))? < revision.length {
        return Ok(false);
    }
    match check_revision(&mut file, revision) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::InvalidData => Ok(false),
        Err(error) => Err(error),
    }
}

/// Refuses a file whose footer ending at `revision.length` does not name its root.
fn check_revision(file: &mut File, revision: Revision) -> io::Result<()> {
    let start = revision
        .length
        .checked_sub(envelope::FOOTER_BYTES as u64)
        .ok_or_else(|| invalid("A revision ends before its footer"))?;
    let mut footer = [0; envelope::FOOTER_BYTES];
    file.seek(SeekFrom::Start(start))?;
    file.read_exact(&mut footer)?;
    if &footer[32..] != envelope::FOOTER_MAGIC || footer[16..32] != revision.root {
        return Err(invalid("The recording does not hold the revision"));
    }
    Ok(())
}

/// Cuts the recording at `path` back to `revision`, which it must hold: every later one goes,
/// durably. Cutting a recording back to the revision it ends at changes nothing.
pub fn cut_back(path: &Path, revision: Revision) -> io::Result<()> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    check_revision(&mut file, revision)?;
    file.set_len(revision.length)?;
    file.sync_all()
}
