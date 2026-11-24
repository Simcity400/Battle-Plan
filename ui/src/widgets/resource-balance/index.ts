import { useCampaignGate } from '../../dashboard/regionGate';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { OptionChoices, WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import { useProduct } from '../../dashboard/products';
import ResourceBalanceWidget from './ResourceBalanceWidget';
import { RESOURCE_BALANCE, resourceName, validateResourceBalance } from './resourceReply.ts';
import { productRequest } from '../../dashboard/request.ts';

function useResourceChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = selectedCountryKeys(widget.countries, track)[0];
  const end = useCampaignGate(runtime, widget).hour;
  const { data } = useProduct(runtime.client, track && tag && end !== null ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: RESOURCE_BALANCE,
    params: { branchId: track.branch.id, country: countryFromKey(tag), hour: end } }) : null, true, validateResourceBalance, track?.source);
  return { resources: (data?.resources ?? []).filter(item => item.ordinal === 1).map(item => ({ value: item.token, label: resourceName(item), icon: item.icon })) };
}

export default { render: ResourceBalanceWidget, useOptionChoices: useResourceChoices } satisfies WidgetRenderer;
