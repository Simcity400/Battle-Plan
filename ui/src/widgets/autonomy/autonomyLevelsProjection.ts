import { colour } from '../../charts/palette.ts';
import { contentTitle } from '../../dashboard/contentTitles.ts';
import { countryKey } from '../../dashboard/countryKey.ts';
import { flagChoiceArt } from '../../families/art/provider.ts';
import { unavailableArt } from '../../families/art/types.ts';
import type { HourRanges } from '../../families/holders.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import type { MeterWidgetDeclaration } from '../../families/meter/declaration.ts';
import type { MeterModel, MeterProjection, MeterRow } from '../../families/meter/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { BoundedReadView, BoundedSeries, DeclaredCountryFilter, FamilyPlanContext } from '../../families/plan.ts';
import type { ArtResult } from '../../families/contract.ts';
import { formatProgress } from '../../families/valueFormat.ts';
import type { AutonomyLevelsReply, AutonomySubject, SubjectLevel } from '../../generated/dashboard';
import { identityName } from '../../families/table/identityCell.ts';

export interface AutonomyLevelsOptions {
  readonly filter: DeclaredCountryFilter;
  readonly filterAll: boolean;
  readonly bySubject: boolean;
  readonly surface: string;
}

const text = (value: unknown) => typeof value === 'string' && value.length > 0;
const optionalText = (value: unknown) => value === null || text(value);
const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);
const id = (value: unknown) => Number.isSafeInteger(value) && (value as number) >= 0;
const optionalId = (value: unknown) => value === undefined || value === null || id(value);
const optionalNumber = (value: unknown) => value === undefined || value === null || (typeof value === 'number' && Number.isFinite(value));

export function validateAutonomyLevels(value: unknown): AutonomyLevelsReply {
  const ok = object(value) && Array.isArray(value.levels) && Array.isArray(value.subjects) && optionalText(value.missing)
    && Array.isArray(value.warnings) && value.warnings.every(item => typeof item === 'string')
    && value.levels.every(level => object(level) && text(level.token) && optionalText(level.name) && typeof level.ordered === 'boolean')
    && value.subjects.every(item => object(item) && id(item.overlordId) && optionalId(item.overlordSlot) && optionalText(item.overlord)
      && id(item.subjectId) && optionalId(item.subjectSlot) && optionalText(item.subject)
      && optionalText(item.level) && optionalText(item.levelBelow) && optionalText(item.levelAbove) && optionalText(item.levelsUnknown)
      && optionalNumber(item.score) && optionalNumber(item.lowerScore) && optionalNumber(item.upperScore)
      && (item.progressMilli === null ? text(item.progressUnknown) : Number.isSafeInteger(item.progressMilli) && item.progressUnknown === null));
  if (!ok) throw new Error('The autonomy levels reply is malformed.');
  return value as unknown as AutonomyLevelsReply;
}

export const within = (ranges: HourRanges | undefined, hour: number) => !!ranges?.some(([first, last]) => first <= hour && hour <= last);

export function filterNext(filter: DeclaredCountryFilter, hour: number): number {
  let low = 0, high = filter.bounds.length;
  while (low < high) {
    const mid = (low + high) >> 1;
    if (filter.bounds[mid]! <= hour) low = mid + 1;
    else high = mid;
  }
  return filter.bounds[low] ?? Infinity;
}

export function subjectKeyOf(item: Pick<AutonomySubject, 'subjectSlot' | 'subject' | 'subjectId'>): string {
  if (item.subjectSlot != null) return countryKey(item.subjectSlot);
  return item.subject ?? `subject:${item.subjectId}`;
}

export function overlordKeyOf(item: Pick<AutonomySubject, 'overlordSlot' | 'overlord' | 'overlordId'>): string {
  if (item.overlordSlot != null) return countryKey(item.overlordSlot);
  return item.overlord ?? `overlord:${item.overlordId}`;
}

export function sampleSparkline(series: BoundedSeries | undefined, hour: number, points = 16): readonly (number | null)[] | undefined {
  if (!series || series.buckets.length === 0) return undefined;
  const pieces: { readonly first: number; readonly last: number; readonly firstValue: number; readonly lastValue: number; readonly starts: boolean; readonly broken: boolean }[] = [];
  for (const bucket of series.buckets) {
    if (!bucket) continue;
    if (bucket.firstHour > hour) break;
    pieces.push({ first: bucket.firstHour, last: Math.min(bucket.lastHour, hour), firstValue: bucket.firstValue, lastValue: bucket.lastValue, starts: bucket.firstBreak, broken: bucket.hasBreak });
  }
  if (!pieces.length) return undefined;
  const first = pieces[0]!.first;
  const last = pieces[pieces.length - 1]!.last;
  if (last <= first) return undefined;
  const missing = [...series.missing];
  const unreadBetween = (from: number, to: number) => missing.some(span => span.firstHour <= to && from <= span.lastHour);
  const out: (number | null)[] = [];
  let cursor = 0;
  let known = 0;
  let previous: { readonly hour: number; readonly piece: number } | null = null;
  for (let i = 0; i < points; i++) {
    const target = first + ((last - first) * i) / (points - 1);
    while (cursor < pieces.length && pieces[cursor]!.last + 1 <= target && cursor + 1 < pieces.length) cursor++;
    const piece = pieces[cursor]!;
    const at = Math.floor(target);
    const inside = piece.first <= target && target < piece.last + 1 && !unreadBetween(at, at) && !piece.broken;
    const broken = previous !== null && (unreadBetween(previous.hour + 1, at)
      || pieces.slice(previous.piece + 1, cursor + 1).some((held, k) => held.starts || held.broken || pieces[previous!.piece + k]!.last + 1 < held.first));
    if (inside && !broken) {
      out.push(target >= piece.last ? piece.lastValue : piece.firstValue);
      known++;
      previous = { hour: at, piece: cursor };
    } else {
      out.push(null);
      previous = inside ? { hour: at, piece: cursor } : null;
    }
  }
  return known >= 2 ? out : undefined;
}

interface Named {
  readonly name: string;
  readonly flag: ArtResult;
  readonly tag: string;
}

function named(readView: BoundedReadView, slot: number | null | undefined, tag: string | null, rawId: number): Named {
  const key = slot != null ? countryKey(slot) : tag;
  if (!key) return { name: `Country ${rawId}`, tag: '?', flag: unavailableArt('Country tag not recorded') };
  const who = readView.who('country', key, readView.hour);
  const name = identityName(readView, 'country', key);
  const flag = flagChoiceArt(readView, who.flagChoice);
  return { name, tag: tag ?? (who.tag?.status === 'ready' ? who.tag.value : '?'), flag };
}

export function levelTitle(levels: readonly SubjectLevel[], token: string | null): string {
  if (token === null) return 'Level not recorded';
  return contentTitle(levels.find(level => level.token === token)?.name, token, 'Autonomy level name unavailable');
}

export function percentText(milli: number): string {
  return formatProgress(milli / 100_000);
}

function row(
  readView: BoundedReadView,
  reply: AutonomyLevelsReply,
  item: AutonomySubject,
  surface: string,
  trend: ReadonlyMap<string, BoundedSeries> | undefined,
): MeterRow {
  const subject = named(readView, item.subjectSlot, item.subject, item.subjectId);
  const overlord = named(readView, item.overlordSlot, item.overlord, item.overlordId);
  const levelName = levelTitle(reply.levels, item.level);
  const order = item.level === null ? -1 : reply.levels.findIndex(level => level.token === item.level);
  const fill = order < 0 ? undefined : colour(surface, order);
  const known = item.progressMilli !== null;
  const figure = known ? percentText(item.progressMilli!) : undefined;
  const tile = (item.subjectSlot != null ? trend?.get(countryKey(item.subjectSlot)) : undefined)
    ?? (item.subject ? trend?.get(item.subject) : undefined);
  const sparkline = sampleSparkline(tile, readView.hour);
  const items: TooltipListItem[] = [
    { label: 'Overlord', value: overlord.name },
    { label: 'Level', value: levelName, ...(fill ? { swatch: fill } : {}) },
    item.levelsUnknown !== null
      ? { label: 'Levels either side', missingReason: item.levelsUnknown }
      : { label: 'Level below', value: item.levelBelow === null ? 'None' : levelTitle(reply.levels, item.levelBelow) },
    ...(item.levelsUnknown === null ? [{ label: 'Level above', value: item.levelAbove === null ? 'None' : levelTitle(reply.levels, item.levelAbove) }] : []),
    known ? { label: 'Within level', value: figure! } : { label: 'Within level', missingReason: item.progressUnknown! },
  ];
  return {
    key: `subject:${item.overlordId}:${item.subjectId}`,
    name: subject.name,
    subtitle: levelName,
    flag: { tag: subject.tag, result: subject.flag },
    bar: { kind: 'threshold', fraction: known ? Math.max(0, Math.min(1, item.progressMilli! / 100_000)) : null, ticks: [], ends: true, ...(fill ? { color: fill } : {}), height: 7 },
    ...(figure ? { figure } : { figureMissing: true, figureMissingLabel: 'Not recorded' }),
    ...(sparkline ? { sparkline } : {}),
    spoken: `${subject.name}, ${levelName}, ${figure ?? `not recorded: ${item.progressUnknown}`} within level`,
    audit: `${item.overlord ?? item.overlordId}>${item.subject ?? item.subjectId}|${item.level ?? 'not-recorded'}|${figure ?? 'not-recorded'}`,
    tooltip: { title: subject.name, hour: readView.hour, items },
  };
}

export const autonomyLevelsProjection: MeterProjection<AutonomyLevelsOptions> = registerFamilyProjection({
  family: 'meters',
  kind: 'autonomy',
  nextChange(readView, options): number {
    return filterNext(options.filter, readView.hour);
  },
  project(readView, options): MeterModel {
    const refused = readView.refused('main');
    if (refused) return { hour: readView.hour, layout: 'rows', refused };
    const raw = readView.panel<unknown>('main');
    if (!raw) return { hour: readView.hour, layout: 'rows', empty: null };
    const reply = validateAutonomyLevels(raw);
    if (reply.missing !== null) {
      return { hour: readView.hour, layout: 'rows', empty: 'Subjects are not recorded at this time.', emptyDetail: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings };
    }
    const unrecorded = options.filter.unrecorded.find(([first, last]) => first <= readView.hour && readView.hour <= last)?.[2] ?? null;
    if (unrecorded) return { hour: readView.hour, layout: 'rows', empty: unrecorded, emptyKind: 'unavailable', warnings: reply.warnings };
    const passing = options.filter.passing;
    const shown = passing
      ? reply.subjects.filter(item => {
          const tag = options.bySubject ? item.subject : item.overlord;
          return tag !== null && within(passing[tag], readView.hour);
        })
      : reply.subjects;
    const count = `${shown.length} ${shown.length === 1 ? 'subject' : 'subjects'}`;
    if (!shown.length) {
      return {
        hour: readView.hour,
        layout: 'rows',
        summaryText: count,
        empty: options.filterAll ? 'No subjects at this time.' : 'No matching subjects at this time.',
        emptyKind: 'empty',
        warnings: reply.warnings,
      };
    }
    const trend = readView.series('trend');
    return {
      hour: readView.hour,
      layout: 'rows',
      summaryText: count,
      sections: [{
        key: 'subjects',
        title: 'Within each level',
        rightCaption: 'Left: level below · Right: level above',
        rows: shown.map(item => row(readView, reply, item, options.surface, trend)),
      }],
      warnings: reply.warnings,
    };
  },
}, 'meters');

export const autonomyLevelsDeclaration: MeterWidgetDeclaration<AutonomyLevelsOptions> = {
  family: 'meters',
  kind: 'autonomy',
  label: 'Meters',
  subject: 'autonomy',
  emptyCampaign: 'Load a campaign to see its subjects’ autonomy.',
  icon: 'link',
  countries: { mode: 'filter', content: 'history' },
  sources: (ctx: FamilyPlanContext) => ({
    owner: `autonomy-levels:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`,
    entries: [
      {
        name: 'main',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'point',
          channel: 'panel',
          coverage: { mode: 'population', form: 'all' },
          product: 'autonomy-levels',
          followQuery: { panel: 'autonomy-levels', branchId: ctx.branch.id },
          params: { branchId: ctx.branch.id },
          validate: validateAutonomyLevels,
        },
      },
      {
        name: 'trend',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'pixel-binned',
          coverage: { mode: 'population', form: 'all' },
          family: { kind: 'autonomy-lines' },
          tags: [],
          headQuery: null,
        },
      },
    ],
  }),
  projection: autonomyLevelsProjection,
  projectionOptions: (ctx, extra) => ({
    filter: ctx.countries.filter,
    filterAll: ctx.countries.all,
    bySubject: ctx.options.countriesAre === 'subjects',
    surface: extra.surface,
  }),
  jumpToDate: true,
  keyByBranch: true,
};
