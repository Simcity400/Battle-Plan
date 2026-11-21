import type { CountryRef } from '../../generated/dashboard';
import { countryKey } from '../../dashboard/countryKey.ts';
import type { CountryIdentityRun, ForcePart, ForceSizeReply } from '../../generated/dashboard';
import type { PartsRaceAt, PartsRaceRun } from '../../dashboard/partsRace';
import { validatePartsRaceReply } from '../../dashboard/partsRace.ts';
import type { NativePartsRaceReply } from './partsRaceDrill';
import type { Span } from '../series';
import { CELL, cellNumber, cellState, cellValue, rowName } from '../../dashboard/follow/message.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import { isMember } from '../../dashboard/groups.ts';
import type { ResolvedCountries } from '../../dashboard/groups';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { CountryRaceEntry } from './countryRace';

export type ForceParams = Record<string, unknown> & { branchId: string; countries: CountryRef[]; top: number | null };

const span = (value: unknown): value is Span => !!value && typeof value === 'object'
  && Number.isSafeInteger((value as Span).firstHour) && Number.isSafeInteger((value as Span).lastHour) && (value as Span).firstHour <= (value as Span).lastHour;

export function validateForceReply(value: unknown, referenced = false): ForceSizeReply {
  const reply = value as ForceSizeReply;
  const fail = () => { throw new Error('The force race reply is malformed.'); };
  if (!reply || typeof reply !== 'object' || !Array.isArray(reply.windows) || !reply.windows.every(span)
    || !(reply.window === null || Number.isSafeInteger(reply.window)) || !(reply.interval === null || span(reply.interval))) return fail();
  if (!Array.isArray(reply.series)) return fail();
  validatePartsRaceReply({ ...reply, series: reply.series.map(series => ({ ...series, tag: countryKey(series.country) })) });
  for (const series of reply.series) {
    if (!Array.isArray(series.identities) || series.identities.some(run => !span(run))) return fail();
    if (referenced && series.runs.some(run => run.referenceTotal === undefined)) return fail();
  }
  return reply;
}

export function forceWindows(windows: readonly Span[], range: Span, at: number, following: boolean): number[] {
  const out: number[] = [];
  windows.forEach((window, index) => {
    if (window.lastHour >= range.firstHour && window.firstHour <= range.lastHour) out.push(index);
  });
  if (following) {
    const current = windows.findIndex(window => window.firstHour <= at && at <= window.lastHour);
    if (current >= 0 && current + 1 < windows.length && !out.includes(current + 1)) out.push(current + 1);
  }
  return out.sort((a, b) => a - b);
}

function joinSpans(spans: readonly Span[]): Span[] {
  const out: Span[] = [];
  for (const next of [...spans].sort((a, b) => a.firstHour - b.firstHour)) {
    const last = out.at(-1);
    if (last && next.firstHour <= last.lastHour + 1) last.lastHour = Math.max(last.lastHour, next.lastHour);
    else out.push({ firstHour: next.firstHour, lastHour: next.lastHour });
  }
  return out;
}

function joinIdentities(runs: readonly CountryIdentityRun[]): CountryIdentityRun[] {
  const out: CountryIdentityRun[] = [];
  for (const run of [...runs].sort((a, b) => a.firstHour - b.firstHour)) {
    const last = out.at(-1);
    if (last && last.slot === run.slot && last.countryId === run.countryId && run.firstHour <= last.lastHour + 1) last.lastHour = Math.max(last.lastHour, run.lastHour);
    else out.push({ ...run });
  }
  return out;
}

export function joinForceWindows(manifest: Pick<ForceSizeReply, 'branchId' | 'warnings'>, windows: ReadonlyMap<number, ForceSizeReply>): NativePartsRaceReply {
  const held = [...windows.entries()].sort((a, b) => a[0] - b[0]).map(entry => entry[1]);
  const parts = new Map<string, ForcePart>();
  for (const reply of held) for (const part of reply.parts) if (!parts.has(part.key)) parts.set(part.key, part);
  const keys = [...parts.keys()].sort((a, b) => a < b ? -1 : a > b ? 1 : 0);
  const at = new Map(keys.map((key, index) => [key, index]));
  const series = new Map<string, { runs: PartsRaceRun[]; segments: Span[]; identities: CountryIdentityRun[]; membership: Span[] | null }>();
  for (const reply of held) {
    const index = reply.parts.map(part => at.get(part.key)!);
    for (const line of reply.series) {
      const tag = countryKey(line.country);
      const into = series.get(tag) ?? { runs: [], segments: [], identities: [], membership: line.membership === null ? null : [] };
      series.set(tag, into);
      for (const run of line.runs) {
        const known = run.values.every(value => value !== null) && run.total !== null;
        const values: (number | null)[] = keys.map(() => known ? 0 : null);
        run.values.forEach((value, part) => { if (known) values[index[part]] = value; });
        into.runs.push({ firstHour: run.firstHour, lastHour: run.lastHour, startsSegment: run.startsSegment, total: run.total, values,
          ...(run.referenceTotal !== undefined ? { referenceTotal: run.referenceTotal } : {}) });
      }
      into.segments.push(...line.segments);
      into.identities.push(...line.identities);
      if (into.membership && line.membership) into.membership.push(...line.membership);
    }
  }
  return {
    branchId: manifest.branchId,
    parts: keys.map(key => parts.get(key)!),
    series: [...series.entries()].map(([tag, line]) => ({ tag, runs: line.runs.sort((a, b) => a.firstHour - b.firstHour),
      segments: joinSpans(line.segments), identities: joinIdentities(line.identities), membership: line.membership && joinSpans(line.membership) })),
    warnings: [...new Set([...manifest.warnings, ...held.flatMap(reply => reply.warnings)])],
  };
}

export function followedForceEntries(answer: FollowedAnswer | undefined, asked: readonly string[] | null, resolved: Pick<ResolvedCountries, 'membership'>,
  hour: number, slots: number, format: (value: number) => string): CountryRaceEntry[] {
  if (!answer || answer.kind === 'notRecorded') return [];
  const rows: { tag: string; raw: number }[] = [];
  for (let row = 0; row < answer.rows; row++) {
    const cell = row * answer.columns;
    if (cellState(answer, cell) !== CELL.value) continue;
    const tag = rowName(answer, row) ?? asked?.[row];
    if (!tag || (resolved.membership && !isMember(resolved.membership.get(tag), hour))) continue;
    rows.push({ tag, raw: Number(cellNumber(answer, cell) ?? cellValue(answer, cell)) });
  }
  rows.sort((a, b) => b.raw - a.raw || compareTags(a.tag, b.tag));
  return rows.slice(0, Math.max(0, slots)).map(({ tag, raw }) => ({ tag, value: raw, label: format(raw), stretch: null, hour }));
}

export function breakdownWindows(windows: readonly Span[], view: Span, extent: readonly [number, number], at: number, origin: number, following: boolean): number[] {
  const range = { firstHour: Math.max(extent[0], view.firstHour), lastHour: Math.min(extent[1], view.lastHour) };
  return [...new Set([...forceWindows(windows, range, at, following), ...forceWindows(windows, { firstHour: origin, lastHour: origin }, origin, false),
    ...forceWindows(windows, { firstHour: at, lastHour: at }, at, false)])].sort((a, b) => a - b);
}

export function breakdownValue(at: PartsRaceAt | null) {
  const remainder = at?.total == null ? null : at.total - at.values.reduce<number>((sum, value) => sum + (value ?? 0), 0);
  return { total: at?.total ?? null, values: at?.values ?? [], remainder, reference: at?.run.referenceTotal ?? null };
}

export function forceRaceEmpty(subject: string, day: string, answer: FollowedAnswer | undefined): string {
  return answer?.kind === 'notRecorded' ? `${subject[0]!.toUpperCase()}${subject.slice(1)} not recorded on ${day}: ${answer.reason}`
    : `No ${subject} recorded on ${day}.`;
}
