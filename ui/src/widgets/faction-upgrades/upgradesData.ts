import type { Failure } from '../../dashboard/queryError';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import type { TimelineLane, TimelineSpan } from '../../charts/timeline/layout';
import type { FactionMemberUpgradeLane, FactionRulesReply, FactionSelectionRun } from '../../generated/dashboard';

type SelectionRun = FactionSelectionRun;
type RulesReply = FactionRulesReply;

export const selectionName = (run: SelectionRun) => run.name || 'Name unavailable';

/** The hour a member lane's holder is named at: the last hour of its last recorded selection,
 * when the lane's own holder held the tag. A lane ends with a run without a selection when
 * it goes away, which reaches past any later holder of the tag, so that run never names it.
 * Null when the lane has no recorded selection (it is not drawn). */
export const memberNameHour = (member: Pick<FactionMemberUpgradeLane, 'runs'>) => member.runs.filter(run => run.token !== null).at(-1)?.lastHour ?? null;

/** What each lane and span is about, for tooltips and menus. */
export interface LaneInfo { nameFailure?: Failure; section: string; slot: string; runs: readonly SelectionRun[] }

/** Spans for one lane's recorded selections, up to `until` (Live) and dropping runs with no
 * recorded selection; the run holding the shown hour is emphasised. */
function spans(id: string, runs: readonly SelectionRun[], at: number, until: number | null): TimelineSpan[] {
  return runs.flatMap((run, index) => run.token === null || (until !== null && run.firstHour > until) ? [] : [{
    id: `${id}:${index}`, firstHour: run.firstHour, lastHour: until === null ? run.lastHour : Math.min(run.lastHour, until),
    label: selectionName(run), emphasis: run.firstHour <= at && at <= run.lastHour, openEnd: until !== null && run.lastHour > until,
  }]);
}

/** Lanes under headings: rules, shared programmes, faction minimum upgrades, members'
 * upgrades and shared doctrines; a lane without any recorded selection is left out. */
export function upgradeLanes(reply: RulesReply, at: number, until: number | null, options: { endedProgrammes: boolean; groupName: (group: number) => string; country: (tag: string, hour: number) => ReturnType<CountryLabel> }) {
  const lanes: TimelineLane[] = [], info = new Map<string, LaneInfo>();
  const section = (key: string, title: string, items: readonly { id: string; label: string; nameFailure?: Failure; runs: readonly SelectionRun[] }[]) => {
    const drawn = items.map(item => ({ item, spans: spans(item.id, item.runs, at, until) })).filter(entry => entry.spans.length);
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
    .filter(programme => options.endedProgrammes || programme.runs.some(run => run.token !== null && run.lastHour >= at))
    .map(programme => ({ id: `programme:${programme.slot}`, label: `Programme ${programme.slot + 1}`, runs: programme.runs })));
  section('minimums', 'Faction minimum upgrades', reply.minimums.map(minimum => ({ id: `minimum:${minimum.group}`, label: options.groupName(minimum.group), runs: minimum.runs })));
  section('members', 'Member upgrades', reply.members.flatMap(member => { const hour = memberNameHour(member);
    if (hour === null) return [];
    const country = options.country(member.tag, hour);
    return [{ id: `member:${member.key}`, label: `${country.name} · ${options.groupName(member.group)}`, nameFailure: country.nameFailure, runs: member.runs }]; }));
  section('doctrines', 'Shared doctrines', reply.doctrines.map(doctrine => ({ id: `doctrine:${doctrine.slot}`, label: `Doctrine ${doctrine.slot + 1}`, runs: doctrine.runs })));
  return { lanes, info };
}
