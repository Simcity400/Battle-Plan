import type { WidgetRuntime } from '../../dashboard/definitions';
import { intelCoverageRequest } from '../../dashboard/widgetRequests';
import type { WidgetSpec } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { validateIntelCoverage } from './intelCoverageData';
import type { IntelCoverageReply } from './intelCoverageData';
import { OPTIONS } from './definition';

export const useIntelOptions = (widget: WidgetSpec) => resolveOptions(OPTIONS, widget.options);

/** The shown countries' networks over the branch: the most active (or the chosen countries),
 * looked up locally at every hour. */
export function useIntelCoverage(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = useIntelOptions(widget);
  const request = intelCoverageRequest(widget, track);
  const product = useProduct<IntelCoverageReply>(runtime.client, request, active, validateIntelCoverage, track?.source);
  return { track, options, ...product };
}
