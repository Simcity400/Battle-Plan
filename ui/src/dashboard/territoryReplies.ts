import type { Decoded } from '../charts/chartSummary';
import { chartSummaryLines } from '../charts/chartSummary.ts';
import type { FactionLine, FactionLinesReply } from '../generated/dashboard';

export type FactionLines = Omit<FactionLinesReply, 'lines'> & { lines: Decoded<FactionLine>[] };

const object = (value: unknown): value is Record<string, unknown> => value !== null && typeof value === 'object' && !Array.isArray(value);
const count = (value: unknown) => Number.isSafeInteger(value);
const strings = (value: unknown): value is string[] => Array.isArray(value) && value.every(item => typeof item === 'string');
const spans = (value: unknown) => Array.isArray(value) && value.every(span => object(span) && count(span.firstHour) && count(span.lastHour)
  && (span.firstHour as number) <= (span.lastHour as number));

function malformed(what: string): never {
  throw new Error(`The ${what} reply is malformed.`);
}

export function validateFactionLines(value: unknown): FactionLines {
  if (!object(value) || typeof value.branchId !== 'string' || typeof value.measure !== 'string' || !count(value.decimals)
    || typeof value.bounds !== 'boolean' || !strings(value.warnings)) malformed('faction lines');
  const lines = chartSummaryLines<Decoded<FactionLine>>(value.lines, line => typeof line.id === 'string' && spans(line.missing));
  if (!lines) malformed('faction lines');
  return { ...value as unknown as FactionLines, lines };
}
