import { tailColour } from '../../charts/palette.ts';
import { countryKey } from '../../dashboard/countryKey.ts';
import { artUrlOf, flagChoiceArt } from '../../families/art/provider.ts';
import { auditRow, type FamilyAuditRow } from '../../families/auditRow.ts';
import {
  FutureDataError,
  type BoundedReadView,
  type FamilyPlanContext,
  type SourceEntry,
  type SourcePlan,
} from '../../families/contract.ts';
import type { DeclaredCountryFilter } from '../../families/plan.ts';
import type { HolderRun } from '../../families/holders.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { identityName } from '../../families/table/identityCell.ts';
import type { TimeChartProjection, TimeChartWidgetDeclaration } from '../../families/time-chart/declaration.ts';
import {
  seriesLookAt,
  type ChartGap,
  type ChartHoverDetail,
  type TimeChartModel,
  type TimeChartSeriesModel,
} from '../../families/time-chart/model.ts';
import {
  branchGapsOf,
  countrySeriesLooks,
  missingSpansOf,
  ALL_POPULATION_TOP,
} from '../../families/time-chart/seriesHelpers.ts';
import { formatScalar, notRecordedLabel } from '../../families/valueFormat.ts';
import type { AutonomyLevelsReply, AutonomySubject } from '../../generated/dashboard';
import {
  filterNext,
  levelTitle,
  overlordKeyOf,
  percentText,
  subjectKeyOf,
  validateAutonomyLevels,
  within,
} from './autonomyLevelsProjection.ts';

const COLOURED = 8;

export interface AutonomyLinesOptions {
  readonly filter: DeclaredCountryFilter;
  readonly filterAll: boolean;
  readonly bySubject: boolean;
  readonly grow: boolean;
  readonly holders: Readonly<Record<string, readonly HolderRun[]>> | null;
  readonly extent: readonly [number, number];
  readonly gaps: readonly ChartGap[];
  readonly surface: string;
}

function matchingSubjects(
  subjects: readonly AutonomySubject[],
  filter: DeclaredCountryFilter,
  bySubject: boolean,
  hour: number,
): readonly AutonomySubject[] {
  const passing = filter.passing;
  if (!passing) return subjects;
  return subjects.filter(item => {
    const tag = bySubject ? item.subject : item.overlord;
    return tag !== null && within(passing[tag], hour);
  });
}

function formatScore(value: number): string {
  return formatScalar(value, { decimals: 0, maxDecimals: 1 });
}

function buildHoverDetail(
  readView: BoundedReadView,
  options: AutonomyLinesOptions,
  looks: ReadonlyMap<string, ReturnType<typeof countrySeriesLooks> extends ReadonlyMap<string, infer T> ? T : never>,
): ChartHoverDetail | null {
  let hover;
  try {
    hover = readView.hover<AutonomyLevelsReply>('hover');
  } catch (error) {
    if (error instanceof FutureDataError) return null;
    throw error;
  }
  if (!hover || !hover.data) return null;
  const { hour, data: reply } = hover;
  if (reply.missing !== null) {
    return {
      hour,
      replacesSeries: true,
      items: [{ label: 'Autonomy', missingReason: reply.missing, tone: 'muted' }],
    };
  }
  const unrecorded = options.filter.unrecorded.find(([first, last]) => first <= hour && hour <= last)?.[2] ?? null;
  if (unrecorded) {
    return {
      hour,
      replacesSeries: true,
      items: [{ label: 'Autonomy', missingReason: unrecorded, tone: 'muted' }],
    };
  }
  const eligible = matchingSubjects(reply.subjects, options.filter, options.bySubject, hour);
  if (eligible.length === 0) return null;

  const items: TooltipListItem[] = [];
  for (const item of eligible) {
    const key = subjectKeyOf(item);
    const look = looks.get(key);
    const atHour = look ? seriesLookAt(look, hour) : null;
    const label = atHour ? atHour.name : identityName(readView, 'country', key, hour);
    const who = readView.who('country', key, hour);
    const flagUrl = artUrlOf(flagChoiceArt(readView, who.flagChoice));
    const overlordKey = overlordKeyOf(item);
    const overlordName = identityName(readView, 'country', overlordKey, hour);
    const levelName = levelTitle(reply.levels, item.level);
    items.push(
      item.progressMilli !== null
        ? {
            label,
            value: percentText(item.progressMilli),
            strong: true,
            ...(atHour ? { swatch: atHour.color } : {}),
            ...(flagUrl ? { countryFlag: { flag: flagUrl } } : {}),
            divider: items.length > 0,
          }
        : {
            label,
            missingReason: item.progressUnknown ?? notRecordedLabel(),
            tone: 'muted',
            ...(atHour ? { swatch: atHour.color } : {}),
            ...(flagUrl ? { countryFlag: { flag: flagUrl } } : {}),
            divider: items.length > 0,
          },
    );
    items.push({ label: 'Overlord', value: overlordName, indent: true });
    items.push({ label: 'Level', value: levelName, indent: true });
    if (item.score != null) {
      const bounds = item.lowerScore != null && item.upperScore != null
        ? ` (${formatScore(item.lowerScore)}–${formatScore(item.upperScore)})`
        : '';
      items.push({ label: 'Score', value: `${formatScore(item.score)}${bounds}`, indent: true });
    }
  }
  return items.length > 0 ? { hour, replacesSeries: true, items } : null;
}

export const autonomyLinesProjection: TimeChartProjection<AutonomyLinesOptions> = registerFamilyProjection({
  kind: 'autonomy',
  family: 'time-chart',
  variant: 'line',
  joinMode: 'step',
  measureTitle: 'Autonomy',
  nextChange(readView, options): number {
    return filterNext(options.filter, readView.hour);
  },
  project(readView: BoundedReadView, options: AutonomyLinesOptions): TimeChartModel {
    const hour = readView.hour;
    const yAxis = {
      format: 'percent' as const,
      decimals: 0,
      fixed: [0, 100] as readonly [number, number],
    };
    const view = options.grow
      ? { x: [options.extent[0], Math.max(options.extent[0] + 1, hour)] as readonly [number, number] }
      : undefined;
    const gaps = options.gaps.length > 0 ? options.gaps : undefined;

    const refused = readView.refused('head') ?? readView.refused('main');
    if (refused) {
      return {
        kind: 'autonomy',
        measureTitle: 'Autonomy',
        mode: 'lines',
        hour,
        extent: options.extent,
        ...(view ? { view } : {}),
        yAxis,
        series: [],
        gaps,
        empty: { kind: 'unavailable', message: refused },
        auditRows: [],
      };
    }

    const raw = readView.panel<unknown>('head');
    if (!raw) {
      return {
        kind: 'autonomy',
        measureTitle: 'Autonomy',
        mode: 'lines',
        hour,
        extent: options.extent,
        ...(view ? { view } : {}),
        yAxis,
        series: [],
        gaps,
        auditRows: [],
      };
    }

    const reply = validateAutonomyLevels(raw);
    if (reply.missing !== null) {
      return {
        kind: 'autonomy',
        measureTitle: 'Autonomy',
        mode: 'lines',
        hour,
        extent: options.extent,
        ...(view ? { view } : {}),
        yAxis,
        series: [],
        gaps,
        warnings: reply.warnings.length > 0 ? reply.warnings : undefined,
        empty: { kind: 'unavailable', message: reply.missing },
        auditRows: [auditRow('__head__', reply.missing)],
      };
    }

    const unrecorded = options.filter.unrecorded.find(([first, last]) => first <= hour && hour <= last)?.[2] ?? null;
    if (unrecorded) {
      return {
        kind: 'autonomy',
        measureTitle: 'Autonomy',
        mode: 'lines',
        hour,
        extent: options.extent,
        ...(view ? { view } : {}),
        yAxis,
        series: [],
        gaps,
        warnings: reply.warnings.length > 0 ? reply.warnings : undefined,
        empty: { kind: 'unavailable', message: unrecorded },
        auditRows: [auditRow('__filter__', unrecorded)],
      };
    }

    const shown = matchingSubjects(reply.subjects, options.filter, options.bySubject, hour);
    if (shown.length === 0) {
      return {
        kind: 'autonomy',
        measureTitle: 'Autonomy',
        mode: 'lines',
        hour,
        extent: options.extent,
        ...(view ? { view } : {}),
        yAxis,
        series: [],
        gaps,
        warnings: reply.warnings.length > 0 ? reply.warnings : undefined,
        empty: {
          kind: 'empty',
          message: options.filterAll ? 'No subjects at this time.' : 'No matching subjects at this time.',
        },
        auditRows: [],
      };
    }

    const mainTiles = readView.series('main');
    const orderedKeys: string[] = [];
    const seen = new Set<string>();
    const itemByKey = new Map<string, AutonomySubject>();
    for (const item of shown) {
      const key = subjectKeyOf(item);
      if (!seen.has(key)) {
        seen.add(key);
        orderedKeys.push(key);
        itemByKey.set(key, item);
      }
    }

    const looks = countrySeriesLooks(orderedKeys, readView, {
      holders: options.holders,
      from: options.extent[0],
      surface: options.surface,
      withFlag: true,
      tailColor: tailColour(options.surface),
    });

    const seriesList: TimeChartSeriesModel[] = [];
    const auditRows: FamilyAuditRow[] = [];
    let maxSeen = 100;
    for (let i = 0; i < orderedKeys.length; i++) {
      const key = orderedKeys[i]!;
      const item = itemByKey.get(key)!;
      const look = looks.get(key)!;
      const tile = (item.subjectSlot != null ? mainTiles?.get(countryKey(item.subjectSlot)) : undefined)
        ?? mainTiles?.get(key)
        ?? (item.subject ? mainTiles?.get(item.subject) : undefined)
        ?? null;
      const isTail = i >= COLOURED;
      const color = isTail ? tailColour(options.surface) : look.color;
      const known = item.progressMilli !== null;
      const headValue = known ? item.progressMilli! / 1000 : null;
      if (headValue !== null && headValue > maxSeen) maxSeen = headValue;
      if (tile) {
        for (const bucket of tile.buckets) {
          if (!bucket) continue;
          const scaled = bucket.maxValue * 100;
          if (Number.isFinite(scaled) && scaled > maxSeen) maxSeen = scaled;
        }
      }
      const absentReason = known ? null : (item.progressUnknown ?? notRecordedLabel());
      const levelName = levelTitle(reply.levels, item.level);

      seriesList.push({
        id: key,
        key,
        name: look.name,
        color,
        role: isTail ? 'tail' : 'primary',
        step: true,
        bounded: tile,
        valueScale: 100,
        headValue,
        headHour: hour,
        flag: look.flag,
        identities: look.identities,
        missingSpans: missingSpansOf(tile),
        absentReason,
        endLabel: !isTail && known
          ? {
              flag: look.flag,
              name: look.name,
              fullName: look.name,
              value: percentText(item.progressMilli!),
            }
          : null,
      });

      auditRows.push(auditRow(
        key,
        known ? `${levelName} · ${percentText(item.progressMilli!)}` : (absentReason ?? notRecordedLabel()),
      ));
    }

    const yTop = maxSeen > 100 ? Math.ceil(maxSeen / 25) * 25 : 100;
    const hoverDetail = buildHoverDetail(readView, options, looks);
    return {
      kind: 'autonomy',
      measureTitle: 'Autonomy',
      mode: 'lines',
      hour,
      extent: options.extent,
      ...(view ? { view } : {}),
      yAxis: {
        ...yAxis,
        fixed: [0, yTop],
      },
      series: seriesList,
      gaps,
      summaryText: 'Progress within each subject’s current autonomy level',
      warnings: reply.warnings.length > 0 ? reply.warnings : undefined,
      hoverDetail,
      auditRows,
    };
  },
}, 'lines');

export const autonomyLinesDeclaration: TimeChartWidgetDeclaration<AutonomyLinesOptions> = {
  family: 'time-chart',
  kind: 'autonomy',
  label: 'Lines',
  subject: 'autonomy',
  emptyCampaign: 'Load a campaign to see its subjects’ autonomy.',
  icon: 'link',
  countries: { mode: 'filter' },
  sources: (ctx: FamilyPlanContext): SourcePlan => {
    const common = { source: ctx.track.source, branch: ctx.branch.id, offset: ctx.track.axisOffset };
    const head: SourceEntry = {
      name: 'head',
      ...common,
      read: {
        shape: 'point',
        channel: 'panel',
        coverage: { mode: 'population', form: 'all' },
        product: 'autonomy-levels',
        followQuery: { panel: 'autonomy-levels', branchId: ctx.branch.id },
        params: { branchId: ctx.branch.id },
        validate: validateAutonomyLevels,
      },
    };
    return {
      owner: `autonomy:lines:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`,
      exactEntry: head,
      entries: [
        {
          name: 'main',
          ...common,
          read: {
            shape: 'pixel-binned',
            coverage: { mode: 'population', form: 'all' },
            family: { kind: 'autonomy-lines' },
            tags: [],
            top: { n: ALL_POPULATION_TOP },
            headQuery: null,
          },
        },
        head,
        {
          name: 'hover',
          ...common,
          read: {
            shape: 'exact-hover',
            product: 'autonomy-levels',
            params: { branchId: ctx.branch.id },
            validate: validateAutonomyLevels,
          },
        },
      ],
    };
  },
  projection: autonomyLinesProjection,
  projectionOptions: (ctx, { extent, surface }) => ({
    filter: ctx.countries.filter,
    filterAll: ctx.countries.all,
    bySubject: ctx.options.countriesAre === 'subjects',
    grow: ctx.options.grow === true,
    holders: ctx.countries.holders,
    extent,
    gaps: branchGapsOf(ctx.branch),
    surface,
  }),
};
