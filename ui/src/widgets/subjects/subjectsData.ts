import { contentTitle } from '../../dashboard/contentTitles.ts';
import type { CountryFlagLook } from '../../components/CountryFlag';
import type { BandStep, TimelineLane, TimelineSpan } from '../../charts/timeline/layout';
import { isSpan } from '../../charts/series.ts';
import { bridgeGaps, coverCounts, runState, stretchesOf, withoutHours } from '../../charts/timeline/runState.ts';
import { Lineage } from '../../dashboard/identity.ts';
import type { CountryIdentityIndex } from '../../dashboard/countryIdentity.ts';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { HourRange } from '../../charts/timeline/runState';
import type { HourSpan, SubjectLevel, SubjectReply, SubjectRun } from '../../generated/dashboard';


const isObject = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value != null;
const listOf = (value: unknown, check: (item: unknown) => boolean) => Array.isArray(value) && value.every(check);
const isText = (value: unknown) => typeof value === 'string';

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

export interface SubjectView {
  subject: string;
  key: string;
  runs: SubjectRun[];
  stretches: { firstHour: number; lastHour: number; runs: SubjectRun[] }[];
}

export interface OverlordView {
  countryIds: ReadonlyMap<string, number>;
  key: string;
  subjects: SubjectView[];
  counts: BandStep[];
  firstSince: number;
  lineage: Lineage;
  identities?: CountryIdentityIndex;
  identityGaps: HourSpan[];
  unknown: HourSpan[];
}

const since = (view: SubjectView) => view.runs[0]?.startHour ?? Infinity;

const pair = (...parts: readonly (string | number)[]) => JSON.stringify(parts);

export function overlordViews(reply: SubjectReply, lineage: Lineage = new Lineage([]), identities?: CountryIdentityIndex): OverlordView[] {
  const byOverlord = new Map<string, Map<string, SubjectRun[]>>();
  const identityGaps: HourSpan[] = [];
  for (const run of reply.runs) {
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
    key, subjects, countryIds: identities?.firstHoursDuring(subjects.flatMap(subject => subject.runs.flatMap(run => [
      { tag: run.overlord, firstHour: run.firstHour, lastHour: run.lastHour },
      { tag: run.subject, firstHour: run.firstHour, lastHour: run.lastHour },
    ]))) ?? new Map(),
    counts: withoutHours(coverCounts(subjects.flatMap(view => view.runs)), unobserved),
    firstSince: subjects.reduce((least, view) => Math.min(least, since(view)), Infinity),
    lineage, identities, identityGaps, unknown: unobserved,
  })).sort((a, b) => firstObserved.get(a.key)! - firstObserved.get(b.key)!);
}

export function runNear(view: SubjectView, hour: number): SubjectRun {
  let found = view.runs[0];
  for (const run of view.runs) if (run.firstHour <= hour) found = run;
  return found;
}

export function overlordTagAt(view: OverlordView, hour: number): string | null {
  return view.identities ? view.identities.aliasesAt(new Set([view.key]), hour)[0] ?? null : view.lineage.tagOf(view.key, hour);
}

export function subjectTagAt(overlord: OverlordView, view: SubjectView, hour: number): string | null {
  return overlord.identities ? overlord.identities.aliasesAt(new Set([view.subject]), hour)[0] ?? null : overlord.lineage.tagOf(view.subject, hour);
}

export function levelColours(levels: readonly SubjectLevel[], palette: readonly string[], other: string): Map<string, string> {
  return new Map(levels.map((level, index) => [level.token, palette[index] ?? other]));
}

export const levelName = (levels: ReadonlyMap<string, SubjectLevel>, token: string | null) =>
  token == null ? 'Level not recorded' : contentTitle(levels.get(token)?.name, token, 'Autonomy level name unavailable');

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
  ended: boolean;
  countries: ReadonlySet<string> | null;
  countryIds?: ReadonlySet<string> | null;
  bandColor?: string;
}

export interface LaneInfo { overlord: OverlordView; subject: SubjectView | null }

export const laneId = (overlord: string, subject: string | null) => JSON.stringify(subject == null ? ['overlord', overlord] : ['subject', overlord, subject]);

const firstHour = (runs: readonly HourRange[]) => runs[0]?.firstHour ?? Infinity;

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
  const involves = (overlord: OverlordView) => { for (const [key, since] of overlord.countryIds) if (since <= hour && chosen!.has(key)) return true; return false; };
  const overlords = views.filter(view => shownSubjects(view).length && (!chosen || involves(view)));
  const tags = new Map(overlords.map(view => [view, overlordTagAt(view, hour)]));
  const counts = new Map(overlords.map(view => [view, heldCount(view)]));
  const countryAt = (tag: string | null) => tag === null ? { name: 'Unknown country', flagReason: 'Unknown country at this hour.' } : text.country(tag);
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
      const label = state.kind === 'held' ? `${levelName(levels, level ?? null)} · Since ${text.day(run.startHour)}`
        : state.kind === 'ended' ? (state.lastSeen == null ? `Ended ${text.day(state.at)}` : `Ended by ${text.day(state.at)}`)
          : state.kind === 'unknown' ? `Last recorded ${text.day(state.lastSeen)}` : '';
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

export function changeHours(reply: Pick<SubjectReply, 'runs' | 'unobserved'>, identityChanges: readonly { hour: number }[] = []): number[] {
  const hours = new Set<number>();
  for (const run of reply.runs) {
    hours.add(run.firstHour); hours.add(run.lastHour + 1);
    for (const step of run.steps) hours.add(step.firstHour);
  }
  for (const span of reply.unobserved) { hours.add(span.firstHour); hours.add(span.lastHour + 1); }
  for (const change of identityChanges) hours.add(change.hour);
  return [...hours].sort((a, b) => a - b);
}
