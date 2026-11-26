import type { Breakdown, BreakdownAmount, BreakdownPart as WirePart } from '../../generated/dashboard';
import { formatRational, formatScalar } from '../valueFormat.ts';
import { PATH, type BreakdownPart, type BreakdownTotal } from './model.ts';

export interface BreakdownWords {
  readonly unit: string;
  readonly verb: string;
  readonly format: (numerator: bigint, denominator: bigint) => string;
}

function sentence(text: string | null): string | undefined {
  return text ? text.charAt(0).toUpperCase() + text.slice(1) : undefined;
}

const EXACT_DECIMALS = 7;

function amount(wire: BreakdownAmount | null, words: BreakdownWords): { readonly value: number; readonly text: string; readonly exact?: string } | null {
  if (wire === null) return null;
  const numerator = BigInt(wire.numerator);
  const denominator = BigInt(wire.denominator);
  const text = words.format(numerator, denominator);
  const exact = formatRational(numerator, denominator, { maxDecimals: EXACT_DECIMALS });
  return { value: Number(numerator) / Number(denominator), text, ...(exact !== text ? { exact } : {}) };
}

function part(wire: WirePart, words: BreakdownWords, parent: string | null): BreakdownPart {
  const known = amount(wire.amount, words);
  const reason = sentence(wire.reason);
  const key = parent === null ? wire.key : `${parent}${PATH}${wire.key}`;
  return {
    key,
    name: wire.name,
    ...(wire.nameMissing ? { nameMissing: sentence(wire.nameMissing) } : {}),
    ordinal: wire.ordinal,
    value: known?.value ?? null,
    text: known?.text ?? '',
    ...(known?.exact ? { exact: known.exact } : {}),
    ...(wire.amount ? { raw: `${wire.amount.numerator}/${wire.amount.denominator}` } : {}),
    share: wire.share,
    parentShare: wire.parentShare,
    ...(wire.other ? { neutral: true } : {}),
    ...(wire.state === 'notRecorded' ? { notRecorded: true, missingReason: reason ?? 'Not recorded' } : {}),
    ...(wire.state === 'partial' || wire.state === 'atLeast' ? { partial: true, ...(reason ? { partialReason: reason } : {}) } : {}),
    ...(wire.state === 'atLeast' ? { atLeast: true } : {}),
    ...(wire.count !== null ? { count: Number(wire.count) } : {}),
    ...(wire.children.length ? { children: wire.children.map(child => part(child, words, key)) } : {}),
  };
}

export function breakdownParts(breakdown: Breakdown, words: BreakdownWords): readonly BreakdownPart[] {
  return breakdown.parts.map(wire => part(wire, words, null));
}

function hours(count: number): string {
  if (count < 48) return `${formatScalar(count, 'integer')} ${count === 1 ? 'hour' : 'hours'} not recorded`;
  const days = Math.floor(count / 24);
  return `${formatScalar(days, 'integer')} ${days === 1 ? 'day' : 'days'} not recorded`;
}

export function breakdownTotal(breakdown: Breakdown, words: BreakdownWords): BreakdownTotal {
  const { total } = breakdown;
  const known = amount(total.amount, words);
  const reason = sentence(total.reason);
  return {
    known: total.state === 'known',
    ...(total.state === 'atLeast' ? { atLeast: true } : {}),
    ...(total.state === 'partial' || total.state === 'atLeast' ? { partial: true } : {}),
    value: known?.value ?? null,
    text: known?.text ?? 'Not recorded',
    ...(known?.exact ? { exact: known.exact } : {}),
    unit: words.unit,
    verb: words.verb,
    ...(reason ? { reason } : {}),
    ...(breakdown.unrecordedHours > 0 ? { shortReason: hours(breakdown.unrecordedHours) } : {}),
  };
}
