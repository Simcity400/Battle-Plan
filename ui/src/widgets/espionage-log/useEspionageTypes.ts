import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import { operationChoices, validateTypes, validateTypeTokens } from './espionageData';
import type { EspionageTypesReply, EspionageTypeTokensReply } from '../../generated/dashboard';
import { productRequest } from '../../dashboard/request.ts';

export function useEspionageTypes(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  return useProduct<EspionageTypesReply>(runtime.client, track ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'espionage-types',
    params: { branchId: track.branch.id } }) : null, active, validateTypes, track?.source);
}

export function useEspionageTypeTokens(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  return useProduct<EspionageTypeTokensReply>(runtime.client, track ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'espionage-type-tokens',
    params: { branchId: track.branch.id } }) : null, active, validateTypeTokens, track?.source);
}

export function useEspionageChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data } = useEspionageTypes(widget, runtime, true);
  const tokens = useEspionageTypeTokens(widget, runtime, true);
  return { types: operationChoices(data, tokens.data) };
}
