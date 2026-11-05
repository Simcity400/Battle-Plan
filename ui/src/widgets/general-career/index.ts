import { nameReadMetadata } from '../../dashboard/nameReads.ts';
import type { OptionChoices, WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { useLeaderCountries } from '../generals/useLeaders';
import { useMemo } from 'react';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import { useProduct } from '../../dashboard/products';
import { finalLeaderChoices, leaderChoices, validateLeaderChoicesReply } from './choicesData';
import GeneralCareerWidget from './GeneralCareerWidget';

const LOADING = { loading: true } as const;

function useLeaderChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const { requestSelection, countries, selectionReason, group } = useLeaderCountries(widget, runtime, track?.branch.lastHour ?? 0, true);
  const request = track && requestSelection ? { sourceId: track.source.id, revision: track.source.revision, product: 'unit-leaders',
    params: { branchId: track.branch.id, selection: requestSelection, view: 'choices', leaders: 'all' } } : null;
  const { data, error, retry } = useProduct(runtime.client, request, true, validateLeaderChoicesReply, track?.source);
  const choices = finalLeaderChoices(data);
  const wanted = useMemo(() => choices?.flatMap(([, , owner, hour]) => owner === null ? [] : [{ hour, tag: owner }]) ?? [], [choices]);
  const names = useCountryNamesAt(runtime.client, track, wanted, true);
  if (!track || selectionReason) return { leader: [] };
  if (error) return { leader: { error, retry } };
  // A failed country-groups read never starts the leaders read; its failure is the picker's.
  if (group.error) return { leader: { error: group.error, retry: group.retry } };
  if (!choices || countries === undefined) return { leader: LOADING };
  return { leader: leaderChoices(choices, track.source.metadata.campaignKey, (hour, tag) => nameReadMetadata(names.get(nameKey(hour, tag)))) };
}
export default { render: GeneralCareerWidget, useOptionChoices: useLeaderChoices } satisfies WidgetRenderer;
