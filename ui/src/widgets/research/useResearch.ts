import { useMemo } from 'react';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import { usePreparedResearch } from './preparedResearch';
import { setToken } from '../../dashboard/options';
import { useChartTokens } from '../../charts/useChartTokens';
import type { ResearchHistory } from '../../generated/dashboard';
import { researchFolderList, researchFolderLooks } from './researchData';

/** Every technology a history names, assignments and slot states alike. */
export function historyTokens(history: ResearchHistory): string[] {
  return [...new Set(history.slots.flatMap(slot => [...slot.entries.map(entry => entry.token),
    ...(slot.states ?? []).flatMap(state => state.token ? [state.token] : [])]))];
}

/** The research reply for a widget's campaign and country, shared by every
 * research widget showing that country. */
export function useResearchReply(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  return { track, tag, ...usePreparedResearch(runtime.client, track?.source, track?.branch.id, tag, active) };
}

/** The folder filter's choices: the folders this country's research shows, in
 * the content's order. */
export function useResearchChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data } = useResearchReply(widget, runtime, true);
  const { categories } = useChartTokens();
  return useMemo(() => ({ folders: data ? researchFolderList(researchFolderLooks(data, historyTokens(data.history), categories))
    .map(look => ({ value: setToken(look.key), label: look.name })) : [] }), [data, categories]);
}
