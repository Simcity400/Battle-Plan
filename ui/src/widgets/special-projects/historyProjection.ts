import { colour } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { productIconArt } from '../../families/art/provider.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import type { HourRanges } from '../../families/holders.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { TimelineWidgetDeclaration } from '../../families/timeline/declaration.ts';
import { clipSpansToHour, countryLook, inHourSpans, nextBoundaryAfter, spanStart, timelineModel } from '../../families/timeline/helpers.ts';
import { laneByKey, spanAt, type TimelineLane, type TimelineProjection, type TimelineSpan } from '../../families/timeline/model.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { HourSpan, ProjectPresentation, ProjectRecord, ProjectStatus, SpecialProjectsReply } from '../../generated/dashboard';
import { validateSpecialProjectsReply } from './specialProjectsData.ts';

export const HISTORY_VIEW = 'history';

export interface SpecialProjectsHistoryOptions {
  readonly tags: readonly string[];
  readonly membership: Readonly<Record<string, HourRanges>> | null;
  readonly firstHour: number;
  readonly branchFirstHour: number;
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

function mergeSpans(spans: readonly HourSpan[]): HourSpan[] {
  if (spans.length <= 1) return spans.map(span => ({ ...span }));
  const sorted = [...spans].sort((a, b) => a.firstHour - b.firstHour || a.lastHour - b.lastHour);
  const out: HourSpan[] = [{ ...sorted[0]! }];
  for (let i = 1; i < sorted.length; i++) {
    const next = sorted[i]!;
    const prev = out[out.length - 1]!;
    if (next.firstHour <= prev.lastHour + 1) prev.lastHour = Math.max(prev.lastHour, next.lastHour);
    else out.push({ ...next });
  }
  return out;
}

function isRecordedAt(reply: SpecialProjectsReply, hour: number): boolean {
  return inHourSpans(reply.available, hour) && !inHourSpans(reply.partial, hour) && !inHourSpans(reply.unresolved, hour);
}

function gapsOf(reply: SpecialProjectsReply, hour: number, branchFirstHour: number): HourSpan[] {
  const gaps: HourSpan[] = [];
  let cursor = branchFirstHour - 1;
  for (const span of reply.available) {
    if (span.firstHour > hour) break;
    if (span.firstHour > cursor + 1) gaps.push({ firstHour: cursor + 1, lastHour: span.firstHour - 1 });
    cursor = Math.max(cursor, span.lastHour);
  }
  if (cursor < hour) gaps.push({ firstHour: cursor + 1, lastHour: hour });
  return mergeSpans(clipSpansToHour([...gaps, ...reply.partial, ...reply.unresolved], hour));
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
  for (const status of project.statuses) {
    if (status.firstHour > hour) break;
    if (hour <= status.lastHour) return status;
  }
  return null;
}

function latestStatusThrough(project: ProjectRecord, hour: number): ProjectStatus | null {
  let held: ProjectStatus | null = null;
  for (const status of project.statuses) {
    if (status.firstHour > hour) break;
    held = status;
  }
  return held;
}

function sameVisual(a: ProjectPresentation, b: ProjectPresentation): boolean {
  return a.name === b.name && a.icon === b.icon && a.iconMissing === b.iconMissing;
}

function completedSince(project: ProjectRecord, shown: ProjectStatus): number {
  let since = shown.firstHour;
  for (let i = project.statuses.indexOf(shown) - 1; i >= 0; i--) {
    const before = project.statuses[i]!;
    if (before.state !== 'completed' || before.lastHour + 1 !== since) break;
    since = before.firstHour;
  }
  return since;
}

function statusSummary(project: ProjectRecord, status: ProjectStatus): string {
  const words = STATE_WORDS[status.state];
  if (status.state === 'running' || status.state === 'paused') return `${words} · ${status.progress}%`;
  if (status.state === 'completed') return `${words} · ${formatDateOnly(completedSince(project, status))}`;
  return words;
}

interface LaneMeta {
  readonly sourceKey: string;
  readonly project: string;
  readonly since?: number;
  readonly heldBefore?: boolean;
}

function projectLane(readView: BoundedReadView, sourceKey: string, reply: SpecialProjectsReply, project: ProjectRecord, ordinal: number, hour: number, gaps: readonly HourSpan[], options: SpecialProjectsHistoryOptions): TimelineLane | null {
  const current = statusAt(project, hour);
  const latest = current ?? latestStatusThrough(project, hour);
  if (!latest) return null;
  const info = reply.presentations[latest.presentation]!;
  const icon = productIconArt(readView, info.icon, info.iconMissing);
  const fill = colour(options.surface, ordinal);
  const ranges = options.membership?.[sourceKey] ?? options.membership?.[reply.tag];
  const stretches: [number, number, boolean, number, number][] = [];
  for (const status of project.statuses) {
    if (status.firstHour > hour) break;
    if (status.state !== 'running' && status.state !== 'paused') continue;
    const last = Math.min(status.lastHour, hour);
    const held = stretches[stretches.length - 1];
    const contiguous = Boolean(held && held[1] + 1 === status.firstHour);
    if (contiguous && sameVisual(reply.presentations[held![3]]!, reply.presentations[status.presentation]!)) {
      held![1] = last;
      held![2] = status.lastHour >= hour;
    } else {
      stretches.push([status.firstHour, last, status.lastHour >= hour, status.presentation, contiguous ? held![4] : status.firstHour]);
    }
  }
  const spans: TimelineSpan[] = [];
  for (const [from, to, running, presentation, stretchStart] of stretches) {
    const spanInfo = reply.presentations[presentation]!;
    const spanIcon = productIconArt(readView, spanInfo.icon, spanInfo.iconMissing);
    for (const [first, last] of within(ranges, from, to)) {
      const open = running && last >= hour;
      const since = ranges ? Math.max(stretchStart, ranges.find(([a, b]) => a <= first && first <= b)?.[0] ?? first) : stretchStart;
      spans.push({
        id: `${project.id}:${first}`,
        firstHour: first,
        lastHour: last,
        color: fill,
        label: spanInfo.name,
        icon: spanIcon,
        ...spanStart(first, options.branchFirstHour, gaps),
        endUnseen: !open && inHourSpans(gaps, last + 1),
        open,
        meta: { sourceKey, project: project.id, since, heldBefore: since <= options.branchFirstHour } satisfies LaneMeta,
      });
    }
  }
  if (ranges && !spans.length && !within(ranges, project.firstRead, hour).length) return null;
  const subtitle = current ? statusSummary(project, current) : isRecordedAt(reply, hour) ? 'Not in progress' : 'Not recorded';
  return {
    key: `${sourceKey}:${project.id}`,
    kind: 'item',
    label: info.name,
    subtitle,
    icon,
    spans,
    auditValue: `${info.name}|${subtitle}`,
    meta: { sourceKey, project: project.id } satisfies LaneMeta,
  };
}

function boundaries(replies: readonly SpecialProjectsReply[]): number[] {
  const hours: number[] = [];
  for (const reply of replies) {
    for (const project of reply.projects) for (const status of project.statuses) hours.push(status.firstHour, status.lastHour + 1);
    for (const span of reply.available) hours.push(span.firstHour, span.lastHour + 1);
    for (const span of reply.partial) hours.push(span.firstHour, span.lastHour + 1);
    for (const span of reply.unresolved) hours.push(span.firstHour, span.lastHour + 1);
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
    for (let i = 0; i < shown.length; i++) {
      const sourceKey = options.tags[i]!;
      const reply = shown[i]!;
      const own = gapsOf(reply, hour, options.branchFirstHour);
      gaps.push(...own);
      const projectLanes = reply.projects
        .map(project => projectLane(readView, sourceKey, reply, project, ordinal++, hour, own, options))
        .filter((lane): lane is TimelineLane => lane !== null);
      if (shown.length > 1) {
        const look = countryLook(readView, sourceKey, hour);
        const headingStatus = reply.unavailable
          ? reply.unavailable
          : projectLanes.length
            ? undefined
            : isRecordedAt(reply, hour) ? 'No special projects yet' : 'Not recorded at this date';
        lanes.push({
          key: `country:${sourceKey}`,
          kind: 'heading',
          label: headingStatus ? `${look.name} · ${headingStatus}` : look.name,
          flag: look.flag,
          flagTag: reply.tag,
          spans: [],
          auditValue: `${look.name}|${headingStatus ?? projectLanes.length}`,
        });
      }
      lanes.push(...projectLanes);
    }
    if (!lanes.some(lane => lane.kind === 'item')) {
      if (shown.every(reply => !isRecordedAt(reply, hour))) {
        return timelineModel({ ...base, empty: 'Special projects were not recorded at this date.', emptyKind: 'unavailable', warnings, nextChangeAt });
      }
      if (shown.every(reply => !reply.unavailable && isRecordedAt(reply, hour))) {
        return timelineModel({ ...base, lanes, empty: 'No special projects yet.', warnings, nextChangeAt });
      }
      return timelineModel({ ...base, lanes, unobserved: gaps, warnings, nextChangeAt });
    }
    return timelineModel({ ...base, lanes, unobserved: gaps, warnings, nextChangeAt });
  },

  tooltip(model, hit, readView) {
    const lane = laneByKey(model, hit.laneKey);
    if (!lane || lane.kind !== 'item') return null;
    const span = spanAt(lane, hit);
    const at = hit.hour ?? model.hour;
    const where = (span?.meta as LaneMeta | undefined) ?? (lane.meta as LaneMeta | undefined);
    const raw = where ? readView.panel<unknown>(entryName(where.sourceKey)) : undefined;
    const reply = raw === undefined ? undefined : validateSpecialProjectsReply(raw);
    const project = reply?.projects.find(item => item.id === where!.project);
    const status = project ? statusAt(project, at) : null;
    const titleStatus = status ?? (project ? latestStatusThrough(project, at) : null);
    const title = project
      ? (titleStatus ? reply!.presentations[titleStatus.presentation]!.name : 'Special project')
      : span?.label ?? lane.label;
    if (span && status) {
      const since = where?.since ?? span.firstHour;
      const heldBefore = where?.heldBefore ?? span.heldBefore;
      return {
        title,
        hour: at,
        items: [
          { label: 'State', value: STATE_WORDS[status.state] },
          { label: 'Progress', value: `${status.progress}%` },
          { label: heldBefore ? 'Held before' : 'Since', value: formatDateOnly(since) },
          { label: 'Read', value: formatDateOnly(status.readHour) },
        ],
      };
    }
    const recorded = reply ? isRecordedAt(reply, at) : !inHourSpans(model.unobserved, at);
    const stateText = status && project
      ? statusSummary(project, status)
      : recorded ? 'Not in progress' : 'Not recorded';
    return {
      title,
      hour: at,
      items: [{ label: 'State', value: stateText }],
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
    branchFirstHour: ctx.branch.firstHour,
    surface: extra.surface,
  }),
};
