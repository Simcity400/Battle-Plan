import { CHART_MEASURE_COLOURS } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import type { HolderRun, HourRanges } from '../../families/holders.ts';
import { measureArtHandle } from '../../families/time-chart/measureArt.ts';
import {
  registerTimeChart,
  type TimeChartProjection,
  type TimeChartWidgetDeclaration,
} from '../../families/time-chart/declaration.ts';
import { formatChartAxisValue } from '../../families/time-chart/layout.ts';
import type { ChartArtSlot, ChartGap, TimeChartSeriesModel } from '../../families/time-chart/model.ts';
import {
  assignCountrySeriesColours,
  branchGapsOf,
  countrySeriesIdentity,
  exactHoverReadings,
  memberAt,
  missingSpansOf,
  rankEntry,
  rankedOrder,
  seriesHeadAt,
  spendMarkersOf,
  withinMapOf,
} from '../../families/time-chart/seriesHelpers.ts';

export interface CommandPowerOptions {
  readonly tags: readonly string[];
  readonly holders?: Readonly<Record<string, readonly HolderRun[]>> | null;
  readonly membership?: Readonly<Record<string, HourRanges>> | null;
  readonly extent: readonly [number, number];
  readonly gaps: readonly ChartGap[];
  readonly surface: string;
  readonly showCeiling: boolean;
  readonly showSpendTicks: boolean;
}


function formatEndAmount(value: number): string {
  return formatChartAxisValue(value, 'power');
}

export const commandPowerProjection: TimeChartProjection<CommandPowerOptions> = registerTimeChart({
  kind: 'command-power',
  family: 'time-chart',
  variant: 'rate-with-ceiling',
  joinMode: 'join',
  measureTitle: 'Command power',
  project(readView: BoundedReadView, options: CommandPowerOptions) {
    const hour = readView.hour;
    const mainMap = readView.series('main');
    const ceilingMap = options.showCeiling ? readView.series('ceiling') : undefined;
    const tags = options.tags;
    const single = tags.length === 1;

    const headByTag = new Map<string, number | null>();
    const ceilingByTag = new Map<string, number | null>();
    for (let i = 0; i < tags.length; i++) {
      const tag = tags[i]!;
      const member = memberAt(options.membership, tag, hour);
      headByTag.set(tag, member ? seriesHeadAt(readView, 'main', tag, i, 5) : null);
      if (options.showCeiling) {
        ceilingByTag.set(tag, member ? seriesHeadAt(readView, 'ceiling', tag, i, 5) : null);
      }
    }

    const exact = exactHoverReadings(readView, 'hover', mainMap?.get(tags[0] ?? '')?.decimals ?? 5);
    const rowOf = new Map(tags.map((tag, row) => [tag, row]));
    const present = tags.filter(tag => memberAt(options.membership, tag, hour));
    const presentSet = new Set(present);
    const ordered = [...rankedOrder(readView, present), ...tags.filter(tag => !presentSet.has(tag))];
    const colourMap = assignCountrySeriesColours(
      ordered,
      options.holders,
      readView,
      hour,
      options.surface,
      CHART_MEASURE_COLOURS.commandPower,
      tag => rowOf.get(tag) ?? 0,
    );

    const measureIcon: ChartArtSlot | undefined = single
      ? { art: readView.art(measureArtHandle('commandPower')), label: 'Command power' }
      : undefined;

    const series: TimeChartSeriesModel[] = [];
    for (let rank = 0; rank < ordered.length; rank++) {
      const tag = ordered[rank]!;
      const isTail = !single && rank >= 8;
      const bounded = mainMap?.get(tag) ?? null;
      const { name, flag } = countrySeriesIdentity(tag, options.holders, readView, hour, !single);
      const color = isTail ? CHART_MEASURE_COLOURS.tailLine : (colourMap.get(tag) ?? CHART_MEASURE_COLOURS.commandPower);
      const headValue = headByTag.get(tag) ?? null;
      const ceilingBounded = options.showCeiling && !isTail ? (ceilingMap?.get(tag) ?? null) : null;
      const ceilingHead = options.showCeiling && !isTail ? (ceilingByTag.get(tag) ?? null) : null;

      series.push({
        id: tag,
        key: tag,
        name: single ? 'Command power' : name,
        color,
        role: isTail ? 'tail' : 'primary',
        step: false,
        area: single,
        bounded,
        headValue,
        headHour: hour,
        exact: exact?.[rowOf.get(tag) ?? -1] ?? null,
        flag,
        icon: single ? measureIcon : undefined,
        missingSpans: missingSpansOf(bounded),
        rank,
        endLabel: isTail || headValue === null
          ? null
          : {
              ...(single ? { icon: measureIcon } : { flag }),
              name: single ? (options.showCeiling ? 'Command power' : null) : name,
              fullName: single ? 'Command power' : name,
              value: formatEndAmount(headValue),
            },
      });

      if (ceilingBounded || ceilingHead !== null) {
        series.push({
          id: `${tag}:ceiling`,
          key: `${tag}:ceiling`,
          name: single ? 'Maximum' : `${name} max`,
          color,
          role: 'ceiling',
          step: true,
          dash: [5, 4],
          opacity: 0.55,
          width: 1.4,
          bounded: ceilingBounded,
          headValue: ceilingHead,
          headHour: hour,
          rank,
          endLabel: ceilingHead === null
            ? null
            : {
                name: single ? 'Maximum' : `${name} max`,
                fullName: single ? 'Maximum command power' : `${name} maximum`,
                value: formatEndAmount(ceilingHead),
                valueMuted: true,
              },
        });
      }
    }

    const markers = single && options.showSpendTicks
      ? spendMarkersOf(mainMap?.get(tags[0]!), 'Command power spent')
      : [];

    return {
      kind: 'command-power',
      measureTitle: 'Command power',
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

export const commandPowerDeclaration: TimeChartWidgetDeclaration<CommandPowerOptions> = {
  family: 'time-chart',
  kind: 'command-power',
  label: 'Command power',
  subject: 'command power',
  emptyCampaign: 'Load a campaign to see command power.',
  icon: 'chart-line',
  countries: { mode: 'multi', labels: true, colours: true, allowSlots: true },
  sources(ctx: FamilyPlanContext): SourcePlan {
    const tags = ctx.countries.tags;
    const within = withinMapOf(ctx.countries.membership);
    const countries = tags.map(countryFromKey);
    const showCeiling = ctx.options.ceiling !== false;
    const mainFamily = { kind: 'country-measure', measure: 'commandPower' };
    const ceilingFamily = { kind: 'country-stability', measure: 'maxCommandPower' };
    const entries: SourcePlan['entries'][number][] = [
      {
        name: 'main',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'pixel-binned',
          coverage: { mode: 'keyed', keys: tags },
          family: mainFamily,
          tags,
          ...(within ? { within } : {}),
          marks: tags.length === 1 && ctx.options.spendTicks !== false,
          headQuery: tags.length
            ? {
                product: 'series-state',
                params: { branchId: ctx.branch.id, family: mainFamily, reads: countries.map(country => ({ country })) },
              }
            : null,
          exactProduct: 'series-state',
        },
      },
    ];
    if (showCeiling) {
      entries.push({
        name: 'ceiling',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'pixel-binned',
          coverage: { mode: 'keyed', keys: tags },
          family: ceilingFamily,
          tags,
          ...(within ? { within } : {}),
          marks: false,
          headQuery: tags.length
            ? {
                product: 'series-state',
                params: { branchId: ctx.branch.id, family: ceilingFamily, reads: countries.map(country => ({ country })) },
              }
            : null,
          exactProduct: 'series-state',
        },
      });
    }
    if (tags.length) {
      entries.push({
        name: 'hover',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'exact-hover',
          product: 'series-state',
          params: { branchId: ctx.branch.id, family: mainFamily, reads: countries.map(country => ({ country })) },
        },
      });
    }
    const rank = rankEntry(ctx, mainFamily, tags);
    if (rank) entries.push(rank);
    return {
      owner: `command-power:${ctx.track.source.id}:${ctx.branch.id}:${tags.join(',')}:${showCeiling ? 1 : 0}:${ctx.following ? 1 : 0}`,
      entries,
    };
  },
  projection: commandPowerProjection,
  projectionOptions(ctx, { extent, surface }) {
    return {
      tags: ctx.countries.tags,
      holders: ctx.countries.holders,
      membership: ctx.countries.membership,
      extent,
      gaps: branchGapsOf(ctx.branch),
      surface,
      showCeiling: ctx.options.ceiling !== false,
      showSpendTicks: ctx.options.spendTicks !== false,
    };
  },
};
