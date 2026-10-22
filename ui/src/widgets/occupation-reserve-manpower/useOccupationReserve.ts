import type { WidgetRuntime } from '../../dashboard/definitions';
import { occupationReserveRequest } from '../../dashboard/widgetRequests';
import type { WidgetSpec } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { validateOccupationReserve } from './occupationReserveData';
import type { OccupationReserveReply } from './occupationReserveData';
import { OPTIONS } from './definition';

/** Every holder's prepared reserve history over the branch (or the chosen countries'),
 * looked up locally at every hour. */
export function useOccupationReserve(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const request = occupationReserveRequest(widget, track);
  const product = useProduct<OccupationReserveReply>(runtime.client, request, active, validateOccupationReserve, track?.source);
  return { track, options, ...product };
}
