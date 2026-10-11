import { usNumber } from '../../charts/numberFormat.ts';
import { runAt, isOrderedSpans, spansCovered } from '../../charts/series.ts';
import type { Span } from '../../charts/series';
import { object, opaqueTag, signed32, unsigned64, int } from '../../dashboard/validation.ts';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { CasualtyPairsReply } from '../../generated/dashboard';

// Country-pair sums can exceed i64; preserve the native signed counters in BigInt.
// Fewer than 39 digits always fit, so only the longest counters are parsed.
const inRange = (value: bigint) => value >= -(1n << 127n) && value < (1n << 127n);
const amount = (value: unknown): value is string => typeof value === 'string'
  && value.length <= 40
  && /^-?(0|[1-9]\d*)$/.test(value)
  && (value.length - (value[0] === '-' ? 1 : 0) < 39 || inRange(BigInt(value)));
const append = (lists: Map<string, Span[]>, key: string, runs: readonly Span[]) => {
  let list = lists.get(key);
  if (!list) lists.set(key, list = []);
  for (const run of runs) list.push(run);
};

export function validateCasualtyPairs(value: unknown): CasualtyPairsReply {
  const bad = () => { throw new Error('The casualty pairs reply is malformed.'); };
  if (!object(value) || !unsigned64(value.branchId) || typeof value.available !== 'boolean'
    || typeof value.hasGap !== 'boolean' || !isOrderedSpans(value.windows) || !isOrderedSpans(value.observed)
    || !(value.window === null || int(value.window) && value.window >= 0 && value.window < value.windows.length)
    || !Array.isArray(value.pairs) || !Array.isArray(value.warnings) || !value.warnings.every(w => typeof w === 'string')) return bad();
  if ([...value.windows, ...value.observed].some(span => !signed32(span.firstHour) || !signed32(span.lastHour))) return bad();
  const keys = new Set<string>();
  const endpoints = new Map<string, Span[]>();
  const identities = new Map<string, Span[]>();
  for (const pair of value.pairs) {
    if (!object(pair) || typeof pair.identity !== 'string' || !pair.identity
      || !opaqueTag(pair.firstCountry) || !opaqueTag(pair.secondCountry) || pair.firstCountry === pair.secondCountry
      || !isOrderedSpans(pair.runs) || !spansCovered(pair.runs, value.observed)) return bad();
    const key = JSON.stringify([pair.identity, pair.firstCountry, pair.secondCountry]);
    if (keys.has(key)) return bad();
    keys.add(key);
    append(endpoints, JSON.stringify([pair.firstCountry, pair.secondCountry].sort(compareTags)), pair.runs);
    append(identities, pair.identity, pair.runs);
    for (const run of pair.runs) {
      if (!signed32(run.firstHour) || !signed32(run.lastHour) || !signed32(run.readHour) || run.readHour > run.firstHour
        || typeof run.hourly !== 'boolean' || typeof run.incomplete !== 'boolean'
        || !amount(run.firstTaken) || !amount(run.secondTaken)) return bad();
      if (value.window !== null && !run.hourly) {
        const start = Math.max(run.readHour, value.windows[value.window].firstHour);
        if (!spansCovered([{ firstHour: start, lastHour: run.firstHour }], value.observed)) return bad();
      }
    }
  }
  // Successive holders may share tags; one holder cannot occupy two aliases at once.
  for (const runs of [...endpoints.values(), ...identities.values()]) if (!isOrderedSpans(runs.sort((a, b) => a.firstHour - b.firstHour))) return bad();
  if (value.window === null && (value.pairs.length || value.observed.length)) return bad();
  if (value.window !== null) {
    const window = value.windows[value.window];
    if (value.observed.some(span => span.firstHour < window.firstHour || span.lastHour > window.lastHour)) return bad();
  }
  return value as unknown as CasualtyPairsReply;
}

export interface PairFlow { id: string; from: string; to: string; value: bigint; readHour: number; incomplete: boolean }
export interface PairSnapshot { countries: string[]; flows: PairFlow[] }

// pairs.rs orders endpoints by native identity, independently of their current tags.
export const pairFlowId = (identity: string, direction: 'first' | 'second') => JSON.stringify([identity, direction]);

/** Source deals the losses, target takes them. Absent relations produce no flow,
 * not zero. Different holders of a tag never sum together. */
export function casualtyPairsAt(reply: CasualtyPairsReply, hour: number, includes: (tag: string) => boolean = () => true,
  between = false): PairSnapshot {
  if (!runAt(reply.observed, hour)) return { countries: [], flows: [] };
  const flows: PairFlow[] = [];
  const countries = new Set<string>();
  for (const pair of reply.pairs) {
    const first = includes(pair.firstCountry), second = includes(pair.secondCountry);
    if (!(between ? first && second : first || second)) continue;
    const run = runAt(pair.runs, hour);
    if (!run) continue;
    const read = run.hourly ? hour : run.readHour;
    countries.add(pair.firstCountry); countries.add(pair.secondCountry);
    flows.push({ id: pairFlowId(pair.identity, 'first'), from: pair.firstCountry,
      to: pair.secondCountry, value: BigInt(run.secondTaken), readHour: read, incomplete: run.incomplete });
    flows.push({ id: pairFlowId(pair.identity, 'second'), from: pair.secondCountry,
      to: pair.firstCountry, value: BigInt(run.firstTaken), readHour: read, incomplete: run.incomplete });
  }
  return { countries: [...countries].sort(compareTags), flows };
}

export const exactCount = (value: number | bigint) => usNumber.format(value);
export const pairAmount = (flow: Pick<PairFlow, 'value' | 'incomplete'>) =>
  `${flow.incomplete ? 'Recorded subtotal · ' : ''}${exactCount(flow.value)}`;

export function casualtyPairWindows(reply: CasualtyPairsReply | undefined, hour: number, following: boolean): number[] {
  const current = reply?.windows.findIndex(span => span.firstHour <= hour && hour <= span.lastHour) ?? -1;
  return current < 0 ? [] : following && current + 1 < reply!.windows.length ? [current, current + 1] : [current];
}
