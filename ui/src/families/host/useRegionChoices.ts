import { useMemo } from 'react';
import type { OptionChoice, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import { productRequest } from '../../dashboard/request.ts';
import type { StrategicRegionsReply } from '../../generated/dashboard';
import { regionIds, regionToken } from '../regionTokens.ts';

export function useRegionChoices(widget: WidgetSpec, runtime: WidgetRuntime, chosen: string, options?: { readonly sea?: boolean }): readonly OptionChoice[] {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track
    ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'strategic-regions', params: { branchId: track.branch.id } })
    : null;
  const { data } = useProduct<StrategicRegionsReply>(runtime.client, request, true, undefined, track?.source);
  const sea = options?.sea === true;
  return useMemo(() => {
    const listed = (data?.regions ?? [])
      .filter(region => !sea || region.sea)
      .map(region => ({ value: regionToken(region.id), label: region.name ?? `${region.nameMissing ?? 'Region name not recorded'} (${region.id})` }))
      .sort((a, b) => a.label.localeCompare(b.label));
    for (const id of regionIds(chosen)) {
      if (!listed.some(choice => choice.value === regionToken(id))) listed.push({ value: regionToken(id), label: `Region ${id} · not in this recording` });
    }
    return listed;
  }, [data, sea, chosen]);
}
