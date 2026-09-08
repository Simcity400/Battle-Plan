use super::*;
use sha2::{Digest, Sha256};
use snapshot::design_catalog::{apply_a27_row_delta, PortableDesignCatalog};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

// Journal framing retains its 256 MiB bound independently of native read accounting.
const MAX_JOURNAL_ENTRY_BYTES: usize = 256 * 1024 * 1024;

pub struct TimelineJournal {
    writer: BufWriter<File>,
    previous: Option<PortableDesignCatalog>,
    path: PathBuf,
    binding: custody::StreamBinding,
}
impl TimelineJournal {
    pub fn create(
        path: PathBuf,
        binding: custody::StreamBinding,
    ) -> std::result::Result<Self, String> {
        let writer = BufWriter::new(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|e| format!("create native A27 journal: {e}"))?,
        );
        let mut journal = Self {
            writer,
            previous: None,
            path,
            binding,
        };
        journal.line(&json!({"kind":"stream_binding","binding":journal.binding}))?;
        Ok(journal)
    }
    pub fn last_hour(&self) -> Option<u64> {
        self.previous.as_ref().map(|c| c.capture_hour_index)
    }
    fn line(&mut self, value: &Value) -> std::result::Result<(), String> {
        serde_json::to_writer(&mut self.writer, value)
            .map_err(|e| format!("write native A27 observation: {e}"))?;
        self.writer
            .write_all(b"\n")
            .and_then(|_| self.writer.flush())
            .map_err(|e| format!("flush native A27 observation: {e}"))
    }
    pub fn refusal(
        &mut self,
        date: Option<u32>,
        reason: &CatalogFailure,
    ) -> std::result::Result<(), String> {
        self.line(&json!({"kind":"refused_observation","current_date_raw":date,"reason":reason}))
    }
    pub fn push(&mut self, first: Value, recheck: Value) -> std::result::Result<(), String> {
        if first != recheck {
            return Err("native A27 same-hour recheck disagrees".into());
        }
        self.binding.validate_snapshot(&first)?;
        let bytes = serde_json::to_vec(&snapshot::design_catalog::canonical_json_value(&first))
            .map_err(|e| e.to_string())?;
        let current = PortableDesignCatalog::decode_a27_live_json(&bytes)?;
        if self
            .last_hour()
            .is_some_and(|hour| current.capture_hour_index <= hour)
        {
            return Err("native A27 accepted hour did not advance".into());
        }
        let hash = hex(&Sha256::digest(&bytes));
        let mut entry = json!({"kind":"capture","capture_hour_index":current.capture_hour_index,"snapshot_sha256":hash,"same_hour_rechecks":1,"same_hour_recheck_sha256":[hash]});
        if let Some(previous) = self.previous.as_ref().filter(|previous| {
            previous.capture_hour_index.checked_add(1) == Some(current.capture_hour_index)
        }) {
            entry["delta"] = current.native_row_delta_from(previous)?;
        } else {
            entry["snapshot"] = first;
        }
        self.line(&entry)?;
        self.previous = Some(current);
        Ok(())
    }
    pub fn finish(mut self) -> std::result::Result<PathBuf, String> {
        self.writer
            .flush()
            .map_err(|e| format!("flush native A27 journal: {e}"))?;
        self.writer
            .get_ref()
            .sync_all()
            .map_err(|e| format!("sync native A27 journal: {e}"))?;
        Ok(self.path)
    }
}

fn write_outcome(
    writer: &mut BufWriter<File>,
    count: &mut u64,
    mut entry: Value,
) -> std::result::Result<(), String> {
    if *count != 0 {
        writer.write_all(b",").map_err(|error| error.to_string())?;
    }
    entry["sequence"] = json!(*count);
    serde_json::to_writer(writer, &entry).map_err(|error| error.to_string())?;
    *count = count
        .checked_add(1)
        .ok_or("native A27 outcome count overflow")?;
    Ok(())
}

fn write_gap(
    writer: &mut BufWriter<File>,
    count: &mut u64,
    first: u64,
    last: u64,
    refusal: Option<&Value>,
) -> std::result::Result<(), String> {
    use snapshot::design_catalog::{DesignCatalogGap, DesignCatalogGapCause};
    if first > last {
        return Ok(());
    }
    let observed = refusal
        .and_then(|refusal| refusal["current_date_raw"].as_u64())
        .and_then(|date| date.checked_sub(u64::from(DATE_EPOCH)));
    if let Some(hour) = observed.filter(|hour| (first..=last).contains(hour)) {
        if hour > first {
            write_gap(writer, count, first, hour - 1, None)?;
        }
        let refusal = refusal.unwrap();
        let cause: DesignCatalogGapCause =
            serde_json::from_value(refusal["reason"]["kind"].clone())
                .map_err(|error| format!("native A27 refusal kind: {error}"))?;
        let gap = DesignCatalogGap {
            first_hour_index: hour,
            last_hour_index: hour,
            cause,
            witness: refusal.clone(),
        };
        write_outcome(writer, count, json!({"capture_hour_index":hour,"gap":gap}))?;
        if hour < last {
            write_gap(writer, count, hour + 1, last, None)?;
        }
    } else {
        let gap = DesignCatalogGap {
            first_hour_index: first,
            last_hour_index: last,
            cause: DesignCatalogGapCause::NotObserved,
            witness: json!({"reason":"no complete current-hour census before seal","last_refusal":refusal}),
        };
        write_outcome(writer, count, json!({"capture_hour_index":first,"gap":gap}))?;
    }
    Ok(())
}

pub fn seal_journal(
    journal: &Path,
    output: &Path,
    first: u64,
    last: u64,
    binding: &custody::StreamBinding,
) -> std::result::Result<(), String> {
    if last < first || last > u64::from(u32::MAX - DATE_EPOCH) {
        return Err("native A27 replay hour range is invalid".into());
    }
    let mut reader = BufReader::new(
        File::open(journal).map_err(|error| format!("open native A27 journal: {error}"))?,
    );
    let mut header = Vec::new();
    std::io::Read::take(&mut reader, 4097)
        .read_until(b'\n', &mut header)
        .map_err(|error| format!("read native A27 binding: {error}"))?;
    if header.len() > 4096 {
        return Err("native A27 binding exceeds bound".into());
    }
    let header = snapshot::parse_json_strict(&header).map_err(|error| error.to_string())?;
    if header != json!({"kind":"stream_binding","binding":binding}) {
        return Err("native A27 journal belongs to a different acknowledged stream".into());
    }
    let temporary = output.with_extension("sealing.json");
    let mut writer = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| format!("create native A27 seal: {error}"))?,
    );
    writer
        .write_all(b"{\"captures\":[")
        .map_err(|error| error.to_string())?;
    let mut current: Option<Value> = None;
    let mut previous_hour: Option<u64> = None;
    let mut count = 0u64;
    let mut measured = 0u64;
    let mut expected = first;
    let mut refusal: Option<Value> = None;
    let mut after_gap = false;
    loop {
        let mut line = Vec::new();
        std::io::Read::take(&mut reader, (MAX_JOURNAL_ENTRY_BYTES + 1) as u64)
            .read_until(b'\n', &mut line)
            .map_err(|error| format!("read native A27 journal: {error}"))?;
        if line.is_empty() {
            break;
        }
        if line.len() > MAX_JOURNAL_ENTRY_BYTES {
            return Err("native A27 journal entry exceeds bound".into());
        }
        let mut entry = snapshot::parse_json_strict(&line)
            .map_err(|error| format!("native A27 journal JSON: {error}"))?;
        if entry["kind"] == "refused_observation" {
            let hour = entry["current_date_raw"]
                .as_u64()
                .and_then(|date| date.checked_sub(u64::from(DATE_EPOCH)));
            if let Some(hour) = hour.filter(|hour| *hour > expected && expected <= last) {
                write_gap(
                    &mut writer,
                    &mut count,
                    expected,
                    (hour - 1).min(last),
                    refusal.as_ref(),
                )?;
                expected = hour.min(last + 1);
                after_gap = true;
                refusal = None;
            }
            if expected <= last && hour.is_none_or(|hour| hour >= expected) {
                refusal = Some(entry);
            }
            continue;
        }
        if entry["kind"] != "capture" {
            return Err("native A27 journal record kind is invalid".into());
        }
        let hour = entry["capture_hour_index"]
            .as_u64()
            .ok_or("native A27 journal hour missing")?;
        if previous_hour.is_some_and(|previous| hour <= previous) {
            return Err("native A27 journal hour did not advance".into());
        }
        let needs_baseline =
            previous_hour.is_none_or(|previous| previous.checked_add(1) != Some(hour));
        let snapshot = if let Some(full) = entry.get("snapshot") {
            if !needs_baseline || entry.get("delta").is_some() {
                return Err("native A27 journal unexpected full baseline".into());
            }
            full.clone()
        } else {
            if needs_baseline {
                return Err("native A27 journal gap recovery lacks complete baseline".into());
            }
            apply_a27_row_delta(
                current
                    .as_ref()
                    .ok_or("native A27 journal delta lacks baseline")?,
                entry
                    .get("delta")
                    .ok_or("native A27 journal omitted delta")?,
            )?
        };
        let bytes = serde_json::to_vec(&snapshot::design_catalog::canonical_json_value(&snapshot))
            .map_err(|error| error.to_string())?;
        let hash = hex(&Sha256::digest(&bytes));
        if entry["snapshot_sha256"] != hash
            || entry["same_hour_rechecks"] != 1
            || entry["same_hour_recheck_sha256"] != json!([hash])
        {
            return Err("native A27 journal witness/hash mismatch".into());
        }
        if snapshot["authority"]["capture_hour_index"] != hour {
            return Err("native A27 journal hour disagrees with observation".into());
        }
        binding.validate_snapshot(&snapshot)?;
        // Admission revalidates the full reconstructed definition and membership, including outside the selected spine.
        PortableDesignCatalog::decode_a27_live_json(&bytes)?;
        if (first..=last).contains(&hour) {
            if hour < expected {
                return Err("native A27 census overlaps a sealed gap".into());
            }
            if hour > expected {
                write_gap(
                    &mut writer,
                    &mut count,
                    expected,
                    hour - 1,
                    refusal.as_ref(),
                )?;
                after_gap = true;
            }
            let object = entry
                .as_object_mut()
                .ok_or("native A27 journal entry is not an object")?;
            object.remove("kind");
            if measured == 0 || after_gap {
                object.remove("delta");
                object.insert("snapshot".into(), snapshot.clone());
            }
            write_outcome(&mut writer, &mut count, entry)?;
            measured += 1;
            expected = hour + 1;
            after_gap = false;
            refusal = None;
        }
        current = Some(snapshot);
        previous_hour = Some(hour);
    }
    if expected <= last {
        write_gap(&mut writer, &mut count, expected, last, refusal.as_ref())?;
    }
    let tail = json!({"schema":snapshot::design_catalog::A27_LIVE_CATALOG_TIMELINE_SCHEMA,
        "authority":{"program":"hoi4.exe 1.19.2.0.a729","program_sha256":PROGRAM_SHA256,"producer":"native_reference_registry","stream_binding":binding},
        "transport":"first_full_then_native_row_deltas","complete_sequence":true,"capture_count":count,
        "first_capture_hour_index":first,"last_capture_hour_index":last,"same_hour_recheck_count":measured});
    writer.write_all(b"]").map_err(|error| error.to_string())?;
    for (key, value) in tail.as_object().unwrap() {
        writer.write_all(b",").map_err(|error| error.to_string())?;
        serde_json::to_writer(&mut writer, key).map_err(|error| error.to_string())?;
        writer.write_all(b":").map_err(|error| error.to_string())?;
        serde_json::to_writer(&mut writer, value).map_err(|error| error.to_string())?;
    }
    writer
        .write_all(b"}")
        .and_then(|_| writer.flush())
        .map_err(|error| error.to_string())?;
    writer
        .get_ref()
        .sync_all()
        .map_err(|error| error.to_string())?;
    drop(writer);
    std::fs::rename(&temporary, output).map_err(|error| format!("publish native A27 seal: {error}"))
}
