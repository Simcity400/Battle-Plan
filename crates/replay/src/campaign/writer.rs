
use super::Revision;
use replay_recording::records::Records;
#[cfg(any(test, feature = "recording"))]
use replay_journal::columnar_file::FileWriter;
#[cfg(any(test, feature = "recording"))]
use replay_settings::compression::Settings;
#[cfg(any(test, feature = "recording"))]
use replay_recording::schema::schema as tables;
#[cfg(any(test, feature = "recording"))]
use replay_data::identity::IdentityCodec;
use replay_recording::columnar_frames::Recording;
use replay_data::format::FormatVersion;
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

#[derive(Default)]
pub struct Addition<'a> {
    pub records: Option<&'a Records>,
    pub facts: Option<&'a [u8]>,
}

pub fn revise(path: &Path, recording: &Recording, addition: &Addition<'_>, saved: bool, level: i32) -> io::Result<Revision> {
    let (root, before): (&Root, Revision) = (recording.root(), recording.revision());
    let records = match addition.records.filter(|records| !records.is_empty()) {
        Some(records) => Some(records.encode(FormatVersion::read(recording.format())?)?),
        None => None,
    };
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
            let extent = container.put(Kind::Facts, Tags::default(), pack.len() as u64, pack)?;
            container.append(SectionKey::FACTS, extent)?;
        }
        if let Some(bytes) = &records {
            let extent = container.put_zstd(Kind::Controls, Tags::default(), bytes, level)?;
            container.append(SectionKey::RECORDS, extent)?;
        }
        let revision = container.commit_carrying(saved, |file| file.sync_data())?;
        container.into_inner().sync_all()?;
        Ok(revision)
    })();
    if written.is_err() {
        cut_back(path, before)?;
    }
    written
}

#[cfg(any(test, feature = "recording"))]
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

pub fn cut_back(path: &Path, revision: Revision) -> io::Result<()> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    check_revision(&mut file, revision)?;
    file.set_len(revision.length)?;
    file.sync_all()
}
