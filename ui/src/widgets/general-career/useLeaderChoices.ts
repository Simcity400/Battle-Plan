import { nameReadMetadata } from '../../dashboard/nameReads.ts';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { useMemo } from 'react';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import { selectedCountryKeys } from '../../dashboard/countrySelection.ts';
import { useProduct } from '../../dashboard/products';
import { choicesSelection, leaderChoices, validateLeaderChoicesReply } from './choicesData';
import { productRequest } from '../../dashboard/request.ts';

const LOADING = { loading: true } as const;

export function useLeaderChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const selection = track ? choicesSelection(widget.countries, selectedCountryKeys(widget.countries, track)) : null;
  const request = track && selection ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'unit-leader-choices',
    params: { branchId: track.branch.id, selection } }) : null;
  const { data, error, retry } = useProduct(runtime.client, request, true, validateLeaderChoicesReply, track?.source);
  const choices = data?.choices;
  const wanted = useMemo(() => choices?.flatMap(([, , owner, hour]) => owner === null ? [] : [{ hour, tag: owner }]) ?? [], [choices]);
  const names = useCountryNamesAt(runtime.client, track, wanted, true);
  if (!track || !selection) return { leader: [] };
  if (error) return { leader: { error, retry } };
  if (!choices) return { leader: LOADING };
  return { leader: leaderChoices(choices, track.source.metadata.campaignKey, (hour, tag) => nameReadMetadata(names.get(nameKey(hour, tag)))) };
}
