use crate::control_transport::{ControlFrame, MessageKind};
use compact_contract::hourly_service::{
    HourlyServiceAckKindV1, HourlyServiceAckV1, HourlyServiceBatchV1, HourlyServiceCensusV1,
    HourlyServiceOutcomeV1, HourlyServiceSampleV1,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const RECEIPT_FILE_NAME: &str = "hourly_service_receipt.jsonl";
const RECEIPT_LIMIT: usize = 8 * 1024 * 1024;
const SAMPLE_LIMIT: usize = 4096;
const SERVICE_LIMIT_NS: u64 = 16_000_000;
const STAGE_MASK: u8 = 0x7f;
const TERMINAL_STAGE: u8 = 1 << 6;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HourlyServiceBinding {
    pub session_sha256: String,
    pub process_id: u32,
    pub process_start_nonce: u64,
    pub collector_sha256: String,
}

impl HourlyServiceBinding {
    pub fn new(
        session_id: [u8; 16],
        process_id: u32,
        process_start_nonce: u64,
        collector_sha256: [u8; 32],
    ) -> Self {
        Self {
            session_sha256: hex(&Sha256::digest(&session_id)),
            process_id,
            process_start_nonce,
            collector_sha256: hex(&collector_sha256),
        }
    }
}

pub fn is_hourly_service_kind(kind: MessageKind) -> bool {
    matches!(
        kind,
        MessageKind::HourlyServiceBatch | MessageKind::HourlyServiceCensus
    )
}

pub fn rejected_ack(frame: &ControlFrame) -> Option<HourlyServiceAckV1> {
    match frame.kind {
        MessageKind::HourlyServiceBatch => {
            HourlyServiceBatchV1::decode(&frame.payload)
                .ok()
                .map(|batch| HourlyServiceAckV1 {
                    kind: HourlyServiceAckKindV1::Batch,
                    batch_id: batch.batch_id,
                    accepted_count: 0,
                })
        }
        MessageKind::HourlyServiceCensus => {
            HourlyServiceCensusV1::decode(&frame.payload)
                .ok()
                .map(|census| HourlyServiceAckV1 {
                    kind: HourlyServiceAckKindV1::Census,
                    batch_id: census.last_batch_id,
                    accepted_count: 0,
                })
        }
        _ => None,
    }
}

pub struct HourlyServiceReceiver {
    binding: HourlyServiceBinding,
    path: PathBuf,
    rows: Vec<Vec<u8>>,
    bytes: usize,
    last_batch: u64,
    samples: usize,
    census_seen: bool,
    failed: bool,
}

impl HourlyServiceReceiver {
    pub fn new(path: &Path, binding: HourlyServiceBinding) -> Self {
        let mut receiver = Self {
            binding,
            path: path.to_path_buf(),
            rows: Vec::new(),
            failed: false,
            bytes: 0,
            last_batch: 0,
            samples: 0,
            census_seen: false,
        };
        receiver.append(json!({"kind":"binding", "schema":"hoi4.hourly_service_receipt.v1", "binding":receiver.binding}));
        receiver
    }

    fn append(&mut self, row: Value) {
        let encoded = serde_json::to_vec(&row);
        let result = encoded
            .map_err(std::io::Error::other)
            .and_then(|mut bytes| {
                bytes.push(b'\n');
                let next = self
                    .bytes
                    .checked_add(bytes.len())
                    .filter(|value| *value <= RECEIPT_LIMIT)
                    .ok_or_else(|| {
                        std::io::Error::other("hourly-service receipt capacity exceeded")
                    })?;
                self.rows.push(bytes);
                self.bytes = next;
                Ok(())
            });
        if result.is_err() {
            self.failed = true;
        }
    }

    pub fn transport_failed(&mut self, error: &str) {
        self.failed = true;
        self.append(json!({"kind":"diagnostic_fault", "cause":"control_transport_failed", "detail":error.chars().take(512).collect::<String>()}));
    }

    pub fn acknowledge_send_failed(&mut self, error: &str) {
        self.failed = true;
        self.append(json!({"kind":"diagnostic_fault", "cause":"ack_send_failed", "detail":error.chars().take(512).collect::<String>()}));
    }

    pub fn receive(&mut self, frame: &ControlFrame) -> Option<HourlyServiceAckV1> {
        if !is_hourly_service_kind(frame.kind) {
            return None;
        }
        let mut ack = rejected_ack(frame);
        let session_matches =
            self.binding.session_sha256 == hex(&Sha256::digest(frame.session_id.as_bytes()));
        let accepted = if !session_matches || self.census_seen {
            false
        } else if frame.kind == MessageKind::HourlyServiceBatch {
            match HourlyServiceBatchV1::decode(&frame.payload) {
                Ok(batch)
                    if self.last_batch.checked_add(1) == Some(batch.batch_id)
                        && self
                            .samples
                            .checked_add(batch.samples.len())
                            .is_some_and(|count| count <= SAMPLE_LIMIT) =>
                {
                    self.last_batch = batch.batch_id;
                    self.samples += batch.samples.len();
                    true
                }
                _ => false,
            }
        } else {
            self.census_seen = true;
            matches!(HourlyServiceCensusV1::decode(&frame.payload), Ok(census) if census.last_batch_id == self.last_batch)
        };
        if !accepted {
            self.failed = true;
        }
        self.append(json!({
            "kind":"message", "control_kind":frame.kind as u16, "control_sequence":frame.sequence,
            "payload_hex":hex(&frame.payload), "accepted":accepted && !self.failed,
        }));
        // Acknowledgment proves bounded host custody, never disk durability or the performance bar.
        if !self.failed && accepted {
            if let Some(value) = ack.as_mut() {
                value.accepted_count = if frame.kind == MessageKind::HourlyServiceBatch {
                    HourlyServiceBatchV1::decode(&frame.payload)
                        .map(|batch| batch.samples.len() as u16)
                        .unwrap_or(0)
                } else {
                    1
                };
            }
        }
        ack
    }
}

impl Drop for HourlyServiceReceiver {
    fn drop(&mut self) {
        let result = (|| -> std::io::Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&self.path)?;
            for row in &self.rows {
                file.write_all(row)?;
            }
            let completion = json!({"kind":"receipt_complete", "census_received":self.census_seen, "fault":self.failed});
            serde_json::to_writer(&mut file, &completion).map_err(std::io::Error::other)?;
            file.write_all(b"\n")
        })();
        let _ = result;
    }
}

#[derive(Default)]
struct Receipt {
    samples: Vec<HourlyServiceSampleV1>,
    census: Option<HourlyServiceCensusV1>,
    problems: Vec<String>,
    sha256: Option<String>,
}

fn problem(problems: &mut Vec<String>, text: impl Into<String>) {
    if problems.len() < 64 {
        problems.push(text.into());
    }
}

fn load_receipt(path: &Path, expected: &HourlyServiceBinding) -> Receipt {
    let mut receipt = Receipt::default();
    let bytes = (|| -> Result<Vec<u8>, String> {
        let mut file =
            File::open(path).map_err(|error| format!("timing receipt unavailable: {error}"))?;
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take((RECEIPT_LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("read timing receipt: {error}"))?;
        if bytes.len() > RECEIPT_LIMIT {
            return Err("timing receipt exceeds capacity".into());
        }
        if !bytes.ends_with(b"\n") {
            return Err("timing receipt has an incomplete final row".into());
        }
        Ok(bytes)
    })();
    let bytes = match bytes {
        Ok(bytes) => bytes,
        Err(error) => {
            problem(&mut receipt.problems, error);
            return receipt;
        }
    };
    receipt.sha256 = Some(hex(&Sha256::digest(&bytes)));
    let mut last_batch = 0u64;
    let mut last_control = None;
    let mut receipt_complete = false;
    for (index, line) in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .enumerate()
    {
        let row: Value = match serde_json::from_slice(line) {
            Ok(row) => row,
            Err(error) => {
                problem(
                    &mut receipt.problems,
                    format!("receipt row {index}: {error}"),
                );
                continue;
            }
        };
        if index == 0 {
            let binding = row
                .get("binding")
                .cloned()
                .and_then(|value| serde_json::from_value::<HourlyServiceBinding>(value).ok());
            if row["kind"] != "binding"
                || row["schema"] != "hoi4.hourly_service_receipt.v1"
                || binding.as_ref() != Some(expected)
            {
                problem(
                    &mut receipt.problems,
                    "receipt is not bound to this authenticated process/session/collector",
                );
            }
            continue;
        }
        if receipt_complete {
            problem(
                &mut receipt.problems,
                "receipt rows follow its final custody marker",
            );
        }
        if row["kind"] == "receipt_complete" {
            receipt_complete = true;
            if row["census_received"] != true || row["fault"] != false {
                problem(
                    &mut receipt.problems,
                    "host timing custody ended incomplete or with a diagnostic fault",
                );
            }
            continue;
        }
        if row["kind"] != "message" {
            problem(
                &mut receipt.problems,
                "receipt contains a diagnostic transport or persistence fault",
            );
            continue;
        }
        let sequence = row["control_sequence"].as_u64();
        if sequence.is_none() || last_control.is_some_and(|last| sequence.unwrap() <= last) {
            problem(
                &mut receipt.problems,
                "receipt control sequence is absent or not strictly increasing",
            );
        }
        last_control = sequence;
        if row["accepted"] != true {
            problem(&mut receipt.problems, "host rejected timing receipt row");
        }
        if receipt.census.is_some() {
            problem(
                &mut receipt.problems,
                "timing message follows the terminal census",
            );
        }
        let payload = match row["payload_hex"].as_str().and_then(unhex) {
            Some(payload) => payload,
            None => {
                problem(&mut receipt.problems, "timing payload is not canonical hex");
                continue;
            }
        };
        match row["control_kind"].as_u64() {
            Some(value) if value == MessageKind::HourlyServiceBatch as u64 => {
                match HourlyServiceBatchV1::decode(&payload) {
                    Ok(batch)
                        if last_batch.checked_add(1) == Some(batch.batch_id)
                            && receipt.samples.len() + batch.samples.len() <= SAMPLE_LIMIT =>
                    {
                        last_batch = batch.batch_id;
                        receipt.samples.extend(batch.samples);
                    }
                    _ => problem(
                        &mut receipt.problems,
                        "timing batch is invalid, duplicated, missing or exceeds capacity",
                    ),
                }
            }
            Some(value) if value == MessageKind::HourlyServiceCensus as u64 => {
                match HourlyServiceCensusV1::decode(&payload) {
                    Ok(census)
                        if census.last_batch_id == last_batch && receipt.census.is_none() =>
                    {
                        receipt.census = Some(census)
                    }
                    _ => problem(
                        &mut receipt.problems,
                        "terminal timing census is invalid or does not cover the received batches",
                    ),
                }
            }
            _ => problem(
                &mut receipt.problems,
                "receipt contains an unknown control message kind",
            ),
        }
    }
    if !receipt_complete {
        problem(
            &mut receipt.problems,
            "host timing receipt is missing its final custody marker",
        );
    }
    if receipt.census.is_none() {
        problem(
            &mut receipt.problems,
            "terminal timing census missing; final hourly tail is not proven",
        );
    }
    receipt
}

#[derive(Default)]
struct RawJoin {
    data_frames: u64,
    gap_only_frames: u64,
    refused_frames: u64,
    partial_frames: u64,
    joined_samples: BTreeSet<usize>,
    streams: Vec<Value>,
    problems: Vec<String>,
}

fn join_stream(
    reader: &mut impl Read,
    stream_id: u64,
    samples: &[HourlyServiceSampleV1],
    join: &mut RawJoin,
) -> Result<(u64, String), String> {
    let mut ranges = samples
        .iter()
        .enumerate()
        .filter_map(|(index, sample)| {
            (sample.owner.logical_stream_id == stream_id)
                .then_some(sample.data_sequence)
                .flatten()
                .map(|(first, after)| (first, after, index))
        })
        .collect::<Vec<_>>();
    ranges.sort_unstable();
    for pair in ranges.windows(2) {
        if pair[0].1 > pair[1].0 {
            problem(&mut join.problems, "DATA timing sequence ranges overlap");
        }
        let prior = samples[pair[0].2].owner;
        let next = samples[pair[1].2].owner;
        if next.epoch < prior.epoch
            || (next.epoch == prior.epoch
                && (next.game_hour_raw <= prior.game_hour_raw
                    || next.observation_ordinal <= prior.observation_ordinal))
        {
            problem(
                &mut join.problems,
                "original request ownership contradicts committed DATA order",
            );
        }
    }
    let mut range_index = 0;
    let mut previous_sequence = None;
    let mut frame_first = None;
    let mut frame_hour = None;
    let mut frame_reader_records = 0u64;
    let mut frame_gaps = 0u64;
    let mut frame_owner = None;
    let mut frame_unowned = false;
    let mut digest = Sha256::new();
    let mut decoded_bytes = 0u64;
    loop {
        let mut header_bytes = [0u8; compact_contract::COMPACT_RECORD_HEADER_LEN];
        let mut read = 0;
        while read < header_bytes.len() {
            let count = reader
                .read(&mut header_bytes[read..])
                .map_err(|error| error.to_string())?;
            if count == 0 {
                break;
            }
            read += count;
        }
        if read == 0 {
            break;
        }
        if read != header_bytes.len() {
            return Err("truncated raw frame header".into());
        }
        let header = compact_contract::parse_compact_record_header(&header_bytes)
            .map_err(|error| error.to_string())?;
        digest.update(&header_bytes);
        decoded_bytes = decoded_bytes
            .checked_add(header_bytes.len() as u64)
            .ok_or("raw byte count overflow")?;
        let identity_origin = previous_sequence.is_none()
            && header.sequence == 0
            && header.domain_id
                == compact_contract::campaign_identity::COMPACT_DOMAIN_STREAM_HEADER
            && header.record_type
                == compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;
        if previous_sequence.map_or(header.sequence != 1 && !identity_origin, |last: u64| {
            last.checked_add(1) != Some(header.sequence)
        }) {
            return Err("raw record sequence is not contiguous from its canonical origin".into());
        }
        if header.record_type
            == compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER
            && !identity_origin
        {
            return Err("raw CID1 header is not the sequence-zero stream origin".into());
        }
        if identity_origin
            && header.payload_len > compact_contract::campaign_identity::CID1_MAX_PAYLOAD_LEN
        {
            return Err("raw CID1 payload exceeds its canonical bound".into());
        }
        let mut identity_payload = identity_origin.then(|| Vec::with_capacity(header.payload_len));
        previous_sequence = Some(header.sequence);
        let mut prefix = [0u8; 32];
        let mut buffer = [0u8; 65536];
        let mut remaining = header.payload_len;
        let mut payload_read = 0;
        while remaining != 0 {
            let count = remaining.min(buffer.len());
            reader
                .read_exact(&mut buffer[..count])
                .map_err(|error| format!("truncated raw payload: {error}"))?;
            if payload_read < prefix.len() {
                let copied = count.min(prefix.len() - payload_read);
                prefix[payload_read..payload_read + copied].copy_from_slice(&buffer[..copied]);
            }
            digest.update(&buffer[..count]);
            if let Some(payload) = identity_payload.as_mut() {
                payload.extend_from_slice(&buffer[..count]);
            }
            remaining -= count;
            payload_read += count;
            decoded_bytes = decoded_bytes
                .checked_add(count as u64)
                .ok_or("raw byte count overflow")?;
        }
        if let Some(payload) = identity_payload {
            compact_contract::campaign_identity::parse_cid1_record(&header, &payload)
                .map_err(|error| format!("raw CID1 origin is invalid: {error}"))?;
        }
        while range_index < ranges.len() && header.sequence >= ranges[range_index].1 {
            range_index += 1;
        }
        let owner = ranges
            .get(range_index)
            .filter(|range| header.sequence >= range.0 && header.sequence < range.1)
            .map(|range| range.2);
        let is_reader = matches!(
            header.record_type,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF
                | compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA
                | compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT
        );
        let is_end = header.record_type == compact_contract::COMPACT_RECORD_TYPE_FRAME_END;
        let is_gap = header.record_type == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP;
        if is_reader || is_end || is_gap {
            frame_first.get_or_insert(header.sequence);
            if frame_hour
                .replace(header.game_hour_raw)
                .is_some_and(|hour| hour != header.game_hour_raw)
            {
                problem(
                    &mut join.problems,
                    "raw frame contains multiple hourly labels",
                );
            }
            if let Some(index) = owner {
                if frame_owner
                    .replace(index)
                    .is_some_and(|prior| prior != index)
                {
                    problem(
                        &mut join.problems,
                        "one raw DATA frame joins multiple timing owners",
                    );
                }
            } else {
                frame_unowned = true;
            }
        }
        if is_reader && header.domain_id != 250 {
            frame_reader_records += 1;
        }
        if is_gap {
            frame_gaps += 1;
        }
        if header.record_type == compact_contract::COMPACT_RECORD_TYPE_FRAME_REFUSED {
            join.refused_frames += 1;
            if frame_owner.is_some() {
                problem(
                    &mut join.problems,
                    "committed DATA timing range contains a FRAME_REFUSED outcome",
                );
            }
            frame_first = None;
            frame_hour = None;
            frame_reader_records = 0;
            frame_gaps = 0;
            frame_owner = None;
            frame_unowned = false;
        }
        if is_end {
            if header.payload_len != 32 {
                return Err("FRAME_END timing join requires the canonical 32-byte payload".into());
            }
            let flags = u32::from_le_bytes(prefix[24..28].try_into().unwrap());
            let gap_only = frame_reader_records == 0;
            if gap_only {
                join.gap_only_frames += 1;
            } else {
                join.data_frames += 1;
            }
            if flags != 0 || frame_gaps != 0 {
                join.partial_frames += 1;
            }
            if let Some(index) = frame_owner {
                let sample = &samples[index];
                let expected_bounds = frame_first.zip(header.sequence.checked_add(1));
                if sample.data_sequence != expected_bounds
                    || sample.owner.game_hour_raw != header.game_hour_raw
                    || frame_unowned
                {
                    problem(&mut join.problems, "timing owner does not match the entire original raw DATA sequence span and hour");
                } else if !join.joined_samples.insert(index) {
                    problem(
                        &mut join.problems,
                        "timing sample joins more than one raw DATA frame",
                    );
                }
            } else if !gap_only {
                problem(
                    &mut join.problems,
                    "raw DATA frame has no complete hourly timing measurement",
                );
            }
            frame_first = None;
            frame_hour = None;
            frame_reader_records = 0;
            frame_gaps = 0;
            frame_owner = None;
            frame_unowned = false;
        }
    }
    if frame_first.is_some() {
        problem(
            &mut join.problems,
            "raw reader records remain without a terminal FRAME_END",
        );
    }
    Ok((decoded_bytes, hex(&digest.finalize())))
}

fn raw_join(streams: &[(u64, PathBuf)], samples: &[HourlyServiceSampleV1]) -> RawJoin {
    let mut join = RawJoin::default();
    let mut ids = BTreeSet::new();
    for (id, path) in streams {
        if !ids.insert(*id) {
            problem(&mut join.problems, "raw logical stream id duplicated");
            continue;
        }
        let result = compact_stream::open(path)
            .map_err(|error| error.to_string())
            .and_then(|mut reader| join_stream(&mut reader, *id, samples, &mut join));
        match result {
            Ok((bytes, sha256)) => join.streams.push(
                json!({"logical_stream_id":id,"decoded_bytes":bytes,"decoded_sha256":sha256}),
            ),
            Err(error) => problem(
                &mut join.problems,
                format!("raw logical stream {id}: {error}"),
            ),
        }
    }
    for (index, sample) in samples.iter().enumerate() {
        if !ids.contains(&sample.owner.logical_stream_id) {
            problem(
                &mut join.problems,
                "timing owner names an unknown raw logical stream",
            );
        }
        if sample.data_sequence.is_some() && !join.joined_samples.contains(&index) {
            problem(
                &mut join.problems,
                "timing DATA sequence span has no exact raw frame join",
            );
        }
    }
    join
}

fn service_values(samples: &[HourlyServiceSampleV1], problems: &mut Vec<String>) -> Vec<u64> {
    let mut owners = BTreeSet::new();
    let mut hours = BTreeSet::new();
    let mut order = BTreeMap::new();
    let mut values = Vec::new();
    for sample in samples {
        let owner = (
            sample.owner.logical_stream_id,
            sample.owner.epoch,
            sample.owner.game_hour_raw,
            sample.owner.observation_ordinal,
        );
        if !owners.insert(owner) {
            problem(problems, "duplicate original timing request owner");
        }
        if !hours.insert((owner.0, owner.1, owner.2)) {
            problem(
                problems,
                "multiple timing request owners name one game hour",
            );
        }
        if order.insert((owner.0, owner.1, owner.3), owner.2).is_some() {
            problem(problems, "duplicate original observation ordinal");
        }
        if sample.fault_bits != 0
            || sample.closed_stages != STAGE_MASK
            || sample.observed_stages | sample.not_applicable_stages != STAGE_MASK
            || sample.observed_stages & sample.not_applicable_stages != 0
            || (matches!(
                sample.outcome,
                HourlyServiceOutcomeV1::Complete | HourlyServiceOutcomeV1::Partial
            ) && sample.not_applicable_stages & !TERMINAL_STAGE != 0)
        {
            problem(
                problems,
                "hourly measurement has a fault or an unclosed/unobserved required stage",
            );
        }
        let mut sum = Some(0u64);
        let mut maximum = 0;
        for (index, value) in sample.stage_ns.iter().enumerate() {
            if value.is_some() != (sample.observed_stages & (1 << index) != 0) {
                problem(
                    problems,
                    "hourly stage presence does not match its observed mask",
                );
            }
            if let Some(value) = value {
                sum = sum.and_then(|sum| sum.checked_add(*value));
                maximum = maximum.max(*value);
            }
        }
        match sample.observed_service_ns {
            Some(value) => {
                if sum.is_none_or(|sum| value > sum) || value < maximum {
                    problem(
                        problems,
                        "hourly service union is inconsistent with the stage unions",
                    );
                }
                values.push(value);
            }
            None => problem(
                problems,
                "hourly measurement has no observed service duration",
            ),
        }
        match sample.outcome {
            HourlyServiceOutcomeV1::Complete | HourlyServiceOutcomeV1::Partial
                if sample.data_sequence.is_none() =>
            {
                problem(
                    problems,
                    "completed DATA outcome has no committed DATA sequence span",
                )
            }
            HourlyServiceOutcomeV1::Refused if sample.data_sequence.is_some() => {
                problem(problems, "no-DATA outcome claims committed DATA")
            }
            _ => {}
        }
        if let (Some(publication), Some(completion)) =
            (sample.edge_to_publication_ns, sample.edge_to_completion_ns)
        {
            if publication > completion {
                problem(problems, "publication latency follows request completion");
            }
        }
    }
    let mut previous = None;
    for ((stream, epoch, ordinal), hour) in order {
        if let Some((prior_stream, prior_epoch, prior_ordinal, prior_hour)) = previous {
            if stream == prior_stream
                && epoch == prior_epoch
                && (ordinal <= prior_ordinal || hour <= prior_hour)
            {
                problem(problems, "original request ordinals and hours are not strictly monotonic within an epoch");
            }
        }
        previous = Some((stream, epoch, ordinal, hour));
    }
    values
}

fn ceil_us(ns: u64) -> u64 {
    ns / 1000 + u64::from(ns % 1000 != 0)
}

fn timing_summary(values: &[u64]) -> Value {
    if values.is_empty() {
        return json!({"count":0,"avg_ns_ceil":null,"p95_ns":null,"p99_ns":null,"max_ns":null,"over_16ms_count":0});
    }
    let mut ordered = values.to_vec();
    ordered.sort_unstable();
    let sum = values
        .iter()
        .try_fold(0u64, |sum, value| sum.checked_add(*value));
    let count = values.len() as u64;
    let average = sum.map(|sum| sum / count + u64::from(sum % count != 0));
    let percentile = |percent: usize| ordered[(ordered.len() * percent).div_ceil(100) - 1];
    let maximum = ordered[ordered.len() - 1];
    json!({"count":count,"total_ns":sum,"avg_ns_ceil":average,"avg_us_ceil":average.map(ceil_us),
        "p95_ns":percentile(95),"p95_us_ceil":ceil_us(percentile(95)),"p99_ns":percentile(99),"p99_us_ceil":ceil_us(percentile(99)),
        "max_ns":maximum,"max_us_ceil":ceil_us(maximum),"over_16ms_count":values.iter().filter(|value| **value > SERVICE_LIMIT_NS).count()})
}

fn validate_census(
    census: &HourlyServiceCensusV1,
    samples: &[HourlyServiceSampleV1],
    problems: &mut Vec<String>,
) {
    let count = samples.len() as u64;
    if !census.sealed
        || census.offered_requests != count
        || census.admitted_requests != count
        || census.closed_requests != count
        || census.queued_measurements != count
        || census.complete_measurements != count
        || census.incomplete_measurements != 0
        || census.lost_measurements != 0
        || census.unmeasured_hours != 0
        || census.active_requests != 0
        || census.pending_delivery != 0
        || census.fault_bits != 0
        || census.unattributed_service_ns != 0
    {
        problem(problems, "terminal census does not prove one closed and received complete measurement for every offered request");
    }
    let partial = samples
        .iter()
        .filter(|sample| sample.outcome == HourlyServiceOutcomeV1::Partial)
        .count() as u64;
    let refused = samples
        .iter()
        .filter(|sample| sample.outcome == HourlyServiceOutcomeV1::Refused)
        .count() as u64;
    let aborted = samples
        .iter()
        .filter(|sample| sample.outcome == HourlyServiceOutcomeV1::Aborted)
        .count() as u64;
    let mut complete = 0u64;
    let mut total = Some(0u64);
    let mut maximum = None;
    let mut over = 0u64;
    for sample in samples {
        if sample.fault_bits != 0 {
            continue;
        }
        let Some(value) = sample.observed_service_ns else {
            continue;
        };
        complete += 1;
        total = total.and_then(|total| total.checked_add(value));
        maximum = Some(maximum.map_or(value, |old: u64| old.max(value)));
        over += u64::from(value > SERVICE_LIMIT_NS);
    }
    if census.partial_requests != partial
        || census.refused_requests != refused
        || census.aborted_requests != aborted
        || total != Some(census.total_service_ns)
        || census.complete_measurements != complete
        || census.incomplete_measurements != count - complete
        || census.maximum_service_ns != maximum
        || census.over_limit_measurements != over
    {
        problem(
            problems,
            "terminal census totals or outcome counts differ from the raw received samples",
        );
    }
}

fn census_json(census: &HourlyServiceCensusV1) -> Value {
    json!({"last_batch_id":census.last_batch_id,"offered_requests":census.offered_requests,"admitted_requests":census.admitted_requests,
        "closed_requests":census.closed_requests,"queued_measurements":census.queued_measurements,"lost_measurements":census.lost_measurements,
        "complete_measurements":census.complete_measurements,"incomplete_measurements":census.incomplete_measurements,"partial_requests":census.partial_requests,
        "refused_requests":census.refused_requests,"aborted_requests":census.aborted_requests,"unmeasured_hours":census.unmeasured_hours,
        "over_limit_measurements":census.over_limit_measurements,"total_service_ns":census.total_service_ns,"maximum_service_ns":census.maximum_service_ns,
        "active_requests":census.active_requests,"pending_delivery":census.pending_delivery,"unattributed_service_ns":census.unattributed_service_ns,
        "diagnostic_transport_ns":census.diagnostic_transport_ns,"terminal_lifecycle_ns":census.terminal_lifecycle_ns,"fault_bits":census.fault_bits,"sealed":census.sealed})
}

pub fn unavailable_report(cause: &str) -> Value {
    json!({"schema":"hoi4.hourly_service.v1","available":false,"timing_population_complete":false,"complete_capture":false,"passes_16ms":false,
        "ceiling_ns":SERVICE_LIMIT_NS,"timing_within_16ms":false,"terminal_lifecycle_ns":null,"problems":[cause],"legacy_frame_and_lane_timings_are_separate":true})
}

pub fn report_for_capture(
    receipt_path: &Path,
    expected: &HourlyServiceBinding,
    streams: &[(u64, PathBuf)],
    capture_complete: bool,
) -> Value {
    let mut receipt = load_receipt(receipt_path, expected);
    if receipt.samples.is_empty() && receipt.census.is_none() {
        let mut report = unavailable_report("authenticated H4CT timing receipt unavailable");
        report["problems"] = json!(receipt.problems);
        report["receipt_sha256"] = json!(receipt.sha256);
        return report;
    }
    let values = service_values(&receipt.samples, &mut receipt.problems);
    if receipt.samples.is_empty() {
        problem(
            &mut receipt.problems,
            "no hourly service samples were received",
        );
    }
    if let Some(census) = receipt.census.as_ref() {
        validate_census(census, &receipt.samples, &mut receipt.problems);
    }
    let join = raw_join(streams, &receipt.samples);
    for error in &join.problems {
        problem(&mut receipt.problems, error.clone());
    }
    let timing_complete = receipt.problems.is_empty();
    let complete_capture = capture_complete
        && join.gap_only_frames == 0
        && join.refused_frames == 0
        && join.partial_frames == 0
        && receipt
            .samples
            .iter()
            .all(|sample| sample.outcome == HourlyServiceOutcomeV1::Complete);
    let timing_within_limit =
        timing_complete && values.iter().all(|value| *value <= SERVICE_LIMIT_NS);
    let passes = timing_within_limit && complete_capture;
    let stages = (0..7)
        .map(|index| {
            timing_summary(
                &receipt
                    .samples
                    .iter()
                    .filter_map(|sample| sample.stage_ns[index])
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    json!({"schema":"hoi4.hourly_service.v1","available":receipt.census.is_some(),"binding":expected,"receipt_file":RECEIPT_FILE_NAME,"receipt_sha256":receipt.sha256,
        "timing_population_complete":timing_complete,"complete_capture":complete_capture,"timing_within_16ms":timing_within_limit,"passes_16ms":passes,"ceiling_ns":SERVICE_LIMIT_NS,
        "service":timing_summary(&values),"stages":stages,"stage_order":["capture","finish","build_tail","publication_prepare","sink","commit","terminal"],"census":receipt.census.as_ref().map(census_json),"edge_to_publication":timing_summary(&receipt.samples.iter().filter_map(|sample|sample.edge_to_publication_ns).collect::<Vec<_>>()),
        "edge_to_completion":timing_summary(&receipt.samples.iter().filter_map(|sample|sample.edge_to_completion_ns).collect::<Vec<_>>()),
        "raw_data_frames":join.data_frames,"raw_gap_only_frames":join.gap_only_frames,"raw_refused_frames":join.refused_frames,"raw_partial_frames":join.partial_frames,
        "raw_joined_samples":join.joined_samples.len(),"raw_streams":join.streams,"problems":receipt.problems,
        "diagnostic_transport_ns":receipt.census.as_ref().map(|census|census.diagnostic_transport_ns),
        "terminal_lifecycle_ns":receipt.census.as_ref().and_then(|census|census.terminal_lifecycle_ns),
        "unattributed_service_ns":receipt.census.as_ref().map(|census|census.unattributed_service_ns),
        "owner_ordinal_authority":"authenticated collector assertion; stream/hour/committed DATA bounds independently joined to raw H4CR",
        "scope":"owned recorder active service union includes preselection work and final completion; optional edge latencies have a separate origin; diagnostic transport excludes the final census exchange",
        "legacy_frame_and_lane_timings_are_separate":true})
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 15) as usize] as char);
    }
    out
}

fn unhex(value: &str) -> Option<Vec<u8>> {
    if value.len() % 2 != 0 {
        return None;
    }
    let nibble = |byte: u8| match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    };
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| Some(nibble(pair[0])? * 16 + nibble(pair[1])?))
        .collect()
}

#[cfg(test)]
mod tests;
