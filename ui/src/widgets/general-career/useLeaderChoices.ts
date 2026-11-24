import { nameReadMetadata } from '../../dashboard/nameReads.ts';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { useMemo } from 'react';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import { useProduct } from '../../dashboard/products';
import { useCountriesSelectionAt } from '../../dashboard/useCountriesSelectionAt';
import { leaderRequestSelection } from '../generals/generalsData';
import { leaderChoices, validateLeaderChoicesReply } from './choicesData';
import { productRequest } from '../../dashboard/request.ts';

const LOADING = { loading: true } as const;

export function useLeaderChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const { requestTags, selectionReason, group } = useCountriesSelectionAt(widget, runtime, true);
  const request = track && requestTags !== undefined ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'unit-leader-choices',
    params: { branchId: track.branch.id, selection: leaderRequestSelection(requestTags) } }) : null;
  const { data, error, retry } = useProduct(runtime.client, request, true, validateLeaderChoicesReply, track?.source);
  const choices = data?.choices;
  const wanted = useMemo(() => choices?.flatMap(([, , owner, hour]) => owner === null ? [] : [{ hour, tag: owner }]) ?? [], [choices]);
  const names = useCountryNamesAt(runtime.client, track, wanted, true);
  if (!track || selectionReason) return { leader: [] };
  if (error) return { leader: { error, retry } };
  if (group.error) return { leader: { error: group.error, retry: group.retry } };
  if (!choices) return { leader: LOADING };
  return { leader: leaderChoices(choices, track.source.metadata.campaignKey, (hour, tag) => nameReadMetadata(names.get(nameKey(hour, tag)))) };
}
