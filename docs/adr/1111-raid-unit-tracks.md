# Place raid units continuously within a published hour

Raid motion uses stored control points and recorded progress. Build straight control paths in the shortest horizontal wrap copy. Use one segment for line arrows and the bounded ceiling of length over segment size for others. Evaluate lift from the wrapped fixed5 endpoint distance and terrain heights. Air motion follows the authored smoothed arc, and the ballistic curve is refitted when its apex would exceed the authored height cap.

Derive phase-4 elapsed and remaining hours with native integer rounding and preserve its invalid or unbounded distance handling. Interpolate current and next-hour progress by the fractional tick, then map it onto the arrow interval beginning at the authored start offset. The tick pose samples the stored polyline and its tangent, adds the entity offset, wraps the position, and faces back along the tangent. Keep animation history by raid ID so the next hour continues the entity. Only stages present on the resolved entity can be selected. If no reached stage exists, its current state persists.

[Unit placement and pose tests](../../crates/viewer-gpu/src/unit_pose.rs)
