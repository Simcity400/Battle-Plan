import type { CipherObservation, DecryptionCoverage, DecryptionPair, DecryptionReply, DecryptionRun, DecryptionTagRun } from '../../generated/dashboard';
import { runAt } from '../../charts/series.ts';
import { formatAmount } from '../../charts/valueAxis.ts';
import { formatDate, gameDate } from '../../dashboard/calendar.ts';
import { compareTags } from '../../dashboard/tagOrder.ts';
import { fitTooltip } from '../../charts/tooltip.ts';
import type { TooltipContent } from '../../charts/tooltip';

const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value != null && !Array.isArray(value);
const hour = (value: unknown): value is number => typeof value === 'number' && Number.isInteger(value) && value >= -2_147_483_648 && value <= 2_147_483_647;
const flags = (value: unknown): value is number => typeof value === 'number' && Number.isInteger(value) && value >= 0 && value <= 0xffff;
const tag = (value: unknown): value is string => typeof value === 'string' && value.length > 0 && !value.includes('\0');
const raw = (value: unknown): value is string => typeof value === 'string' && /^-?(0|[1-9]\d{0,18})$/.test(value)
  && BigInt(value) >= -(1n << 63n) && BigInt(value) < (1n << 63n);
const id = (value: unknown): value is string => typeof value === 'string' && /^(0|[1-9]\d{0,9})$/.test(value) && BigInt(value) <= 0xffff_ffffn;
const ordered = <T extends { firstHour: number; lastHour: number }>(runs: readonly T[]) => runs.every((run, i) => hour(run.firstHour) && hour(run.lastHour)
  && run.firstHour <= run.lastHour && (i === 0 || runs[i - 1].lastHour < run.firstHour));
export const NATIVE_PROGRESS_UNKNOWN = 'Native progress arithmetic could not be evaluated.';

export interface DecryptionValue {
  amountRaw: string;
  thresholdRaw: string | null;
  thresholdUnknown: string | null;
  progressPercent: number | null;
  bonusDays: number;
  bonusActive: number;
  decrypting: number;
  broken: boolean | null;
  started: CipherObservation | null;
}

/** cipher_row_bind@141A0EC20, matching dashboard/cryptology.rs. Preserve the
 * fixed5 invalid sentinel, wrapping i64 arithmetic and signed division overflow.
 * fixed_round@1424D8760 rounds halves away from zero before the native i32 cast. */
export function progressPercent(amount: bigint, active: number, threshold: bigint): number | null {
  let ratio: bigint;
  if (threshold === 0n || BigInt.asUintN(64, BigInt.asUintN(64, amount) + 0x7fff_ffff_ffff_fffen) > 0xffff_ffff_ffff_fffcn) {
    ratio = 0xffff_ffffn;
  } else {
    const numerator = BigInt.asIntN(64, amount * 100_000n);
    if (numerator === -(1n << 63n) && threshold === -1n) return null;
    ratio = numerator / threshold;
  }
  if (active !== 0) ratio = 100_000n;
  const percent = BigInt.asIntN(64, ratio * 100n);
  const rounded = BigInt.asIntN(64, percent + (percent < 0n ? -50_000n : 50_000n)) / 100_000n;
  return Math.max(1, Math.min(100, Number(BigInt.asIntN(32, rounded))));
}

const elapsedDays = (run: DecryptionRun, at: number) => Math.floor((at - run.firstHour) / 24);
const amountAt = (run: DecryptionRun, at: number) => BigInt(run.startAmountRaw) + BigInt(run.stepRawPerDay) * BigInt(elapsedDays(run, at));
const bonusAt = (run: DecryptionRun, at: number) => BigInt(run.bonusDays) + BigInt(run.bonusDaysStepPerDay) * BigInt(elapsedDays(run, at));
const working = (run: DecryptionRun, at: number) => (run.stateFlags & 0xff) !== 0 || amountAt(run, at) > 0n;
const brokenAt = (run: DecryptionRun, at: number) => (run.stateFlags >>> 8) !== 0 ? true
  : run.thresholdRaw === null ? null : amountAt(run, at) >= BigInt(run.thresholdRaw);

/** Every integer hour in a run was read. Daily steps reproduce those reads;
 * fractional hours, gaps and hours outside the run have no decoded value. */
export function valueAt(run: DecryptionRun, at: number): DecryptionValue | null {
  if (!hour(at) || at < run.firstHour || at > run.lastHour) return null;
  const amount = amountAt(run, at), bonusActive = run.stateFlags >>> 8;
  return { amountRaw: amount.toString(), thresholdRaw: run.thresholdRaw, thresholdUnknown: run.thresholdUnknown,
    progressPercent: run.thresholdRaw === null ? null : progressPercent(amount, bonusActive, BigInt(run.thresholdRaw)),
    bonusDays: Number(bonusAt(run, at)), bonusActive, decrypting: run.stateFlags & 0xff, broken: run.broken, started: run.started };
}

/** Checks every value before local playback; neither malformed missing fields nor
 * overlapping runs can turn unknown or an unrecorded hour into zero. */
export function validateDecryptionReply(data: unknown): DecryptionReply {
  const invalid = () => { throw new Error('The decryption reply is malformed.'); };
  if (!object(data) || typeof data.branchId !== 'string' || !/^\d{1,20}$/.test(data.branchId) || BigInt(data.branchId) >= (1n << 64n)
    || !Array.isArray(data.pairs) || !Array.isArray(data.coverage) || !Array.isArray(data.warnings) || !data.warnings.every(value => typeof value === 'string')) return invalid();
  if (!data.coverage.every(run => object(run) && typeof run.complete === 'boolean'
    && (run.complete ? run.reason === null : typeof run.reason === 'string' && run.reason.length > 0))
    || !ordered(data.coverage as DecryptionCoverage[])) return invalid();
  const coverage = data.coverage as DecryptionCoverage[];
  const keys = new Set<string>();
  for (const pair of data.pairs) {
    if (!object(pair) || !id(pair.ownerId) || !id(pair.targetId) || !Array.isArray(pair.runs) || !Array.isArray(pair.tagRuns) || !Array.isArray(pair.breaks)) return invalid();
    const key = JSON.stringify([pair.ownerId, pair.targetId]);
    if (keys.has(key)) return invalid();
    keys.add(key);
    for (const run of pair.runs) {
      if (!object(run) || !hour(run.firstHour) || !hour(run.lastHour) || run.lastHour < run.firstHour
        || !raw(run.startAmountRaw) || !raw(run.stepRawPerDay)
        || !(run.thresholdRaw === null || raw(run.thresholdRaw)) || !hour(run.bonusDays) || !hour(run.bonusDaysStepPerDay) || !flags(run.stateFlags)
        || !(run.broken === null || typeof run.broken === 'boolean')
        || !(run.started === null || (object(run.started) && hour(run.started.hour) && run.started.hour <= run.firstHour && typeof run.started.upperBound === 'boolean'))
        || (run.thresholdRaw === null ? typeof run.thresholdUnknown !== 'string' || !run.thresholdUnknown.length : run.thresholdUnknown !== null)) return invalid();
      const encoded = run as unknown as DecryptionRun;
      const lastAmount = amountAt(encoded, encoded.lastHour), lastBonus = bonusAt(encoded, encoded.lastHour);
      // Linear values are monotonic: endpoints prove native bounds and constant
      // predicates at every recorded hour, without expanding a long campaign.
      if (lastAmount < -(1n << 63n) || lastAmount >= (1n << 63n) || lastBonus < -(1n << 31n) || lastBonus >= (1n << 31n)
        || brokenAt(encoded, encoded.firstHour) !== encoded.broken || brokenAt(encoded, encoded.lastHour) !== encoded.broken
        || working(encoded, encoded.firstHour) !== working(encoded, encoded.lastHour)
        || (encoded.started === null && elapsedDays(encoded, encoded.lastHour) > 0 && BigInt(encoded.stepRawPerDay) > 0n && lastAmount > 0n)) return invalid();
    }
    if (!ordered(pair.runs as DecryptionRun[])) return invalid();
    const runs = pair.runs as DecryptionRun[];
    if (!pair.tagRuns.every(run => object(run) && (run.ownerTag === null || tag(run.ownerTag))
      && (run.targetTag === null || tag(run.targetTag))) || !ordered(pair.tagRuns as DecryptionTagRun[])) return invalid();
    // Identity and state compress separately but describe exactly the same recorded hours.
    if (!sameHours(runs, pair.tagRuns as DecryptionTagRun[])) return invalid();
    const bounded = (index: number) => runAt(coverage, runs[index].firstHour - 1)?.complete !== true;
    const observation = (value: unknown, index: number, upperBound: boolean) => object(value) && hour(value.hour)
      && value.hour === runs[index].firstHour && value.upperBound === upperBound;
    // A zero-progress pause looks idle until complete absence or an observed end proves otherwise.
    let nextBreak = 0, idleKnown = false;
    for (let index = 0; index < runs.length; index++) {
      const run = runs[index], previous = runs[index - 1];
      const adjacent = previous?.lastHour === run.firstHour - 1;
      const amount = amountAt(run, run.firstHour), active = run.stateFlags >>> 8;
      let started: CipherObservation | null = null;
      if (!adjacent) {
        idleKnown = !bounded(index);
        if (working(run, run.firstHour) || active !== 0) started = { hour: run.firstHour, upperBound: !idleKnown };
      } else {
        const previousAmount = amountAt(previous, previous.lastHour);
        const ended = ((previous.stateFlags >>> 8) !== 0 && active === 0)
          || (previousAmount > 0n && amount <= 0n && (run.stateFlags & 0xff) === 0 && active === 0);
        if (ended) idleKnown = true;
        started = ended ? null : previous.started;
        if (started === null && ((run.stateFlags & 0xff) !== 0 || active !== 0 || (amount > 0n && amount > previousAmount))) {
          started = { hour: run.firstHour, upperBound: !idleKnown };
        }
      }
      if (!sameObservation(run.started, started)) return invalid();
      if (run.broken !== true || (adjacent && previous.broken === true)) continue;
      const upperBound = adjacent ? previous.broken !== false : bounded(index);
      if (!observation(pair.breaks[nextBreak++], index, upperBound)) return invalid();
    }
    if (nextBreak !== pair.breaks.length) return invalid();
  }
  // Pair observations must be inside recorded coverage, including a partial read.
  for (const pair of data.pairs as DecryptionPair[]) for (const run of pair.runs) {
    let at = run.firstHour;
    while (at <= run.lastHour) {
      const cover = runAt(data.coverage as DecryptionCoverage[], at);
      if (!cover) return invalid();
      at = cover.lastHour + 1;
    }
  }
  return data as unknown as DecryptionReply;
}

const sameObservation = (a: CipherObservation | null, b: CipherObservation | null) => a === null || b === null
  ? a === b : a.hour === b.hour && a.upperBound === b.upperBound;

/** Two independently compressed histories must have the same gaps. */
function sameHours(a: readonly { firstHour: number; lastHour: number }[], b: readonly { firstHour: number; lastHour: number }[]): boolean {
  const stretches = (runs: typeof a) => {
    const spans: { firstHour: number; lastHour: number }[] = [];
    for (const run of runs) {
      const last = spans.at(-1);
      if (last && last.lastHour + 1 === run.firstHour) last.lastHour = run.lastHour;
      else spans.push({ firstHour: run.firstHour, lastHour: run.lastHour });
    }
    return spans;
  };
  const first = stretches(a), second = stretches(b);
  return first.length === second.length && first.every((span, index) => span.firstHour === second[index].firstHour && span.lastHour === second[index].lastHour);
}

export interface RaceRow { key: string; pair: DecryptionPair; run: DecryptionValue; tags: DecryptionTagRun; broken: CipherObservation | null; hour: number }

/** Latest break known at this hour, including a new observation after unknown hours. */
export function breakAt(observations: readonly CipherObservation[], hour: number): CipherObservation | null {
  let lo = 0, hi = observations.length;
  while (lo < hi) { const mid = (lo + hi) >>> 1; if (observations[mid].hour <= hour) lo = mid + 1; else hi = mid; }
  return observations[lo - 1] ?? null;
}
export type RaceOrder = 'progress' | 'decryptor';
export type CountrySide = 'decryptor' | 'target';

/** Exact sparse lookup: future starts, missing pairs and gaps never appear. */
export function raceRows(reply: DecryptionReply, hour: number, order: RaceOrder, countries: ReadonlySet<string> | null = null, side: CountrySide = 'decryptor'): RaceRow[] {
  const rows: RaceRow[] = [];
  if (!Number.isInteger(hour)) return rows;
  for (const pair of reply.pairs) {
    const encoded = runAt(pair.runs, hour);
    const run = encoded ? valueAt(encoded, hour) : null;
    const tags = runAt(pair.tagRuns, hour);
    const broken = breakAt(pair.breaks, hour);
    if (!run || !tags || !(run.started || run.broken === true)) continue;
    const endpoint = side === 'decryptor' ? tags.ownerTag : tags.targetTag;
    if (countries && (endpoint == null || !countries.has(endpoint))) continue;
    rows.push({ key: JSON.stringify([pair.ownerId, pair.targetId]), pair, run, tags, broken, hour });
  }
  const progress = (a: RaceRow, b: RaceRow) => {
    if ((a.run.broken === true) !== (b.run.broken === true)) return Number(b.run.broken === true) - Number(a.run.broken === true);
    if (a.run.progressPercent == null || b.run.progressPercent == null) return Number(a.run.progressPercent == null) - Number(b.run.progressPercent == null);
    const percent = b.run.progressPercent - a.run.progressPercent;
    if (percent !== 0) return percent;
    const x = BigInt(a.run.thresholdRaw!), y = BigInt(b.run.thresholdRaw!);
    if ((x > 0n) !== (y > 0n)) return Number(y > 0n) - Number(x > 0n);
    if (x <= 0n) return 0;
    const difference = BigInt(b.run.amountRaw) * x - BigInt(a.run.amountRaw) * y;
    return difference < 0n ? -1 : difference > 0n ? 1 : 0;
  };
  const owner = (row: RaceRow) => row.tags.ownerTag ?? `Country ${row.pair.ownerId}`;
  const target = (row: RaceRow) => row.tags.targetTag ?? `Country ${row.pair.targetId}`;
  return rows.sort((a, b) => (order === 'decryptor' ? compareTags(owner(a), owner(b)) || compareTags(a.pair.ownerId, b.pair.ownerId) : 0)
    || progress(a, b) || compareTags(owner(a), owner(b)) || compareTags(target(a), target(b)) || compareTags(a.key, b.key));
}

export function observationDate(observation: CipherObservation | null, hour: number): string {
  if (!observation || observation.hour > hour) return 'Not recorded';
  const date = gameDate(observation.hour);
  return `${observation.upperBound ? 'by ' : ''}${date ? formatDate(date) : `hour ${observation.hour}`}`;
}

/** The shared native port in dashboard/cryptology.rs displays bonus days only while active.
 * 1413E1DE0 decrements it from CCountry::daily_update@1406DAAF0 via 140FC6AB0;
 * expiry stores -1 and clears active. Preserve the signed native duration. */
export const activeBonusDays = (run: DecryptionValue): number | null => run.broken === true && run.bonusActive !== 0 ? run.bonusDays : null;
export const amountText = (run: DecryptionValue) => formatAmount(run.amountRaw, 5, 5);
export const CIPHER_BROKEN = 'Cipher broken';
export const progressLabel = (run: DecryptionValue) => run.progressPercent == null ? 'Progress unknown' : `${run.progressPercent}%`;
export const progressText = (run: DecryptionValue) => run.progressPercent == null ? `${progressLabel(run)} · ${amountText(run)}` : progressLabel(run);
export const workingStatus = (run: DecryptionValue, capitalized = false) => {
  const status = run.decrypting !== 0 ? 'decrypting' : 'paused';
  return capitalized ? status[0].toUpperCase() + status.slice(1) : status;
};
export const cipherStatus = (row: RaceRow) => row.run.broken === true ? `${CIPHER_BROKEN} ${observationDate(row.broken, row.hour)}`
  : row.run.broken === null ? `${workingStatus(row.run, true)}; cipher status unknown` : workingStatus(row.run, true);

export function decryptionTooltip(row: RaceRow, ownerName: string, targetName: string, height: number): TooltipContent {
  const bonus = activeBonusDays(row.run);
  const content: TooltipContent = { title: `${ownerName} → ${targetName}`, rows: [
    { label: 'Progress', value: progressText(row.run) },
    { label: 'Started', value: observationDate(row.run.started, row.hour) },
    { label: 'Bonus days', value: bonus == null ? 'none' : String(bonus) },
    { label: 'Status', value: cipherStatus(row), divider: true },
  ], note: row.run.thresholdUnknown ?? (row.run.progressPercent == null ? NATIVE_PROGRESS_UNKNOWN : undefined) };
  return fitTooltip([content], height, { keep: 1 });
}
