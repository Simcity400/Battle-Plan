import { formatFixed } from '../../charts/valueAxis.ts';
import { contentTitle } from '../../dashboard/contentTitles.ts';
import type { HourSpan, SubjectLevel, SubjectReply, SubjectRun } from '../../generated/dashboard';


export interface AutonomyLane {
  tag: string;
  runs: SubjectRun[];
  spans: { id: string; firstHour: number; lastHour: number; level: string | null }[];
  within: HourSpan[];
  changes: number[];
}

export function autonomyLanes(reply: SubjectReply, chosen: ReadonlySet<string> | null, bySubject: boolean): AutonomyLane[] {
  const bySubjectTag = new Map<string, SubjectRun[]>();
  for (const run of reply.runs) {
    const held = bySubjectTag.get(run.subject);
    if (held) held.push(run); else bySubjectTag.set(run.subject, [run]);
  }
  const lanes: AutonomyLane[] = [];
  for (const [tag, runs] of bySubjectTag) {
    if (chosen && !(bySubject ? chosen.has(tag) : runs.some(run => chosen.has(run.overlord)))) continue;
    runs.sort((a, b) => a.firstHour - b.firstHour);
    const spans = runs.flatMap(run => run.steps.map(step => ({ id: `${run.overlord}@${run.startHour}@${step.firstHour}`, firstHour: step.firstHour, lastHour: step.lastHour, level: step.level })));
    const within: HourSpan[] = [];
    for (const run of runs) {
      const last = within.at(-1);
      if (last && run.firstHour <= last.lastHour + 1) last.lastHour = Math.max(last.lastHour, run.lastHour);
      else within.push({ firstHour: run.firstHour, lastHour: run.lastHour });
    }
    const changes = runs.flatMap(run => run.steps.slice(1).filter((step, at) => step.level !== run.steps[at].level).map(step => step.firstHour));
    lanes.push({ tag, runs, spans, within, changes });
  }
  return lanes.sort((a, b) => a.runs[0].firstHour - b.runs[0].firstHour || (a.tag < b.tag ? -1 : a.tag > b.tag ? 1 : 0));
}

export const runAt = (lane: AutonomyLane, hour: number) => lane.runs.find(run => run.firstHour <= hour && hour <= run.lastHour) ?? null;

export function levelAt(lane: AutonomyLane, hour: number): string | null | undefined {
  const run = runAt(lane, hour);
  return run ? run.steps.find(step => step.firstHour <= hour && hour <= step.lastHour)?.level ?? null : undefined;
}

export function levelOrder(levels: readonly SubjectLevel[]): Map<string, { index: number; lowest: boolean; highest: boolean }> {
  const ordered = levels.filter(level => level.ordered);
  return new Map(levels.map((level, index) => [level.token, {
    index, lowest: level.ordered && ordered[0]?.token === level.token, highest: level.ordered && ordered.at(-1)?.token === level.token,
  }]));
}

export const levelName = (levels: readonly SubjectLevel[], token: string | null | undefined) =>
  token == null ? 'Level not recorded' : contentTitle(levels.find(level => level.token === token)?.name, token, 'Autonomy level name unavailable');

export function formatProgress(raw: string | null | undefined, missing?: string | null): string {
  if (raw == null) return missing ?? 'Not recorded';
  return `${(Number(raw) / 1000).toFixed(1)}%`;
}

export function formatScore(raw: string | null | undefined, decimals: number | undefined, missing: string | null | undefined): string {
  if (raw == null || decimals === undefined) return missing ?? 'Not recorded';
  return formatFixed(raw, decimals, 1);
}

export function headKey(lanes: readonly AutonomyLane[], levels: readonly SubjectLevel[], hour: number, unobserved: readonly HourSpan[]): string {
  return lanes.map(lane => laneLabel(lane, levels, hour, unobserved).pill).join('|');
}

export function laneLabel(lane: AutonomyLane, levels: readonly SubjectLevel[], hour: number, unobserved: readonly HourSpan[]): { label: string; pill: string } {
  const level = levelAt(lane, hour);
  if (level !== undefined) return { label: levelName(levels, level), pill: levelName(levels, level) };
  if (unobserved.some(span => span.firstHour <= hour && hour <= span.lastHour)) return { label: 'Not recorded at this hour', pill: 'Not recorded' };
  return lane.runs[0].firstHour > hour ? { label: 'Not yet a subject', pill: 'Not a subject' } : { label: 'No longer a subject', pill: 'Not a subject' };
}

export function holderSince(holders: ReadonlyMap<string, readonly HourSpan[]> | null, tag: string, hour: number): number | null | undefined {
  if (!holders) return undefined;
  return holders.get(tag)?.find(run => run.firstHour <= hour && hour <= run.lastHour)?.firstHour ?? null;
}

export function laneIdentity<L extends { name: string }>(tag: string, since: number | null | undefined, named: L): { label: string; countryFlag: L | { flagReason: string } } {
  return since === null ? { label: tag, countryFlag: { flagReason: UNHELD } } : { label: named.name, countryFlag: named };
}

const UNHELD = 'Country unavailable at this time.';
