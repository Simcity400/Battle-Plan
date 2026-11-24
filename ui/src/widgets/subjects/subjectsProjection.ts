import { CHART_MEASURE_COLOURS, colour, readableColour } from '../../charts/palette.ts';
import { countryIdentityIndex } from '../../dashboard/countryIdentity.ts';
import { Lineage } from '../../dashboard/identity.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { TimelineWidgetDeclaration } from '../../families/timeline/declaration.ts';
import { countryLook, inHourSpans, nextBoundaryAfter, timelineModel, spanStart, clipSpansToHour } from '../../families/timeline/helpers.ts';
import { laneByKey, spanAt, type TimelineLane, type TimelineProjection, type TimelineSpan } from '../../families/timeline/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { groupCountsRead, rankedCounts } from '../../dashboard/groupCounts.ts';
import { groupsAt, historyAtRead, subjectsAt } from '../../dashboard/historyAt.ts';
import type { CountryGroupReply, GroupCountsReply, SubjectsAtReply, SubjectLevel, SubjectReply } from '../../generated/dashboard';
import {
  changeHours,
  laneCountries,
  levelColours,
  overlordViews,
  subjectLanes,
  validateSubjectReply,
  type OverlordOrder,
  type SubjectOrder,
} from './subjectsData.ts';

export interface SubjectsOptions {
  readonly order: OverlordOrder;
  readonly subjects: SubjectOrder;
  readonly showEnded: boolean;
  readonly levelNames: boolean;
  readonly firstHour: number;
  readonly surface: string;
  readonly selected: readonly string[] | null;
}

const views = new WeakMap<SubjectReply, WeakMap<CountryGroupReply, ReturnType<typeof overlordViews>>>();

function viewsOf(reply: SubjectReply, groups: CountryGroupReply) {
  let byGroups = views.get(reply);
  if (!byGroups) views.set(reply, byGroups = new WeakMap());
  let held = byGroups.get(groups);
  if (!held) byGroups.set(groups, held = overlordViews(reply, new Lineage(groups.identityChanges), countryIdentityIndex(groups.countryIdentities)));
  return held;
}

function boundaries(reply: SubjectReply, groups: CountryGroupReply | undefined): number[] {
  return changeHours(reply, groups?.identityChanges);
}

export const subjectsProjection: TimelineProjection<SubjectsOptions> = registerFamilyProjection<TimelineProjection<SubjectsOptions>>({
  family: 'timeline',
  kind: 'subjects',

  nextChange(readView) {
    const main = subjectsAt(readView, 'main');
    const groups = groupsAt(readView, 'groups');
    const counts = readView.panel<GroupCountsReply>('counts')?.nextChange ?? Infinity;
    return main ? Math.min(counts, main.nextChange, groups?.nextChange ?? Infinity, nextBoundaryAfter(boundaries(main.subjects, groups?.groups), readView.hour)) : Infinity;
  },

  project(readView, opt) {
    const hour = readView.hour;
    const main = subjectsAt(readView, 'main');
    const groupsRead = groupsAt(readView, 'groups');
    const reply = main?.subjects;
    const groups = groupsRead?.groups;
    const base = { hour, firstHour: opt.firstHour, labelWidth: 230 };
    if (!reply || !groups) return timelineModel(base);
    const counts = rankedCounts(readView.panel<GroupCountsReply>('counts'));
    const nextChangeAt = Math.min(counts?.nextChange ?? Infinity, main!.nextChange, groupsRead!.nextChange, nextBoundaryAfter(boundaries(reply, groups), hour));
    const warnings = [...reply.warnings, ...groups.warnings];
    if (!reply.runs.length) return timelineModel({ ...base, empty: 'No subjects recorded', warnings, nextChangeAt });
    const overlords = viewsOf(reply, groups);
    const levels = new Map<string, SubjectLevel>(reply.levels.map(level => [level.token, level]));
    const other = readableColour(CHART_MEASURE_COLOURS.other, opt.surface);
    const fills = levelColours(reply.levels, reply.levels.map((_, index) => colour(opt.surface, index)), other);
    const unobserved = clipSpansToHour(overlords[0]?.unknown ?? reply.unobserved, hour);
    const selected = opt.selected ? new Set(opt.selected) : null;
    const built = subjectLanes(overlords, levels, fills, hour, {
      order: opt.order,
      subjects: opt.subjects,
      ended: opt.showEnded,
      countries: selected,
      countryIds: selected ? overlords[0]?.identities?.selected([...selected]) ?? null : null,
      counts,
    }, { country: tag => ({ name: countryLook(readView, tag, hour).name }), day: formatDateOnly }, unobserved);
    if (!built.lanes.length) {
      return timelineModel({ ...base, empty: selected ? 'None of the chosen countries has been an overlord or a subject yet' : 'No subjects yet', warnings, nextChangeAt });
    }
    const tagOf = new Map<string, string>();
    for (const lane of built.lanes) {
      const [tag] = laneCountries([lane], built.info, hour);
      if (tag) tagOf.set(lane.id, tag);
    }
    let headerKey = '';
    const lanes: TimelineLane[] = built.lanes.map(source => {
      const tag = tagOf.get(source.id);
      const look = tag ? countryLook(readView, tag, hour) : null;
      const heading = source.heading === true;
      if (heading) headerKey = source.id;
      const spans: TimelineSpan[] = [];
      for (const span of source.spans) {
        if (span.firstHour > hour) continue;
        const open = span.lastHour >= hour;
        spans.push({
          id: `${source.id}@${span.id}`,
          firstHour: span.firstHour,
          lastHour: Math.min(span.lastHour, hour),
          color: span.color ?? other,
          ...(!heading && opt.levelNames && span.label ? { label: span.label } : {}),
          ...spanStart(span.firstHour, opt.firstHour, unobserved),
          endUnseen: !open && inHourSpans(unobserved, span.lastHour + 1),
          open,
        });
      }
      const name = source.identity?.label ?? 'Unknown country';
      return {
        key: source.id,
        kind: heading ? 'group' as const : 'item' as const,
        ...(heading ? {} : { depth: 1, parentKey: headerKey }),
        label: name,
        subtitle: source.label,
        ...(look ? { flag: look.flag, flagTag: look.tag } : { flagTag: '?' }),
        ...(heading && look?.colour ? { ruleColor: look.colour } : {}),
        spans,
        auditValue: `${name}|${source.label}`,
      };
    });
    return timelineModel({ ...base, lanes, unobserved, warnings, nextChangeAt });
  },

  tooltip(model, hit) {
    const lane = laneByKey(model, hit.laneKey);
    if (!lane) return null;
    const at = hit.hour ?? model.hour;
    if (lane.kind === 'group') return { title: lane.label, subtitle: 'Overlord', hour: at, items: [{ label: 'Subjects', value: lane.subtitle ?? '' }] };
    const span = spanAt(lane, hit);
    const overlord = model.lanes.find(item => item.key === lane.parentKey);
    const level = span?.label;
    return {
      title: lane.label,
      subtitle: overlord ? `Subject of ${overlord.label}` : 'Subject',
      hour: at,
      items: [
        ...(level && span ? [{ label: 'Level', value: level, swatch: span.color }] : []),
        { label: 'Status', value: lane.subtitle ?? '' },
      ],
    };
  },
});

function subjectsRead(branchId: string) {
  const read = historyAtRead('subjects-at', branchId);
  return { ...read, validate: (data: unknown) => {
    const reply = read.validate!(data) as SubjectsAtReply;
    validateSubjectReply(reply.subjects);
    return reply;
  } };
}

export const subjectsDeclaration: TimelineWidgetDeclaration<SubjectsOptions> = {
  family: 'timeline',
  kind: 'subjects',
  label: 'Overlords and subjects',
  subject: 'subjects',
  emptyCampaign: 'Load a campaign to see its overlords and subjects.',
  icon: 'hierarchy',
  countries: { mode: 'multi', labels: true, allTagsForLabels: true },
  jumpToDate: true,
  keyByBranch: true,
  sources: ctx => {
    return {
      owner: `subjects:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`,
      entries: [
        { name: 'main', source: ctx.track.source, branch: ctx.branch.id, offset: ctx.track.axisOffset, read: subjectsRead(ctx.branch.id) },
        { name: 'groups', source: ctx.track.source, branch: ctx.branch.id, offset: ctx.track.axisOffset, read: historyAtRead('country-groups-at', ctx.branch.id) },
        { name: 'counts', source: ctx.track.source, branch: ctx.branch.id, offset: ctx.track.axisOffset, read: groupCountsRead(ctx.branch.id, 'overlords') },
      ],
    };
  },
  projection: subjectsProjection,
  projectionOptions: (ctx, extra) => ({
    order: (ctx.options.order as OverlordOrder) ?? 'first',
    subjects: (ctx.options.subjects as SubjectOrder) ?? 'since',
    showEnded: ctx.options.ended !== 'hidden',
    levelNames: ctx.options.names !== false,
    firstHour: extra.extent[0],
    surface: extra.surface,
    selected: ctx.countries.all ? null : ctx.countries.tags,
  }),
};
