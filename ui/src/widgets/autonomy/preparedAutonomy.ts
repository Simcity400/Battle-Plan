import type { AutonomyReply, AutonomyRun, AutonomySubject, CountryIdentityHistory } from '../../generated/dashboard';
import { overlayCountryIdentities, validCountryIdentityHistories } from '../../dashboard/countryIdentity.ts';
import type { ChartSummary } from '../../charts/chartSummary';
import { chartSummaryRecord } from '../../charts/chartSummary.ts';
import { validateAutonomyReply } from './autonomyData.ts';
import type { Span } from '../../charts/series';
import { isOrderedSpans } from '../../charts/series.ts';
import { withoutHours } from '../../charts/timeline/runState.ts';
import { preparationCoverage } from '../../dashboard/preparedPrefix.ts';
import type { PreparedCoverage } from '../../shell/preparation/contract';

type Band = Omit<AutonomyRun, 'scoreRaw' | 'lowerRaw' | 'upperRaw' | 'ratioRaw'>;
export interface PreparedAutonomy extends AutonomyReply {
  bands: { tag: string; runs: Band[] }[];
  summaries: Record<string, ChartSummary>;
  exactHours: number[];
  identities: CountryIdentityHistory[];
  metadata: AutonomyReply;
  approximate?: readonly Span[];
  pending?: readonly Span[];
  coverage?: PreparedCoverage;
}

/** Categorical operands feed the existing holder/level helpers; numeric labels use exact subjects. */
export function validatePreparedAutonomy(value: unknown): PreparedAutonomy {
  const data = validateAutonomyReply(value) as PreparedAutonomy;
  return checkedAutonomy(data, chartSummaryRecord(data.summaries));
}
/** `data` checked with its summaries decoded (null when they were malformed), and its metadata made. */
function checkedAutonomy(data: PreparedAutonomy, summaries: Record<string, ChartSummary> | null): PreparedAutonomy {
  if (!validCountryIdentityHistories(data.identities) || !Array.isArray(data.bands) || !Array.isArray(data.exactHours) || !data.exactHours.every(Number.isSafeInteger)
    || data.approximate !== undefined && !isOrderedSpans(data.approximate) || data.pending !== undefined && !isOrderedSpans(data.pending)
    || !summaries
    || !data.bands.every(subject => subject && typeof subject.tag === 'string' && Array.isArray(subject.runs)
      && subject.runs.every(run => run && typeof run.startsSegment === 'boolean' && typeof run.lowest === 'boolean' && typeof run.highest === 'boolean')))
    throw new Error('The prepared autonomy reply is malformed.');
  const subjects: AutonomySubject[] = data.bands.map(subject => ({ tag: subject.tag, runs: subject.runs.map(run => ({ ...run,
    scoreRaw: '0', lowerRaw: '0', upperRaw: '0', ratioRaw: null })) }));
  return { ...data, summaries, metadata: validateAutonomyReply({ ...data, subjects }) };
}
/** Summary pages retain every page's unfinished history and shared prefix. */
export function mergePreparedAutonomy(a: PreparedAutonomy, b: PreparedAutonomy): PreparedAutonomy {
  return mergeAutonomy(a, b, 'pages');
}

/** Reached evidence replaces rough values at its hours, including known missing
 * values, without revising geometry's historical census. */
export function overlayReachedAutonomy(a: PreparedAutonomy, b: PreparedAutonomy): PreparedAutonomy {
  return mergeAutonomy(a, b, 'reached');
}

function mergeAutonomy(a: PreparedAutonomy, b: PreparedAutonomy, mode: 'pages' | 'reached'): PreparedAutonomy {
  const levels = [...a.levels];
  const indices = new Map(levels.map((level, index) => [level.id, index]));
  const remap = b.levels.map(level => {
    let index = indices.get(level.id);
    if (index === undefined) { index = levels.length; indices.set(level.id, index); levels.push(level); }
    else levels[index] = level;
    return index;
  });
  const reached = mode === 'reached' ? b.exactHours.map(hour => ({ firstHour: hour, lastHour: hour })).sort((x, y) => x.firstHour - y.firstHour) : [];
  const overlay = <R extends Band>(rough: readonly R[], exact: readonly R[]): R[] => {
    const cuts = [...reached, ...exact].sort((a, b) => a.firstHour - b.firstHour);
    const retained: R[] = [];
    let start = 0;
    for (const run of rough) {
      while (start < cuts.length && cuts[start].lastHour < run.firstHour) start++;
      let first = run.firstHour;
      for (let at = start; at < cuts.length && cuts[at].firstHour <= run.lastHour; at++) {
        const cut = cuts[at];
        if (first < cut.firstHour) retained.push({ ...run, firstHour: first, lastHour: cut.firstHour - 1, startsSegment: run.startsSegment && first === run.firstHour });
        first = Math.max(first, cut.lastHour + 1);
        if (first > run.lastHour) break;
      }
      if (first <= run.lastHour) retained.push({ ...run, firstHour: first, startsSegment: run.startsSegment && first === run.firstHour });
    }
    return [...retained, ...exact].sort((a, b) => a.firstHour - b.firstHour);
  };
  const mergeSubjects = <R extends Band>(rough: readonly { tag: string; runs: R[] }[], exact: readonly { tag: string; runs: R[] }[]) => {
    const patches = new Map(exact.map(subject => [subject.tag, subject.runs.map(run => ({ ...run, level: remap[run.level] }))]));
    const tags = new Set([...rough.map(subject => subject.tag), ...exact.map(subject => subject.tag)]);
    const rows = new Map(rough.map(subject => [subject.tag, subject.runs]));
    return [...tags].map(tag => ({ tag, runs: overlay(rows.get(tag) ?? [], patches.get(tag) ?? []) }));
  };
  const spans = (first: readonly Span[], second: readonly Span[]): Span[] => {
    const joined: Span[] = [];
    for (const span of [...first, ...second].sort((a, b) => a.firstHour - b.firstHour)) {
      const last = joined.at(-1);
      if (last && span.firstHour <= last.lastHour + 1) last.lastHour = Math.max(last.lastHour, span.lastHour);
      else joined.push({ ...span });
    }
    return joined;
  };
  // Geometry owns the historical census. Reached reads cannot turn its
  // sampled bands into exact history, or put an already drawn span on hold.
  const aCoverage = preparationCoverage(a), bCoverage = preparationCoverage(b);
  const coverage = mode === 'reached' ? aCoverage : aCoverage && bCoverage ? {
    ...aCoverage, firstHour: Math.max(aCoverage.firstHour, bCoverage.firstHour),
    lastHour: Math.min(aCoverage.lastHour, bCoverage.lastHour), complete: aCoverage.complete && bCoverage.complete,
  } : undefined;
  const prepared = coverage && coverage.firstHour <= coverage.lastHour ? [coverage] : [];
  const approximate = spans(a.approximate ?? [], mode === 'pages' ? b.approximate ?? [] : withoutHours(b.approximate ?? [], prepared));
  const unfinished = spans(a.pending ?? [], b.pending ?? []);
  const pending = mode === 'pages' ? unfinished : withoutHours(unfinished, spans(prepared, approximate));
  const merged = { ...a, levels, subjects: mergeSubjects(a.subjects, b.subjects), bands: mergeSubjects(a.bands, b.bands),
    // An exact read answers its hours alone: a missing read there is not the rough block's coverage.
    recorded: spans(withoutHours(a.recorded, reached), b.recorded), unidentified: spans(withoutHours(a.unidentified, reached), b.unidentified),
    warnings: [...new Set([...a.warnings, ...b.warnings])], summaries: { ...a.summaries, ...b.summaries },
    exactHours: [...new Set([...a.exactHours, ...b.exactHours])].sort((a, b) => a - b),
    coverage, approximate, pending,
    identities: overlayCountryIdentities(a.identities, b.identities) };
  // Pages are decoded already: the merge is checked as a reply is, its summaries as they are.
  return checkedAutonomy(validateAutonomyReply(merged) as PreparedAutonomy, merged.summaries);
}
