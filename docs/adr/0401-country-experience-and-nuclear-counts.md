# 0401. Keep native experience, progress and ready counts independent

Country experience retains nine signed fixed-point operands, including separate army and navy training and non-training gains. The native totals clamp and the daily pass updates the smoothed army-count training divisor and resets the counters. The capture preserves every operand's raw representation regardless of save precision. It authenticates the country and status together. The header and operands are copied in one contiguous range without widening into unrelated data. An unreadable allocated status does not produce a zero row.

A country's nuclear entries are inline and authenticated in the stored array order. Loading can validly produce any count, including zero, and it uses a different persistent default from normal construction. Raw production progress and the independently updated ready launch count both retain their original signed values. Capture does not derive one from the other. Recorded ownership keeps raw aliases and type ordinals identify the stored nuclear types.

[Experience operands](../../crates/engine-events/src/ccountry_experience_status.rs) · [Nuclear entries](../../crates/engine-events/src/cnuke.rs)
