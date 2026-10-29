import type { EconomyRun, FuelBranch, FuelBranchCountry, FuelBranchesReply } from '../../generated/dashboard';
import type { Run, Span } from '../../charts/series';
import { clipToSpans } from '../../charts/series.ts';
import { pendingAt, preparedPrefix } from '../../dashboard/preparedPrefix.ts';
import { object } from '../../dashboard/validation.ts';
import { bigAt, isEconomyRuns, isHolderTags, numberAt, runAt, tagAt, validateEconomyReply } from '../../dashboard/economyHistory.ts';

export const FUEL_BRANCHES = 'fuel-branches';

export function validateFuelBranches(value: unknown): FuelBranchesReply {
  return validateEconomyReply<FuelBranchesReply>(value, reply => Number.isSafeInteger(reply.scale) && (reply.scale as number) > 0 && isEconomyRuns(reply.known)
    && Array.isArray(reply.consumerNames) && reply.consumerNames.every(name => name === null || typeof name === 'string')
    && Array.isArray(reply.countries) && reply.countries.every(country => object(country) && typeof country.holder === 'string'
      && isHolderTags(country.tags) && isEconomyRuns(country.read) && Array.isArray(country.branches)
      && country.branches.every(branch => object(branch) && Number.isSafeInteger(branch.index)
        && isEconomyRuns(branch.requested) && isEconomyRuns(branch.received) && isEconomyRuns(branch.priority))), 'fuel by branch');
}

/** One branch at an hour: the read's amounts, or null where not recorded. */
export interface BranchAt {
  index: number;
  name: string;
  requested: bigint | null;
  received: bigint | null;
  /** The part requested and not received; null unless both are known. */
  unmet: bigint | null;
  priority: number | null;
}

export const branchName = (reply: FuelBranchesReply, index: number) => reply.consumerNames[index] ?? 'Branch name unavailable';

/** The country holder that had `tag` at `hour`: one whose consumers were read
 * then, so a reused tag never selects a former holder whose last tag entry merely
 * precedes the hour. Null where no holder with the tag was read at the hour. */
export function countryAt(reply: FuelBranchesReply, tag: string, hour: number): FuelBranchCountry | null {
  return reply.countries.find(country => tagAt(country.tags, hour) === tag && runAt(country.read, hour) !== null) ?? null;
}

/** The holder an over-time chart of `tag` draws: the one valid at `hour`, else the
 * latest holder read with the tag at or before it. */
export function chartCountryAt(reply: FuelBranchesReply, tag: string, hour: number): FuelBranchCountry | null {
  const valid = countryAt(reply, tag, hour);
  if (valid) return valid;
  let best: FuelBranchCountry | null = null, bestHour = -Infinity;
  for (const country of reply.countries) for (const run of country.read) {
    if (run[0] <= hour && run[1] > bestHour && tagAt(country.tags, run[1]) === tag) { best = country; bestHour = run[1]; }
  }
  return best;
}

/** Whether the consumer table was read complete at `hour`, every consumer's country identified. */
export const consumersKnown = (reply: FuelBranchesReply, hour: number) => runAt(reply.known, hour) !== null;

/** Each branch the recording names for the country, in native order, at `hour`. */
export function branchesAt(reply: FuelBranchesReply, country: FuelBranchCountry, hour: number): BranchAt[] {
  return [...country.branches].sort((a, b) => a.index - b.index).map(branch => {
    const requested = bigAt(branch.requested, hour), received = bigAt(branch.received, hour);
    // The game shows what was not received of a request; a receipt above it leaves none unmet.
    const unmet = requested !== null && received !== null ? (requested > received ? requested - received : 0n) : null;
    return { index: branch.index, name: branchName(reply, branch.index), requested, received, unmet, priority: numberAt(branch.priority, hour) };
  });
}

/** The read supplying the country's consumers at `hour`. */
export const readAt = (country: FuelBranchCountry, hour: number): number | null => numberAt(country.read, hour);

/** Received over requested in percent, where something was requested; hours with no
 * request or no read are gaps, never 0%. */
export function ratioRuns(branch: FuelBranch): Run[] {
  const runs: Run[] = [];
  let other = 0;
  for (const asked of branch.requested) {
    const requested = BigInt(asked[2]);
    while (other < branch.received.length && branch.received[other][1] < asked[0]) other++;
    for (let index = other; index < branch.received.length && branch.received[index][0] <= asked[1]; index++) {
      const got: EconomyRun = branch.received[index];
      const first = Math.max(asked[0], got[0]), last = Math.min(asked[1], got[1]);
      if (first > last || requested <= 0n) continue;
      const value = Number(BigInt(got[2]) * 1_000_000n / requested) / 10_000;
      const previous = runs.at(-1);
      if (previous && previous.lastHour + 1 === first && previous.value === value) previous.lastHour = last;
      else runs.push({ firstHour: first, lastHour: last, value, breakBefore: !previous || previous.lastHour + 1 !== first });
    }
  }
  return runs;
}

/** The ratio axis: at least 0-100%, high enough for every recorded ratio, with at most
 * eight round steps (25, 50, 100, 250, 500, 1,000% ...). */
export function ratioAxis(lines: readonly { runs: readonly Run[] }[]): { top: number; ticks: number[] } {
  let most = 100;
  for (const line of lines) for (const run of line.runs) if (run.value > most) most = run.value;
  let step = 25;
  for (let scale = 1; Math.ceil(most / step) > 8; scale *= 10) step = [25, 50, 100].map(base => base * scale).find(size => Math.ceil(most / size) <= 8) ?? 100 * scale;
  const top = Math.ceil(most / step) * step;
  return { top, ticks: Array.from({ length: top / step + 1 }, (_, index) => index * step) };
}

/** Why a time view from `from` through `hour` has no line: its hour is still being
 * prepared ('pending'), the consumers were known at every hour and the country had
 * none ('none', proven), known at only some ('partial'), or at none ('unrecorded').
 * A known hour never stands in for a pending or unknown one. */
export function emptyTimeView(reply: FuelBranchesReply, from: number, hour: number): 'none' | 'partial' | 'pending' | 'unrecorded' {
  if (reply.lastHour === null || reply.lastHour < hour || pendingAt(preparedPrefix(reply), hour)) return 'pending';
  let covered = 0, next = from;
  for (const [first, last] of reply.known) {
    const start = Math.max(first, next), end = Math.min(last, hour);
    if (start > end) continue;
    covered += end - start + 1;
    next = end + 1;
  }
  return covered === 0 ? 'unrecorded' : covered === hour - from + 1 ? 'none' : 'partial';
}

/** A branch's received share inside the group's membership `within` (every hour
 * when absent): a country outside the group draws nothing. */
export function memberRatioRuns(branch: FuelBranch, within: readonly Span[] | null | undefined): Run[] {
  const runs = ratioRuns(branch);
  return within ? clipToSpans(runs, within) : runs;
}

/** Whether `hour` is a member hour (every hour when no membership applies). */
export const memberAt = (within: readonly Span[] | null | undefined, hour: number) =>
  !within || within.some(span => span.firstHour <= hour && hour <= span.lastHour);

/** Each change hour of a branch, for keyboard stops. */
export function changeHours(country: FuelBranchCountry): number[] {
  return [...new Set(country.branches.flatMap(branch => [...branch.requested, ...branch.received].map(run => run[0])))].sort((a, b) => a - b);
}

export const percentText = (value: number) => `${value.toFixed(value % 1 === 0 ? 0 : 2)}%`;
