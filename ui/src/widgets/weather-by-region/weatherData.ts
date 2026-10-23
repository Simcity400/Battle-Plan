import { runAtHour, validateKeyedWindow } from '../../dashboard/keyedHistory.ts';
import { blockSpan } from '../../dashboard/batch5Requests.ts';
import type { KeyedRun, KeyedWindow } from '../../dashboard/keyedHistory';
import type { KeyedDay } from '../../dashboard/keyedDays';
import type { HeatCell, HeatPeriod, HeatRow } from '../../charts/heat/layout';
import type { TooltipContent } from '../../charts/tooltip';

export interface Choice { id: string; name: string }
export interface WeatherTheatre extends Choice { regions: readonly string[] }
/** Whole days folded into columns: per region, each flag's recorded days set, or each
 * value's lowest raw value, exact sum and highest over its days with a value (`days`). */
export interface WeatherMeasure { id: string; days: readonly number[]; values?: readonly (readonly [string, string, string] | null)[] }
/** Conditions: the days a region's conditions were read (`observed`) and those with none set (`clear`). */
export interface WeatherColumnRegion { id: string; observed?: readonly number[]; clear?: readonly number[]; measures: readonly WeatherMeasure[] }
export interface WeatherColumn { firstHour: number; lastHour: number; sampledHour: number | null; readHour: number | null; recorded: number }
export interface WeatherColumns { days: number; columns: readonly WeatherColumn[]; regions: readonly WeatherColumnRegion[] }
export interface WeatherReply {
  window: KeyedWindow; dayHours: number; regions: readonly Choice[]; flags: readonly Choice[]; modifiers: readonly Choice[];
  theatres?: readonly WeatherTheatre[]; temperature: string; columns?: WeatherColumns; warnings: readonly string[];
}
/** A column shown, clipped to the extent; `column` indexes the reply's columns. */
export interface WeatherPeriod extends HeatPeriod { column: number }
export type WeatherShow = 'conditions' | 'temperature' | 'modifiers';

/** Temperatures are the game's fixed-point degrees Celsius. */
export const TEMPERATURE_SCALE = 100000;

/** Raw modifier entries are the game's fixed-point values. */
export const MODIFIER_SCALE = 100000;

export const DAY_HOURS = 24;
const dayOf = (hour: number) => Math.floor(hour / DAY_HOURS);

/** The hours to ask for and the most columns to fold them into, for `pixels` columns
 * over `extent`. A view whose days fit keeps the playback blocks holding it, one day
 * a column whatever the viewport; a longer one asks for its own whole days, folded
 * from its first. */
export function weatherWindow(branch: { firstHour: number; lastHour: number }, extent: readonly [number, number], pixels: number) {
  if (dayOf(extent[1]) - dayOf(extent[0]) + 1 <= pixels) {
    const [first, last] = blockSpan(branch, extent);
    return { first, last, width: dayOf(last) - dayOf(first) + 1 };
  }
  return { first: Math.max(branch.firstHour, dayOf(extent[0]) * DAY_HOURS), last: Math.min(branch.lastHour, (dayOf(extent[1]) + 1) * DAY_HOURS - 1), width: pixels };
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
  return folded.columns.every((column, index) => column && Number.isSafeInteger(column.firstHour) && Number.isSafeInteger(column.lastHour)
      && column.firstHour <= column.lastHour && (index === 0 || folded.columns[index - 1].lastHour < column.firstHour)
      && count(column.recorded) && (column.sampledHour === null) === (column.recorded === 0)
      && (column.sampledHour === null || Number.isSafeInteger(column.sampledHour) && column.firstHour <= column.sampledHour && column.sampledHour <= column.lastHour)
      && (column.readHour === null || column.sampledHour !== null && Number.isSafeInteger(column.readHour)))
    && folded.regions.every(region => region && typeof region.id === 'string' && (region.clear === undefined || perColumn(region.clear, count))
      && (region.observed === undefined || perColumn(region.observed, count)) && (region.clear === undefined) === (region.observed === undefined)
      && (region.clear ?? []).every((clear: number, column: number) => clear <= region.observed![column])
      && Array.isArray(region.measures) && region.measures.every((measure: WeatherMeasure) => measure && typeof measure.id === 'string' && perColumn(measure.days, count)
        && (measure.values === undefined || perColumn(measure.values, stats => stats === null || Array.isArray(stats) && stats.length === 3
          && stats.every(text => typeof text === 'string' && /^-?\d+$/.test(text)) && BigInt(stats[0]) <= BigInt(stats[2])))
        && (measure.values ?? []).every((stats, column) => (stats === null) === (measure.days[column] === 0))));
}

export function validateWeather(value: unknown): WeatherReply {
  const reply = value as WeatherReply;
  if (!reply || typeof reply !== 'object' || !choices(reply.regions) || !choices(reply.flags) || !choices(reply.modifiers)
    || !Number.isSafeInteger(reply.dayHours) || reply.dayHours <= 0 || typeof reply.temperature !== 'string'
    || reply.theatres !== undefined && !(Array.isArray(reply.theatres) && reply.theatres.every(theatre => isChoice(theatre)
      && Array.isArray((theatre as WeatherTheatre).regions) && (theatre as WeatherTheatre).regions.every(region => typeof region === 'string')))
    || reply.columns !== undefined && !validColumns(reply.columns)
    || !Array.isArray(reply.warnings) || !reply.warnings.every(warning => typeof warning === 'string'))
    throw new Error('The regional weather reply is malformed.');
  return { ...reply, window: validateKeyedWindow(reply.window) };
}

/** The hours of `extent` a reply's window does not reach. */
export function uncoveredSpans(extent: readonly [number, number], window: { firstHour: number; lastHour: number }): { firstHour: number; lastHour: number }[] {
  if (window.firstHour > window.lastHour || window.lastHour < extent[0] || window.firstHour > extent[1]) return [{ firstHour: extent[0], lastHour: extent[1] }];
  return [...extent[0] < window.firstHour ? [{ firstHour: extent[0], lastHour: window.firstHour - 1 }] : [],
    ...window.lastHour < extent[1] ? [{ firstHour: window.lastHour + 1, lastHour: extent[1] }] : []];
}

/** The hour a drill opens for the day holding `hour`: its last hour shown. */
export const pickedDay = (hour: number, dayHours: number, extent: readonly [number, number]) =>
  Math.min(Math.floor(hour / dayHours) * dayHours + dayHours - 1, extent[1]);

/** A column's calendar days. */
export const columnDays = (column: WeatherColumn, dayHours: number) => Math.floor(column.lastHour / dayHours) - Math.floor(column.firstHour / dayHours) + 1;

/** The reply's columns within `extent`, clipped to it. A column of several days keeps
 * the pointer's own hour, rather than snapping to its latest read. */
export function weatherPeriods(columns: WeatherColumns, extent: readonly [number, number], dayHours: number): WeatherPeriod[] {
  return columns.columns.flatMap((column, index) => column.lastHour < extent[0] || column.firstHour > extent[1] ? []
    // A day sampled without a recorded read is shown at the hour it was sampled.
    : [{ firstHour: Math.max(column.firstHour, extent[0]), lastHour: Math.min(column.lastHour, extent[1]), readHour: column.readHour ?? column.sampledHour,
      column: index, ...columnDays(column, dayHours) > 1 ? { exact: false } : {} }]);
}

/** Each region's runs by name part (`<flag>` or `mod:<modifier>`). */
export function regionSeries(window: KeyedWindow): Map<string, Map<string, readonly KeyedRun[]>> {
  const out = new Map<string, Map<string, readonly KeyedRun[]>>();
  for (const series of window.series) {
    const at = series.name.indexOf('|');
    if (at < 0) continue;
    const region = series.name.slice(0, at);
    const names = out.get(region) ?? new Map<string, readonly KeyedRun[]>();
    names.set(series.name.slice(at + 1), series.runs);
    out.set(region, names);
  }
  return out;
}

/** The regions of the chosen theatres and regions; null keeps every region. */
export function shownRegions(reply: WeatherReply, regions: readonly string[], theatres: readonly string[]): Choice[] {
  const inTheatres = theatres.length ? new Set((reply.theatres ?? []).filter(theatre => theatres.includes(theatre.id)).flatMap(theatre => theatre.regions)) : null;
  return reply.regions.filter(region => (!regions.length || regions.includes(region.id)) && (!inTheatres || inTheatres.has(region.id)));
}

type Measures = Map<string, { region: WeatherColumnRegion; measures: Map<string, WeatherMeasure> }>;
const regionMeasures = (reply: WeatherReply): Measures =>
  new Map((reply.columns?.regions ?? []).map(region => [region.id, { region, measures: new Map(region.measures.map(measure => [measure.id, measure])) }]));
/** A column's mean, lowest and highest raw value over its days holding one. */
function stats(measure: WeatherMeasure | undefined, column: number) {
  const values = measure?.values?.[column];
  return values ? { mean: Number(BigInt(values[1])) / measure!.days[column], low: values[0], high: values[2], days: measure!.days[column],
    zero: values[0] === '0' && values[2] === '0' } : null;
}

/** A heat cell for each column of each region. Conditions: every flag set on one of its
 * days, as stripes in the recording's flag order sized by days, beside the days with
 * none set and the days the region's conditions were not read (none set on any day is
 * blank). Temperature: the mean of the days' averages. Modifier: the chosen modifier's
 * mean on a ramp from grey, or hatched where no day witnessed it. A value missing on
 * some of a column's days marks the cell approximate. A one-day column shows that
 * day's value. */
export function weatherRows(reply: WeatherReply, regions: readonly Choice[], periods: readonly WeatherPeriod[], show: WeatherShow, modifier: string | null,
  colours: { flag: (index: number) => string; ramp: (value: number, highest: number) => string; temperature?: (value: number, low: number, high: number) => string;
    clear?: string; gap?: string }): { rows: HeatRow[]; highest: number } {
  const byRegion = regionMeasures(reply);
  const columns = reply.columns?.columns ?? [];
  const extremes = (id: string) => [...byRegion.values()].flatMap(({ measures }) => measures.get(id)?.values ?? [])
    .flatMap(values => values ? [[Number(values[0]), Number(values[2])] as const] : []);
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
        // A region with any province unknown has no average: its cell stays blank.
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
      // Days the region's conditions were not read are never clear.
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

/** A column's tooltip from the column alone. A one-day column reads as that day; a
 * folded one summarises its days: which were recorded, the days each condition was
 * set, and the mean with the lowest and highest values. */
export function columnTooltip(reply: WeatherReply, region: Choice, column: number, show: WeatherShow, modifier: string | null,
  date: string, label: (hour: number) => string): TooltipContent {
  const shown = reply.columns!.columns[column];
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
  const head = days === 1 ? [{ label: date }] : [{ label: `${label(shown.firstHour)} to ${label(shown.lastHour)}` },
    { label: 'Recorded days', value: `${shown.recorded} of ${days}` }];
  if (!shown.recorded) return { title: region.name, rows: [...head, { label: 'Not recorded' }] };
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

/** Every flag and modifier the region recorded at the day's hour, dated by its read.
 * Only the conditions view reads conditions, so only it lists them. */
export function weatherTooltip(reply: WeatherReply, region: Choice, day: KeyedDay | null, date: string, readDate: string | null,
  show: WeatherShow = 'conditions'): TooltipContent {
  if (!day || day.readHour === null) return { title: region.name, rows: [{ label: date }, { label: 'Not recorded' }] };
  const names = regionSeries(reply.window).get(region.id);
  const flags = reply.flags.filter(flag => { const run = runAtHour(names?.get(flag.id) ?? [], day.at); return run && run[2] !== '0'; });
  const modifiers = reply.modifiers.flatMap(modifier => {
    const run = runAtHour(names?.get(`mod:${modifier.id}`) ?? [], day.at);
    return run ? [{ label: modifier.name, lookupKey: modifier.id, value: modifierText(run[2]) }] : [];
  });
  return {
    title: region.name,
    rows: [
      { label: date },
      ...show === 'conditions' ? [{ label: 'Conditions', value: !names ? 'Not recorded' : flags.length ? flags.map(flag => flag.name).join(', ') : 'None' }] : [],
      { label: 'Temperature (average of provinces)', value: (() => { const run = runAtHour(names?.get('temp') ?? [], day.at); return run ? temperatureText(run[2]) : 'Not available'; })() },
      ...modifiers,
      ...readDate ? [{ label: 'As of', value: readDate }] : [],
    ],
  };
}

/** A modifier's fixed-point value as the game's signed percent. */
export function modifierText(raw: string): string {
  const value = Number(raw) / MODIFIER_SCALE * 100;
  return `${value > 0 ? '+' : value < 0 ? '−' : ''}${Math.abs(value).toLocaleString('en-US', { maximumFractionDigits: 1 })}%`;
}

/** A temperature's fixed-point value in degrees Celsius, to one decimal. */
export function temperatureText(raw: string): string {
  const value = Number(raw) / TEMPERATURE_SCALE;
  return `${value < 0 ? '−' : ''}${Math.abs(value).toLocaleString('en-US', { minimumFractionDigits: 1, maximumFractionDigits: 1 })} °C`;
}

/** One region's provinces at a reached hour; mud is whether the game shows it. */
export interface ProvinceWeather { id: string; name: string | null; snow: string | null; mud: boolean | null; temperature: string | null }
export interface ProvincesReply { view: 'provinces'; hour: number; readHour: number | null; region: string; provinces: readonly ProvinceWeather[]; warnings: readonly string[] }

const optional = (value: unknown) => value === null || typeof value === 'string';
export function validateProvinces(value: unknown): ProvincesReply {
  const reply = value as ProvincesReply;
  if (!reply || typeof reply !== 'object' || reply.view !== 'provinces' || !Number.isSafeInteger(reply.hour) || !(reply.readHour === null || Number.isSafeInteger(reply.readHour))
    || typeof reply.region !== 'string' || !Array.isArray(reply.warnings) || !Array.isArray(reply.provinces)
    || !reply.provinces.every(province => province && typeof province.id === 'string' && optional(province.name) && optional(province.snow)
      && (province.mud === null || typeof province.mud === 'boolean') && optional(province.temperature)))
    throw new Error('The province weather reply is malformed.');
  return reply;
}
