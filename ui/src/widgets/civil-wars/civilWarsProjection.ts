import { formatDay } from '../../dashboard/calendar.ts';
import { unknownHours } from '../../dashboard/politicsHistory.ts';
import { startUnseen } from '../../charts/timeline/runState.ts';
import type { TimelineWidgetDeclaration } from '../../families/timeline/declaration.ts';
import { clipSpansToHour, formatDurationShort, nextBoundaryAfter, paletteAt, resolveCountryChip } from '../../families/timeline/helpers.ts';
import type { TimelineCountryChip, TimelineLaneModel, TimelineModel, TimelineProjection } from '../../families/timeline/model.ts';
import { registerTimelineProjection } from '../../families/timeline/registry.ts';
import type { CivilWarsReply, HourSpan } from '../../generated/dashboard';
import { changeHours, civilWarsAt, summaryText, validateCivilWarsReply } from './civilWarsData.ts';
import { OPTIONS } from './definition.ts';

export interface CivilWarsProjectionOptions {
  readonly showEnded: boolean;
  readonly branchFirst: number;
  readonly branchGaps: readonly HourSpan[];
  readonly surface: string;
  readonly selectedTags: readonly string[] | null;
}

export const civilWarsProjection: TimelineProjection<CivilWarsProjectionOptions> = registerTimelineProjection({
  kind: 'civil-wars',

  nextChange(readView, opt) {
    const reply = readView.panel<CivilWarsReply>('main');
    if (!reply) return Infinity;
    const unread = unknownHours(opt.branchGaps, reply.known, opt.branchFirst, reply.lastHour);
    return nextBoundaryAfter(changeHours(reply, unread), readView.hour);
  },

  project(readView, opt): TimelineModel {
    const hour = readView.hour;
    const reply = readView.panel<CivilWarsReply>('main');
    const extent: [number, number] = [opt.branchFirst, Math.max(opt.branchFirst + 1, hour)];
    if (!reply) {
      return { mode: 'lanes', hour, extent, playhead: hour, lanes: [], empty: 'Loading civil wars\u2026' };
    }

    const unread = unknownHours(opt.branchGaps, reply.known, opt.branchFirst, reply.lastHour);
    const view = civilWarsAt(reply, hour, {
      ended: opt.showEnded,
      countryIds: null,
      identities: null,
      unread,
      lastHour: reply.lastHour,
    });

    const tagSet = opt.selectedTags ? new Set(opt.selectedTags) : null;
    const filteredWars = tagSet
      ? view.wars.filter(w => w.sides.some(s => s.tag !== null && tagSet.has(s.tag)))
      : view.wars;

    const headerSummary = summaryText({
      ...view,
      wars: filteredWars,
      total: filteredWars.length,
      ongoing: filteredWars.filter(w => w.ongoing).length,
    });

    if (!filteredWars.length) {
      const msg = view.reached && tagSet && !filteredWars.length
        ? 'No civil wars for these countries.'
        : view.total
          ? 'No ongoing civil wars.'
          : 'No civil wars yet.';
      return {
        mode: 'lanes',
        hour,
        extent,
        playhead: hour,
        headerSummary,
        lanes: [],
        empty: msg,
        warnings: reply.warnings,
      };
    }

    const firstStart = Math.min(...filteredWars.map(w => w.war.firstHour));
    const windowStart = Math.max(opt.branchFirst, firstStart);
    const warExtent: [number, number] = [windowStart, Math.max(windowStart + 1, hour)];
    const lanes: TimelineLaneModel[] = [];

    for (const cw of filteredWars) {
      const warKey = `cw:${cw.war.id}`;
      const titleChip = resolveCountryChip(readView, cw.titleTag, cw.war.firstHour, { fallbackName: 'Unknown country' });
      const title = `Civil war in ${titleChip.name}`;

      const chips: { chip: TimelineCountryChip; side: typeof cw.sides[number]; color: string }[] = cw.sides.map((s, idx) => {
        const chip = resolveCountryChip(readView, s.tag, Math.min(hour, s.labelHour), {
          dimmed: s.muted,
          fallbackName: 'Unknown side',
        });
        const color = chip.color ?? paletteAt(opt.surface, idx);
        return { chip, side: s, color };
      });

      const primary = chips[0];
      const opposing = chips.slice(1);
      const attackerLead = primary?.chip ?? titleChip;
      const defenderLead = opposing[0]?.chip ?? titleChip;
      const defenderAllies = opposing.slice(1).map(c => c.chip);
      const topColor = primary?.color ?? paletteAt(opt.surface, 0);
      const botColor = opposing[0]?.color ?? paletteAt(opt.surface, 1);

      const winnerChip = cw.outcome && cw.winnerTag
        ? resolveCountryChip(readView, cw.winnerTag, Math.min(hour, cw.outcome.hour))
        : null;

      const warEnd = cw.ongoing ? hour : Math.min(hour, cw.war.lastHour);
      const outcome = cw.ongoing
        ? {
            status: 'running' as const,
            label: `Ongoing ${formatDurationShort(Math.max(0, hour - cw.war.firstHour))}`,
          }
        : winnerChip && cw.outcome
          ? {
              status: 'finished' as const,
              label: `${winnerChip.name} won`,
              detail: formatDay(cw.outcome.hour),
            }
          : {
              status: 'ended' as const,
              label: 'Ended',
              detail: formatDay(warEnd),
            };

      lanes.push({
        key: warKey,
        kind: 'matchup',
        label: title,
        expandable: true,
        defaultOpen: cw.ongoing,
        matchup: {
          title,
          attackers: { lead: attackerLead, allies: [], dimmed: primary?.side.muted },
          defenders: { lead: defenderLead, allies: defenderAllies, dimmed: opposing.every(o => o.side.muted) },
          outcome,
        },
        spans: [{
          id: `${warKey}:span`,
          firstHour: cw.war.firstHour,
          lastHour: Math.max(cw.war.firstHour, warEnd),
          color: topColor,
          bottomColor: botColor,
          openAtPlayhead: cw.ongoing,
          ended: !cw.ongoing,
        }],
        auditValue: `${title}|${outcome.label}`,
        meta: { kind: 'civil-war', warId: cw.war.id, title, outcomeLabel: outcome.label, startHour: cw.war.firstHour, endHour: warEnd, ongoing: cw.ongoing },
      });

      for (const { chip, side, color } of chips) {
        const sEnd = side.live && cw.ongoing ? hour : Math.min(hour, side.side.lastHour);
        const joinedLate = side.side.firstHour > cw.war.firstHour;
        const exitText = side.exit ? `Annexed ${formatDay(side.exit.hour)}` : undefined;
        lanes.push({
          key: `cw-side:${cw.war.id}:${side.index}`,
          kind: 'participant',
          parentKey: warKey,
          depth: 1,
          label: chip.name,
          subtitle: joinedLate ? `Joined ${formatDay(side.side.firstHour)}` : undefined,
          flag: chip.flag,
          flagTag: side.tag ?? undefined,
          flagDimmed: side.muted,
          dimmed: side.muted,
          spans: [{
            id: `cw-side:${cw.war.id}:${side.index}:bar`,
            firstHour: side.side.firstHour,
            lastHour: Math.max(side.side.firstHour, sEnd),
            color,
            heldBefore: startUnseen(side.side.firstHour, opt.branchFirst, unread),
            openAtPlayhead: side.live && cw.ongoing,
            ended: !(side.live && cw.ongoing),
            endText: exitText,
          }],
          auditValue: `${chip.name}|${exitText ?? (side.winner ? 'Won' : 'Active')}`,
          meta: { kind: 'civil-war-side', warTitle: title, sideName: chip.name, startHour: side.side.firstHour, endHour: sEnd, exitText, winner: side.winner },
        });
      }
    }

    return {
      mode: 'lanes',
      hour,
      extent: warExtent,
      playhead: hour,
      headerSummary,
      lanes,
      unobserved: clipSpansToHour(unread, hour),
      iconsOnBars: false,
      labelWidth: 280,
      outcomeWidth: 196,
      warnings: reply.warnings,
      nextChangeAt: nextBoundaryAfter(changeHours(reply, unread), hour),
    };
  },

  tooltip(model, target, hour) {
    const lane = model.lanes?.find(l => l.key === (target.laneKey ?? target.key));
    if (!lane) return null;
    const meta = lane.meta as {
      kind?: string;
      title?: string;
      warTitle?: string;
      outcomeLabel?: string;
      startHour?: number;
      endHour?: number;
      ongoing?: boolean;
      exitText?: string;
      winner?: boolean;
    } | undefined;
    if (meta?.kind === 'civil-war') {
      return {
        title: meta.title ?? lane.label,
        subtitle: meta.ongoing ? 'Ongoing civil war' : 'Civil war',
        hour,
        items: [
          ...(meta.startHour !== undefined ? [{ label: 'Outbreak', value: formatDay(meta.startHour) }] : []),
          { label: 'Outcome', value: meta.outcomeLabel ?? 'Ended' },
        ],
      };
    }
    return {
      title: lane.label,
      subtitle: meta?.warTitle ?? 'Civil war side',
      hour,
      items: [
        ...(meta?.startHour !== undefined ? [{ label: 'From', value: formatDay(meta.startHour) }] : []),
        ...(meta?.exitText ? [{ label: 'Exit', value: meta.exitText }] : []),
        ...(meta?.winner ? [{ label: 'Outcome', value: 'Won' }] : []),
      ],
    };
  },
});

export const civilWarsDeclaration: TimelineWidgetDeclaration<CivilWarsProjectionOptions> = {
  family: 'timeline',
  kind: 'civil-wars',
  label: 'Civil wars',
  subject: 'civil wars',
  emptyCampaign: 'Load a campaign to see its civil wars.',
  icon: 'flag-torn',
  optionFields: OPTIONS,
  countries: { mode: 'multi', labels: true, colours: true, allTagsForLabels: true },
  jumpToDate: true,
  keyByBranch: true,
  sources: ctx => ({
    owner: `civil-wars:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`,
    entries: [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'branch-to-hour',
        coverage: { mode: 'population', form: 'all' },
        product: 'civil-wars',
        followQuery: null,
        params: { branchId: ctx.branch.id },
        buildSpanParams: base => base,
        validate: validateCivilWarsReply,
      },
    }],
    allowEmptyFollow: true,
  }),
  projection: civilWarsProjection,
  projectionOptions: ctx => ({
    showEnded: ctx.options.ended !== 'hidden',
    branchFirst: ctx.branch.firstHour,
    branchGaps: ctx.branch.gaps,
    surface: '#16181d',
    selectedTags: ctx.countries.all ? null : ctx.countries.tags,
  }),
};
