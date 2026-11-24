import { colour } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { contentKey, contentSet } from '../../dashboard/options.ts';
import type { OptionValue } from '../../dashboard/options';
import { auditRow } from '../../families/auditRow.ts';
import { FutureDataError, type BoundedReadView, type FamilyPlanContext, type SourceEntry, type SourcePlan } from '../../families/contract.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { TimeChartProjection, TimeChartWidgetDeclaration } from '../../families/time-chart/declaration.ts';
import type { ChartGap, ChartHoverDetail, ChartLegendEntry, TimeChartSeriesModel } from '../../families/time-chart/model.ts';
import { branchGapsOf, missingSpansOf, withinMapOf } from '../../families/time-chart/seriesHelpers.ts';
import { formatCompactScalar, formatDateOnly, formatScalar } from '../../families/valueFormat.ts';
import type { StockpileLineAt, StockpileLinesReply, StockpileTileFamily } from '../../generated/dashboard';

const STOCKPILE_TIME_VIEW = 'time';
const MAIN_TYPES = 5;

type Measure = 'stock' | 'share';

interface StockpileTimeOptions {
  readonly chosen: readonly string[] | null;
  readonly measure: Measure;
  readonly endLabels: boolean;
  readonly yMin: number | null;
  readonly yMax: number | null;
  readonly log: boolean;
  readonly extent: readonly [number, number];
  readonly gaps: readonly ChartGap[];
  readonly surface: string;
}

const TITLES: Record<Measure, string> = { stock: 'In stock', share: 'Surplus or shortage, share of need' };

export function formatStockpileValue(value: number, measure: Measure): string {
  return measure === 'share'
    ? formatScalar(value, { decimals: 0, maxDecimals: 1, percent: true })
    : formatCompactScalar(Math.round(value), 'integer').text;
}

export function validateStockpileLines(raw: unknown): StockpileLinesReply {
  const reply = raw as StockpileLinesReply | null;
  if (!reply || typeof reply !== 'object' || !Array.isArray(reply.lines) || typeof reply.hour !== 'number'
    || (reply.nextChange !== null && typeof reply.nextChange !== 'number')) {
    throw new Error('The stockpile lines reply is malformed.');
  }
  return reply;
}

export const stockpileTimeProvenInterval = (data: unknown, hour: number): readonly [number, number] => {
  const next = (data as StockpileLinesReply | null)?.nextChange;
  return [hour, next == null ? Infinity : next - 1];
};

const lineName = (line: StockpileLineAt) => line.name ?? line.key;

function displayedKeys(readView: BoundedReadView, head: StockpileLinesReply | undefined, options: StockpileTimeOptions): readonly string[] {
  if (options.chosen) return options.chosen;
  return head?.top?.keys ?? [...readView.series('stock')?.keys() ?? []];
}

function hoverDetail(readView: BoundedReadView, keys: readonly string[], colours: ReadonlyMap<string, string>, measure: Measure): ChartHoverDetail | null {
  let hover;
  try {
    hover = readView.hover<StockpileLinesReply>('hover');
  } catch (error) {
    if (error instanceof FutureDataError) return null;
    throw error;
  }
  if (!hover) return null;
  const byKey = new Map(hover.data.lines.map(line => [line.key, line]));
  const items: TooltipListItem[] = [{ label: formatDateOnly(hover.hour), strong: true }];
  for (const key of keys) {
    const line = byKey.get(key);
    const label = line ? lineName(line) : key;
    const swatch = colours.get(key);
    items.push(line?.value != null
      ? { label, value: formatStockpileValue(line.value, measure), ...(swatch ? { swatch } : {}) }
      : { label, missingReason: line?.missing ?? 'This campaign has no such equipment type.', tone: 'muted', ...(swatch ? { swatch } : {}) });
  }
  return { hour: hover.hour, items, replacesSeries: true };
}

export const stockpileTimeProjection: TimeChartProjection<StockpileTimeOptions> = registerFamilyProjection<TimeChartProjection<StockpileTimeOptions>>({
  kind: 'equipment-stockpile',
  family: 'time-chart',
  variant: 'line',
  joinMode: 'step',
  measureTitle: 'Equipment stockpile',
  project(readView: BoundedReadView, options: StockpileTimeOptions) {
    const hour = readView.hour;
    const head = readView.panel<StockpileLinesReply>('lines');
    const refused = options.chosen ? null : head?.top?.refused ?? null;
    const byKey = new Map((head?.lines ?? []).map(line => [line.key, line]));
    const keys = refused ? [] : displayedKeys(readView, head, options);
    const tiles = readView.series('stock');
    const colours = new Map(keys.map(key => [key, colour(options.surface, byKey.get(key)?.ordinal ?? byKey.size)]));
    const series: TimeChartSeriesModel[] = keys.map((key, rank) => {
      const line = byKey.get(key);
      const name = line ? lineName(line) : key;
      const bounded = tiles?.get(key) ?? null;
      const headValue = line?.value ?? null;
      const color = colours.get(key)!;
      return {
        id: key,
        key,
        name,
        color,
        role: 'primary',
        step: true,
        bounded,
        headValue,
        headHour: hour,
        absentReason: line ? line.missing : head ? 'This campaign has no such equipment type.' : null,
        missingSpans: missingSpansOf(bounded),
        rank,
        endLabel: !options.endLabels || headValue === null
          ? null
          : { colorBar: color, name, fullName: name, value: formatStockpileValue(headValue, options.measure) },
      };
    });
    const legend: ChartLegendEntry[] | undefined = options.endLabels ? undefined : series.map(s => ({ id: s.id, label: s.name, color: s.color }));
    return {
      kind: 'equipment-stockpile',
      measureTitle: TITLES[options.measure],
      mode: 'lines',
      hour,
      extent: options.extent,
      yAxis: { format: options.measure === 'share' ? 'percent' : 'compact', min: options.yMin, max: options.yMax, log: options.log },
      series,
      ...(legend ? { legend } : {}),
      gaps: options.gaps.length > 0 ? options.gaps : undefined,
      empty: refused ? { message: 'Main types not ranked at this date', detail: refused, kind: 'unavailable' } : null,
      hoverDetail: hoverDetail(readView, keys, colours, options.measure),
      auditRows: series.map(s => auditRow(s.id, s.headValue == null ? (s.absentReason ?? '') : formatStockpileValue(s.headValue, options.measure))),
    };
  },
}, STOCKPILE_TIME_VIEW);

function chosenTypes(options: Readonly<Record<string, OptionValue>>): readonly string[] | null {
  const keys = contentSet(options.chosen ?? '').map(contentKey);
  return keys.length > 0 ? keys : null;
}

const numberOption = (value: OptionValue | undefined) => (typeof value === 'number' && Number.isFinite(value) ? value : null);

function stockpileFamily(ctx: FamilyPlanContext, tag: string): StockpileTileFamily {
  const included = contentSet(ctx.options.included ?? '').map(contentKey);
  const membership = withinMapOf(ctx.countries.membership)?.get(tag)?.flatMap(span => {
    const gated = ctx.gateSpan(span.firstHour, span.lastHour);
    return gated ? [{ firstHour: gated.firstHour, lastHour: gated.lastHour }] : [];
  });
  return {
    kind: 'equipment-stockpile',
    measure: ctx.options.measure === 'share' ? 'share' : 'stock',
    country: countryFromKey(tag),
    included,
    ...(membership ? { membership } : {}),
  };
}

export const stockpileTimeDeclaration: TimeChartWidgetDeclaration<StockpileTimeOptions> = {
  family: 'time-chart',
  kind: 'equipment-stockpile',
  label: 'Equipment stockpile',
  subject: 'equipment stockpile',
  emptyCampaign: 'Load a campaign and choose a country to see its equipment stockpile.',
  icon: 'chart-line',
  countries: { mode: 'multi', allowSlots: true },
  sources(ctx: FamilyPlanContext): SourcePlan {
    const tag = ctx.countries.tags[0];
    const chosen = chosenTypes(ctx.options);
    const owner = `equipment-stockpile:time:${ctx.track.source.id}:${ctx.branch.id}:${tag ?? ''}`;
    if (!tag) return { owner, entries: [] };
    const family = stockpileFamily(ctx, tag);
    const common = { source: ctx.track.source, branch: ctx.branch.id, offset: ctx.track.axisOffset };
    const asked = { branchId: ctx.branch.id, family, ...(chosen ? {} : { top: MAIN_TYPES }) };
    const entries: SourceEntry[] = [
      {
        name: 'stock',
        ...common,
        read: {
          shape: 'pixel-binned',
          coverage: chosen ? { mode: 'keyed', keys: chosen } : { mode: 'population', form: 'top', limit: MAIN_TYPES },
          family,
          tags: chosen ?? [],
          ...(chosen ? {} : { top: { n: MAIN_TYPES } }),
          headQuery: null,
        },
      },
      {
        name: 'lines',
        ...common,
        read: {
          shape: 'point',
          channel: 'panel',
          coverage: { mode: 'keyed', keys: [tag] },
          product: 'equipment-stockpile-lines',
          followQuery: null,
          params: asked,
          provenInterval: stockpileTimeProvenInterval,
          dynamicKeys: ['hour'],
          validate: validateStockpileLines,
        },
      },
      {
        name: 'hover',
        ...common,
        read: { shape: 'exact-hover', product: 'equipment-stockpile-lines', params: { branchId: ctx.branch.id, family }, validate: validateStockpileLines },
      },
    ];
    return { owner: `${owner}:${JSON.stringify(asked)}`, entries };
  },
  projection: stockpileTimeProjection,
  projectionOptions(ctx, { extent, surface }) {
    return {
      chosen: chosenTypes(ctx.options),
      measure: ctx.options.measure === 'share' ? 'share' : 'stock',
      endLabels: ctx.options.endLabels !== false,
      yMin: numberOption(ctx.options.yMin),
      yMax: numberOption(ctx.options.yMax),
      log: ctx.options.log === true,
      extent,
      gaps: branchGapsOf(ctx.branch),
      surface,
    };
  },
};
