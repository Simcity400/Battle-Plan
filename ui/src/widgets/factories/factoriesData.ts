/** The Factories widget's data: the `country-factories` reply checked, and each
 * country's factories at an hour as the chosen count reads them. Every value is a
 * whole number of factories as the game shows it (the product truncates the
 * engine's fixed5 totals as the game does); the counts are the formulas below. */
import type { CountryRaceEntry } from '../../charts/race/countryRace';
import type { Run } from '../../charts/series';
import { runAt, runIndexAt, stretchStarts } from '../../charts/series.ts';
import { shownRuns } from '../../charts/race/lines.ts';
import { compareTags } from '../../dashboard/tagOrder.ts';
import { validCountryIdentityRuns } from '../../dashboard/countryIdentity.ts';
import { readRunAt } from '../../dashboard/reads.ts';
import { COUNT_MODES } from '../../dashboard/factoryMeasure.ts';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import type { CountryFactoriesReply, CountryIdentityRun, FactoriesParams, FactoryCount, FactoryGroupInfo, FactoryGroupKey, FactoryPart, FactoryReadRun, FactoryRun, HourSpan } from '../../generated/dashboard';

/** Which count the widget shows (user decision, 29 Sep: free after withheld by default). */
export type CountMode = FactoryCount;

export { COUNT_MODES };

/** The civilian consumers, in the order the game runs them, and the transfers out. */
const CONSUMERS: readonly FactoryPart[] = ['consumerGoods', 'contractPayments', 'specialProjects', 'licenseImports', 'tradeImports'];

/** Checks the reply's shape: known fields, runs in hour order with one value per field. */
export function validateFactoriesReply(data: unknown): CountryFactoriesReply {
  const reply = data as CountryFactoriesReply;
  const ok = typeof reply === 'object' && reply != null && Array.isArray(reply.fields) && Array.isArray(reply.groups)
    && Array.isArray(reply.series) && Array.isArray(reply.warnings)
    && reply.groups.every(group => typeof group.key === 'string' && typeof group.flag === 'string' && Array.isArray(group.buildings))
    && reply.series.every(series => typeof series.tag === 'string' && Array.isArray(series.runs)
      && validCountryIdentityRuns(series.identities)
      && (series.reads === undefined || Array.isArray(series.reads) && series.reads.every((read, index) => read != null
        && Number.isInteger(read.firstHour) && Number.isInteger(read.lastHour) && Number.isInteger(read.readHour)
        && read.firstHour <= read.lastHour && read.readHour <= read.firstHour && typeof read.hourly === 'boolean'
        && (!read.hourly || read.readHour === read.firstHour) && (index === 0 || series.reads![index - 1].lastHour < read.firstHour)))
      && Array.isArray(series.segments) && series.segments.every((span, index) =>
        Number.isInteger(span.firstHour) && Number.isInteger(span.lastHour) && span.firstHour <= span.lastHour
        && (index === 0 || series.segments[index - 1].lastHour < span.firstHour))
      && series.runs.every((run, index) => Number.isInteger(run.firstHour) && run.firstHour <= run.lastHour
        && Array.isArray(run.values) && run.values.length === reply.fields.length && run.values.every(Number.isInteger)
        && (index === 0 || series.runs[index - 1].lastHour < run.firstHour))
      && (series.membership === null || (Array.isArray(series.membership) && series.membership.every((span, index) =>
        span.firstHour <= span.lastHour && (index === 0 || series.membership![index - 1].lastHour < span.firstHour)))));
  if (!ok) throw new Error('The factories reply is malformed.');
  return reply;
}

/** The engine's factory groups, as the product names them: which of them a top-N
 * request sums (none: all). They are engine structures, not content
 * (`CProductionStatus` +0x2b0, +0x310, +0x370, rebuilt by
 * `RecalculateFactoryTotals@140e57f10`), so a mod cannot add one; its factory types
 * count in them through their flags and are named from content. Kept complete by
 * the type below. */
const GROUP_KEYS = { military: true, naval: true, civilian: true } as const satisfies Record<FactoryGroupKey, true>;
const isGroupKey = (token: string): token is FactoryGroupKey => Object.hasOwn(GROUP_KEYS, token);

/** What the widget asks the product for: the top N by the count shown, of the chosen
 * types (all when none are), or the chosen countries. */
export function factoriesRequest(branchId: string, resolved: { tags: readonly string[]; top?: number }, mode: CountMode, chosen: readonly string[]): FactoriesParams {
  if (resolved.top != null) return { branchId, tags: [], top: { count: resolved.top, mode, groups: chosen.filter(isGroupKey) } };
  return { branchId, tags: [...resolved.tags], top: null };
}

/** One country's runs, where each unbroken stretch of them starts, and for a top-N
 * reply the hours it was among the top N (its runs are cut to them). */
export interface FactorySeriesModel { runs: readonly FactoryRun[]; starts: readonly number[]; segments: readonly HourSpan[];
  reads: readonly FactoryReadRun[]; identities: readonly CountryIdentityRun[]; membership: readonly HourSpan[] | null }

/** A reply ready for lookups, built once per reply. */
export interface FactoryModel {
  reply: CountryFactoriesReply;
  /** Each field's position in a run's values, by `group:part`. */
  index: ReadonlyMap<string, number>;
  series: ReadonlyMap<string, FactorySeriesModel>;
  tags: readonly string[];
}

const fieldKey = (group: FactoryGroupKey, part: FactoryPart) => `${group}:${part}`;

export function factoryModel(reply: CountryFactoriesReply): FactoryModel {
  const index = new Map(reply.fields.map((field, position) => [fieldKey(field.group, field.part), position]));
  const series = new Map(reply.series.map(item => [item.tag, { runs: item.runs, starts: stretchStarts(item.runs), segments: item.segments,
    reads: item.reads ?? [], identities: item.identities ?? [], membership: item.membership }]));
  return { reply, index, series, tags: reply.series.map(item => item.tag) };
}

/** Equal fresh values retain a fresh confirmation; carried values retain their read. */
export function factoryReadAt(model: FactoryModel, tag: string, hour: number): number | null {
  return readRunAt(model.series.get(tag)?.reads ?? [], Math.floor(hour));
}

/** A country's identity/data segment from the full scan, independent of whether
 * it is shown or in the top N at this hour. */
export function factorySegmentAt(model: FactoryModel | null, tag: string, hour: number): number | null {
  return runAt(model?.series.get(tag)?.segments ?? [], Math.floor(hour))?.firstHour ?? null;
}

/** A recorded part of a group in a run's values; 0 where the reply has no such field. */
export function part(model: FactoryModel, values: readonly number[], group: FactoryGroupKey, name: FactoryPart): number {
  const position = model.index.get(fieldKey(group, name));
  return position == null ? 0 : values[position];
}

/** Factories withheld before any are free: every civilian consumer and both
 * transfers out for the civilian group, the transfers out for the others. */
export function withheld(model: FactoryModel, values: readonly number[], group: FactoryGroupKey): number {
  const out = part(model, values, group, 'toOverlord') + part(model, values, group, 'toOthers');
  return group === 'civilian' ? out + CONSUMERS.reduce((sum, name) => sum + part(model, values, group, name), 0) : out;
}

/** A group's count under a mode, and its damaged factories:
 * - free: built − damaged − withheld;
 * - built: the game's Total;
 * - top bar: built − damaged − given to the overlord − given to other countries. */
export function groupCount(model: FactoryModel, values: readonly number[], group: FactoryGroupKey, mode: CountMode): { count: number; damaged: number } {
  const total = part(model, values, group, 'total');
  const damaged = part(model, values, group, 'damaged');
  const out = part(model, values, group, 'toOverlord') + part(model, values, group, 'toOthers');
  const count = mode === 'built' ? total : mode === 'topBar' ? total - damaged - out : total - damaged - withheld(model, values, group);
  return { count, damaged };
}

/** A country at an hour: each chosen group's count and damaged factories, their sum,
 * the run it was read from and the unbroken stretch it belongs to. Null when not
 * recorded at that hour. */
export interface FactoriesAt {
  total: number;
  groups: readonly { key: FactoryGroupKey; count: number; damaged: number }[];
  run: FactoryRun;
  stretch: number;
}

export function factoriesAt(model: FactoryModel, tag: string, hour: number, groups: readonly FactoryGroupKey[], mode: CountMode): FactoriesAt | null {
  const series = model.series.get(tag);
  if (!series) return null;
  const position = runIndexAt(series.runs, Math.floor(hour));
  if (position < 0) return null;
  const run = series.runs[position];
  const counts = groups.map(key => ({ key, ...groupCount(model, run.values, key, mode) }));
  return { total: counts.reduce((sum, item) => sum + item.count, 0), groups: counts, run, stretch: series.starts[position] };
}

/** The countries ranked by the count shown at an hour, highest first, ties broken by
 * tag exactly as the product breaks them (`compareTags`), at most `count`; a country
 * not recorded then is left out, never ranked as zero. */
export function rankFactories(model: FactoryModel, candidates: readonly string[], hour: number, groups: readonly FactoryGroupKey[], mode: CountMode,
  count: number): { tag: string; at: FactoriesAt }[] {
  return candidates.flatMap(tag => { const at = factoriesAt(model, tag, hour, groups, mode); return at ? [{ tag, at }] : []; })
    .sort((a, b) => b.at.total - a.at.total || compareTags(a.tag, b.tag))
    .slice(0, Math.max(0, count));
}

/** Recorded counts and damage for the shared race. Built includes its damaged
 * factories; the shared view hatches that amount inside each count. */
export function raceEntries(ranked: readonly { tag: string; at: FactoriesAt }[], look: { mode: CountMode; split: boolean }): CountryRaceEntry[] {
  return ranked.map(({ tag, at }) => ({
    tag, value: at.total, label: formatCount(at.total), stretch: at.stretch,
    ...(look.split ? { parts: at.groups.map(group => ({ id: group.key, value: group.count,
      hatched: look.mode === 'built' ? group.damaged : 0, hatchInside: true })) }
      : { hatched: look.mode === 'built' ? at.groups.reduce((sum, group) => sum + group.damaged, 0) : 0 }),
  }));
}

/** A country's count over time as plotting runs: equal neighbours merge, and a run
 * that starts a new line (after a gap or another country) keeps its break. */
export function countRuns(model: FactoryModel, tag: string, groups: readonly FactoryGroupKey[], mode: CountMode, group?: FactoryGroupKey): Run[] {
  const runs: Run[] = [];
  for (const run of model.series.get(tag)?.runs ?? []) {
    const value = (group ? [group] : groups).reduce((sum, key) => sum + groupCount(model, run.values, key, mode).count, 0);
    const previous = runs.at(-1);
    if (previous && !run.startsSegment && previous.lastHour + 1 === run.firstHour && previous.value === value) previous.lastHour = run.lastHour;
    else runs.push({ firstHour: run.firstHour, lastHour: run.lastHour, value, ...(run.startsSegment ? { breakBefore: true } : {}) });
  }
  return runs;
}

/** One country's count over time as a line, cut as `shownRuns` cuts. */
export function lineRuns(model: FactoryModel, tag: string, groups: readonly FactoryGroupKey[], mode: CountMode, membership: readonly HourSpan[] | null, range: HourSpan): Run[] {
  return shownRuns(countRuns(model, tag, groups, mode), membership, range);
}

/** One country's types as signed runs to stack, each cut as `shownRuns` cuts. */
export function mixBands(model: FactoryModel, tag: string, groups: readonly FactoryGroupKey[], mode: CountMode, membership: readonly HourSpan[] | null, range: HourSpan): Run[][] {
  return groups.map(group => shownRuns(countRuns(model, tag, groups, mode, group), membership, range));
}

/** A group's name: the game's names of the content buildings its flag counts,
 * else their content names, else the flag itself. */
export function groupName(group: FactoryGroupInfo): string {
  if (!group.buildings.length) return group.flag;
  return group.buildings.map(building => building.name ?? building.token).join(' · ');
}

/** The types the settings offer: every group the reply names, as its content names it. */
export function typeChoices(reply: CountryFactoriesReply | undefined): { value: string; label: string }[] {
  return (reply?.groups ?? []).map(group => ({ value: group.key, label: groupName(group) }));
}

/** The groups the widget counts: the chosen ones (in the reply's order), else all. */
export function chosenGroups(model: FactoryModel, chosen: readonly string[]): FactoryGroupKey[] {
  const all = model.reply.groups.map(group => group.key);
  const picked = all.filter(key => chosen.includes(key));
  return picked.length ? picked : all;
}

const signed = (value: number) => value > 0 ? `+${value.toLocaleString('en-US')}` : value < 0 ? `−${Math.abs(value).toLocaleString('en-US')}` : '0';
/** A signed change and its tone: what it does to the count, so a recorded deduction
 * of −1 (a negative damaged total) reads +1, as it adds one. */
const change = (effect: number): { text: string; tone?: TooltipRow['tone'] } =>
  ({ text: signed(effect), ...(effect < 0 ? { tone: 'negative' as const } : effect > 0 ? { tone: 'positive' as const } : {}) });
export const formatCount = (value: number) => value < 0 ? `−${Math.abs(value).toLocaleString('en-US')}` : value.toLocaleString('en-US');

/** The game's own breakdown of one group (its construction and top-bar tooltips):
 * Total, Owned, From occupation and what was added; then Damaged and what is
 * withheld; then the count shown. `all` keeps lines that are zero. */
export function groupRows(model: FactoryModel, values: readonly number[], group: FactoryGroupInfo, mode: CountMode, color: string | undefined, all: boolean): TooltipRow[] {
  const key = group.key;
  const get = (name: FactoryPart) => part(model, values, key, name);
  const { count } = groupCount(model, values, key, mode);
  const line = (label: string, value: number, text: string, tone?: TooltipRow['tone']): TooltipRow[] =>
    all || value !== 0 ? [{ label, value: text, indent: true, ...(tone ? { tone } : {}) }] : [];
  // An addition (From …) keeps its sign; a deduction (Damaged, withheld) is shown as
  // what it does to the count: its value negated, never a doubled minus.
  const added = (label: string, value: number) => line(label, value, signed(value));
  const deducted = (label: string, value: number) => { const effect = change(-value); return line(label, value, effect.text, effect.tone); };
  const civilian = key === 'civilian';
  return [
    { label: groupName(group), value: formatCount(count), strong: true, divider: true, ...(color ? { swatch: color } : {}) },
    { label: 'Total', value: formatCount(get('total')), indent: true },
    ...line('Owned', get('owned'), formatCount(get('owned'))),
    ...added('From occupation', get('controlled') - get('owned')),
    ...added('From subjects', get('fromSubjects')),
    ...added('From other countries', get('fromOthers')),
    ...(civilian ? added('Trade exports', get('tradeExports')) : []),
    ...(civilian ? added('License exports', get('licenseExports')) : []),
    ...deducted('Damaged', get('damaged')),
    ...(civilian ? deducted('Consumer Goods', get('consumerGoods')) : []),
    ...(civilian ? deducted('Trade imports', get('tradeImports')) : []),
    ...(civilian ? deducted('National Projects', get('specialProjects')) : []),
    ...(civilian ? deducted('License imports', get('licenseImports')) : []),
    ...(civilian ? deducted('Market Contracts', get('contractPayments')) : []),
    ...deducted('Given to Overlord', get('toOverlord')),
    ...deducted('Given to other countries', get('toOthers')),
  ];
}

/** The tooltip for a country at a read, fullest first: every line of every group,
 * then only the lines that are not zero, then the groups' counts alone. */
export function* factoryTooltips(model: FactoryModel, name: string, at: FactoriesAt, date: string, mode: CountMode,
  colors: ReadonlyMap<string, string>): Generator<TooltipContent> {
  const groups = at.groups.map(item => model.reply.groups.find(group => group.key === item.key)!).filter(Boolean);
  const label = COUNT_MODES.find(item => item.value === mode)!.label;
  const total: TooltipRow = { label, value: formatCount(at.total), strong: true, divider: true };
  const base = { title: name, subtitle: `${date} · ${label}` };
  for (const all of [true, false]) {
    yield { ...base, rows: [...groups.flatMap(group => groupRows(model, at.run.values, group, mode, colors.get(group.key), all)), ...(groups.length > 1 ? [total] : [])] };
  }
  yield { ...base, rows: [...at.groups.map(item => {
    const group = model.reply.groups.find(entry => entry.key === item.key)!;
    return { label: groupName(group), value: formatCount(item.count), ...(colors.get(item.key) ? { swatch: colors.get(item.key) } : {}) };
  }), ...(groups.length > 1 ? [total] : [])] };
}
