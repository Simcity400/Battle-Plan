import { isEngineHour } from './calendar.ts';
import { object as recordValue, signed32 as i32Value, signed64 as raw64Value, opaqueTag as tokenValue } from './validation.ts';

export const artworkUrlValue = (value: unknown): value is string => typeof value === 'string'
  && /^(?:http:\/\/artwork\.localhost|\/api\/artwork)\/[0-9a-f]{32}\.png$/.test(value);

const hourSpanValue = (value: unknown): value is PayloadSpan => recordValue(value)
  && typeof value.firstHour === 'number' && isEngineHour(value.firstHour)
  && typeof value.lastHour === 'number' && isEngineHour(value.lastHour) && value.firstHour <= value.lastHour;

