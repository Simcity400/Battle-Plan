import { CHART_MEASURE_COLOURS, identityColour, readableColour } from '../../charts/palette.ts';
import { contentKey } from '../../dashboard/options.ts';
import type { BoundedReadView, SourcePlan } from '../../families/contract.ts';
import type { HeatLanesWidgetDeclaration } from '../../families/heat/lanesDeclaration.ts';
import {
  spanIndexAt,
  type HeatLaneCell,
  type HeatLaneReadout,
  type HeatLaneRow,
  type HeatLaneRun,
  type HeatLanesModel,
  type HeatLanesProjection,
} from '../../families/heat/lanesModel.ts';
import { LANE_READOUT_MAX_WIDTH } from '../../families/heat/lanesLayout.ts';
import { regionLabel, signedRampColour } from '../../families/heat/scale.ts';
import type { FamilyTooltipPayload } from '../../families/host/tooltips.ts';
import { readRegionChoice, regionParams } from '../../families/regions/regionChoice.ts';
import { regionLanes } from '../../families/regions/regionRows.ts';
import { formatDateOnly, formatDateParts, formatScalar } from '../../families/valueFormat.ts';
import type {
  RegionalWeatherColumn,
  RegionalWeatherColumnRegion,
  RegionalWeatherReply,
  WeatherReadout,
} from '../../generated/dashboard';
import { surfaceMix } from '../../shell/appearance/color.ts';
import { type WeatherShow } from './definition.ts';

export const WEATHER_WINDOW_HOURS = 7 * 24;
export const WEATHER_TOP = 7;
const FIXED = 100_000;
const WIDTH_STEP = 256;

export interface WeatherProjectionOptions {
  readonly show: WeatherShow;
  readonly modifier: string | null;
  readonly surface: string;
}

export function weatherShow(options: Readonly<Record<string, unknown>>): WeatherShow {
  return options.show === 'temperature' || options.show === 'modifiers' ? options.show : 'conditions';
}

export function temperatureText(raw: number): string {
  return formatScalar(raw / FIXED, { decimals: 0, maxDecimals: 1, fixedDecimals: true, unit: '°C' });
}

export function modifierText(raw: number): string {
  return formatScalar((raw / FIXED) * 100, { decimals: 0, maxDecimals: 1, percent: true, signed: true });
}

interface Palette {
  readonly flag: ReadonlyMap<string, string>;
  readonly track: string;
  readonly cold: string;
  readonly warm: string;
  readonly ramp: string;
}

function palette(reply: RegionalWeatherReply, surface: string): Palette {
  const flag = new Map<string, string>();
  reply.flags.forEach((item, ordinal) => flag.set(item.id, identityColour(surface, 'weather', item.id, ordinal)));
  return {
    flag,
    track: surfaceMix(surface, 0.04),
    cold: readableColour(CHART_MEASURE_COLOURS.temperatureCold, surface),
    warm: readableColour(CHART_MEASURE_COLOURS.temperatureWarm, surface),
    ramp: readableColour(CHART_MEASURE_COLOURS.weatherModifier, surface),
  };
}

function measureOf(region: RegionalWeatherColumnRegion, id: string) {
  for (const measure of region.measures) if (measure.id === id) return measure;
  return undefined;
}

interface ColumnCell {
  readonly key: string;
  readonly cell: HeatLaneCell;
  readonly items: FamilyTooltipPayload['items'];
  readonly missing?: string;
}

function conditionCell(reply: RegionalWeatherReply, region: RegionalWeatherColumnRegion, column: number, colours: Palette): ColumnCell {
  const observed = region.observed?.[column] ?? 0;
  const shown = reply.columns.columns[column]!;
  const span = Math.max(observed, Math.ceil((shown.lastHour - shown.firstHour + 1) / reply.dayHours));
  if (observed === 0) return { key: 'missing', cell: { kind: 'missing', reason: 'Region weather was not recorded' }, items: [], missing: 'Region weather was not recorded' };
  const clear = region.clear?.[column] ?? 0;
  const counted = reply.flags.map(flag => [flag, measureOf(region, flag.id)?.days[column] ?? 0] as const).filter(([, days]) => days > 0);
  const total = counted.reduce((sum, [, days]) => sum + days, clear);
  const scale = total > 0 ? observed / Math.max(observed, total) / span : 0;
  const parts = [];
  const items = [];
  for (const [flag, days] of counted) {
    const color = colours.flag.get(flag.id)!;
    parts.push({ id: flag.id, label: flag.name, color, share: days * scale });
    items.push({ label: flag.name, swatch: color, ...(span > 1 ? { value: `${days} of ${span} days` } : {}) });
  }
  if (parts.length === 0 && observed === span) return { key: 'clear', cell: { kind: 'clear', label: 'Clear' }, items: [{ label: 'Clear' }] };
  if (parts.length === 1 && clear === 0 && observed === span) {
    const part = parts[0]!;
    return { key: `solid:${part.id}`, cell: { kind: 'solid', color: part.color, label: part.label }, items };
  }
  if (clear > 0) {
    parts.push({ id: 'clear', label: 'Clear', color: colours.track, share: clear * scale });
    items.push({ label: 'Clear', ...(span > 1 ? { value: `${clear} of ${span} days` } : {}) });
  }
  if (observed < span) items.push({ label: 'Not recorded', value: `${span - observed} of ${span} days` });
  return { key: `parts:${parts.map(part => `${part.id}=${part.share}`).join(',')}`, cell: { kind: 'parts', parts, look: 'split' }, items };
}

function valueCell(
  reply: RegionalWeatherReply,
  region: RegionalWeatherColumnRegion,
  column: number,
  show: WeatherShow,
  modifier: string | null,
  colours: Palette,
): ColumnCell {
  const id = show === 'temperature' ? 'temp' : `mod:${modifier ?? ''}`;
  const stats = measureOf(region, id)?.values?.[column] ?? null;
  const days = measureOf(region, id)?.days[column] ?? 0;
  if (!stats || days <= 0) {
    const reason = show === 'temperature' && reply.temperature !== 'available' ? reply.temperature : 'Weather reading was not recorded';
    return { key: 'missing', cell: { kind: 'missing', reason }, items: [], missing: reason };
  }
  const mean = Number(stats[1]) / days;
  const [low, high] = reply.extrema ? [Number(reply.extrema[0]), Number(reply.extrema[1])] : [mean, mean];
  const color = show === 'temperature'
    ? signedRampColour(mean, low, high, colours.track, colours.cold, colours.warm)
    : signedRampColour(mean, -Math.max(Math.abs(low), Math.abs(high)), Math.max(Math.abs(low), Math.abs(high)), colours.track, colours.ramp, colours.ramp);
  const text = show === 'temperature' ? temperatureText : modifierText;
  const label = show === 'temperature' ? 'Temperature' : reply.modifiers.find(item => item.id === modifier)?.name ?? 'Modifier';
  const value = days > 1 ? `${text(mean)} (${text(Number(stats[0]))} to ${text(Number(stats[2]))})` : text(mean);
  return { key: `v:${color}`, cell: { kind: 'solid', color }, items: [{ label, value, swatch: color }] };
}

function readoutOf(readout: WeatherReadout | undefined, show: WeatherShow, modifier: string | null, reply: RegionalWeatherReply, colours: Palette): HeatLaneReadout | undefined {
  if (!readout) return undefined;
  if (readout.kind === 'notRecorded') return { kind: 'missing', reason: readout.reason };
  if (readout.kind === 'condition') {
    const since = readout.since_hour === null ? null : formatDateParts(readout.since_hour)?.dayMonth ?? null;
    const sinceWord = readout.since_bounded ? 'Observed since' : 'Since';
    const swatch = readout.id === 'clear' ? 'hollow' : colours.flag.get(readout.id);
    return {
      kind: 'value',
      form: 'word',
      primary: readout.name,
      ...(since ? { secondary: `${sinceWord} ${since}` } : {}),
      ...(swatch ? { swatch } : {}),
      audit: since ? `${readout.name} ${sinceWord.toLowerCase()} ${since}` : readout.name,
    };
  }
  const raw = Number(readout.value);
  const text = show === 'temperature' ? temperatureText(raw) : modifierText(raw);
  const [low, high] = reply.extrema ? [Number(reply.extrema[0]), Number(reply.extrema[1])] : [raw, raw];
  const swatch = show === 'temperature'
    ? signedRampColour(raw, low, high, colours.track, colours.cold, colours.warm)
    : signedRampColour(raw, -Math.max(Math.abs(low), Math.abs(high)), Math.max(Math.abs(low), Math.abs(high)), colours.track, colours.ramp, colours.ramp);
  const secondary = show === 'temperature' ? 'Average' : reply.modifiers.find(item => item.id === modifier)?.name ?? 'Modifier';
  return { kind: 'value', form: 'value', primary: text, secondary, swatch };
}

function columnCell(
  reply: RegionalWeatherReply,
  region: RegionalWeatherColumnRegion,
  c: number,
  options: WeatherProjectionOptions,
  modifier: string | null,
  colours: Palette,
): ColumnCell {
  if (reply.columns.columns[c]!.recorded === 0) {
    const missing = 'Weather was not recorded on these days';
    return { key: 'unrecorded', cell: { kind: 'missing', reason: missing }, items: [], missing };
  }
  return options.show === 'conditions'
    ? conditionCell(reply, region, c, colours)
    : valueCell(reply, region, c, options.show, modifier, colours);
}

function lane(
  reply: RegionalWeatherReply,
  region: RegionalWeatherColumnRegion,
  shownThrough: number,
  options: WeatherProjectionOptions,
  modifier: string | null,
  colours: Palette,
): HeatLaneRun[] {
  const runs: HeatLaneRun[] = [];
  let key = '';
  const columns: readonly RegionalWeatherColumn[] = reply.columns.columns;
  for (let c = 0; c < columns.length; c++) {
    const column = columns[c]!;
    if (column.firstHour > shownThrough) break;
    const lastHour = Math.min(column.lastHour, shownThrough);
    const made = columnCell(reply, region, c, options, modifier, colours);
    const previous = runs[runs.length - 1];
    if (previous && made.key === key && previous.lastHour + 1 >= column.firstHour) {
      runs[runs.length - 1] = { ...previous, lastHour, readHour: column.readHour ?? previous.readHour };
      continue;
    }
    key = made.key;
    runs.push({ firstHour: column.firstHour, lastHour, readHour: column.readHour, cell: made.cell });
  }
  return runs;
}

function inspectColumn(
  reply: RegionalWeatherReply,
  region: RegionalWeatherColumnRegion,
  title: string,
  hour: number,
  options: WeatherProjectionOptions,
  modifier: string | null,
  colours: Palette,
): FamilyTooltipPayload | null {
  const columns = reply.columns.columns;
  const c = spanIndexAt(columns, hour, column => column.firstHour);
  const column = columns[c];
  if (!column || hour > column.lastHour) return null;
  const made = columnCell(reply, region, c, options, modifier, colours);
  const dated = column.readHour === null ? { subtitle: formatDateOnly(column.firstHour) } : { hour: column.readHour };
  return made.missing ? { title, ...dated, missingReason: made.missing } : { title, ...dated, items: made.items };
}

function emptyModel(hour: number, extra: Partial<HeatLanesModel>): HeatLanesModel {
  return { hour, firstHour: hour, lastHour: hour, dayHours: 24, playhead: null, rows: [], ...extra };
}

export function projectWeather(readView: BoundedReadView, options: WeatherProjectionOptions): HeatLanesModel {
  const hour: number = readView.hour;
  const refused = readView.refused('weather');
  if (refused) return emptyModel(hour, { refused });
  const reply = readView.panel<RegionalWeatherReply>('weather');
  if (!reply || !options.surface || reply.lastHour > hour) return emptyModel(hour, {});
  const through = reply.lastHour;
  const colours = palette(reply, options.surface);
  const modifier = options.show === 'modifiers' ? options.modifier ?? reply.modifiers[0]?.id ?? null : null;
  const names = new Map(reply.regions.map(region => [region.id, region.name]));
  const regions = reply.columns.regions;
  const rows: HeatLaneRow[] = regions.map(region => {
    const { label, nameMissing } = regionLabel(readView, region.id, names.get(region.id) ?? null);
    const readout = readoutOf(region.readout, options.show, modifier, reply, colours);
    return {
      id: region.id,
      label,
      ...(nameMissing ? { nameMissing } : {}),
      pinned: region.pinned,
      region: Number(region.id),
      runs: lane(reply, region, through, options, modifier, colours),
      ...(readout ? { readout } : {}),
    };
  });
  return {
    hour: through,
    firstHour: reply.firstHour,
    lastHour: through,
    dayHours: reply.dayHours,
    playhead: through,
    gaps: reply.gaps.map(gap => ({ firstHour: gap.firstHour, lastHour: gap.lastHour, reason: 'The recording holds no hour here' })),
    rows,
    ...(options.show === 'conditions' ? { readoutWidth: LANE_READOUT_MAX_WIDTH } : {}),
    regions: regionLanes(null, 'Most active this week'),
    inspect: (rowIndex, at) => {
      const region = regions[rowIndex];
      return region ? inspectColumn(reply, region, rows[rowIndex]!.label, at, options, modifier, colours) : null;
    },
    ...(rows.length ? {} : { empty: 'No weather recorded for these regions at this date.' }),
    warnings: reply.temperature === 'available' || options.show !== 'temperature' ? reply.warnings : [...reply.warnings, reply.temperature],
  };
}

export const weatherProjection: HeatLanesProjection<WeatherProjectionOptions> = {
  kind: 'weather-by-region',
  family: 'heat-lanes',
  project: projectWeather,
};

export const weatherDeclaration: HeatLanesWidgetDeclaration<WeatherProjectionOptions> = {
  family: 'heat-lanes',
  kind: 'weather-by-region',
  label: 'Weather by region',
  subject: 'weather',
  emptyCampaign: 'Load a campaign to see weather.',
  icon: 'sun',
  countries: { mode: 'none' },
  sources: (ctx, view): SourcePlan => {
    const show = weatherShow(ctx.options);
    const modifier = show === 'modifiers' && ctx.options.modifier ? contentKey(String(ctx.options.modifier)) : null;
    const chosen = regionParams(readRegionChoice(ctx.options.regions));
    const firstHour = ctx.branch.firstHour;
    const width = Math.ceil(Math.max(1, view.width ?? WIDTH_STEP) / WIDTH_STEP) * WIDTH_STEP;
    const params = {
      branchId: ctx.branch.id,
      width,
      show: show === 'modifiers' ? 'modifier' : show,
      modifier,
      regions: null,
      ...chosen,
      top: WEATHER_TOP,
    };
    return {
      owner: `weather-by-region:${ctx.branch.id}:${show}:${modifier ?? ''}:${JSON.stringify(chosen)}:${width}`,
      entries: [{
        name: 'weather',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'point',
          channel: 'followed',
          coverage: { mode: 'population', form: 'top', limit: WEATHER_TOP },
          product: 'regional-weather',
          followQuery: null,
          params,
          dynamicKeys: ['firstHour', 'lastHour'],
          buildExactParams: (base, hour) => ({ ...base, firstHour: Math.max(firstHour, hour - WEATHER_WINDOW_HOURS + 1), lastHour: hour }),
        },
      }],
    };
  },
  projection: weatherProjection,
  projectionOptions: (ctx, extra) => ({
    show: weatherShow(ctx.options),
    modifier: weatherShow(ctx.options) === 'modifiers' && ctx.options.modifier ? contentKey(String(ctx.options.modifier)) : null,
    surface: extra.surface,
  }),
  jumpToDate: true,
};
