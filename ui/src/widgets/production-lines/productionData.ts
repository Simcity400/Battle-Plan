import { contentTitle } from '../../dashboard/contentTitles.ts';
import { runAt } from '../../charts/series.ts';
import { entryAt } from '../../dashboard/reads.ts';
import { campaignToken } from '../../dashboard/chosen.ts';
import { contentKey, setToken } from '../../dashboard/options.ts';
import type { BandStep, TimelineLane, TimelineSpan } from '../../charts/timeline/layout';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { formatAmount, formatFixed } from '../../charts/valueAxis.ts';
import { gbNumber } from '../../charts/numberFormat.ts';
import type { LineDetail, LineHistory, LineRun, LineStep, OrganisationName, OutputRun, ProductionLineDetailReply, ProductionLinesReply, ProductionLineBreakdownReply, ProductionModifierRun,
  VariantName } from '../../generated/dashboard';

/** A country's production lines, from the recorded `ProductionLines`,
 * `ProductionLineOutput` and `IndustrialOrganisations` (see
 * `crates/dashboard/src/products/production_lines.rs` for what each field is and
 * the evidence): IC per unit is `cost`, output per week is the product's figure by
 * the game's weekly formula, efficiency is the recorded Q15 value rounded as the row
 * shows it, and a line's total is the units the collector counted it deliver.
 *
 * `production-lines` holds each
 * line's life (its factory steps and final count) with the names, icons and hours
 * recorded; `production-line-detail` holds every value read in one window of
 * `detailHours`, which the live table and the history's tooltip ask for as they
 * need it. `production-line-breakdown` supplies one line's selected factor over
 * the branch. Full histories grow with recorded changes; the transport bounds
 * each reply. */

const RAW = /^-?\d{1,20}$/;
const PART_RAW = /^-?\d+$/;
const HOUR = (value: unknown) => Number.isInteger(value);
const KINDS = ['military', 'naval', 'shipRefit', 'railwayGun'];
const NULLABLE = (value: unknown) => value === null || typeof value === 'string';

const ordered = <T extends { firstHour: number; lastHour: number }>(runs: readonly T[]) =>
  runs.every((run, index) => run.firstHour <= run.lastHour && (index === 0 || runs[index - 1].lastHour < run.firstHour));

/** Native line keys belong to one campaign branch and producing country. */
export const productionLineToken = (campaign: string, branch: string, tag: string, key: string) =>
  setToken(campaignToken(campaign, JSON.stringify([branch, tag, key])));

export function productionLineKey(campaign: string, branch: string, tag: string, token: unknown): string | null {
  if (typeof token !== 'string') return null;
  const value = contentKey(token); const prefix = campaignToken(campaign, '');
  if (!value.startsWith(prefix)) return null;
  try {
    const parts: unknown = JSON.parse(value.slice(prefix.length));
    return Array.isArray(parts) && parts.length === 3 && parts[0] === branch && parts[1] === tag
      && typeof parts[2] === 'string' && parts[2] !== '' ? parts[2] : null;
  } catch { return null; }
}

const isStep = (step: LineStep) => HOUR(step.firstHour) && HOUR(step.lastHour) && typeof step.startsSegment === 'boolean'
  && RAW.test(step.variant) && HOUR(step.activeFactories) && HOUR(step.requestedFactories);

const isRun = (run: LineRun) => HOUR(run.firstHour) && HOUR(run.lastHour) && typeof run.startsSegment === 'boolean'
  && RAW.test(run.variant) && RAW.test(run.mio) && RAW.test(run.costRaw) && RAW.test(run.speedRaw) && RAW.test(run.efficiencyRaw)
  && (run.weeklyRaw === null || RAW.test(run.weeklyRaw))
  && HOUR(run.activeFactories) && HOUR(run.requestedFactories) && HOUR(run.maxFactories) && HOUR(run.priority) && HOUR(run.amount);

const isOutput = (run: OutputRun) => HOUR(run.firstHour) && HOUR(run.lastHour) && run.firstHour <= run.lastHour && HOUR(run.sinceHour)
  && typeof run.wholeLife === 'boolean' && Array.isArray(run.parts)
  && run.parts.every(part => RAW.test(part.variant) && RAW.test(part.mio) && RAW.test(part.units) && typeof part.exact === 'boolean');

export const isModifierRun = (run: ProductionModifierRun) => HOUR(run.firstHour) && HOUR(run.lastHour)
  && Array.isArray(run.warnings) && run.warnings.every(warning => typeof warning === 'string')
  && Array.isArray(run.groups) && run.groups.every(group => typeof group.id === 'string' && typeof group.label === 'string'
    && typeof group.note === 'string' && typeof group.unit === 'string' && group.decimals === 5
    && (group.totalRaw === null || typeof group.totalRaw === 'string' && RAW.test(group.totalRaw)) && Array.isArray(group.parts)
    && group.parts.every(part => typeof part.id === 'string' && NULLABLE(part.label) && typeof part.raw === 'string' && PART_RAW.test(part.raw) && NULLABLE(part.unknown))
    && new Set(group.parts.map(part => part.id)).size === group.parts.length
    && (group.totalRaw === null || group.parts.reduce((sum, part) => sum + BigInt(part.raw), 0n) === BigInt(group.totalRaw)));

export function validateBreakdownReply(data: unknown): ProductionLineBreakdownReply {
  const reply = data as ProductionLineBreakdownReply;
  if (!reply || typeof reply.branchId !== 'string' || typeof reply.tag !== 'string' || typeof reply.key !== 'string'
    || !Array.isArray(reply.groups) || !reply.groups.every(group => typeof group.id === 'string' && typeof group.label === 'string')
    || !Array.isArray(reply.reads) || !reply.reads.every((hour, index) => HOUR(hour) && (index === 0 || reply.reads[index - 1] < hour))
    || !Array.isArray(reply.runs) || !ordered(reply.runs) || !reply.runs.every(isModifierRun)
    || !Array.isArray(reply.warnings) || !reply.warnings.every(warning => typeof warning === 'string')) throw new Error('The production modifier reply is malformed.');
  return reply;
}

/** Checks the history's shape: each line's steps in hour order without overlaps,
 * exact raw values, names in id then hour order, and reads ascending. */
export function validateProductionReply(data: unknown): ProductionLinesReply {
  const reply = data as ProductionLinesReply;
  const byIdThenHour = (items: readonly { id: string; firstHour: number }[]) =>
    items.every((item, index) => RAW.test(item.id) && HOUR(item.firstHour) && (index === 0 || items[index - 1].id !== item.id || items[index - 1].firstHour < item.firstHour));
  const ok = typeof reply === 'object' && reply != null && typeof reply.branchId === 'string' && typeof reply.tag === 'string'
    && Number.isInteger(reply.decimals) && reply.decimals >= 0 && reply.decimals <= 12
    && Number.isInteger(reply.detailHours) && reply.detailHours > 0
    && Array.isArray(reply.lines) && reply.lines.every(line => typeof line.key === 'string' && RAW.test(line.handle)
      && (line.kind === null || KINDS.includes(line.kind)) && Array.isArray(line.variants) && line.variants.every(id => RAW.test(id))
      // A line has reads, or was made and deleted between two (its final output only).
      && Array.isArray(line.steps) && ordered(line.steps) && line.steps.every(isStep)
      && (line.steps.length > 0 || (line.betweenReads != null && line.finalOutput != null && ordered([line.betweenReads])))
      && (line.finalOutput === null || isOutput(line.finalOutput))
      && (line.endedBy === null || HOUR(line.endedBy)) && typeof line.beforeRecording === 'boolean')
    && new Set(reply.lines.map(line => line.key)).size === reply.lines.length
    && Array.isArray(reply.variants) && byIdThenHour(reply.variants) && reply.variants.every(variant => typeof variant.token === 'string' && typeof variant.known === 'boolean'
      && NULLABLE(variant.name) && NULLABLE(variant.typeName) && NULLABLE(variant.icon) && NULLABLE(variant.iconUnknown))
    && Array.isArray(reply.organisations) && byIdThenHour(reply.organisations)
      && reply.organisations.every(item => typeof item.token === 'string' && NULLABLE(item.name) && NULLABLE(item.icon)
        && NULLABLE(item.nameUnknown) && NULLABLE(item.iconUnknown))
    && Array.isArray(reply.icons) && reply.icons.every(icon => typeof icon.key === 'string' && (icon.image === null || (typeof icon.image === 'string' && icon.image.startsWith('data:image/png;base64,'))))
    && Array.isArray(reply.reads) && reply.reads.every((hour, index) => HOUR(hour) && (index === 0 || reply.reads[index - 1] < hour))
    && [reply.outputHours, reply.lineHours, reply.organisationHours, reply.unreadOutput].every(spans => Array.isArray(spans) && ordered(spans))
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string');
  if (!ok) throw new Error('The production lines reply is malformed.');
  return reply;
}

/** Checks a window's shape: each line's runs and output in hour order without
 * overlaps. Modifier factors come from the breakdown, never a window. */
export function validateDetailReply(data: unknown): ProductionLineDetailReply {
  const reply = data as ProductionLineDetailReply;
  const ok = typeof reply === 'object' && reply != null && typeof reply.branchId === 'string' && typeof reply.tag === 'string'
    && Number.isInteger(reply.decimals) && HOUR(reply.window) && HOUR(reply.firstHour) && HOUR(reply.lastHour) && reply.firstHour <= reply.lastHour
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string')
    && Array.isArray(reply.lines) && reply.lines.every(line => typeof line.key === 'string'
      && Array.isArray(line.runs) && ordered(line.runs) && line.runs.every(isRun)
      && Array.isArray(line.output) && ordered(line.output) && line.output.every(isOutput))
    && new Set(reply.lines.map(line => line.key)).size === reply.lines.length;
  if (!ok) throw new Error('The production lines reply is malformed.');
  return reply;
}

/** The detail window holding an hour. */
export const windowOf = (reply: ProductionLinesReply, hour: number) => Math.floor(hour / reply.detailHours);

const details = new WeakMap<ProductionLineDetailReply, Map<string, LineDetail>>();

/** A line's reads in a window, by its key; null when the window holds none of it. */
export function detailOf(detail: ProductionLineDetailReply | null | undefined, key: string): LineDetail | null {
  if (!detail) return null;
  let byKey = details.get(detail);
  if (!byKey) { byKey = new Map(detail.lines.map(line => [line.key, line])); details.set(detail, byKey); }
  return byKey.get(key) ?? null;
}

const grouped = new WeakMap<readonly { id: string }[], Map<string, readonly { id: string; firstHour: number }[]>>();

/** A reply's entries of one id, in hour order: grouped once per reply. */
function entriesOf<T extends { id: string; firstHour: number }>(items: readonly T[], id: string): readonly T[] {
  let groups = grouped.get(items);
  if (!groups) {
    groups = new Map();
    for (const item of items) groups.set(item.id, [...groups.get(item.id) ?? [], item]);
    grouped.set(items, groups);
  }
  return (groups.get(id) ?? []) as readonly T[];
}

/** A variant's names and icon as recorded at an hour (the kit's `entryAt`): none
 * where the variant definitions were not known (an entry with `known` false, from the
 * shared `VariantCatalogue`: never an earlier read's name across an unread change),
 * nor before its first read (a line can list a variant the day before its definition
 * is read). */
export function variantAt(reply: ProductionLinesReply, id: string, hour: number): VariantName | null {
  const entry = entryAt(entriesOf(reply.variants, id), hour);
  return entry?.known ? entry : null;
}

/** An organisation as recorded at an hour: none where the organisations were not
 * recorded (never an earlier read's name across an unread rename), else the latest
 * read at or before it. */
export const organisationAt = (reply: ProductionLinesReply, id: string, hour: number): OrganisationName | null =>
  id === '0' || runAt(reply.organisationHours, hour) == null ? null : entryAt(entriesOf(reply.organisations, id), hour);

/** A line's name: the game's, else its equipment type's script key; where the
 * variant is not recorded, its id, so a line is never blank. */
export const variantName = (variant: VariantName | null, id: string) => variant ? variant.name ?? 'Variant name unavailable' : `Variant ${id}`;

export function productionLineLabel(reply: ProductionLinesReply, line: LineHistory, index: number, hour: number): string {
  const step = entryAt(line.steps, hour);
  const id = step?.variant ?? line.variants[0];
  const variant = id === undefined ? null : variantAt(reply, id, hour);
  return `${variant?.name ?? variant?.typeName ?? 'Equipment unavailable'} · line ${index + 1}`;
}

/** Why a variant shows only its id, for its tooltip. */
const UNNAMED = 'The variant’s name is not recorded at this time.';

/** Why lines may be missing at an hour: a due output read, the only place a line
 * made and ended between two reads publishes its rows, could not be read. */
export function unreadNote(reply: ProductionLinesReply, hour: number): string | undefined {
  return runAt(reply.unreadOutput, hour) ? 'Some production lines may be missing.' : undefined;
}

/** A tooltip's note with any further ones, joined. */
export const withNote = (note: string | undefined, ...more: (string | undefined)[]) => [note, ...more].filter(Boolean).join(' ') || undefined;

/** Whether the country's lines are recorded at an hour (a complete census, read or
 * carried): outside, the lines are unknown, which is not "none". */
export const linesKnown = (reply: ProductionLinesReply, hour: number) => runAt(reply.lineHours, hour) != null;

/** The final count of a line that has ended by `hour`: what it made in all, with
 * each variant's part. Null while it runs, before the count was published (a census
 * can miss the line before its output is readable again), or when it never was. */
export const finalAt = (line: LineHistory, hour: number): OutputRun | null =>
  line.finalOutput && line.endedBy != null && hour >= line.endedBy && hour >= line.finalOutput.firstHour ? line.finalOutput : null;

/** A picture by its key in the reply; null when there is none or it could not be read. */
export function iconImage(reply: ProductionLinesReply, key: string | null): string | null {
  return key == null ? null : reply.icons.find(icon => icon.key === key)?.image ?? null;
}

/** Units per week by the game's weekly formula, fixed5, as the product computes it
 * (`equipment::weekly_units_raw`); null without a positive cost. */
export const weeklyOutput = (run: LineRun): bigint | null => run.weeklyRaw == null ? null : BigInt(run.weeklyRaw);

/** Units per week as the row shows them: one decimal, or every recorded decimal
 * for a slow line (a ship at a few thousandths a week) that one would round to 0,
 * so a line still producing never reads as making nothing. */
export function weeklyText(weekly: bigint, decimals: number): string {
  const short = formatAmount(weekly, decimals, 1);
  return short === '0' && weekly !== 0n ? formatAmount(weekly, decimals, decimals) : short;
}

/** Q15 efficiency's full scale: 100%. */
const EFFICIENCY_FULL = 3276800n;

/** The efficiency as the production row shows it: `(raw + 0x4000) >> 15` percent. */
export const efficiencyPercent = (run: LineRun) => (BigInt(run.efficiencyRaw) + 0x4000n) >> 15n;

/** The efficiency bar's fill, 0–1 of 100%. */
export const efficiencyFraction = (run: LineRun) => Math.max(0, Math.min(1, Number(BigInt(run.efficiencyRaw)) / Number(EFFICIENCY_FULL)));

export type LineOrder = 'game' | 'factories' | 'output';

/** A line running at the hour shown, with its reads in the window and its values there. */
export interface LineAt { line: LineHistory; detail: LineDetail; run: LineRun }

/** The lines running at an hour (read in `detail`, the window holding it), in the
 * game's order (their queue place), by factories assigned, or by output as IC a day
 * (units of different equipment do not compare); ties keep the game's order. */
export function linesAt(reply: ProductionLinesReply, detail: ProductionLineDetailReply, hour: number, order: LineOrder): LineAt[] {
  const running = reply.lines.flatMap(line => {
    const lineDetail = detailOf(detail, line.key);
    const run = lineDetail && runAt(lineDetail.runs, hour);
    return lineDetail && run ? [{ line, detail: lineDetail, run }] : [];
  });
  const game = (a: LineAt, b: LineAt) => a.run.priority - b.run.priority || a.line.key.localeCompare(b.line.key);
  const compare = order === 'factories' ? (a: LineAt, b: LineAt) => b.run.activeFactories - a.run.activeFactories || game(a, b)
    : order === 'output' ? (a: LineAt, b: LineAt) => {
      const [x, y] = [BigInt(a.run.speedRaw), BigInt(b.run.speedRaw)];
      return x === y ? game(a, b) : x > y ? -1 : 1;
    } : game;
  return running.sort(compare);
}

/** The most factories any line was assigned or asked for over the branch: the live
 * bars' shared scale, so they never rescale during playback. */
export function factoryScale(reply: ProductionLinesReply): number {
  let most = 0;
  for (const line of reply.lines) for (const step of line.steps) most = Math.max(most, step.activeFactories, step.requestedFactories);
  return most;
}

/** A line's output as recorded at an hour: the read behind it (in its window's
 * reads), else, after the line ended, the final count published then. */
export function outputAt(line: LineHistory, detail: LineDetail | null, hour: number): OutputRun | null {
  const run = detail && runAt(detail.output, hour);
  if (run) return run;
  return line.finalOutput && hour >= line.finalOutput.firstHour ? line.finalOutput : null;
}

/** A line's total: the sum of its parts' units when every part is exact, else null
 * (a tick could not be counted, so no number is true). */
export function outputTotal(output: OutputRun): bigint | null {
  if (output.parts.some(part => !part.exact)) return null;
  return output.parts.reduce((sum, part) => sum + BigInt(part.units), 0n);
}

/** A line's total as the TOTAL column shows it. */
export function totalText(output: OutputRun | null): string {
  if (!output) return 'Not recorded';
  const total = outputTotal(output);
  return total == null ? 'Unknown' : gbNumber.format(total);
}

/** What a total counts, for its tooltip. */
export function totalNote(output: OutputRun | null): string | undefined {
  if (!output) return 'Units made are not recorded at this time.';
  const from = output.wholeLife ? 'Since the line was created.' : '';
  return outputTotal(output) == null
    ? 'Total unknown; some production is missing.'
    : from || undefined;
}

/** The organisation column: its name, else its script key; a dash for none. */
export const organisationText = (organisation: OrganisationName | null, run: LineRun) =>
  run.mio === '0' ? '—' : organisation ? organisation.nameUnknown ? 'Unknown owner' : contentTitle(organisation.name, organisation.token, 'Organisation name unavailable') : 'Not recorded';

/** A tooltip in its forms, fullest first, for the kit's `fitTooltip`: all of it, then
 * with a count's parts summarised, so a line with many parts fits a short window. */
export type TooltipForms = readonly TooltipContent[];

/** What a count made of each variant, in first-seen order: a variant made under
 * several organisations is one part (the recorder counts each pairing), its units
 * their sum, exact only when every one of its parts is. */
export function variantParts(output: OutputRun): { variant: string; units: bigint | null }[] {
  const byVariant = new Map<string, bigint | null>();
  for (const part of output.parts) {
    const before = byVariant.has(part.variant) ? byVariant.get(part.variant)! : 0n;
    byVariant.set(part.variant, before == null || !part.exact ? null : before + BigInt(part.units));
  }
  return [...byVariant].map(([variant, units]) => ({ variant, units }));
}

/** A count's rows: its total, and what it made of each variant when it made several
 * (`parts: false` counts them instead, for a compact form). */
function countRows(reply: ProductionLinesReply, output: OutputRun | null, hour: number, label: string, parts = true): TooltipRow[] {
  const total = output ? outputTotal(output) : null;
  const byVariant = output ? variantParts(output) : [];
  const split = byVariant.length > 1 ? byVariant : [];
  const rows: TooltipRow[] = parts ? split.map(part => ({
    label: variantName(variantAt(reply, part.variant, hour), part.variant), value: part.units == null ? 'Unknown' : gbNumber.format(part.units), indent: true, short: true,
  })) : split.length ? [{ label: `Of ${split.length} variants`, tone: 'muted', indent: true }] : [];
  return [{ label, value: totalText(output), strong: true, divider: true, tone: output && total == null ? 'muted' : undefined }, ...rows];
}

/** Both forms of a tooltip whose rows hold counts. */
const forms = (build: (parts: boolean) => TooltipContent): TooltipForms => [build(true), build(false)];

/** Both hover forms retain the whole's status; the full form supplies speech. */
export function modifierTooltip(group: ProductionModifierRun['groups'][number] | undefined, title: string,
  color: (id: string) => string | undefined): TooltipForms {
  if (!group) return [{ title, rows: [{ label: 'Breakdown', value: 'Not recorded', tone: 'muted' }] }];
  const total: TooltipRow = { label: group.label, value: group.totalRaw == null ? 'Not recorded'
    : `${formatFixed(group.totalRaw, group.decimals, group.decimals)} ${group.unit}`, strong: true };
  const rows: TooltipRow[] = group.parts.map(part => ({ label: part.label ?? 'Other',
    value: `${formatFixed(part.raw, group.decimals, group.decimals)} ${group.unit}`, swatch: color(part.id),
    tone: part.unknown == null ? undefined : 'muted' }));
  return [{ title, rows: [total, ...rows] }, { title, rows: [total,
    { label: `${group.parts.length} factors`, tone: 'muted' }] }];
}

/** A live cell's recorded line values. Its modifier factors are the breakdown's,
 * which the cell opens: the read window carries no modifier trees, and nothing
 * here calls them missing. */
export function breakdownTooltip(reply: ProductionLinesReply, run: LineRun, hour: number, part: 'efficiency' | 'output',
  head: Pick<TooltipContent, 'subtitle' | 'note'>): TooltipForms {
  const variant = variantAt(reply, run.variant, hour);
  const weekly = weeklyOutput(run);
  const exact = (raw: string) => formatFixed(raw, reply.decimals, reply.decimals);
  const rows: TooltipRow[] = part === 'efficiency' ? [
    { label: 'Efficiency', value: `${efficiencyPercent(run)}%`, strong: true },
    { label: 'Factories included in average', value: String(Math.max(0, Math.min(run.activeFactories, run.maxFactories))) },
  ] : [
    { label: 'Output', value: weekly == null ? 'Not recorded' : `${weeklyText(weekly, reply.decimals)} per week`, strong: true },
    { label: 'Factories', value: `${run.activeFactories} of ${run.requestedFactories} requested` },
    { label: 'Efficiency', value: `${efficiencyPercent(run)}%` },
    { label: 'Output rate', value: `${exact(run.speedRaw)} IC` },
    { label: 'Cost per unit', value: `${exact(run.costRaw)} IC` },
  ];
  return [{ title: variantName(variant, run.variant), icon: iconImage(reply, variant?.icon ?? null) ?? undefined,
    subtitle: head.subtitle, rows, note: head.note }];
}

/** The tooltip of a line at an hour after it ended, or of one made and deleted
 * between two reads: its final count, with each variant's part. */
export function finalTooltip(reply: ProductionLinesReply, line: LineHistory, hour: number,
  head: Pick<TooltipContent, 'subtitle' | 'note'>, day: (hour: number) => string): TooltipForms {
  const final = finalAt(line, hour);
  const id = line.steps.at(-1)?.variant ?? line.variants[0] ?? '';
  const variant = variantAt(reply, id, hour);
  const when: TooltipRow = line.betweenReads
    ? { label: line.beforeRecording
      ? `Ran before recording began; ended by ${day(line.betweenReads.lastHour)}`
      : `Made and deleted between ${day(line.betweenReads.firstHour)} and ${day(line.betweenReads.lastHour)}`, tone: 'muted' }
    : { label: line.endedBy != null ? `Ended by ${day(line.endedBy)}` : 'Ended', tone: 'muted' };
  const note = [head.note, final ? totalNote(final)
    : 'Final total unavailable at this time.'].filter(Boolean).join(' ');
  return forms(parts => ({ title: variantName(variant, id), icon: iconImage(reply, variant?.icon ?? null) ?? undefined, subtitle: head.subtitle,
    rows: [when, ...countRows(reply, final, hour, 'Made in all', parts)], note }));
}

/** One line's tooltip at an hour, from its reads in the window holding it: what the
 * game's production row shows; once the line has ended by `until` (the moment the
 * view shows), its final count too. */
export function lineTooltip(reply: ProductionLinesReply, line: LineHistory, detail: LineDetail | null, run: LineRun, hour: number,
  head: Pick<TooltipContent, 'subtitle' | 'note'>, day: (hour: number) => string, until = hour): TooltipForms {
  const variant = variantAt(reply, run.variant, hour);
  const organisation = organisationAt(reply, run.mio, hour);
  const weekly = weeklyOutput(run);
  const output = outputAt(line, detail, hour);
  const final = finalAt(line, until);
  const note = [head.note, totalNote(output)].filter(Boolean).join(' ');
  return forms(parts => {
    const ended: TooltipRow[] = final ? [{ label: `Ended by ${day(line.endedBy!)}`, tone: 'muted', divider: true },
      ...countRows(reply, final, until, 'Made in all', parts).map(row => ({ ...row, divider: false }))] : [];
    const rows: TooltipRow[] = [
      ...(variant?.typeName ? [{ label: variant.typeName, tone: 'muted' as const }] : []),
      { label: 'Factories', value: `${run.activeFactories} of ${run.requestedFactories} requested`, divider: true },
      { label: 'Efficiency', value: `${efficiencyPercent(run)}%` },
      { label: 'Output', value: weekly == null ? 'Not recorded' : `${weeklyText(weekly, reply.decimals)} per week` },
      { label: 'Cost', value: `${formatAmount(run.costRaw, reply.decimals)} IC per unit` },
      { label: 'Priority', value: String(run.priority + 1) },
      { label: 'Ordered', value: run.amount < 0 ? 'No limit' : `${run.amount} more` },
      { label: 'Military-industrial organisation', value: organisationText(organisation, run), icon: iconImage(reply, organisation?.icon ?? null) ?? undefined },
      ...countRows(reply, output, hour, 'Produced so far', parts),
      ...ended,
    ];
    return { title: variantName(variant, run.variant), icon: iconImage(reply, variant?.icon ?? null) ?? undefined, subtitle: head.subtitle, rows, note };
  });
}

/** A line's tooltip at an hour whose window is not loaded (yet, or it failed): what
 * the history itself holds, its variant and factories, and why the rest is missing. */
export function stepTooltip(reply: ProductionLinesReply, step: LineStep, hour: number, head: Pick<TooltipContent, 'subtitle' | 'note'>,
  why: string): TooltipForms {
  const variant = variantAt(reply, step.variant, hour);
  return [{ title: variantName(variant, step.variant), icon: iconImage(reply, variant?.icon ?? null) ?? undefined, subtitle: head.subtitle,
    rows: [...(variant?.typeName ? [{ label: variant.typeName, tone: 'muted' as const }] : []),
      { label: 'Factories', value: `${step.activeFactories} of ${step.requestedFactories} requested`, divider: true }],
    note: [head.note, variant ? null : UNNAMED, why].filter(Boolean).join(' ') }];
}

/** A line's tooltip at an hour it has no step: its status then (not recorded, not
 * started, ended), named after the variant it last made before that hour (or its
 * first, before it started), never a value from another hour of the lane. */
export function gapTooltip(reply: ProductionLinesReply, line: LineHistory, hour: number, subtitle: string,
  day: (hour: number) => string, note?: string): TooltipContent {
  const step = line.steps.filter(item => item.firstHour <= hour).at(-1) ?? line.steps[0];
  const variant = step ? variantAt(reply, step.variant, Math.min(hour, step.lastHour)) : null;
  return { title: step ? variantName(variant, step.variant) : 'Production line', icon: iconImage(reply, variant?.icon ?? null) ?? undefined, subtitle,
    rows: [{ label: laneStatus(line, hour, day).text, tone: 'muted' }], note };
}

/** How a line is described beside its lane: factories at the hour shown, or when it
 * ended (by the first read without it), or when it was last recorded. */
export function laneStatus(line: LineHistory, hour: number, day: (hour: number) => string): { text: string; ended: boolean } {
  if (line.betweenReads) {
    if (line.beforeRecording) return { text: `Ended by ${day(line.betweenReads.lastHour)}`, ended: true };
    return hour < line.betweenReads.firstHour ? { text: `Made after ${day(line.betweenReads.firstHour)}`, ended: false }
      : { text: `Made and deleted by ${day(line.betweenReads.lastHour)}`, ended: true };
  }
  const step = runAt(line.steps, hour);
  if (step) return { text: `${step.activeFactories} ${step.activeFactories === 1 ? 'factory' : 'factories'}`, ended: false };
  const last = line.steps.at(-1)!;
  if (hour < line.steps[0].firstHour) return { text: `Starts ${day(line.steps[0].firstHour)}`, ended: false };
  if (line.endedBy != null && hour >= line.endedBy) return { text: `Ended by ${day(line.endedBy)}`, ended: true };
  if (hour > last.lastHour) return { text: `Last recorded ${day(last.lastHour)}`, ended: true };
  return { text: 'Not recorded then', ended: false };
}

/** Lanes for the history: one per line first seen by `until` (the playhead, or the
 * end), in the order first seen. Each unbroken stretch of one variant is a span whose
 * band is the factories assigned at each read; a line running at `until` is accented
 * across all its variants, an ended one grey. Ended lines can be left out. The
 * keyboard stops at every read (a line's efficiency and count change at reads its
 * band does not show) and at the hour a line made between reads published its count. */
export function historyLanes(reply: ProductionLinesReply, until: number, showEnded: boolean, day: (hour: number) => string): TimelineLane[] {
  const lanes: TimelineLane[] = [];
  for (const line of reply.lines) {
    const between = line.betweenReads;
    if ((between ? between.lastHour : line.steps[0].firstHour) > until) continue;
    const status = laneStatus(line, until, day);
    if (status.ended && !showEnded) continue;
    if (between) {
      // Made and deleted between two reads: its factories are not known, so a plain
      // bar over the hours it could have run, in the ended look.
      const id = line.variants[0] ?? '';
      const variant = variantAt(reply, id, until);
      const icon = iconImage(reply, variant?.icon ?? null) ?? undefined;
      lanes.push({ id: line.key, label: status.text, muted: true, identity: { label: variantName(variant, id), icon },
        spans: [{ id: `${line.key}@${between.firstHour}`, firstHour: between.firstHour, lastHour: between.lastHour, label: variantName(variant, id), icon,
          stops: line.finalOutput ? [line.finalOutput.firstHour] : undefined }] });
      continue;
    }
    const spans: TimelineSpan[] = [];
    let band: BandStep[] = [];
    let first: LineStep | null = null;
    const close = (last: LineStep) => {
      if (!first) return;
      const variant = variantAt(reply, first.variant, first.firstHour);
      spans.push({ id: `${line.key}@${first.firstHour}`, firstHour: first.firstHour, lastHour: last.lastHour, label: variantName(variant, first.variant),
        icon: iconImage(reply, variant?.icon ?? null) ?? undefined, emphasis: !status.ended, band, stops: reply.reads });
      band = [];
      first = null;
    };
    line.steps.forEach((step, index) => {
      const previous = line.steps[index - 1];
      if (previous && (step.startsSegment || previous.variant !== step.variant || previous.lastHour + 1 !== step.firstHour)) close(previous);
      first ??= step;
      band.push({ firstHour: step.firstHour, lastHour: step.lastHour, value: step.activeFactories });
    });
    close(line.steps.at(-1)!);
    const shown = runAt(line.steps, until) ?? line.steps.filter(step => step.firstHour <= until).at(-1)!;
    const variant = variantAt(reply, shown.variant, Math.min(until, shown.lastHour));
    lanes.push({ id: line.key, label: status.text, spans, muted: status.ended,
      identity: { label: variantName(variant, shown.variant), icon: iconImage(reply, variant?.icon ?? null) ?? undefined } });
  }
  return lanes;
}

/** The most factories any line was assigned: the history's band scale. */
export function bandScale(reply: ProductionLinesReply): number {
  let most = 0;
  for (const line of reply.lines) for (const step of line.steps) most = Math.max(most, step.activeFactories);
  return most;
}
