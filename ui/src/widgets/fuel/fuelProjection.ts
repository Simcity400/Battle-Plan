import { CHART_MEASURE_COLOURS } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { FutureDataError, type BoundedReadView, type FamilyPlanContext, type SourceEntry, type SourcePlan } from '../../families/contract.ts';
import { type TimeChartProjection, type TimeChartWidgetDeclaration } from '../../families/time-chart/declaration.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { measureArtHandle } from '../../families/time-chart/measureArt.ts';
import type { ChartGap, ChartHoverDetail, TimeChartSeriesModel } from '../../families/time-chart/model.ts';
import {
  branchGapsOf,
  cardItems,
  missingSpansOf,
  withinMapOf,
  auditReading,
  headReading,
  headValueOf,
  type HeadReading,
} from '../../families/time-chart/seriesHelpers.ts';
import { formatCompactScalar, formatDateOnly } from '../../families/valueFormat.ts';
import type { CountryFuelReply } from '../../generated/dashboard';
import { fuelDecimals, fuelTooltips, validateCountryFuel } from './fuelData.ts';

export const FUEL_SCALE = 1 / 32_768;
const CEILING_DASH = [5, 4] as const;

export interface FuelOptions {
  readonly tag: string | null;
  readonly capacity: boolean;
  readonly area: boolean;
  readonly digits: number;
  readonly extent: readonly [number, number];
  readonly gaps: readonly ChartGap[];
}

export function formatFuelAmount(value: number): string {
  const rounded = Math.round(value);
  return rounded === 0 ? '0' : formatCompactScalar(rounded, 'integer').text;
}

export const fuelProjection: TimeChartProjection<FuelOptions> = registerFamilyProjection({
  kind: 'fuel',
  family: 'time-chart',
  variant: 'rate-with-ceiling',
  joinMode: 'join',
  measureTitle: 'Fuel',
  project(readView: BoundedReadView, options: FuelOptions) {
    const hour = readView.hour;
    const tag = options.tag;
    const fuel = tag ? readView.series('fuel')?.get(tag) ?? null : null;
    const readings: [string, HeadReading | null][] = [];
    const fuelReading = tag ? headReading(readView, 'fuel', { tag }, 0, FUEL_SCALE) : null;
    if (tag) readings.push(['fuel', fuelReading]);
    const fuelHead = headValueOf(fuelReading);
    const icon = { art: readView.art(measureArtHandle('fuel')), label: 'Fuel' };
    const series: TimeChartSeriesModel[] = [{
      id: 'fuel',
      key: 'fuel',
      name: 'Fuel',
      color: CHART_MEASURE_COLOURS.fuel,
      role: 'primary',
      step: false,
      area: options.area,
      bounded: fuel,
      valueScale: FUEL_SCALE,
      headValue: fuelHead,
      headHour: hour,
      icon,
      missingSpans: missingSpansOf(fuel),
      rank: 0,
      endLabel: fuelHead === null ? null : { icon, name: 'Fuel', fullName: 'Fuel', value: formatFuelAmount(fuelHead) },
    }];
    if (options.capacity) {
      const capacity = tag ? readView.series('capacity')?.get(tag) ?? null : null;
      const capacityReading = tag ? headReading(readView, 'capacity', { tag }, 0, FUEL_SCALE) : null;
      if (tag) readings.push(['capacity', capacityReading]);
      const capacityHead = headValueOf(capacityReading);
      series.push({
        id: 'capacity',
        key: 'capacity',
        name: 'Capacity',
        color: CHART_MEASURE_COLOURS.fuel,
        role: 'ceiling',
        step: true,
        dash: CEILING_DASH,
        opacity: 0.55,
        width: 1.4,
        bounded: capacity,
        valueScale: FUEL_SCALE,
        headValue: capacityHead,
        headHour: hour,
        missingSpans: missingSpansOf(capacity),
        rank: 1,
        endLabel: capacityHead === null
          ? null
          : { name: 'Capacity', fullName: 'Fuel capacity', value: formatFuelAmount(capacityHead), valueMuted: true },
      });
    }
    return {
      kind: 'fuel',
      measureTitle: 'Fuel',
      mode: 'lines',
      hour,
      extent: options.extent,
      yAxis: { format: 'compact' },
      series,
      gaps: options.gaps.length > 0 ? options.gaps : undefined,
      hoverDetail: fuelHoverDetail(readView, options.digits),
      auditRows: readings.map(([row, reading]) => auditReading(row, reading, formatFuelAmount)),
    };
  },
});

const HOVER_COLOURS = {
  fuel: CHART_MEASURE_COLOURS.fuel,
  capacity: CHART_MEASURE_COLOURS.fuel,
  gained: CHART_MEASURE_COLOURS.supplyOk,
  burned: CHART_MEASURE_COLOURS.short,
};

function fuelHoverDetail(readView: BoundedReadView, digits: number): ChartHoverDetail | null {
  let hover;
  try {
    hover = readView.hover<CountryFuelReply>('fuelHover');
  } catch (error) {
    if (error instanceof FutureDataError) return null;
    throw error;
  }
  if (!hover) return null;
  const [card] = fuelTooltips(hover.data, formatDateOnly(hover.hour), HOVER_COLOURS, digits);
  if (!card) return null;
  const items = cardItems(card.rows, false);
  return { hour: hover.hour, items: card.note ? [...items, { label: card.note, tone: 'muted', divider: true }] : items, replacesSeries: true };
}

function fuelEntry(ctx: FamilyPlanContext, measure: 'fuel' | 'capacity', tag: string): SourceEntry {
  const family = { kind: 'country-fuel', measure };
  const within = withinMapOf(ctx.countries.membership);
  return {
    name: measure,
    source: ctx.track.source,
    branch: ctx.branch.id,
    offset: ctx.track.axisOffset,
    read: {
      shape: 'pixel-binned',
      coverage: { mode: 'keyed', keys: [tag] },
      family,
      tags: [tag],
      ...(within ? { within } : {}),
      headQuery: { product: 'series-state', params: { branchId: ctx.branch.id, family, reads: [{ country: countryFromKey(tag) }] } },
      exactProduct: 'series-state',
    },
  };
}

export const fuelDeclaration: TimeChartWidgetDeclaration<FuelOptions> = {
  family: 'time-chart',
  kind: 'fuel',
  label: 'Fuel',
  subject: 'fuel',
  emptyCampaign: 'Load a campaign to see fuel.',
  icon: 'chart-line',
  countries: { mode: 'multi', allowSlots: true },
  sources(ctx: FamilyPlanContext): SourcePlan {
    const tag = ctx.countries.tags[0];
    const capacity = ctx.options.capacity !== false;
    if (!tag) return { owner: `fuel:${ctx.track.source.id}:${ctx.branch.id}:`, entries: [] };
    return {
      owner: `fuel:${ctx.track.source.id}:${ctx.branch.id}:${tag}:${capacity ? 1 : 0}`,
      entries: [
        fuelEntry(ctx, 'fuel', tag),
        ...(capacity ? [fuelEntry(ctx, 'capacity', tag)] : []),
        {
          name: 'fuelHover',
          source: ctx.track.source,
          branch: ctx.branch.id,
          offset: ctx.track.axisOffset,
          read: {
            shape: 'exact-hover',
            product: 'country-fuel',
            params: { branchId: ctx.branch.id, country: countryFromKey(tag) },
            validate: validateCountryFuel,
          },
        },
      ],
    };
  },
  projection: fuelProjection,
  projectionOptions(ctx, { extent }) {
    return {
      tag: ctx.countries.tags[0] ?? null,
      capacity: ctx.options.capacity !== false,
      area: ctx.options.areaFill !== false,
      digits: fuelDecimals(ctx.options),
      extent,
      gaps: branchGapsOf(ctx.branch),
    };
  },
};
