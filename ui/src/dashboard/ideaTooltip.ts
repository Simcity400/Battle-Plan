import type { TooltipContent, TooltipRow } from '../charts/tooltip';
import type { EffectGroup, EffectLine, IdeaHistoryReply, IdeaInfo, ModifierFormat, TraitEffects } from '../generated/dashboard';

/** An idea's effects as the game's own tooltip shows them (mock-up politics-4),
 * formatted with the game's rules from the recorded modifier definitions:
 * `FormatValue@14054FE50` and `Fixed5ToString@1424B6200` (evidence beside the
 * `idea_format` product). A key without a definition is shown as authored and
 * marked, never guessed. */

/** The precision the idea tooltip asks for (FUN_140FBC770 passes 2). */
export const IDEA_PRECISION = 2;
const FIXED = 100_000n;

/** A fixed5 value as the game writes it: truncated, never rounded; with exactly
 * `precision` decimals when it is positive (trailing zeros kept), none when it
 * is zero, and trailing zeros dropped when it is negative. */
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

/** A script value as the game's fixed point: the script conversion keeps only
 * the first five decimals of the authored text (`1424b0480`, beside
 * `installed-content/src/terrain.rs`). The reader's number prints back as its shortest
 * decimal form, which is the authored text, so that text is truncated here. */
export function fixed5Of(value: number): bigint {
  const text = String(value);
  // Exponent forms are far below or above anything a modifier holds.
  if (/e/i.test(text)) return BigInt(Math.trunc(value * 100_000));
  const negative = text.startsWith('-');
  const [whole, fraction = ''] = (negative ? text.slice(1) : text).split('.');
  const raw = BigInt(whole) * 100_000n + BigInt(`${fraction}00000`.slice(0, 5));
  return negative ? -raw : raw;
}

export type EffectTone = 'positive' | 'negative' | 'neutral';

/** One modifier value as the game formats it: yes/no, or a sign (unless the
 * definition drops it), the number at the lower of the two precisions as a
 * percentage when flagged, coloured good or bad (or neutral), then the postfix. */
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

/** What the tooltip builder reads from an idea-history reply. */
export interface IdeaTooltipSources {
  formats: ReadonlyMap<string, ModifierFormat>;
  /** A localisation key's text in the viewer's language, or null. */
  text: (key: string) => string | null;
  countryName: (tag: string) => string;
  /** A country's name as recorded at an hour, where the widget knows it. */
  nameAt?: (tag: string, hour: number) => string;
}

export function ideaTooltipSources(reply: Pick<IdeaHistoryReply, 'modifiers' | 'texts'>, countryName: (tag: string) => string,
  nameAt?: (tag: string, hour: number) => string): IdeaTooltipSources {
  const formats = new Map(reply.modifiers.map(format => [format.key, format]));
  const texts = new Map(reply.texts.map(item => [item.key, item.text]));
  return { formats, text: key => texts.get(key) ?? null, countryName, nameAt };
}

/** The sources naming countries as they were at `hour`. */
export function sourcesAt(sources: IdeaTooltipSources, hour: number | undefined): IdeaTooltipSources {
  const nameAt = sources.nameAt;
  return hour == null || !nameAt ? sources : { ...sources, countryName: tag => nameAt(tag, hour) };
}

/** The rows of the effects, and whether any is shown as authored. */
/** Rows, whether any is shown as authored, and whether any needs a game feature. */
interface Section { rows: TooltipRow[]; authored: boolean; featured?: boolean }

/** Modifier lines as the game lists them (FormatDescription@14054E050): in
 * ascending key index (a scripted modifier after every built-in one), the last
 * value of a repeated key, every key a `hidden_modifier` names left out entirely
 * (the engine keeps a hidden-key set), zero values only where the definition
 * shows them. */
function modifierRows(lines: readonly EffectLine[], sources: IdeaTooltipSources): Section {
  const hidden = new Set(lines.filter(line => line.hidden).map(line => line.key));
  const latest = new Map<string, EffectLine>();
  for (const line of lines) if (!hidden.has(line.key)) latest.set(line.key, line);
  const words = { yes: sources.text('YES') ?? 'Yes', no: sources.text('NO') ?? 'No' };
  const order = (key: string) => sources.formats.get(key)?.index ?? Number.MAX_SAFE_INTEGER;
  let authored = false;
  let featured = false;
  const rows: TooltipRow[] = [];
  for (const line of [...latest.values()].sort((a, b) => order(a.key) - order(b.key) || a.key.localeCompare(b.key))) {
    const format = sources.formats.get(line.key);
    if (line.value == null) {
      authored = true;
      rows.push({ label: `${format?.label ?? line.key}:`, value: line.unsupported ?? 'not evaluated', tone: 'muted' });
      continue;
    }
    // A number the engine's script conversion cannot read (such as `.5`) has no
    // game value: it is shown as written, not given one.
    if (line.fixed5 == null) {
      authored = true;
      rows.push({ label: `${format?.label ?? line.key}:`, value: `${line.value} (not read by the game)`, tone: 'muted' });
      continue;
    }
    // The accumulator's getter clamps a flag-0x10 modifier to [0, 1] (modifier_accumulator::value@140551820).
    const authoredRaw = BigInt(line.fixed5);
    const raw = format && format.flags & 0x10 ? (authoredRaw < 0n ? 0n : authoredRaw > 100_000n ? 100_000n : authoredRaw) : authoredRaw;
    // Without a format the game's zero rule is not known: the authored value stays.
    // No format: the engine's exact value, unformatted.
    if (!format) {
      authored = true;
      rows.push({ label: `${line.key}:`, value: fixed5Text(raw, -1), tone: 'muted' });
      continue;
    }
    if (raw === 0n && !(format.flags & 0x100)) continue;
    const shown = formatModifier(format, raw, words);
    // The game writes the name, then ": ", then the value (FormatLine@14054F6D0).
    // A name localisation lacks is shown as the script key; a unit whose text is
    // missing is shown as its key, marked, never dropped.
    const unitMissing = format.postfix == null && format.postfixKey != null;
    authored ||= format.label == null || unitMissing;
    // A modifier that needs a game feature applies only while it is on, which the
    // recording does not say: it is marked, not shown as certainly in effect.
    if (format.feature !== 0) featured = true;
    rows.push({ label: `${format.label ?? line.key}:`, value: `${unitMissing ? `${shown.text} [${format.postfixKey}]` : shown.text}${format.feature !== 0 ? ' *' : ''}`,
      tone: unitMissing || format.feature !== 0 ? 'muted' : shown.tone });
  }
  return { rows, authored, featured };
}

/** A section: its heading (if any) above its rows, set apart from what came before. */
function section(heading: string | null, rows: readonly TooltipRow[]): TooltipRow[] {
  if (!rows.length && heading == null) return [];
  const all: TooltipRow[] = heading == null ? [...rows] : [{ label: heading, strong: true }, ...rows];
  return all.map((row, index) => index === 0 ? { ...row, divider: true } : row);
}

/** A trait: its name, its modifiers and custom tooltips, then its targeted
 * modifiers and equipment bonuses, each in its own group as the idea's are. */
function traitSection(found: TraitEffects, sources: IdeaTooltipSources): Section {
  const name = found.name ?? found.token;
  if (!found.known) return { rows: section(name, [{ label: 'Effects', value: 'not in the content files', tone: 'muted' }]), authored: false };
  const own = modifierRows(found.lines, sources);
  const groups = [...found.targeted.map(group => targetedSection(group, sources)), ...found.equipment.map(group => equipmentSection(group, sources))];
  return { rows: [...section(name, [...own.rows, ...found.tooltips.map(key => ({ label: sources.text(key) ?? key }))]), ...groups.flatMap(group => group.rows)],
    authored: own.authored || groups.some(group => group.authored), featured: own.featured || groups.some(group => group.featured) };
}

function targetedSection(group: EffectGroup, sources: IdeaTooltipSources): Section {
  const lines = modifierRows(group.lines, sources);
  return { rows: section(group.target ? `Against ${sources.countryName(group.target)}:` : 'Against a country:', lines.rows), authored: lines.authored, featured: lines.featured };
}

/** An equipment bonus: its stats are named and formatted by an engine table the
 * recording does not hold, so they are shown as authored. */
function equipmentSection(group: EffectGroup, sources: IdeaTooltipSources): Section {
  const name = group.target ? sources.text(group.target) ?? group.target : 'Equipment';
  const rows = group.lines.filter(line => !line.hidden).map(line => ({ label: `${line.key}:`, tone: 'muted' as const,
    value: line.value == null ? line.unsupported ?? 'not evaluated' : line.fixed5 != null ? fixed5Text(BigInt(line.fixed5), -1) : `${line.value} (not read by the game)` }));
  return { rows: section(`${name}${group.instant ? ' (at once)' : ''}:`, rows), authored: rows.length > 0 };
}

/** A research bonus (`POLITICAL_RESEARCH_BONUS_ENTRY`: category, then the factor
 * as a signed whole percentage). */
function researchRows(lines: readonly EffectLine[], sources: IdeaTooltipSources): TooltipRow[] {
  return lines.filter(line => !line.hidden).map(line => {
    const label = `${sources.text(`${line.key}_research`) ?? line.key}:`;
    if (line.value == null) return { label, value: line.unsupported ?? 'not evaluated', tone: 'muted' as const };
    if (line.fixed5 == null) return { label, value: `${line.value} (not read by the game)`, tone: 'muted' as const };
    const raw = BigInt(line.fixed5);
    return { label, value: `${raw > 0n ? '+' : ''}${fixed5Text(raw * 100n, 0)}%`, tone: raw < 0n ? 'negative' as const : 'positive' as const };
  });
}

/** Every effect of an idea, in the game's order: its traits, its modifiers and
 * custom tooltips, its targeted modifiers, research bonuses, then equipment
 * bonuses. `note` says when some are shown as authored. */
export function ideaEffectRows(info: IdeaInfo, sources: IdeaTooltipSources): { rows: TooltipRow[]; note?: string } {
  // Unknown effects are said to be unknown, never shown as none.
  if (!info.effectsKnown) return { rows: section(null, [{ label: 'Effects', value: 'not in the content files', tone: 'muted' }]) };
  const sections: Section[] = [
    ...info.traits.map(found => traitSection(found, sources)),
  ];
  const own = modifierRows(info.effects, sources);
  sections.push({ rows: section(null, [...own.rows, ...info.tooltips.map(key => ({ label: sources.text(key) ?? key }))]), authored: own.authored, featured: own.featured });
  sections.push(...info.targeted.map(group => targetedSection(group, sources)));
  sections.push({ rows: section(null, researchRows(info.research, sources)), authored: false });
  sections.push(...info.equipment.map(group => equipmentSection(group, sources)));
  const rows = sections.flatMap(item => item.rows);
  const notes = [
    sections.some(item => item.authored) ? "Grey effects are shown unformatted, as the game reads the content files: the game's format for them is not recorded." : '',
    sections.some(item => item.featured) ? '* Applies only while the game feature it needs is on; the recording does not say whether it is.' : '',
  ].filter(Boolean);
  return { rows, note: notes.length ? notes.join(' ') : undefined };
}

/** Most effect rows a tooltip shows; the rest are counted, so it stays on screen. */
export const MAX_TOOLTIP_ROWS = 40;

/** An idea's game-style tooltip (mock-up politics-4): its icon, name and a line
 * saying when it was held, any rows the widget adds, its effects, then its
 * description. Countries its targeted modifiers name are named as they were at
 * `hour` (when the idea was taken on). */
export function ideaTooltip(head: { title: string; icon?: string | null; subtitle?: string }, info: IdeaInfo | undefined, sources: IdeaTooltipSources,
  description?: string | null, before: readonly TooltipRow[] = [], note?: string, hour?: number, maxRows = tooltipRowBudget()): TooltipContent {
  const effects = info ? ideaEffectRows(info, sourcesAt(sources, hour)) : { rows: [] as TooltipRow[], note: undefined };
  const rows = [...before, ...effects.rows];
  // The budget is in lines: a long label wraps (about 34 characters to a line in
  // the tooltip's width), so it takes more than one.
  // When not all fit, the count of the rest takes two lines of the budget.
  // A title or subtitle longer than the header's allowance (two lines) takes rows too.
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

/** Lines a row takes in the tooltip: its label and value share a line of about
 * 30 characters at the widest fonts, and a divider's spacing counts as one more. */
export const rowLines = (row: TooltipRow) => (row.short ? 1 : Math.max(1, Math.ceil((row.label.length + (row.value?.length ?? 0) + 3) / 30))) + (row.divider ? 1 : 0);

/** A tooltip as one line of text, every row included, for screen readers: the kit's. */
export { tooltipText } from '../charts/tooltip.ts';

/** How many effect rows fit the window under a tooltip's header, description
 * and note, so the tooltip never runs off screen. */
export function tooltipRowBudget(height = typeof window === 'undefined' ? 800 : window.innerHeight): number {
  return Math.max(4, Math.min(MAX_TOOLTIP_ROWS, Math.floor((height - 260) / 18)));
}
