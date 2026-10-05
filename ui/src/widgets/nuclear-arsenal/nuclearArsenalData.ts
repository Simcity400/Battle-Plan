import type { CountryRaceEntry } from '../../charts/race/countryRace';
import { shownRuns } from '../../charts/race/lines.ts';
import { runAt, runIndexAt, stretchStarts } from '../../charts/series.ts';
import type { Run, Span } from '../../charts/series';
import type { TooltipContent } from '../../charts/tooltip';
import { compareTags } from '../../dashboard/tagOrder.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { readAt } from '../../dashboard/reads.ts';
import { validSpans } from '../../dashboard/spans.ts';
import type { CountryNukesReply, NukeRun, NukesParams } from '../../generated/dashboard';

const integer = (value: unknown) => Number.isSafeInteger(value);

export function validateNukesReply(data: unknown): CountryNukesReply {
  const reply = data as CountryNukesReply;
  const ok = reply != null && typeof reply === 'object' && typeof reply.branchId === 'string'
    && Array.isArray(reply.readHours) && reply.readHours.every((hour, index) => integer(hour) && (index === 0 || reply.readHours[index - 1] < hour))
    && Array.isArray(reply.emptyReadHours) && reply.emptyReadHours.every((hour, index) => integer(hour)
      && (index === 0 || reply.emptyReadHours[index - 1] < hour) && readAt(reply.readHours, hour) === hour)
    && Array.isArray(reply.types) && Array.isArray(reply.series) && Array.isArray(reply.warnings) && reply.warnings.every(item => typeof item === 'string')
    && new Set(reply.types.map(type => type.ordinal)).size === reply.types.length
    && reply.types.every(type => integer(type.ordinal) && type.ordinal >= 0 && typeof type.supported === 'boolean'
      && (type.token === null || typeof type.token === 'string') && (type.name === null || typeof type.name === 'string'))
    && new Set(reply.series.map(series => series.tag)).size === reply.series.length
    && reply.series.every(series => typeof series.tag === 'string' && Array.isArray(series.runs) && Array.isArray(series.segments)
      && validSpans(series.segments) && (series.membership === null || validSpans(series.membership))
      && series.runs.every((run, index) => integer(run.firstHour) && integer(run.lastHour) && run.firstHour <= run.lastHour
        && typeof run.startsSegment === 'boolean' && (run.total === null || integer(run.total))
        && (index === 0 || series.runs[index - 1].lastHour < run.firstHour) && Array.isArray(run.counts)
        && new Set(run.counts.map(count => count.ordinal)).size === run.counts.length
        && run.counts.every(count => integer(count.ordinal) && integer(count.value) && reply.types.some(type => type.ordinal === count.ordinal))));
  if (!ok) throw new Error('The nuclear stockpile reply is malformed.');
  return reply;
}

export function nukesRequest(branchId: string, resolved: { tags: readonly string[]; top?: number }): NukesParams {
  return { branchId, tags: resolved.top == null ? [...resolved.tags] : [], top: resolved.top ?? null };
}

export function nukeTypeName(type: CountryNukesReply['types'][number]): string {
  return type.name ?? (type.token ? `Unknown name (${type.token})` : `Unknown type ${type.ordinal}`);
}
export const formatNukes = (value: number) => value.toLocaleString('en-US');
export function nuclearReadNote(model: NuclearModel, hour: number): string {
  const read = readAt(model.reply.readHours, hour);
  return `${read == null ? 'Read time unknown' : `Read ${formatDateTime(read)}`}. The game counter counts the stockpile. The recorded “ready” field counts queued drops after their delay and is outside this widget.`;
}

export function nuclearModel(reply: CountryNukesReply) {
  return { reply, tags: reply.series.map(series => series.tag),
    series: new Map(reply.series.map(series => [series.tag, { ...series, starts: stretchStarts(series.runs) }])),
    parts: reply.types.map(type => ({ id: String(type.ordinal), label: nukeTypeName(type) })),
  };
}
export type NuclearModel = ReturnType<typeof nuclearModel>;

export function nuclearSegmentAt(model: NuclearModel | null, tag: string, hour: number): number | null {
  return runAt(model?.series.get(tag)?.segments ?? [], Math.floor(hour))?.firstHour ?? null;
}
export function nukesAt(model: NuclearModel, tag: string, hour: number): { run: NukeRun; stretch: number } | null {
  const series = model.series.get(tag);
  if (!series) return null;
  const index = runIndexAt(series.runs, Math.floor(hour));
  return index < 0 ? null : { run: series.runs[index], stretch: series.starts[index] };
}
export function rankNukes(model: NuclearModel, candidates: readonly string[], hour: number, count: number, omitZero = false): CountryRaceEntry[] {
  return candidates.flatMap(tag => {
    const at = nukesAt(model, tag, hour);
    return at?.run.total == null || nuclearSegmentAt(model, tag, hour) == null || (omitZero && at.run.total === 0) ? []
      : [{ tag, value: at.run.total, label: formatNukes(at.run.total), stretch: at.stretch,
      parts: at.run.counts.map(part => ({ id: String(part.ordinal), value: part.value })) }];
  }).sort((a, b) => b.value - a.value || compareTags(a.tag, b.tag)).slice(0, Math.max(0, count));
}

/** Unknown totals are absent plotting runs; never connected through their hours. */
export function nuclearRuns(model: NuclearModel, tag: string, membership: readonly Span[] | null, range: Span, ordinal?: number): Run[] {
  const runs: Run[] = [];
  for (const read of model.series.get(tag)?.runs ?? []) {
    const value = read.total == null ? null : ordinal == null ? read.total : read.counts.find(count => count.ordinal === ordinal)?.value ?? null;
    if (value == null) continue;
    const previous = runs.at(-1);
    if (previous && !read.startsSegment && previous.lastHour + 1 === read.firstHour && previous.value === value) previous.lastHour = read.lastHour;
    else runs.push({ firstHour: read.firstHour, lastHour: read.lastHour, value, ...(read.startsSegment ? { breakBefore: true } : {}) });
  }
  return shownRuns(runs, membership, range);
}

export function nuclearTooltip(model: NuclearModel, tag: string, hour: number, name: string, colours: ReadonlyMap<string, string>, date: string): TooltipContent[] | null {
  const at = nukesAt(model, tag, hour);
  if (!at) return null;
  return [{ title: name, subtitle: date,
    rows: [
      ...model.reply.types.map(type => ({ label: nukeTypeName(type), value: (() => {
        const value = at.run.counts.find(count => count.ordinal === type.ordinal)?.value;
        return value == null ? 'Unknown' : formatNukes(value);
      })(), swatch: colours.get(String(type.ordinal)) })),
      { label: 'Nuclear stockpile (game counter)', value: at.run.total == null ? 'Unknown' : formatNukes(at.run.total), strong: true, divider: true },
    ], note: nuclearReadNote(model, hour),
  }];
}
