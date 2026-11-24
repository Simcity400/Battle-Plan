import type { HeatCell, HeatPeriod, HeatRow } from '../../charts/heat/layout';
import type { TooltipContent } from '../../charts/tooltip';
import type { NavalSupremacyReply, SupremacyRegion, SupremacyRun } from '../../generated/dashboard';

export type Control = 'friendly' | 'enemy' | 'neither';

export const STEP = 24 * 30;

export const followedEnd = (hour: number, playing: boolean) => playing ? Math.floor(hour / STEP) * STEP + STEP - 1 : Math.floor(hour);

const text = (value: unknown): value is string => typeof value === 'string';
const optionalDecimal = (value: unknown) => value === null || text(value) && /^-?\d+$/u.test(value);
const day = (value: unknown) => Number.isSafeInteger(value) && (value as number) >= 0;

export function validateSupremacy(value: unknown): NavalSupremacyReply {
  const reply = value as NavalSupremacyReply;
  const run = (item: SupremacyRun) => !!item && day(item.firstDay) && day(item.lastDay) && item.firstDay <= item.lastDay && item.lastDay < reply.days
    && (item.control === null || [1, 0, -1].includes(item.control)) && optionalDecimal(item.friendly) && optionalDecimal(item.enemy)
    && optionalDecimal(item.share) && optionalDecimal(item.own);
  if (!reply || typeof reply !== 'object' || !Number.isSafeInteger(reply.firstHour) || !Number.isSafeInteger(reply.lastHour) || (reply.firstHour > reply.lastHour && reply.days > 0)
    || !Number.isSafeInteger(reply.firstDayHour) || !Number.isSafeInteger(reply.dayHours) || reply.dayHours <= 0 || !day(reply.days)
    || !Number.isSafeInteger(reply.scale) || reply.scale <= 0 || !Array.isArray(reply.warnings) || !reply.warnings.every(text)
    || !Array.isArray(reply.unknown) || !reply.unknown.every(span => !!span && day(span.firstDay) && day(span.lastDay) && span.firstDay <= span.lastDay && text(span.reason))
    || !Array.isArray(reply.regions) || !reply.regions.every((region: SupremacyRegion) => !!region && text(region.id) && text(region.name) && Array.isArray(region.runs) && region.runs.every(run)))
    throw new Error('The naval supremacy reply is malformed.');
  return reply;
}

export function supremacyDays(reply: NavalSupremacyReply, extent: readonly [number, number]): (HeatPeriod & { index: number })[] {
  const unknown = new Set(reply.unknown.flatMap(span => Array.from({ length: span.lastDay - span.firstDay + 1 }, (_, at) => span.firstDay + at)));
  const out: (HeatPeriod & { index: number })[] = [];
  const from = Math.max(extent[0], reply.firstHour), to = Math.min(extent[1], reply.lastHour);
  for (let index = Math.max(0, Math.floor((from - reply.firstDayHour) / reply.dayHours)); index < reply.days; index++) {
    const start = reply.firstDayHour + index * reply.dayHours;
    if (start > to) break;
    const firstHour = Math.max(start, from), lastHour = Math.min(start + reply.dayHours - 1, to);
    out.push({ index, firstHour, lastHour, readHour: unknown.has(index) ? null : lastHour });
  }
  return out;
}

export function runOn(runs: readonly SupremacyRun[], index: number): SupremacyRun | null {
  let low = 0, high = runs.length - 1;
  while (low <= high) {
    const middle = (low + high) >> 1, run = runs[middle];
    if (index < run.firstDay) high = middle - 1; else if (index > run.lastDay) low = middle + 1; else return run;
  }
  return null;
}

export const controlOf = (run: SupremacyRun | null): Control | null => run?.control == null ? null : run.control > 0 ? 'friendly' : run.control < 0 ? 'enemy' : 'neither';

export function supremacyRows(regions: readonly SupremacyRegion[], days: readonly (HeatPeriod & { index: number })[], colours: Record<Control, string>, labels: Record<Control, string>): HeatRow[] {
  const blank: HeatCell = { parts: [] }, unknown: HeatCell = { parts: [], unknown: true };
  const cells: Record<Control, HeatCell> = {
    friendly: { parts: [{ id: 'friendly', label: labels.friendly, color: colours.friendly, share: 1 }] },
    enemy: { parts: [{ id: 'enemy', label: labels.enemy, color: colours.enemy, share: 1 }] },
    neither: { parts: [{ id: 'neither', label: labels.neither, color: colours.neither, share: 1 }] },
  };
  return regions.map(region => ({ id: region.id, label: region.name, cells: days.map(item => {
    if (item.readHour === null) return blank;
    const control = controlOf(runOn(region.runs, item.index));
    return control === null ? unknown : cells[control];
  }) }));
}

export function amountText(value: string | null, scale: number): string {
  if (value === null) return 'Not recorded';
  return (Number(value) / scale).toLocaleString('en-US', { maximumFractionDigits: 2 });
}

export function shareText(value: string | null, scale: number): string {
  if (value === null) return 'Not recorded';
  return `${(Number(value) / scale * 100).toLocaleString('en-US', { maximumFractionDigits: 1 })}%`;
}

export function supremacyTooltip(region: SupremacyRegion, run: SupremacyRun | null, viewer: string, labels: Record<Control, string>, scale: number, date: string, reason: string | null): TooltipContent {
  if (reason !== null || !run) return { title: region.name, rows: [{ label: date }, { label: reason ?? 'Not recorded' }] };
  const control = controlOf(run);
  return { title: region.name, rows: [
    { label: date },
    { label: 'Control', value: control === null ? 'Not recorded' : labels[control] },
    { label: `Friendly to ${viewer}`, value: amountText(run.friendly, scale) },
    { label: `Enemies of ${viewer}`, value: amountText(run.enemy, scale) },
    { label: 'Supremacy share', value: shareText(run.share, scale) },
    { label: `${viewer}'s own`, value: amountText(run.own, scale) },
  ] };
}

export function unknownReason(reply: NavalSupremacyReply, index: number): string | null {
  return reply.unknown.find(span => span.firstDay <= index && index <= span.lastDay)?.reason ?? null;
}

export function mergeSupremacy(windows: ReadonlyMap<number, NavalSupremacyReply>): NavalSupremacyReply | null {
  const held = [...windows].sort((a, b) => a[0] - b[0]).map(([, reply]) => reply).filter(reply => reply.days > 0);
  const first = held[0];
  if (!first) return windows.values().next().value ?? null;
  const base = first.firstDayHour, dayHours = first.dayHours;
  const regions = new Map<string, SupremacyRegion>();
  const unknown: NavalSupremacyReply['unknown'] = [];
  const warnings = new Set<string>();
  let days = first.days, lastHour = first.lastHour;
  const same = (a: SupremacyRun, b: SupremacyRun) => a.control === b.control && a.friendly === b.friendly && a.enemy === b.enemy && a.share === b.share && a.own === b.own;
  for (const reply of [...windows.values()]) for (const warning of reply.warnings) warnings.add(warning);
  for (const reply of held) {
    const offset = Math.round((reply.firstDayHour - base) / dayHours);
    days = Math.max(days, offset + reply.days);
    lastHour = Math.max(lastHour, reply.lastHour);
    for (const region of reply.regions) {
      let into = regions.get(region.id);
      if (!into) {
        into = { id: region.id, name: region.name, pinned: region.pinned, runs: [] };
        regions.set(region.id, into);
      }
      for (const run of region.runs) {
        const placed = { ...run, firstDay: run.firstDay + offset, lastDay: run.lastDay + offset };
        const open = into.runs.at(-1);
        if (open && open.lastDay + 1 === placed.firstDay && same(open, placed)) into.runs[into.runs.length - 1] = { ...open, lastDay: placed.lastDay };
        else into.runs.push(placed);
      }
    }
    for (const span of reply.unknown) {
      const placed = { ...span, firstDay: span.firstDay + offset, lastDay: span.lastDay + offset };
      const open = unknown.at(-1);
      if (open && open.lastDay + 1 === placed.firstDay && open.reason === placed.reason) unknown[unknown.length - 1] = { ...open, lastDay: placed.lastDay };
      else unknown.push(placed);
    }
  }
  return { ...first, lastHour, days, unknown, regions: [...regions.values()], warnings: [...warnings] };
}
