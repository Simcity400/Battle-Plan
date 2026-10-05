import type { BandStep, TimelineLane, TimelineSpan } from '../../charts/timeline/layout';
import { isSpan } from '../../charts/series.ts';
import { bridgeGaps, coverCounts, runState, stretchesOf, withoutHours } from '../../charts/timeline/runState.ts';
import { Lineage } from '../../dashboard/identity.ts';
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
  /** The subject's country (`Lineage.countryAt`). */
  subject: string;
  /** Unique within the reply: its overlord's and subject's countries (`Lineage`), as JSON. */
  key: string;
  runs: SubjectRun[];
  /** Its relations where nothing recorded says they stopped (`bridgeGaps`): runs of one
   * relation (the same start hour) joined across unrecorded hours and changes of tag.
   * What its state is said of, so a relation continued across a gap is never "ended". */
  stretches: { firstHour: number; lastHour: number; runs: SubjectRun[] }[];
}

export interface OverlordView {
  /** Unique within the reply: its country (`Lineage.countryAt`). */
  key: string;
  subjects: SubjectView[];
  /** How many subjects it held over the hours its relations were recorded: its band's
   * steps, with a gap wherever they were not. */
  counts: BandStep[];
  /** When its first subject became one, as the game dates it. */
  firstSince: number;
  /** The countries behind the tags (`Lineage`), shared by every overlord of a reply. */
  lineage: Lineage;
}

/** When a subject became one, as the game dates it: the start hour of its first run. */
const since = (view: SubjectView) => view.runs[0]?.startHour ?? Infinity;

const pair = (...parts: readonly (string | number)[]) => JSON.stringify(parts);

/** Each overlord with its subjects, from the reply's runs. Every run's overlord and
 * subject are resolved to countries through the recorded changes of tag (`Lineage`),
 * so a lane is one subject country under one overlord country, and an overlord one
 * country, however either was renamed; a tag given up and later taken by another
 * country names that other country. Linear in the runs (plus a sort and a binary
 * search per run). */
export function overlordViews(reply: SubjectReply, lineage: Lineage = new Lineage([])): OverlordView[] {
  // Each overlord country's lanes, one per subject country, in order of first run.
  const byOverlord = new Map<string, Map<string, SubjectRun[]>>();
  for (const run of reply.runs) {
    // Either tag can keep its native run while another country takes it. Split at
    // both histories' changes, keeping the levels dated to their recorded hours.
    const starts = [run.firstHour, ...new Set([
      ...lineage.changeHours(run.overlord, run.firstHour + 1, run.lastHour),
      ...lineage.changeHours(run.subject, run.firstHour + 1, run.lastHour),
    ].sort((a, b) => a - b))];
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
      const overlord = lineage.countryAt(run.overlord, firstHour);
      const subject = lineage.countryAt(run.subject, firstHour);
      let lanes = byOverlord.get(overlord);
      if (!lanes) { lanes = new Map(); byOverlord.set(overlord, lanes); }
      const runs = lanes.get(subject);
      if (runs) runs.push(piece); else lanes.set(subject, [piece]);
    }
  }
  const unobserved = [...reply.unobserved].sort((a, b) => a.firstHour - b.firstHour);
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
    key, subjects,
    counts: withoutHours(coverCounts(subjects.flatMap(view => view.runs)), unobserved),
    firstSince: subjects.reduce((least, view) => Math.min(least, since(view)), Infinity),
    lineage,
  })).sort((a, b) => firstObserved.get(a.key)! - firstObserved.get(b.key)!);
}

/** The run of a lane at an hour, else its last before, else its first. */
export function runNear(view: SubjectView, hour: number): SubjectRun {
  let found = view.runs[0];
  for (const run of view.runs) if (run.firstHour <= hour) found = run;
  return found;
}

/** An overlord's tag at an hour, through its country's whole history of tags
 * (`Lineage.tagOf`), never one another country took later. */
export function overlordTagAt(view: OverlordView, hour: number): string {
  return view.lineage.tagOf(view.key, hour);
}

/** A subject's tag at an hour, the same way: a former subject renamed after it was
 * released is named by its tag then. */
export function subjectTagAt(overlord: OverlordView, view: SubjectView, hour: number): string {
  return overlord.lineage.tagOf(view.subject, hour);
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
  country: (tag: string) => { name: string; flag?: string };
  day: (hour: number) => string;
}

export interface LaneOptions {
  order: OverlordOrder;
  subjects: SubjectOrder;
  /** Former subjects (and overlords with none left) are shown. */
  ended: boolean;
  /** Only overlords that are, or held, one of these countries; null for all. */
  countries: ReadonlySet<string> | null;
  /** Group members resolved at the hour their membership counts. */
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
  const shownSubjects = (overlord: OverlordView) => overlord.subjects.filter(view => firstHour(view.runs) <= hour
    && (options.ended || runState(view.stretches, hour, unobserved).kind !== 'ended'));
  const heldCount = (overlord: OverlordView) => overlord.counts.find(step => step.firstHour <= hour && hour <= step.lastHour)?.value ?? 0;
  // Groups count tags at their membership hour; direct choices use the moment shown.
  const chosen = options.countryIds !== undefined ? options.countryIds : options.countries && views[0]
    ? new Set([...options.countries].map(tag => views[0].lineage.countryAt(tag, hour))) : null;
  const involves = (overlord: OverlordView) => chosen!.has(overlord.key) || overlord.subjects.some(view => chosen!.has(view.subject));
  const overlords = views.filter(view => shownSubjects(view).length && (!chosen || involves(view)));
  // Each overlord's tag and subjects at the hour, found once rather than in every comparison.
  const tags = new Map(overlords.map(view => [view, overlordTagAt(view, hour)]));
  const counts = new Map(overlords.map(view => [view, heldCount(view)]));
  const name = (tag: string) => text.country(tag).name;
  const overlordName = (view: OverlordView) => name(tags.get(view)!);
  overlords.sort(options.order === 'name' ? (a, b) => overlordName(a).localeCompare(overlordName(b)) || compareTags(a.key, b.key)
    : options.order === 'subjects' ? (a, b) => counts.get(b)! - counts.get(a)! || firstHour(a.counts) - firstHour(b.counts) || compareTags(a.key, b.key)
      : (a, b) => a.firstSince - b.firstSince || compareTags(a.key, b.key));
  const unknown = unobserved.some(span => span.firstHour <= hour && hour <= span.lastHour);
  for (const overlord of overlords) {
    const country = text.country(tags.get(overlord)!);
    const count = counts.get(overlord)!;
    const headingId = laneId(overlord.key, null);
    info.set(headingId, { overlord, subject: null });
    lanes.push({
      id: headingId, heading: true, identity: { label: country.name, icon: country.flag }, icons: false, names: false,
      label: unknown ? 'Subjects not recorded' : count ? `${count} ${count === 1 ? 'subject' : 'subjects'}` : 'No subjects', muted: !unknown && !count,
      spans: stretchesOf(overlord.counts).map(stretch => ({ id: `${stretch.firstHour}`, firstHour: stretch.firstHour, lastHour: stretch.lastHour,
        label: country.name, band: stretch.steps, ...(options.bandColor ? { color: options.bandColor } : {}) })),
    });
    const subjects = shownSubjects(overlord);
    const subjectTags = new Map(subjects.map(view => [view, subjectTagAt(overlord, view, hour)]));
    const subjectName = (view: SubjectView) => name(subjectTags.get(view)!);
    subjects.sort(options.subjects === 'name' ? (a, b) => subjectName(a).localeCompare(subjectName(b)) || compareTags(a.key, b.key)
      : (a, b) => since(a) - since(b) || subjectName(a).localeCompare(subjectName(b)) || compareTags(a.key, b.key));
    for (const view of subjects) {
      const subject = text.country(subjectTags.get(view)!);
      const state = runState(view.stretches, hour, unobserved);
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
      lanes.push({ id, depth: 1, identity: { label: subject.name, icon: subject.flag }, icons: false, label, muted: state.kind === 'ended', spans });
    }
  }
  return { lanes, info };
}

/** The countries the given lanes name at `hour`, for reading their names and flags. */
export function laneCountries(lanes: readonly TimelineLane[], info: ReadonlyMap<string, LaneInfo>, hour: number): string[] {
  const tags = new Set<string>();
  for (const lane of lanes) {
    const about = info.get(lane.id);
    if (about) tags.add(about.subject ? subjectTagAt(about.overlord, about.subject, hour) : overlordTagAt(about.overlord, hour));
  }
  return [...tags];
}
