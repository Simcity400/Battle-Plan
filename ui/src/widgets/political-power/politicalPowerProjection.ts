import { CHART_MEASURE_COLOURS, colour, nudgeApart, readableColour } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { flagArtHandle } from '../../families/art/provider.ts';
import { failedArt, missingArt, PENDING_ART } from '../../families/art/types.ts';
import type { ArtHandle, BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import { holderTagAt, type HolderRun } from '../../families/holders.ts';
import {
  registerTimeChart,
  type TimeChartProjection,
  type TimeChartWidgetDeclaration,
} from '../../families/time-chart/declaration.ts';
import { formatChartAxisValue } from '../../families/time-chart/layout.ts';
import { exactHoverReadings, seriesHeadValue } from '../../families/time-chart/seriesHelpers.ts';
import type {
  ChartArtSlot,
  ChartGap,
  ChartMarker,
  TimeChartSeriesModel,
} from '../../families/time-chart/model.ts';
import { parseColor, toHex } from '../../shell/appearance/color.ts';

export interface PoliticalPowerOptions {
  readonly tags: readonly string[];
  readonly holders?: Readonly<Record<string, readonly HolderRun[]>> | null;
  readonly extent: readonly [number, number];
  readonly gaps: readonly ChartGap[];
  readonly surface: string;
}

const PP_ICON_HANDLE: ArtHandle = { kind: 'sprite', key: 'GFX_pol_power', frame: 0 };

function formatEndAmount(value: number): string {
  if (Math.abs(value) >= 10_000) return formatChartAxisValue(value, 'power');
  const sign = value < 0 ? '\u2212' : '';
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
    const surface = options.surface;

    const headByTag = new Map<string, number | null>();
    for (let i = 0; i < tags.length; i++) {
      const tag = tags[i]!;
      headByTag.set(tag, seriesHeadValue(seriesMap?.get(tag), answer, i, 5));
    }
    const exact = exactHoverReadings(readView, 'hover', seriesMap?.get(tags[0] ?? '')?.decimals ?? 5);
    const rowOf = new Map(tags.map((tag, row) => [tag, row]));

    const ordered = [...tags].sort((a, b) => (headByTag.get(b) ?? -Infinity) - (headByTag.get(a) ?? -Infinity));
    const topTags = ordered.slice(0, 8);
    const colourMap = new Map<string, string>();
    if (single) {
      colourMap.set(tags[0]!, CHART_MEASURE_COLOURS.politicalPower);
    } else {
      const bg = parseColor(surface) ?? { r: 17, g: 17, b: 17 };
      const rawRgbs = topTags.map((tag, ordinal) => {
        const identityTag = holderTagAt(options.holders?.[tag], hour) ?? tag;
        const who = readView.who('country', identityTag);
        const hex = who.colour?.status === 'ready'
          ? readableColour(who.colour.value, surface)
          : colour(surface, ordinal);
        return parseColor(hex) ?? { r: 120, g: 160, b: 220 };
      });
      const distinct = nudgeApart(rawRgbs, bg);
      topTags.forEach((tag, idx) => colourMap.set(tag, toHex(distinct[idx]!)));
    }

    const measureIcon: ChartArtSlot | undefined = single
      ? { art: readView.art(PP_ICON_HANDLE), label: 'Political power' }
      : undefined;

    const spendHoursSet = new Set<number>();
    const series: TimeChartSeriesModel[] = ordered.map((tag, rank) => {
      const isTail = !single && rank >= 8;
      const bounded = seriesMap?.get(tag) ?? null;
      const identityTag = holderTagAt(options.holders?.[tag], hour) ?? tag;
      const who = readView.who('country', identityTag);
      const name = who.name.status === 'ready' ? who.name.value : identityTag;
      const flag: ChartArtSlot | undefined = single
        ? undefined
        : who.flagChoice?.status === 'ready'
          ? { art: readView.art(flagArtHandle(who.flagChoice.value, 'panel')), tag: identityTag }
          : who.flagChoice?.status === 'absent'
            ? { art: missingArt(who.flagChoice.reason), tag: identityTag }
            : who.flagChoice?.status === 'failed'
              ? { art: failedArt(who.flagChoice.reason, who.flagChoice.retryable), tag: identityTag }
              : { art: PENDING_ART, tag: identityTag };
      const color = isTail ? 'rgba(245, 245, 245, 0.22)' : (colourMap.get(tag) ?? CHART_MEASURE_COLOURS.politicalPower);
      const headValue = headByTag.get(tag) ?? null;

      if (single && bounded) {
        for (const cluster of bounded.marks) {
          if (cluster.bins && cluster.bins.length > 0) {
            for (const [binHour] of cluster.bins) spendHoursSet.add(binHour);
          } else {
            spendHoursSet.add(cluster.firstHour);
          }
        }
      }

      const missingSpans = bounded && bounded.missing.length > 0
        ? [...bounded.missing].map(sp => ({ firstHour: sp.firstHour, lastHour: sp.lastHour, reason: sp.reason ?? 'Not recorded' }))
        : undefined;

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
        icon: single ? measureIcon : undefined,
        missingSpans,
        rank,
        endLabel: isTail || headValue === null
          ? null
          : {
              ...(single ? { icon: measureIcon } : { flag }),
              name: single ? null : name,
              fullName: name,
              value: formatEndAmount(headValue),
            },
      };
    });

    const markers: ChartMarker[] = [...spendHoursSet]
      .sort((a, b) => a - b)
      .map((h, idx) => ({
        id: `spend:${idx}:${h}`,
        hour: h,
        title: 'Political power spent',
      }));

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
      auditRows: topTags.flatMap(tag => {
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
    const rawGaps = (ctx.branch as { gaps?: readonly { firstHour: number; lastHour: number; reason?: string }[] }).gaps;
    const gaps: ChartGap[] = rawGaps?.length
      ? rawGaps.map(g => ({ firstHour: g.firstHour, lastHour: g.lastHour, reason: g.reason ?? 'Recording gap' }))
      : [];
    return {
      tags: ctx.countries.tags,
      holders: ctx.countries.holders,
      extent,
      gaps,
      surface,
    };
  },
};
