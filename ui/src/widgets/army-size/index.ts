import type { OptionChoices, WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import type { ArmySizeReply } from '../../generated/dashboard';
import ArmySizeWidget from './ArmySizeWidget';
import ArmyCountryChip from './ArmyCountryChip';
import { armyMetadataRequest, measureChoices, validateArmySizeReply } from './armySizeData';

/** An empty selection answers the recording's measure capabilities alone. */
function useArmyChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = armyMetadataRequest(track);
  const { data } = useProduct<ArmySizeReply>(runtime.client, request, true, validateArmySizeReply);
  return { measure: measureChoices(data) };
}

export default { render: ArmySizeWidget, useOptionChoices: useArmyChoices, countryChip: ArmyCountryChip } satisfies WidgetRenderer;
