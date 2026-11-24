import { CHART_MEASURE_COLOURS } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { BoundedReadView, FamilyPlanContext, SourceEntry, SourcePlan } from '../../families/contract.ts';
import { type TimeChartProjection, type TimeChartWidgetDeclaration } from '../../families/time-chart/declaration.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { measureArtHandle, type ChartMeasureArt } from '../../families/time-chart/measureArt.ts';
import type { ChartGap, TimeChartSeriesModel } from '../../families/time-chart/model.ts';
import {
  branchGapsOf,
  exactHoverReadings,
  missingSpansOf,
  withinMapOf,
  auditReading,
  headReading,
  headValueOf,
  type HeadReading,
  type KnownHeadReading,
} from '../../families/time-chart/seriesHelpers.ts';
import { formatRational, formatScalar } from '../../families/valueFormat.ts';

export const EXPERIENCE_SCALE = 1 / 32_768;
const EXPERIENCE_RATIONAL_SCALE = { denominator: 32_768n } as const;

export const EXPERIENCE_BRANCHES: readonly { readonly id: 'army' | 'navy' | 'air'; readonly label: string; readonly colour: string; readonly art: ChartMeasureArt }[] = [
  { id: 'army', label: 'Army', colour: CHART_MEASURE_COLOURS.armyExperience, art: 'armyExperience' },
  { id: 'navy', label: 'Navy', colour: CHART_MEASURE_COLOURS.navyExperience, art: 'navyExperience' },
  { id: 'air', label: 'Air', colour: CHART_MEASURE_COLOURS.airExperience, art: 'airExperience' },
];

export interface ExperienceOptions {
  readonly tag: string | null;
  readonly extent: readonly [number, number];
  readonly gaps: readonly ChartGap[];
}

export function formatExperience(value: number): string {
  return formatScalar(value, { decimals: 0, maxDecimals: 2 });
}

function formatExperienceReading(reading: KnownHeadReading): string {
  if (reading.raw !== undefined && reading.denominator !== undefined) {
    return formatRational(reading.raw, reading.denominator, { maxDecimals: 2 });
  }
  return formatExperience(reading.value);
}

export const experienceProjection: TimeChartProjection<ExperienceOptions> = registerFamilyProjection({
  kind: 'experience',
  family: 'time-chart',
  variant: 'line',
  joinMode: 'join',
  measureTitle: 'Experience',
  project(readView: BoundedReadView, options: ExperienceOptions) {
    const hour = readView.hour;
    const tag = options.tag;
    const readings: [string, HeadReading | null][] = [];
    const series: TimeChartSeriesModel[] = EXPERIENCE_BRANCHES.map((branch, rank) => {
      const bounded = tag ? readView.series(branch.id)?.get(tag) ?? null : null;
      const reading = tag ? headReading(readView, branch.id, { tag }, 0, EXPERIENCE_RATIONAL_SCALE) : null;
      if (tag) readings.push([branch.id, reading]);
      const headValue = headValueOf(reading);
      const exact = exactHoverReadings(readView, `${branch.id}Hover`, 0, EXPERIENCE_RATIONAL_SCALE)?.[0] ?? null;
      const icon = { art: readView.art(measureArtHandle(branch.art)), label: `${branch.label} experience` };
      return {
        id: branch.id,
        key: branch.id,
        name: branch.label,
        color: branch.colour,
        role: 'primary',
        step: false,
        bounded,
        valueScale: EXPERIENCE_SCALE,
        headValue,
        headHour: hour,
        exact,
        icon,
        missingSpans: missingSpansOf(bounded),
        rank,
        endLabel: !reading || !('value' in reading)
          ? null
          : { icon, name: branch.label, fullName: `${branch.label} experience`, value: formatExperienceReading(reading) },
      };
    });
    return {
      kind: 'experience',
      measureTitle: 'Experience',
      mode: 'lines',
      hour,
      extent: options.extent,
      yAxis: { format: 'number', decimals: 2 },
      series,
      gaps: options.gaps.length > 0 ? options.gaps : undefined,
      auditRows: readings.map(([row, reading]) => auditReading(row, reading, formatExperienceReading)),
    };
  },
});

function branchEntries(ctx: FamilyPlanContext, branch: string, tag: string): SourceEntry[] {
  const family = { kind: 'country-experience', measure: branch };
  const within = withinMapOf(ctx.countries.membership);
  const reads = [{ country: countryFromKey(tag) }];
  return [
    {
      name: branch,
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'pixel-binned',
        coverage: { mode: 'keyed', keys: [tag] },
        family,
        tags: [tag],
        ...(within ? { within } : {}),
        headQuery: { product: 'series-state', params: { branchId: ctx.branch.id, family, reads } },
        exactProduct: 'series-state',
      },
    },
    {
      name: `${branch}Hover`,
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: { shape: 'exact-hover', product: 'series-state', params: { branchId: ctx.branch.id, family, reads } },
    },
  ];
}

export const experienceDeclaration: TimeChartWidgetDeclaration<ExperienceOptions> = {
  family: 'time-chart',
  kind: 'experience',
  label: 'Experience',
  subject: 'experience',
  emptyCampaign: 'Load a campaign to see experience.',
  icon: 'chart-line',
  countries: { mode: 'multi', allowSlots: true },
  sources(ctx: FamilyPlanContext): SourcePlan {
    const tag = ctx.countries.tags[0];
    return {
      owner: `experience:${ctx.track.source.id}:${ctx.branch.id}:${tag ?? ''}`,
      entries: tag ? EXPERIENCE_BRANCHES.flatMap(branch => branchEntries(ctx, branch.id, tag)) : [],
    };
  },
  projection: experienceProjection,
  projectionOptions(ctx, { extent }) {
    return { tag: ctx.countries.tags[0] ?? null, extent, gaps: branchGapsOf(ctx.branch) };
  },
};
