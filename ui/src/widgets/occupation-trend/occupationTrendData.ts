import type { ChartSummary } from '../../charts/chartSummary';
import { SECONDARY_DASH } from '../../charts/palette.ts';
import type { CountryRaceEntry, CountryRaceLine } from '../../charts/race/countryRace';
import type { LabelAt } from '../../charts/race/lines';
import { shownRuns } from '../../charts/race/lines.ts';
import { clipToSpans, isOrderedSpans, runAt, runIndexAt, spansCovered, stretchStarts } from '../../charts/series.ts';
import type { Span } from '../../charts/series';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { fixedNumber, formatAmount } from '../../charts/valueAxis.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { countryDrillSpans, countryDrillTagAt, countryIdentityIndex, validCountryIdentityRuns } from '../../dashboard/countryIdentity.ts';
import { compareTags } from '../../dashboard/tagOrder.ts';
import { int, object, opaqueTag as tag, signed32, signed64 as raw, unsigned32 } from '../../dashboard/validation.ts';
import type { OccupationCollaboration, OccupationCollaborationActor, OccupationTrendParams, OccupationTrendReply, OccupationTrendRun } from '../../generated/dashboard';

export type OccupationMeasure = 'resistance' | 'compliance';
export type ShownMeasure = OccupationMeasure | 'both';
export const measureName = (measure: OccupationMeasure) => measure === 'resistance' ? 'Resistance' : 'Compliance';
export const rankingMeasure = (shown: ShownMeasure, ranking: OccupationMeasure): OccupationMeasure => shown === 'both' ? ranking : shown;

export function validateOccupationTrendReply(data: unknown): OccupationTrendReply {
  if (!object(data) || typeof data.branchId !== 'string' || !/^\d+$/.test(data.branchId) || data.decimals !== 5
    || !Array.isArray(data.series) || !Array.isArray(data.warnings) || !data.warnings.every(warning => typeof warning === 'string')
    || !data.series.every(series => object(series) && tag(series.tag) && isOrderedSpans(series.runs) && isOrderedSpans(series.segments)
      && (series.membership === null || isOrderedSpans(series.membership)) && validCountryIdentityRuns(series.identities) && spansCovered(series.runs, series.segments)
      && series.runs.every((run, index, runs) => int(run.readHour) && run.readHour <= run.firstHour && typeof run.startsSegment === 'boolean'
        && raw(run.resistanceRaw) && raw(run.complianceRaw) && int(run.states) && run.states > 0
        && int(run.heldStates) && run.heldStates >= run.states && typeof run.heldComplete === 'boolean'
        && (index === 0 || Number(runs[index - 1].readHour) <= run.readHour)
        && Array.isArray(run.collaboration) && run.collaboration.every(item => object(item) && (item.tag === null || tag(item.tag))
          && (item.raw === null || raw(item.raw)) && (item.readHour === null || (int(item.readHour) && item.readHour <= run.firstHour))
          && (item.actor === undefined || object(item.actor) && unsigned32(item.actor.slot) && unsigned32(item.actor.countryId)
            && (item.actor.nameHour === null || signed32(item.actor.nameHour) && item.actor.nameHour <= run.firstHour))
          && (item.tag !== null || item.actor !== undefined && item.actor.nameHour === null)
          && (item.raw === null) === (item.readHour === null))
        && new Set(run.collaboration.map(item => JSON.stringify(item.actor ? ['native', item.actor.slot, item.actor.countryId] : ['tag', item.tag]))).size === run.collaboration.length))
    || new Set(data.series.map(series => series.tag)).size !== data.series.length) {
    throw new Error('The occupation trend reply is malformed.');
  }
  return data as unknown as OccupationTrendReply;
}

export function occupationRequest(branchId: string, tags: readonly string[], top: number | undefined, measure: OccupationMeasure, collaboration = false): OccupationTrendParams {
  return { branchId, tags: top == null ? [...tags] : [], top: top == null ? null : { count: top, measure }, collaboration };
}
export function occupationModel(reply: OccupationTrendReply) {
  return { reply, identities: reply.series.every(series => series.identities !== undefined)
    ? countryIdentityIndex(reply.series.map(series => ({ tag: series.tag, identities: series.identities! }))) : null,
    tags: reply.series.map(series => series.tag), series: new Map(reply.series.map(series => [series.tag,
    { ...series, starts: stretchStarts(series.runs) }])) };
}
export type OccupationModel = ReturnType<typeof occupationModel>;
export function occupationSegmentAt(model: OccupationModel | null, tag: string, hour: number): number | null {
  const series = model?.series.get(tag);
  return runAt(series?.identities ?? series?.segments ?? [], Math.floor(hour))?.firstHour ?? null;
}
export function occupationAt(model: OccupationModel, tag: string, hour: number): { run: OccupationTrendRun; stretch: number } | null {
  const series = model.series.get(tag);
  if (!series || occupationSegmentAt(model, tag, hour) === null) return null;
  const index = runIndexAt(series.runs, Math.floor(hour));
  return index < 0 ? null : { run: series.runs[index], stretch: series.starts[index] };
}
const measureRaw = (run: OccupationTrendRun, measure: OccupationMeasure) => measure === 'resistance' ? run.resistanceRaw : run.complianceRaw;
/** Means use the recorded state scalars; cached group fields are not captured.
 * Means stay exact outside geometry. */
export const occupationPercent = (value: string, digits = 5) => `${formatAmount(value, 5, digits)}%`;
export function occupationEntries(model: OccupationModel, candidates: readonly string[], hour: number, measure: OccupationMeasure, count: number, digits = 2): CountryRaceEntry[] {
  return candidates.flatMap(tag => {
    const at = occupationAt(model, tag, hour);
    if (!at) return [];
    const raw = measureRaw(at.run, measure);
    return [{ tag, raw, amount: BigInt(raw), stretch: at.stretch }];
  }).sort((a, b) => {
    const difference = b.amount - a.amount;
    return difference === 0n ? compareTags(a.tag, b.tag) : difference > 0n ? 1 : -1;
  }).slice(0, count).map(entry => ({ tag: entry.tag, value: fixedNumber(entry.raw, 5), label: occupationPercent(entry.raw, digits), stretch: entry.stretch }));
}
export function occupationLines(model: OccupationModel, tags: readonly string[], measure: ShownMeasure, range: Span,
  membership: ReadonlyMap<string, readonly Span[]> | null, digits = 2, summaries?: ReadonlyMap<string, ChartSummary>): CountryRaceLine[] {
  const measures: OccupationMeasure[] = measure === 'both' ? ['resistance', 'compliance'] : [measure];
  return tags.flatMap(tag => measures.flatMap(key => {
    const series = model.series.get(tag);
    const summary = summaries?.get(JSON.stringify([tag, key]));
    const measured = summary?.runs ?? series?.runs.map(run => ({ firstHour: run.firstHour, lastHour: run.lastHour,
      value: fixedNumber(measureRaw(run, key), 5), ...(run.startsSegment ? { breakBefore: true } : {}) })) ?? [];
    const spans = series?.identities ?? series?.segments ?? [];
    const starts = new Set(spans.map(span => span.firstHour));
    const identityRuns = clipToSpans(measured, spans).map(run => ({ ...run, breakBefore: run.breakBefore || starts.has(run.firstHour) }));
    const runs = shownRuns(identityRuns, membership ? membership.get(tag) ?? [] : series?.membership ?? null, range);
    return runs.length || summary?.buckets.some(Boolean) ? [{ tag, id: JSON.stringify([tag, key]), runs, ...(measure === 'both' && key === 'compliance' ? { dash: SECONDARY_DASH } : {}),
      endLabel: (hour: number, name: string) => {
        const at = occupationAt(model, tag, hour);
        const shown = measure === 'both' ? `${name} ${measureName(key)}` : name;
        return `${shown} ${at ? occupationPercent(measureRaw(at.run, key), digits) : 'Not recorded'}`;
      },
    }] : [];
  }));
}
export function occupationHolderMix(model: OccupationModel, tag: string, originHour: number, hour: number,
  measure: ShownMeasure, range: Span, membership: ReadonlyMap<string, readonly Span[]> | null, digits = 2, summaries?: ReadonlyMap<string, ChartSummary>) {
  const holder = countryDrillSpans(model.reply.series, tag, originHour);
  const counted = membership ? new Map([...holder].map(([alias, spans]) => [alias, clipToSpans(spans, membership.get(alias) ?? [])])) : holder;
  const alias = countryDrillTagAt(counted, hour);
  return { tag: alias ?? tag, available: alias !== null, membership: counted,
    lines: occupationLines(model, [...holder.keys()], measure, range, counted, digits, summaries) };
}

export function heldCoverage(run: OccupationTrendRun): string {
  return run.heldComplete ? `${run.states}/${run.heldStates} states` : `${run.states} states; total unknown`;
}
const collaborationNamed = (item: OccupationCollaboration): item is OccupationCollaboration & { tag: string; actor: OccupationCollaborationActor & { nameHour: number } } =>
  item.tag !== null && item.actor !== undefined && item.actor.nameHour !== null;
export function occupationTooltipTags(model: OccupationModel, hour: number, tags: readonly string[], collaboration: boolean): string[] {
  return collaboration ? [...new Set(tags.flatMap(tag => occupationAt(model, tag, hour)?.run.collaboration.filter(collaborationNamed).map(item => item.tag) ?? []))] : [];
}
function collaborationRows(run: OccupationTrendRun, hour: number, labelAt: LabelAt): TooltipRow[] {
  return run.collaboration.flatMap(item => {
    const named = collaborationNamed(item);
    const reason = item.actor === undefined ? 'Country identity was not recorded for this collaboration.'
      : 'The occupied country identity has no recorded name at this hour.';
    const label = named ? labelAt(hour)(item.tag) : { name: 'Country name unavailable', flagReason: reason, flagPending: false };
    return [{ label: `Country collaboration: ${label.name}`, countryFlag: label,
      value: item.raw === null ? 'Not recorded' : `${formatAmount(BigInt(item.raw) * 100n, 5, 5)}%` }];
  });
}
export function occupationTooltip(model: OccupationModel, tag: string, hour: number, labelAt: LabelAt, shown: ShownMeasure, collaboration: boolean): TooltipContent | null {
  const at = occupationAt(model, tag, hour);
  if (!at) return null;
  const label = labelAt(hour)(tag);
  return { title: label.name, subtitle: formatDateTime(hour), rows: [
    ...(shown !== 'compliance' ? [{ label: 'Average resistance', value: occupationPercent(at.run.resistanceRaw), strong: true }] : []),
    ...(shown !== 'resistance' ? [{ label: 'Average compliance', value: occupationPercent(at.run.complianceRaw), strong: true }] : []),
    { label: 'States', value: heldCoverage(at.run) },
    ...(collaboration ? collaborationRows(at.run, hour, labelAt) : []),
  ], note: `Average across ${at.run.states} states.` };
}
export function occupationLineTooltip(model: OccupationModel, hour: number, tags: readonly string[], labelAt: LabelAt,
  colour: (tag: string) => string, shown: ShownMeasure, rank: OccupationMeasure, collaboration: boolean): TooltipContent {
  const ranked = occupationEntries(model, tags, hour, rank, tags.length, 5);
  const rows = ranked.flatMap(entry => {
    const label = labelAt(hour)(entry.tag);
    const at = occupationAt(model, entry.tag, hour)!;
    return [{ label: label.name, value: entry.label, swatch: colour(entry.tag), strong: true, countryFlag: label },
      ...(shown === 'both' ? [{ label: `Average ${measureName(rank === 'resistance' ? 'compliance' : 'resistance').toLowerCase()}`,
        value: occupationPercent(measureRaw(at.run, rank === 'resistance' ? 'compliance' : 'resistance')) }] : []),
      { label: 'States', value: heldCoverage(at.run) },
      ...(collaboration ? collaborationRows(at.run, hour, labelAt) : [])];
  });
  return { title: formatDateTime(hour), subtitle: shown === 'both' ? `Occupation averages; ${measureName(rank).toLowerCase()} first` : `Average ${measureName(rank).toLowerCase()}`,
    rows: rows.length ? rows : [{ label: 'Not recorded', tone: 'muted' }],
  };
}
