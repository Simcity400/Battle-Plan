import { contentTitle } from '../../dashboard/contentTitles.ts';
import type { CountryRaceEntry } from '../../charts/race/countryRace';
import { shownRuns } from '../../charts/race/lines.ts';
import { isOrderedSpans, runAt, runIndexAt, stretchStarts } from '../../charts/series.ts';
import type { Run, Span } from '../../charts/series';
import { usNumber } from '../../charts/numberFormat.ts';
import type { TooltipContent } from '../../charts/tooltip';
import { compareTags } from '../../dashboard/tagOrder.ts';
import { readAt } from '../../dashboard/reads.ts';
import type { CountryNukesReply, NukeRun, NukesParams } from '../../generated/dashboard';
import { pinPartsRace } from '../../charts/race/partsRaceDrill.ts';
import { CLOSED, reduceDrill } from '../../charts/drill/drillState.ts';
import type { DrillAction, DrillState } from '../../charts/drill/drillState';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';

const integer = (value: unknown) => Number.isSafeInteger(value);

export interface NuclearDrillState { key: string | null; drill: DrillState }
export interface NuclearOrigin { key: string; tag: string; hour: number }

export function reduceNuclearDrill(state: NuclearDrillState, action: { key: string; action: DrillAction }): NuclearDrillState {
  return { key: action.key, drill: reduceDrill(state.key === action.key ? state.drill : CLOSED, action.action) };
}

/** Old reducer state cannot select an actor or acquire a new context before the
 * reset effect runs. Freezing an origin does not change the reducer's owner. */
export function nuclearDrillSelection(state: NuclearDrillState, key: string, selection: NuclearOrigin | null,
  pinned: boolean, mixTag: string | null, hour: number) {
  const drill = state.key === key ? state.drill : CLOSED;
  const origin = selection?.key === key ? selection : null;
  const tag = drill.breakdown ?? (pinned ? origin?.tag ?? mixTag : null);
  return { drill, origin, tag, hour: origin?.tag === tag ? origin.hour : hour };
}

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
      && isOrderedSpans(series.segments) && (series.membership === null || isOrderedSpans(series.membership))
      && series.runs.every((run, index) => integer(run.firstHour) && integer(run.lastHour) && run.firstHour <= run.lastHour
        && typeof run.startsSegment === 'boolean' && (run.total === null || integer(run.total))
        && (index === 0 || series.runs[index - 1].lastHour < run.firstHour) && Array.isArray(run.counts)
        && new Set(run.counts.map(count => count.ordinal)).size === run.counts.length
        && run.counts.every(count => integer(count.ordinal) && integer(count.value) && reply.types.some(type => type.ordinal === count.ordinal))));
  if (!ok) throw new Error('The nuclear stockpile reply is malformed.');
  return reply;
}

export function nukesRequest(branchId: string, resolved: { tags: readonly string[]; top?: number }, originHour: number | null = null): NukesParams {
  return { branchId, tags: resolved.top == null ? [...resolved.tags] : [], top: resolved.top ?? null, originHour: originHour == null ? null : Math.floor(originHour) };
}

/** The origin-hour product clips aliases to one recorded native (slot,id).
 * Retain their dated tags for looks while joining their numeric history. */
export function nuclearDrillAliasAt(reply: CountryNukesReply | null, hour: number): string | null {
  const aliases = reply?.series.filter(series => runAt(series.membership ?? [], Math.floor(hour))) ?? [];
  return aliases.length === 1 ? aliases[0].tag : null;
}

export function nuclearIdentityModel(reply: CountryNukesReply, tag: string): NuclearModel {
  const observed = reply.series.flatMap(series => series.runs.map(run => ({ series, run })))
    .sort((a, b) => a.run.firstHour - b.run.firstHour);
  const runs = observed.map(({ series, run }, index) => {
    const previous = observed[index - 1];
    const before = previous && runAt(previous.series.membership ?? [], previous.run.lastHour);
    const after = runAt(series.membership ?? [], run.firstHour);
    // Adjacent origin memberships prove a rename of the same native country.
    // Clear the tag-only boundary, retaining missing reads and unknown totals.
    const rename = previous && previous.series.tag !== series.tag && previous.run.total != null && run.total != null
      && previous.run.lastHour + 1 === run.firstHour && before && after && before.lastHour + 1 === after.firstHour;
    return rename && run.startsSegment ? { ...run, startsSegment: false } : run;
  });
  const segments: Span[] = [];
  for (const span of reply.series.flatMap(series => series.segments).sort((a, b) => a.firstHour - b.firstHour)) {
    const previous = segments.at(-1);
    if (previous && previous.lastHour + 1 === span.firstHour) previous.lastHour = span.lastHour;
    else segments.push({ ...span });
  }
  const series = { tag, membership: null, runs, segments };
  return nuclearModel({ ...reply, series: [series] });
}

export function validateNuclearDrillReply(data: unknown): CountryNukesReply {
  const reply = validateNukesReply(data);
  const spans = reply.series.flatMap(series => series.membership ?? []).sort((a, b) => a.firstHour - b.firstHour);
  if (reply.series.some(series => series.membership == null) || !isOrderedSpans(spans)) {
    throw new Error('The selected nuclear country has conflicting or missing recorded identity spans.');
  }
  return reply;
}

export function pinNuclearDrill(widget: WidgetSpec, track: Pick<BoundTrack, 'id' | 'branch'>, tag: string,
  hour: number, originHour: number, follow: boolean): WidgetSpec {
  return pinPartsRace({ ...widget, time: follow ? { mode: 'follow' } : widget.time,
    options: { ...widget.options, drillOriginHour: Math.floor(originHour) } }, track, tag, hour, follow, 'Weapon types');
}

export function nukeTypeName(type: CountryNukesReply['types'][number]): string {
  return contentTitle(type.name, type.token, 'Nuclear weapon name unavailable');
}
export const formatNukes = (value: number) => usNumber.format(value);

export function nuclearModel(reply: CountryNukesReply) {
  return { reply, tags: reply.series.map(series => series.tag),
    series: new Map(reply.series.map(series => [series.tag, { ...series,
      starts: stretchStarts(series.runs.map((run, index) => ({ ...run, breakBefore: run.total === null || series.runs[index - 1]?.total === null }))),
    }])),
    parts: reply.types.map(type => ({ id: String(type.ordinal), label: nukeTypeName(type) })),
  };
}
export type NuclearModel = ReturnType<typeof nuclearModel>;

/** Content labels may refresh without changing the recorded actor geometry. */
export function nuclearNumbers(reply: CountryNukesReply): CountryNukesReply {
  return { ...reply, warnings: [], types: reply.types.map(type => ({ ...type, name: null })) };
}

/** The game adds signed dwords. Keep an exact wrapping
 * difference separate from identified types, and never fill an absent ordinal. */
export function nuclearDrillAt(model: NuclearModel, tag: string, hour: number) {
  const at = nukesAt(model, tag, hour);
  if (!at || nuclearSegmentAt(model, tag, hour) == null) return null;
  const counts = new Map(at.run.counts.map(part => [part.ordinal, part.value]));
  const values = model.reply.types.map(type => counts.get(type.ordinal) ?? null);
  return { total: at.run.total, values,
    remainder: at.run.total == null ? null : at.run.total - values.reduce<number>((sum, value) => sum + (value ?? 0), 0) };
}

/** Parts and their exact residual share the total's known hours. A missing total
 * breaks every band even when the read retained an individual type count. */
export function nuclearDrillBands(model: NuclearModel, tag: string, membership: readonly Span[] | null, range: Span): Run[][] {
  const bands: Run[][] = Array.from({ length: model.reply.types.length + 1 }, () => []);
  for (const read of model.series.get(tag)?.runs ?? []) {
    const at = nuclearDrillAt(model, tag, read.firstHour);
    if (!at || at.total == null) continue;
    bands.forEach((runs, index) => {
      const value = index < at.values.length ? at.values[index] ?? 0 : at.remainder!;
      const previous = runs.at(-1);
      if (previous && !read.startsSegment && previous.lastHour + 1 === read.firstHour && previous.value === value) previous.lastHour = read.lastHour;
      else runs.push({ firstHour: read.firstHour, lastHour: read.lastHour, value, ...(read.startsSegment ? { breakBefore: true } : {}) });
    });
  }
  return bands.map(runs => shownRuns(runs, membership, range));
}

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
      { label: 'Nuclear stockpile', value: at.run.total == null ? 'Unknown' : formatNukes(at.run.total), strong: true, divider: true },
    ],
  }];
}
