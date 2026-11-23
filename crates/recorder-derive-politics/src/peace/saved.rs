
use super::{Conference, Countries, Government, Peace, Snapshot, Tag, Wars, invalid};
use replay_columnar::leaf::{Reader, Writer as Bytes};
use replay_data::rows::PeaceConferenceSpanRow;
use std::{collections::HashMap, io, sync::Arc};

fn int(blob: &mut Bytes, value: i64) -> io::Result<()> {
    blob.varint(((value << 1) ^ (value >> 63)) as u64)
}

fn read_int(reader: &mut Reader<'_>) -> io::Result<i64> {
    let bits = reader.varint()?;
    Ok(((bits >> 1) as i64) ^ -((bits & 1) as i64))
}

fn read_i32(reader: &mut Reader<'_>) -> io::Result<i32> {
    i32::try_from(read_int(reader)?).map_err(|_| invalid("Peace's state holds an hour past 32 bits"))
}

fn optional(blob: &mut Bytes, value: Option<i64>) -> io::Result<()> {
    match value {
        None => blob.u8(0),
        Some(value) => {
            blob.u8(1)?;
            int(blob, value)
        }
    }
}

fn read_optional(reader: &mut Reader<'_>) -> io::Result<Option<i64>> {
    Ok(match reader.u8()? {
        0 => None,
        _ => Some(read_int(reader)?),
    })
}

fn bytes(blob: &mut Bytes, bytes: &[u8]) -> io::Result<()> {
    blob.varint(bytes.len() as u64)?;
    blob.raw(bytes)
}

fn read_bytes(reader: &mut Reader<'_>) -> io::Result<Box<[u8]>> {
    let length = reader.count()?;
    Ok(reader.raw(length)?.into())
}

fn optional_bytes(blob: &mut Bytes, value: Option<&[u8]>) -> io::Result<()> {
    match value {
        None => blob.u8(0),
        Some(value) => {
            blob.u8(1)?;
            bytes(blob, value)
        }
    }
}

fn read_optional_bytes(reader: &mut Reader<'_>) -> io::Result<Option<Box<[u8]>>> {
    Ok(match reader.u8()? {
        0 => None,
        _ => Some(read_bytes(reader)?),
    })
}

fn count(reader: &mut Reader<'_>, blob: usize) -> io::Result<usize> {
    let count = reader.count()?;
    if count > blob {
        return Err(invalid("Peace's state counts more entries than it holds"));
    }
    Ok(count)
}

fn sorted<K: Ord + Copy, V>(map: &HashMap<K, V>) -> Vec<(K, &V)> {
    let mut entries: Vec<(K, &V)> = map.iter().map(|(key, value)| (*key, value)).collect();
    entries.sort_unstable_by_key(|(key, _)| *key);
    entries
}

fn snapshot(blob: &mut Bytes, snapshot: &Snapshot) -> io::Result<()> {
    int(blob, snapshot.hour.into())?;
    blob.u8(u8::from(snapshot.roster_complete))?;
    let tags = sorted(&snapshot.countries.tags);
    blob.varint(tags.len() as u64)?;
    for (raw, tag) in tags {
        blob.varint(raw.into())?;
        blob.varint((*tag).into())?;
    }
    let identities = sorted(&snapshot.countries.identities);
    blob.varint(identities.len() as u64)?;
    for (tag, (slot, id)) in identities {
        blob.varint(tag.into())?;
        blob.varint((*slot).into())?;
        blob.varint((*id).into())?;
    }
    let owners = sorted(&snapshot.owners);
    blob.varint(owners.len() as u64)?;
    for (state, owner) in owners {
        blob.varint(state.into())?;
        blob.varint((*owner).into())?;
    }
    let pairs = sorted(&snapshot.wars.pairs);
    blob.varint(pairs.len() as u64)?;
    for ((a, b), (start, end)) in pairs {
        blob.varint(a.into())?;
        blob.varint(b.into())?;
        int(blob, (*start).into())?;
        int(blob, (*end).into())?;
    }
    blob.varint(snapshot.wars.unresolved.len() as u64)?;
    for (a, b) in &snapshot.wars.unresolved {
        blob.varint((*a).into())?;
        blob.varint((*b).into())?;
    }
    match &snapshot.vps {
        None => blob.u8(0)?,
        Some(vps) => {
            blob.u8(1)?;
            let vps = sorted(vps);
            blob.varint(vps.len() as u64)?;
            for (state, points) in vps {
                blob.varint(state.into())?;
                int(blob, *points)?;
            }
        }
    }
    match &snapshot.subjects {
        None => blob.u8(0)?,
        Some(subjects) => {
            blob.u8(1)?;
            let subjects = sorted(subjects);
            blob.varint(subjects.len() as u64)?;
            for ((overlord, subject), start) in subjects {
                blob.varint(overlord.into())?;
                blob.varint(subject.into())?;
                int(blob, (*start).into())?;
            }
        }
    }
    match &snapshot.governments {
        None => blob.u8(0)?,
        Some(governments) => {
            blob.u8(1)?;
            let governments = sorted(governments);
            blob.varint(governments.len() as u64)?;
            for (tag, government) in governments {
                blob.varint(tag.into())?;
                optional(blob, government.ruling.map(i64::from))?;
                optional_bytes(blob, government.party.as_deref())?;
                optional_bytes(blob, government.ideology.as_deref())?;
            }
        }
    }
    match &snapshot.exists {
        None => blob.u8(0),
        Some(exists) => {
            blob.u8(1)?;
            let exists = sorted(exists);
            blob.varint(exists.len() as u64)?;
            for (raw, exists) in exists {
                blob.varint(raw.into())?;
                blob.u8(u8::from(*exists))?;
            }
            Ok(())
        }
    }
}

fn read_snapshot(reader: &mut Reader<'_>, blob: usize) -> io::Result<Snapshot> {
    let hour = read_i32(reader)?;
    let roster_complete = reader.u8()? != 0;
    let mut countries = Countries::default();
    for _ in 0..count(reader, blob)? {
        countries.tags.insert(reader.varint_u32()?, reader.varint_u32()?);
    }
    for _ in 0..count(reader, blob)? {
        let tag = reader.varint_u32()?;
        countries.identities.insert(tag, (reader.varint_u32()?, reader.varint_u32()?));
    }
    let mut owners = HashMap::new();
    for _ in 0..count(reader, blob)? {
        owners.insert(reader.varint_u32()?, reader.varint_u32()?);
    }
    let mut wars = Wars::default();
    for _ in 0..count(reader, blob)? {
        let pair = (reader.varint_u32()?, reader.varint_u32()?);
        wars.pairs.insert(pair, (read_i32(reader)?, read_i32(reader)?));
    }
    for _ in 0..count(reader, blob)? {
        wars.unresolved.push((reader.varint_u32()?, reader.varint_u32()?));
    }
    let vps = match reader.u8()? {
        0 => None,
        _ => {
            let mut vps = HashMap::new();
            for _ in 0..count(reader, blob)? {
                vps.insert(reader.varint_u32()?, read_int(reader)?);
            }
            Some(Arc::new(vps))
        }
    };
    let subjects = match reader.u8()? {
        0 => None,
        _ => {
            let mut subjects = HashMap::new();
            for _ in 0..count(reader, blob)? {
                let pair = (reader.varint_u32()?, reader.varint_u32()?);
                subjects.insert(pair, read_i32(reader)?);
            }
            Some(Arc::new(subjects))
        }
    };
    let governments = match reader.u8()? {
        0 => None,
        _ => {
            let mut governments = HashMap::new();
            for _ in 0..count(reader, blob)? {
                let tag: Tag = reader.varint_u32()?;
                let ruling = read_optional(reader)?.map(|ruling| u32::try_from(ruling).map_err(|_| invalid("Peace's state holds a party past 32 bits"))).transpose()?;
                let party = read_optional_bytes(reader)?;
                let ideology = read_optional_bytes(reader)?;
                governments.insert(tag, Government { ruling, party, ideology });
            }
            Some(Arc::new(governments))
        }
    };
    let exists = match reader.u8()? {
        0 => None,
        _ => {
            let mut exists = HashMap::new();
            for _ in 0..count(reader, blob)? {
                exists.insert(reader.varint_u32()?, reader.u8()? != 0);
            }
            Some(Arc::new(exists))
        }
    };
    Ok(Snapshot { hour, roster_complete, countries: Arc::new(countries), owners: Arc::new(owners), wars: Arc::new(wars), vps, subjects, governments, exists })
}

fn optional_snapshot(blob: &mut Bytes, value: Option<&Snapshot>) -> io::Result<()> {
    match value {
        None => blob.u8(0),
        Some(value) => {
            blob.u8(1)?;
            snapshot(blob, value)
        }
    }
}

fn read_optional_snapshot(reader: &mut Reader<'_>, blob: usize) -> io::Result<Option<Arc<Snapshot>>> {
    Ok(match reader.u8()? {
        0 => None,
        _ => Some(Arc::new(read_snapshot(reader, blob)?)),
    })
}

pub(super) fn save(peace: &Peace, blob: &mut Bytes) -> io::Result<()> {
    let mut tags: Vec<(&Box<[u8]>, &Tag)> = peace.tags.iter().collect();
    tags.sort_unstable_by_key(|(_, number)| **number);
    blob.varint(tags.len() as u64)?;
    for (tag, _) in tags {
        bytes(blob, tag)?;
    }
    let existing = sorted(&peace.first_existing);
    blob.varint(existing.len() as u64)?;
    for (tag, hour) in existing {
        blob.varint(tag.into())?;
        int(blob, (*hour).into())?;
    }
    blob.u8(u8::from(peace.has_active) | u8::from(peace.has_ambiguous) << 1)?;
    optional_snapshot(blob, peace.previous.as_deref())?;
    blob.varint(peace.conferences.len() as u64)?;
    for (id, conference) in &peace.conferences {
        let row = &conference.row;
        blob.varint(*id)?;
        int(blob, row.first_observed_hour.into())?;
        int(blob, row.last_observed_hour.into())?;
        int(blob, row.current_turn.into())?;
        blob.u8(row.finalized)?;
        optional(blob, row.first_finalized_hour.map(i64::from))?;
        optional(blob, row.first_absent_hour.map(i64::from))?;
        blob.u8(row.comparison)?;
        optional(blob, row.before_hour.map(i64::from))?;
        optional(blob, row.compared_hour.map(i64::from))?;
        blob.u8(u8::from(conference.open) | u8::from(conference.waiting) << 1 | u8::from(conference.processed) << 2 | u8::from(conference.unassigned) << 3)?;
        optional_snapshot(blob, conference.before.as_deref())?;
    }
    Ok(())
}

fn hour(value: Option<i64>) -> io::Result<Option<i32>> {
    value.map(|value| i32::try_from(value).map_err(|_| invalid("Peace's state holds an hour past 32 bits"))).transpose()
}

pub(super) fn load(peace: &mut Peace, blob: &[u8]) -> io::Result<()> {
    let size = blob.len();
    let mut reader = Reader::new(blob);
    for number in 0..count(&mut reader, size)? {
        let tag = read_bytes(&mut reader)?;
        peace.tags.insert(tag, number as Tag);
    }
    for _ in 0..count(&mut reader, size)? {
        let tag = reader.varint_u32()?;
        peace.first_existing.insert(tag, read_i32(&mut reader)?);
    }
    let flags = reader.u8()?;
    (peace.has_active, peace.has_ambiguous) = (flags & 1 != 0, flags & 2 != 0);
    peace.previous = read_optional_snapshot(&mut reader, size)?;
    for _ in 0..count(&mut reader, size)? {
        let id = reader.varint()?;
        let row = PeaceConferenceSpanRow {
            conference: id,
            first_observed_hour: read_i32(&mut reader)?,
            last_observed_hour: read_i32(&mut reader)?,
            current_turn: read_i32(&mut reader)?,
            finalized: reader.u8()?,
            first_finalized_hour: hour(read_optional(&mut reader)?)?,
            first_absent_hour: hour(read_optional(&mut reader)?)?,
            comparison: reader.u8()?,
            before_hour: hour(read_optional(&mut reader)?)?,
            compared_hour: hour(read_optional(&mut reader)?)?,
            ..Default::default()
        };
        let flags = reader.u8()?;
        let before = read_optional_snapshot(&mut reader, size)?;
        peace.conferences.insert(id, Conference { row, before, open: flags & 1 != 0, waiting: flags & 2 != 0, processed: flags & 4 != 0, unassigned: flags & 8 != 0 });
    }
    reader.finish()
}
