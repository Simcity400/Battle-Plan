import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import { minefieldsRequest } from '../../dashboard/widgetRequests';
import type { WidgetSpec } from '../../dashboard/model';
import { resolveOptions, setToken } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { validateMinefields } from './minefieldsData';
import type { MinefieldsReply } from './minefieldsData';
import { OPTIONS } from './definition';

/** The owners' prepared mines over the branch (every owner, or the chosen countries),
 * looked up locally at every hour. */
export function useMinefields(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const request = minefieldsRequest(widget, track);
  const product = useProduct<MinefieldsReply>(runtime.client, request, active, validateMinefields, track?.source);
  return { track, options, ...product };
}

/** Sea regions as the content stack names them. */
export function useMinefieldChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data } = useMinefields(widget, runtime, true);
  return { regions: (data?.regions ?? []).map(region => ({ value: setToken(region.id), label: region.name })) };
}
