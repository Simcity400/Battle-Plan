import { useMemo } from 'react';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import { usePreparedResearch } from './preparedResearch';
import { setToken } from '../../dashboard/options';
import { useChartTokens } from '../../charts/useChartTokens';
import type { ResearchHistory } from '../../generated/dashboard';
import { researchFolderList, researchFolderLooks } from './researchData';
import type { ResearchUse } from './researchData';

/** Includes both slot-state tokens and assignment definitions, deduplicated separately. */
export function historyTokens(history: ResearchHistory): ResearchUse[] {
  const tokens = new Set<string>(), definitions = new Map<string, Set<number>>();
  for (const slot of history.slots) {
    for (const entry of slot.entries) {
      if (entry.databaseIndex == null) tokens.add(entry.token);
      else definitions.set(entry.token, (definitions.get(entry.token) ?? new Set<number>()).add(entry.databaseIndex));
    }
    for (const state of slot.states ?? []) if (state.token) tokens.add(state.token);
  }
  return [...tokens, ...[...definitions].flatMap(([token, indices]) => [...indices].map(index => [token, index] as const))];
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
