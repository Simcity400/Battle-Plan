import type { OptionChoices, WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { selectedTags } from '../../dashboard/countrySelection';
import { useProduct } from '../../dashboard/products';
import ResourceBalanceWidget from './ResourceBalanceWidget';
import { RESOURCE_BALANCE, resourceName, validateResourceBalance } from './resourceData';

/** The recording's resources at the branch's last hour, one choice per token. */
function useResourceChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = selectedTags(widget.countries, track)[0];
  const { data } = useProduct(runtime.client, track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: RESOURCE_BALANCE,
    params: { branchId: track.branch.id, tag, hour: track.branch.lastHour } } : null, true, validateResourceBalance, track?.source);
  return { resources: (data?.resources ?? []).filter(item => item.ordinal === 1).map(item => ({ value: item.token, label: resourceName(item), icon: item.icon })) };
}

export default { render: ResourceBalanceWidget, useOptionChoices: useResourceChoices } satisfies WidgetRenderer;
