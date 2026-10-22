//! Prefix bounds, equal time sampling and measured growth comparisons.
use super::*;
use replay::container::BlockIndex;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(super) fn parse(value: &str) -> Vec<u32> {
    let hours: Vec<u32> = value
        .split(',')
        .map(|hour| {
            hour.trim()
                .parse()
                .expect("prefix hours must be positive integers")
        })
        .collect();
    assert!(
        hours.iter().all(|&hour| hour > 0),
        "prefix hours must be positive"
    );
    assert!(
        hours.windows(2).all(|pair| pair[0] < pair[1]),
        "prefix hours must increase strictly"
    );
    hours
}

/// Prefix hours are elapsed from the first recorded hour, including that hour.
pub(super) fn blocks(index: &[BlockIndex], hours: Option<u32>) -> usize {
    let first = index.first().expect("measurement recording has no blocks");
    let Some(hours) = hours else {
        return index.len();
    };
    assert!(hours > 0);
    let through = i64::from(first.first_hour) + i64::from(hours) - 1;
    index
        .iter()
        .position(|block| i64::from(block.last_hour) >= through)
        .expect("recording ends before requested prefix")
        + 1
}

pub(super) fn ranges(index: &[BlockIndex]) -> BTreeMap<BranchId, (Hour, Hour)> {
    let mut ranges = BTreeMap::new();
    for block in index {
        let range = ranges
            .entry(block.branch)
            .or_insert((block.first_hour, block.last_hour));
        range.0 = range.0.min(block.first_hour);
        range.1 = range.1.max(block.last_hour);
    }
    ranges
}

pub(super) fn targets(index: &[BlockIndex]) -> BTreeSet<(BranchId, Hour)> {
    let mut targets = BTreeSet::new();
    for (branch, (first, last)) in ranges(index) {
        for sample in 0..=40 {
            let hour = i64::from(first) + (i64::from(last) - i64::from(first)) * sample / 40;
            targets.insert((branch, Hour::try_from(hour).unwrap()));
        }
    }
    targets
}

pub(super) fn production(row: &Value) -> &Value {
    row["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["stride"] == row["productionStride"])
        .unwrap()
}

fn ratio(current: &Value, previous: &Value) -> Option<f64> {
    let (current, previous) = (current.as_f64()?, previous.as_f64()?);
    (previous > 0.0).then(|| current / previous)
}

pub(super) fn growth(current: &Value, previous: Option<&Value>) -> Value {
    let Some(previous) = previous else {
        return Value::Null;
    };
    let (now, before) = (production(current), production(previous));
    let hours = ratio(&current["recordedFrames"], &previous["recordedFrames"]).unwrap();
    let mut representations = serde_json::Map::new();
    for field in [
        "totalMapStoreBytes",
        "checkpointBytes",
        "updateBytes",
        "resumeBytes",
        "checkpointOnlyStoreBytes",
        "recordingKeyframesOnlyStoreBytes",
    ] {
        let bytes = ratio(&now[field], &before[field]);
        representations.insert(
            field.into(),
            json!({"bytesRatio": bytes, "bytesPerHourRatio": bytes.map(|bytes| bytes / hours)}),
        );
    }
    let mut tables = Vec::new();
    for table in current["tableAttribution"]["tables"].as_array().unwrap() {
        let prior = previous["tableAttribution"]["tables"]
            .as_array()
            .unwrap()
            .iter()
            .find(|prior| prior["table"] == table["table"])
            .unwrap();
        let bytes = ratio(
            &table["independentNativeRowZstd9Bytes"],
            &prior["independentNativeRowZstd9Bytes"],
        );
        tables.push(json!({"table": table["table"],
            "independentNativeRowZstd9Bytes": table["independentNativeRowZstd9Bytes"],
            "additionalIndependentNativeRowZstd9Bytes": table["independentNativeRowZstd9Bytes"].as_f64().unwrap() - prior["independentNativeRowZstd9Bytes"].as_f64().unwrap(),
            "bytesRatio": bytes, "bytesPerHourRatio": bytes.map(|bytes| bytes / hours),
            "rawBytesRatio": ratio(&table["nativeRowRawBytes"], &prior["nativeRowRawBytes"]),
            "sourceLaterBytesRatio": ratio(&table["recordingLaterBytes"], &prior["recordingLaterBytes"])}));
    }
    tables.sort_by(|left, right| {
        right["additionalIndependentNativeRowZstd9Bytes"]
            .as_f64()
            .unwrap()
            .total_cmp(
                &left["additionalIndependentNativeRowZstd9Bytes"]
                    .as_f64()
                    .unwrap(),
            )
    });
    let mut source = Vec::new();
    for trial in now["recordingReadCandidate"]["trials"].as_array().unwrap() {
        if let Some(prior) = before["recordingReadCandidate"]["trials"]
            .as_array()
            .unwrap()
            .iter()
            .find(|prior| prior["workers"] == trial["workers"])
        {
            source.push(json!({"workers": trial["workers"],
                "keyframeSeekP95Ratio": ratio(&trial["seekRecordingKeyframe"]["p95Us"], &prior["seekRecordingKeyframe"]["p95Us"]),
                "stepP95Ratio": ratio(&trial["stepRetainedBlock"]["p95Us"], &prior["stepRetainedBlock"]["p95Us"]),
                "blockLoadP95Ratio": ratio(&trial["stepBlockLoad"]["p95Us"], &prior["stepBlockLoad"]["p95Us"])}));
        }
    }
    let mut strides = Vec::new();
    for candidate in current["candidates"].as_array().unwrap() {
        if let Some(prior) = previous["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|prior| prior["stride"] == candidate["stride"])
        {
            let bytes = ratio(
                &candidate["totalMapStoreBytes"],
                &prior["totalMapStoreBytes"],
            );
            strides.push(json!({"stride": candidate["stride"], "bytesRatio": bytes,
                "bytesPerHourRatio": bytes.map(|bytes| bytes / hours),
                "seekP50Ratio": ratio(&candidate["seekEmptyDecoder"]["p50Us"], &prior["seekEmptyDecoder"]["p50Us"]),
                "seekP95Ratio": ratio(&candidate["seekEmptyDecoder"]["p95Us"], &prior["seekEmptyDecoder"]["p95Us"]),
                "stepP95Ratio": ratio(&candidate["stepRetainedBlock"]["p95Us"], &prior["stepRetainedBlock"]["p95Us"]),
                "blockLoadP95Ratio": ratio(&candidate["stepBlockLoad"]["p95Us"], &prior["stepBlockLoad"]["p95Us"])}));
        }
    }
    let mut result = json!({"previousRequestedPrefixHours": previous["requestedPrefixHours"], "recordedHoursRatio": hours,
        "roundedElapsedHoursRatio": ratio(&current["roundedElapsedHours"], &previous["roundedElapsedHours"]),
        "representations": representations, "tablesByAdditionalBytes": tables, "sourceReads": source,
        "preparedSeekP50Ratio": ratio(&now["seekEmptyDecoder"]["p50Us"], &before["seekEmptyDecoder"]["p50Us"]),
        "preparedSeekP95Ratio": ratio(&now["seekEmptyDecoder"]["p95Us"], &before["seekEmptyDecoder"]["p95Us"]),
        "preparedStepP50Ratio": ratio(&now["step"]["p50Us"], &before["step"]["p50Us"]),
        "preparedStepP95Ratio": ratio(&now["step"]["p95Us"], &before["step"]["p95Us"]),
        "peakPrivateBytesRatio": ratio(&current["peakPrivateBytes"], &previous["peakPrivateBytes"]),
        "scope": "same recording and production stride; bytesPerHourRatio above 1 is faster than linear growth; table independent compression is attribution, not additive Store bytes"});
    result["strides"] = json!(strides);
    result
}
