import { formatFixed } from '../../charts/valueAxis.ts';
import { contentTitle } from '../../dashboard/contentTitles.ts';
import type { HourSpan, SubjectLevel, SubjectReply, SubjectRun } from '../../generated/dashboard';

/** The Autonomy widget's data: one lane per subject from the `subjects` reply, its level as
 * coloured stretches, and the hours it was a subject, over which the declared `autonomy` series
 * draws its progress between the score it moves down below and the one it moves up at. */

/** One subject's lane: every run of it under any overlord, in hour order. */
export interface AutonomyLane {
  tag: string;
  runs: SubjectRun[];
  /** Its stretches of one recorded level: `level` the token, null where not recorded. */
  spans: { id: string; firstHour: number; lastHour: number; level: string | null }[];
  /** The hours it was a subject, joined where runs touch: where its progress is drawn. */
  within: HourSpan[];
  /** Hours its level changed inside a run. */
  changes: number[];
}

/** Each subject's lane in order of first run, then tag: those whose subject (`bySubject`) or
 * whose overlord at some hour is one of `chosen`, every subject when `chosen` is null. */
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

/** The run of a lane holding `hour`. */
export const runAt = (lane: AutonomyLane, hour: number) => lane.runs.find(run => run.firstHour <= hour && hour <= run.lastHour) ?? null;

/** The level a lane's subject held at `hour`: undefined when it was no subject then, null when not recorded. */
export function levelAt(lane: AutonomyLane, hour: number): string | null | undefined {
  const run = runAt(lane, hour);
  return run ? run.steps.find(step => step.firstHour <= hour && hour <= step.lastHour)?.level ?? null : undefined;
}

/** Each level's place in the content's order (least free first), and whether it is the first or last of them. */
export function levelOrder(levels: readonly SubjectLevel[]): Map<string, { index: number; lowest: boolean; highest: boolean }> {
  const ordered = levels.filter(level => level.ordered);
  return new Map(levels.map((level, index) => [level.token, {
    index, lowest: level.ordered && ordered[0]?.token === level.token, highest: level.ordered && ordered.at(-1)?.token === level.token,
  }]));
}

export const levelName = (levels: readonly SubjectLevel[], token: string | null | undefined) =>
  token == null ? 'Level not recorded' : contentTitle(levels.find(level => level.token === token)?.name, token, 'Autonomy level name unavailable');

/** A progress value as a share: fixed5, so 100,000 is the score it moves up at. */
export function formatProgress(raw: string | null | undefined): string {
  if (raw == null) return 'Not recorded';
  return `${(Number(raw) / 1000).toFixed(1)}%`;
}

/** A score as the game counts it, with one decimal: `decimals` the series' fixed-point places. */
export function formatScore(raw: string | null | undefined, decimals: number | undefined, missing: string | null | undefined): string {
  if (raw == null || decimals === undefined) return missing ?? 'Not recorded';
  return formatFixed(raw, decimals, 1);
}

/** What the lanes show at `hour` as a key: each lane's pill. The widget renders only when it changes. */
export function headKey(lanes: readonly AutonomyLane[], levels: readonly SubjectLevel[], hour: number, unobserved: readonly HourSpan[]): string {
  return lanes.map(lane => laneLabel(lane, levels, hour, unobserved).pill).join('|');
}

/** The lane's label at `hour`: its level and how it stands, or why it has none. */
export function laneLabel(lane: AutonomyLane, levels: readonly SubjectLevel[], hour: number, unobserved: readonly HourSpan[]): { label: string; pill: string } {
  const level = levelAt(lane, hour);
  if (level !== undefined) return { label: levelName(levels, level), pill: levelName(levels, level) };
  if (unobserved.some(span => span.firstHour <= hour && hour <= span.lastHour)) return { label: 'Not recorded at this hour', pill: 'Not recorded' };
  return lane.runs[0].firstHour > hour ? { label: 'Not yet a subject', pill: 'Not a subject' } : { label: 'No longer a subject', pill: 'Not a subject' };
}

/** Where `tag`'s current holder began holding it, from the recording's tag holders (`countryIdentities`): null
 * when no country holds it at `hour`, undefined while the holders are not known. Bounds a lane's name to its
 * holder at the hour shown. */
export function holderSince(holders: ReadonlyMap<string, readonly HourSpan[]> | null, tag: string, hour: number): number | null | undefined {
  if (!holders) return undefined;
  return holders.get(tag)?.find(run => run.firstHour <= hour && hour <= run.lastHour)?.firstHour ?? null;
}

/** A lane's name and flag at the hour shown: its country's as read there, or, where no country holds the lane's tag
 * then (a subject before it existed), the tag alone with why it has no flag, never another holder's name. */
export function laneIdentity<L extends { name: string }>(tag: string, since: number | null | undefined, named: L): { label: string; countryFlag: L | { flagReason: string } } {
  return since === null ? { label: tag, countryFlag: { flagReason: UNHELD } } : { label: named.name, countryFlag: named };
}

const UNHELD = 'Country unavailable at this time.';
