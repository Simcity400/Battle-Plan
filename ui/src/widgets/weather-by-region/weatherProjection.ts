import { extendLastRun, type LaneRuns } from '../../families/heat/laneRuns.ts';
import { identityColour, measureColour, surfaceGrey } from '../../charts/palette.ts';
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
import { noRegionsBody, regionLanes } from '../../families/regions/regionRows.ts';
import { formatDateOnly, formatDateParts, formatExactRational, formatRational } from '../../families/valueFormat.ts';
import type {
  RegionalWeatherColumn,
  RegionalWeatherColumnRegion,
  RegionalWeatherReply,
  WeatherReadout,
} from '../../generated/dashboard';
import { type WeatherShow } from './definition.ts';

export const WEATHER_WINDOW_DAYS = 7;
export const WEATHER_TOP = 7;
const WIDTH_STEP = 256;

export interface WeatherProjectionOptions {
  readonly show: WeatherShow;
  readonly modifier: string | null;
  readonly surface: string;
}

export function weatherShow(options: Readonly<Record<string, unknown>>): WeatherShow {
  return options.show === 'temperature' || options.show === 'modifiers' ? options.show : 'conditions';
}

const WEATHER_SCALE = 100_000n;

export function temperatureText(sum: bigint, days = 1n): string {
  return formatRational(sum, days * WEATHER_SCALE, { maxDecimals: 1, fixedDecimals: true, unit: '°C' });
}

export function modifierText(sum: bigint, days = 1n): string {
  return formatRational(sum * 100n, days * WEATHER_SCALE, { maxDecimals: 5, percent: true, signed: true });
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
    track: surfaceGrey(surface, 0.04),
    cold: measureColour('temperatureCold', surface),
    warm: measureColour('temperatureWarm', surface),
    ramp: measureColour('weatherModifier', surface),
  };
}

function measureOf(region: RegionalWeatherColumnRegion, id: string) {
  for (const measure of region.measures) if (measure.id === id) return measure;
  return undefined;
}

interface ColumnCell {
  readonly key: string;
  readonly cell: HeatLaneCell;
  readonly items?: FamilyTooltipPayload['items'];
  readonly missing?: string;
}

function conditionCell(reply: RegionalWeatherReply, region: RegionalWeatherColumnRegion, column: number, colours: Palette, inspect: boolean): ColumnCell {
  const observed = region.observed?.[column] ?? 0;
  const shown = reply.columns.columns[column]!;
  const span = Math.max(observed, Math.ceil((shown.lastHour - shown.firstHour + 1) / reply.dayHours));
  if (observed === 0) return { key: 'missing', cell: { kind: 'missing', reason: 'Region weather was not recorded' }, items: [], missing: 'Region weather was not recorded' };
  const clear = region.clear?.[column] ?? 0;
  let total = clear;
  let conditions = 0;
  let only: RegionalWeatherReply['flags'][number] | undefined;
  for (const flag of reply.flags) {
    const days = measureOf(region, flag.id)?.days[column] ?? 0;
    if (days > 0) {
      total += days;
      conditions++;
      only = flag;
    }
  }
  if (conditions === 0 && observed === span) return { key: 'clear', cell: { kind: 'clear', label: 'Clear' }, ...(inspect ? { items: [{ label: 'Clear' }] } : {}) };
  if (!inspect && conditions === 1 && clear === 0 && observed === span && only) {
    return { key: `solid:${only.id}`, cell: { kind: 'solid', color: colours.flag.get(only.id)!, label: only.name } };
  }
  const scale = total > 0 ? observed / Math.max(observed, total) / span : 0;
  const parts = [];
  const items: { label: string; value?: string; swatch?: string }[] | undefined = inspect ? [] : undefined;
  for (const flag of reply.flags) {
    const days = measureOf(region, flag.id)?.days[column] ?? 0;
    if (days <= 0) continue;
    const color = colours.flag.get(flag.id)!;
    parts.push({ id: flag.id, label: flag.name, color, share: days * scale });
    items?.push({ label: flag.name, swatch: color, ...(span > 1 ? { value: `${days} of ${span} days` } : {}) });
  }
  if (parts.length === 1 && clear === 0 && observed === span) {
    const part = parts[0]!;
    return { key: `solid:${part.id}`, cell: { kind: 'solid', color: part.color, label: part.label }, items };
  }
  if (clear > 0) {
    parts.push({ id: 'clear', label: 'Clear', color: colours.track, share: clear * scale });
    items?.push({ label: 'Clear', ...(span > 1 ? { value: `${clear} of ${span} days` } : {}) });
  }
  if (observed < span) items?.push({ label: 'Not recorded', value: `${span - observed} of ${span} days` });
  return { key: `parts:${parts.map(part => `${part.id}=${part.share}`).join(',')}`, cell: { kind: 'parts', parts, look: 'split' }, items };
}

function valueCell(
  reply: RegionalWeatherReply,
  region: RegionalWeatherColumnRegion,
  column: number,
  show: WeatherShow,
  modifier: string | null,
  colours: Palette,
  inspect: boolean,
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
  if (!inspect) return { key: `v:${color}`, cell: { kind: 'solid', color } };
  const text = (sum: bigint, days = 1n) => show === 'temperature'
    ? formatExactRational(sum, days * WEATHER_SCALE, { unit: '°C' })
    : formatExactRational(sum * 100n, days * WEATHER_SCALE, { percent: true, signed: true });
  const label = show === 'temperature' ? 'Temperature' : reply.modifiers.find(item => item.id === modifier)?.name ?? 'Modifier';
  const meanText = text(BigInt(stats[1]), BigInt(days));
  const value = days > 1 ? `${meanText} (${text(BigInt(stats[0]))} to ${text(BigInt(stats[2]))})` : meanText;
  return { key: `v:${color}`, cell: { kind: 'solid', color }, items: [{ label, value, swatch: color }] };
}

function readoutOf(readout: WeatherReadout | undefined, show: WeatherShow, modifier: string | null, reply: RegionalWeatherReply, colours: Palette): HeatLaneReadout | undefined {
  if (!readout) return undefined;
  if (readout.kind === 'notRecorded') return { kind: 'missing', reason: readout.reason };
  if (readout.kind === 'condition') {
    const since = readout.since_hour === null ? null : formatDateParts(readout.since_hour)?.dayMonth ?? null;
    const swatch = readout.id === 'clear' ? 'hollow' : colours.flag.get(readout.id);
    return {
      kind: 'value',
      form: 'word',
      primary: readout.name,
      ...(since ? { secondary: `Since ${since}` } : {}),
      ...(swatch ? { swatch } : {}),
      audit: since ? `${readout.name} since ${since}` : readout.name,
    };
  }
  const rawFixed = BigInt(readout.value);
  const raw = Number(readout.value);
  const text = show === 'temperature' ? temperatureText(rawFixed) : modifierText(rawFixed);
  const [low, high] = reply.extrema ? [Number(reply.extrema[0]), Number(reply.extrema[1])] : [raw, raw];
  const swatch = show === 'temperature'
    ? signedRampColour(raw, low, high, colours.track, colours.cold, colours.warm)
    : signedRampColour(raw, -Math.max(Math.abs(low), Math.abs(high)), Math.max(Math.abs(low), Math.abs(high)), colours.track, colours.ramp, colours.ramp);
  const secondary = show === 'temperature' ? 'Average' : reply.modifiers.find(item => item.id === modifier)?.name ?? 'Modifier';
  const audit = show === 'temperature'
    ? formatExactRational(rawFixed, WEATHER_SCALE, { unit: '°C' })
    : formatExactRational(rawFixed * 100n, WEATHER_SCALE, { percent: true, signed: true });
  return { kind: 'value', form: 'value', primary: text, secondary, swatch, audit };
}

function columnCell(
  reply: RegionalWeatherReply,
  region: RegionalWeatherColumnRegion,
  c: number,
  options: WeatherProjectionOptions,
  modifier: string | null,
  colours: Palette,
  inspect: boolean,
): ColumnCell {
  if (reply.columns.columns[c]!.recorded === 0) {
    const missing = 'Weather was not recorded on these days';
    return { key: 'unrecorded', cell: { kind: 'missing', reason: missing }, items: [], missing };
  }
  return options.show === 'conditions'
    ? conditionCell(reply, region, c, colours, inspect)
    : valueCell(reply, region, c, options.show, modifier, colours, inspect);
}

function sameColumnWindow(a: RegionalWeatherReply, b: RegionalWeatherReply): boolean {
  const before = a.columns.columns;
  const after = b.columns.columns;
  if (a.dayHours !== b.dayHours || before.length !== after.length || !after.length) return false;
  return before === after || after.every((col, i) => {
    const prev = before[i]!;
    if (col.firstHour !== prev.firstHour || (col.recorded === 0) !== (prev.recorded === 0)) return false;
    if (i !== after.length - 1) return col.lastHour === prev.lastHour && col.readHour === prev.readHour;
    return (col.readHour === prev.readHour || col.readHour !== null)
      && Math.floor((col.lastHour - col.firstHour) / b.dayHours) === Math.floor((prev.lastHour - prev.firstHour) / a.dayHours);
  });
}

function sameCounts(x: readonly number[] | undefined, y: readonly number[] | undefined): boolean {
  return x === y || (!!x && !!y && x.length === y.length && x.every((n, i) => n === y[i]));
}

function sameWeatherFacts(a: RegionalWeatherColumnRegion, b: RegionalWeatherColumnRegion, show: WeatherShow, modifier: string | null): boolean {
  if (a === b) return true;
  if (show === 'conditions' && (!sameCounts(a.observed, b.observed) || !sameCounts(a.clear, b.clear))) return false;
  if (a.measures === b.measures) return true;
  if (show === 'conditions') return a.measures.length === b.measures.length && a.measures.every((m, i) => {
    const next = b.measures[i]!;
    return m.id === next.id && sameCounts(m.days, next.days);
  });
  const id = show === 'temperature' ? 'temp' : `mod:${modifier ?? ''}`;
  const x = measureOf(a, id);
  const y = measureOf(b, id);
  return x === y || (!!x && !!y && sameCounts(x.days, y.days) && (x.values === y.values ||
    (!!x.values && !!y.values && x.values.length === y.values.length && x.values.every((v, i) => v?.[1] === y.values![i]?.[1]))));
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
    const lastHour = c === columns.length - 1 ? shownThrough : Math.min(column.lastHour, shownThrough);
    const made = columnCell(reply, region, c, options, modifier, colours, false);
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
  if (!column || hour > (c === columns.length - 1 ? reply.validThrough : column.lastHour)) return null;
  const made = columnCell(reply, region, c, options, modifier, colours, true);
  const dated = column.readHour === null ? { subtitle: formatDateOnly(column.firstHour) } : { hour: column.readHour };
  return made.missing ? { title, ...dated, missingReason: made.missing } : { title, ...dated, items: made.items };
}

function emptyModel(hour: number, extra: Partial<HeatLanesModel>): HeatLanesModel {
  return { hour, firstHour: hour, lastHour: hour, dayHours: 24, playhead: null, rows: [], ...extra };
}

interface WeatherLanes {
  readonly surface: string;
  readonly show: WeatherShow;
  readonly modifier: string | null;
  readonly colours: Palette;
  readonly names: ReadonlyMap<string, string | null>;
  readonly reply: RegionalWeatherReply;
  readonly rows: Map<string, { region: RegionalWeatherColumnRegion; through: number; runs: LaneRuns }>;
}

const heldLanes = new WeakMap<RegionalWeatherReply, WeatherLanes>();

const windows = new WeakMap<WeatherProjectionOptions, WeatherLanes>();

export function projectWeather(readView: BoundedReadView, options: WeatherProjectionOptions): HeatLanesModel {
  const hour: number = readView.hour;
  const previous = windows.get(options);
  windows.delete(options);
  const refused = readView.refused('weather');
  if (refused) return emptyModel(hour, { refused });
  const reply = readView.panel<RegionalWeatherReply>('weather');
  if (!reply || !options.surface || reply.lastHour > hour) return emptyModel(hour, {});
  const through = Math.min(hour, reply.validThrough);
  const modifier = options.show === 'modifiers' ? options.modifier ?? reply.modifiers[0]?.id ?? null : null;
  let held = heldLanes.get(reply);
  if (!held || held.surface !== options.surface || held.show !== options.show || held.modifier !== modifier) {
    heldLanes.set(reply, held = { reply, surface: options.surface, show: options.show, modifier, colours: palette(reply, options.surface),
      names: new Map(reply.regions.map(region => [region.id, region.name])), rows: new Map() });
  }
  windows.set(options, held);
  const prior = previous && previous !== held && previous.surface === options.surface && previous.show === options.show && previous.modifier === modifier
    && sameColumnWindow(previous.reply, reply) && (options.show === 'conditions'
      ? previous.reply.flags.length === reply.flags.length && reply.flags.every((flag, i) => {
        const before = previous.reply.flags[i]!;
        return flag.id === before.id && flag.name === before.name;
      })
      : previous.reply.temperature === reply.temperature && previous.reply.extrema?.[0] === reply.extrema?.[0] && previous.reply.extrema?.[1] === reply.extrema?.[1])
    ? previous : undefined;
  const { colours, names, rows: cached } = held;
  const last = reply.columns.columns.at(-1);
  const regions = reply.columns.regions;
  const rows: HeatLaneRow[] = regions.map(region => {
    const { label, nameMissing } = regionLabel(readView, region.id, names.get(region.id) ?? null);
    const readout = readoutOf(region.readout, options.show, modifier, reply, colours);
    let history = cached.get(region.id);
    if (!history || history.through !== through) {
      const before = history ?? prior?.rows.get(region.id);
      const canExtend = before && last && Math.min(through, before.through) >= last.firstHour
        && (history || sameWeatherFacts(before.region, region, options.show, modifier));
      const runs = canExtend ? extendLastRun(before.runs, through, last.readHour ?? before.runs.at(-1)?.readHour ?? null)
        : lane(reply, region, through, options, modifier, colours);
      cached.set(region.id, history = { region, through, runs });
    }
    return {
      id: region.id,
      label,
      ...(nameMissing ? { nameMissing } : {}),
      pinned: region.pinned,
      region: Number(region.id),
      runs: history.runs,
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
    regions: { ...regionLanes(null), active: reply.activeUnlisted },
    inspect: (rowIndex, at) => {
      const region = regions[rowIndex];
      return region ? inspectColumn(reply, region, rows[rowIndex]!.label, at, options, modifier, colours) : null;
    },
    ...(reply.empty ? noRegionsBody(reply.empty, { one: 'region with weather', many: 'regions with weather', none: 'No region has weather at this date.' })
      : rows.length ? {} : reply.rankMissing ? { empty: reply.rankMissing, emptyKind: 'unavailable' as const } : { empty: 'No weather recorded for these regions at this date.' }),
    warnings: [...reply.warnings, ...(reply.temperature === 'available' || options.show !== 'temperature' ? [] : [reply.temperature]), ...(rows.length && reply.rankMissing ? [reply.rankMissing] : [])],
  };
}

function nextWeatherChange(readView: BoundedReadView): number {
  const reply = readView.panel<RegionalWeatherReply>('weather');
  return reply && readView.hour < reply.validThrough ? readView.hour + 1 : Infinity;
}

export const weatherProjection: HeatLanesProjection<WeatherProjectionOptions> = {
  kind: 'weather-by-region',
  family: 'heat-lanes',
  project: projectWeather,
  nextChange: nextWeatherChange,
};

export const weatherDeclaration: HeatLanesWidgetDeclaration<WeatherProjectionOptions> = {
  family: 'heat-lanes',
  kind: 'weather-by-region',
  label: 'Weather by region',
  subject: 'weather',
  emptyCampaign: 'Load a campaign to see weather.',
  icon: 'cloud',
  countries: { mode: 'none' },
  sources: (ctx, view): SourcePlan => {
    const show = weatherShow(ctx.options);
    const modifier = show === 'modifiers' && ctx.options.modifier ? contentKey(String(ctx.options.modifier)) : null;
    const chosen = regionParams(readRegionChoice(ctx.options.regions));
    const width = Math.ceil(Math.max(1, view.width ?? WIDTH_STEP) / WIDTH_STEP) * WIDTH_STEP;
    const params = {
      branchId: ctx.branch.id,
      width,
      show: show === 'modifiers' ? 'modifier' : show,
      modifier,
      regions: null,
      ...chosen,
      top: WEATHER_TOP,
      windowDays: WEATHER_WINDOW_DAYS,
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
          channel: 'panel',
          coverage: { mode: 'population', form: 'top', limit: WEATHER_TOP },
          product: 'regional-weather',
          followQuery: { panel: 'regional-weather', ...params },
          params,
          dynamicKeys: ['lastHour'],
          buildExactParams: (base, hour) => ({ ...base, lastHour: hour }),
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
