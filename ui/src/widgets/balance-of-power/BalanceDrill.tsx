import { useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import DrillParts from '../../charts/drill/DrillParts';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import type { PoliticsHistoryReply } from '../../generated/dashboard';
import { BALANCE_UNAVAILABLE, changeText, driftParts, RANGE_UNAVAILABLE, SIDE_UNAVAILABLE, valueText, weeklyState } from './balanceOfPowerData';
import { BalanceHeadline, membersText } from './BalanceCountryView';
import type { BalanceDetails } from './useBalanceDetails';

const plain = (value: number) => String(value);

/** What makes up a balance's weekly change at one hour: the derived drift sources
 * and each member's recorded weekly and daily contribution, each signed and exact
 * and totalled on its own; the two are independent and never reconciled. `token` null
 * shows the record the game shows; the country is `tag`'s holder at `at` in `history`,
 * found by the tag it held at the detail's hour. */
export default function BalanceDrill({ runtime, track, history, tag, at, token, details, active }: {
  runtime: WidgetRuntime; track: BoundTrack; history: PoliticsHistoryReply; tag: string; at: number; token: string | null; details: BalanceDetails; active: boolean;
}) {
  const hour = details.hour;
  const state = useMemo(() => details.data ? weeklyState(history, tag, at, details.data, token) : null, [history, tag, at, details.data, token]);
  const record = state?.kind === 'record' ? state.record : null;
  const wanted = useMemo(() => record && hour !== null ? record.members.flatMap(member => member.tag === null ? [] : [{ hour, tag: member.tag }]) : [], [record, hour]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const parts = useMemo(() => record && hour !== null ? driftParts(record, labelAt(hour)) : null, [record, hour, labelAt]);
  if (hour === null) return <p className="drill-empty">Choose a day.</p>;
  if (details.error) return <WidgetState kind="error" what="the weekly change" detail={details.error} onRetry={details.retry} />;
  if (!details.data || !state) return <WidgetState kind="loading" label="Loading the weekly change…" />;
  if (state.kind === 'unread') return <WidgetState kind="unavailable" message="The balance of power was not recorded at this date." />;
  if (state.kind === 'preparing') return <WidgetState kind="loading" label="This range is still being prepared." />;
  if (!record || !parts) return <WidgetState kind="empty" message="No balance of power is active at this date." />;
  const detail = [record.range ? record.range.name ?? RANGE_UNAVAILABLE : null, valueText(record.value),
    record.trending ? `toward ${record.trending.name ?? SIDE_UNAVAILABLE}` : null].filter(Boolean).join(' · ');
  const memberNames = record.members.flatMap(member => member.tag === null ? [] : [labelAt(hour)(member.tag).name]);
  return <div className="balance-drill">
    <BalanceHeadline name={record.name ?? BALANCE_UNAVAILABLE} token={record.token} detail={detail} members={membersText(memberNames)} />
    {parts.sources === null && <p className="drill-empty">Sources unavailable.</p>}
    <DrillParts parts={parts.sources ?? []} format={plain} showShares={false} active={active}
      totalRow={{ label: 'Sources weekly', value: parts.sourced === null ? 'Not recorded' : changeText(parts.sourced) }} />
    <p className="chart-line-key"><strong>Members</strong></p>
    <DrillParts parts={parts.members} format={plain} active={active}
      totalRow={{ label: 'Members weekly', value: parts.weekly === null ? 'Not recorded' : changeText(parts.weekly) }} />
  </div>;
}
