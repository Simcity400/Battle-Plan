use super::{Frames, Recording, SharedView, TableId, DecodedFrame};

pub fn read_on(file: &Recording, tables: &[TableId]) -> Vec<(DecodedFrame, Vec<Option<i32>>)> {
    let mut frames = Frames::new(file, tables).unwrap();
    let mut out = Vec::new();
    for run in 0..file.run_count() {
        let held = file.run(run).unwrap();
        for ordinal in 0..held.ordinals {
            frames.go_to(run, ordinal).unwrap();
            let frame = frames.frame(held.branch).unwrap();
            let sampled = frame.tables.iter().map(SharedView::sampled_at).collect();
            out.push((frame, sampled));
        }
    }
    out
}
