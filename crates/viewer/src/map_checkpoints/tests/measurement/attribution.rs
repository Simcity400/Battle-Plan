//! Native checkpoint row attribution and recording section sizes, test only.
use super::*;
use replay::container::Section;
use std::collections::BTreeMap;

#[derive(Default)]
struct Native {
    checkpoints: u64,
    rows: u64,
    raw: u64,
    compressed: u64,
}

#[derive(Default)]
pub(super) struct Attribution {
    checkpoints: BTreeMap<u32, BTreeMap<TableId, Native>>,
    source: BTreeMap<TableId, (u64, u64)>,
    elapsed_us: u64,
}

impl Attribution {
    pub(super) fn observe_checkpoint(
        &mut self,
        stride: u32,
        frame: &DecodedFrame,
        selected: &[TableId],
    ) {
        let started = Instant::now();
        let totals = self.checkpoints.entry(stride).or_default();
        for &table in selected {
            let total = totals.entry(table).or_default();
            let mut bytes = Vec::new();
            match &*frame.tables[table.index()] {
                TableView::Complete(rows) | TableView::Truncated { rows, .. } => {
                    rows.encode_checkpoint_rows(&mut bytes).unwrap();
                    total.rows += rows.len() as u64;
                }
                TableView::Unavailable { reason } => bytes.extend_from_slice(reason.as_bytes()),
            }
            total.checkpoints += 1;
            total.raw += bytes.len() as u64;
            total.compressed += zstd::bulk::compress(&bytes, 9).unwrap().len() as u64;
        }
        self.elapsed_us += micros(started);
    }

    pub(super) fn source(&mut self, replay: &Replay, blocks: usize, selected: &[TableId]) {
        for &table in selected {
            self.source.entry(table).or_default();
        }
        for block in replay.index.iter().take(blocks) {
            for entry in replay.block_sections(block).unwrap().unwrap_or_default() {
                match entry.section {
                    Section::Keyframe(table) => {
                        if let Some(total) = self.source.get_mut(&table) {
                            total.0 += entry.compressed;
                        }
                    }
                    Section::Later(table) => {
                        if let Some(total) = self.source.get_mut(&table) {
                            total.1 += entry.compressed;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    pub(super) fn report(&self, evidence: &Path, selected: &[TableId]) -> serde_json::Value {
        let mut csv = BufWriter::new(File::create(evidence.join("table-checkpoints.csv")).unwrap());
        writeln!(csv, "stride,table,checkpoints,rows,native_row_raw_bytes,independent_native_row_zstd9_bytes,recording_keyframe_bytes,recording_later_bytes").unwrap();
        let mut candidates = Vec::new();
        let mut production = Vec::new();
        for (&stride, totals) in &self.checkpoints {
            let mut tables = Vec::new();
            for &table in selected {
                let total = &totals[&table];
                let (keyframe, later) = self.source[&table];
                writeln!(
                    csv,
                    "{stride},{table:?},{},{},{},{},{keyframe},{later}",
                    total.checkpoints, total.rows, total.raw, total.compressed
                )
                .unwrap();
                tables.push(serde_json::json!({"table": format!("{table:?}"),
                    "checkpoints": total.checkpoints, "rows": total.rows,
                    "nativeRowRawBytes": total.raw, "independentNativeRowZstd9Bytes": total.compressed,
                    "recordingKeyframeBytes": keyframe, "recordingLaterBytes": later}));
            }
            if stride == dashboard_base::prepared_map::CHECKPOINT_STRIDE {
                production = tables.clone();
            }
            candidates.push(serde_json::json!({"stride": stride, "tables": tables}));
        }
        csv.flush().unwrap();
        serde_json::json!({"tables": production, "checkpointCandidates": candidates,
            "attributionUs": self.elapsed_us,
            "scope": "native row payloads compressed independently by table for attribution; excludes table availability/sampling headers and is not additive attribution of a whole compressed checkpoint; recording section bytes are actual selected sections, stored only in the source recording"})
    }
}
