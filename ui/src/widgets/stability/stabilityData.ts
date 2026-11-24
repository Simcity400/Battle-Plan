import { contentTitle } from '../../dashboard/contentTitles.ts';
import type { Run } from '../../charts/series';
import { runAt } from '../../charts/series.ts';
import type { TooltipRow } from '../../charts/tooltip';
import { fixedNumber, formatFixed } from '../../charts/valueAxis.ts';
import type { FixedRun } from './types';
import type { ChartSummary } from '../../charts/chartSummary';
import { rawOf, summaryEnd } from '../country-measure/measureData.ts';
import type { BreakdownLine, FormulaBreakdown, HourSpan, ModifierBreakdown, StabilityBreakdown, StabilityReadsReply } from '../../generated/dashboard';

const NATIONAL_SPIRIT_TYPE = 'national_spirit';


export type Measure = 'stability' | 'warSupport';

export function stabilityEnd(summary: ChartSummary | undefined, end: number, decimals: number): { value: number; raw: string } | null {
  const value = summaryEnd(summary, end);
  return value == null ? null : { value, raw: rawOf(value, decimals) };
}

export function stabilityEndLabels(
  frame: { readonly x: readonly [number, number]; readonly plotHeight: number; toY(value: number): number },
  until: number | null,
  extentEnd: number,
  lines: readonly { readonly tag: string; readonly hex: string }[],
  shown: readonly { readonly key: Measure; readonly label: string }[],
  lineOf: (measure: Measure, tag: string) => ChartSummary | undefined,
  labelAt: (hour: number) => (tag: string) => { readonly name: string },
  decimals: number,
  endLabelHeight: number,
  endLabelGap: number,
): { readonly ends: readonly { id: string; y: number; text: string; color: string }[]; readonly gap: number } | readonly [] {
  const end = Math.min(until ?? extentEnd, frame.x[1], extentEnd);
  if (end < frame.x[0]) return [];
  const ends = lines.flatMap(item => shown.flatMap(measure => {
    const at = stabilityEnd(lineOf(measure.key, item.tag), end, decimals);
    if (!at) return [];
    const text = `${lines.length > 1 ? `${labelAt(end)(item.tag).name}: ` : ''}${measure.label} ${formatPercent(at.raw, decimals)}`;
    return [{ id: `${item.tag}:${measure.key}`, y: frame.toY(at.value), text, color: item.hex }];
  }));
  const room = Math.floor((frame.plotHeight + endLabelGap) / (endLabelHeight + endLabelGap));
  const fitted = ends.length <= room ? ends : ends.filter(item => item.id.endsWith(':stability'));
  if (fitted.length > room) return [];
  return { ends: fitted, gap: endLabelGap };
}

export function percentRuns(runs: readonly FixedRun[], decimals: number): Run[] {
  return runs.map(run => ({ firstHour: run.firstHour, lastHour: run.lastHour, value: fixedNumber(run.valueRaw, decimals) * 100, breakBefore: run.startsSegment }));
}

export function formatPercent(raw: string, decimals: number, signed = false): string {
  const text = formatFixed(BigInt(raw) * 100n, decimals, 1);
  return `${signed && BigInt(raw) > 0n ? '+' : ''}${text}%`;
}

export function rawAt(runs: readonly FixedRun[], hour: number): string | null {
  return runAt(runs, hour)?.valueRaw ?? null;
}

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

export const TOOLTIP_LINE_BUDGET = 8;
export const linesPerMeasure = (measures: number) => Math.max(3, Math.floor(TOOLTIP_LINE_BUDGET / Math.max(1, measures)));

export function zoomRange(shown: readonly [number, number] | null): [number, number] | null {
  if (!shown) return null;
  const [low, high] = shown;
  const room = Math.max(1, (high - low) * 0.1);
  return [Math.floor(low - room), Math.ceil(high + room)];
}

export function termLabel(measure: Measure, term: string): string {
  return TERMS[measure].find(item => item.term === term)?.label ?? 'Formula term name unavailable';
}

export function lineName(line: BreakdownLine): { name: string; recorded: boolean } {
  if (line.text != null && line.label == null) return { name: line.text, recorded: true };
  const name = contentTitle(line.label, line.identity, 'Source name unavailable');
  const prefix = line.groupKind === NATIONAL_SPIRIT_TYPE ? 'National spirit'
    : line.group ?? (line.kind === 'countryLeader' ? 'Country leader' : line.kind === 'ideology' ? 'Ideology' : null);
  return { name: prefix ? `${prefix}: ${name}` : name, recorded: false };
}

const abs = (raw: string) => { const value = BigInt(raw); return Number(value < 0n ? -value : value); };
const tone = (raw: string): TooltipRow['tone'] => BigInt(raw) > 0n ? 'positive' : BigInt(raw) < 0n ? 'negative' : undefined;

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

export function validateStabilityReads(value: unknown): StabilityReadsReply {
  const reply = value as StabilityReadsReply;
  if (typeof reply !== 'object' || reply === null || !Array.isArray(reply.reads) || !Array.isArray(reply.warnings)
    || !reply.reads.every((span, index) => Number.isSafeInteger(span.firstHour) && Number.isSafeInteger(span.lastHour) && span.firstHour <= span.lastHour
      && (index === 0 || reply.reads[index - 1].lastHour < span.firstHour))) throw new Error('The stability reads reply is malformed.');
  return reply;
}

export function readHourOf(reads: readonly HourSpan[], hour: number): number | null {
  let low = 0, high = reads.length - 1, found = -1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    if (reads[middle].firstHour <= hour) { found = middle; low = middle + 1; } else high = middle - 1;
  }
  return found < 0 ? null : Math.min(hour, reads[found].lastHour);
}
