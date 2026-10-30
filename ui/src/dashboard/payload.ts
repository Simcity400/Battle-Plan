import { isEngineHour } from './calendar.ts';
import { object as recordValue, signed32 as i32Value, signed64 as raw64Value, opaqueTag as tokenValue } from './validation.ts';

export { recordValue, i32Value, raw64Value, tokenValue };

/** Product shape checks shared by history widgets; content meaning stays with
 * each product's own validator. Extra record fields remain forward-compatible. */
export const textValue = (value: unknown): value is string => typeof value === 'string' && !value.includes('\0');
/** Where the development server serves product images; the desktop app serves them from its `artwork` scheme. */
export const BROWSER_ARTWORK_BASE = '/api/artwork/';
/** A product image's name: its content hash. */
export const artworkName = (value: string): boolean => /^[0-9a-f]{32}\.png$/.test(value);
/** A product image: the URL the desktop app or the development server serves it at. */
export const artworkUrlValue = (value: unknown): value is string => typeof value === 'string'
  && /^(?:http:\/\/artwork\.localhost|\/api\/artwork)\/[0-9a-f]{32}\.png$/.test(value);
export const nullableValue = (check: (value: unknown) => boolean) => (value: unknown): boolean => value === null || check(value);

export type PayloadSpan = Record<string, unknown> & { firstHour: number; lastHour: number };
const hourSpanValue = (value: unknown): value is PayloadSpan => recordValue(value)
  && typeof value.firstHour === 'number' && isEngineHour(value.firstHour)
  && typeof value.lastHour === 'number' && isEngineHour(value.lastHour) && value.firstHour <= value.lastHour;

/** Inclusive spans in increasing, nonoverlapping order. Adjacency is allowed;
 * continuity is a separate per-product promise. */
export const orderedHourSpans = (value: unknown): value is PayloadSpan[] => Array.isArray(value)
  && value.every((span, index) => hourSpanValue(span) && (!index || value[index - 1].lastHour < span.firstHour));
