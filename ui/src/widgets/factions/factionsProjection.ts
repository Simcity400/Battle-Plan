import { formatDay } from '../../dashboard/calendar.ts';
import { describeRun, runState, startUnseen } from '../../charts/timeline/runState.ts';
import type { TimelineWidgetDeclaration } from '../../families/timeline/declaration.ts';
import { clipSpansToHour, inHourSpans, nextBoundaryAfter, paletteAt, resolveCountryChip } from '../../families/timeline/helpers.ts';
import type { TimelineLaneModel, TimelineModel, TimelineProjection, TimelineSpanModel } from '../../families/timeline/model.ts';
import { registerTimelineProjection } from '../../families/timeline/registry.ts';
import type { CountryGroupReply } from '../../generated/dashboard';
import { OPTIONS } from './definition.ts';
import {
  factionChangeHours,
  factionGaps,
  factionViews,
  leaderAt,
  membersAt,
  tagAt,
  type FactionOrder,
  type MemberOrder,
} from './factionsData.ts';

export interface FactionsProjectionOptions {
  readonly order: FactionOrder;
  readonly members: MemberOrder;
  readonly showEnded: boolean;
  readonly branchFirst: number;
  readonly surface: string;
  readonly selectedTags: readonly string[] | null;
  readonly customColors?: Readonly<Record<string, string>>;
}

function factionNameFromReply(reply: CountryGroupReply, factionId: string, hour: number): { name: string | null; reason: string | null } {
  const f = reply.factions.find(item => item.id === factionId);
  if (!f) return { name: null, reason: null };
  let chosen = f.names[0];
  for (const run of f.names) {
    if (run.firstHour <= hour) chosen = run;
  }
  if (chosen) return { name: chosen.name, reason: chosen.reason };
  return { name: f.name, reason: f.nameReason };
}

export const factionsProjection: TimelineProjection<FactionsProjectionOptions> = registerTimelineProjection({
  kind: 'factions',

  nextChange(readView, opt) {
    const reply = readView.panel<CountryGroupReply>('main');
    if (!reply) return Infinity;
    const views = factionViews(
      reply,
      (f, h) => factionNameFromReply(reply, f.id, h ?? readView.hour),
      id => opt.customColors?.[id] ?? paletteAt(opt.surface, Number(id) || 0),
    );
    return nextBoundaryAfter(factionChangeHours(reply, views), readView.hour);
  },

  project(readView, opt): TimelineModel {
    const hour = readView.hour;
    const reply = readView.panel<CountryGroupReply>('main');
    const extent: [number, number] = [opt.branchFirst, Math.max(opt.branchFirst + 1, hour)];
    if (!reply) {
      return { mode: 'lanes', hour, extent, playhead: hour, lanes: [], empty: 'Loading factions\u2026' };
    }
    if (!reply.factions.length) {
      return {
        mode: 'lanes',
        hour,
        extent,
        playhead: hour,
        lanes: [],
        empty: reply.warnings.length ? 'Faction data unavailable.' : 'No factions recorded.',
        warnings: reply.warnings,
      };
    }

    const gaps = factionGaps(reply);
    const unobserved = clipSpansToHour(gaps, hour);
    const factionIndex = new Map<string, number>();
    reply.factions.forEach((f, idx) => factionIndex.set(f.id, idx));

    const views = factionViews(
      reply,
      (f, h) => factionNameFromReply(reply, f.id, h ?? hour),
      id => opt.customColors?.[id] ?? paletteAt(opt.surface, factionIndex.get(id) ?? 0),
    );

    const tagSet = opt.selectedTags ? new Set(opt.selectedTags) : null;
    const shownViews = views.filter(v => {
      const first = v.lifetime[0]?.firstHour ?? Infinity;
      if (first > hour) return false;
      if (!opt.showEnded && runState(v.lifetime, hour, gaps).kind === 'ended') return false;
      if (tagSet) {
        const hasMatch = v.members.some(m => m.runs.some(r => r.firstHour <= hour && tagSet.has(r.tag)));
        if (!hasMatch) return false;
      }
      return true;
    });

    if (!shownViews.length) {
      return {
        mode: 'lanes',
        hour,
        extent,
        playhead: hour,
        lanes: [],
        empty: tagSet ? 'None of the chosen countries has been in a faction yet.' : 'No factions yet.',
        warnings: reply.warnings,
      };
    }

    const factionTitle = (v: typeof shownViews[number]): string => {
      const who = readView.who('faction', v.id, hour);
      if (who.name.status === 'ready') return who.name.value;
      const fromReply = factionNameFromReply(reply, v.id, hour).name;
      return fromReply ?? 'Unnamed faction';
    };

    shownViews.sort(
      opt.order === 'name'
        ? (a, b) => factionTitle(a).localeCompare(factionTitle(b))
        : opt.order === 'members'
          ? (a, b) => (membersAt(b, hour) ?? 0) - (membersAt(a, hour) ?? 0) || (a.lifetime[0]?.firstHour ?? 0) - (b.lifetime[0]?.firstHour ?? 0)
          : (a, b) => (a.lifetime[0]?.firstHour ?? 0) - (b.lifetime[0]?.firstHour ?? 0),
    );

    const memberWords = {
      since: (h: number) => `Joined ${formatDay(h)}`,
      before: (h: number) => `Member before ${formatDay(h)}`,
      ended: (h: number) => `Left ${formatDay(h)}`,
      endedBy: (h: number) => `Left by ${formatDay(h)}`,
      lastSeen: (h: number) => `Last recorded ${formatDay(h)}`,
    };

    const lanes: TimelineLaneModel[] = [];
    for (const v of shownViews) {
      const title = factionTitle(v);
      const count = membersAt(v, hour);
      const lifeStart = v.lifetime[0]?.firstHour ?? opt.branchFirst;
      const lifeEnd = Math.min(hour, v.lifetime.at(-1)?.lastHour ?? hour);
      const countSub = count === null ? 'Members not recorded' : `${count} ${count === 1 ? 'member' : 'members'}`;
      lanes.push({
        key: `faction:${v.id}`,
        kind: 'group-header',
        label: title,
        subtitle: countSub,
        colorSwatch: v.color,
        ruleColor: v.color,
        ruleSpan: [lifeStart, Math.max(lifeStart, lifeEnd)],
        auditValue: `${title}|${countSub}`,
        meta: { kind: 'faction', factionId: v.id, title },
      });

      const currentLeader = leaderAt(v.leaders, hour);
      const visibleMembers = v.members.filter(m => {
        const first = m.runs[0]?.firstHour ?? Infinity;
        if (first > hour) return false;
        if (!opt.showEnded && runState(m.stretches, hour, v.unknown).kind === 'ended') return false;
        return true;
      });

      const memberInfo = visibleMembers.map(m => {
        const latestRun = m.runs.filter(r => r.firstHour <= hour).at(-1);
        const tag = tagAt(v, m, Math.min(hour, latestRun?.lastHour ?? hour)) ?? latestRun?.tag ?? null;
        const chip = resolveCountryChip(readView, tag, Math.min(hour, latestRun?.lastHour ?? hour));
        const isLeader = tag !== null && tag === currentLeader;
        return { m, tag, chip, isLeader, first: m.runs[0]?.firstHour ?? Infinity };
      });

      memberInfo.sort(
        opt.members === 'name'
          ? (a, b) => Number(b.isLeader) - Number(a.isLeader) || a.chip.name.localeCompare(b.chip.name)
          : (a, b) => Number(b.isLeader) - Number(a.isLeader) || a.first - b.first || a.chip.name.localeCompare(b.chip.name),
      );

      for (const { m, tag, chip, isLeader } of memberInfo) {
        const spans: TimelineSpanModel[] = [];
        for (const run of m.runs) {
          if (run.firstHour > hour) continue;
          const openAtPlayhead = run.lastHour >= hour;
          const ended = !openAtPlayhead;
          spans.push({
            id: `${v.id}:${m.key}:${run.firstHour}`,
            firstHour: run.firstHour,
            lastHour: Math.min(run.lastHour, hour),
            color: v.color,
            heldBefore: startUnseen(run.firstHour, opt.branchFirst, v.unknown),
            fadedEnd: ended && inHourSpans(v.unknown, run.lastHour + 1),
            ended,
            openAtPlayhead,
          });
        }
        const desc = describeRun(m.stretches, hour, memberWords, v.unknown, opt.branchFirst) ?? '';
        const subtitle = isLeader ? (desc ? `Leader \u00b7 ${desc}` : 'Leader') : desc;
        const stillIn = spans.some(s => s.openAtPlayhead);
        lanes.push({
          key: `member:${v.id}:${m.key}`,
          kind: 'standard',
          depth: 1,
          label: chip.name,
          subtitle,
          flag: chip.flag,
          flagTag: tag ?? undefined,
          dimmed: !stillIn,
          spans,
          auditValue: `${chip.name}|${subtitle}`,
          meta: { kind: 'member', factionId: v.id, factionTitle: title, tag, isLeader },
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
      nextChangeAt: nextBoundaryAfter(factionChangeHours(reply, views), hour),
    };
  },

  tooltip(model, target, hour) {
    const lane = model.lanes?.find(l => l.key === (target.laneKey ?? target.key));
    if (!lane) return null;
    const meta = lane.meta as { kind?: string; factionTitle?: string; isLeader?: boolean } | undefined;
    if (meta?.kind === 'faction') {
      return {
        title: lane.label,
        subtitle: 'Faction · click for details',
        hour,
        items: [{ label: 'Members', value: lane.subtitle ?? 'Not recorded', swatch: lane.colorSwatch }],
      };
    }
    const span = target.spanId
      ? lane.spans?.find(s => s.id === target.spanId)
      : lane.spans?.find(s => s.firstHour <= hour && hour <= s.lastHour) ?? lane.spans?.at(-1);
    return {
      title: lane.label,
      subtitle: meta?.factionTitle ?? 'Faction member',
      hour,
      items: [
        { label: 'Role', value: meta?.isLeader ? 'Leader' : 'Member' },
        ...(span ? [
          { label: span.heldBefore ? 'Member before' : 'Joined', value: formatDay(span.firstHour) },
          { label: 'Status', value: span.openAtPlayhead ? 'Still a member' : `Left ${formatDay(span.lastHour)}` },
        ] : []),
      ],
    };
  },
});

export const factionsDeclaration: TimelineWidgetDeclaration<FactionsProjectionOptions> = {
  family: 'timeline',
  kind: 'factions',
  label: 'Faction membership',
  subject: 'factions',
  emptyCampaign: 'Load a campaign to see its factions.',
  icon: 'shield',
  optionFields: OPTIONS,
  countries: { mode: 'multi', labels: true, allTagsForLabels: true, requireGroupsWhen: () => true },
  jumpToDate: true,
  keyByBranch: true,
  sources: ctx => ({
    owner: `factions:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`,
    entries: [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'branch-to-hour',
        coverage: { mode: 'population', form: 'all' },
        product: 'country-groups',
        followQuery: null,
        params: { branchId: ctx.branch.id },
        buildSpanParams: base => base,
      },
    }],
    allowEmptyFollow: true,
  }),
  projection: factionsProjection,
  projectionOptions: ctx => ({
    order: (ctx.options.order as FactionOrder) ?? 'founded',
    members: (ctx.options.members as MemberOrder) ?? 'joined',
    showEnded: ctx.options.ended !== 'hidden',
    branchFirst: ctx.branch.firstHour,
    surface: '#16181d',
    selectedTags: ctx.countries.all ? null : ctx.countries.tags,
  }),
};
