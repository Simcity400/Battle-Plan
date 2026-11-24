import { formatDay } from '../../dashboard/calendar.ts';
import { runState, startUnseen } from '../../charts/timeline/runState.ts';
import { Lineage } from '../../dashboard/identity.ts';
import type { TimelineWidgetDeclaration } from '../../families/timeline/declaration.ts';
import { clipSpansToHour, inHourSpans, nextBoundaryAfter, paletteAt, resolveCountryChip } from '../../families/timeline/helpers.ts';
import type { TimelineLaneModel, TimelineModel, TimelineProjection, TimelineSpanModel } from '../../families/timeline/model.ts';
import { registerTimelineProjection } from '../../families/timeline/registry.ts';
import type { SubjectLevel, SubjectReply } from '../../generated/dashboard';
import { OPTIONS } from './definition.ts';
import {
  changeHours,
  levelAt,
  levelName,
  overlordViews,
  runNear,
  validateSubjectReply,
  type OverlordOrder,
  type SubjectOrder,
} from './subjectsData.ts';

export interface SubjectsProjectionOptions {
  readonly order: OverlordOrder;
  readonly subjects: SubjectOrder;
  readonly showEnded: boolean;
  readonly showLevelNames: boolean;
  readonly branchFirst: number;
  readonly surface: string;
  readonly selectedTags: readonly string[] | null;
}

export const subjectsProjection: TimelineProjection<SubjectsProjectionOptions> = registerTimelineProjection({
  kind: 'subjects',

  nextChange(readView) {
    const reply = readView.panel<SubjectReply>('main');
    if (!reply) return Infinity;
    return nextBoundaryAfter(changeHours(reply), readView.hour);
  },

  project(readView, opt): TimelineModel {
    const hour = readView.hour;
    const reply = readView.panel<SubjectReply>('main');
    const extent: [number, number] = [opt.branchFirst, Math.max(opt.branchFirst + 1, hour)];
    if (!reply) {
      return { mode: 'lanes', hour, extent, playhead: hour, lanes: [], empty: 'Loading subjects\u2026' };
    }
    if (!reply.runs.length) {
      return {
        mode: 'lanes',
        hour,
        extent,
        playhead: hour,
        lanes: [],
        empty: reply.warnings.length ? 'Subject relations are unavailable.' : 'No subjects recorded.',
        warnings: reply.warnings,
      };
    }

    const levels = new Map<string, SubjectLevel>(reply.levels.map(l => [l.token, l]));
    const levelColor = new Map<string, string>(reply.levels.map((l, idx) => [l.token, paletteAt(opt.surface, idx)]));
    const fallbackColor = paletteAt(opt.surface, reply.levels.length);
    const unobserved = clipSpansToHour(reply.unobserved, hour);
    const views = overlordViews(reply, new Lineage([]));
    const tagSet = opt.selectedTags ? new Set(opt.selectedTags) : null;

    const overlords = views
      .map(ov => {
        const shown = ov.subjects.filter(sv => {
          const first = sv.runs[0]?.firstHour ?? Infinity;
          if (first > hour) return false;
          if (!opt.showEnded && runState(sv.stretches, hour, reply.unobserved).kind === 'ended') return false;
          return true;
        });
        if (!shown.length) return null;
        if (tagSet) {
          const matched = shown.some(sv => sv.runs.some(r => r.firstHour <= hour && (tagSet.has(r.overlord) || tagSet.has(r.subject))));
          if (!matched) return null;
        }
        const latestRun = shown[0]?.runs.find(r => r.firstHour <= hour) ?? shown[0]?.runs[0];
        const overlordTag = latestRun?.overlord ?? ov.key;
        const overlordChip = resolveCountryChip(readView, overlordTag, hour);
        const activeCount = shown.filter(sv => sv.runs.some(r => r.firstHour <= hour && hour <= r.lastHour)).length;
        return { ov, shown, overlordTag, overlordChip, activeCount };
      })
      .filter((x): x is NonNullable<typeof x> => x !== null);

    if (!overlords.length) {
      return {
        mode: 'lanes',
        hour,
        extent,
        playhead: hour,
        lanes: [],
        empty: tagSet ? 'None of the chosen countries has been an overlord or a subject yet.' : 'No subjects yet.',
        warnings: reply.warnings,
      };
    }

    overlords.sort(
      opt.order === 'name'
        ? (a, b) => a.overlordChip.name.localeCompare(b.overlordChip.name)
        : opt.order === 'subjects'
          ? (a, b) => b.activeCount - a.activeCount || a.ov.firstSince - b.ov.firstSince
          : (a, b) => a.ov.firstSince - b.ov.firstSince,
    );

    const lanes: TimelineLaneModel[] = [];
    for (const { ov, shown, overlordTag, overlordChip, activeCount } of overlords) {
      const ruleStart = Math.min(...shown.map(s => s.runs[0]?.firstHour ?? hour));
      const ruleEnd = Math.min(hour, Math.max(...shown.map(s => s.runs.at(-1)?.lastHour ?? ruleStart)));
      const subText = `${activeCount} ${activeCount === 1 ? 'subject' : 'subjects'}`;
      lanes.push({
        key: `overlord:${ov.key}`,
        kind: 'group-header',
        label: overlordChip.name,
        subtitle: subText,
        flag: overlordChip.flag,
        flagTag: overlordTag,
        ruleColor: overlordChip.color ?? '#8c95a6',
        ruleSpan: [ruleStart, Math.max(ruleStart, ruleEnd)],
        auditValue: `${overlordChip.name}|${subText}`,
        meta: { kind: 'overlord', overlordTag },
      });

      const subItems = shown.map(sv => {
        const run = runNear(sv, hour);
        const subjectTag = run.subject;
        const chip = resolveCountryChip(readView, subjectTag, Math.min(hour, run.lastHour));
        return { sv, run, subjectTag, chip };
      });
      subItems.sort(
        opt.subjects === 'name'
          ? (a, b) => a.chip.name.localeCompare(b.chip.name)
          : (a, b) => a.run.startHour - b.run.startHour || a.chip.name.localeCompare(b.chip.name),
      );

      for (const { sv, run, subjectTag, chip } of subItems) {
        const spans: TimelineSpanModel[] = [];
        for (const r of sv.runs) {
          if (r.firstHour > hour) continue;
          for (const step of r.steps) {
            if (step.firstHour > hour) continue;
            const openAtPlayhead = step.lastHour >= hour;
            const ended = !openAtPlayhead;
            const color = step.level ? (levelColor.get(step.level) ?? fallbackColor) : fallbackColor;
            spans.push({
              id: `${sv.key}:${step.firstHour}`,
              firstHour: step.firstHour,
              lastHour: Math.min(step.lastHour, hour),
              color,
              label: opt.showLevelNames ? levelName(levels, step.level) : undefined,
              heldBefore: startUnseen(step.firstHour, opt.branchFirst, reply.unobserved),
              fadedEnd: ended && inHourSpans(reply.unobserved, step.lastHour + 1),
              ended,
              openAtPlayhead,
              meta: { level: step.level, startHour: r.startHour },
            });
          }
        }
        const active = run.firstHour <= hour && hour <= run.lastHour;
        const currentLvl = active ? levelAt(run, hour) ?? null : null;
        const subtitle = active
          ? `${levelName(levels, currentLvl)} \u00b7 since ${formatDay(run.startHour)}`
          : `Ended ${formatDay(Math.min(hour, run.lastHour))}`;
        lanes.push({
          key: `subject:${sv.key}`,
          kind: 'standard',
          depth: 1,
          label: chip.name,
          subtitle,
          flag: chip.flag,
          flagTag: subjectTag,
          dimmed: !active,
          spans,
          auditValue: `${chip.name}|${subtitle}`,
          meta: { kind: 'subject', overlordName: overlordChip.name, subjectTag, startHour: run.startHour },
        });
      }
    }

    return {
      mode: 'lanes',
      hour,
      extent,
      playhead: hour,
      lanes,
      unobserved,
      iconsOnBars: false,
      labelWidth: 224,
      warnings: reply.warnings,
      nextChangeAt: nextBoundaryAfter(changeHours(reply), hour),
    };
  },

  tooltip(model, target, hour) {
    const lane = model.lanes?.find(l => l.key === (target.laneKey ?? target.key));
    if (!lane) return null;
    const meta = lane.meta as { kind?: string; overlordName?: string; startHour?: number } | undefined;
    if (meta?.kind === 'overlord') {
      return {
        title: lane.label,
        subtitle: 'Overlord',
        hour,
        items: [{ label: 'Subjects', value: lane.subtitle ?? '0' }],
      };
    }
    const span = target.spanId
      ? lane.spans?.find(s => s.id === target.spanId)
      : lane.spans?.find(s => s.firstHour <= hour && hour <= s.lastHour) ?? lane.spans?.at(-1);
    return {
      title: lane.label,
      subtitle: meta?.overlordName ? `Subject of ${meta.overlordName}` : 'Subject',
      hour,
      items: [
        ...(span?.label ? [{ label: 'Level', value: span.label, swatch: span.color }] : []),
        ...(meta?.startHour !== undefined ? [{ label: 'Subject since', value: formatDay(meta.startHour) }] : []),
        ...(span ? [{ label: 'Status', value: span.openAtPlayhead ? 'Still a subject' : `Ended ${formatDay(span.lastHour)}` }] : []),
      ],
    };
  },
});

export const subjectsDeclaration: TimelineWidgetDeclaration<SubjectsProjectionOptions> = {
  family: 'timeline',
  kind: 'subjects',
  label: 'Overlords and subjects',
  subject: 'subjects',
  emptyCampaign: 'Load a campaign to see its overlords and subjects.',
  icon: 'hierarchy',
  optionFields: OPTIONS,
  countries: { mode: 'multi', labels: true, allTagsForLabels: true },
  jumpToDate: true,
  keyByBranch: true,
  sources: ctx => ({
    owner: `subjects:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`,
    entries: [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'branch-to-hour',
        coverage: { mode: 'population', form: 'all' },
        product: 'subjects',
        followQuery: null,
        params: { branchId: ctx.branch.id },
        buildSpanParams: base => base,
        validate: validateSubjectReply,
      },
    }],
    allowEmptyFollow: true,
  }),
  projection: subjectsProjection,
  projectionOptions: ctx => ({
    order: (ctx.options.order as OverlordOrder) ?? 'first',
    subjects: (ctx.options.subjects as SubjectOrder) ?? 'since',
    showEnded: ctx.options.ended !== 'hidden',
    showLevelNames: ctx.options.names !== false,
    branchFirst: ctx.branch.firstHour,
    surface: '#16181d',
    selectedTags: ctx.countries.all ? null : ctx.countries.tags,
  }),
};
