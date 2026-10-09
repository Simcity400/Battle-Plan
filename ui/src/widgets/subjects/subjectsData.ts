import type { CountryFlagLook } from '../../components/CountryFlag';
import type { BandStep, TimelineLane, TimelineSpan } from '../../charts/timeline/layout';
import { isSpan } from '../../charts/series.ts';
import { bridgeGaps, coverCounts, runState, stretchesOf, withoutHours } from '../../charts/timeline/runState.ts';
import { Lineage } from '../../dashboard/identity.ts';
import type { CountryIdentityIndex } from '../../dashboard/countryIdentity.ts';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { HourRange } from '../../charts/timeline/runState';
import type { HourSpan, SubjectLevel, SubjectReply, SubjectRun } from '../../generated/dashboard';

/** The Subjects widget's data: an overlord tree from the `subjects` product, each
 * overlord heading the subjects it held, with a bar per relation coloured by the
 * subject's level. Levels, their names and their order come from the recording's
 * content (autonomy states); colours are the categorical palette in that order,
 * never keyed on a level's name, so a mod's levels work. */

const isObject = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value != null;
const listOf = (value: unknown, check: (item: unknown) => boolean) => Array.isArray(value) && value.every(check);
const isText = (value: unknown) => typeof value === 'string';

/** Checks a `subjects` reply's shape: every level, run and step, so a reply from
 * another version is refused rather than read wrongly. A run's steps must cover it
 * in order, and every level a step names must be listed. */
export function validateSubjectReply(data: unknown): SubjectReply {
  const reply = data as Record<string, unknown>;
  const levels = isObject(reply) && listOf(reply.levels, level => isObject(level) && isText(level.token)
    && (level.name === null || isText(level.name)) && typeof level.ordered === 'boolean')
    ? new Set((reply.levels as SubjectLevel[]).map(level => level.token)) : null;
  const ok = isObject(reply) && isText(reply.branchId) && levels != null
    && listOf(reply.warnings, isText) && listOf(reply.unobserved, isSpan)
    && listOf(reply.runs, run => isSpan(run) && isObject(run) && isText(run.overlord) && isText(run.subject) && Number.isInteger(run.startHour)
      && listOf(run.steps, step => isSpan(step) && isObject(step) && (step.level === null || (isText(step.level) && levels.has(step.level as string))))
      && (run.steps as HourRange[]).every((step, index, steps) => step.firstHour === (index ? steps[index - 1].lastHour + 1 : run.firstHour))
      && (run.steps as HourRange[]).at(-1)?.lastHour === run.lastHour);
  if (!ok) throw new Error('The subjects reply is malformed.');
  return data as SubjectReply;
}

export type OverlordOrder = 'first' | 'subjects' | 'name';
export type SubjectOrder = 'since' | 'name';

/** One subject lane: a subject under an overlord, every recorded run of it in hour
 * order, joined across a change of either country's tag (the runs then name other
 * tags), so a tag switch is never read as one subject released and another taken. */
export interface SubjectView {
  /** The subject's native holder when the catalogue is supplied. */
  subject: string;
  /** Unique within the reply: its overlord and subject holders, as JSON. */
  key: string;
  runs: SubjectRun[];
  /** Its relations where nothing recorded says they stopped (`bridgeGaps`): runs of one
   * relation (the same start hour) joined across unrecorded hours and changes of tag.
   * What its state is said of, so a relation continued across a gap is never "ended". */
  stretches: { firstHour: number; lastHour: number; runs: SubjectRun[] }[];
}

export interface OverlordView {
  /** Native holders proved on either endpoint of this overlord's relations. */
  countryIds: ReadonlySet<string>;
  /** Unique within the reply: its overlord holder. */
  key: string;
  subjects: SubjectView[];
  /** How many subjects it held over the hours its relations were recorded: its band's
   * steps, with a gap wherever they were not. */
  counts: BandStep[];
  /** When its first subject became one, as the game dates it. */
  firstSince: number;
  /** Label history for the legacy helper path without a native catalogue. */
  lineage: Lineage;
  identities?: CountryIdentityIndex;
  /** Relation hours whose native endpoints could not be attributed. */
  identityGaps: HourSpan[];
  /** Missing relation reads or native endpoints; shared by this reply's views. */
  unknown: HourSpan[];
}

/** When a subject became one, as the game dates it: the start hour of its first run. */
const since = (view: SubjectView) => view.runs[0]?.startHour ?? Infinity;

const pair = (...parts: readonly (string | number)[]) => JSON.stringify(parts);

/** Each native overlord with its native subject holders, splitting at either
 * endpoint's recorded identity boundaries. The legacy helper path is used only
 * when no native catalogue is supplied. */
export function overlordViews(reply: SubjectReply, lineage: Lineage = new Lineage([]), identities?: CountryIdentityIndex): OverlordView[] {
  // Each overlord country's lanes, one per subject country, in order of first run.
  const byOverlord = new Map<string, Map<string, SubjectRun[]>>();
  const identityGaps: HourSpan[] = [];
  for (const run of reply.runs) {
    // Either tag can keep its native run while another country takes it. Split at
    // both histories' changes, keeping the levels dated to their recorded hours.
    const native = identities?.membershipRuns([{ tag: run.overlord, firstHour: run.firstHour, lastHour: run.lastHour },
      { tag: run.subject, firstHour: run.firstHour, lastHour: run.lastHour }]);
    const starts = [...new Set([run.firstHour, ...(native ? native.flatMap(span => [span.firstHour, span.lastHour + 1]) : [
      ...lineage.changeHours(run.overlord, run.firstHour + 1, run.lastHour),
      ...lineage.changeHours(run.subject, run.firstHour + 1, run.lastHour),
    ])])].filter(hour => hour <= run.lastHour).sort((a, b) => a - b);
    let stepIndex = 0;
    for (let index = 0; index < starts.length; index++) {
      const firstHour = starts[index];
      const lastHour = (starts[index + 1] ?? run.lastHour + 1) - 1;
      const steps: SubjectRun['steps'] = [];
      while (starts.length > 1 && stepIndex < run.steps.length && run.steps[stepIndex].firstHour <= lastHour) {
        const step = run.steps[stepIndex];
        steps.push({ ...step, firstHour: Math.max(firstHour, step.firstHour), lastHour: Math.min(lastHour, step.lastHour) });
        if (step.lastHour > lastHour) break;
        stepIndex++;
      }
      const piece = starts.length === 1 ? run : { ...run, firstHour, lastHour, steps };
      const overlord = identities ? identities.keyAt(run.overlord, firstHour) : lineage.countryAt(run.overlord, firstHour);
      const subject = identities ? identities.keyAt(run.subject, firstHour) : lineage.countryAt(run.subject, firstHour);
      if (overlord === null || subject === null) { identityGaps.push({ firstHour, lastHour }); continue; }
      let lanes = byOverlord.get(overlord);
      if (!lanes) { lanes = new Map(); byOverlord.set(overlord, lanes); }
      const runs = lanes.get(subject);
      if (runs) runs.push(piece); else lanes.set(subject, [piece]);
    }
  }
  const unobserved: HourSpan[] = [];
  for (const span of [...reply.unobserved, ...identityGaps].sort((a, b) => a.firstHour - b.firstHour)) {
    const last = unobserved.at(-1);
    if (last && span.firstHour <= last.lastHour + 1) last.lastHour = Math.max(last.lastHour, span.lastHour);
    else unobserved.push({ ...span });
  }
  const groups = new Map<string, SubjectView[]>();
  const firstObserved = new Map<string, number>();
  for (const [overlord, lanes] of byOverlord) {
    groups.set(overlord, [...lanes].map(([subject, runs]) => ({
      key: pair(overlord, subject), runs, subject,
      stretches: bridgeGaps(runs.sort((a, b) => a.firstHour - b.firstHour), unobserved, (earlier, later) => earlier.startHour === later.startHour),
    })));
    firstObserved.set(overlord, [...lanes.values()].reduce((first, runs) => Math.min(first, runs[0].firstHour), Infinity));
  }
  return [...groups].map(([key, subjects]) => ({
    key, subjects, countryIds: identities?.keysDuring(subjects.flatMap(subject => subject.runs.flatMap(run => [
      { tag: run.overlord, firstHour: run.firstHour, lastHour: run.lastHour },
      { tag: run.subject, firstHour: run.firstHour, lastHour: run.lastHour },
    ]))) ?? new Set(),
    counts: withoutHours(coverCounts(subjects.flatMap(view => view.runs)), unobserved),
    firstSince: subjects.reduce((least, view) => Math.min(least, since(view)), Infinity),
    lineage, identities, identityGaps, unknown: unobserved,
  })).sort((a, b) => firstObserved.get(a.key)! - firstObserved.get(b.key)!);
}

/** The run of a lane at an hour, else its last before, else its first. */
export function runNear(view: SubjectView, hour: number): SubjectRun {
  let found = view.runs[0];
  for (const run of view.runs) if (run.firstHour <= hour) found = run;
  return found;
}

/** An overlord's proved alias at this hour; a native gap never acquires a tag. */
export function overlordTagAt(view: OverlordView, hour: number): string | null {
  return view.identities ? view.identities.aliasesAt(new Set([view.key]), hour)[0] ?? null : view.lineage.tagOf(view.key, hour);
}

/** A subject's tag at an hour, the same way: a former subject renamed after it was
 * released is named by its tag then. */
export function subjectTagAt(overlord: OverlordView, view: SubjectView, hour: number): string | null {
  return overlord.identities ? overlord.identities.aliasesAt(new Set([view.subject]), hour)[0] ?? null : overlord.lineage.tagOf(view.subject, hour);
}

/** The colour of each level: the categorical palette in the levels' order (least free
 * first, as content orders them), so the same level keeps its colour in every widget
 * reading these levels; a level past the palette shares the `other` colour. */
export function levelColours(levels: readonly SubjectLevel[], palette: readonly string[], other: string): Map<string, string> {
  return new Map(levels.map((level, index) => [level.token, palette[index] ?? other]));
}

/** A level's name as shown: the game's, else its id, so a missing name is never blank. */
export const levelName = (levels: ReadonlyMap<string, SubjectLevel>, token: string | null) =>
  token == null ? 'Level not recorded' : levels.get(token)?.name ?? token;

/** The level a run's subject held at an hour; undefined outside the run. */
export function levelAt(run: SubjectRun, hour: number): string | null | undefined {
  const step = run.steps.find(item => item.firstHour <= hour && hour <= item.lastHour);
  return step ? step.level : undefined;
}

export interface SubjectText {
  country: (tag: string) => CountryFlagLook & { name: string };
  day: (hour: number) => string;
}

export interface LaneOptions {
  order: OverlordOrder;
  subjects: SubjectOrder;
  /** Former subjects (and overlords with none left) are shown. */
  ended: boolean;
  /** Only overlords that are, or held, one of these countries; null for all. */
  countries: ReadonlySet<string> | null;
  /** Manual all-ever holders or group holders at their counted membership dates. */
  countryIds?: ReadonlySet<string> | null;
  /** The neutral colour of an overlord's band (a theme colour, as the mock-up's war bars). */
  bandColor?: string;
}

/** What a lane is about: an overlord, and for a subject lane the subject. */
export interface LaneInfo { overlord: OverlordView; subject: SubjectView | null }

/** A lane's id: structured, so no tag can blur two lanes, and never parsed back. */
export const laneId = (overlord: string, subject: string | null) => JSON.stringify(subject == null ? ['overlord', overlord] : ['subject', overlord, subject]);

const firstHour = (runs: readonly HourRange[]) => runs[0]?.firstHour ?? Infinity;

/** The timeline's lanes at `hour` (the playhead, or the end with Live off): each
 * overlord that had a subject by then, heading a band as thick as its number of
 * subjects, and a lane per subject beneath it, indented, with a bar per stretch of
 * one level. Only what has started by `hour` is listed, so lanes appear as playback
 * reaches them. Countries are named by the tag they had at `hour`. */
export function subjectLanes(views: readonly OverlordView[], levels: ReadonlyMap<string, SubjectLevel>, colours: ReadonlyMap<string, string>,
  hour: number, options: LaneOptions, text: SubjectText, unobserved: readonly HourSpan[]): { lanes: TimelineLane[]; info: Map<string, LaneInfo> } {
  const lanes: TimelineLane[] = [];
  const info = new Map<string, LaneInfo>();
  const missing = views[0]?.identityGaps ?? [];
  const unread = missing.length ? [...unobserved, ...missing] : unobserved;
  const shownSubjects = (overlord: OverlordView) => overlord.subjects.filter(view => firstHour(view.runs) <= hour
    && (options.ended || runState(view.stretches, hour, unread).kind !== 'ended'));
  const heldCount = (overlord: OverlordView) => overlord.counts.find(step => step.firstHour <= hour && hour <= step.lastHour)?.value ?? 0;
  const chosen = options.countryIds ?? (options.countries === null ? null : new Set<string>());
  const involves = (overlord: OverlordView) => { for (const key of overlord.countryIds) if (chosen!.has(key)) return true; return false; };
  const overlords = views.filter(view => shownSubjects(view).length && (!chosen || involves(view)));
  // Each overlord's tag and subjects at the hour, found once rather than in every comparison.
  const tags = new Map(overlords.map(view => [view, overlordTagAt(view, hour)]));
  const counts = new Map(overlords.map(view => [view, heldCount(view)]));
  const countryAt = (tag: string | null) => tag === null ? { name: 'Native country identity not recorded', flagReason: 'Native country identity not recorded at this hour.' } : text.country(tag);
  const name = (tag: string | null) => countryAt(tag).name;
  const overlordName = (view: OverlordView) => name(tags.get(view) ?? null);
  overlords.sort(options.order === 'name' ? (a, b) => overlordName(a).localeCompare(overlordName(b)) || compareTags(a.key, b.key)
    : options.order === 'subjects' ? (a, b) => counts.get(b)! - counts.get(a)! || firstHour(a.counts) - firstHour(b.counts) || compareTags(a.key, b.key)
      : (a, b) => a.firstSince - b.firstSince || compareTags(a.key, b.key));
  for (const overlord of overlords) {
    const unknown = unread.some(span => span.firstHour <= hour && hour <= span.lastHour);
    const country = countryAt(tags.get(overlord) ?? null);
    const count = counts.get(overlord)!;
    const headingId = laneId(overlord.key, null);
    info.set(headingId, { overlord, subject: null });
    lanes.push({
      id: headingId, heading: true, identity: { label: country.name, countryFlag: country }, icons: false, names: false,
      label: unknown ? 'Subjects not recorded' : count ? `${count} ${count === 1 ? 'subject' : 'subjects'}` : 'No subjects', muted: !unknown && !count,
      spans: stretchesOf(overlord.counts).map(stretch => ({ id: `${stretch.firstHour}`, firstHour: stretch.firstHour, lastHour: stretch.lastHour,
        label: country.name, band: stretch.steps, ...(options.bandColor ? { color: options.bandColor } : {}) })),
    });
    const subjects = shownSubjects(overlord);
    const subjectTags = new Map(subjects.map(view => [view, subjectTagAt(overlord, view, hour)]));
    const subjectName = (view: SubjectView) => name(subjectTags.get(view) ?? null);
    subjects.sort(options.subjects === 'name' ? (a, b) => subjectName(a).localeCompare(subjectName(b)) || compareTags(a.key, b.key)
      : (a, b) => since(a) - since(b) || subjectName(a).localeCompare(subjectName(b)) || compareTags(a.key, b.key));
    for (const view of subjects) {
      const subject = countryAt(subjectTags.get(view) ?? null);
      const state = runState(view.stretches, hour, unread);
      const run = runNear(view, hour);
      const level = state.kind === 'held' ? levelAt(run, hour) : undefined;
      // A relation continued across unrecorded hours is held, at an unknown level then.
      const label = state.kind === 'held' ? `${levelName(levels, level ?? null)} · since ${text.day(run.startHour)}`
        : state.kind === 'ended' ? (state.lastSeen == null ? `Ended ${text.day(state.at)}` : `Ended by ${text.day(state.at)}`)
          : state.kind === 'unknown' ? `Last recorded ${text.day(state.lastSeen)}` : '';
      // A bar per stretch of one level; a stretch whose level was not recorded is uncoloured.
      const spans: TimelineSpan[] = view.runs.flatMap(item => item.steps.map(step => ({
        id: `${item.startHour}@${step.firstHour}`, firstHour: step.firstHour, lastHour: step.lastHour, label: levelName(levels, step.level),
        ...(step.level != null && colours.get(step.level) ? { color: colours.get(step.level) } : {}),
      })));
      const id = laneId(overlord.key, view.key);
      info.set(id, { overlord, subject: view });
      lanes.push({ id, depth: 1, identity: { label: subject.name, countryFlag: subject }, icons: false, label, muted: state.kind === 'ended', spans });
    }
  }
  return { lanes, info };
}

/** The countries the given lanes name at `hour`, for reading their names and flags. */
export function laneCountries(lanes: readonly TimelineLane[], info: ReadonlyMap<string, LaneInfo>, hour: number): string[] {
  const tags = new Set<string>();
  for (const lane of lanes) {
    const about = info.get(lane.id);
    if (about) {
      const tag = about.subject ? subjectTagAt(about.overlord, about.subject, hour) : overlordTagAt(about.overlord, hour);
      if (tag !== null) tags.add(tag);
    }
  }
  return [...tags];
}
