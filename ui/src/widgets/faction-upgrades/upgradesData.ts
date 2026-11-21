import type { Failure } from '../../dashboard/queryError';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import type { TimelineLane, TimelineSpan } from '../../charts/timeline/layout';
import type { FactionMemberUpgradeLane, FactionRulesReply, FactionSelectionRun } from '../../generated/dashboard';

export const selectionName = (run: FactionSelectionRun) => run.name || 'Name unavailable';

export const memberNameHour = (member: Pick<FactionMemberUpgradeLane, 'runs'>) => member.runs.at(-1)?.lastHour ?? null;

export function eventHours(reply: FactionRulesReply): number[] {
  const hours = new Set<number>();
  for (const lane of [...reply.rules, ...reply.programmes, ...reply.minimums, ...reply.members, ...reply.doctrines])
    for (const run of lane.runs) { hours.add(run.firstHour); hours.add(run.lastHour + 1); }
  return [...hours].sort((a, b) => a - b);
}

export function eventsThrough(hours: readonly number[], hour: number): number {
  let low = 0, high = hours.length;
  while (low < high) { const mid = (low + high) >> 1; if (hours[mid] <= hour) low = mid + 1; else high = mid; }
  return low;
}

export interface LaneInfo { nameFailure?: Failure; section: string; slot: string; runs: readonly FactionSelectionRun[] }

function spans(id: string, runs: readonly FactionSelectionRun[], at: number, live: boolean): TimelineSpan[] {
  return runs.flatMap((run, index) => live && run.firstHour > at ? [] : [{
    id: `${id}:${index}`, firstHour: run.firstHour, lastHour: run.lastHour,
    label: selectionName(run), emphasis: run.firstHour <= at && at <= run.lastHour, openEnd: live,
  }]);
}

export function upgradeLanes(reply: FactionRulesReply, at: number, live: boolean, options: { endedProgrammes: boolean; groupName: (group: number) => string;
  country: (tag: string | null, hour: number) => ReturnType<CountryLabel> }) {
  const lanes: TimelineLane[] = [], info = new Map<string, LaneInfo>();
  const section = (key: string, title: string, items: readonly { id: string; label: string; nameFailure?: Failure; runs: readonly FactionSelectionRun[] }[]) => {
    const drawn = items.map(item => ({ item, spans: spans(item.id, item.runs, at, live) })).filter(entry => entry.spans.length);
    if (!drawn.length) return;
    lanes.push({ id: `heading:${key}`, label: title, spans: [], heading: true, compact: true, singleLine: true });
    for (const { item, spans: shown } of drawn) {
      lanes.push({ id: item.id, label: item.label, spans: shown, compact: true, depth: 1, names: true, icons: false,
        muted: !shown.some(span => span.lastHour >= at) });
      info.set(item.id, { section: title, slot: item.label, runs: item.runs, ...(item.nameFailure ? { nameFailure: item.nameFailure } : {}) });
    }
  };
  section('rules', 'Rules', reply.rules.map(rule => ({ id: `rule:${rule.slot}`, label: `Rule ${rule.slot + 1}`, runs: rule.runs })));
  section('programmes', 'Shared programmes', reply.programmes
    .filter(programme => options.endedProgrammes || programme.runs.some(run => run.lastHour >= at))
    .map(programme => ({ id: `programme:${programme.slot}`, label: `Programme ${programme.slot + 1}`, runs: programme.runs })));
  section('minimums', 'Faction minimum upgrades', reply.minimums.map(minimum => ({ id: `minimum:${minimum.group}`, label: options.groupName(minimum.group), runs: minimum.runs })));
  section('members', 'Member upgrades', reply.members.flatMap(member => {
    const hour = memberNameHour(member);
    if (hour === null) return [];
    const country = options.country(member.tag, hour);
    return [{ id: `member:${member.key}`, label: `${country.name} · ${options.groupName(member.group)}`, nameFailure: country.nameFailure, runs: member.runs }];
  }));
  section('doctrines', 'Shared doctrines', reply.doctrines.map(doctrine => ({ id: `doctrine:${doctrine.slot}`, label: `Doctrine ${doctrine.slot + 1}`, runs: doctrine.runs })));
  return { lanes, info };
}
