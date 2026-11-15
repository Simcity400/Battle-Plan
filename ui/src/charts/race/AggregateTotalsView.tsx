import { useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import { useCountryLooks } from '../../dashboard/countryLooks';
import { groupLabel, resolveCountries } from '../../dashboard/groups';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import FactionTotalsView from './FactionTotalsView';
import type { FactionTotalsViewProps } from './FactionTotalsView';
import { groupTotal } from './factionTotalsData.ts';

/** Whether a widget's countries are a group shown as one total (`aggregate`). */
export const aggregated = (countries: FactionTotalsViewProps['widget']['countries']) => countries.kind === 'group' && countries.aggregate;

/** A group's members summed as one total: a faction's own total, or any other group's members (players, a war's
 * side, a continent's countries) summed at each hour they are members, a race bar and a line of one. */
export default function AggregateTotalsView(props: FactionTotalsViewProps) {
  const { widget, runtime, active } = props;
  const selection = widget.countries;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const faction = selection.kind === 'group' && selection.group.type === 'faction' ? selection.group.id : null;
  const groups = useCountryGroups(runtime, track, active && faction === null);
  const lastHour = track?.branch.lastHour ?? 0;
  const resolved = useMemo(() => resolveCountries(selection, [], groups.data, lastHour, Number.MAX_SAFE_INTEGER), [selection, groups.data, lastHour]);
  const warTag = selection.kind === 'group' && selection.group.type === 'war' ? [selection.group.tag] : [];
  const looks = useCountryLooks(runtime.client, track, warTag, lastHour, active && warTag.length > 0);
  const label = selection.kind === 'group' ? groupLabel(selection.group, groups.data, tag => looks.name(tag), lastHour) : 'Countries';
  const group = useMemo(() => faction === null && groups.data && !resolved.reason ? groupTotal(resolved, label) : undefined, [faction, groups.data, resolved, label]);
  if (faction !== null) return <FactionTotalsView {...props} only={faction} />;
  if (groups.error && !groups.data) return <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />;
  if (!groups.data) return <WidgetState kind="loading" label="Loading country group…" />;
  if (resolved.reason) return <WidgetState kind="empty" icon="shield" message={resolved.reason} />;
  return <FactionTotalsView {...props} group={group} />;
}
