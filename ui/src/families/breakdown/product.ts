import type { Breakdown, BreakdownPart as WirePart } from '../../generated/dashboard';
import type { BreakdownPart, BreakdownTotal } from './model.ts';

export interface BreakdownWords {
  readonly unit: string;
  readonly verb: string;
  readonly format: (raw: bigint, decimals: number) => string;
}

function scaled(raw: bigint, decimals: number): number {
  return Number(raw) / 10 ** decimals;
}

function part(wire: WirePart, decimals: number, words: BreakdownWords): BreakdownPart {
  const raw = wire.raw === null ? null : BigInt(wire.raw);
  const reason = wire.reason ?? undefined;
  return {
    key: wire.key,
    name: wire.name,
    ordinal: wire.ordinal,
    value: raw === null ? null : scaled(raw, decimals),
    text: raw === null ? '' : words.format(raw, decimals),
    share: wire.share,
    ...(wire.other ? { neutral: true } : {}),
    ...(wire.state === 'notRecorded' ? { notRecorded: true, missingReason: reason ?? 'Not recorded' } : {}),
    ...(wire.state === 'partial' || wire.state === 'atLeast' ? { partial: true, ...(reason ? { partialReason: reason } : {}) } : {}),
    ...(wire.state === 'atLeast' ? { atLeast: true } : {}),
    ...(wire.count !== null ? { count: Number(wire.count) } : {}),
    ...(wire.children.length ? { children: wire.children.map(child => part(child, decimals, words)) } : {}),
  };
}

export function breakdownParts(breakdown: Breakdown, words: BreakdownWords): readonly BreakdownPart[] {
  return breakdown.parts.map(wire => part(wire, breakdown.decimals, words));
}

function hours(count: number): string {
  if (count < 48) return `${count.toLocaleString('en-US')} ${count === 1 ? 'hour' : 'hours'} not recorded`;
  const days = Math.floor(count / 24);
  return `${days.toLocaleString('en-US')} ${days === 1 ? 'day' : 'days'} not recorded`;
}

export function breakdownTotal(breakdown: Breakdown, words: BreakdownWords): BreakdownTotal {
  const { total, decimals } = breakdown;
  const raw = total.raw === null ? null : BigInt(total.raw);
  const reason = total.reason ?? undefined;
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
