import { useMemo } from 'react';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { DashboardClient } from '../../dashboard/client';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { contentKey, contentSet, setToken } from '../../dashboard/options';
import { useFollowedProduct, useProduct } from '../../dashboard/products';
import type { EquipmentStockpileReply, StockpileCatalogueReply, StockpileHolder } from '../../generated/dashboard';
import { chartChoices, choiceKey, CHOSEN_KEYS, stockGroups, typeName, validateStockpileCatalogue, validateStockpileReply } from './stockpileData';
import type { Grouping } from './stockpileData';
import { pinnedGrouping } from './stockpileDrill';
import { holderAt, stockpileHolders, stockpileView } from './stockpileCatalogue';
import { datedCountrySpans } from '../../dashboard/datedCountry';
import type { DatedCountrySelection } from '../../dashboard/datedCountry';
import { groupNumbers } from '../../dashboard/groupsReply';
import { NumericReply } from '../../dashboard/numericReply';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { resolveWindow } from '../../dashboard/model';
import type { BoundTrack } from '../../dashboard/model';
import { usePlaybackHour } from '../../dashboard/playback';
import type { Span } from '../../charts/series';
import type { CountryGroupReply } from '../../generated/dashboard';

/** A reply's numbers alone: names come from content and reload with it, designs are identity. */
const nameless = <T extends { warnings: string[]; types: { name: string | null }[]; variants: { names: { name: string | null }[] }[] }>(reply: T): T =>
  ({ ...reply, warnings: [], types: reply.types.map(type => ({ ...type, name: null })),
    variants: reply.variants.map(variant => ({ ...variant, names: variant.names.map(name => ({ ...name, name: null })) })) });

/** The catalogue and hour's values a request was last answered with, shown by their numbers while
 * content-dependent names reload. */
function useHeld(client: DashboardClient, keys: readonly [string, string], catalogue: StockpileCatalogueReply | undefined, at: EquipmentStockpileReply | undefined) {
  const catalogues = useMemo(() => new NumericReply<StockpileCatalogueReply>(), [client]);
  const hours = useMemo(() => new NumericReply<EquipmentStockpileReply>(), [client]);
  const heldCatalogue = catalogues.update(keys[0], catalogue, nameless);
  const heldAt = hours.update(keys[1], at, nameless);
  return [catalogue ?? heldCatalogue, at ?? heldAt] as const;
}

/** The included types a widget saves, as content keys. */
export const includedTypes = (widget: WidgetSpec) => contentSet(typeof widget.options?.included === 'string' ? widget.options.included : '').map(contentKey);

/** The holders' catalogue (`equipment-stockpile-catalogue`): their variants, types, reads and peaks. */
export function useStockpileCatalogue(client: DashboardClient, track: BoundTrack | undefined, holders: readonly StockpileHolder[], included: readonly string[], active: boolean) {
  const request = track && holders.length ? { sourceId: track.source.id, revision: track.source.revision, product: 'equipment-stockpile-catalogue',
    params: { branchId: track.branch.id, holders, included } } : null;
  const tags = JSON.stringify(holders.map(holder => holder.tag));
  return useProduct<StockpileCatalogueReply>(client, request, active, value => {
    const reply = validateStockpileCatalogue(value);
    if (!track || reply.branchId !== track.branch.id || reply.campaignKey !== track.source.metadata.campaignKey || JSON.stringify(reply.tags) !== tags)
      throw new Error('The stockpile catalogue belongs to another country, campaign or branch.');
    return reply;
  }, track?.source);
}

/** One holder's stockpile at an hour (`equipment-stockpile-at`), the last hour's held while the next loads. */
export function useStockpileAt(client: DashboardClient, track: BoundTrack | undefined, tag: string | null, hour: number, active: boolean) {
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'equipment-stockpile-at',
    params: { branchId: track.branch.id, tag, hour } } : null;
  const product = useFollowedProduct<EquipmentStockpileReply>(client, request, ['hour'], active, value => {
    const reply = validateStockpileReply(value);
    if (!track || reply.branchId !== track.branch.id || reply.tag !== tag || reply.campaignKey !== track.source.metadata.campaignKey)
      throw new Error('The stockpile reply belongs to another country, campaign or branch.');
    return reply;
  }, track?.source);
  // The hour the shown reply answers, which a followed read holds until the next lands.
  const shown = product.moment === undefined ? hour : (JSON.parse(product.moment) as [number])[0];
  return { ...product, hour: shown };
}

/** The widget's country and its equipment stockpile at the hour it shows, over the catalogue. */
export function useEquipmentStockpile(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, within?: readonly Span[] | null) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  const clock = usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const window = track && resolveWindow(widget, track, runtime.tracks, clock);
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const includedText = typeof widget.options?.included === 'string' ? widget.options.included : '';
  const included = useMemo(() => includedTypes(widget), [includedText]); // eslint-disable-line react-hooks/exhaustive-deps
  const withinKey = JSON.stringify(within ?? null);
  const holders = useMemo<StockpileHolder[]>(() => tag ? [{ tag, ...(within ? { within: within.map(span => ({ firstHour: span.firstHour, lastHour: span.lastHour })) } : {}) }] : [],
    [tag, withinKey]); // eslint-disable-line react-hooks/exhaustive-deps
  const catalogue = useStockpileCatalogue(runtime.client, track, holders, included, active);
  const at = useStockpileAt(runtime.client, track, tag ?? null, hour, active);
  const scope = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id]);
  const [shownCatalogue, shownAt] = useHeld(runtime.client, [JSON.stringify([scope, holders, included]), JSON.stringify([scope, tag, at.hour])], catalogue.data, at.data);
  const data = useMemo(() => shownCatalogue ? stockpileView(shownCatalogue, shownAt, at.hour) : undefined, [shownCatalogue, shownAt, at.hour]);
  return { track, tag, holders, included, catalogue: shownCatalogue, data, error: catalogue.error ?? at.error,
    retry: () => { catalogue.retry(); at.retry(); } };
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

/** A dated country's stockpile at an hour: its catalogue over every tag it held, each within the hours
 * it held it, and the hour's values from the tag it held then. */
export function useStockpileCountry(selection: DatedCountrySelection | null, runtime: WidgetRuntime, track: BoundTrack | undefined, active: boolean, hour: number,
  included: readonly string[]) {
  const groups = useCountryGroups(runtime, track, active && !!selection);
  const identities = useMemo(() => new NumericReply<CountryGroupReply>(), [runtime.client]);
  const history = identities.update(JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id]), groups.data, groupNumbers);
  const spans = useMemo(() => selection && history && track && history.branchId === track.branch.id
    && selection.campaign === track.source.metadata.campaignKey && selection.branch === track.branch.id
    ? datedCountrySpans(history.countryIdentities, selection, track.branch.firstHour, track.branch.lastHour) : null,
  [selection, history, track?.source.metadata.campaignKey, track?.branch.id, track?.branch.firstHour, track?.branch.lastHour]);
  const holders = useMemo(() => spans ? stockpileHolders(spans) : [], [spans]);
  const live = active && !!selection;
  const catalogue = useStockpileCatalogue(runtime.client, track, holders, included, live);
  const tag = holderAt(holders, hour);
  const at = useStockpileAt(runtime.client, track, tag, hour, live);
  const scope = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id]);
  const [shownCatalogue, shownAt] = useHeld(runtime.client, [JSON.stringify([scope, holders, included]), JSON.stringify([scope, tag, at.hour])], catalogue.data, at.data);
  const data = useMemo(() => selection && shownCatalogue ? stockpileView(shownCatalogue, shownAt, at.hour, selection.tag) : undefined,
    [selection, shownCatalogue, shownAt, at.hour]);
  const error = catalogue.error ?? at.error;
  const reason = ((groups.error && !history) || error ? 'Couldn’t load country.' : null) ?? (!history ? 'Loading country…'
    : !spans ? 'Choose a country again.' : !data ? 'Loading the selected stockpile history…' : null);
  return { data, catalogue: shownCatalogue, holders, spans, reason, error: (history ? undefined : groups.error) ?? error,
    retry: groups.error && !history ? groups.retry : () => { catalogue.retry(); at.retry(); }, identityError: history ? groups.error : undefined, identityRetry: groups.retry };
}
