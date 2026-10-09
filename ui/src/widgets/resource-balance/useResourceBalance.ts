import { useMemo } from 'react';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { CountrySelection, WidgetSpec } from '../../dashboard/model';
import { setToken } from '../../dashboard/options';
import { useProduct, useProducts } from '../../dashboard/products';
import { NumericReply } from '../../dashboard/numericReply';
import { groupNumbers } from '../../dashboard/groupsReply';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { BoundTrack } from '../../dashboard/model';
import { resolveCountries } from '../../dashboard/groups';
import type { CountryGroupReply, ResourceBalanceReply } from '../../generated/dashboard';
import { resourceName, resourceNumbers, validateResourceBalanceReply } from './balanceData';
import { resourceCountryData, resourceCountrySpans } from './resourceDrill';
import type { ResourceCountrySelection } from './resourceDrill';

/** The widget's country and its resource balance reply. */
export function useResourceBalance(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'resource-balance',
    params: { branchId: track.branch.id, tag } } : null;
  const product = useProduct<ResourceBalanceReply>(runtime.client, request, active, validateResourceBalanceReply);
  const retained = useMemo(() => new NumericReply<ResourceBalanceReply>(), [runtime.client]);
  const key = JSON.stringify(request);
  const numbers = retained.update(key, product.data, resourceNumbers);
  return { track, tag, ...product, numbers, data: product.data ?? numbers,
    contentPending: !!numbers && !product.data && !product.error };
}

/** Only the dated holder's aliases supply a selected breakdown. */
export function useResourceCountry(selection: ResourceCountrySelection | null, runtime: WidgetRuntime, track: BoundTrack | undefined,
  active: boolean, group?: CountrySelection | null) {
  const catalogue = useCountryGroups(runtime, track, active && !!selection);
  const identities = useMemo(() => new NumericReply<CountryGroupReply>(), [runtime.client]);
  const history = identities.update(JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id]), catalogue.data, groupNumbers);
  const members = useMemo(() => group?.kind === 'group'
    ? resolveCountries(group, [], history, track?.branch.lastHour ?? 0, Number.MAX_SAFE_INTEGER) : null,
  [group, history, track?.branch.lastHour]);
  const spans = useMemo(() => selection && history && track && history.branchId === track.branch.id
    && selection.campaign === track.source.metadata.campaignKey && selection.branch === track.branch.id
    ? resourceCountrySpans(history.countryIdentities, selection, track.branch.firstHour, track.branch.lastHour, members?.reason ? new Map() : members?.membership) : null,
  [selection, history, track?.source.metadata.campaignKey, track?.branch.id, track?.branch.firstHour, track?.branch.lastHour, members]);
  const requests = track && spans ? [...spans.keys()].map(tag => ({ sourceId: track.source.id, revision: track.source.revision,
    product: 'resource-balance', params: { branchId: track.branch.id, tag } })) : [];
  const product = useProducts<ResourceBalanceReply>(runtime.client, requests, active && !!selection, value => {
    const reply = validateResourceBalanceReply(value);
    if (!track || reply.branchId !== track.branch.id || !spans?.has(reply.tag)) throw new Error('The selected resource history belongs to another country or branch.');
    return reply;
  });
  const store = useMemo(() => new NumericReply<ResourceBalanceReply[]>(), [runtime.client]);
  const numbers = store.update(JSON.stringify(requests), product.data, replies => replies.map(resourceNumbers));
  const data = useMemo(() => spans && selection && (product.data ?? numbers)
    ? resourceCountryData((product.data ?? numbers)!, spans, selection.tag) : undefined, [product.data, numbers, spans, selection]);
  const reason = catalogue.error ?? product.error ?? members?.reason ?? (!history ? 'Loading the selected country identity…'
    : !spans ? 'The selected country identity is unknown or conflicting at its original choice.'
      : !data ? 'Loading the selected resource history…' : null);
  return { data, numbers, spans, reason, error: catalogue.error ?? product.error, retry: catalogue.error ? catalogue.retry : product.retry };
}

/** The Resources setting's choices: the campaign's own resources, named and pictured
 * as the game does, one per token (a repeated declaration is chosen with its token). */
export function useResourceChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data } = useResourceBalance(widget, runtime, true);
  return { resources: (data?.resources ?? []).filter(item => item.ordinal === 1).map(item => ({ value: item.token, label: resourceName(item), icon: item.icon })),
    partResource: (data?.resources ?? []).map(item => ({ value: setToken(JSON.stringify([item.token, item.ordinal])), label: resourceName(item), icon: item.icon })) };
}
