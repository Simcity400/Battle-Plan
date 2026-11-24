import { contentTitle } from '../../dashboard/contentTitles.ts';
import { runAt } from '../../charts/series.ts';
import type { TooltipRow } from '../../charts/tooltip';
import { formatFixed } from '../../charts/valueAxis.ts';
import type { FixedRun } from './types';
import type { BreakdownLine, FormulaBreakdown, ModifierBreakdown, StabilityBreakdown } from '../../generated/dashboard';

const NATIONAL_SPIRIT_TYPE = 'national_spirit';


export type Measure = 'stability' | 'warSupport';

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

