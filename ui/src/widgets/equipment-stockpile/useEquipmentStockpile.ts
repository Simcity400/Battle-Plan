import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { setToken } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import type { EquipmentStockpileReply } from '../../generated/dashboard';
import { chartChoices, CHOSEN_KEYS, stockGroups, typeName, validateStockpileReply } from './stockpileData';
import type { Grouping } from './stockpileData';

/** The widget's country and its equipment stockpile reply. */
export function useEquipmentStockpile(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'equipment-stockpile',
    params: { branchId: track.branch.id, tag } } : null;
  return { track, tag, ...useProduct<EquipmentStockpileReply>(runtime.client, request, active, validateStockpileReply) };
}

/** The settings' choices: the included types are the campaign's own types, named as
 * content names them; the lines over time are the groups of the grouping chosen that
 * can be drawn (`chartChoices`). Each is offered as a `setToken` token, so a key a
 * setting cannot store (such as a mod's `mod/rifles`) can be included and chosen alike. */
export function useStockpileChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data } = useEquipmentStockpile(widget, runtime, true);
  if (!data) return { included: [], chosen: [], chosenVariant: [], chosenDomain: [] };
  const hour = Infinity;
  const choices = (grouping: Grouping) => chartChoices(data, grouping);
  return {
    included: stockGroups(data, 'type', hour).map(group => ({ value: setToken(group.key), label: typeName(data, group.key) })),
    [CHOSEN_KEYS.type]: choices('type'),
    [CHOSEN_KEYS.variant]: choices('variant'),
    [CHOSEN_KEYS.domain]: choices('domain'),
  };
}
