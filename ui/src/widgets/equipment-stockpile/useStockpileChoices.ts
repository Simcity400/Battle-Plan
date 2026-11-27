import { countryFromKey } from '../../dashboard/countryKey.ts';
import { selectedCountryKeys } from '../../dashboard/countrySelection.ts';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import { unresolvedTypeName } from '../../dashboard/equipment.ts';
import type { WidgetSpec } from '../../dashboard/model';
import { setToken } from '../../dashboard/options.ts';
import { useProduct } from '../../dashboard/products.ts';
import { RefusedError } from '../../dashboard/queryError';
import { useCampaignGate } from '../../dashboard/regionGate.ts';
import { productRequest } from '../../dashboard/request.ts';
import type { StockpileTypesReply } from '../../generated/dashboard';
import { validateStockpileTypes } from './stockpileProjection.ts';

export function useStockpileChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = selectedCountryKeys(widget.countries, track)[0] ?? null;
  const end = useCampaignGate(runtime, widget).hour;
  const request = track && tag && end !== null
    ? productRequest({
      sourceId: track.source.id,
      revision: track.source.revision,
      product: 'equipment-stockpile-types',
      params: { branchId: track.branch.id, country: countryFromKey(tag), hour: end },
    })
    : null;
  const { data, error, retry } = useProduct<StockpileTypesReply>(runtime.client, request, true, validateStockpileTypes, track?.source);
  const choices = (data?.rows ?? []).map(row => ({ value: setToken(row.key), label: row.name ?? unresolvedTypeName(row.key) }));
  const failure = data?.rowsMissing ? new RefusedError(data.rowsMissing) : error;
  return { chosen: failure ? { choices, error: failure, retry } : choices };
}
