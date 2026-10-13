import type { OptionChoices, WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { setToken } from '../../dashboard/options';
import EquipmentProducedWidget from './EquipmentProducedWidget';
import { PartsCountryChip } from '../../dashboard/CountryChip';
import { typeLabel } from './producedData';
import { useProducedTypeList } from './useProducedTypes';

/** The types to include: every type produced in the recording, named by its content.
 * Keys of types content does not resolve (`?token`) are escaped so they can be stored too. */
function useProducedTypes(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const types = useProducedTypeList(runtime, track, true);
  return { included: types.map(type => ({ value: setToken(type.key), label: typeLabel(type) })) };
}

export default { render: EquipmentProducedWidget, countryChip: PartsCountryChip, useOptionChoices: useProducedTypes } satisfies WidgetRenderer;
