import { object, int } from '../../dashboard/validation.ts';
import { formatDate, formatDateTime, gameDate, isEngineHour } from '../../dashboard/calendar.ts';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import type { AirAce, AirAcePortraitReply, AirAcesReply } from '../../generated/dashboard';

const text = (v: unknown) => v === null || typeof v === 'string';
const handle = (v: unknown): v is string => typeof v === 'string' && /^\d+$/.test(v) && BigInt(v) <= 0xffff_ffff_ffff_ffffn;
const hour = (v: unknown) => int(v) && isEngineHour(v);
const nullableHour = (v: unknown) => v === null || hour(v);
const warnings = (v: unknown) => Array.isArray(v) && v.every(item => typeof item === 'string');
const invalid = (): never => { throw new Error('Invalid ace roster reply'); };

function validAce(v: unknown): boolean {
  if (!object(v) || !handle(v.id) || !text(v.tag) || !text(v.name) || !text(v.callsign) || !(v.wing === null || handle(v.wing)) || !text(v.wingName)
    || typeof v.alive !== 'boolean' || !nullableHour(v.firstSeen)) return false;
  if (!(v.aircraft === null || Array.isArray(v.aircraft) && v.aircraft.every(plane => object(plane) && handle(plane.id) && text(plane.name) && text(plane.missing)
    && (plane.name === null) !== (plane.missing === null)))) return false;
  if (v.alive) return v.death === null;
  const d = v.death;
  return object(d) && nullableHour(d.lastAlive) && nullableHour(d.byHour)
    && ((d.lastAlive === null && d.byHour === null) || (int(d.lastAlive) && int(d.byHour) && d.lastAlive < d.byHour))
    && (d.killerAce === null || handle(d.killerAce)) && text(d.killerName) && text(d.killerTag);
}

export function validateAces(v: unknown): AirAcesReply {
  if (!object(v) || !nullableHour(v.readHour) || !text(v.missing) || !Array.isArray(v.aces) || !warnings(v.warnings)
    || (v.missing !== null && v.aces.length > 0)) return invalid();
  const ids = new Set<string>();
  for (const ace of v.aces) {
    if (!validAce(ace) || ids.has((ace as AirAce).id)) return invalid();
    ids.add((ace as AirAce).id);
  }
  return v as unknown as AirAcesReply;
}

export function validatePortraits(v: unknown): AirAcePortraitReply {
  if (!object(v) || !Array.isArray(v.portraits) || !v.portraits.every(p => object(p) && handle(p.id) && text(p.image) && text(p.missing)) || !warnings(v.warnings)) return invalid();
  return v as unknown as AirAcePortraitReply;
}

export interface AceRow { key: string; ace: AirAce; hour: number }

export function aceRows(reply: AirAcesReply, hour: number, matches: (tag: string | null, read: number) => boolean = () => true): AceRow[] {
  return reply.aces.flatMap(ace => matches(ace.tag, hour) ? [{ key: ace.id, ace, hour }] : []);
}

const date = (hour: number) => { const d = gameDate(hour); return d ? formatDate(d) : formatDateTime(hour); };
export const aceCallsign = (row: AceRow) => row.ace.callsign;
export const aceNameValue = (row: AceRow) => row.ace.name;
export const aceName = (row: AceRow) => aceNameValue(row) || 'Unknown ace';
export const wingName = (row: AceRow) => row.ace.wing === null ? 'No wing' : row.ace.wingName ?? 'Wing name not recorded';
export const aircraftName = (row: AceRow) => row.ace.wing === null ? '' : row.ace.aircraft === null ? 'Aircraft not recorded'
  : row.ace.aircraft.length ? row.ace.aircraft.map(plane => plane.name ?? 'Unknown aircraft').join(' · ') : 'No aircraft recorded';
export const aceStatus = (row: AceRow) => row.ace.alive
  ? row.ace.firstSeen === null ? 'Alive; first sighting not recorded' : `Ace by ${date(row.ace.firstSeen)}`
  : row.ace.death?.byHour != null ? `Killed by ${date(row.ace.death.byHour)}` : 'Killed, date not recorded';
export const aceJump = (row: AceRow) => row.ace.death?.byHour ?? row.ace.firstSeen ?? row.hour;
export const aceDate = (row: AceRow) => row.ace.firstSeen;

export function aceTooltip(row: AceRow, name: (tag: string) => string): TooltipContent {
  const rows: TooltipRow[] = [
    { label: row.ace.tag === null ? 'Country not recorded' : name(row.ace.tag) },
    { label: 'Air wing', value: wingName(row) },
    ...(row.ace.wing === null ? [] : [{ label: 'Aircraft', value: aircraftName(row) }]),
    { label: 'Became an ace', value: row.ace.firstSeen === null ? 'Not recorded' : `by ${date(row.ace.firstSeen)}`, divider: true },
    { label: 'Status', value: row.ace.alive ? 'Alive' : aceStatus(row) },
  ];
  const d = row.ace.death;
  if (d?.lastAlive != null && d.byHour != null) rows.push({ label: 'Death interval', value: `After ${date(d.lastAlive)}; by ${date(d.byHour)}` });
  if (d?.killerAce) rows.push({ label: 'Killer ace', value: d.killerName || 'Unknown ace' });
  if (d) rows.push({ label: 'Killer country', value: d.killerTag === null ? 'Not recorded' : name(d.killerTag) });
  return { title: `${aceName(row)}${aceCallsign(row) ? ` “${aceCallsign(row)}”` : ''}`, rows };
}
