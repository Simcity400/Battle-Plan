import { contentTitle } from '../../dashboard/contentTitles.ts';
import type { Run } from '../../charts/series';
import { runAt } from '../../charts/series.ts';
import type { TooltipRow } from '../../charts/tooltip';
import { fixedNumber, formatFixed } from '../../charts/valueAxis.ts';
import type { FixedRun } from './types';
import type { ChartSummary } from '../../charts/chartSummary';
import { rawOf, summaryEnd } from '../country-measure/measureData.ts';
import type { BreakdownLine, FormulaBreakdown, HourSpan, ModifierBreakdown, StabilityBreakdown, StabilityReadsReply } from '../../generated/dashboard';

/** The content type of a national spirit's idea category. */
const NATIONAL_SPIRIT_TYPE = 'national_spirit';

/** Stability and war support: the displayed values, in percent, from the
 * `country-stability` product, and the breakdown behind one of them. */

export type Measure = 'stability' | 'warSupport';

/** A line's end label where it ends at `end` (the last hour drawn): its value as plotted (a fraction) and as raw fixed
 * point; null where the line holds none there. */
export function stabilityEnd(summary: ChartSummary | undefined, end: number, decimals: number): { value: number; raw: string } | null {
  const value = summaryEnd(summary, end);
  return value == null ? null : { value, raw: rawOf(value, decimals) };
}

/** A fraction in fixed point (100000 = 100%) as plotting percent. */
export function percentRuns(runs: readonly FixedRun[], decimals: number): Run[] {
  return runs.map(run => ({ firstHour: run.firstHour, lastHour: run.lastHour, value: fixedNumber(run.valueRaw, decimals) * 100, breakBefore: run.startsSegment }));
}

/** An exact fraction as percent with one decimal;
 * `signed` adds a plus to positive parts. */
export function formatPercent(raw: string, decimals: number, signed = false): string {
  const text = formatFixed(BigInt(raw) * 100n, decimals, 1);
  return `${signed && BigInt(raw) > 0n ? '+' : ''}${text}%`;
}

/** The recorded value at an hour, or null when it was not recorded. */
export function rawAt(runs: readonly FixedRun[], hour: number): string | null {
  return runAt(runs, hour)?.valueRaw ?? null;
}

/** The formula terms each measure lists, with the modifier whose tree lines
 * replace the term's single row. The labels name the game's operands; the terms
 * are the engine's formula (`engine::country_stability`), not content, and the
 * line keys are its recorded modifier script names (`KEY_NAMES` there). */
const TERMS: Record<Measure, readonly { term: string; label: string; lines?: string }[]> = {
  stability: [
    { term: 'base', label: 'Base value' },
    { term: 'popularity', label: 'Party popularity' },
    { term: 'war', label: 'At war' },
    { term: 'propaganda', label: 'Enemy propaganda' },
    { term: 'coastalProtection', label: 'Coastal protection' },
    { term: 'stabilityFactor', label: 'Modifiers', lines: 'stability_factor' },
  ],
  warSupport: [
    { term: 'base', label: 'Base value' },
    { term: 'tension', label: 'World tension' },
    { term: 'offensiveWar', label: 'Offensive war' },
    { term: 'defensiveWar', label: 'Defensive war' },
    { term: 'bombing', label: 'Enemy bombing' },
    { term: 'interception', label: 'Air interception' },
    { term: 'heroes', label: 'Heroes killed' },
    { term: 'convoys', label: 'Convoys sunk' },
    { term: 'propaganda', label: 'Enemy propaganda' },
    { term: 'warSupportFactor', label: 'Modifiers', lines: 'war_support_factor' },
  ],
};

/** Most tree lines the tooltip lists in all, shared by the measures shown; the
 * rest are counted, and the drill-down shows them all. With the terms, headings
 * and totals this keeps the tooltip inside a 720-pixel window. */
export const TOOLTIP_LINE_BUDGET = 8;
/** Tree lines each measure may list when `measures` are shown. */
export const linesPerMeasure = (measures: number) => Math.max(3, Math.floor(TOOLTIP_LINE_BUDGET / Math.max(1, measures)));

/** The value axis for Zoom: both bounds fitted to the values drawn in view (in
 * percent), with a little room; a mod may allow values outside 0-100%, so
 * nothing is clamped; null when nothing is shown. */
export function zoomRange(shown: readonly [number, number] | null): [number, number] | null {
  if (!shown) return null;
  const [low, high] = shown;
  const room = Math.max(1, (high - low) * 0.1);
  return [Math.floor(low - room), Math.ceil(high + room)];
}

/** A formula term's label, as the tooltip names it. */
export function termLabel(measure: Measure, term: string): string {
  return TERMS[measure].find(item => item.term === term)?.label ?? 'Formula term name unavailable';
}

/** A tree line's name: the viewer-language label content gives it, prefixed
 * with what it is (a national spirit, a law's or advisor's slot, the leader),
 * else the game's own recorded text, else an explicit unavailable title. */
export function lineName(line: BreakdownLine): { name: string; recorded: boolean } {
  if (line.text != null && line.label == null) return { name: line.text, recorded: true };
  const name = contentTitle(line.label, line.identity, 'Source name unavailable');
  const prefix = line.groupKind === NATIONAL_SPIRIT_TYPE ? 'National spirit'
    : line.group ?? (line.kind === 'countryLeader' ? 'Country leader' : line.kind === 'ideology' ? 'Ideology' : null);
  return { name: prefix ? `${prefix}: ${name}` : name, recorded: false };
}

const abs = (raw: string) => { const value = BigInt(raw); return Number(value < 0n ? -value : value); };
const tone = (raw: string): TooltipRow['tone'] => BigInt(raw) > 0n ? 'positive' : BigInt(raw) < 0n ? 'negative' : undefined;

/** The breakdown of one measure as tooltip rows:
 * its name as a heading, each nonzero term signed, the modifier
 * term replaced by its tree lines (the game's order), whatever the lines do not
 * add up to as its own row, any limit the range imposed, and the total.
 * `recorded` tells whether a row shows the game's own text. */
export function breakdownRows(breakdown: StabilityBreakdown, measure: Measure, decimals: number, heading: string, maxLines = TOOLTIP_LINE_BUDGET): { rows: TooltipRow[]; recorded: boolean; unavailable: boolean } {
  const formula: FormulaBreakdown = breakdown[measure];
  const rows: TooltipRow[] = [{ label: heading, strong: true }];
  let recorded = false;
  let unavailable = false;
  for (const item of TERMS[measure]) {
    const term = formula.terms.find(entry => entry.term === item.term);
    if (!term) continue;
    if (term.valueRaw == null) {
      rows.push({ label: item.label, value: 'Not recorded', tone: 'muted' });
      unavailable = true;
      continue;
    }
    if (item.lines) {
      const modifier: ModifierBreakdown | undefined = breakdown.modifiers.find(entry => entry.key === item.lines);
      if (breakdown.linesAvailable && modifier) {
        // The largest lines, in the game's order; the others are counted.
        const largest = new Set([...modifier.lines].sort((a, b) => abs(b.valueRaw) - abs(a.valueRaw) || a.line - b.line).slice(0, maxLines));
        let hidden = 0n;
        for (const line of [...modifier.lines].sort((a, b) => a.line - b.line)) {
          if (!largest.has(line)) { hidden += BigInt(line.valueRaw); continue; }
          const named = lineName(line);
          recorded ||= named.recorded;
          rows.push({ label: named.name, value: formatPercent(line.valueRaw, decimals, true), tone: named.recorded ? 'muted' : tone(line.valueRaw), short: true });
        }
        const more = modifier.lines.length - largest.size;
        if (more > 0) rows.push({ label: `${more} more ${more === 1 ? 'line' : 'lines'}`, value: formatPercent(hidden.toString(), decimals, true), tone: 'muted' });
        if (modifier.remainderRaw != null) rows.push({ label: 'Other', value: formatPercent(modifier.remainderRaw, decimals, true), tone: 'muted' });
        continue;
      }
      if (term.valueRaw !== '0') {
        rows.push({ label: item.label, value: formatPercent(term.valueRaw, decimals, true), tone: tone(term.valueRaw) });
        unavailable ||= !breakdown.linesAvailable;
      }
      continue;
    }
    if (term.valueRaw === '0' && item.term !== 'base') continue;
    rows.push({ label: item.label, value: formatPercent(term.valueRaw, decimals, true), tone: tone(term.valueRaw) });
  }
  // Unknown formula terms keep their exact facts and expose keys in hover details.
  for (const term of formula.terms) {
    if (TERMS[measure].some(item => item.term === term.term)) continue;
    if (term.valueRaw == null) {
      rows.push({ label: termLabel(measure, term.term), lookupKey: term.term, value: 'Not recorded', tone: 'muted' });
      unavailable = true;
    } else {
      rows.push({ label: termLabel(measure, term.term), lookupKey: term.term, value: term.valueRaw });
    }
  }
  if (formula.clampRaw != null && formula.clampRaw !== '0') rows.push({ label: 'Limited to the allowed range', value: formatPercent(formula.clampRaw, decimals, true), tone: 'muted' });
  rows.push({ label: 'Total', value: formula.valueRaw == null ? 'Not recorded' : formatPercent(formula.valueRaw, decimals), strong: true, tone: formula.valueRaw == null ? 'muted' : undefined });
  return { rows, recorded, unavailable };
}

/** The stability inputs' read hours over a span (`stability-reads`). */
export function validateStabilityReads(value: unknown): StabilityReadsReply {
  const reply = value as StabilityReadsReply;
  if (typeof reply !== 'object' || reply === null || !Array.isArray(reply.reads) || !Array.isArray(reply.warnings)
    || !reply.reads.every((span, index) => Number.isSafeInteger(span.firstHour) && Number.isSafeInteger(span.lastHour) && span.firstHour <= span.lastHour
      && (index === 0 || reply.reads[index - 1].lastHour < span.firstHour))) throw new Error('The stability reads reply is malformed.');
  return reply;
}

/** The hour of the read behind `hour`'s values: the hour itself when the inputs were read then, else the last read
 * before it; null before the first read. */
export function readHourOf(reads: readonly HourSpan[], hour: number): number | null {
  let low = 0, high = reads.length - 1, found = -1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    if (reads[middle].firstHour <= hour) { found = middle; low = middle + 1; } else high = middle - 1;
  }
  return found < 0 ? null : Math.min(hour, reads[found].lastHour);
}
