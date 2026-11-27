import { isEngineHour } from '../../dashboard/calendar.ts';
import { object } from '../../dashboard/validation.ts';
import type { NavalBattleCourseReply, NavalBattleStatus, NavalCourseRead } from '../../generated/dashboard';

export type NavalFigure = { readonly value: number; readonly soFar: boolean } | { readonly missing: string };
export interface NavalSideFigures { readonly ships: NavalFigure; readonly convoys: NavalFigure; readonly sunk: NavalFigure; readonly icLost: NavalFigure }

export const NOT_AT_THIS_HOUR = 'Not recorded at this hour';

const STATUSES: readonly NavalBattleStatus[] = ['ongoing', 'attackerWon', 'defenderWon', 'unknown'];
const malformed = () => new Error('The naval battle course is malformed.');
const hour = (v: unknown): v is number => typeof v === 'number' && isEngineHour(v);
const pair = (v: unknown, valid: (item: number) => boolean) => Array.isArray(v) && v.length === 2 && v.every(item => item === null || typeof item === 'number' && valid(item));
const count = (item: number) => Number.isInteger(item) && item >= 0;
const amount = (item: number) => Number.isFinite(item) && item >= 0;

export function validateNavalBattleCourse(v: unknown): NavalBattleCourseReply {
  if (!object(v) || typeof v.branchId !== 'string' || !Array.isArray(v.reads) || !Array.isArray(v.missing) || !Array.isArray(v.warnings)) throw malformed();
  v.reads.forEach((read: unknown, index: number, all: unknown[]) => {
    if (!object(read) || !hour(read.hour) || !STATUSES.includes(read.result as NavalBattleStatus)
      || !pair(read.ships, count) || !pair(read.convoys, count) || !pair(read.shipsSunk, count) || !pair(read.convoysSunk, count) || !pair(read.icLost, amount) || typeof read.resulted !== 'boolean'
      || index > 0 && (all[index - 1] as { hour: number }).hour >= read.hour) throw malformed();
  });
  if (!v.missing.every((gap: unknown) => object(gap) && hour(gap.firstHour) && hour(gap.lastHour) && gap.firstHour <= gap.lastHour && typeof gap.reason === 'string')) throw malformed();
  return v as NavalBattleCourseReply;
}

function readAt(course: NavalBattleCourseReply, at: number): NavalCourseRead | undefined {
  let found: NavalCourseRead | undefined;
  for (const read of course.reads) {
    if (read.hour > at) break;
    found = read;
  }
  return found;
}

export function navalCourseFigures(course: NavalBattleCourseReply, at: number): [NavalSideFigures, NavalSideFigures] {
  const read = readAt(course, at);
  const gap = read?.resulted ? undefined : course.missing.find(span => span.firstHour <= at && at <= span.lastHour);
  const side = (index: 0 | 1): NavalSideFigures => {
    if (!read || gap) {
      const missing = { missing: gap?.reason ?? NOT_AT_THIS_HOUR };
      return { ships: missing, convoys: missing, sunk: missing, icLost: missing };
    }
    const soFar = !read.resulted;
    const figure = (value: number | null, missing: string, partial: boolean): NavalFigure => value === null ? { missing } : { value, soFar: partial };
    return {
      ships: figure(read.ships[index], 'Ships in the battle not recorded at this hour', false),
      convoys: figure(read.convoys[index], 'Convoys in the battle not recorded at this hour', false),
      sunk: figure(read.shipsSunk[index] === null || read.convoysSunk[index] === null ? null : read.shipsSunk[index]! + read.convoysSunk[index]!, 'Ships sunk not recorded at this hour', soFar),
      icLost: figure(read.icLost[index], 'IC lost not recorded at this hour', soFar),
    };
  };
  return [side(0), side(1)];
}

export function navalCourseGaps(course: NavalBattleCourseReply, first: number, last: number): { first: number; last: number }[] {
  return course.missing.filter(gap => gap.firstHour <= last && gap.lastHour >= first)
    .map(gap => ({ first: Math.max(first, gap.firstHour), last: Math.min(last, gap.lastHour) }));
}
