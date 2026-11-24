import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import { armyMetadataRequest, measureChoices, validateArmySizeReply, type ArmySizeReply } from './armySizeData';

export function useArmyChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = armyMetadataRequest(track);
  const { data } = useProduct<ArmySizeReply>(runtime.client, request, true, validateArmySizeReply, track?.source);
  return { measure: measureChoices(data) };
}
