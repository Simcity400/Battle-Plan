/** Shared JSON-shape primitives; no coercion or repaired values. */
export const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);
export const int = (value: unknown): value is number => typeof value === 'number' && Number.isSafeInteger(value);
export const signed32 = (value: unknown): value is number => int(value) && value >= -0x8000_0000 && value <= 0x7fff_ffff;
export const unsigned32 = (value: unknown): value is number => int(value) && value >= 0 && value <= 0xffff_ffff;
/** Native 64-bit IDs and lengths travel as decimal strings without rounding. */
export const unsigned64 = (value: unknown): value is string => typeof value === 'string' && /^\d{1,20}$/.test(value) && BigInt(value) < (1n << 64n);

/** Recorded signed 64-bit integers travel as decimal strings, never numbers. */
export const signed64 = (value: unknown): value is string => typeof value === 'string' && /^-?\d{1,19}$/.test(value)
  && BigInt(value) >= -(1n << 63n) && BigInt(value) < (1n << 63n);

/** Country tags are opaque recorded text; no vanilla spelling is assumed. */
export const opaqueTag = (value: unknown): value is string => typeof value === 'string' && value.length > 0 && !value.includes('\0');
