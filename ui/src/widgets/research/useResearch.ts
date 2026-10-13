import { useMemo } from 'react';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import { useProduct } from '../../dashboard/products';
import { setToken } from '../../dashboard/options';
import { useChartTokens } from '../../charts/useChartTokens';
import type { ResearchHistory, ResearchReply } from '../../generated/dashboard';
import { researchFolderList, researchFolderLooks, researchRequest, validateResearchReply } from './researchData';

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
  const request = useMemo(() => track && tag ? researchRequest(track.source.id, track.source.revision, { branchId: track.branch.id, tag }) : null,
    [track, tag]);
  return { track, tag, ...useProduct<ResearchReply>(runtime.client, request, active, validateResearchReply) };
}

/** The folder filter's choices: the folders this country's research shows, in
 * the content's order. */
export function useResearchChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data } = useResearchReply(widget, runtime, true);
  const { categories } = useChartTokens();
  return useMemo(() => ({ folders: data ? researchFolderList(researchFolderLooks(data, historyTokens(data.history), categories))
    .map(look => ({ value: setToken(look.key), label: look.name })) : [] }), [data, categories]);
}
