import { isEngineHour } from './calendar.ts';
import { object as recordValue, signed32 as i32Value, signed64 as raw64Value, opaqueTag as tokenValue } from './validation.ts';

export { recordValue, i32Value, raw64Value, tokenValue };

/** Product shape checks shared by history widgets; content meaning stays with
 * each product's own validator. Extra record fields remain forward-compatible. */
export const textValue = (value: unknown): value is string => typeof value === 'string' && !value.includes('\0');
export const pngDataValue = (value: unknown): value is string => textValue(value) && value.startsWith('data:image/png;base64,');
export const nullableValue = (check: (value: unknown) => boolean) => (value: unknown): boolean => value === null || check(value);

export type PayloadSpan = Record<string, unknown> & { firstHour: number; lastHour: number };
const hourSpanValue = (value: unknown): value is PayloadSpan => recordValue(value)
  && typeof value.firstHour === 'number' && isEngineHour(value.firstHour)
  && typeof value.lastHour === 'number' && isEngineHour(value.lastHour) && value.firstHour <= value.lastHour;

/** Inclusive spans in increasing, nonoverlapping order. Adjacency is allowed;
 * continuity is a separate per-product promise. */
export const orderedHourSpans = (value: unknown): value is PayloadSpan[] => Array.isArray(value)
  && value.every((span, index) => hourSpanValue(span) && (!index || value[index - 1].lastHour < span.firstHour));
