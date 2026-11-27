import { formatDateTime, isEngineHour } from '../../dashboard/calendar.ts';
import { IC_UNRECORDED } from './definition.ts';
import { int, object } from '../../dashboard/validation.ts';
import { formatAmount, formatFixed } from '../../charts/valueAxis.ts';
import type { Run } from '../../charts/series';
import type { ChartInlinePoint } from '../../families/time-chart/model.ts';

export interface LandDesign { name: string | null; variant: string }
export interface CourseSide { manpower: number | string; equipment: Map<number, bigint> | string }
export interface CourseRead { hour: number; terminal: boolean; sides: [CourseSide, CourseSide] | string }
export interface SideRead { hour: number; sides: [number | null, number | null] }
export interface CourseRun { firstHour: number; lastHour: number; counts: [number, number]; countsUnknown: [boolean, boolean]; countries: [string[], string[]]; countriesUnknown: [boolean, boolean]; leads: [CourseLead | null, CourseLead | null] }
export interface CourseLead { slot: number; tag: string }
export type CourseEnd = { hour: number; bound: 'exact' | 'by' } | { unknown: string } | null;
export interface LandCourse { id: string; end: CourseEnd; resultHour: number | null; runs: CourseRun[];
  reads: CourseRead[]; ic: SideRead[]; men: SideRead[]; menMissing: string | null; icMissing: string | null; designs: LandDesign[]; contentWarnings: string[] }
export interface LandTerrain { provinceId: number; name: string | null; missing: string | null }

const hour = (v: unknown): v is number => typeof v === 'number' && isEngineHour(v);
const texts = (v: unknown): v is string[] => Array.isArray(v) && v.every(text => typeof text === 'string');
const amount = (v: unknown): v is string => typeof v === 'string' && v.length <= 40 && /^-?(0|[1-9]\d*)$/.test(v);
const pairs = (v: unknown, size: number): v is [number, string][] => Array.isArray(v) && v.every(entry => Array.isArray(entry) && entry.length === 2
  && int(entry[0]) && entry[0] >= 0 && entry[0] < size && amount(entry[1]));
const malformed = () => new Error('The land battle course is malformed.');
const sidePair = (v: unknown, valid: (item: unknown) => boolean): v is [unknown, unknown] => Array.isArray(v) && v.length === 2 && v.every(item => item === null || valid(item));
const finite = (v: unknown) => typeof v === 'number' && Number.isFinite(v);

export function validateLandBattleCourse(v: unknown): LandCourse {
  if (!object(v) || !object(v.battle) || typeof v.battle.id !== 'string' || !Array.isArray(v.designs) || !texts(v.contentWarnings)) throw malformed();
  const designs: LandDesign[] = [];
  for (const design of v.designs) {
    if (!object(design) || !(design.name === null || typeof design.name === 'string') || typeof design.variant !== 'string') throw malformed();
    designs.push({ name: design.name as string | null, variant: design.variant });
  }
  const battle = v.battle;
  if (!Array.isArray(battle.runs) || !Array.isArray(battle.losses) || !Array.isArray(battle.men) || !Array.isArray(battle.ic)
    || !(battle.menMissing === null || typeof battle.menMissing === 'string')
    || !(battle.icMissing === null || typeof battle.icMissing === 'string')) throw malformed();
  const sideReads = (list: unknown[], valid: (item: unknown) => boolean): SideRead[] => list.map((read, index, all) => {
    if (!object(read) || !hour(read.hour) || !sidePair(read.sides, valid) || index > 0 && (all[index - 1] as { hour: number }).hour >= read.hour) throw malformed();
    return { hour: read.hour, sides: read.sides as [number | null, number | null] };
  });
  const men = sideReads(battle.men, int);
  const ic = sideReads(battle.ic, finite);
  const ending = battle.end;
  const end: CourseEnd = ending === null ? null
    : object(ending) && typeof ending.unknown === 'string' ? { unknown: ending.unknown }
      : object(ending) && hour(ending.hour) && (ending.bound === 'exact' || ending.bound === 'by') ? { hour: ending.hour, bound: ending.bound }
        : (() => { throw malformed(); })();
  const runs = battle.runs.map(run => {
    if (!object(run) || !hour(run.firstHour) || !hour(run.lastHour) || run.firstHour > run.lastHour
      || !Array.isArray(run.countsUnknown) || run.countsUnknown.length !== 2 || !run.countsUnknown.every(unknown => typeof unknown === 'boolean')
      || !Array.isArray(run.counts) || run.counts.length !== 2 || !run.counts.every(count => int(count) && count >= 0)
      || !Array.isArray(run.countries) || run.countries.length !== 2 || !run.countries.every(texts)
      || !Array.isArray(run.countriesUnknown) || run.countriesUnknown.length !== 2 || !run.countriesUnknown.every(known => typeof known === 'boolean')
      || !sidePair(run.leads, lead => object(lead) && Number.isSafeInteger(lead.slot) && (lead.slot as number) >= 0 && typeof lead.tag === 'string')) throw malformed();
    return { firstHour: run.firstHour, lastHour: run.lastHour, counts: run.counts as [number, number], countsUnknown: run.countsUnknown as [boolean, boolean],
      countries: run.countries as [string[], string[]], countriesUnknown: run.countriesUnknown as [boolean, boolean], leads: run.leads as [CourseLead | null, CourseLead | null] };
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
  const resultHour = object(battle.result) && hour(battle.result.hour) ? battle.result.hour : null;
  return { id: battle.id as string, end, resultHour, runs, reads, ic, men, menMissing: battle.menMissing as string | null, icMissing: battle.icMissing as string | null, designs, contentWarnings: v.contentWarnings };
}

export function validateLandBattleTerrain(v: unknown): { provinces: LandTerrain[]; warnings: string[] } {
  if (!object(v) || !Array.isArray(v.provinces) || !texts(v.warnings) || !v.provinces.every(row => object(row) && int(row.provinceId) && row.provinceId >= 0
    && (row.name === null ? typeof row.missing === 'string' : typeof row.name === 'string' && row.missing === null))) throw new Error('The land battle terrain reply is malformed.');
  return v as { provinces: LandTerrain[]; warnings: string[] };
}

export interface CourseSpan { readonly firstHour: number; readonly lastHour: number }

export function coursePoints(runs: readonly Run[], until: number): ChartInlinePoint[] {
  const points: ChartInlinePoint[] = [];
  runs.forEach((run, index) => {
    const previous = runs[index - 1];
    if (previous && run.breakBefore) points.push({ hour: previous.lastHour + 1, value: previous.value }, { hour: previous.lastHour + 1, value: null });
    points.push({ hour: run.firstHour, value: run.value });
  });
  const last = runs.at(-1);
  if (last) points.push({ hour: Math.min(last.lastHour + 1, until), value: last.value });
  if (last && last.lastHour < until) points.push({ hour: last.lastHour + 1, value: null });
  return points;
}

export function courseMissing(runs: readonly Run[], from: number, until: number): CourseSpan[] {
  const missing: CourseSpan[] = [];
  let next = from;
  for (const run of runs) {
    if (run.firstHour > until) break;
    if (run.firstHour > next) missing.push({ firstHour: next, lastHour: run.firstHour - 1 });
    next = Math.max(next, run.lastHour + 1);
  }
  if (next <= until) missing.push({ firstHour: next, lastHour: until });
  return missing;
}

export function sharedSpans(first: readonly CourseSpan[], second: readonly CourseSpan[]): CourseSpan[] {
  const shared: CourseSpan[] = [];
  let i = 0, j = 0;
  while (i < first.length && j < second.length) {
    const a = first[i]!, b = second[j]!;
    const firstHour = Math.max(a.firstHour, b.firstHour), lastHour = Math.min(a.lastHour, b.lastHour);
    if (firstHour <= lastHour) shared.push({ firstHour, lastHour });
    if (a.lastHour < b.lastHour) i++; else j++;
  }
  return shared;
}

export function withinCourse(series: readonly Run[], course: LandCourse): Run[] {
  const spans = [...course.runs.map(run => ({ firstHour: run.firstHour, lastHour: run.lastHour })),
    ...course.reads.filter(read => read.terminal).map(read => ({ firstHour: read.hour, lastHour: read.hour }))].sort((a, b) => a.firstHour - b.firstHour);
  const covered: CourseSpan[] = [];
  for (const span of spans) {
    const last = covered.at(-1);
    if (last && span.firstHour <= last.lastHour + 1) covered[covered.length - 1] = { firstHour: last.firstHour, lastHour: Math.max(last.lastHour, span.lastHour) };
    else covered.push(span);
  }
  const clipped: Run[] = [];
  for (const run of series) for (const span of covered) {
    const firstHour = Math.max(run.firstHour, span.firstHour), lastHour = Math.min(run.lastHour, span.lastHour);
    if (firstHour > lastHour) continue;
    const previous = clipped.at(-1);
    const piece: Run = { firstHour, lastHour, value: run.value };
    if (previous && (run.breakBefore || firstHour !== run.firstHour || previous.lastHour + 1 < firstHour)) piece.breakBefore = true;
    clipped.push(piece);
  }
  return clipped;
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
      if (run.countsUnknown[side]) { previous = null; continue; }
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

export type CourseFigure = { readonly value: number; readonly soFar: boolean } | { readonly missing: string };
export interface CourseSideFigures { readonly divisions: CourseFigure; readonly men: CourseFigure; readonly casualties: CourseFigure; readonly icLost: CourseFigure }

export const NOT_AT_THIS_HOUR = 'Not recorded at this hour';

function latest<T extends { hour: number }>(items: readonly T[], at: number): T | undefined {
  let found: T | undefined;
  for (const item of items) { if (item.hour > at) break; found = item; }
  return found;
}

export function courseFigures(course: LandCourse, at: number, final: boolean): [CourseSideFigures, CourseSideFigures] {
  const run = final ? course.runs.at(-1) : course.runs.find(span => span.firstHour <= at && at <= span.lastHour);
  const reach = final ? Number.MAX_SAFE_INTEGER : at;
  const read = latest(course.reads, reach);
  const ic = latest(course.ic, reach);
  const men = latest(course.men, reach);
  const until = (hour: number | undefined) => hour !== undefined && hour > at ? ` until ${formatDateTime(hour)}` : '';
  return ([0, 1] as const).map((side): CourseSideFigures => {
    if (!run) {
      const gap = { missing: NOT_AT_THIS_HOUR };
      return { divisions: gap, men: gap, casualties: gap, icLost: gap };
    }
    const divisions: CourseFigure = run.countsUnknown[side] ? { missing: 'Divisions not recorded' } : { value: run.counts[side], soFar: false };
    const strength: CourseFigure = men?.sides[side] != null ? { value: men.sides[side]!, soFar: false }
      : { missing: course.menMissing ?? 'Men not recorded: a division’s manpower was not read' };
    if (!read) {
      const lostFrom = course.reads.find(each => typeof each.sides !== 'string' && typeof each.sides[side].manpower === 'number')?.hour;
      const spentFrom = course.ic.find(each => each.sides[side] != null)?.hour;
      return { divisions, men: strength, casualties: { missing: `Casualties not recorded${until(lostFrom)}` }, icLost: { missing: `IC lost not recorded${until(spentFrom)}` } };
    }
    const soFar = !read.terminal;
    const lost = typeof read.sides === 'string' ? read.sides : read.sides[side].manpower;
    const spent = ic?.sides[side] ?? null;
    return {
      divisions, men: strength,
      casualties: typeof lost === 'number' ? { value: lost, soFar } : { missing: lost },
      icLost: spent === null ? { missing: course.icMissing ?? IC_UNRECORDED } : { value: spent, soFar },
    };
  }) as [CourseSideFigures, CourseSideFigures];
}

export function runCovering(course: LandCourse, at: number): CourseRun | null {
  return course.runs.find(span => span.firstHour <= at && at <= span.lastHour) ?? null;
}
