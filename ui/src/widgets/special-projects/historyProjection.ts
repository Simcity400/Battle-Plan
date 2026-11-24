import { colour } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { urlArtHandle } from '../../families/art/provider.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import type { HourRanges } from '../../families/holders.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { TimelineWidgetDeclaration } from '../../families/timeline/declaration.ts';
import { clipSpansToHour, countryLook, inHourSpans, nextBoundaryAfter, spanStart, timelineModel } from '../../families/timeline/helpers.ts';
import { laneByKey, spanAt, type TimelineLane, type TimelineProjection, type TimelineSpan } from '../../families/timeline/model.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { HourSpan, ProjectRecord, ProjectStatus, SpecialProjectsReply } from '../../generated/dashboard';
import { validateSpecialProjectsReply } from './specialProjectsData.ts';

export const HISTORY_VIEW = 'history';

export interface SpecialProjectsHistoryOptions {
  readonly tags: readonly string[];
  readonly membership: Readonly<Record<string, HourRanges>> | null;
  readonly firstHour: number;
  readonly surface: string;
}

const entryName = (tag: string) => `projects:${tag}`;

const STATE_WORDS: Readonly<Record<ProjectStatus['state'], string>> = {
  running: 'In progress',
  paused: 'Paused',
  completed: 'Completed',
  cancelled: 'Cancelled',
  unknown: 'State not recorded',
};

function gapsOf(reply: SpecialProjectsReply, hour: number): HourSpan[] {
  const gaps: HourSpan[] = [];
  let cursor = Number.NEGATIVE_INFINITY;
  for (const span of reply.available) {
    if (span.firstHour > hour) break;
    if (Number.isFinite(cursor) && span.firstHour > cursor + 1) gaps.push({ firstHour: cursor + 1, lastHour: span.firstHour - 1 });
    cursor = span.lastHour;
  }
  return clipSpansToHour([...gaps, ...reply.partial], hour);
}

function within(ranges: HourRanges | undefined, first: number, last: number): readonly (readonly [number, number])[] {
  if (!ranges) return [[first, last]];
  const out: (readonly [number, number])[] = [];
  for (const [from, to] of ranges) {
    const a = Math.max(first, from);
    const b = Math.min(last, to);
    if (a <= b) out.push([a, b]);
  }
  return out;
}

function statusAt(project: ProjectRecord, hour: number): ProjectStatus | null {
  let held: ProjectStatus | null = null;
  for (const status of project.statuses) {
    if (status.firstHour > hour) break;
    held = status;
  }
  return held;
}

function completedSince(project: ProjectRecord, shown: ProjectStatus): number {
  let since = shown.firstHour;
  for (let i = project.statuses.indexOf(shown) - 1; i >= 0; i--) {
    const before = project.statuses[i]!;
    if (before.state !== 'completed') break;
    since = before.firstHour;
  }
  return since;
}

function projectLane(reply: SpecialProjectsReply, project: ProjectRecord, ordinal: number, hour: number, gaps: readonly HourSpan[], options: SpecialProjectsHistoryOptions): TimelineLane | null {
  const shown = statusAt(project, hour);
  if (!shown) return null;
  const info = reply.presentations[shown.presentation]!;
  const icon = urlArtHandle(info.icon);
  const fill = colour(options.surface, ordinal);
  const ranges = options.membership?.[reply.tag];
  const stretches: [number, number, boolean][] = [];
  for (const status of project.statuses) {
    if (status.firstHour > hour) break;
    if (status.state !== 'running' && status.state !== 'paused') continue;
    const last = Math.min(status.lastHour, hour);
    const held = stretches[stretches.length - 1];
    if (held && held[1] + 1 === status.firstHour) {
      held[1] = last;
      held[2] = status.lastHour >= hour;
    } else {
      stretches.push([status.firstHour, last, status.lastHour >= hour]);
    }
  }
  const spans: TimelineSpan[] = [];
  for (const [from, to, running] of stretches) {
    for (const [first, last] of within(ranges, from, to)) {
      const open = running && last >= hour;
      spans.push({
        id: `${project.id}:${first}`,
        firstHour: first,
        lastHour: last,
        color: fill,
        label: info.name,
        icon,
        iconMissing: !icon,
        ...spanStart(first, options.firstHour, gaps),
        endUnseen: !open && inHourSpans(gaps, last + 1),
        open,
        meta: { tag: reply.tag, project: project.id },
      });
    }
  }
  if (ranges && !spans.length && !within(ranges, project.firstRead, hour).length) return null;
  const words = STATE_WORDS[shown.state];
  const subtitle = shown.state === 'running' || shown.state === 'paused'
    ? `${words} · ${shown.progress}%`
    : shown.state === 'completed' ? `${words} · by ${formatDateOnly(completedSince(project, shown))}` : words;
  return {
    key: `${reply.tag}:${project.id}`,
    kind: 'item',
    label: info.name,
    subtitle,
    icon,
    iconMissing: !icon,
    spans,
    auditValue: `${info.name}|${subtitle}`,
    meta: { tag: reply.tag, project: project.id },
  };
}

function boundaries(replies: readonly SpecialProjectsReply[]): number[] {
  const hours: number[] = [];
  for (const reply of replies) {
    for (const project of reply.projects) for (const status of project.statuses) hours.push(status.firstHour, status.lastHour + 1);
    for (const span of reply.available) hours.push(span.firstHour, span.lastHour + 1);
  }
  return [...new Set(hours)].sort((a, b) => a - b);
}

function repliesOf(readView: BoundedReadView, tags: readonly string[]): (SpecialProjectsReply | undefined)[] {
  return tags.map(tag => {
    const raw = readView.panel<unknown>(entryName(tag));
    return raw === undefined ? undefined : validateSpecialProjectsReply(raw);
  });
}

export const specialProjectsHistoryProjection: TimelineProjection<SpecialProjectsHistoryOptions> = registerFamilyProjection<TimelineProjection<SpecialProjectsHistoryOptions>>({
  family: 'timeline',
  kind: 'special-projects',

  nextChange(readView, options) {
    const replies = repliesOf(readView, options.tags).filter((reply): reply is SpecialProjectsReply => reply !== undefined);
    return nextBoundaryAfter(boundaries(replies), readView.hour);
  },

  project(readView, options) {
    const hour = readView.hour;
    const base = { hour, firstHour: options.firstHour, labelWidth: 240, iconsOnBars: true };
    const replies = repliesOf(readView, options.tags);
    if (!options.tags.length || replies.some(reply => reply === undefined)) return timelineModel(base);
    const shown = replies as SpecialProjectsReply[];
    const nextChangeAt = nextBoundaryAfter(boundaries(shown), hour);
    const warnings = [...new Set(shown.flatMap(reply => reply.warnings))];
    if (shown.every(reply => reply.unavailable)) {
      return timelineModel({ ...base, empty: shown[0]!.unavailable ?? 'Special projects were not recorded.', emptyKind: 'unavailable', warnings, nextChangeAt });
    }
    const lanes: TimelineLane[] = [];
    const gaps: HourSpan[] = [];
    let ordinal = 0;
    for (const reply of shown) {
      const own = gapsOf(reply, hour);
      gaps.push(...own);
      const projectLanes = reply.projects
        .map(project => projectLane(reply, project, ordinal++, hour, own, options))
        .filter((lane): lane is TimelineLane => lane !== null);
      if (shown.length > 1) {
        const look = countryLook(readView, reply.tag, hour);
        lanes.push({
          key: `country:${reply.tag}`,
          kind: 'heading',
          label: look.name,
          ...(look.flag ? { flag: look.flag } : {}),
          flagTag: reply.tag,
          ...(reply.unavailable ? { subtitle: reply.unavailable } : projectLanes.length ? {} : { subtitle: 'No special projects yet' }),
          spans: [],
          auditValue: `${look.name}|${reply.unavailable ?? projectLanes.length}`,
        });
      }
      lanes.push(...projectLanes);
    }
    if (!lanes.some(lane => lane.kind === 'item')) {
      const unrecorded = shown.every(reply => !reply.available.some(span => span.firstHour <= hour));
      return timelineModel(unrecorded
        ? { ...base, empty: 'Special projects were not recorded at this date.', emptyKind: 'unavailable', warnings, nextChangeAt }
        : { ...base, lanes, empty: 'No special projects yet.', warnings, nextChangeAt });
    }
    return timelineModel({ ...base, lanes, unobserved: gaps, warnings, nextChangeAt });
  },

  tooltip(model, hit, readView) {
    const lane = laneByKey(model, hit.laneKey);
    if (!lane || lane.kind !== 'item') return null;
    const span = spanAt(lane, hit);
    const at = hit.hour ?? model.hour;
    const where = span?.meta as { readonly tag: string; readonly project: string } | undefined;
    const raw = where ? readView.panel<unknown>(entryName(where.tag)) : undefined;
    const project = raw === undefined ? undefined : validateSpecialProjectsReply(raw).projects.find(item => item.id === where!.project);
    const status = project ? statusAt(project, at) : null;
    const meta = status ? { state: status.state, progress: status.progress, read: status.readHour } : undefined;
    return {
      title: lane.label,
      hour: at,
      items: span && meta
        ? [
            { label: 'State', value: STATE_WORDS[meta.state] },
            { label: 'Progress', value: `${meta.progress}%` },
            { label: span.heldBefore ? 'Held before' : 'Since', value: `${span.startBy ? 'By ' : ''}${formatDateOnly(span.firstHour)}` },
            { label: 'Read', value: formatDateOnly(meta.read) },
          ]
        : [{ label: 'State', value: inHourSpans(model.unobserved, at) ? 'Not recorded' : lane.subtitle ?? 'Not in progress' }],
    };
  },
}, HISTORY_VIEW);

export const specialProjectsHistoryDeclaration: TimelineWidgetDeclaration<SpecialProjectsHistoryOptions> = {
  family: 'timeline',
  kind: 'special-projects',
  label: 'Special project history',
  subject: 'special projects',
  emptyCampaign: 'Load a campaign and choose a country to see its special projects.',
  icon: 'flask',
  countries: { mode: 'multi', allowSlots: true },
  jumpToDate: true,
  keyByBranch: true,
  sources: ctx => ({
    owner: `special-projects-history:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${ctx.countries.tags.join(',')}`,
    entries: ctx.countries.tags.map(tag => ({
      name: entryName(tag),
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point' as const,
        channel: 'panel' as const,
        coverage: { mode: 'keyed' as const, keys: [tag] },
        product: 'special-projects' as const,
        followQuery: { panel: 'special-projects', branchId: ctx.branch.id, country: countryFromKey(tag) },
        params: { branchId: ctx.branch.id, country: countryFromKey(tag) },
        validate: validateSpecialProjectsReply,
      },
    })),
  }),
  projection: specialProjectsHistoryProjection,
  projectionOptions: (ctx, extra) => ({
    tags: ctx.countries.tags,
    membership: ctx.countries.membership,
    firstHour: extra.extent[0],
    surface: extra.surface,
  }),
};
