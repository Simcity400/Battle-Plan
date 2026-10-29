import { runAtHour, validateKeyedWindow } from '../../dashboard/keyedHistory.ts';
import type { KeyedRun, KeyedWindow } from '../../dashboard/keyedHistory';
import type { KeyedDay } from '../../dashboard/keyedDays';
import type { HeatCell, HeatRow } from '../../charts/heat/layout';
import type { TooltipContent } from '../../charts/tooltip';

export interface Choice { id: string; name: string }
export interface SupremacyReply { window: KeyedWindow; dayHours: number; regions: readonly Choice[]; scale: number; warnings: readonly string[] }
export type Control = 'friendly' | 'enemy' | 'neither';

const isChoice = (value: unknown): value is Choice => !!value && typeof value === 'object'
  && typeof (value as Choice).id === 'string' && typeof (value as Choice).name === 'string';

export function validateSupremacy(value: unknown): SupremacyReply {
  const reply = value as SupremacyReply;
  if (!reply || typeof reply !== 'object' || !Array.isArray(reply.regions) || !reply.regions.every(isChoice)
    || !Number.isSafeInteger(reply.dayHours) || reply.dayHours <= 0 || !Number.isSafeInteger(reply.scale) || reply.scale <= 0
    || !Array.isArray(reply.warnings) || !reply.warnings.every(warning => typeof warning === 'string'))
    throw new Error('The naval supremacy reply is malformed.');
  return { ...reply, window: validateKeyedWindow(reply.window) };
}

/** One region's day: the game's control outcome from the viewer's point of view, the control
 * rule's sums, the explanatory share (the game's own quotient) and its parts, and the viewer's
 * own current, each null where the read holds none (a refused operand leaves the others). */
export interface RegionDay { control: Control | null; friendly: bigint | null; enemy: bigint | null; share: bigint | null; shareNum: bigint | null; shareDen: bigint | null; own: bigint | null }

export function regionSeries(window: KeyedWindow): Map<string, Map<string, readonly KeyedRun[]>> {
  const out = new Map<string, Map<string, readonly KeyedRun[]>>();
  for (const series of window.series) {
    const [region, field] = series.name.split('|');
    if (!field) continue;
    const names = out.get(region) ?? new Map<string, readonly KeyedRun[]>();
    names.set(field, series.runs);
    out.set(region, names);
  }
  return out;
}

export function regionDay(names: ReadonlyMap<string, readonly KeyedRun[]> | undefined, hour: number): RegionDay {
  const value = (field: string) => { const run = runAtHour(names?.get(field) ?? [], hour); return run ? BigInt(run[2]) : null; };
  const control = value('control');
  return { control: control === null ? null : control > 0n ? 'friendly' : control < 0n ? 'enemy' : 'neither',
    friendly: value('friendly'), enemy: value('enemy'), share: value('share'), shareNum: value('shareNum'), shareDen: value('shareDen'), own: value('own') };
}

/** Heat rows: each region's daily control outcome as one colour; a read day without an admitted
 * control, whether or not the region has values on other days, is hatched. */
export function supremacyRows(reply: SupremacyReply, regions: readonly Choice[], days: readonly KeyedDay[],
  colours: Record<Control, string>, labels: Record<Control, string>): HeatRow[] {
  const series = regionSeries(reply.window);
  const blank: HeatCell = { parts: [] }, unknown: HeatCell = { parts: [], unknown: true };
  const cells: Record<Control, HeatCell> = {
    friendly: { parts: [{ id: 'friendly', label: labels.friendly, color: colours.friendly, share: 1 }] },
    enemy: { parts: [{ id: 'enemy', label: labels.enemy, color: colours.enemy, share: 1 }] },
    neither: { parts: [{ id: 'neither', label: labels.neither, color: colours.neither, share: 1 }] },
  };
  return regions.map(region => {
    const names = series.get(region.id);
    return { id: region.id, label: region.name, cells: days.map(day => {
      if (day.readHour === null) return blank;
      const control = regionDay(names, day.at).control;
      return control === null ? unknown : cells[control];
    }) };
  });
}

/** A fixed-point amount on the reply's scale. */
export function amountText(value: bigint | null, scale: number): string {
  if (value === null) return 'Not recorded';
  return (Number(value) / scale).toLocaleString('en-US', { maximumFractionDigits: 2 });
}

/** The explanatory share as a percent: the backend's native fixed-point quotient, shown as is. */
export function shareText(day: RegionDay, scale: number): string {
  if (day.share === null) return 'Not recorded';
  return `${(Number(day.share) / scale * 100).toLocaleString('en-US', { maximumFractionDigits: 1 })}%`;
}

export function supremacyTooltip(region: Choice, day: RegionDay | null, viewer: string, labels: Record<Control, string>, scale: number, date: string, readDate: string | null): TooltipContent {
  if (!day) return { title: region.name, rows: [{ label: date }, { label: 'Not recorded' }] };
  return {
    title: region.name,
    rows: [
      { label: date },
      { label: 'Control', value: day.control === null ? 'Not recorded' : labels[day.control] },
      { label: `Friendly to ${viewer}`, value: amountText(day.friendly, scale) },
      { label: `Enemies of ${viewer}`, value: amountText(day.enemy, scale) },
      { label: 'Supremacy share', value: shareText(day, scale) },
      { label: `${viewer}'s own`, value: amountText(day.own, scale) },
      ...readDate ? [{ label: 'As of', value: readDate }] : [],
    ],
  };
}
