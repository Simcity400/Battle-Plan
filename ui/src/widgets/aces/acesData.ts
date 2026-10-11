import { object, int } from '../../dashboard/validation.ts';
import { isOrderedSpans, runAt } from '../../charts/series.ts';
import { formatDate, formatDateTime, gameDate, isEngineHour } from '../../dashboard/calendar.ts';
import { variantNameAt } from '../../dashboard/equipment.ts';
import { readAt } from '../../dashboard/reads.ts';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import type { AirAceEntry, AirAceRun, AirAcesManifestReply, AirAcesWindowReply, AirAcePortraitReply } from '../../generated/dashboard';

const text = (v: unknown) => v === null || typeof v === 'string';
const handle = (v: unknown): v is string => typeof v === 'string' && /^\d+$/.test(v) && BigInt(v) <= 0xffff_ffff_ffff_ffffn;
const hour = (v: unknown) => int(v) && isEngineHour(v);
const nullableHour = (v: unknown) => v === null || hour(v);
const span = (v: unknown) => object(v) && hour(v.firstHour) && hour(v.lastHour) && (v.firstHour as number) <= (v.lastHour as number);
const invalid = (): never => { throw new Error('Invalid ace roster reply'); };
const warnings = (v: unknown) => Array.isArray(v) && v.every(item => typeof item === 'string');
function validRun(v: unknown, texts: readonly string[]): boolean {
  const index = (v: unknown) => v === null || (int(v) && v >= 0 && v < texts.length);
  if (!object(v) || !span(v)
    || !text(v.tag) || !['name', 'callsign', 'wingName'].every(key => index(v[key])) || !(v.wing === null || handle(v.wing)) || typeof v.alive !== 'boolean'
    || !(v.aircraft === null || (Array.isArray(v.aircraft) && v.aircraft.every(handle)))) return false;
  if (v.alive) return v.death === null;
  const d = v.death;
  return object(d) && nullableHour(d.lastAlive) && nullableHour(d.byHour)
    && ((d.lastAlive === null && d.byHour === null) || (int(d.lastAlive) && int(d.byHour) && d.lastAlive < d.byHour && d.byHour <= (v.firstHour as number)))
    && (d.killerAce === null || handle(d.killerAce)) && text(d.killerTag);
}
export function validateAces(v: unknown): AirAcesManifestReply {
  if (!object(v) || !span(v) || !handle(v.branchId) || typeof v.available !== 'boolean' || !text(v.unavailable)
    || !Array.isArray(v.observed) || !isOrderedSpans(v.observed)
    || v.observed.some(s => !span(s) || s.firstHour < (v.firstHour as number) || s.lastHour > (v.lastHour as number))
    || !Array.isArray(v.readHours) || !v.readHours.every((h, i, all) => hour(h) && h <= (v.lastHour as number) && (i === 0 || all[i - 1] < h))
    || !Array.isArray(v.windows) || !isOrderedSpans(v.windows) || v.windows.some(s => !span(s) || s.firstHour < (v.firstHour as number) || s.lastHour > (v.lastHour as number))
    || !Array.isArray(v.aces) || !warnings(v.warnings) || !object(v.variants)
    || !Object.entries(v.variants).every(([id, names]) => handle(id) && Array.isArray(names) && names.every((n, i, all) => object(n) && hour(n.firstHour) && text(n.name) && text(n.design) && typeof n.known === 'boolean' && (i === 0 || (all[i - 1] as { firstHour: number }).firstHour < (n.firstHour as number))))) return invalid();
  const ids = new Set<string>();
  for (const a of v.aces) {
    if (!object(a) || !handle(a.id) || ids.has(a.id) || !hour(a.firstSeen) || (a.firstSeen as number) < (v.firstHour as number) || (a.firstSeen as number) > (v.lastHour as number)) return invalid();
    ids.add(a.id);
  }
  return v as unknown as AirAcesManifestReply;
}
export function validateAcesWindow(v: unknown): AirAcesWindowReply {
  if (!object(v) || !span(v) || !handle(v.branchId) || !int(v.window) || v.window < 0
    || !Array.isArray(v.texts) || !v.texts.every(t => typeof t === 'string') || !warnings(v.warnings) || !Array.isArray(v.aces)) return invalid();
  const ids = new Set<string>();
  for (const a of v.aces) {
    if (!object(a) || !handle(a.id) || ids.has(a.id) || !hour(a.firstSeen) || !Array.isArray(a.runs) || !a.runs.length || !isOrderedSpans(a.runs)
      || !a.runs.every(r => validRun(r, v.texts as string[]))
      || a.runs.some(r => r.firstHour > (v.lastHour as number) || r.lastHour < (v.firstHour as number) || r.lastHour > (v.lastHour as number) || r.firstHour < (a.firstSeen as number))) return invalid();
    ids.add(a.id);
  }
  return v as unknown as AirAcesWindowReply;
}
export function validatePortraits(v: unknown): AirAcePortraitReply {
  if (!object(v) || !Array.isArray(v.portraits) || !v.portraits.every(p => object(p) && handle(p.id) && text(p.image) && text(p.missing)) || !warnings(v.warnings)) return invalid();
  return v as unknown as AirAcePortraitReply;
}
export interface AceRow { key: string; ace: AirAceEntry; run: AirAceRun; hour: number; data: AirAcesManifestReply; window: AirAcesWindowReply; roster: ReadonlyMap<string, AirAceEntry> }
const rosters = new WeakMap<AirAcesWindowReply, ReadonlyMap<string, AirAceEntry>>();
export function aceRows(data: AirAcesManifestReply, window: AirAcesWindowReply, hour: number, matches: (tag: string | null, read: number) => boolean = () => true): AceRow[] {
  if (hour < window.firstHour || hour > window.lastHour || !runAt(data.observed, hour)) return [];
  const read = readAt(data.readHours, hour);
  let roster = rosters.get(window);
  if (!roster) {
    roster = new Map(window.aces.map(ace => [ace.id, ace]));
    rosters.set(window, roster);
  }
  return window.aces.flatMap(ace => {
    const run = runAt(ace.runs, hour);
    return run && read !== null && matches(run.tag, read) ? [{ key: ace.id, ace, run, hour: read, data, window, roster }] : [];
  });
}
const date = (hour: number) => { const d = gameDate(hour); return d ? formatDate(d) : formatDateTime(hour); };
const runText = (window: AirAcesWindowReply, index: number | null) => index === null ? null : window.texts[index];
export const aceCallsign = (row: AceRow) => runText(row.window, row.run.callsign);
export const aceNameValue = (row: AceRow) => runText(row.window, row.run.name);
export const aceName = (row: AceRow) => aceNameValue(row) || 'Unknown ace';
export const wingName = (row: AceRow) => row.run.wing === null ? 'No wing' : runText(row.window, row.run.wingName) ?? 'Wing name not recorded';
export const aircraftName = (row: AceRow) => row.run.wing === null ? '' : row.run.aircraft === null ? 'Aircraft not recorded' : row.run.aircraft.length ? row.run.aircraft.map(id => variantNameAt(row.data.variants[id] ?? [], row.hour, 'Unknown aircraft')).join(' · ') : 'No aircraft recorded';
export const aceStatus = (row: AceRow) => row.run.alive ? `Ace by ${date(row.ace.firstSeen)}` : row.run.death?.byHour != null ? `Killed by ${date(row.run.death.byHour)}` : 'Killed, date not recorded';
export const aceJump = (row: AceRow) => row.run.death?.byHour ?? row.ace.firstSeen;
export function aceTooltip(row: AceRow, name: (tag: string) => string): TooltipContent {
  const rows: TooltipRow[] = [
    { label: row.run.tag === null ? 'Country not recorded' : name(row.run.tag) },
    { label: 'Air wing', value: wingName(row) },
    ...(row.run.wing === null ? [] : [{ label: 'Aircraft', value: aircraftName(row) }]),
    { label: 'Became an ace', value: `by ${date(row.ace.firstSeen)}`, divider: true },
    { label: 'Status', value: row.run.alive ? 'Alive' : aceStatus(row) },
  ];
  const d = row.run.death;
  if (d?.lastAlive != null && d.byHour != null) rows.push({ label: 'Death interval', value: `After ${date(d.lastAlive)}; by ${date(d.byHour)}` });
  if (d?.killerAce) {
    const killer = row.roster.get(d.killerAce);
    const run = killer && runAt(killer.runs, row.hour);
    rows.push({ label: 'Killer ace', value: (run && runText(row.window, run.name)) || 'Unknown ace' });
  }
  if (d) rows.push({ label: 'Killer country', value: d.killerTag === null ? 'Not recorded' : name(d.killerTag) });
  return { title: `${aceName(row)}${aceCallsign(row) ? ` “${aceCallsign(row)}”` : ''}`, rows };
}
