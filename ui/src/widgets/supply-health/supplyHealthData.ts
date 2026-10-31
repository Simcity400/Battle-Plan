import { countryDrillSpans } from '../../dashboard/countryIdentity.ts';
import type { SupplyHealthReply, SupplyHealthRun } from '../../generated/dashboard';
import { object, unsigned32 } from '../../dashboard/validation.ts';
import { clipToSpans, isOrderedSpans } from '../../charts/series.ts';
import { chartSummaryFrom } from '../../charts/chartSummary.ts';
import type { ChartSummary } from '../../charts/chartSummary';
import type { ShareRun } from '../../charts/share';
import { formatFixed, rescale } from '../../charts/valueAxis.ts';
import { usNumber } from '../../charts/numberFormat.ts';
import type { TooltipContent } from '../../charts/tooltip';

const text = (value: unknown): value is string => typeof value === 'string' && value.length > 0;
const count = unsigned32;
export const STATES = ['noSupply', 'strained', 'inSupply'] as const;
export const STATE_LABELS = { noSupply: 'No supply', strained: 'Strained', inSupply: 'In supply' } as const;

/** A supply reply with its prepared summaries decoded. */
export type SupplyReply = Omit<SupplyHealthReply, 'summaries'> & { summaries?: { tag: string; values: ChartSummary[] }[] };

export function validateSupplyHealthReply(value: unknown): SupplyReply {
  if (!object(value) || !text(value.branchId) || typeof value.available !== 'boolean'
    || !(value.reason === null || text(value.reason)) || !Array.isArray(value.missingInputs)
    || !value.missingInputs.every(text) || new Set(value.missingInputs).size !== value.missingInputs.length
    || !Array.isArray(value.warnings) || !value.warnings.every(item => typeof item === 'string')
    || !Array.isArray(value.series) || !value.series.every(series => object(series) && text(series.tag)
      && (series.identities === undefined || isOrderedSpans(series.identities)
        && series.identities.every(identity => unsigned32(identity.slot) && unsigned32(identity.countryId)))
      && isOrderedSpans(series.runs) && series.runs.every(run => typeof run.startsSegment === 'boolean'
        && STATES.every(state => count(run[state]))))
    || new Set(value.series.map(series => series.tag)).size !== value.series.length
    || (value.available ? value.reason !== null || value.missingInputs.length !== 0
      : !text(value.reason) || value.missingInputs.length === 0 || value.series.some(series => series.runs.length > 0))) {
    throw new Error('The supply health reply is malformed.');
  }
  // Prepared replies add each country's summaries, decoded from the dashboard's columns.
  if (value.summaries === undefined) return value as unknown as SupplyReply;
  const summaries = Array.isArray(value.summaries) ? value.summaries.map(line => {
    const values = object(line) && text(line.tag) && Array.isArray(line.values) ? line.values.map(chartSummaryFrom) : null;
    return values && values.every(summary => summary !== null) ? { ...line, values } : null;
  }) : null;
  if (!summaries || summaries.some(line => line === null)) throw new Error('The supply health reply is malformed.');
  return { ...value, summaries } as unknown as SupplyReply;
}

export function supplyShareRuns(runs: readonly SupplyHealthRun[]): ShareRun[] {
  return runs.map(run => ({ firstHour: run.firstHour, lastHour: run.lastHour,
    startsSegment: run.startsSegment, counts: STATES.map(state => run[state]) }));
}

export const totalDivisions = (run: SupplyHealthRun) => STATES.reduce((sum, state) => sum + run[state], 0);
export const formatShare = (count: number, total: number) =>
  total ? `${formatFixed(rescale(BigInt(count) * 100n, total, 2), 2, 2)}%` : 'Not known';

export function supplyTooltip(date: string, name: string, countryFlag: TooltipContent['countryFlag'], run: SupplyHealthRun | null,
  colors: readonly string[]): TooltipContent {
  if (!run) return { title: `${date} · ${name}`, countryFlag,
    rows: [{ label: 'Supply health', value: 'Not known' }], note: 'Supply was not recorded at this time.' };
  const total = totalDivisions(run);
  if (!total) return { title: `${date} · ${name}`, countryFlag, rows: [{ label: 'Divisions', value: '0' }],
    note: 'No divisions.' };
  return { title: `${date} · ${name}`, countryFlag,
    rows: [...STATES].reverse().map(state => ({ label: STATE_LABELS[state], swatch: colors[STATES.indexOf(state)],
      value: `${formatShare(run[state], total)} · ${usNumber.format(run[state])} ${run[state] === 1 ? 'division' : 'divisions'}` })) };
}

/** Supply's buckets exhaust a complete native read; a missing read has no denominator. */
export function supplyDrillHistories(reply: SupplyHealthReply) {
  return reply.series.map(series => {
    const segments: { firstHour: number; lastHour: number }[] = [];
    for (const run of series.runs) {
      const previous = segments.at(-1);
      if (previous && !run.startsSegment && previous.lastHour + 1 === run.firstHour) previous.lastHour = run.lastHour;
      else segments.push({ firstHour: run.firstHour, lastHour: run.lastHour });
    }
    return { ...series, segments };
  });
}

export function supplyDrillRuns(reply: SupplyHealthReply, tag: string, originHour: number): SupplyHealthRun[] {
  const spans = countryDrillSpans(supplyDrillHistories(reply), tag, originHour);
  return reply.series.flatMap(series => clipToSpans(series.runs, spans.get(series.tag) ?? []))
    .sort((a, b) => a.firstHour - b.firstHour);
}


/** Warnings are reported separately; counts and native identities own chart picks. */
export function supplyDrillNumbers(reply: SupplyHealthReply): SupplyHealthReply {
  return { ...reply, warnings: [] };
}
