import type { Failure } from './queryError';
import { contentTitle } from './contentTitles.ts';
import type { TooltipContent, TooltipRow } from '../charts/tooltip';
import type { EffectGroup, EffectLine, IdeaHistoryReply, IdeaInfo, ModifierFormat, TraitEffects } from '../generated/dashboard';


export const IDEA_PRECISION = 2;
const FIXED = 100_000n;

export function fixed5Text(raw: bigint, precision: number): string {
  const negative = raw < 0n;
  const size = negative ? -raw : raw;
  const fraction = size % FIXED;
  let text = `${negative ? '-' : ''}${size / FIXED}`;
  if (precision !== 0 && (fraction !== 0n || precision >= 0)) {
    const digits = (fraction + FIXED).toString().slice(1);
    const kept = precision >= 0 ? digits.slice(0, precision) : digits.replace(/0+$/, '');
    if (kept) text += `.${kept}`;
  }
  return text;
}

export function fixed5Of(value: number): bigint {
  const text = String(value);
  if (/e/i.test(text)) return BigInt(Math.trunc(value * 100_000));
  const negative = text.startsWith('-');
  const [whole, fraction = ''] = (negative ? text.slice(1) : text).split('.');
  const raw = BigInt(whole) * 100_000n + BigInt(`${fraction}00000`.slice(0, 5));
  return negative ? -raw : raw;
}

export type EffectTone = 'positive' | 'negative' | 'neutral';

export function formatModifier(format: ModifierFormat, raw: bigint, words: { yes: string; no: string }, precision = IDEA_PRECISION): { text: string; tone: EffectTone } {
  const flags = format.flags;
  let text: string;
  if (flags & 0x20) text = raw === 0n ? words.no : words.yes;
  else {
    const digits = Math.min(precision, format.precision);
    const sign = raw > 0n && !(flags & 0x08) ? '+' : '';
    text = sign + (flags & 0x01 ? `${fixed5Text(raw * 100n, digits)}%` : flags & 0x80 ? `${fixed5Text(raw, digits)}%` : fixed5Text(raw, digits));
  }
  const good = (flags & 0x02) !== 0;
  const tone: EffectTone = flags & 0x04 || raw === 0n ? 'neutral' : (raw > 0n) === good ? 'positive' : 'negative';
  if (format.postfix) text += ` ${format.postfix}`;
  return { text, tone };
}

export interface IdeaTooltipSources {
  formats: ReadonlyMap<string, ModifierFormat>;
  text: (key: string) => string | null;
  countryName: (tag: string) => string;
  nameAt?: (tag: string, hour: number) => string;
  countryFailure?: (tag: string) => Failure | undefined;
  failureAt?: (tag: string, hour: number) => Failure | undefined;
}

export function ideaTargetTags(info: IdeaInfo | null | undefined): string[] {
  if (!info) return [];
  return [...new Set([...info.targeted, ...info.traits.flatMap(trait => trait.targeted)]
    .flatMap(group => group.target === null ? [] : [group.target]))];
}

export function ideaTooltipSources(reply: Pick<IdeaHistoryReply, 'modifiers' | 'texts'>, countryName: (tag: string) => string,
  nameAt?: (tag: string, hour: number) => string): IdeaTooltipSources {
  const formats = new Map(reply.modifiers.map(format => [format.key, format]));
  const texts = new Map(reply.texts.map(item => [item.key, item.text]));
  return { formats, text: key => texts.get(key) ?? null, countryName, nameAt };
}

export function sourcesAt(sources: IdeaTooltipSources, hour: number | undefined): IdeaTooltipSources {
  const nameAt = sources.nameAt;
  return hour == null || !nameAt ? sources : { ...sources, countryName: tag => nameAt(tag, hour), countryFailure: tag => sources.failureAt?.(tag, hour) };
}

interface Section { rows: TooltipRow[]; featured?: boolean }

function modifierRows(lines: readonly EffectLine[], sources: IdeaTooltipSources): Section {
  const words = { yes: sources.text('YES') ?? 'Yes', no: sources.text('NO') ?? 'No' };
  const order = (key: string) => sources.formats.get(key)?.index ?? Number.MAX_SAFE_INTEGER;
  let featured = false;
  const rows: TooltipRow[] = [];
  const listed = lines.filter(line => !line.hidden);
  for (const line of [...listed].sort((a, b) => order(a.key) - order(b.key) || a.key.localeCompare(b.key))) {
    const format = sources.formats.get(line.key);
    const authoredRaw = BigInt(line.fixed5);
    const raw = format && format.flags & 0x10 ? (authoredRaw < 0n ? 0n : authoredRaw > 100_000n ? 100_000n : authoredRaw) : authoredRaw;
    if (!format) {
      rows.push({ label: 'Modifier name unavailable:', lookupKey: line.key, value: fixed5Text(raw, -1), tone: 'muted' });
      continue;
    }
    if (raw === 0n && !(format.flags & 0x100)) continue;
    const shown = formatModifier(format, raw, words);
    const unitMissing = format.postfix == null && format.postfixKey != null;
    if (format.feature !== 0) featured = true;
    rows.push({ label: `${contentTitle(format.label, line.key, 'Modifier name unavailable')}:`, lookupKey: line.key, value: `${unitMissing ? `${shown.text} [unit unavailable]` : shown.text}${format.feature !== 0 ? ' *' : ''}`,
      tone: unitMissing || format.feature !== 0 ? 'muted' : shown.tone });
  }
  return { rows, featured };
}

function section(heading: string | null, rows: readonly TooltipRow[], detail?: string): TooltipRow[] {
  if (!rows.length && heading == null) return [];
  const all: TooltipRow[] = heading == null ? [...rows] : [{ label: heading, strong: true }, ...rows];
  return all.map((row, index) => index === 0 ? { ...row, divider: true, ...(detail ? { detail } : {}) } : row);
}

function traitSection(found: TraitEffects, sources: IdeaTooltipSources): Section {
  const name = contentTitle(found.name, found.token, 'Trait name unavailable');
  if (!found.known) return { rows: section(name, []) };
  const own = modifierRows(found.lines, sources);
  const groups = [...found.targeted.map(group => targetedSection(group, sources)), ...found.equipment.map(group => equipmentSection(group, sources))];
  return { rows: [...section(name, [...own.rows, ...found.tooltips.map(key => ({ label: contentTitle(sources.text(key), key, 'Tooltip text unavailable'), lookupKey: key }))]), ...groups.flatMap(group => group.rows)],
    featured: own.featured || groups.some(group => group.featured) };
}

function targetedSection(group: EffectGroup, sources: IdeaTooltipSources): Section {
  const lines = modifierRows(group.lines, sources);
  const rows = [...lines.rows, ...group.tooltips.map(key => ({ label: contentTitle(sources.text(key), key, 'Tooltip text unavailable'), lookupKey: key }))];
  return { rows: section(group.target ? `Against ${sources.countryName(group.target)}:` : 'Against a country:', rows, group.target ? sources.countryFailure?.(group.target)?.message : undefined), featured: lines.featured };
}

function equipmentSection(group: EffectGroup, sources: IdeaTooltipSources): Section {
  const name = group.target ? contentTitle(sources.text(group.target), group.target, 'Equipment name unavailable') : 'Equipment';
  const rows = group.lines.filter(line => !line.hidden).map(line => ({ label: 'Equipment modifier name unavailable:', lookupKey: line.key, tone: 'muted' as const,
    value: fixed5Text(BigInt(line.fixed5), -1) }));
  return { rows: section(`${name}${group.instant ? ' (at once)' : ''}:`, rows) };
}

function researchRows(lines: readonly EffectLine[], sources: IdeaTooltipSources): TooltipRow[] {
  return lines.filter(line => !line.hidden).map(line => {
    const label = `${contentTitle(sources.text(`${line.key}_research`), `${line.key}_research`, 'Research category name unavailable')}:`;
    const raw = BigInt(line.fixed5);
    return { label, lookupKey: line.key, value: `${raw > 0n ? '+' : ''}${fixed5Text(raw * 100n, 0)}%`, tone: raw < 0n ? 'negative' as const : 'positive' as const };
  });
}

export function ideaEffectRows(info: IdeaInfo, sources: IdeaTooltipSources): { rows: TooltipRow[]; note?: string } {
  if (!info.effectsKnown) return { rows: [] };
  const sections: Section[] = [
    ...info.traits.map(found => traitSection(found, sources)),
  ];
  const own = modifierRows(info.effects, sources);
  sections.push({ rows: section(null, [...own.rows, ...info.tooltips.map(key => ({ label: contentTitle(sources.text(key), key, 'Tooltip text unavailable'), lookupKey: key }))]), featured: own.featured });
  sections.push(...info.targeted.map(group => targetedSection(group, sources)));
  sections.push({ rows: section(null, researchRows(info.research, sources)) });
  sections.push(...info.equipment.map(group => equipmentSection(group, sources)));
  const rows = sections.flatMap(item => item.rows);
  return { rows, note: sections.some(item => item.featured) ? '* Depends on an enabled game feature.' : undefined };
}

export const MAX_TOOLTIP_ROWS = 40;

export function ideaTooltip(head: { title: string; icon?: string | null; subtitle?: string }, info: IdeaInfo | undefined, sources: IdeaTooltipSources,
  description?: string | null, before: readonly TooltipRow[] = [], note?: string, hour?: number, maxRows = tooltipRowBudget()): TooltipContent {
  const effects = info ? ideaEffectRows(info, sourcesAt(sources, hour)) : { rows: [] as TooltipRow[], note: undefined };
  const rows = [...before, ...effects.rows];
  const header = Math.ceil(head.title.length / 26) + (head.subtitle ? Math.ceil(head.subtitle.length / 40) : 0);
  const room = maxRows - Math.max(0, header - 2);
  const total = rows.reduce((sum, row) => sum + rowLines(row), 0);
  const budget = total <= room ? room : room - 2;
  let used = 0;
  let fit = 0;
  while (fit < rows.length && used + rowLines(rows[fit]) <= budget) used += rowLines(rows[fit++]);
  const more = rows.length - fit;
  const shown = more > 0 ? [...rows.slice(0, fit), { label: `${more} more effects`, tone: 'muted' as const, divider: true }] : rows;
  const notes = [effects.note, note].filter(Boolean).join(' ');
  return { title: head.title, icon: head.icon ?? undefined, subtitle: head.subtitle, rows: shown,
    description: description ?? undefined, note: notes || undefined };
}

export const rowLines = (row: TooltipRow) => (row.short ? 1 : Math.max(1, Math.ceil((row.label.length + (row.value?.length ?? 0) + 3) / 30))) + (row.divider ? 1 : 0);

export { tooltipText } from '../charts/tooltip.ts';

export function tooltipRowBudget(height = typeof window === 'undefined' ? 800 : window.innerHeight): number {
  return Math.max(4, Math.min(MAX_TOOLTIP_ROWS, Math.floor((height - 260) / 18)));
}
