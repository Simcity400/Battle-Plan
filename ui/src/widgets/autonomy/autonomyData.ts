import { clipToSpans, runAt } from '../../charts/series.ts';
import type { Run, Span } from '../../charts/series';
import { inSpans } from '../../charts/timeline/layout.ts';
import type { TimelineSpan } from '../../charts/timeline/layout';
import { formatFixed } from '../../charts/valueAxis.ts';
import { contentTitle } from '../../dashboard/contentTitles.ts';
import type { CountryIdentityIndex } from '../../dashboard/countryIdentity.ts';
import type { AutonomyLevel, AutonomyReply, AutonomyRun, AutonomySubject } from '../../generated/dashboard';

/** Subjects' autonomy (the `autonomy` product): each subject's level spans, its
 * progress between the move-down and move-up scores, and where its level changed.
 * Values stay the recorded integers; progress is a plotting number only. */

const RAW = /^-?\d{1,20}$/;

/** Decimal places of a power-of-ten scale, or null for any other scale. */
function decimalsOf(scale: number): number | null {
  const digits = Math.log10(scale);
  return Number.isInteger(digits) && digits >= 0 && 10 ** digits === scale ? digits : null;
}

/** Checks the reply's shape: levels are listed once, each run names one, raw values
 * are integers and runs are in hour order without overlap. */
export function validateAutonomyReply(data: unknown): AutonomyReply {
  const reply = data as AutonomyReply;
  const levels = Array.isArray(reply?.levels) ? reply.levels.length : 0;
  const run = (item: AutonomyRun, index: number, runs: readonly AutonomyRun[]) => Number.isInteger(item.firstHour) && Number.isInteger(item.lastHour)
    && item.firstHour <= item.lastHour && (index === 0 || runs[index - 1].lastHour < item.firstHour)
    && Number.isInteger(item.level) && item.level >= 0 && item.level < levels
    && RAW.test(item.scoreRaw) && RAW.test(item.lowerRaw) && RAW.test(item.upperRaw) && (item.ratioRaw == null || RAW.test(item.ratioRaw))
    && (item.overlord == null || typeof item.overlord === 'string');
  const ordered = (spans: unknown) => Array.isArray(spans) && spans.every((span: Span, index) => Number.isInteger(span.firstHour)
    && Number.isInteger(span.lastHour) && span.firstHour <= span.lastHour && (index === 0 || spans[index - 1].lastHour < span.firstHour));
  const ok = typeof reply === 'object' && reply != null && decimalsOf(reply.scoreScale) != null
    && reply.levels.every((level: AutonomyLevel) => typeof level.id === 'string' && (level.name == null || typeof level.name === 'string'))
    && new Set(reply.levels.map(level => level.id)).size === levels
    && Array.isArray(reply.subjects) && reply.subjects.every((subject: AutonomySubject) => typeof subject.tag === 'string' && Array.isArray(subject.runs) && subject.runs.every(run))
    && ordered(reply.recorded) && ordered(reply.unidentified)
    && Array.isArray(reply.warnings);
  if (!ok) throw new Error('The autonomy reply is malformed.');
  return reply;
}

/** An unavailable localized name stays distinct from a recorded level key. */
export const levelName = (level: AutonomyLevel | undefined) => level
  ? contentTitle(level.name, level.id, 'Level name unavailable') : 'Unknown level';

/** Where a subject's score sits between its move-down (0) and move-up (1) scores;
 * null when the two are equal, as then there is no span to sit in. Past either
 * score (a subject at the top level can pass its upper bound) it is below 0 or above 1. */
export function progressOf(run: AutonomyRun): number | null {
  const lower = BigInt(run.lowerRaw);
  const span = BigInt(run.upperRaw) - lower;
  if (span === 0n) return null;
  // Exact to 1e-9 before the one division into a plotting number.
  return Number((BigInt(run.scoreRaw) - lower) * 1_000_000_000n / span) / 1e9;
}

/** Progress as a percentage for labels, one decimal. */
export function formatProgress(run: AutonomyRun): string {
  const progress = progressOf(run);
  return progress == null ? 'Not defined (both scores are equal)' : `${(Math.round(progress * 1000) / 10).toFixed(1)}%`;
}

/** A score as the game counts it, with one decimal. */
export function formatScore(reply: AutonomyReply, raw: string): string {
  return formatFixed(raw, decimalsOf(reply.scoreScale)!, 1);
}

/** One subject, ready to draw. */
export interface SubjectLane {
  id: string;
  tag: string;
  runs: readonly AutonomyRun[];
  /** Each stretch at one level, unbroken: the lane's background bands. */
  spans: TimelineSpan[];
  /** Level index of each span, by span id. */
  spanLevels: ReadonlyMap<string, number>;
  /** Progress as runs, for the kit's curves: hours with no defined progress are gaps. */
  progress: Run[];
  /** Unbroken recorded stretches: the dashed threshold edges run along them. */
  stretches: { firstHour: number; lastHour: number }[];
  /** Hours where the level changed from the hour before (never across a gap). */
  changes: number[];
}

/** A lane: its level spans each stop the keyboard at every recorded change inside
 * them (score, thresholds or overlord: each run's start, `TimelineSpan.stops`), so
 * every value the tooltip shows is reachable, not only the level's start. */
export function subjectLane(subject: AutonomySubject & { countryId?: string }, levels: readonly AutonomyLevel[]): SubjectLane {
  const spans: (TimelineSpan & { stops: number[] })[] = [];
  const spanLevels = new Map<string, number>();
  const progress: Run[] = [];
  const stretches: { firstHour: number; lastHour: number }[] = [];
  const changes: number[] = [];
  subject.runs.forEach((run, index) => {
    const previous = subject.runs[index - 1];
    const continues = previous != null && !run.startsSegment && previous.lastHour + 1 === run.firstHour;
    const last = spans.at(-1);
    if (continues && last && spanLevels.get(last.id) === run.level) { last.lastHour = run.lastHour; last.stops.push(run.firstHour); }
    else {
      const id = `${subject.countryId ?? subject.tag}\u0000${run.firstHour}`;
      spans.push({ id, firstHour: run.firstHour, lastHour: run.lastHour, label: levelName(levels[run.level]), stops: [] });
      spanLevels.set(id, run.level);
      if (continues && previous.level !== run.level) changes.push(run.firstHour);
    }
    const stretch = stretches.at(-1);
    if (continues && stretch) stretch.lastHour = run.lastHour; else stretches.push({ firstHour: run.firstHour, lastHour: run.lastHour });
    const value = progressOf(run);
    if (value != null) progress.push({ firstHour: run.firstHour, lastHour: run.lastHour, value, breakBefore: !continues || progress.at(-1)?.lastHour !== run.firstHour - 1 });
  });
  return { id: subject.countryId ?? subject.tag, tag: subject.tag, runs: subject.runs, spans, spanLevels, progress, stretches, changes };
}

/** One subject country across its recorded tags, with overlord tag changes split
 * too. A known rename continues its line; a gap or another country still breaks it. */
export function autonomyViews(reply: AutonomyReply, identities: CountryIdentityIndex): (AutonomySubject & { countryId: string })[] {
  const holders = new Map<string, { tag: string; countryId: string; runs: (AutonomyRun & { tag: string })[] }>();
  for (const subject of reply.subjects) {
    const spans = identities.membershipRuns(subject.runs.map(run => ({ tag: subject.tag, firstHour: run.firstHour, lastHour: run.lastHour })));
    for (const span of spans) {
      const source = runAt(subject.runs, span.firstHour)!;
      const countryId = identities.keyAt(subject.tag, span.firstHour)!;
      let holder = holders.get(countryId);
      if (!holder) { holder = { tag: subject.tag, countryId, runs: [] }; holders.set(countryId, holder); }
      // Overlord lifetime changes split the operands, even if the tag stays the same.
      const overlords = source.overlord === null ? [] : identities.membershipRuns([{ ...span, tag: source.overlord }]);
      const cuts = [...new Set([span.firstHour, span.lastHour + 1, ...overlords.flatMap(part => [part.firstHour, part.lastHour + 1])])].sort((a, b) => a - b);
      for (let index = 0; index + 1 < cuts.length; index++) holder.runs.push({ ...source, tag: subject.tag,
        firstHour: cuts[index], lastHour: cuts[index + 1] - 1,
        startsSegment: source.startsSegment && cuts[index] === source.firstHour });
    }
  }
  for (const holder of holders.values()) {
    holder.runs.sort((a, b) => a.firstHour - b.firstHour);
  }
  return [...holders.values()].map(holder => ({ ...holder, runs: holder.runs.map(({ tag, ...run }, index) => {
    const previous = holder.runs[index - 1];
    const continues = previous?.lastHour === run.firstHour - 1;
    return { ...run, startsSegment: !continues || run.startsSegment && previous?.tag === tag };
  }) }));
}

/** The chosen subject countries or subjects of chosen overlord countries. A
 * country keeps its whole history across renames; a reused tag names its new holder. */
export function chosenViews(views: readonly (AutonomySubject & { countryId: string })[], countryIds: ReadonlySet<string>, bySubject: boolean,
  identities: CountryIdentityIndex): (AutonomySubject & { countryId: string })[] {
  return views.filter(subject => bySubject ? countryIds.has(subject.countryId)
    : subject.runs.some(run => run.overlord != null && countryIds.has(identities.keyAt(run.overlord, run.firstHour) ?? '')));
}

/** An unresolved overlord cannot prove a known subject falls outside the selection. */
export function overlordIdentityUnknownFor(subject: AutonomySubject, identities: CountryIdentityIndex): boolean {
  return subject.runs.some(run => run.overlord !== null && identities.keyAt(run.overlord, run.firstHour) === null);
}

/** The recorded run at an hour, or null. */
export const runOf = (lane: SubjectLane, hour: number) => runAt(lane.runs, Math.floor(hour));

/** A subject's runs cut to the hours it is shown for a smart group counted at each
 * hour (`membership`, by native key, null for none): as a chosen subject (`bySubject`), while
 * it is a member itself; else while its overlord at that hour is one (a run without an
 * overlord then shows nowhere). Hours whose membership was not recorded
 * (`unobserved`) are cut out too, as unknown, but only then: a group counted at one
 * moment (no `membership`) chose its countries there, and its gaps elsewhere cut
 * nothing. A cut starts a new line where hours are missing between the pieces. */
export function clipSubject<S extends AutonomySubject & { countryId?: string }>(subject: S, bySubject: boolean, membership: ReadonlyMap<string, readonly Span[]> | null,
  unobserved: readonly Span[], identities?: CountryIdentityIndex): S {
  let runs: AutonomyRun[] = subject.runs.slice();
  if (membership) {
    if (bySubject) runs = clipToSpans(runs, membership.get(subject.countryId ?? '') ?? []);
    else {
      // Each overlord's runs are cut by its membership in one pass (`clipToSpans` is
      // linear), never once per run: a membership counted at each hour can hold tens
      // of thousands of spans. The pieces are put back in hour order.
      const byOverlord = new Map<string, AutonomyRun[]>();
      for (const run of runs) {
        if (run.overlord == null) continue;
        const overlord = identities?.keyAt(run.overlord, run.firstHour) ?? null;
        if (overlord === null) continue;
        const held = byOverlord.get(overlord);
        if (held) held.push(run); else byOverlord.set(overlord, [run]);
      }
      runs = [...byOverlord].flatMap(([overlord, held]) => clipToSpans(held, membership.get(overlord) ?? []));
      if (byOverlord.size > 1) runs.sort((a, b) => a.firstHour - b.firstHour);
    }
  }
  if (membership && unobserved.length && runs.length) {
    // The hours around the unobserved ones, over this subject's own hours.
    const known: Span[] = [];
    let from = runs[0].firstHour;
    for (const span of [...unobserved].sort((a, b) => a.firstHour - b.firstHour)) {
      if (span.firstHour > from) known.push({ firstHour: from, lastHour: span.firstHour - 1 });
      from = Math.max(from, span.lastHour + 1);
    }
    known.push({ firstHour: from, lastHour: runs.at(-1)!.lastHour });
    runs = clipToSpans(runs, known.filter(span => span.firstHour <= span.lastHour));
  }
  return runs.length === subject.runs.length && runs.every((run, index) => run === subject.runs[index]) ? subject : { ...subject, runs };
}

/** Whether any hour of the branch was recorded without the autonomy tables: then a
 * country without a lane may be one nobody saw, not one that had no subjects. */
export function partlyRecorded(reply: AutonomyReply, branch: Span): boolean {
  const covered = reply.recorded.reduce((sum, span) => sum + Math.max(0, Math.min(span.lastHour, branch.lastHour) - Math.max(span.firstHour, branch.firstHour) + 1), 0);
  return covered < branch.lastHour - branch.firstHour + 1;
}

/** Whether some hour of `within` was recorded with a subject or overlord the
 * recording does not name: then a country without a lane may be one of them. */
export const partlyIdentified = (reply: AutonomyReply, within: Span) =>
  reply.unidentified.some(span => span.firstHour <= within.lastHour && span.lastHour >= within.firstHour);

/** What is known of a lane at an hour it has no run: never "not a subject" where
 * the group's membership, the tables or a country's identity were not recorded.
 * `text` is the tooltip's wording, `short` the level pill's. */
export function absentAt(reply: AutonomyReply, hour: number, unobserved: readonly Span[]): { text: string; short: string } {
  if (inSpans(unobserved, hour)) return { text: 'Group membership not recorded', short: 'Membership not recorded' };
  if (!inSpans(reply.recorded, hour)) return { text: 'Not recorded', short: 'Not recorded' };
  if (inSpans(reply.unidentified, hour)) return { text: 'Not known: the recording does not name every subject and overlord at this hour', short: 'Not known' };
  return { text: 'Not a subject shown here', short: 'Not a subject here' };
}

/** A lane's labels at an hour: its level, else what is known of its absence, as the
 * tooltip says it. */
export function laneLabels(reply: AutonomyReply, lane: SubjectLane, hour: number, unobserved: readonly Span[], identityRecorded = true): { label: string; pill: string; description: string } {
  if (!identityRecorded) return { label: 'Native country identity not recorded', pill: 'Not recorded', description: 'Native country identity not recorded' };
  const run = runOf(lane, hour);
  if (run) {
    const label = levelName(reply.levels[run.level]);
    return { label, pill: label, description: `${label}, progress ${formatProgress(run)}` };
  }
  const absent = absentAt(reply, hour, unobserved);
  return { label: absent.text, pill: absent.short, description: absent.text };
}

/** Whether a group's unknown membership (`unobserved`) cut any hour a lane could
 * have shown: a chosen subject's run (`bySubject`), else a run with an overlord.
 * Then a missing lane may be one the group held, not one it lacked. Linear in the
 * runs and spans. */
export function membershipUnknownFor(subjects: readonly AutonomySubject[], bySubject: boolean, unobserved: readonly Span[]): boolean {
  if (!unobserved.length) return false;
  const spans = [...unobserved].sort((a, b) => a.firstHour - b.firstHour);
  return subjects.some(subject => {
    let at = 0;
    return subject.runs.some(run => {
      if (!bySubject && run.overlord == null) return false;
      while (at < spans.length && spans[at].lastHour < run.firstHour) at++;
      return at < spans.length && spans[at].firstHour <= run.lastHour;
    });
  });
}

/** The message when no chosen country has a lane in `branch`: it claims no absence
 * the recording cannot support, naming the hours it lacks, a group's membership it
 * did not record where a lane was cut (`membershipUnknown`, `membershipUnknownFor`)
 * and the countries it could not name. `names` are the chosen overlords' names. */
export function noLanesMessage(reply: AutonomyReply, branch: Span, bySubject: boolean, names: readonly string[], membershipUnknown = false): string {
  const where = partlyRecorded(reply, branch) ? 'in the hours this campaign branch recorded' : 'in this campaign branch';
  if (membershipUnknown) {
    return bySubject
      ? `None of these countries is shown as a subject ${where}: the group’s membership was not recorded at some hours one of them was a subject.`
      : `No subjects of ${names.join(', ')} are shown ${where}: the group’s membership was not recorded at some hours they may have had subjects.`;
  }
  if (partlyIdentified(reply, branch)) {
    return bySubject
      ? `None of these countries was recorded as a subject ${where}, but at some hours the recording does not name every subject or overlord, so one of them may have been.`
      : `${names.join(', ')} had no subjects the recording names ${where}; at some hours it does not name every subject or overlord, so they may have had one.`;
  }
  return bySubject ? `None of these countries was a subject ${where}.` : `${names.join(', ')} had no subjects ${where}.`;
}
