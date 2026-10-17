import { object, int } from '../../dashboard/validation.ts';
import { isOrderedSpans, runAt } from '../../charts/series.ts';
import { formatDate, formatDateTime, gameDate, isEngineHour } from '../../dashboard/calendar.ts';
import { variantNameAt } from '../../dashboard/equipment.ts';
import { readAt } from '../../dashboard/reads.ts';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import type { AirAceEntry as WireAceEntry, AirAceRun, AirAcesManifestReply, AirAcesWindowReply as WireAceWindow, AirAcePortraitReply } from '../../generated/dashboard';

export type AirAceEntry = Omit<WireAceEntry, 'firstSeen'> & { firstSeen: number | null };
export type AirAcesWindowReply = Omit<WireAceWindow, 'aces'> & { aces: AirAceEntry[] };
export interface AceRosterData { manifest: AirAcesManifestReply; roster: AirAcesWindowReply; readHour: number | null; preparing: string[] }
export function validateAceRoster(value: unknown): AceRosterData {
  if (!object(value) || !nullableHour(value.readHour) || !Array.isArray(value.preparing) || !value.preparing.every(field => typeof field === 'string')) return invalid();
  const manifest = validateAces(value.manifest), roster = validateAcesWindow(value.roster);
  const preparing = value.preparing.includes('firstSeen');
  if (roster.aces.some(ace => (ace.firstSeen === null) !== preparing)
    || value.readHour !== null && !manifest.readHours.includes(value.readHour as number)) return invalid();
  return value as unknown as AceRosterData;
}

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
    if (!object(a) || !handle(a.id) || ids.has(a.id) || !nullableHour(a.firstSeen) || !Array.isArray(a.runs) || !a.runs.length || !isOrderedSpans(a.runs)
      || !a.runs.every(r => validRun(r, v.texts as string[]))
      || a.runs.some(r => r.firstHour > (v.lastHour as number) || r.lastHour < (v.firstHour as number) || r.lastHour > (v.lastHour as number) || a.firstSeen !== null && r.firstHour < (a.firstSeen as number))) return invalid();
    ids.add(a.id);
  }
  return v as unknown as AirAcesWindowReply;
}
export function validatePortraits(v: unknown): AirAcePortraitReply {
  if (!object(v) || !Array.isArray(v.portraits) || !v.portraits.every(p => object(p) && handle(p.id) && text(p.image) && text(p.missing)) || !warnings(v.warnings)) return invalid();
  return v as unknown as AirAcePortraitReply;
}
export interface AceRow { key: string; ace: AirAceEntry; run: AirAceRun; hour: number; data: AirAcesManifestReply; window: AirAcesWindowReply; roster: ReadonlyMap<string, AirAceEntry>; preparing?: boolean }
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
export const aceStatus = (row: AceRow) => row.preparing || row.ace.firstSeen === null ? (row.run.alive ? 'Alive; dates still being prepared' : 'Killed; dates still being prepared') : row.run.alive ? `Ace by ${date(row.ace.firstSeen!)}` : row.run.death?.byHour != null ? `Killed by ${date(row.run.death.byHour)}` : 'Killed, date not recorded';
export const aceJump = (row: AceRow) => row.run.death?.byHour ?? row.ace.firstSeen ?? row.hour;

/** Exact row operands and their referenced text/design inputs. Whole replies
 * and process-wide content diagnostics are not operands of one ace row. */
export function aceRowModel(row: AceRow) {
  const killerIds = [...new Set(row.ace.runs.flatMap(run => run.death?.killerAce ? [run.death.killerAce] : []))].sort();
  const killers = killerIds.map(id => ({ id, ace: row.roster.get(id) ?? null }));
  const entries = [row.ace, ...killers.flatMap(killer => killer.ace ? [killer.ace] : [])];
  const indices = [...new Set(entries.flatMap(ace => ace.runs.flatMap(run => [run.name, run.callsign, run.wingName]
    .filter((index): index is number => index !== null))))].sort((a, b) => a - b);
  const variants = [...new Set(entries.flatMap(ace => ace.runs.flatMap(run => run.aircraft ?? [])))].sort();
  return { key: row.key, ace: row.ace, run: row.run, hour: row.hour, killers,
    coverage: { branchId: row.data.branchId, firstHour: row.data.firstHour, lastHour: row.data.lastHour,
      observed: runAt(row.data.observed, row.hour), window: { index: row.window.window, firstHour: row.window.firstHour, lastHour: row.window.lastHour } },
    texts: indices.map(index => ({ index, value: row.window.texts[index] })),
    variants: variants.map(id => ({ id, runs: row.data.variants[id] ?? [] })) };
}
export function aceTooltip(row: AceRow, name: (tag: string) => string): TooltipContent {
  const rows: TooltipRow[] = [
    { label: row.run.tag === null ? 'Country not recorded' : name(row.run.tag) },
    { label: 'Air wing', value: wingName(row) },
    ...(row.run.wing === null ? [] : [{ label: 'Aircraft', value: aircraftName(row) }]),
    { label: 'Became an ace', value: row.preparing || row.ace.firstSeen === null ? 'Still being prepared' : `by ${date(row.ace.firstSeen!)}`, divider: true },
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
