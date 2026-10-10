import { pinnedBreakdown } from '../../charts/drill/drillState.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import type { TensionEvent } from '../../generated/dashboard';
import type { StackRun } from '../../charts/stack';
import type { Run, Span } from '../../charts/series';
import { formatFixed, rescale } from '../../charts/valueAxis.ts';
import { lookPlan } from '../../dashboard/countryLooks.ts';
import type { RecordedBranch } from '../../dashboard/countryLooks';
import { runAt } from '../../charts/series.ts';
import { contributionAt, contributionPercent, layerTagAt, readAt, tensionPercent } from './tensionData.ts';
import type { TensionModel } from './tensionData';

export const TENSION_BREAKDOWN = { id: 'breakdown', label: 'Country contributions' };

/** Native canonical contributor slots outlive countries. Date cached-tag looks
 * to the proven holder, never to another country that later reused its tag. */
export function tensionLookAt(model: TensionModel, key: string, hour: number, branch: RecordedBranch) {
  const identities = model.identities.get(key) ?? [];
  const origin = runAt(identities, model.raw.get(key)?.[0]?.firstHour ?? NaN);
  if (!origin) return null;
  const holders = identities.filter(identity => identity.firstHour <= hour);
  const holder = holders.at(-1);
  if (!holder || holder.countryId !== origin.countryId || holder.slot !== origin.slot) return null;
  if (branch.gaps.some(gap => holder.lastHour < gap.firstHour && gap.lastHour < hour)) return null;
  const tag = model.tagOf.get(key)!;
  return lookPlan(branch, [tag], Math.min(hour, holder.lastHour), () => holder.firstHour).samples[0] ?? null;
}

export function tensionVisibleParts(parts: readonly { key: string; tag: string | null }[], ids: readonly string[]) {
  const visible = new Set(ids);
  return parts.filter(part => visible.has(part.key));
}

/** Shares retain each contribution's sign against the exact sum's magnitude.
 * Missing or zero totals cannot establish a share. */
export function tensionShareText(raw: string, total: string | null): string {
  if (total == null || BigInt(total) === 0n) return 'Not known';
  const denominator = BigInt(total);
  const numerator = BigInt(raw) * 100n;
  return `${formatFixed(rescale(numerator, denominator < 0n ? -denominator : denominator, 2), 2, 2)}%`;
}

function append(runs: StackRun[], firstHour: number, lastHour: number, value: number, startsSegment: boolean) {
  const previous = runs.at(-1);
  if (previous && previous.lastHour + 1 === firstHour && !startsSegment && previous.value === value) previous.lastHour = lastHour;
  else runs.push({ firstHour, lastHour, value, startsSegment, ...(startsSegment ? { breakBefore: true } : {}) });
}

/** Retain native changes, not a country-by-daily-read matrix. Country arrays are
 * sparse: absence within a complete global read is zero, while unread hours break
 * every band. Residuals use exact event deltas before converting for geometry. */
export function tensionDrillHistory(model: TensionModel, keys: readonly string[]) {
  const coverage: Span[] = [];
  const world: Run[] = [];
  const deltas = new Map<number, bigint>();
  const delta = (hour: number, value: bigint) => deltas.set(hour, (deltas.get(hour) ?? 0n) + value);
  for (const read of model.reply.reads) {
    const previous = coverage.at(-1);
    if (previous && previous.lastHour + 1 === read.firstHour && !read.startsSegment) previous.lastHour = read.lastHour;
    else coverage.push({ firstHour: read.firstHour, lastHour: read.lastHour });
    append(world, read.firstHour, read.lastHour, tensionPercent(model, read.tensionRaw), read.startsSegment);
    delta(read.firstHour, BigInt(read.sumRaw));
    delta(read.lastHour + 1, -BigInt(read.sumRaw));
  }
  const bands = keys.map(key => {
    const runs = model.raw.get(key) ?? [];
    const band: StackRun[] = [];
    let index = 0;
    for (const span of coverage) {
      let first = span.firstHour;
      while (first <= span.lastHour) {
        while (index < runs.length && runs[index].lastHour < first) index++;
        const run = runs[index];
        const known = run && run.firstHour <= first;
        const last = Math.min(span.lastHour, known ? run.lastHour : (run?.firstHour ?? Infinity) - 1);
        const raw = known ? run.valueRaw : '0';
        const startsSegment = first === span.firstHour || Boolean(known && run.firstHour === first && run.startsSegment);
        append(band, first, last, contributionPercent(model, raw), startsSegment);
        delta(first, -BigInt(raw));
        delta(last + 1, BigInt(raw));
        first = last + 1;
      }
    }
    return band;
  });
  const changes = [...deltas].filter(([, value]) => value !== 0n).sort(([a], [b]) => a - b);
  const residual: StackRun[] = [];
  let index = 0;
  let raw = 0n;
  for (const span of coverage) {
    while (index < changes.length && changes[index][0] < span.firstHour) raw += changes[index++][1];
    let first = span.firstHour;
    while (first <= span.lastHour) {
      while (index < changes.length && changes[index][0] === first) raw += changes[index++][1];
      const last = Math.min(span.lastHour, (changes[index]?.[0] ?? Infinity) - 1);
      append(residual, first, last, contributionPercent(model, raw.toString()), first === span.firstHour);
      first = last + 1;
    }
  }
  return { bands: [...bands, residual], world };
}

/** Country aggregates are supplied independently of the source list.
 * Keep every recorded identity and the exact difference from their native sum. */
export function tensionDrillAt(model: TensionModel, hour: number) {
  const read = readAt(model, hour);
  if (!read) return null;
  const parts = [...model.raw.keys()].map(key => ({ key, tag: layerTagAt(model, key, hour), raw: contributionAt(model, key, hour)! }));
  const known = parts.reduce((sum, part) => sum + BigInt(part.raw), 0n);
  return { read, parts, remainder: (BigInt(read.sumRaw) - known).toString() };
}

/** An event's final_raw is its post-ramp amount, not its current contribution.
 * Only report a value after its supplying read, dated to that read; disappearance
 * or missing current-source operands never proves an event contributes zero. */
export function tensionCauseAt(event: TensionEvent, hour: number) {
  const limit = Math.min(Math.floor(hour), event.lastRead);
  let found: TensionEvent['added'][number] | null = null;
  for (const added of event.added) if (added.hour <= limit) found = added;
  return found;
}

export function pinTensionDrill(source: WidgetSpec, track: Pick<BoundTrack, 'id' | 'branch'>, hour: number, follow: boolean): WidgetSpec {
  const pinned = pinnedBreakdown(source, TENSION_BREAKDOWN, track.id);
  const offset = Math.floor(hour) - track.branch.firstHour;
  return { ...pinned, time: follow ? { mode: 'follow' } : { mode: 'pin', windows: { [track.id]: { firstOffset: offset, lastOffset: offset } } } };
}
