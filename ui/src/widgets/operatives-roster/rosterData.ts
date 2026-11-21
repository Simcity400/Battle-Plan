import { countryNameDetails } from '../../charts/tooltip.ts';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { meterValue } from '../../charts/meter/meterValue.ts';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import { formatDateTime } from '../../dashboard/calendar.ts';
import type { Country, DepartedOperative, OperativeTenuresReply, OperativesDepartedReply, OperativesRosterReply, RosterOperative } from '../../generated/dashboard';

const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);
const integer = (value: unknown): value is number => Number.isSafeInteger(value);
const finite = (value: unknown) => value === null || (typeof value === 'number' && Number.isFinite(value));
const text = (value: unknown) => value === null || typeof value === 'string';
const strings = (value: unknown): value is string[] => Array.isArray(value) && value.every(item => typeof item === 'string');
const country = (value: unknown): value is Country => object(value) && integer(value.id) && (value.tag === null || (typeof value.tag === 'string' && value.tag.length > 0));
const index = (artwork: readonly unknown[]) => (value: unknown) => value === null || (integer(value) && value >= 0 && value < artwork.length);
const malformed = (what: string) => new Error(`The ${what} reply is malformed.`);

function operativeOk(item: unknown, artwork: readonly unknown[]): item is RosterOperative {
  const art = index(artwork);
  return object(item) && typeof item.id === 'string' && /^\d{1,20}$/.test(item.id) && typeof item.roster === 'string' && country(item.owner)
    && text(item.name) && typeof item.unnamed === 'boolean' && (!item.unnamed || item.name === null) && text(item.nameMissing)
    && art(item.portrait) && text(item.portraitMissing) && (item.skill === null || integer(item.skill))
    && typeof item.experienceRaw === 'string' && /^-?\d{1,20}$/.test(item.experienceRaw) && text(item.experienceText) && finite(item.experienceRatio)
    && text(item.experienceMissing) && integer(item.assignment) && text(item.assignmentName) && text(item.assignmentDuration) && finite(item.assignmentProgress)
    && text(item.assignmentMissing) && art(item.assignmentIcon) && text(item.assignmentIconMissing) && (item.target === null || country(item.target))
    && typeof item.captured === 'boolean' && (item.capturer === null || country(item.capturer)) && (item.capturedHour === null || integer(item.capturedHour));
}

export function validateRoster(value: unknown): OperativesRosterReply {
  if (!object(value) || !strings(value.artwork) || !Array.isArray(value.operatives) || !value.operatives.every(item => operativeOk(item, value.artwork as string[]))
    || !strings(value.unheld) || !(value.agencyFeature === null || typeof value.agencyFeature === 'boolean') || !(value.readHour === null || integer(value.readHour))
    || !text(value.missing) || !strings(value.warnings)) throw malformed('operatives roster');
  return value as unknown as OperativesRosterReply;
}

const leftOk = (item: Record<string, unknown>) => integer(item.firstHour) && integer(item.lastHour) && integer(item.departedHour)
  && item.firstHour <= item.lastHour && item.lastHour < item.departedHour && typeof item.departedReason === 'string';

export function validateDeparted(value: unknown): OperativesDepartedReply {
  if (!object(value) || !strings(value.artwork) || !Array.isArray(value.departed) || !strings(value.warnings)
    || !value.departed.every(item => operativeOk(item, value.artwork as string[]) && leftOk(item as unknown as Record<string, unknown>))) throw malformed('departed operatives');
  return value as unknown as OperativesDepartedReply;
}

export function validateTenures(value: unknown): OperativeTenuresReply {
  if (!object(value) || !integer(value.firstHour) || !integer(value.lastHour) || !Array.isArray(value.tenures) || !strings(value.warnings)
    || !value.tenures.every(item => object(item) && typeof item.id === 'string' && typeof item.roster === 'string' && integer(item.firstHour)
      && integer(item.lastHour) && item.firstHour <= item.lastHour)) throw malformed('operative tenures');
  return value as unknown as OperativeTenuresReply;
}

export interface OperativeRow { key: string; operative: RosterOperative; artwork: readonly string[]; departed: DepartedOperative | null; joinedHour: number | null }

export function rosterRows(roster: OperativesRosterReply | undefined, departed: OperativesDepartedReply | undefined, tenures: OperativeTenuresReply | undefined,
  hour: number, matches: (tag: string, hour: number) => boolean = () => true): OperativeRow[] {
  const joined = new Map<string, number>();
  if (tenures && tenures.firstHour <= hour && hour <= tenures.lastHour) {
    for (const tenure of tenures.tenures) if (tenure.firstHour <= hour && hour <= tenure.lastHour) joined.set(`${tenure.roster}\u0000${tenure.id}`, tenure.firstHour);
  }
  const rows: OperativeRow[] = [];
  for (const operative of roster?.operatives ?? []) {
    if (!matches(operative.roster, hour)) continue;
    rows.push({ key: JSON.stringify([operative.roster, operative.id]), operative, artwork: roster!.artwork, departed: null,
      joinedHour: joined.get(`${operative.roster}\u0000${operative.id}`) ?? null });
  }
  for (const left of departed?.departed ?? []) {
    if (!matches(left.roster, left.lastHour)) continue;
    rows.push({ key: JSON.stringify([left.roster, left.id, left.departedHour]), operative: left, artwork: departed!.artwork, departed: left, joinedHour: left.firstHour });
  }
  return rows;
}

export function rosterStructure(reply: OperativesRosterReply): string {
  return JSON.stringify([reply.missing, reply.unheld, reply.agencyFeature, reply.artwork.map(url => url.length), reply.operatives.map(operative => {
    const { experienceRaw: _raw, experienceText: _text, experienceRatio: _ratio, assignmentDuration: _duration, assignmentProgress: _progress, ...fixed } = operative;
    return fixed;
  })]);
}

export interface Painted { experience: string | null; experienceRatio: number | null; experienceRaw: number; duration: string | null; progress: number | null }
export type PaintedField = 'experience' | 'experienceMeter' | 'duration' | 'progress';

export function paintedOf(operative: RosterOperative): Painted {
  return { experience: operative.experienceText, experienceRatio: operative.experienceRatio, experienceRaw: Number(operative.experienceRaw),
    duration: operative.assignmentDuration, progress: operative.assignmentProgress };
}

export function paintedRows(reply: OperativesRosterReply): Map<string, Painted> {
  return new Map(reply.operatives.map(operative => [JSON.stringify([operative.roster, operative.id]), paintedOf(operative)]));
}

export function paintedText(painted: Painted | undefined, field: PaintedField): string {
  if (!painted) return '';
  switch (field) {
    case 'experience': return painted.experience ?? 'Unknown';
    case 'duration': return painted.duration ?? '';
    case 'experienceMeter': return `${(meterValue(painted.experienceRatio) ?? 0) * 100}%`;
    case 'progress': return `${(meterValue(painted.progress) ?? 0) * 100}%`;
  }
}

export function paintRoster(root: Element | null, rows: ReadonlyMap<string, Painted>) {
  if (!root) return;
  for (const element of root.querySelectorAll<HTMLElement>('[data-op-row]')) {
    const field = element.dataset.opField as PaintedField;
    const value = paintedText(rows.get(element.dataset.opRow!), field);
    if (field === 'experienceMeter' || field === 'progress') {
      const fill = element.firstElementChild as HTMLElement | null;
      if (fill && fill.style.width !== value) fill.style.width = value;
    } else if (element.textContent !== value) element.textContent = value;
  }
}

export function operativeName(operative: Pick<RosterOperative, 'name' | 'unnamed'>): string {
  return operative.unnamed ? 'Unnamed operative' : operative.name ?? 'Name unknown';
}

export const labelHour = (row: OperativeRow, shown: number) => row.departed ? row.departed.lastHour : shown;

const leftWord = (reason: string) => reason === 'Left (reason not recorded)' ? 'Left' : reason;
export function departureText(row: OperativeRow): string | null {
  return row.departed ? `${leftWord(row.departed.departedReason)} by ${formatDateTime(row.departed.departedHour)}` : null;
}

export interface OperativeMoment { kind: 'joined' | 'captured' | 'departed'; hour: number; label: string }
export function operativeMoments(row: OperativeRow): OperativeMoment[] {
  const moments: OperativeMoment[] = [];
  if (row.joinedHour !== null) moments.push({ kind: 'joined', hour: row.joinedHour, label: `Jump to first seen ${formatDateTime(row.joinedHour)}` });
  if (row.operative.captured && row.operative.capturedHour !== null) moments.push({ kind: 'captured', hour: row.operative.capturedHour, label: `Jump to capture ${formatDateTime(row.operative.capturedHour)}` });
  if (row.departed) moments.push({ kind: 'departed', hour: row.departed.departedHour, label: `Jump to departure by ${formatDateTime(row.departed.departedHour)}` });
  return moments;
}

export function capturedText(operative: RosterOperative, name: (tag: string) => string): string {
  if (!operative.captured) return 'No';
  const by = operative.capturer?.tag ? name(operative.capturer.tag) : 'Yes · capturer unknown';
  return [by, operative.capturedHour === null ? 'Capture date unknown' : formatDateTime(operative.capturedHour)].join(' · ');
}

export function operativeTooltip(row: OperativeRow, painted: Painted | undefined, shown: number, label: (tag: string, hour: number) => ReturnType<CountryLabel>): TooltipContent {
  const operative = row.operative;
  const at = labelHour(row, shown);
  const countryRow = (title: string, value: Country | null): TooltipRow => {
    if (!value?.tag) return { label: title, value: 'Unknown' };
    const named = label(value.tag, at);
    return { ...countryNameDetails(named), label: title, value: named.name, countryFlag: {
      flag: named.flag, flagReason: named.flagReason, ...named.flagDetail !== undefined ? { flagDetail: named.flagDetail } : {}, flagPending: named.flagPending,
    } };
  };
  const roster: Country = { id: operative.owner.id, tag: operative.roster };
  const values = painted ?? paintedOf(operative);
  const rows: TooltipRow[] = [
    ...operative.roster === operative.owner.tag ? [countryRow('Country', roster)] : [countryRow('Roster country', roster), countryRow('Owner', operative.owner)],
    { label: 'Skill', value: operative.skill === null ? 'Unknown' : String(operative.skill) },
    { label: 'Experience', value: values.experience ?? operative.experienceMissing ?? 'Unknown' },
    { label: 'Assignment', value: operative.assignmentName ?? operative.assignmentMissing ?? 'Unknown' },
    ...values.duration === null ? [] : [{ label: 'Assignment duration', value: values.duration }],
    ...operative.target === null ? [] : [countryRow('Target', operative.target)],
    { label: 'Captured', value: operative.captured ? 'Yes' : 'No' },
    ...operative.captured ? [countryRow('Capturer', operative.capturer),
      { label: 'Captured on', value: operative.capturedHour === null ? 'Unknown' : formatDateTime(operative.capturedHour) }] : [],
    ...row.joinedHour === null ? [] : [{ label: 'Present', value: `by ${formatDateTime(row.joinedHour)}` }],
    ...row.departed ? [{ label: leftWord(row.departed.departedReason), value: `by ${formatDateTime(row.departed.departedHour)}` }] : [],
    ...operative.portraitMissing ? [{ label: 'Portrait', value: operative.portraitMissing }] : [],
  ];
  return { title: operativeName(operative), icon: operative.portrait === null ? undefined : row.artwork[operative.portrait], rows };
}
