import type { NavalLossesParams, NavalLossesReply, NavalLossRun } from '../../generated/dashboard';
import type { CountryRaceEntry, CountryRaceLine, CountryRacePart } from '../../charts/race/countryRace';
import type { LabelAt } from '../../charts/race/lines';
import { shownRuns } from '../../charts/race/lines.ts';
import { clipToSpans, isOrderedSpans, runAt, runIndexAt, stretchStarts } from '../../charts/series.ts';
import type { Span } from '../../charts/series';
import { usNumber } from '../../charts/numberFormat.ts';
import type { TooltipContent } from '../../charts/tooltip';
import type { CountryPickerRead } from '../../components/CountryPicker';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { sinceLabel, stretchSince } from '../../dashboard/stretch.ts';
import { countryIdentityIndex, validCountryIdentityRuns } from '../../dashboard/countryIdentity.ts';
import { compareTags } from '../../dashboard/tagOrder.ts';
import { readAt } from '../../dashboard/reads.ts';
import { int, object } from '../../dashboard/validation.ts';

export type NavalKind = NavalLossesParams['kind'];
export type NavalAttribution = NavalLossesParams['attribution'];
export type NavalPeriod = NavalLossesParams['period'];
const count = (value: unknown) => value === null || int(value);
const reads = (value: unknown): value is number[] => Array.isArray(value)
  && value.every((hour, index) => int(hour) && (index === 0 || value[index - 1] < hour));

export function validateNavalLossesReply(data: unknown, window = false): NavalLossesReply {
  const reply = data as NavalLossesReply;
  const ok = object(reply) && typeof reply.branchId === 'string' && typeof reply.available === 'boolean'
    && [reply.firstRead, reply.shipFirstRead, reply.convoyFirstRead, reply.shipLastRead, reply.convoyLastRead].every(value => value === null || int(value))
    && reads(reply.shipReads) && reads(reply.convoyReads)
    && (window ? reply.shipReads.every(hour => reply.shipFirstRead !== null && reply.shipLastRead !== null && hour >= reply.shipFirstRead && hour <= reply.shipLastRead)
      && reply.convoyReads.every(hour => reply.convoyFirstRead !== null && reply.convoyLastRead !== null && hour >= reply.convoyFirstRead && hour <= reply.convoyLastRead)
      : reply.shipReads[0] === (reply.shipFirstRead ?? undefined) && reply.shipReads.at(-1) === (reply.shipLastRead ?? undefined)
        && reply.convoyReads[0] === (reply.convoyFirstRead ?? undefined) && reply.convoyReads.at(-1) === (reply.convoyLastRead ?? undefined))
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string')
    && (reply.restarts === undefined || reads(reply.restarts))
    && Array.isArray(reply.series) && new Set(reply.series.map(series => series?.tag)).size === reply.series.length
    && reply.series.every(series => object(series) && typeof series.tag === 'string' && isOrderedSpans(series.segments)
      && (series.membership === null || isOrderedSpans(series.membership)) && validCountryIdentityRuns(series.identities) && Array.isArray(series.runs)
      && series.runs.every((run, index) => object(run) && int(run.firstHour) && int(run.lastHour) && run.firstHour <= run.lastHour
        && typeof run.startsSegment === 'boolean' && count(run.ships) && (run.ships === null || run.ships >= 0) && count(run.convoys)
        && (run.ships === null || run.convoys === null || int(run.ships + run.convoys))
        && (index === 0 || series.runs[index - 1].lastHour < run.firstHour)));
  if (!ok) throw new Error('The naval losses reply is malformed.');
  return reply;
}

export function navalRequest(branchId: string, resolved: { tags: readonly string[]; top?: number }, kind: NavalKind,
  attribution: NavalAttribution, period: NavalPeriod): NavalLossesParams {
  return { branchId, tags: resolved.top == null ? [...resolved.tags] : [], top: resolved.top ?? null, kind, attribution, period };
}

export function navalModel(reply: NavalLossesReply) {
  return { reply, identities: reply.series.every(series => series.identities !== undefined)
    ? countryIdentityIndex(reply.series.map(series => ({ tag: series.tag, identities: series.identities! }))) : null,
    tags: reply.series.map(series => series.tag), series: new Map(reply.series.map(series => [series.tag,
    { ...series, starts: stretchStarts(series.runs.map(run => ({ ...run, breakBefore: run.startsSegment }))) }])) };
}
export type NavalModel = ReturnType<typeof navalModel>;
export const navalNumbers = (reply: NavalLossesReply): NavalLossesReply => ({ ...reply, warnings: [] });

/** Drawn candidates retain their dated native holder; unknown parts cannot pick. */
export function navalDrillActors(model: NavalModel, tags: readonly string[], hour: number, kind: NavalKind): string[] {
  return [...new Set(tags)].filter(tag => {
    const run = runAt(model.series.get(tag)?.runs ?? [], hour);
    return navalSegmentAt(model, tag, hour) !== null && run !== null && navalValue(run, kind) !== null;
  });
}
export const formatNavalCount = (value: number) => usNumber.format(value);
export const navalKindLabel = (kind: NavalKind) => kind === 'ships' ? 'Ships' : kind === 'convoys' ? 'Convoys' : 'Ships and convoys';

/** A missing selected part makes the whole unknown; a known zero remains zero. */
export function navalValue(run: NavalLossRun, kind: NavalKind): number | null {
  if (kind === 'ships') return run.ships;
  if (kind === 'convoys') return run.convoys;
  return run.ships === null || run.convoys === null ? null : run.ships + run.convoys;
}

export function navalSegmentAt(model: NavalModel | null, tag: string, hour: number): number | null {
  const series = model?.series.get(tag);
  return runAt(series?.identities ?? series?.segments ?? [], Math.floor(hour))?.firstHour ?? null;
}

/** `top` ranks only countries with losses: a top N of zeros would list countries in tag order. */
export function rankNaval(model: NavalModel, candidates: readonly string[], hour: number, kind: NavalKind, limit: number, top = false): CountryRaceEntry[] {
  return candidates.flatMap(tag => {
    const series = model.series.get(tag);
    const index = runIndexAt(series?.runs ?? [], Math.floor(hour));
    if (!series || index < 0 || navalSegmentAt(model, tag, hour) === null) return [];
    const value = navalValue(series.runs[index], kind);
    return value === null ? [] : [{ tag, value, label: formatNavalCount(value), stretch: series.starts[index],
      ...(kind === 'both' ? { parts: [{ id: 'ships', value: series.runs[index].ships! }, { id: 'convoys', value: series.runs[index].convoys! }] } : {}) }];
  }).filter(entry => !top || entry.value > 0).sort((a, b) => (b.value - a.value || compareTags(a.tag, b.tag))).slice(0, limit);
}

/** Whether every country the reply lists at the hour has a known tally of zero. An unknown
 * tally is not a zero; a Top N reply lists only known tallies, so this speaks for those alone. */
export function navalAllZero(model: NavalModel, candidates: readonly string[], hour: number, kind: NavalKind): boolean {
  let known = false;
  for (const tag of candidates) {
    const series = model.series.get(tag);
    const index = runIndexAt(series?.runs ?? [], Math.floor(hour));
    if (!series || index < 0 || navalSegmentAt(model, tag, hour) === null) continue;
    const value = navalValue(series.runs[index], kind);
    if (value !== 0) return false;
    known = true;
  }
  return known;
}

export function navalLines(model: NavalModel, tags: readonly string[], kind: NavalKind, range: Span,
  membership: ReadonlyMap<string, readonly Span[]> | null): CountryRaceLine[] {
  return tags.flatMap(tag => {
    const series = model.series.get(tag);
    if (!series) return [];
    const spans = series.identities ?? series.segments;
    const starts = new Set(spans.map(segment => segment.firstHour));
    const measured = series.runs.flatMap(run => {
      const value = navalValue(run, kind);
      return value === null ? [] : [{ firstHour: run.firstHour, lastHour: run.lastHour, value, breakBefore: run.startsSegment }];
    });
    const identityRuns = clipToSpans(measured, spans).map(run => ({ ...run, breakBefore: run.breakBefore || starts.has(run.firstHour) }));
    const runs = shownRuns(identityRuns, membership ? membership.get(tag) ?? [] : series.membership, range);
    return runs.length ? [{ tag, runs }] : [];
  });
}

/** What the totals shown at `hour` count from: a running total from its stretch's first read,
 * and either after hours the recording lost, from where its stretch starts. */
export function navalPeriodLabel(reply: NavalLossesReply, period: NavalPeriod, kind: NavalKind = 'both', hour?: number): string {
  const restarts = reply.restarts ?? [];
  const since = hour === undefined ? restarts.at(-1) ?? null : stretchSince(restarts, hour);
  if (period === 'month') return since === null ? 'Per month' : `Per month, counted ${sinceLabel(since)}`;
  // The reply's first read begins its latest stretch; the branch's own reads begin its first.
  const branchFirst = kind === 'ships' ? reply.shipFirstRead : kind === 'convoys' ? reply.convoyFirstRead
    : reply.shipFirstRead !== null && reply.convoyFirstRead !== null ? Math.max(reply.shipFirstRead, reply.convoyFirstRead) : null;
  const from = !restarts.length || since === restarts.at(-1) ? reply.firstRead : since ?? branchFirst;
  return from === null ? 'Running total' : `Since ${formatDateTime(from)}`;
}

function navalNote(attribution: NavalAttribution): string | undefined {
  return attribution === 'sunkBy' ? 'Non-hostile causes and assisted kills are excluded.' : undefined;
}

/** Both keeps each part's date; its single country look uses the newest read. */
export function navalReadHour(reply: NavalLossesReply, hour: number, kind: NavalKind): number | null {
  const ships = kind !== 'convoys' ? readAt(reply.shipReads, hour) : null;
  const convoys = kind !== 'ships' ? readAt(reply.convoyReads, hour) : null;
  return ships === null ? convoys : convoys === null ? ships : Math.max(ships, convoys);
}

/** A supplying read cannot name a different holder of the drawn alias. */
export function navalLookHour(model: NavalModel | null, tag: string | null, hour: number, kind: NavalKind): number | null {
  if (!model) return null;
  const read = navalReadHour(model.reply, hour, kind);
  if (read === null || tag === null || model.identities === null) return read;
  const key = model.identities.keyAt(tag, Math.floor(hour));
  return key !== null && model.identities.keyAt(tag, read) === key ? read : null;
}

/** Each pickable actor keeps its drawn identity; artwork comes from that
 * holder's authenticated alias at the supplying read, never a reused tag. */
export function navalChoiceReads(model: NavalModel | null, tags: readonly string[], hour: number,
  kind: NavalKind, branchId: string): ReadonlyMap<string, CountryPickerRead | null> {
  const read = model ? navalReadHour(model.reply, hour, kind) : null;
  return new Map(tags.map((tag): [string, CountryPickerRead | null] => {
    if (!model || read === null) return [tag, null];
    if (model.identities === null) {
      const segment = navalSegmentAt(model, tag, hour);
      return [tag, segment !== null && navalSegmentAt(model, tag, read) === segment ? { tag, branchId, hour: read } : null];
    }
    const key = model.identities.keyAt(tag, Math.floor(hour));
    const aliases = key === null ? [] : model.identities.aliasesAt(new Set([key]), read);
    return [tag, aliases.length === 1 ? { tag: aliases[0], branchId, hour: read } : null];
  }));
}

export function navalTooltip(model: NavalModel, tag: string, hour: number, name: string, kind: NavalKind,
  attribution: NavalAttribution): TooltipContent {
  const run = navalSegmentAt(model, tag, hour) === null ? null : runAt(model.series.get(tag)?.runs ?? [], Math.floor(hour));
  const value = run ? navalValue(run, kind) : null;
  return { title: name, subtitle: formatDateTime(hour), rows: [
    ...(kind !== 'convoys' ? [{ label: 'Ships', value: run?.ships == null ? 'Unknown' : formatNavalCount(run.ships) }] : []),
    ...(kind !== 'ships' ? [{ label: 'Convoys', value: run?.convoys == null ? 'Unknown' : formatNavalCount(run.convoys) }] : []),
    ...(kind === 'both' ? [{ label: attribution === 'lost' ? 'Lost' : 'Sunk by', value: value === null ? 'Unknown' : formatNavalCount(value), strong: true, divider: true }] : []),
  ], note: navalNote(attribution) };
}

export function navalLineTooltip(model: NavalModel, hour: number, tags: readonly string[], labelAt: LabelAt,
  colour: (tag: string) => string, kind: NavalKind, attribution: NavalAttribution): TooltipContent {
  return { title: `${navalKindLabel(kind)} ${attribution === 'lost' ? 'lost' : 'sunk by'}`, subtitle: formatDateTime(hour),
    rows: tags.length ? tags.map(tag => {
      const label = labelAt(hour)(tag);
      const run = runAt(model.series.get(tag)?.runs ?? [], Math.floor(hour));
      const value = run ? navalValue(run, kind) : null;
      return { label: label.name, countryFlag: label, swatch: colour(tag), value: value === null ? 'Unknown' : formatNavalCount(value) };
    }) : [{ label: 'Counts unknown at this time', tone: 'muted' }], note: navalNote(attribution) };
}

export const NAVAL_PARTS: readonly CountryRacePart[] = [{ id: 'ships', label: 'Ships' }, { id: 'convoys', label: 'Convoys' }];

export function navalBands(model: NavalModel, tag: string, range: Span, membership: readonly Span[] | null) {
  const series = model.series.get(tag);
  return NAVAL_PARTS.map(part => {
    const measured = (series?.runs ?? []).flatMap(run => navalValue(run, 'both') === null ? []
      : [{ firstHour: run.firstHour, lastHour: run.lastHour, value: part.id === 'ships' ? run.ships! : run.convoys!, breakBefore: run.startsSegment }]);
    return shownRuns(clipToSpans(measured, series?.identities ?? series?.segments ?? []), membership ?? series?.membership ?? null, range);
  });
}

/** Signed corrections stay signed: the stack draws them below zero. */
export function navalHolderBands(model: NavalModel, membership: ReadonlyMap<string, readonly Span[]>, range: Span) {
  const byTag = [...membership].map(([tag, spans]) => navalBands(model, tag, range, spans));
  return NAVAL_PARTS.map((_, index) => byTag.flatMap(bands => bands[index]).sort((a, b) => a.firstHour - b.firstHour));
}

export interface NavalManifest extends NavalLossesReply { windows: { firstHour: number; lastHour: number }[] }
export const validateNavalWindow = (value: unknown) => validateNavalLossesReply(value, true);
export function validateNavalManifest(value: unknown): NavalManifest {
  const reply = validateNavalWindow(value) as NavalManifest;
  if (!isOrderedSpans(reply.windows)) throw Error('The naval loss windows are malformed.');
  return reply;
}
export function mergeNavalWindows(manifest: NavalLossesReply, pages: Iterable<NavalLossesReply>): NavalLossesReply {
  const series = new Map(manifest.series.map(series => [series.tag, { ...series, runs: [] as NavalLossRun[] }]));
  const shipReads = new Set(manifest.shipReads), convoyReads = new Set(manifest.convoyReads);
  const warnings = new Set(manifest.warnings);
  for (const page of pages) {
    for (const read of page.shipReads) shipReads.add(read);
    for (const read of page.convoyReads) convoyReads.add(read);
    for (const warning of page.warnings) warnings.add(warning);
    for (const row of page.series) {
      let prior = series.get(row.tag);
      if (!prior) { prior = { ...row, runs: [] }; series.set(row.tag, prior); }
      prior.runs.push(...row.runs);
      if (row.membership) prior.membership = [...prior.membership ?? [], ...row.membership];
    }
  }
  for (const row of series.values()) {
    const ordered = [...new Map(row.runs.map(run => [JSON.stringify(run), run])).values()].sort((a,b) => a.firstHour-b.firstHour);
    const joined: NavalLossRun[] = [];
    for (const run of ordered) {
      const prior = joined.at(-1);
      if (prior && !run.startsSegment && prior.lastHour+1 === run.firstHour && prior.ships === run.ships && prior.convoys === run.convoys) prior.lastHour = run.lastHour;
      else joined.push({ ...run });
    }
    row.runs = joined;
    if (row.membership) row.membership = [...new Map(row.membership.map(span => [JSON.stringify(span),span])).values()].sort((a,b) => a.firstHour-b.firstHour);
  }
  return { ...manifest, series: [...series.values()], shipReads: [...shipReads].sort((a,b)=>a-b), convoyReads: [...convoyReads].sort((a,b)=>a-b), warnings: [...warnings] };
}
