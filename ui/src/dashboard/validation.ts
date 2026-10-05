/** Shared JSON-shape primitives; no coercion or repaired values. */
export const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);
export const int = (value: unknown): value is number => typeof value === 'number' && Number.isSafeInteger(value);
