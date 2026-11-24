import type { HeatCell, HeatPeriod, HeatRow } from '../../charts/heat/layout';
import type { TooltipContent } from '../../charts/tooltip';
import type {
  RegionalWeatherChoice as Choice, RegionalWeatherProvincesReply as ProvincesReply, RegionalWeatherColumn as WeatherColumn, RegionalWeatherColumnRegion as WeatherColumnRegion, RegionalWeatherColumns as WeatherColumns, RegionalWeatherColumnMeasure as WeatherMeasure, RegionalWeatherReply as WeatherReply,
} from '../../generated/dashboard';

export type {
  RegionalWeatherChoice as Choice, RegionalWeatherProvince as ProvinceWeather, RegionalWeatherProvincesReply as ProvincesReply, RegionalWeatherReply as WeatherReply,
} from '../../generated/dashboard';
export interface WeatherPeriod extends HeatPeriod { column: number }
export type WeatherShow = 'conditions' | 'temperature' | 'modifiers';

export const TEMPERATURE_SCALE = 100000;

export const MODIFIER_SCALE = 100000;

export const DAY_HOURS = 24;
const BLOCK_HOURS = 90 * 24;
const dayOf = (hour: number) => Math.floor(hour / DAY_HOURS);

export function weatherWindow(branch: { firstHour: number; lastHour: number }, extent: readonly [number, number], pixels: number) {
  if (dayOf(extent[1]) - dayOf(extent[0]) + 1 <= pixels) {
    const first = Math.max(branch.firstHour, Math.floor(extent[0] / BLOCK_HOURS) * BLOCK_HOURS);
    const last = Math.min(branch.lastHour, Math.ceil((extent[1] + 1) / BLOCK_HOURS) * BLOCK_HOURS - 1);
    return { first, last, width: Math.max(1, dayOf(last) - dayOf(first) + 1) };
  }
  return { first: Math.max(branch.firstHour, dayOf(extent[0]) * DAY_HOURS), last: Math.min(branch.lastHour, (dayOf(extent[1]) + 1) * DAY_HOURS - 1), width: pixels };
}

export function weatherTailWindow(branch: { firstHour: number; lastHour: number }, extent: readonly [number, number], pixels: number, limit?: number) {
  if (limit == null) return null;
  const cap = Math.min(branch.lastHour, Math.floor(limit));
  if (cap < Math.max(branch.firstHour, extent[0]) || cap > extent[1]) return null;
  const base = weatherWindow(branch, extent, pixels);
  if (dayOf(extent[1]) - dayOf(extent[0]) + 1 <= pixels) {
    const dayFirst = Math.max(branch.firstHour, dayOf(cap) * DAY_HOURS);
    const dayLast = Math.min(branch.lastHour, (dayOf(cap) + 1) * DAY_HOURS - 1);
    return cap < dayLast ? { first: dayFirst, last: cap, width: 1 } : null;
  }
  const totalDays = Math.max(1, dayOf(base.last) - dayOf(base.first) + 1);
  const daysPerColumn = Math.max(1, Math.ceil(totalDays / Math.max(1, base.width)));
  const colIndex = Math.floor((dayOf(cap) - dayOf(base.first)) / daysPerColumn);
  const colFirst = Math.max(branch.firstHour, (dayOf(base.first) + colIndex * daysPerColumn) * DAY_HOURS);
  const colLast = Math.min(base.last, (dayOf(base.first) + (colIndex + 1) * daysPerColumn) * DAY_HOURS - 1);
  return cap < colLast ? { first: colFirst, last: cap, width: 1 } : null;
}

export function mergeWeatherTail(base: WeatherReply, tail: WeatherReply | undefined): WeatherReply {
  if (!tail || tail.columns.columns.length !== 1) return base;
  const tailCol = tail.columns.columns[0];
  const baseCols = base.columns.columns;
  const slot = baseCols.findIndex(col => col.firstHour <= tailCol.firstHour && tailCol.firstHour <= col.lastHour);
  if (slot < 0) return base;
  const columns = baseCols.map((col, index) => index === slot ? tailCol : col);
  const length = columns.length;
  const tailRegions = new Map(tail.columns.regions.map(region => [region.id, region]));
  const baseRegionIds = new Set(base.columns.regions.map(region => region.id));
  const mergeRegion = (region: WeatherColumnRegion, incoming: WeatherColumnRegion | undefined): WeatherColumnRegion => {
    const observed = region.observed ? region.observed.map((value, index) => index === slot ? incoming?.observed?.[0] ?? 0 : value) : undefined;
    const clear = region.clear ? region.clear.map((value, index) => index === slot ? incoming?.clear?.[0] ?? 0 : value) : undefined;
    const incomingMeasures = new Map((incoming?.measures ?? []).map(measure => [measure.id, measure]));
    const seen = new Set<string>();
    const measures: WeatherMeasure[] = region.measures.map(measure => {
      seen.add(measure.id);
      const next = incomingMeasures.get(measure.id);
      return {
        id: measure.id,
        days: measure.days.map((value, index) => index === slot ? next?.days[0] ?? 0 : value),
        ...(measure.values !== undefined ? { values: measure.values.map((value, index) => index === slot ? next?.values?.[0] ?? null : value) } : {}),
      };
    });
    for (const [id, next] of incomingMeasures) {
      if (seen.has(id)) continue;
      const days = new Array<number>(length).fill(0);
      days[slot] = next.days[0];
      const values = next.values !== undefined ? new Array<([string, string, string] | null)>(length).fill(null) : undefined;
      if (values) values[slot] = next.values![0];
      measures.push({ id, days, ...(values !== undefined ? { values } : {}) });
    }
    return { id: region.id, ...(observed !== undefined ? { observed } : {}), ...(clear !== undefined ? { clear } : {}), measures };
  };
  const regions: WeatherColumnRegion[] = base.columns.regions.map(region => mergeRegion(region, tailRegions.get(region.id)));
  for (const incoming of tail.columns.regions) {
    if (baseRegionIds.has(incoming.id)) continue;
    const empty: WeatherColumnRegion = {
      id: incoming.id,
      ...(incoming.observed !== undefined ? { observed: new Array<number>(length).fill(0) } : {}),
      ...(incoming.clear !== undefined ? { clear: new Array<number>(length).fill(0) } : {}),
      measures: [],
    };
    regions.push(mergeRegion(empty, incoming));
  }
  const regionList = base.regions.length >= tail.regions.length ? base.regions : tail.regions;
  const warnings = [...new Set([...base.warnings, ...tail.warnings])];
  return { ...base, lastHour: Math.max(base.lastHour, tail.lastHour), regions: regionList, columns: { ...base.columns, columns, regions }, warnings };
}

const isChoice = (value: unknown): value is Choice => !!value && typeof value === 'object'
  && typeof (value as Choice).id === 'string' && typeof (value as Choice).name === 'string';
const choices = (value: unknown) => Array.isArray(value) && value.every(isChoice);
const count = (value: unknown) => Number.isSafeInteger(value) && (value as number) >= 0;

function validColumns(value: unknown): boolean {
  const folded = value as WeatherColumns;
  if (!folded || typeof folded !== 'object' || !count(folded.days) || folded.days < 1 || !Array.isArray(folded.columns) || !Array.isArray(folded.regions)) return false;
  const length = folded.columns.length;
  const perColumn = (items: unknown, item: (value: unknown) => boolean) => Array.isArray(items) && items.length === length && items.every(item);
  return folded.columns.every((column: WeatherColumn, index) => column && Number.isSafeInteger(column.firstHour) && Number.isSafeInteger(column.lastHour)
      && column.firstHour <= column.lastHour && (index === 0 || folded.columns[index - 1].lastHour < column.firstHour)
      && count(column.recorded) && (column.sampledHour === null) === (column.recorded === 0)
      && (column.sampledHour === null || Number.isSafeInteger(column.sampledHour) && column.firstHour <= column.sampledHour && column.sampledHour <= column.lastHour)
      && (column.readHour === null || column.sampledHour !== null && Number.isSafeInteger(column.readHour) && column.readHour <= column.sampledHour))
    && folded.regions.every((region: WeatherColumnRegion) => region && typeof region.id === 'string' && (region.clear === undefined || perColumn(region.clear, count))
      && (region.observed === undefined || perColumn(region.observed, count)) && (region.clear === undefined) === (region.observed === undefined)
      && (region.clear ?? []).every((clear: number, column: number) => clear <= region.observed![column])
      && Array.isArray(region.measures) && region.measures.every((measure: WeatherMeasure) => measure && typeof measure.id === 'string' && perColumn(measure.days, count)
        && (measure.values === undefined || perColumn(measure.values, stats => stats === null || Array.isArray(stats) && stats.length === 3
          && stats.every(text => typeof text === 'string' && /^-?\d+$/.test(text)) && BigInt(stats[0]) <= BigInt(stats[2])))
        && (measure.values ?? []).every((stats, column) => (stats === null) === (measure.days[column] === 0))));
}

export function validateWeather(value: unknown): WeatherReply {
  const reply = value as WeatherReply;
  if (!reply || typeof reply !== 'object' || !Number.isSafeInteger(reply.firstHour) || !Number.isSafeInteger(reply.lastHour)
    || !choices(reply.regions) || !choices(reply.flags) || !choices(reply.modifiers)
    || !Number.isSafeInteger(reply.dayHours) || reply.dayHours <= 0 || typeof reply.temperature !== 'string' || !validColumns(reply.columns)
    || !Array.isArray(reply.warnings) || !reply.warnings.every(warning => typeof warning === 'string'))
    throw new Error('The regional weather reply is malformed.');
  return reply;
}

export function uncoveredSpans(extent: readonly [number, number], reply: { firstHour: number; lastHour: number }): { firstHour: number; lastHour: number }[] {
  if (reply.firstHour > reply.lastHour || reply.lastHour < extent[0] || reply.firstHour > extent[1]) return [{ firstHour: extent[0], lastHour: extent[1] }];
  return [...extent[0] < reply.firstHour ? [{ firstHour: extent[0], lastHour: reply.firstHour - 1 }] : [],
    ...reply.lastHour < extent[1] ? [{ firstHour: reply.lastHour + 1, lastHour: extent[1] }] : []];
}

export const pickedDay = (hour: number, dayHours: number, extent: readonly [number, number]) =>
  Math.min(Math.floor(hour / dayHours) * dayHours + dayHours - 1, extent[1]);

export const columnDays = (column: WeatherColumn, dayHours: number) => Math.floor(column.lastHour / dayHours) - Math.floor(column.firstHour / dayHours) + 1;

export function weatherPeriods(columns: WeatherColumns, extent: readonly [number, number], dayHours: number): WeatherPeriod[] {
  return columns.columns.flatMap((column, index) => column.lastHour < extent[0] || column.firstHour > extent[1] ? []
    : [{ firstHour: Math.max(column.firstHour, extent[0]), lastHour: Math.min(column.lastHour, extent[1]), readHour: column.readHour ?? column.sampledHour,
      column: index, ...columnDays(column, dayHours) > 1 ? { exact: false } : {} }]);
}

export function shownRegions(reply: WeatherReply, regions: readonly string[], inTheatres: ReadonlySet<string> | null = null): Choice[] {
  return reply.regions.filter(region => (!regions.length || regions.includes(region.id)) && (!inTheatres || inTheatres.has(region.id)));
}

type Measures = Map<string, { region: WeatherColumnRegion; measures: Map<string, WeatherMeasure> }>;
const regionMeasures = (reply: WeatherReply): Measures =>
  new Map((reply.columns.regions).map(region => [region.id, { region, measures: new Map(region.measures.map(measure => [measure.id, measure])) }]));
function stats(measure: WeatherMeasure | undefined, column: number) {
  const values = measure?.values?.[column];
  return values ? { mean: Number(BigInt(values[1])) / measure!.days[column], low: values[0], high: values[2], days: measure!.days[column],
    zero: values[0] === '0' && values[2] === '0' } : null;
}

export function weatherRows(reply: WeatherReply, regions: readonly Choice[], periods: readonly WeatherPeriod[], show: WeatherShow, modifier: string | null,
  colours: { flag: (index: number) => string; ramp: (value: number, highest: number) => string; temperature?: (value: number, low: number, high: number) => string;
    clear?: string; gap?: string }): { rows: HeatRow[]; highest: number } {
  const byRegion = regionMeasures(reply);
  const columns = reply.columns.columns;
  const shownColumns = new Set(periods.map(period => period.column));
  const extremes = (id: string) => [...byRegion.values()].flatMap(({ measures }) =>
    (measures.get(id)?.values ?? []).flatMap((values, column) => values && shownColumns.has(column) ? [[Number(values[0]), Number(values[2])] as const] : []));
  let highest = 0;
  if (show === 'modifiers' && modifier !== null)
    for (const [min, max] of extremes(`mod:${modifier}`)) highest = Math.max(highest, Math.abs(min) / MODIFIER_SCALE, Math.abs(max) / MODIFIER_SCALE);
  let low = 0, high = 0;
  if (show === 'temperature')
    for (const [min, max] of extremes('temp')) { low = Math.min(low, min / TEMPERATURE_SCALE); high = Math.max(high, max / TEMPERATURE_SCALE); }
  const blank: HeatCell = { parts: [] }, unknown: HeatCell = { parts: [], unknown: true };
  const rows = regions.map(region => {
    const entry = byRegion.get(region.id);
    const cells = periods.map(({ column, readHour }): HeatCell => {
      if (readHour === null) return blank;
      const days = columnDays(columns[column], reply.dayHours);
      if (show === 'temperature') {
        const value = stats(entry?.measures.get('temp'), column);
        if (!value) return blank;
        const degrees = value.mean / TEMPERATURE_SCALE;
        return { parts: [{ id: 'temp', label: temperatureText(String(value.mean)),
          color: colours.temperature?.(degrees, low, high) ?? colours.ramp(Math.abs(degrees), Math.max(-low, high)), share: 1 }],
          ...value.days < days ? { approximate: true } : {} };
      }
      if (show === 'modifiers') {
        const value = modifier === null ? null : stats(entry?.measures.get(`mod:${modifier}`), column);
        if (!value) return unknown;
        const raw = value.mean / MODIFIER_SCALE;
        const partial = value.days < days ? { approximate: true } : {};
        return value.zero ? { parts: [], ...partial } : { parts: [{ id: modifier!, label: String(raw), color: colours.ramp(Math.abs(raw), highest), share: 1 }], ...partial };
      }
      const observed = entry?.region.observed?.[column] ?? 0;
      if (!observed) return unknown;
      const set = reply.flags.flatMap((flag, index) => {
        const flagged = entry!.measures.get(flag.id)?.days[column] ?? 0;
        return flagged > 0 ? [{ index, flag, flagged }] : [];
      });
      const unread = days - observed;
      if (!set.length && !unread) return blank;
      const clear = entry!.region.clear?.[column] ?? 0;
      const total = set.reduce((sum, item) => sum + item.flagged, clear + unread);
      return { parts: [...set.map(({ index, flag, flagged }) => ({ id: flag.id, label: flag.name, color: colours.flag(index), share: flagged / total })),
        ...clear ? [{ id: 'clear', label: 'No condition', color: colours.clear ?? 'transparent', share: clear / total }] : [],
        ...unread ? [{ id: 'unread', label: 'Not recorded', color: colours.gap ?? 'transparent', share: unread / total }] : []] };
    });
    return { id: region.id, label: region.name, cells };
  });
  return { rows, highest };
}

export function columnTooltip(reply: WeatherReply, region: Choice, column: number, show: WeatherShow, modifier: string | null,
  date: string, label: (hour: number) => string): TooltipContent {
  const shown = reply.columns.columns[column];
  const entry = regionMeasures(reply).get(region.id);
  const days = columnDays(shown, reply.dayHours);
  const range = (value: NonNullable<ReturnType<typeof stats>>, text: (raw: string) => string) =>
    days === 1 ? text(String(value.mean)) : `${text(String(value.mean))} (${text(value.low)} to ${text(value.high)})`;
  const plural = (count: number) => `${count} ${count === 1 ? 'day' : 'days'}`;
  const conditions = (() => {
    const observed = entry?.region.observed?.[column];
    if (!observed) return [{ label: 'Conditions', value: 'Not recorded' }];
    const set = reply.flags.flatMap(flag => { const flagged = entry!.measures.get(flag.id)?.days[column] ?? 0; return flagged ? [{ flag, flagged }] : []; });
    if (days === 1) return [{ label: 'Conditions', value: set.length ? set.map(({ flag }) => flag.name).join(', ') : 'None' }];
    const clear = entry!.region.clear?.[column] ?? 0;
    return [...set.map(({ flag, flagged }) => ({ label: flag.name, value: plural(flagged) })),
      ...clear ? [{ label: 'No condition', value: plural(clear) }] : [],
      ...observed < days ? [{ label: 'Conditions not recorded', value: plural(days - observed) }] : []];
  })();
  const temperature = stats(entry?.measures.get('temp'), column);
  const modifiers = reply.modifiers.flatMap(item => {
    const value = stats(entry?.measures.get(`mod:${item.id}`), column);
    return value && (show !== 'modifiers' || item.id === modifier) ? [{ label: item.name, lookupKey: item.id, value: range(value, modifierText) }] : [];
  });
  const recorded = show === 'conditions' ? entry?.region.observed?.[column] ?? 0
    : stats(entry?.measures.get(show === 'temperature' ? 'temp' : `mod:${modifier}`), column)?.days ?? 0;
  const head = days === 1 ? [{ label: date }] : [{ label: `${label(shown.firstHour)} to ${label(shown.lastHour)}` },
    { label: 'Recorded days', value: `${recorded} of ${days}` }];
  if (!recorded) return { title: region.name, rows: [...head, { label: 'Not recorded' }] };
  return {
    title: region.name,
    rows: [
      ...head,
      ...show === 'conditions' ? conditions : [],
      ...show === 'temperature' || days === 1 ? [{ label: 'Temperature (average of provinces)', value: temperature ? range(temperature, temperatureText) : 'Not available' }] : [],
      ...modifiers,
      ...shown.readHour !== null ? [{ label: 'As of', value: label(shown.readHour) }] : [],
    ],
  };
}

export function modifierText(raw: string): string {
  const value = Number(raw) / MODIFIER_SCALE * 100;
  return `${value > 0 ? '+' : value < 0 ? '−' : ''}${Math.abs(value).toLocaleString('en-US', { maximumFractionDigits: 1 })}%`;
}

export function temperatureText(raw: string): string {
  const value = Number(raw) / TEMPERATURE_SCALE;
  return `${value < 0 ? '−' : ''}${Math.abs(value).toLocaleString('en-US', { minimumFractionDigits: 1, maximumFractionDigits: 1 })} °C`;
}


const optional = (value: unknown) => value === null || typeof value === 'string';
export function validateProvinces(value: unknown): ProvincesReply {
  const reply = value as ProvincesReply;
  if (!reply || typeof reply !== 'object' || !(reply.readHour === null || Number.isSafeInteger(reply.readHour))
    || typeof reply.region !== 'string' || !Array.isArray(reply.warnings) || !Array.isArray(reply.provinces)
    || !reply.provinces.every(province => province && typeof province.id === 'string' && optional(province.name) && optional(province.snow)
      && (province.mud === null || typeof province.mud === 'boolean') && optional(province.temperature)))
    throw new Error('The province weather reply is malformed.');
  return reply;
}
