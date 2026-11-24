import { formatDateTime, isEngineHour } from '../../dashboard/calendar.ts';
import { int, object } from '../../dashboard/validation.ts';
import { dayLabel } from '../../charts/draw.ts';
import { formatAmount, formatFixed } from '../../charts/valueAxis.ts';
import { usNumber } from '../../charts/numberFormat.ts';
import type { Run } from '../../charts/series';
import { battleDate, battlePartial, battleValueRow, battleValueText, markPartial } from '../battles/battleRows.ts';
import type { BattleCountryLabel, BattleSide, BattleValue } from '../battles/battleRows';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { landManpowerSides } from './landBattlesData.ts';
import type { LandBattleRow } from './landBattlesData';

export interface LandDesign { name: string | null; variant: string }
export interface CourseSide { manpower: number | string; equipment: Map<number, bigint> | string }
export interface CourseRead { hour: number; terminal: boolean; sides: [CourseSide, CourseSide] | string }
export interface LandCourse { id: string; runs: { firstHour: number; lastHour: number; counts: [number, number]; rosterUnknown: boolean }[];
  reads: CourseRead[]; designs: LandDesign[]; contentWarnings: string[] }
export interface LandTerrain { provinceId: number; name: string | null; missing: string | null }

const hour = (v: unknown): v is number => typeof v === 'number' && isEngineHour(v);
const texts = (v: unknown): v is string[] => Array.isArray(v) && v.every(text => typeof text === 'string');
const amount = (v: unknown): v is string => typeof v === 'string' && v.length <= 40 && /^-?(0|[1-9]\d*)$/.test(v);
const pairs = (v: unknown, size: number): v is [number, string][] => Array.isArray(v) && v.every(entry => Array.isArray(entry) && entry.length === 2
  && int(entry[0]) && entry[0] >= 0 && entry[0] < size && amount(entry[1]));
const malformed = () => new Error('The land battle course is malformed.');

export function validateLandBattleCourse(v: unknown): LandCourse {
  if (!object(v) || !object(v.battle) || typeof v.battle.id !== 'string' || !Array.isArray(v.designs) || !texts(v.contentWarnings)) throw malformed();
  const designs: LandDesign[] = [];
  for (const design of v.designs) {
    if (!object(design) || !(design.name === null || typeof design.name === 'string') || typeof design.variant !== 'string') throw malformed();
    designs.push({ name: design.name as string | null, variant: design.variant });
  }
  const battle = v.battle;
  if (!Array.isArray(battle.runs) || !Array.isArray(battle.losses)) throw malformed();
  const runs = battle.runs.map(run => {
    if (!object(run) || !hour(run.firstHour) || !hour(run.lastHour) || run.firstHour > run.lastHour || typeof run.rosterUnknown !== 'boolean'
      || !Array.isArray(run.counts) || run.counts.length !== 2 || !run.counts.every(count => int(count) && count >= 0)) throw malformed();
    return { firstHour: run.firstHour, lastHour: run.lastHour, counts: run.counts as [number, number], rosterUnknown: run.rosterUnknown };
  });
  const reads: CourseRead[] = [];
  let previous: ([number, string][] | null)[] = [null, null];
  for (const read of battle.losses) {
    const last = reads.at(-1);
    if (!object(read) || !hour(read.hour) || typeof read.terminal !== 'boolean' || last && (read.hour < last.hour || last.terminal)) throw malformed();
    const unread = read.sides;
    if (typeof unread === 'string') { reads.push({ hour: read.hour, terminal: read.terminal, sides: unread }); previous = [null, null]; continue; }
    if (!Array.isArray(unread) || unread.length !== 2) throw malformed();
    const sides = unread.map((side, index): CourseSide => {
      if (!object(side) || !(typeof side.manpower === 'string' || int(side.manpower))) throw malformed();
      if (typeof side.unknown === 'string') { previous[index] = null; return { manpower: side.manpower as number | string, equipment: side.unknown }; }
      let list: [number, string][];
      if (pairs(side.variants, designs.length)) list = side.variants;
      else if (previous[index] && pairs(side.changed, previous[index]!.length)) {
        list = [...previous[index]!];
        for (const [at, value] of side.changed) list[at] = [list[at][0], value];
      } else throw malformed();
      if (new Set(list.map(([design]) => design)).size !== list.length) throw malformed();
      previous[index] = list;
      return { manpower: side.manpower as number | string, equipment: new Map(list.map(([design, value]) => [design, BigInt(value)])) };
    }) as [CourseSide, CourseSide];
    reads.push({ hour: read.hour, terminal: read.terminal, sides });
  }
  return { id: battle.id as string, runs, reads, designs, contentWarnings: v.contentWarnings };
}

export function validateLandBattleTerrain(v: unknown): { provinces: LandTerrain[]; warnings: string[] } {
  if (!object(v) || !Array.isArray(v.provinces) || !texts(v.warnings) || !v.provinces.every(row => object(row) && int(row.provinceId) && row.provinceId >= 0
    && (row.name === null ? typeof row.missing === 'string' : typeof row.name === 'string' && row.missing === null))) throw new Error('The land battle terrain reply is malformed.');
  return v as { provinces: LandTerrain[]; warnings: string[] };
}

export interface HeldTerrain { client: object; scope: string; names: Map<number, LandTerrain> }
export function heldTerrain(held: HeldTerrain | null, client: object, source: { id: string; revision: string }, content: number): HeldTerrain {
  const scope = JSON.stringify([source.id, source.revision, content]);
  return held?.client === client && held.scope === scope ? held : { client, scope, names: new Map() };
}

export function missingTerrain(held: HeldTerrain, drawn: readonly number[]): number[] {
  return [...new Set(drawn)].filter(province => !held.names.has(province)).sort((a, b) => a - b);
}

export function courseExtent(runs: readonly (readonly Run[])[]): [number, number] | null {
  let most = 1, any = false;
  for (const side of runs) for (const run of side) { any = true; if (run.value > most) most = run.value; }
  return any ? [0, most] : null;
}

export function drillChartHour(row: Pick<LandBattleRow, 'end' | 'settledHour'>, shownHour: number): number {
  const bound = row.end && 'hour' in row.end ? row.end.hour : row.settledHour;
  return bound !== undefined ? Math.min(shownHour, bound) : shownHour;
}

export function courseManpower(course: LandCourse, at: number): [Run[], Run[]] {
  const runs: [Run[], Run[]] = [[], []];
  const reads = course.reads.filter(read => read.hour <= at);
  for (const side of [0, 1]) {
    let broken = false;
    reads.forEach((read, index) => {
      const value = typeof read.sides === 'string' ? null : read.sides[side].manpower;
      if (typeof value !== 'number') { broken = true; return; }
      const lastHour = Math.max(read.hour, (reads[index + 1]?.hour ?? at + 1) - 1);
      runs[side].push({ firstHour: read.hour, lastHour, value, ...(broken ? { breakBefore: true } : {}) });
      broken = false;
    });
  }
  return runs;
}

export function courseDivisions(course: LandCourse, at: number): [Run[], Run[]] {
  const runs: [Run[], Run[]] = [[], []];
  for (const side of [0, 1]) {
    let previous: number | null = null;
    for (const run of course.runs) {
      if (run.firstHour > at) break;
      if (run.rosterUnknown) { previous = null; continue; }
      const breakBefore = previous === null ? runs[side].length > 0 : run.firstHour > previous + 1;
      runs[side].push({ firstHour: run.firstHour, lastHour: Math.min(run.lastHour, at), value: run.counts[side], ...(breakBefore ? { breakBefore: true } : {}) });
      previous = Math.min(run.lastHour, at);
    }
  }
  return runs;
}

export interface DesignLoss { name: string; raw: bigint; designs: number; variants: string[] }
export function courseEquipment(course: LandCourse, at: number): { hour: number; terminal: boolean; sides: [DesignLoss[] | string, DesignLoss[] | string]; zero: [number, number] } | null {
  let read: CourseRead | undefined;
  for (let index = course.reads.length - 1; index >= 0; index--) {
    if (course.reads[index].hour <= at) { read = course.reads[index]; break; }
  }
  if (!read) return null;
  if (typeof read.sides === 'string') return { hour: read.hour, terminal: read.terminal, sides: [read.sides, read.sides], zero: [0, 0] };
  const zero: [number, number] = [0, 0];
  const sides = read.sides.map((side, index) => {
    if (typeof side.equipment === 'string') return side.equipment;
    const byName = new Map<string, DesignLoss>();
    for (const [design, raw] of side.equipment) {
      const { name, variant } = course.designs[design];
      const label = name ?? 'Name not recorded';
      const entry = byName.get(label) ?? { name: label, raw: 0n, designs: 0, variants: [] };
      entry.raw += raw; entry.designs++; entry.variants.push(variant);
      byName.set(label, entry);
    }
    const listed = [...byName.values()];
    zero[index] = listed.filter(entry => entry.raw === 0n).length;
    return listed.filter(entry => entry.raw !== 0n).sort((a, b) => (b.raw > a.raw ? 1 : b.raw < a.raw ? -1 : 0) || a.name.localeCompare(b.name));
  }) as [DesignLoss[] | string, DesignLoss[] | string];
  return { hour: read.hour, terminal: read.terminal, sides, zero };
}

export const wholeEquipment = (raw: bigint) => formatFixed(raw, 5, 0);
export const exactEquipment = (raw: bigint) => formatAmount(raw, 5, 5);

export function lengthText(hours: number, long: boolean): string {
  const days = Math.floor(hours / 24), rest = hours % 24;
  const unit = (count: number, one: string, many: string) => `${usNumber.format(count)} ${count === 1 ? one : many}`;
  return days === 0 ? unit(hours, 'hour', 'hours')
    : long && rest ? `${unit(days, 'day', 'days')} ${unit(rest, 'hour', 'hours')}` : unit(days, 'day', 'days');
}

export interface SideLook { countries: { tag: string; hour: number; name: string; flag?: string; flagReason?: string; flagDetail?: string; flagPending?: boolean }[]; name: string | null; unknown: string | null }
export function sideLook(side: BattleSide, country: BattleCountryLabel): SideLook {
  const countries = side.countries.map(part => ({ tag: part.tag, hour: part.hour, ...country(part.tag, part.hour) }));
  return { countries, name: countries.length === 1 && !side.countriesUnknown ? countries[0].name : null, unknown: side.countriesUnknown ?? null };
}

export type BattleOutcome = { kind: 'ongoing' } | { kind: 'won'; side: number } | { kind: 'unknown'; text: string };
export function battleOutcome(row: LandBattleRow): BattleOutcome {
  if (row.end === null) return { kind: 'ongoing' };
  if ('side' in row.winner) return { kind: 'won', side: row.winner.side };
  if ('unknown' in row.end) return { kind: 'unknown', text: row.end.unknown };
  return { kind: 'unknown', text: row.winner.unknown };
}

export function lossNote(row: LandBattleRow): string | undefined {
  if (!row.lossRead) return undefined;
  return `${row.lossRead.terminal ? 'Final losses' : 'Losses so far'}, read ${formatDateTime(row.lossRead.hour)}`;
}

export const peakText = (peak: { value: number; hour: number } | null) =>
  peak === null ? 'Not recorded' : `${usNumber.format(peak.value)}, ${dayLabel(peak.hour)}`;
export const peakValue = (row: LandBattleRow): BattleValue => row.peakUnknown ? { label: 'Peak size', value: null, unknown: 'Roster not recorded' }
  : { label: 'Peak size', value: row.peak.value, atLeast: row.start?.bound === 'by' || row.end !== null && 'unknown' in row.end };

const SIDES = ['Attackers', 'Defenders'] as const;
const placeName = (row: LandBattleRow, pending: boolean) => row.place.name ?? (row.provinceId === null ? 'Province not recorded' : pending ? 'Loading…' : 'Unnamed province');

export function winners(look: SideLook, side: string): string {
  if (look.unknown) return side;
  const names = look.countries.map(part => part.name);
  if (names.length === 0) return side;
  if (names.length <= 2) return names.join(' and ');
  return `${names[0]} and ${names.length - 1} more`;
}

export function battleTooltip(row: LandBattleRow, looks: readonly SideLook[], terrain: string | null, colours: readonly string[], pending: boolean): TooltipContent {
  const rows: TooltipRow[] = [];
  if (terrain) rows.push({ label: 'Terrain', value: terrain });
  rows.push({ label: 'Started', value: battleDate(row.start, 'Not recorded') });
  rows.push({ label: 'Ended', value: battleDate(row.end, 'Ongoing') });
  if (row.length.value !== null) rows.push(battleValueRow('Length', row.length, lengthText(row.length.value, true)));
  rows.push(battleValueRow('Most divisions at once', peakValue(row), row.peakUnknown ? 'Roster not recorded' : peakText(row.peak)));
  row.sides.forEach((side, index) => {
    rows.push({ label: SIDES[index], strong: true, divider: true, swatch: colours[index] });
    for (const part of looks[index].countries) rows.push({ label: part.name, countryFlag: part, indent: true });
    if (side.countriesUnknown) rows.push({ label: 'Countries not recorded', indent: true, tone: 'muted' });
    if (row.sidePeaks) rows.push({ label: 'Most divisions at once', value: peakText(row.sidePeaks[index]), indent: true });
    const count = side.counts[0];
    rows.push(count ? battleValueRow('Divisions at the last read', count, battleValueText(count), { indent: true }) : { label: 'Divisions at the last read', value: 'Not recorded', indent: true });
    for (const value of side.losses) {
      const raw = value.label === 'Equipment lost' ? row.lossRead?.equipmentRaw?.[index] : null;
      rows.push(battleValueRow(value.label, value, raw != null ? wholeEquipment(BigInt(raw)) : battleValueText(value), {
        indent: true, ...(value.value === null && raw == null ? { tone: 'muted' as const } : {}) }));
    }
    if (!side.losses.length) rows.push({ label: 'Losses', value: 'Not recorded', indent: true, tone: 'muted' });
  });
  const outcome = battleOutcome(row);
  rows.push({ label: 'Result', divider: true, value: outcome.kind === 'ongoing' ? 'Ongoing' : outcome.kind === 'unknown' ? outcome.text
    : `${winners(looks[outcome.side], SIDES[outcome.side])} won` });
  return { title: placeName(row, pending), rows, note: lossNote(row) };
}

export const lossWidth = (value: number, larger: number) => larger > 0 && value > 0 ? `${Math.max(3, value / larger * 50)}%` : '0';

export const sameKnownSides = (a: LandBattleRow, b: LandBattleRow) => {
  const [left, right] = [landManpowerSides(a), landManpowerSides(b)];
  return (left[0] === null) === (right[0] === null) && (left[1] === null) === (right[1] === null);
};

export function subLine(row: LandBattleRow): string | null {
  const hours = row.length.value;
  if (hours === null || row.end === null && hours < 24) return null;
  return markPartial(`${hours < 24 ? hours : `d${Math.floor(hours / 24)}`}`, battlePartial(row.length));
}

export function subText(row: LandBattleRow, terrain: string | null) {
  return [terrain, row.end !== null || row.length.value !== null && row.length.value >= 24
    ? row.length.value === null ? null : markPartial(lengthText(row.length.value, false), battlePartial(row.length)) : null].filter(Boolean).join(' · ');
}

interface BattleParts { values: readonly HTMLElement[]; bars: readonly HTMLElement[]; sub: HTMLElement | null;
  lost: readonly (number | null)[]; line: string | null; terrain: string | null | undefined }
const battlePartsCache = new WeakMap<HTMLElement, BattleParts>();
export function clearBattleParts(element: HTMLElement) { battlePartsCache.delete(element); }
function battleParts(element: HTMLElement): BattleParts {
  let held = battlePartsCache.get(element);
  if (!held || (held.sub && !element.contains(held.sub)) || (held.values[0] && !element.contains(held.values[0]))) {
    held = { values: [...element.querySelectorAll<HTMLElement>('.lb-losses .log-value')], bars: [...element.querySelectorAll<HTMLElement>('.lb-losses .lb-bar')],
      sub: element.querySelector<HTMLElement>('.lb-place small'), lost: [], line: null, terrain: undefined };
    battlePartsCache.set(element, held);
  }
  return held;
}

export function paintBattle(element: HTMLElement, row: LandBattleRow, terrain: string | null) {
  const held = battleParts(element);
  const { values, bars, sub } = held;
  const lost = landManpowerSides(row);
  if (values.length === 2 && (lost[0] !== held.lost[0] || lost[1] !== held.lost[1])) {
    held.lost = lost;
    const both = lost[0] !== null && lost[1] !== null;
    const larger = both ? Math.max(lost[0]!, lost[1]!) : 0;
    lost.forEach((value, side) => {
      if (value !== null && !values[side].classList.contains('is-muted') && values[side].firstElementChild === null) {
        const text = usNumber.format(value);
        if (values[side].textContent !== text) values[side].textContent = text;
      }
      if (both && bars.length === 2) {
        const width = lossWidth(value!, larger);
        if (bars[side].style.width !== width) bars[side].style.width = width;
      }
    });
  }
  const line = subLine(row);
  if (sub && (line !== held.line || terrain !== held.terrain)) {
    held.line = line; held.terrain = terrain;
    const text = subText(row, terrain);
    if (sub.textContent !== text) sub.textContent = text;
  }
}
