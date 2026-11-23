import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useMemo, useSyncExternalStore } from 'react';
import type { Span } from '../../charts/series';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import { datedCountrySpans } from '../../dashboard/datedCountry';
import type { DatedCountrySelection } from '../../dashboard/datedCountry';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import { groupNumbers } from '../../dashboard/groupsReply';
import { useHeadProduct } from '../../dashboard/headProduct.ts';
import { useCampaignGate, useGate } from '../../dashboard/regionGate';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { NumericReply } from '../../dashboard/numericReply';
import { contentKey, contentSet, setToken } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { failureText } from '../../dashboard/queryError.ts';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { CountryGroupReply, StockpileHolder } from '../../generated/dashboard';
import { atReply, validateStockpileAt } from './stockpileAt.ts';
import { holderAt, stockpileHolders, stockpileView } from './stockpileCatalogue';
import type { PreparedStockpileReply } from './stockpileCatalogue';
import { chartChoices, choiceKey, CHOSEN_KEYS, stockGroups, typeName, validateStockpileCatalogue } from './stockpileData';
import type { Grouping } from './stockpileData';
import { pinnedGrouping } from './stockpileDrill';
import type { StockpileCatalogueReply } from './types';
import { SPEC, type RegionAction, regionFollows, axisAt } from '../../dashboard/regionTime.ts';
import { useRegion, useRegionHourAt } from '../../dashboard/widgetTime.ts';
import { useAtHead } from '../../dashboard/atHead.ts';
import { productRequest } from '../../dashboard/request.ts';

export const includedTypes = (widget: WidgetSpec) => contentSet(typeof widget.options?.included === 'string' ? widget.options.included : '').map(contentKey);

export function useStockpileCatalogue(runtime: WidgetRuntime, widget: WidgetSpec, track: BoundTrack | undefined, holders: readonly StockpileHolder[], included: readonly string[], active: boolean) {
  const client = runtime.client;
  const gate = useCampaignGate(runtime, widget, active);
  const bounded = useMemo(() => holders.map(holder => ({ country: holder.country,
    ...(holder.within ? { within: holder.within.flatMap(span => gate.span(span.firstHour, span.lastHour) ?? []) } : {}) })), [holders, gate.span]);
  const request = track && holders.length && gate.open ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'equipment-stockpile-catalogue',
    params: { branchId: track.branch.id, holders: bounded, included } }) : null;
  const tags = JSON.stringify(holders.map(holder => holder.country));
  return useProduct<StockpileCatalogueReply>(client, request, active, value => {
    const reply = validateStockpileCatalogue(value);
    if (!track || reply.branchId !== track.branch.id || reply.campaignKey !== track.source.metadata.campaignKey || JSON.stringify(reply.countries) !== tags)
      throw new Error('The stockpile catalogue belongs to another country, campaign or branch.');
    return reply;
  }, track?.source);
}

export function useStockpileAt(runtime: WidgetRuntime, widget: WidgetSpec, tag: string | null, active: boolean, time: RegionAction = SPEC) {
  return useHeadProduct(runtime, useGate(runtime, widget, time), 'equipment-stockpile-at', tag ? { country: countryFromKey(tag) } : null,
    { active, validate: validateStockpileAt, structure: reply => JSON.stringify(reply) });
}

function useView(catalogue: StockpileCatalogueReply | undefined, at: ReturnType<typeof useStockpileAt>, track: BoundTrack | undefined, tag?: string) {
  const values = useSyncExternalStore(at.live.subscribe, at.live.read, at.live.read);
  const hour = at.read === 'followed' ? at.shownAt() ?? at.at : at.hour;
  const view = useMemo<PreparedStockpileReply | undefined>(() => catalogue && track && hour !== null && at.data
    ? stockpileView(catalogue, atReply(at.data, values, hour, track.branch.id), hour, tag) : undefined,
  [catalogue, at.data, values, hour, track, tag]);
  return { view, hour };
}

export function useEquipmentStockpile(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, within?: readonly Span[] | null) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryKey(track) ?? undefined;
  const includedText = typeof widget.options?.included === 'string' ? widget.options.included : '';
  const included = useMemo(() => includedTypes(widget), [includedText]);
  const withinKey = JSON.stringify(within ?? null);
  const holders = useMemo<StockpileHolder[]>(() => tag ? [{ country: countryFromKey(tag), ...(within ? { within: within.map(span => ({ firstHour: span.firstHour, lastHour: span.lastHour })) } : {}) }] : [],
    [tag, withinKey]);
  const catalogue = useStockpileCatalogue(runtime, widget, track, holders, included, active);
  const at = useStockpileAt(runtime, widget, tag ?? null, active);
  const { view, hour } = useView(catalogue.data, at, track);
  return { track, tag, holders, included, catalogue: catalogue.data, data: view, hour, at, error: catalogue.error ?? at.error,
    retry: () => { if (catalogue.error) catalogue.retry(); if (at.error) at.retry(); } };
}

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

export function useStockpileCountry(selection: DatedCountrySelection | null, runtime: WidgetRuntime, track: BoundTrack | undefined, widget: WidgetSpec,
  active: boolean, time: RegionAction, included: readonly string[]) {
  const groups = useCountryGroups(runtime, track, active && !!selection);
  const identities = useMemo(() => new NumericReply<CountryGroupReply>(), [runtime.client]);
  const history = identities.update(JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id]), groups.data, groupNumbers);
  const spans = useMemo(() => selection && history && track && history.branchId === track.branch.id
    && selection.campaign === track.source.metadata.campaignKey && selection.branch === track.branch.id
    ? datedCountrySpans(history.countryIdentities, selection, track.branch.firstHour, track.branch.lastHour) : null,
  [selection, history, track?.source.metadata.campaignKey, track?.branch.id, track?.branch.firstHour, track?.branch.lastHour]);
  const holders = useMemo(() => spans ? stockpileHolders(spans) : [], [spans]);
  const live = active && !!selection;
  const catalogue = useStockpileCatalogue(runtime, widget, track, holders, included, live);
  const region = useRegion(widget, time);
  const hourAt = useRegionHourAt(runtime, region);
  const tag = useAtHead(runtime.clock, live && regionFollows(region), clockHour => { const at = hourAt(axisAt(clockHour)); return at === null ? null : holderAt(holders, at); });
  const at = useStockpileAt(runtime, widget, tag, live, time);
  const { view, hour: shownHour } = useView(catalogue.data, at, track, selection?.tag);
  const error = catalogue.error ?? at.error;
  const failure = (history ? undefined : groups.error) ?? error;
  const reason = (failure ? failureText(failure, 'Couldn’t load country.') : null) ?? (!history ? 'Loading country…'
    : !spans ? 'Choose a country again.' : !view ? 'Loading the selected stockpile history…' : null);
  return { data: selection ? view : undefined, hour: shownHour, at, catalogue: catalogue.data, holders, spans, reason, error: (history ? undefined : groups.error) ?? error,
    retry: groups.error && !history ? groups.retry : () => { if (catalogue.error) catalogue.retry(); if (at.error) at.retry(); }, identityError: history ? groups.error : undefined, identityRetry: groups.retry };
}
