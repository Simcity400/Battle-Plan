import { useMemo } from 'react';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { contentKey, contentSet, setToken } from '../../dashboard/options';
import { useFollowedProduct, useProducts } from '../../dashboard/products';
import { NumericReply } from '../../dashboard/numericReply';
import type { EquipmentStockpileReply } from '../../generated/dashboard';
import { chartChoices, choiceKey, CHOSEN_KEYS, stockGroups, typeName, validateStockpileReply } from './stockpileData';
import type { Grouping } from './stockpileData';
import { pinnedGrouping, stockpileCountryData, stockpileNumbers } from './stockpileDrill';
import { datedCountrySpans } from '../../dashboard/datedCountry';
import type { DatedCountrySelection } from '../../dashboard/datedCountry';
import { groupNumbers } from '../../dashboard/groupsReply';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { resolveWindow } from '../../dashboard/model';
import type { BoundTrack } from '../../dashboard/model';
import { usePlaybackHour } from '../../dashboard/playback';
import type { Span } from '../../charts/series';
import type { CountryGroupReply } from '../../generated/dashboard';

/** The widget's country and its equipment stockpile reply. */
export function useEquipmentStockpile(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, within?: readonly Span[] | null) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  const clock = usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const window = track && resolveWindow(widget, track, runtime.tracks, clock);
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const prepared = !!runtime.client.preparedQuery;
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'equipment-stockpile',
    params: { branchId: track.branch.id, tag, ...(prepared ? { from: hour, to: hour, width: 1, within, included: contentSet(typeof widget.options?.included === 'string' ? widget.options.included : '').map(contentKey) } : {}) } } : null;
  const product = useFollowedProduct<EquipmentStockpileReply>(runtime.client, request, ['from', 'to'], active, validateStockpileReply, prepared ? track?.source : undefined);
  const retained = useMemo(() => new NumericReply<EquipmentStockpileReply>(), [runtime.client]);
  const numbers = retained.update(JSON.stringify(request), product.data, stockpileNumbers);
  return { track, tag, ...product, numbers, data: product.data ?? numbers, contentPending: !!numbers && !product.data && !product.error };
}

/** The settings' choices: the included types are the campaign's own types, named as
 * content names them; the lines over time are the groups of the grouping chosen that
 * can be drawn (`chartChoices`). Each is offered as a `setToken` token, so a key a
 * setting cannot store (such as a mod's `mod/rifles`) can be included and chosen alike. */
export function useStockpileChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data } = useEquipmentStockpile(widget, runtime, true);
  if (!data) return { included: [], chosen: [], chosenVariant: [], chosenDomain: [], partGroup: [] };
  const hour = Infinity;
  const choices = (grouping: Grouping) => chartChoices(data, grouping);
  return {
    included: stockGroups(data, 'type', hour).map(group => ({ value: setToken(group.key), label: typeName(data, group.key) })),
    [CHOSEN_KEYS.type]: choices('type'),
    partGroup: stockGroups(data, pinnedGrouping(widget.options), hour).flatMap(group => {
      const key = choiceKey(group, hour);
      return key == null ? [] : [{ value: setToken(key), label: group.label }];
    }),
  };
}

/** Alias histories share the native catalogue and retain same-client numeric replies. */
export function useStockpileCountry(selection: DatedCountrySelection | null, runtime: WidgetRuntime, track: BoundTrack | undefined, active: boolean, hour: number) {
  const catalogue = useCountryGroups(runtime, track, active && !!selection);
  const identities = useMemo(() => new NumericReply<CountryGroupReply>(), [runtime.client]);
  const history = identities.update(JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id]), catalogue.data, groupNumbers);
  const spans = useMemo(() => selection && history && track && history.branchId === track.branch.id
    && selection.campaign === track.source.metadata.campaignKey && selection.branch === track.branch.id
    ? datedCountrySpans(history.countryIdentities, selection, track.branch.firstHour, track.branch.lastHour) : null,
  [selection, history, track?.source.metadata.campaignKey, track?.branch.id, track?.branch.firstHour, track?.branch.lastHour]);
  const requests = track && spans ? [...spans.keys()].map(tag => ({ sourceId: track.source.id, revision: track.source.revision,
    product: 'equipment-stockpile', params: { branchId: track.branch.id, tag } })) : [];
  const validate = (value: unknown) => {
    const reply = validateStockpileReply(value);
    if (!track || reply.branchId !== track.branch.id || reply.campaignKey !== track.source.metadata.campaignKey || !spans?.has(reply.tag))
      throw new Error('The selected stockpile history belongs to another country, campaign or branch.');
    return reply;
  };
  const prepared = !!runtime.client.preparedQuery;
  const legacy = useProducts<EquipmentStockpileReply>(runtime.client, prepared ? [] : requests, active && !!selection && !prepared, validate);
  const incoming = useFollowedProduct<EquipmentStockpileReply[]>(runtime.client, prepared && track && spans && selection ? {
    sourceId: track.source.id, revision: track.source.revision, product: 'equipment-stockpile',
    params: { branchId: track.branch.id, tag: selection.tag, tags: [...spans.keys()], withinByTag: Object.fromEntries(spans), from: hour, to: hour, width: 1 },
  } : null, ['from', 'to'], active && !!selection && prepared, value => {
    if (!Array.isArray(value)) throw new Error('The selected stockpile country reply is malformed.');
    return value.map(validate);
  }, track?.source);
  const product = prepared ? incoming : legacy;
  const store = useMemo(() => new NumericReply<EquipmentStockpileReply[]>(), [runtime.client]);
  const numbers = store.update(JSON.stringify(requests), product.data, replies => replies.map(stockpileNumbers));
  const data = useMemo(() => spans && selection && (product.data ?? numbers)
    ? stockpileCountryData((product.data ?? numbers)!, spans, selection.tag) : undefined, [product.data, numbers, spans, selection]);
  const reason = ((catalogue.error && !history) || product.error ? 'Couldn’t load country.' : null) ?? (!history ? 'Loading country…'
    : !spans ? 'Choose a country again.' : !data ? 'Loading the selected stockpile history…' : null);
  return { data, numbers, spans, reason, error: (history ? undefined : catalogue.error) ?? product.error,
    retry: catalogue.error && !history ? catalogue.retry : product.retry, identityError: history ? catalogue.error : undefined, identityRetry: catalogue.retry };
}
