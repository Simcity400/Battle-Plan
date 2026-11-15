import { liveKey } from '../charts/live.ts';
import type { Live, LiveValues } from '../charts/live';
import type { LiveTexts } from '../charts/LiveText';
import { object } from './validation.ts';
import type { FactionDetailReply } from '../generated/dashboard';

const text = (value: unknown) => value === null || typeof value === 'string';
const count = (value: unknown) => Number.isSafeInteger(value) && (value as number) >= 0;
const listOf = (value: unknown, item: (entry: Record<string, unknown>) => boolean) => Array.isArray(value) && value.every(entry => object(entry) && item(entry));
const nullableList = (value: unknown, item: (entry: Record<string, unknown>) => boolean) => value === null || listOf(value, item);

function definition(entry: Record<string, unknown>): boolean {
  return ['token', 'name', 'description', 'icon', 'category', 'categoryName', 'reason'].every(field => text(entry[field]))
    && listOf(entry.bands, band => text(band.name) && (band.min === null || typeof band.min === 'number') && (band.max === null || typeof band.max === 'number'));
}

/** A `faction-details` reply, checked whole: every index names a definition it holds. */
export function validateFactionDetails(value: unknown): FactionDetailReply {
  if (!object(value) || typeof value.faction !== 'string' || typeof value.branchId !== 'string' || !text(value.logo)
    || !listOf(value.definitions, definition)) throw new Error('The faction details reply is malformed.');
  const definitions = (value.definitions as unknown[]).length;
  const index = (entry: unknown) => count(entry) && (entry as number) < definitions;
  if (!(value.manifest === null || index(value.manifest))
    || !listOf(value.goals, goal => count(goal.ordinal) && index(goal.definition) && Number.isInteger(goal.status) && Array.isArray(goal.activeSections))
    || !Array.isArray(value.completed) || !value.completed.every(index) || !Array.isArray(value.cancelled) || !value.cancelled.every(index)
    || !text(value.goalReason)
    || !nullableList(value.resources, resource => count(resource.ordinal) && ['token', 'name', 'icon', 'reason'].every(field => text(resource[field])))
    || !nullableList(value.manpower, entry => count(entry.ordinal) && text(entry.tag))
    || !nullableList(value.doctrineFolders, folder => count(folder.ordinal) && ['token', 'name', 'reason'].every(field => text(folder[field])))
    || !(value.researchCountries === null || Array.isArray(value.researchCountries) && value.researchCountries.every(tag => typeof tag === 'string'))
    || !listOf(value.refusals, refusal => typeof refusal.fact === 'string' && count(refusal.count) && typeof refusal.reason === 'string')
    || !Array.isArray(value.warnings) || !value.warnings.every(warning => typeof warning === 'string'))
    throw new Error('The faction details reply is malformed.');
  return value as unknown as FactionDetailReply;
}

/** What a followed reply renders on: all of it, its numbers being live cells. */
export const detailsStructure = (reply: FactionDetailReply): string => JSON.stringify(reply);

/** The live cells' keys. */
export const FACTION_ROW = 'faction';
export const projectionKey = liveKey(FACTION_ROW, 'projection');
export const manpowerTotalKey = liveKey(FACTION_ROW, 'manpower');
export const researchBaseKey = liveKey(FACTION_ROW, 'research');
export const goalProgressKey = (ordinal: number) => liveKey(`goal:${ordinal}`, 'progress');
export const resourceKey = (ordinal: number) => liveKey(`resource:${ordinal}`, 'amount');
export const manpowerKey = (ordinal: number) => liveKey(`manpower:${ordinal}`, 'value');

/** The game's saturated fixed-point cache, left in the progress of goals without ratio progress. */
export const FIXED5_MAX = 92233720124192;

/** A goal's progress from its live cache (a thousandth of a percent): its percentage, `unmeasured` when the game
 * measures none, or missing with the cell's reason. */
export type GoalProgress = { kind: 'percent'; percent: number } | { kind: 'unmeasured' } | { kind: 'missing'; reason: string };
export function goalProgress(values: LiveValues<number>, ordinal: number): GoalProgress {
  const key = goalProgressKey(ordinal);
  const raw = values.get(key);
  if (raw === undefined) return { kind: 'missing', reason: values.missing(key) || 'Not recorded' };
  if (raw === FIXED5_MAX) return { kind: 'unmeasured' };
  return Number.isFinite(raw) && Math.abs(raw) < 2 ** 53 ? { kind: 'percent', percent: raw / 1000 } : { kind: 'missing', reason: 'Not recorded' };
}

/** A live number, or why it is missing. */
export function liveNumber(values: LiveValues<number>, key: string): { value: number } | { reason: string } {
  const value = values.get(key);
  return value === undefined ? { reason: values.missing(key) || 'Not recorded' } : { value };
}

/** Texts painted from the live cells: `write` makes a key's text from the values the frame reads. */
export function liveTextsOf(live: Live<LiveValues<number>>, write: (values: LiveValues<number>, key: string) => string): LiveTexts {
  return { text: key => write(live.read(), key), subscribe: changed => live.subscribe(changed) };
}
