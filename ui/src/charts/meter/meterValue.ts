export const meterValue = (value: number | null): number | null => value === null || !Number.isFinite(value) ? null : Math.max(0, Math.min(1, value));
