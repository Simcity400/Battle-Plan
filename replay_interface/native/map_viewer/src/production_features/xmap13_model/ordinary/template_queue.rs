use crate::Result;

// 141919100: the model payload (+0x40) survives independently of the outer
// priority (+0x00 in a queue entry). Entity-token bytes own every dedup decision.
#[derive(Debug, PartialEq)]
pub(super) struct ModelRow<T> {
    pub label: String,
    pub token: String,
    pub payload_raw: i64,
    pub entity: Option<T>,
}

struct QueueEntry<T> {
    priority_raw: i64,
    row: ModelRow<T>,
}

pub(super) struct TemplateQueues<T> {
    queues: [Vec<QueueEntry<T>>; 3],
}

impl<T> Default for TemplateQueues<T> {
    fn default() -> Self {
        Self { queues: std::array::from_fn(|_| Vec::new()) }
    }
}

fn reserve_row<T>(rows: &mut Vec<T>, refusal: &'static str) -> Result<()> {
    rows.len().checked_add(1).ok_or("XMAP13 template queue row count overflow")?;
    rows.try_reserve(1).map_err(|_| refusal)?;
    Ok(())
}

fn copy_text(source: &str, refusal: &'static str) -> Result<String> {
    let mut output = String::new();
    output.try_reserve(source.len()).map_err(|_| refusal)?;
    output.push_str(source);
    Ok(output)
}

impl<T: Clone> ModelRow<T> {
    fn try_copy(&self) -> Result<Self> {
        Ok(Self {
            label: copy_text(&self.label, "XMAP13 template model label allocation refused")?,
            token: copy_text(&self.token, "XMAP13 template model token allocation refused")?,
            payload_raw: self.payload_raw,
            entity: self.entity.clone(),
        })
    }
}

// 1419197df..141919854 reads the low dword for BOTH nested traversals.
pub(super) fn outer_bits(mask: u64) -> impl Iterator<Item = u32> {
    let mut remaining = mask as u32;
    std::iter::from_fn(move || {
        if remaining == 0 { return None; }
        let bit = remaining & remaining.wrapping_neg();
        remaining &= remaining - 1;
        Some(bit)
    })
}

fn validate_outer_bit(outer_bit: u32) -> Result<()> {
    if !outer_bit.is_power_of_two() {
        return Err("XMAP13 template outer category is not one low-32 bit".into());
    }
    Ok(())
}

// Unlike the nested traversal, 141919c25 compares the FULL subunit qword.
// Return an index in the caller's three persistent suffix ModelLists (q3..5).
pub(super) fn suffix_bucket(
    outer_bit: u32, subunit_mask: u64, strict: bool, visual_present: bool,
) -> Option<usize> {
    if subunit_mask == u64::from(outer_bit) { Some(0) }
        else if strict { None }
        else if visual_present { Some(1) }
        else { Some(2) }
}

impl<T: Clone> TemplateQueues<T> {
    // This is one OUTER iteration; the caller interleaves the existing suffix
    // loader immediately afterward, including repeated qualifying variants.
    pub(super) fn admit_variant_for_bit(
        &mut self, outer_bit: u32, subunit_mask: u64, strict: bool,
        visual_priority: Option<i32>, normal: i32, explicit: i32,
        explicit_token: &str, rows: &[ModelRow<T>],
    ) -> Result<()> {
        validate_outer_bit(outer_bit)?;
        for inner_bit in outer_bits(subunit_mask) {
            let bucket = if inner_bit == outer_bit { 0 }
                else if strict { continue }
                else if visual_priority.is_some() { 1 }
                else { 2 };
            for row in rows.iter().rev() {
                // Stored q0..2 tokens implement the one global variant seen set;
                // suffix admissions deliberately never participate in it.
                if self.queues.iter().flatten().any(|entry| entry.row.token == row.token) {
                    continue;
                }
                let priority = if row.token == explicit_token { explicit } else { normal };
                let priority_raw = i64::from(priority) * 100_000;
                let copied = row.try_copy()?;
                let queue = &mut self.queues[bucket];
                reserve_row(queue, "XMAP13 template variant queue allocation refused")?;
                // Signed comparison: 141919a30..141919a63 inserts BEFORE equals.
                let index = queue.partition_point(|entry| entry.priority_raw > priority_raw);
                queue.insert(index, QueueEntry { priority_raw, row: copied });
            }
        }
        Ok(())
    }
}

impl<T> TemplateQueues<T> {
    // The caller retains three ModelLists across every suffix_families call so
    // local dedup also controls the existing helper's preferred fallback arm.
    // 141919d3d..141919f23 flattens in queue memory order, never by a cross-queue
    // priority. The caller then performs byte-rank grouping and label decoration.
    pub(super) fn finish(
        self, default: ModelRow<T>, suffix_queues: [Vec<ModelRow<T>>; 3],
    ) -> Result<Vec<ModelRow<T>>> {
        if !default.token.is_empty() || default.entity.is_some() || default.payload_raw != -100_000 {
            return Err("XMAP13 template default model row is malformed".into());
        }
        let mut output = Vec::new();
        reserve_row(&mut output, "XMAP13 template output allocation refused")?;
        output.push(default);
        for row in self.queues.into_iter().flatten().map(|entry| entry.row)
            .chain(suffix_queues.into_iter().flatten()) {
            if output.iter().any(|previous| previous.token == row.token) { continue; }
            reserve_row(&mut output, "XMAP13 template output allocation refused")?;
            output.push(row);
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(label: &str, token: &str, payload_raw: i64) -> ModelRow<u32> {
        ModelRow { label: label.into(), token: token.into(), payload_raw, entity: Some(7) }
    }

    fn default_row() -> ModelRow<u32> {
        ModelRow { label: "default".into(), token: String::new(), payload_raw: -100_000, entity: None }
    }

    fn tokens(rows: &[ModelRow<u32>]) -> Vec<&str> {
        rows.iter().map(|row| row.token.as_str()).collect()
    }

    #[test]
    fn earlier_inner_mismatch_owns_token_before_later_match() {
        let mut queues = TemplateQueues::default();
        queues.admit_variant_for_bit(2, 3, false, None, 5, 9, "", &[row("mismatch", "A", 33)]).unwrap();
        assert!(queues.queues[0].is_empty());
        assert_eq!(queues.queues[2][0].row.label, "mismatch");
        queues.admit_variant_for_bit(2, 2, false, None, 100, 100, "", &[row("later match", "A", 99)]).unwrap();
        assert!(queues.queues[0].is_empty(), "global variant dedup never moves a previously admitted row");
        let output = queues.finish(default_row(), Default::default()).unwrap();
        assert_eq!((output[1].label.as_str(), output[1].payload_raw), ("mismatch", 33));

        let mut strict = TemplateQueues::default();
        strict.admit_variant_for_bit(2, 3, true, None, 5, 9, "", &[row("match", "A", 33)]).unwrap();
        assert_eq!(strict.queues[0].len(), 1);
        assert!(strict.queues[1..3].iter().all(Vec::is_empty));
    }

    #[test]
    fn visual_presence_selects_bucket_one_even_for_zero_and_buckets_outrank_priority() {
        let mut queues = TemplateQueues::default();
        queues.admit_variant_for_bit(2, 1, false, None, i32::MAX, i32::MAX, "",
            &[row("no visual", "Q2", 1)]).unwrap();
        queues.admit_variant_for_bit(2, 3, false, Some(0), 0, 0, "",
            &[row("present zero", "Q1", 2)]).unwrap();
        queues.admit_variant_for_bit(2, 2, true, None, i32::MIN, i32::MIN, "",
            &[row("exact", "Q0", 3)]).unwrap();
        assert_eq!(queues.queues[1][0].row.token, "Q1");
        let output = queues.finish(default_row(), Default::default()).unwrap();
        assert_eq!(tokens(&output), ["", "Q0", "Q1", "Q2"]);
    }

    #[test]
    fn reverse_rows_and_before_equal_restore_inner_order_and_put_later_variant_first() {
        let mut queues = TemplateQueues::default();
        let first = [default_row(), row("same", "A", 1), row("same", "B", 2)];
        queues.admit_variant_for_bit(1, 1, false, None, 4, 4, "A", &first).unwrap();
        assert_eq!(queues.queues[0].iter().map(|entry| entry.row.token.as_str()).collect::<Vec<_>>(), ["", "A", "B"]);
        let second = [default_row(), row("later", "C", 3), row("later", "D", 4)];
        queues.admit_variant_for_bit(1, 1, false, None, 4, 4, "C", &second).unwrap();
        let output = queues.finish(default_row(), Default::default()).unwrap();
        assert_eq!(tokens(&output), ["", "C", "D", "A", "B"]);
        assert_eq!(output[1].entity, Some(7));
    }

    #[test]
    fn priorities_are_signed_and_explicit_token_is_exact_without_replacing_payload() {
        let mut queues = TemplateQueues::default();
        let rows = [row("normal", "Model", 999), row("explicit", "model", -999)];
        queues.admit_variant_for_bit(1, 1, false, None, i32::MIN, i32::MAX, "model", &rows).unwrap();
        assert_eq!(queues.queues[0][0].priority_raw, i64::from(i32::MAX) * 100_000);
        assert_eq!(queues.queues[0][1].priority_raw, i64::from(i32::MIN) * 100_000);
        let output = queues.finish(default_row(), Default::default()).unwrap();
        assert_eq!(tokens(&output), ["", "model", "Model"]);
        assert_eq!((output[1].payload_raw, output[2].payload_raw), (-999, 999));
    }

    #[test]
    fn variant_bucket_two_beats_suffix_bucket_three_even_when_suffix_arrived_first() {
        let mut queues = TemplateQueues::default();
        let suffix = vec![row("suffix", "X", i64::MAX), row("suffix-only", "Y", 0)];
        queues.admit_variant_for_bit(2, 1, false, None, -5, -5, "", &[row("variant", "X", -77)]).unwrap();
        let output = queues.finish(default_row(), [suffix, Vec::new(), Vec::new()]).unwrap();
        assert_eq!(tokens(&output), ["", "X", "Y"]);
        assert_eq!((output[1].label.as_str(), output[1].payload_raw), ("variant", -77));
    }

    #[test]
    fn bit_traversal_is_low_32_but_suffix_equality_uses_all_64() {
        assert_eq!(outer_bits((1u64 << 63) | (1u64 << 31) | 3).collect::<Vec<_>>(), [1, 2, 1u32 << 31]);
        assert_eq!(outer_bits(1u64 << 32).count(), 0);
        assert_eq!(suffix_bucket(1, 1, true, false), Some(0));
        assert_eq!(suffix_bucket(1, (1u64 << 32) | 1, true, false), None);
        assert_eq!(suffix_bucket(1, (1u64 << 32) | 1, false, true), Some(1));
        assert_eq!(suffix_bucket(1, (1u64 << 32) | 1, false, false), Some(2));
        let mut queues = TemplateQueues::default();
        queues.admit_variant_for_bit(1, (1u64 << 32) | 1, true, None, 0, 0, "", &[row("low match", "X", 7)]).unwrap();
        assert_eq!(queues.queues[0].len(), 1);
        queues.admit_variant_for_bit(1, 1u64 << 32, false, None, 0, 0, "", &[row("high only", "Y", 8)]).unwrap();
        assert_eq!(tokens(&queues.finish(default_row(), Default::default()).unwrap()), ["", "X"]);
    }

    #[test]
    fn suffix_transfer_keeps_existing_ties_and_final_dedup_uses_bucket_order() {
        let queues = TemplateQueues::default();
        let output = queues.finish(default_row(), [
            vec![row("first", "X", 0), row("tie", "A", 0), row("tie", "B", 0)],
            vec![row("middle", "Y", 5)],
            vec![row("last", "X", 999), row("last", "Z", 0)],
        ]).unwrap();
        assert_eq!(tokens(&output), ["", "X", "A", "B", "Y", "Z"]);
        assert_eq!(output[1].label, "first");
    }

    #[test]
    fn malformed_outer_bit_and_default_are_named_refusals() {
        let mut queues = TemplateQueues::default();
        assert!(queues.admit_variant_for_bit(0, 1, false, None, 0, 0, "", &[row("a", "X", 1)]).is_err());
        assert!(queues.admit_variant_for_bit(3, 1, false, None, 0, 0, "", &[row("a", "X", 1)]).is_err());
        assert!(queues.queues.iter().all(Vec::is_empty));
        let error = queues.finish(row("bad default", "X", 0), Default::default()).unwrap_err();
        assert!(error.to_string().contains("default model row is malformed"));
    }
}
