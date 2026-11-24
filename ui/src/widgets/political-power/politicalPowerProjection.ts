import { CHART_MEASURE_COLOURS } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import type { HolderRun, HourRanges } from '../../families/holders.ts';
import { type TimeChartProjection, type TimeChartWidgetDeclaration } from '../../families/time-chart/declaration.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { measureArtHandle } from '../../families/time-chart/measureArt.ts';
import { chartAxis, chartWindow, DEFAULT_POWER_SETTINGS, powerChartSettings, type PowerChartSettings } from '../../families/time-chart/settings.ts';
import type { ChartArtSlot, ChartGap, TimeChartSeriesModel } from '../../families/time-chart/model.ts';
import {
  branchGapsOf,
  countrySeriesLooks,
  exactHoverReadings,
  memberAt,
  missingSpansOf,
  rankEntry,
  rankedOrder, unnamedLooks,
  spendMarkerEntry,
  spendMarkersOf,
  auditReading,
  formatPowerAmount,
  headReading,
  headValueOf,
  type HeadReading,
  type KnownHeadReading,
} from '../../families/time-chart/seriesHelpers.ts';

export interface PoliticalPowerOptions {
  readonly tags: readonly string[];
  readonly holders?: Readonly<Record<string, readonly HolderRun[]>> | null;
  readonly membership?: Readonly<Record<string, HourRanges>> | null;
  readonly extent: readonly [number, number];
  readonly gaps: readonly ChartGap[];
  readonly surface: string;
  readonly settings?: PowerChartSettings;
}


export const politicalPowerProjection: TimeChartProjection<PoliticalPowerOptions> = registerFamilyProjection({
  kind: 'political-power',
  family: 'time-chart',
  variant: 'line',
  joinMode: 'join',
  measureTitle: 'Political power',
  project(readView: BoundedReadView, options: PoliticalPowerOptions) {
    const hour = readView.hour;
    const seriesMap = readView.series('main');
    const tags = options.tags;
    const single = tags.length === 1;
    const settings = options.settings ?? DEFAULT_POWER_SETTINGS;
    const format = (reading: KnownHeadReading) => formatPowerAmount(reading, settings.decimals);
    const view = chartWindow(settings, hour, options.extent[0]);
    const rowOf = new Map(tags.map((tag, row) => [tag, row]));

    const headByTag = new Map<string, number | null>();
    const readingByTag = new Map<string, HeadReading | null>();
    for (const tag of tags) {
      const reading = memberAt(options.membership, tag, hour) ? headReading(readView, 'main', { tag }, 5) : undefined;
      if (reading !== undefined) readingByTag.set(tag, reading);
      headByTag.set(tag, headValueOf(reading));
    }
    const exact = exactHoverReadings(readView, 'hover', seriesMap?.get(tags[0] ?? '')?.decimals ?? 5);

    const present = tags.filter(tag => memberAt(options.membership, tag, hour));
    const presentSet = new Set(present);
    const ranking = rankedOrder(readView, present);
    const ordered = [...ranking.order, ...tags.filter(tag => !presentSet.has(tag))];
    const looks = unnamedLooks(countrySeriesLooks(ordered, readView, {
      holders: options.holders,
      from: options.extent[0],
      surface: options.surface,
      withFlag: !single,
      recordedOrdinal: tag => rowOf.get(tag) ?? 0,
      singleColor: CHART_MEASURE_COLOURS.politicalPower,
    }), ranking.unnamed, hour);
    const measureIcon: ChartArtSlot | undefined = single
      ? { art: readView.art(measureArtHandle('politicalPower')), label: 'Political power' }
      : undefined;

    const series: TimeChartSeriesModel[] = ordered.map((tag, rank) => {
      const isTail = !single && rank >= 8;
      const bounded = seriesMap?.get(tag) ?? null;
      const { name, flag, color, identities } = looks.get(tag)!;
      const reading = readingByTag.get(tag);
      const headValue = headByTag.get(tag) ?? null;
      return {
        id: tag,
        key: tag,
        name,
        color,
        role: isTail ? 'tail' : 'primary',
        step: false,
        area: single && settings.area,
        negRed: true,
        bounded,
        headValue,
        headHour: hour,
        exact: exact?.[rowOf.get(tag) ?? -1] ?? null,
        flag,
        ...(single ? {} : { identities }),
        icon: measureIcon,
        missingSpans: missingSpansOf(bounded),
        endLabel: isTail || !settings.endLabels || !reading || !('value' in reading)
          ? null
          : {
              ...(single ? { icon: measureIcon } : { flag }),
              name: single ? null : name,
              fullName: single ? 'Political power' : name,
              value: formatPowerAmount(reading, settings.decimals),
            },
      };
    });

    const spent = spendMarkersOf(readView, 'Political power spent', options.extent[0]);
    const warnings = [...ranking.warnings, ...spent.warnings];

    return {
      kind: 'political-power',
      measureTitle: 'Political power',
      mode: 'lines',
      hour,
      extent: options.extent,
      ...(view ? { view } : {}),
      yAxis: { format: 'power', decimals: settings.decimals, ...chartAxis(settings) },
      series,
      markers: spent.markers.length > 0 ? spent.markers : undefined,
      ...(warnings.length > 0 ? { warnings } : {}),
      gaps: options.gaps.length > 0 ? options.gaps : undefined,
      auditRows: ordered.flatMap(tag => {
        const reading = readingByTag.get(tag);
        return reading === undefined ? [] : [auditReading(tag, reading, format)];
      }),
    };
  },
});

export const politicalPowerDeclaration: TimeChartWidgetDeclaration<PoliticalPowerOptions> = {
  family: 'time-chart',
  kind: 'political-power',
  label: 'Political power',
  subject: 'political power',
  emptyCampaign: 'Load a campaign to see political power.',
  icon: 'chart-line',
  countries: { mode: 'multi', allowSlots: true },
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
    const rank = rankEntry(ctx, family, tags);
    const spending = spendMarkerEntry(ctx, 'Political');
    return {
      owner: `political-power:${ctx.track.source.id}:${ctx.branch.id}:${tags.join(',')}:${ctx.options.spendTicks !== false ? 1 : 0}`,
      entries: [
        ...(rank ? [rank] : []),
        ...(spending ? [spending] : []),
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
            headQuery: tags.length
              ? {
                  product: 'series-state',
                  coverage: 'row',
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
      settings: powerChartSettings(ctx.options),
    };
  },
};
