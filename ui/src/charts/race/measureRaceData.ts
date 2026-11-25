import type { SeriesRankReply, SeriesRankRow, SeriesState } from '../../generated/dashboard';

const isInteger = (value: unknown): value is number => Number.isSafeInteger(value);
const isRaw = (value: unknown): value is string => typeof value === 'string' && /^-?\d+$/.test(value);

function validState(value: unknown): value is SeriesState {
  if (!value || typeof value !== 'object') return false;
  const state = value as Record<string, unknown>;
  const run = state.run as Record<string, unknown> | null;
  return (typeof state.country === 'string' || typeof state.country === 'number') && isInteger(state.hour) && (state.missing === null || typeof state.missing === 'string')
    && (run === null || (!!run && typeof run === 'object' && isRaw(run.valueRaw) && isInteger(run.firstHour) && isInteger(run.lastHour)));
}

function validRankRow(value: unknown, measures: number): boolean {
  const row = value as { country?: unknown; readings?: unknown } | null;
  return !!row && (typeof row.country === 'string' || typeof row.country === 'number')
    && Array.isArray(row.readings) && row.readings.length === measures && row.readings.every(validState);
}

export function validateRankReply(value: unknown): SeriesRankReply {
  const reply = value as Partial<SeriesRankReply> | null;
  if (!reply || typeof reply !== 'object' || !isInteger(reply.hour) || !isInteger(reply.from) || !isInteger(reply.until)
    || !(reply.from <= reply.hour && reply.hour <= reply.until) || !(reply.refused === null || typeof reply.refused === 'string')
    || !Array.isArray(reply.measures) || !reply.measures.length
    || !reply.measures.every(measure => !!measure && typeof measure.name === 'string' && isInteger(measure.decimals))
    || !Array.isArray(reply.rows) || !reply.rows.every(row => validRankRow(row, reply.measures!.length))) {
    throw new Error('The rank reply is malformed.');
  }
  return reply as SeriesRankReply;
}

export function rankProvenInterval(data: unknown, hour: number): readonly [number, number] {
  const reply = data as SeriesRankReply;
  return reply.from <= hour && hour <= reply.until ? [reply.from, reply.until] : [hour, hour];
}

export function rankReading(reply: SeriesRankReply, row: SeriesRankRow, measure: string): SeriesState | null {
  const at = reply.measures.findIndex(held => held.name === measure);
  return at < 0 ? null : row.readings[at] ?? null;
}

export function formatRaw(raw: string, decimals: number, digits = 0): string {
  const negative = raw.startsWith('-'), body = (negative ? raw.slice(1) : raw).padStart(decimals + 1, '0');
  const whole = body.slice(0, body.length - decimals), fraction = body.slice(body.length - decimals, body.length - decimals + Math.min(digits, decimals)).replace(/0+$/, '');
  const grouped = whole.replace(/\B(?=(\d{3})+(?!\d))/g, ',');
  const sign = negative && (/[1-9]/.test(whole) || fraction !== '') ? '−' : '';
  return `${sign}${grouped}${fraction ? `.${fraction}` : ''}`;
}
