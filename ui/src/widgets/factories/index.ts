import type { OptionChoices, WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import { PartsCountryChip } from '../../dashboard/CountryChip';
import type { WidgetSpec } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import FactoriesWidget from './FactoriesWidget';
import { typeChoices, validateFactoryGroups } from './factoriesData';
import { productRequest } from '../../dashboard/request.ts';

function useFactoryTypes(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'factory-groups', params: { branchId: track.branch.id } }) : null;
  const { data } = useProduct(runtime.client, request, true, validateFactoryGroups, track?.source);
  return { types: typeChoices(data) };
}

export default { render: FactoriesWidget, countryChip: PartsCountryChip, useOptionChoices: useFactoryTypes } satisfies WidgetRenderer;
