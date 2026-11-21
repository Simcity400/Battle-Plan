import { isEngineHour } from './calendar.ts';
import { object as recordValue, signed32 as i32Value, signed64 as raw64Value, opaqueTag as tokenValue } from './validation.ts';

export { recordValue, i32Value, raw64Value, tokenValue };

export const textValue = (value: unknown): value is string => typeof value === 'string' && !value.includes('\0');
export const BROWSER_ARTWORK_BASE = '/api/artwork/';
export const artworkName = (value: string): boolean => /^[0-9a-f]{32}\.png$/.test(value);
export const artworkUrlValue = (value: unknown): value is string => typeof value === 'string'
  && /^(?:http:\/\/artwork\.localhost|\/api\/artwork)\/[0-9a-f]{32}\.png$/.test(value);
export const nullableValue = (check: (value: unknown) => boolean) => (value: unknown): boolean => value === null || check(value);

export type PayloadSpan = Record<string, unknown> & { firstHour: number; lastHour: number };
const hourSpanValue = (value: unknown): value is PayloadSpan => recordValue(value)
  && typeof value.firstHour === 'number' && isEngineHour(value.firstHour)
  && typeof value.lastHour === 'number' && isEngineHour(value.lastHour) && value.firstHour <= value.lastHour;

export const orderedHourSpans = (value: unknown): value is PayloadSpan[] => Array.isArray(value)
  && value.every((span, index) => hourSpanValue(span) && (!index || value[index - 1].lastHour < span.firstHour));
