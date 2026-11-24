import type { Breakdown, BreakdownPart as WirePart } from '../../generated/dashboard';
import { formatScalar } from '../valueFormat.ts';
import { PATH, type BreakdownPart, type BreakdownTotal } from './model.ts';

export interface BreakdownWords {
  readonly unit: string;
  readonly verb: string;
  readonly format: (raw: bigint, decimals: number) => string;
}

function sentence(text: string | null): string | undefined {
  return text ? text.charAt(0).toUpperCase() + text.slice(1) : undefined;
}

function scaled(raw: bigint, decimals: number): number {
  return Number(raw) / 10 ** decimals;
}

function part(wire: WirePart, decimals: number, words: BreakdownWords, parent: string | null): BreakdownPart {
  const raw = wire.raw === null ? null : BigInt(wire.raw);
  const reason = sentence(wire.reason);
  const key = parent === null ? wire.key : `${parent}${PATH}${wire.key}`;
  return {
    key,
    name: wire.name,
    ordinal: wire.ordinal,
    value: raw === null ? null : scaled(raw, decimals),
    text: raw === null ? '' : words.format(raw, decimals),
    share: wire.share,
    parentShare: wire.parentShare,
    ...(wire.other ? { neutral: true } : {}),
    ...(wire.state === 'notRecorded' ? { notRecorded: true, missingReason: reason ?? 'Not recorded' } : {}),
    ...(wire.state === 'partial' || wire.state === 'atLeast' ? { partial: true, ...(reason ? { partialReason: reason } : {}) } : {}),
    ...(wire.state === 'atLeast' ? { atLeast: true } : {}),
    ...(wire.count !== null ? { count: Number(wire.count) } : {}),
    ...(wire.children.length ? { children: wire.children.map(child => part(child, decimals, words, key)) } : {}),
  };
}

export function breakdownParts(breakdown: Breakdown, words: BreakdownWords): readonly BreakdownPart[] {
  return breakdown.parts.map(wire => part(wire, breakdown.decimals, words, null));
}

function hours(count: number): string {
  if (count < 48) return `${formatScalar(count, 'integer')} ${count === 1 ? 'hour' : 'hours'} not recorded`;
  const days = Math.floor(count / 24);
  return `${formatScalar(days, 'integer')} ${days === 1 ? 'day' : 'days'} not recorded`;
}

export function breakdownTotal(breakdown: Breakdown, words: BreakdownWords): BreakdownTotal {
  const { total, decimals } = breakdown;
  const raw = total.raw === null ? null : BigInt(total.raw);
  const reason = sentence(total.reason);
  return {
    known: total.state === 'known',
    ...(total.state === 'atLeast' ? { atLeast: true } : {}),
    ...(total.state === 'partial' || total.state === 'atLeast' ? { partial: true } : {}),
    value: raw === null ? null : scaled(raw, decimals),
    text: raw === null ? 'Not recorded' : words.format(raw, decimals),
    unit: words.unit,
    verb: words.verb,
    ...(reason ? { reason } : {}),
    ...(breakdown.unrecordedHours > 0 ? { shortReason: hours(breakdown.unrecordedHours) } : {}),
  };
}
