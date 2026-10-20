import { runAtHour, validateKeyedWindow } from '../../dashboard/keyedHistory.ts';
import type { KeyedRun, KeyedWindow } from '../../dashboard/keyedHistory';
import type { KeyedDay } from '../../dashboard/keyedDays';
import type { HeatCell, HeatRow } from '../../charts/heat/layout';
import type { TooltipContent } from '../../charts/tooltip';

export interface Choice { id: string; name: string }
export interface WeatherTheatre extends Choice { regions: readonly string[] }
export interface WeatherReply {
  window: KeyedWindow; dayHours: number; regions: readonly Choice[]; flags: readonly Choice[]; modifiers: readonly Choice[];
  theatres?: readonly WeatherTheatre[]; temperature: string; warnings: readonly string[];
}
export type WeatherShow = 'conditions' | 'temperature' | 'modifiers';

/** Temperatures are the game's fixed-point degrees Celsius. */
export const TEMPERATURE_SCALE = 100000;

/** Raw modifier entries are the game's fixed-point values. */
export const MODIFIER_SCALE = 100000;

const isChoice = (value: unknown): value is Choice => !!value && typeof value === 'object'
  && typeof (value as Choice).id === 'string' && typeof (value as Choice).name === 'string';
const choices = (value: unknown) => Array.isArray(value) && value.every(isChoice);

export function validateWeather(value: unknown): WeatherReply {
  const reply = value as WeatherReply;
  if (!reply || typeof reply !== 'object' || !choices(reply.regions) || !choices(reply.flags) || !choices(reply.modifiers)
    || !Number.isSafeInteger(reply.dayHours) || reply.dayHours <= 0 || typeof reply.temperature !== 'string'
    || reply.theatres !== undefined && !(Array.isArray(reply.theatres) && reply.theatres.every(theatre => isChoice(theatre)
      && Array.isArray((theatre as WeatherTheatre).regions) && (theatre as WeatherTheatre).regions.every(region => typeof region === 'string')))
    || !Array.isArray(reply.warnings) || !reply.warnings.every(warning => typeof warning === 'string'))
    throw new Error('The regional weather reply is malformed.');
  return { ...reply, window: validateKeyedWindow(reply.window) };
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

/** A heat cell for each day of each region. Conditions: every flag set that day, as
 * stripes in the recording's flag order (none set is blank). Modifier: the chosen
 * modifier's value on a ramp from grey, or hatched where it was not witnessed. */
export function weatherRows(reply: WeatherReply, regions: readonly Choice[], days: readonly KeyedDay[], show: WeatherShow, modifier: string | null,
  colours: { flag: (index: number) => string; ramp: (value: number, highest: number) => string; temperature?: (value: number, low: number, high: number) => string }): { rows: HeatRow[]; highest: number } {
  const series = regionSeries(reply.window);
  let highest = 0;
  if (show === 'modifiers' && modifier !== null)
    for (const names of series.values()) for (const run of names.get(`mod:${modifier}`) ?? []) highest = Math.max(highest, Math.abs(Number(run[2]) / MODIFIER_SCALE));
  let low = 0, high = 0;
  if (show === 'temperature')
    for (const names of series.values()) for (const run of names.get('temp') ?? []) { const value = Number(run[2]) / TEMPERATURE_SCALE; low = Math.min(low, value); high = Math.max(high, value); }
  const blank: HeatCell = { parts: [] }, unknown: HeatCell = { parts: [], unknown: true };
  const rows = regions.map(region => {
    const names = series.get(region.id);
    const cells = days.map((day): HeatCell => {
      if (day.readHour === null) return blank;
      if (show === 'temperature') {
        // A region with any province unknown has no average: its cell stays blank.
        const run = runAtHour(names?.get('temp') ?? [], day.at);
        if (!run) return blank;
        const value = Number(run[2]) / TEMPERATURE_SCALE;
        return { parts: [{ id: 'temp', label: temperatureText(run[2]), color: colours.temperature?.(value, low, high) ?? colours.ramp(Math.abs(value), Math.max(-low, high)), share: 1 }] };
      }
      if (show === 'modifiers') {
        const run = modifier === null ? undefined : runAtHour(names?.get(`mod:${modifier}`) ?? [], day.at);
        if (!run) return unknown;
        const value = Number(run[2]) / MODIFIER_SCALE;
        return value === 0 ? blank : { parts: [{ id: modifier!, label: String(value), color: colours.ramp(Math.abs(value), highest), share: 1 }] };
      }
      if (!names) return unknown;
      const set = reply.flags.flatMap((flag, index) => {
        const run = runAtHour(names.get(flag.id) ?? [], day.at);
        return run && run[2] !== '0' ? [{ index, flag }] : [];
      });
      return { parts: set.map(({ index, flag }) => ({ id: flag.id, label: flag.name, color: colours.flag(index), share: 1 / set.length })) };
    });
    return { id: region.id, label: region.name, cells };
  });
  return { rows, highest };
}

/** Every flag and modifier the region recorded at the day's hour, dated by its read. */
export function weatherTooltip(reply: WeatherReply, region: Choice, day: KeyedDay | null, date: string, readDate: string | null): TooltipContent {
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
      { label: 'Conditions', value: !names ? 'Not recorded' : flags.length ? flags.map(flag => flag.name).join(', ') : 'None' },
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
