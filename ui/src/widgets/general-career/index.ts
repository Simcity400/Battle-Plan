import { nameReadMetadata } from '../../dashboard/nameReads.ts';
import type { OptionChoices, WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { useLeaderCountries, useLeaders } from '../generals/useLeaders';
import { useMemo } from 'react';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import { leaderChoices } from './choicesData';
import GeneralCareerWidget from './GeneralCareerWidget';

function useLeaderChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const { requestSelection, countries, selectionReason } = useLeaderCountries(widget, runtime, track?.branch.lastHour ?? 0, true);
  const { data } = useLeaders(runtime, widget.trackIds[0], requestSelection, 'choices', true);
  const wanted = useMemo(() => data?.leaders.flatMap(l => {
    const run = l.runs.at(-1); const tag = run && data.roles[run.value.role].owner;
    return tag && run ? [{ hour: run.lastHour, tag }] : [];
  }) ?? [], [data]);
  const names = useCountryNamesAt(runtime.client, track, wanted, true);
  return { leader: data && countries !== undefined && !selectionReason ? leaderChoices(data, track!.source.metadata.campaignKey, (hour, tag) => nameReadMetadata(names.get(nameKey(hour, tag)))) : [] };
}
export default { render: GeneralCareerWidget, useOptionChoices: useLeaderChoices } satisfies WidgetRenderer;
