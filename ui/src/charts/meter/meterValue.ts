/** Geometry and accessibility share the clamp; missing never becomes zero. */
export const meterValue = (value: number | null): number | null => value === null || !Number.isFinite(value) ? null : Math.max(0, Math.min(1, value));
