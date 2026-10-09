import type { OptionChoices, WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { setToken } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import type { EquipmentProducedReply } from '../../generated/dashboard';
import EquipmentProducedWidget from './EquipmentProducedWidget';
import { PartsCountryChip } from '../../dashboard/CountryChip';
import { typeLabel, validateProducedReply } from './producedData';

/** The types to include: every type produced in the recording, named by its content
 * (a request naming no country answers with the types alone). */
function useProducedTypes(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'equipment-produced',
    params: { branchId: track.branch.id, tags: [], top: null, measure: 'total', grouping: 'type', included: [], refits: true } } : null;
  const { data } = useProduct<EquipmentProducedReply>(runtime.client, request, true, validateProducedReply);
  // Keys of types content does not resolve (`?token`) are escaped so they can be stored too.
  return { included: (data?.types ?? []).map(type => ({ value: setToken(type.key), label: typeLabel(type) })) };
}

export default { render: EquipmentProducedWidget, countryChip: PartsCountryChip, useOptionChoices: useProducedTypes } satisfies WidgetRenderer;
