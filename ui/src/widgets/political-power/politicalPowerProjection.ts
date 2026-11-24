import { CHART_MEASURE_COLOURS } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import type { HolderRun, HourRanges } from '../../families/holders.ts';
import {
  registerTimeChart,
  type TimeChartProjection,
  type TimeChartWidgetDeclaration,
} from '../../families/time-chart/declaration.ts';
import { formatChartAxisValue } from '../../families/time-chart/layout.ts';
import { measureArtHandle } from '../../families/time-chart/measureArt.ts';
import type { ChartArtSlot, ChartGap, TimeChartSeriesModel } from '../../families/time-chart/model.ts';
import {
  assignCountrySeriesColours,
  branchGapsOf,
  countrySeriesIdentity,
  exactHoverReadings,
  memberAt,
  missingSpansOf,
  seriesHeadValue,
  spendMarkersOf,
} from '../../families/time-chart/seriesHelpers.ts';

export interface PoliticalPowerOptions {
  readonly tags: readonly string[];
  readonly holders?: Readonly<Record<string, readonly HolderRun[]>> | null;
  readonly membership?: Readonly<Record<string, HourRanges>> | null;
  readonly extent: readonly [number, number];
  readonly gaps: readonly ChartGap[];
  readonly surface: string;
}

function formatEndAmount(value: number): string {
  if (Math.abs(value) >= 10_000) return formatChartAxisValue(value, 'power');
  const sign = value < 0 ? '−' : '';
  return `${sign}${Math.round(Math.abs(value)).toLocaleString('en-US')}`;
}

export const politicalPowerProjection: TimeChartProjection<PoliticalPowerOptions> = registerTimeChart({
  kind: 'political-power',
  family: 'series',
  variant: 'line',
  joinMode: 'join',
  measureTitle: 'Political power',
  project(readView: BoundedReadView, options: PoliticalPowerOptions) {
    const hour = readView.hour;
    const seriesMap = readView.series('main');
    const answer = readView.answer('main');
    const tags = options.tags;
    const single = tags.length === 1;
    const rowOf = new Map(tags.map((tag, row) => [tag, row]));

    const headByTag = new Map<string, number | null>();
    for (let i = 0; i < tags.length; i++) {
      const tag = tags[i]!;
      headByTag.set(tag, memberAt(options.membership, tag, hour) ? seriesHeadValue(seriesMap?.get(tag), answer, i, 5) : null);
    }
    const exact = exactHoverReadings(readView, 'hover', seriesMap?.get(tags[0] ?? '')?.decimals ?? 5);

    const ordered = [...tags].sort((a, b) => (headByTag.get(b) ?? -Infinity) - (headByTag.get(a) ?? -Infinity));
    const colourMap = assignCountrySeriesColours(ordered, options.holders, readView, hour, options.surface,
      CHART_MEASURE_COLOURS.politicalPower, tag => rowOf.get(tag) ?? 0);
    const measureIcon: ChartArtSlot | undefined = single
      ? { art: readView.art(measureArtHandle('politicalPower')), label: 'Political power' }
      : undefined;

    const series: TimeChartSeriesModel[] = ordered.map((tag, rank) => {
      const isTail = !single && rank >= 8;
      const bounded = seriesMap?.get(tag) ?? null;
      const { name, flag } = countrySeriesIdentity(tag, options.holders, readView, hour, !single);
      const color = isTail ? 'rgba(245, 245, 245, 0.22)' : (colourMap.get(tag) ?? CHART_MEASURE_COLOURS.politicalPower);
      const headValue = headByTag.get(tag) ?? null;
      return {
        id: tag,
        key: tag,
        name,
        color,
        role: isTail ? 'tail' : 'primary',
        step: false,
        area: single,
        negRed: true,
        bounded,
        headValue,
        headHour: hour,
        exact: exact?.[rowOf.get(tag) ?? -1] ?? null,
        flag,
        icon: measureIcon,
        missingSpans: missingSpansOf(bounded),
        rank,
        endLabel: isTail || headValue === null
          ? null
          : {
              ...(single ? { icon: measureIcon } : { flag }),
              name: single ? null : name,
              fullName: single ? 'Political power' : name,
              value: formatEndAmount(headValue),
            },
      };
    });

    const markers = single ? spendMarkersOf(seriesMap?.get(tags[0]!), 'Political power spent') : [];

    return {
      kind: 'political-power',
      measureTitle: 'Political power',
      mode: 'lines',
      hour,
      extent: options.extent,
      yAxis: { format: 'power', decimals: 0 },
      series,
      markers: markers.length > 0 ? markers : undefined,
      gaps: options.gaps.length > 0 ? options.gaps : undefined,
      auditRows: ordered.slice(0, 8).flatMap(tag => {
        const v = headByTag.get(tag);
        return v === null || v === undefined ? [] : [{ row: tag, key: tag, value: formatEndAmount(v) }];
      }),
    };
  },
});

export const politicalPowerDeclaration: TimeChartWidgetDeclaration<PoliticalPowerOptions> = {
  family: 'series',
  kind: 'political-power',
  label: 'Political power',
  subject: 'political power',
  emptyCampaign: 'Load a campaign to see political power.',
  icon: 'chart-line',
  countries: { mode: 'multi', labels: true, colours: true, allowSlots: true },
  sources(ctx: FamilyPlanContext): SourcePlan {
    const tags = ctx.countries.tags;
    const withinEntries = ctx.countries.membership
      ? Object.entries(ctx.countries.membership)
          .filter(([t]) => t.length > 0)
          .map(([t, ranges]) => [t, ranges.map(([firstHour, lastHour]) => ({ firstHour, lastHour }))] as const)
      : [];
    const within = withinEntries.length > 0 ? new Map(withinEntries) : undefined;
    const countries = tags.map(countryFromKey);
    const family = { kind: 'country-measure', measure: 'politicalPower' };
    return {
      owner: `political-power:${ctx.track.source.id}:${ctx.branch.id}:${tags.join(',')}`,
      entries: [
        {
          name: 'main',
          source: ctx.track.source,
          branch: ctx.branch.id,
          offset: ctx.track.axisOffset,
          read: {
            shape: 'pixel-binned',
            coverage: { mode: 'keyed', keys: tags },
            family,
            tags,
            ...(within ? { within } : {}),
            marks: tags.length === 1,
            headQuery: tags.length
              ? {
                  product: 'series-state',
                  params: {
                    branchId: ctx.branch.id,
                    family,
                    reads: countries.map(country => ({ country })),
                  },
                }
              : null,
            exactProduct: 'series-state',
          },
        },
        ...(tags.length
          ? [{
              name: 'hover',
              source: ctx.track.source,
              branch: ctx.branch.id,
              offset: ctx.track.axisOffset,
              read: {
                shape: 'exact-hover' as const,
                product: 'series-state' as const,
                params: {
                  branchId: ctx.branch.id,
                  family,
                  reads: countries.map(country => ({ country })),
                },
              },
            }]
          : []),
      ],
    };
  },
  projection: politicalPowerProjection,
  projectionOptions(ctx, { extent, surface }) {
    const gaps = branchGapsOf(ctx.branch);
    return {
      tags: ctx.countries.tags,
      holders: ctx.countries.holders,
      membership: ctx.countries.membership,
      extent,
      gaps,
      surface,
    };
  },
};
