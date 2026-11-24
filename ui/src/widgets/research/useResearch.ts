import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useMemo } from 'react';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import { useProduct } from '../../dashboard/products.ts';
import { setToken } from '../../dashboard/options';
import { useChartTokens } from '../../charts/useChartTokens';
import type { ResearchData } from './types';
import { historyTokens, researchFolderList, researchFolderLooks, validateResearchReply } from './researchData';
import { productRequest } from '../../dashboard/request.ts';

function researchTarget(widget: WidgetSpec, runtime: WidgetRuntime) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryKey(track) ?? undefined;
  return { track, tag };
}

export function useResearchReply(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, covered = true) {
  const { track, tag } = researchTarget(widget, runtime);
  const request = track && tag && covered ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'research', params: { branchId: track.branch.id, country: countryFromKey(tag) } }) : null;
  return { track, tag, ...useProduct<ResearchData>(runtime.client, request, active, validateResearchReply, track?.source) };
}

export function useResearchChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data } = useResearchReply(widget, runtime, true);
  const { categories } = useChartTokens();
  return useMemo(() => ({ folders: data ? researchFolderList(researchFolderLooks(data, historyTokens(data.history), categories))
    .map(look => ({ value: setToken(look.key), label: look.name })) : [] }), [data, categories]);
}
