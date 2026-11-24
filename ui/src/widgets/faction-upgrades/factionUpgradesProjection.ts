import { formatDay } from '../../dashboard/calendar.ts';
import { nextBoundaryAfter, resolveCountryChip } from '../../families/timeline/helpers.ts';
import type { TimelineModel, TimelineProjection, TimelineSectionGroup, TimelineSectionItem } from '../../families/timeline/model.ts';
import { registerTimelineProjection } from '../../families/timeline/registry.ts';
import type { FactionRulesReply, FactionSelectionRun } from '../../generated/dashboard';
import { eventHours } from './upgradesData.ts';

export interface FactionUpgradesProjectionOptions {
  readonly factionName: string;
  readonly endedProgrammes: boolean;
  readonly branchFirst: number;
}

function latestActiveRun(runs: readonly FactionSelectionRun[], hour: number): FactionSelectionRun | null {
  let found: FactionSelectionRun | null = null;
  for (const r of runs) {
    if (r.firstHour <= hour && hour <= r.lastHour) found = r;
  }
  return found;
}

function pastRunsBefore(runs: readonly FactionSelectionRun[], hour: number): FactionSelectionRun[] {
  return runs.filter(r => r.lastHour < hour);
}

export const factionUpgradesProjection: TimelineProjection<FactionUpgradesProjectionOptions> = registerTimelineProjection({
  kind: 'faction-upgrades',

  nextChange(readView) {
    const reply = readView.panel<FactionRulesReply>('main');
    if (!reply) return Infinity;
    return nextBoundaryAfter(eventHours(reply), readView.hour);
  },

  project(readView, opt): TimelineModel {
    const hour = readView.hour;
    const reply = readView.panel<FactionRulesReply>('main');
    const extent: [number, number] = [opt.branchFirst, Math.max(opt.branchFirst + 1, hour)];
    if (!reply) {
      return {
        mode: 'sections',
        hour,
        extent,
        playhead: hour,
        headerSummary: opt.factionName || null,
        sections: [],
        empty: 'Loading faction rules\u2026',
      };
    }

    const groupNames = new Map(reply.groups.map(g => [g.group, g.name ?? `Group ${g.group}`]));
    const activeRules: TimelineSectionItem[] = [];
    const pastRules: TimelineSectionItem[] = [];
    const activeUpgrades: TimelineSectionItem[] = [];
    const pastUpgrades: TimelineSectionItem[] = [];

    for (const lane of reply.rules) {
      const active = latestActiveRun(lane.runs, hour);
      if (active) {
        const missing = !active.name;
        activeRules.push({
          key: `rule:${lane.slot}:${active.firstHour}`,
          label: active.name ?? 'Name not recorded',
          missingName: missing,
          rightText: `Since ${formatDay(active.firstHour)}`,
          meta: { section: 'Rules', firstHour: active.firstHour, lastHour: active.lastHour, active: true, reason: active.reason },
        });
      }
      for (const past of pastRunsBefore(lane.runs, hour)) {
        pastRules.push({
          key: `past-rule:${lane.slot}:${past.firstHour}`,
          label: past.name ?? 'Name not recorded',
          missingName: !past.name,
          rightText: `Ended ${formatDay(past.lastHour)}`,
          muted: true,
          meta: { section: 'Past rules', firstHour: past.firstHour, lastHour: past.lastHour, active: false, reason: past.reason },
        });
      }
    }

    for (const lane of reply.programmes) {
      const active = latestActiveRun(lane.runs, hour);
      if (active) {
        activeUpgrades.push({
          key: `prog:${lane.slot}:${active.firstHour}`,
          label: active.name ?? 'Name not recorded',
          missingName: !active.name,
          rightText: `Since ${formatDay(active.firstHour)}`,
          meta: { section: 'Shared programmes', firstHour: active.firstHour, lastHour: active.lastHour, active: true, reason: active.reason },
        });
      }
      if (opt.endedProgrammes) {
        for (const past of pastRunsBefore(lane.runs, hour)) {
          pastUpgrades.push({
            key: `past-prog:${lane.slot}:${past.firstHour}`,
            label: past.name ?? 'Name not recorded',
            missingName: !past.name,
            rightText: `Ended ${formatDay(past.lastHour)}`,
            muted: true,
            meta: { section: 'Past programmes', firstHour: past.firstHour, lastHour: past.lastHour, active: false, reason: past.reason },
          });
        }
      }
    }

    for (const lane of reply.minimums) {
      const active = latestActiveRun(lane.runs, hour);
      if (active) {
        const grp = groupNames.get(lane.group);
        activeUpgrades.push({
          key: `min:${lane.group}:${active.firstHour}`,
          label: active.name ? `${active.name}${grp ? ` (${grp})` : ''}` : 'Name not recorded',
          missingName: !active.name,
          rightText: `Since ${formatDay(active.firstHour)}`,
          meta: { section: 'Faction minimum upgrades', firstHour: active.firstHour, lastHour: active.lastHour, active: true, reason: active.reason },
        });
      }
    }

    for (const lane of reply.doctrines) {
      const active = latestActiveRun(lane.runs, hour);
      if (active) {
        activeUpgrades.push({
          key: `doc:${lane.slot}:${active.firstHour}`,
          label: active.name ?? 'Name not recorded',
          missingName: !active.name,
          rightText: `Since ${formatDay(active.firstHour)}`,
          meta: { section: 'Shared doctrines', firstHour: active.firstHour, lastHour: active.lastHour, active: true, reason: active.reason },
        });
      }
    }

    for (const member of reply.members) {
      const active = latestActiveRun(member.runs, hour);
      if (active) {
        const chip = resolveCountryChip(readView, member.tag, hour);
        const grp = groupNames.get(member.group);
        activeUpgrades.push({
          key: `mem:${member.key}:${active.firstHour}`,
          label: active.name ? `${active.name} \u00b7 ${chip.name}` : `${chip.name}${grp ? ` (${grp})` : ''}`,
          missingName: !active.name,
          rightText: `Since ${formatDay(active.firstHour)}`,
          meta: { section: 'Member upgrades', firstHour: active.firstHour, lastHour: active.lastHour, active: true, reason: active.reason },
        });
      }
    }

    const sections: TimelineSectionGroup[] = [];
    if (activeRules.length > 0) {
      sections.push({
        key: 'rules',
        title: 'Rules',
        countText: `${activeRules.length} in force`,
        column: 0,
        items: activeRules,
      });
    }
    if (activeUpgrades.length > 0) {
      sections.push({
        key: 'upgrades',
        title: 'Upgrades',
        countText: String(activeUpgrades.length),
        column: activeRules.length > 0 ? 1 : 0,
        items: activeUpgrades,
      });
    }
    if (pastRules.length > 0) {
      sections.push({
        key: 'past-rules',
        title: 'Past rules',
        countText: String(pastRules.length),
        column: 'full',
        items: pastRules,
      });
    }
    if (pastUpgrades.length > 0) {
      sections.push({
        key: 'past-upgrades',
        title: 'Past programmes',
        countText: String(pastUpgrades.length),
        column: 'full',
        items: pastUpgrades,
      });
    }

    if (!sections.length) {
      return {
        mode: 'sections',
        hour,
        extent,
        playhead: hour,
        headerSummary: opt.factionName || null,
        sections: [],
        empty: 'This faction\u2019s rules were not recorded.',
        emptyKind: 'unavailable',
        warnings: reply.warnings,
      };
    }

    return {
      mode: 'sections',
      hour,
      extent,
      playhead: hour,
      headerSummary: opt.factionName || null,
      sections,
      warnings: reply.warnings,
      nextChangeAt: nextBoundaryAfter(eventHours(reply), hour),
    };
  },

  tooltip(model, target, hour) {
    for (const sec of model.sections ?? []) {
      const item = sec.items.find(i => i.key === target.key);
      if (item) {
        const meta = item.meta as { section?: string; firstHour?: number; lastHour?: number; active?: boolean; reason?: string | null } | undefined;
        return {
          title: item.missingName ? 'Name not recorded' : item.label,
          subtitle: meta?.section ?? sec.title,
          hour,
          items: [
            ...(meta?.firstHour !== undefined ? [{ label: 'Since', value: formatDay(meta.firstHour) }] : []),
            { label: 'Status', value: meta?.active ? 'In force' : meta?.lastHour !== undefined ? `Ended ${formatDay(meta.lastHour)}` : 'Ended' },
            ...(meta?.reason ? [{ label: 'Name', value: meta.reason, tone: 'muted' as const }] : []),
          ],
        };
      }
    }
    return null;
  },
});
