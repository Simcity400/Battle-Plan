import { CHART_MEASURE_COLOURS } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { ArtHandle, BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import type { HolderRun } from '../../families/holders.ts';
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
  missingSpansOf,
  seriesHeadValue,
  withinMapOf,
} from '../../families/time-chart/seriesHelpers.ts';
import { OPTIONS } from './definition.ts';
import { COUNTRY_FUEL, QUANTITY_SCALE } from './fuelData.ts';

export interface FuelOptions {
  readonly tags: readonly string[];
  readonly holders?: Readonly<Record<string, readonly HolderRun[]>> | null;
  readonly extent: readonly [number, number];
  readonly gaps: readonly ChartGap[];
  readonly surface: string;
  readonly showCapacity: boolean;
  readonly showArea: boolean;
  readonly yMin: number | null;
  readonly yMax: number | null;
  readonly log: boolean;
}

const FUEL_ICON_HANDLE: ArtHandle = { kind: 'sprite', key: 'GFX_fuel_texticon', frame: 0 };

function formatFuelEnd(value: number): string {
  return formatChartAxisValue(value, 'compact');
}

export const fuelProjection: TimeChartProjection<FuelOptions> = registerTimeChart({
  kind: 'fuel',
  family: 'series',
  variant: 'rate-with-ceiling',
  joinMode: 'join',
  measureTitle: 'Fuel',
  project(readView: BoundedReadView, options: FuelOptions) {
    const hour = readView.hour;
    const fuelMap = readView.series('fuel');
    const capMap = options.showCapacity ? readView.series('capacity') : undefined;
    const fuelAnswer = readView.answer('fuel');
    const capAnswer = options.showCapacity ? readView.answer('capacity') : undefined;
    const tags = options.tags;
    const single = tags.length === 1;
    const scale = 1 / QUANTITY_SCALE;

    const headByTag = new Map<string, number | null>();
    const capByTag = new Map<string, number | null>();
    for (let i = 0; i < tags.length; i++) {
      const tag = tags[i]!;
      headByTag.set(tag, seriesHeadValue(fuelMap?.get(tag), fuelAnswer, i, 0, scale));
      if (options.showCapacity) {
        capByTag.set(tag, seriesHeadValue(capMap?.get(tag), capAnswer, i, 0, scale));
      }
    }

    const ordered = [...tags].sort((a, b) => (headByTag.get(b) ?? -Infinity) - (headByTag.get(a) ?? -Infinity));
    const colourMap = assignCountrySeriesColours(
      ordered,
      options.holders,
      readView,
      hour,
      options.surface,
      CHART_MEASURE_COLOURS.fuel,
    );

    const measureIcon: ChartArtSlot | undefined = single
      ? { art: readView.art(FUEL_ICON_HANDLE), label: 'Fuel' }
      : undefined;

    const series: TimeChartSeriesModel[] = [];
    for (let rank = 0; rank < ordered.length; rank++) {
      const tag = ordered[rank]!;
      const isTail = !single && rank >= 8;
      const bounded = fuelMap?.get(tag) ?? null;
      const { name, flag } = countrySeriesIdentity(tag, options.holders, readView, hour, !single);
      const color = isTail ? 'rgba(245, 245, 245, 0.22)' : (colourMap.get(tag) ?? CHART_MEASURE_COLOURS.fuel);
      const headValue = headByTag.get(tag) ?? null;
      const capBounded = options.showCapacity && !isTail ? (capMap?.get(tag) ?? null) : null;
      const capHead = options.showCapacity && !isTail ? (capByTag.get(tag) ?? null) : null;

      series.push({
        id: tag,
        key: tag,
        name: single ? 'Fuel' : name,
        color,
        role: isTail ? 'tail' : 'primary',
        step: false,
        area: single && options.showArea,
        bounded,
        valueScale: scale,
        headValue,
        headHour: hour,
        flag,
        icon: single ? measureIcon : undefined,
        missingSpans: missingSpansOf(bounded),
        rank,
        endLabel: isTail || headValue === null
          ? null
          : {
              ...(single ? { icon: measureIcon } : { flag }),
              name: single ? (options.showCapacity ? 'Fuel' : null) : name,
              fullName: single ? 'Fuel' : name,
              value: formatFuelEnd(headValue),
            },
      });

      if (capBounded || capHead !== null) {
        series.push({
          id: `${tag}:capacity`,
          key: `${tag}:capacity`,
          name: single ? 'Capacity' : `${name} cap`,
          color,
          role: 'ceiling',
          step: true,
          dash: [5, 4],
          opacity: 0.55,
          width: 1.4,
          bounded: capBounded,
          valueScale: scale,
          headValue: capHead,
          headHour: hour,
          rank,
          endLabel: capHead === null
            ? null
            : {
                name: single ? 'Capacity' : `${name} cap`,
                fullName: single ? 'Fuel capacity' : `${name} capacity`,
                value: formatFuelEnd(capHead),
                valueMuted: true,
              },
        });
      }
    }

    return {
      kind: 'fuel',
      measureTitle: 'Fuel',
      mode: 'lines',
      hour,
      extent: options.extent,
      yAxis: {
        format: 'compact',
        min: options.yMin,
        max: options.yMax,
        log: options.log,
      },
      series,
      gaps: options.gaps.length > 0 ? options.gaps : undefined,
      auditRows: ordered.slice(0, 8).flatMap(tag => {
        const v = headByTag.get(tag);
        return v === null || v === undefined ? [] : [{ row: tag, key: tag, value: formatFuelEnd(v) }];
      }),
    };
  },
});

export const fuelDeclaration: TimeChartWidgetDeclaration<FuelOptions> = {
  family: 'series',
  kind: 'fuel',
  label: 'Fuel',
  subject: 'fuel',
  emptyCampaign: 'Load a campaign to see fuel.',
  icon: 'chart-line',
  optionFields: OPTIONS,
  countries: { mode: 'multi', labels: true, colours: true, allowSlots: true },
  sources(ctx: FamilyPlanContext): SourcePlan {
    const tags = ctx.countries.tags;
    const within = withinMapOf(ctx.countries.membership);
    const countries = tags.map(countryFromKey);
    const showCapacity = ctx.options.capacity !== false;
    const fuelFamily = { kind: COUNTRY_FUEL, measure: 'fuel' };
    const capFamily = { kind: COUNTRY_FUEL, measure: 'capacity' };
    const entries: SourcePlan['entries'][number][] = [
      {
        name: 'fuel',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'pixel-binned',
          coverage: { mode: 'keyed', keys: tags },
          family: fuelFamily,
          tags,
          ...(within ? { within } : {}),
          marks: false,
          headQuery: tags.length
            ? {
                product: 'series-state',
                params: { branchId: ctx.branch.id, family: fuelFamily, reads: countries.map(country => ({ country })) },
              }
            : null,
          exactProduct: 'series-state',
        },
      },
    ];
    if (showCapacity) {
      entries.push({
        name: 'capacity',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'pixel-binned',
          coverage: { mode: 'keyed', keys: tags },
          family: capFamily,
          tags,
          ...(within ? { within } : {}),
          marks: false,
          headQuery: tags.length
            ? {
                product: 'series-state',
                params: { branchId: ctx.branch.id, family: capFamily, reads: countries.map(country => ({ country })) },
              }
            : null,
          exactProduct: 'series-state',
        },
      });
    }
    if (tags.length === 1) {
      entries.push({
        name: 'hover',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'exact-hover',
          product: 'country-fuel',
          params: { branchId: ctx.branch.id, country: countries[0]! },
        },
      });
    }
    return {
      owner: `fuel:${ctx.track.source.id}:${ctx.branch.id}:${tags.join(',')}:${showCapacity ? 1 : 0}`,
      entries,
    };
  },
  projection: fuelProjection,
  projectionOptions(ctx, { extent, surface }) {
    return {
      tags: ctx.countries.tags,
      holders: ctx.countries.holders,
      extent,
      gaps: branchGapsOf(ctx.branch),
      surface,
      showCapacity: ctx.options.capacity !== false,
      showArea: ctx.options.areaFill !== false,
      yMin: typeof ctx.options.yMin === 'number' ? ctx.options.yMin : null,
      yMax: typeof ctx.options.yMax === 'number' ? ctx.options.yMax : null,
      log: ctx.options.log === true,
    };
  },
};
