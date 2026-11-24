import { useMemo } from 'react';
import type { OptionChoice, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { setToken } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { productRequest } from '../../dashboard/request.ts';
import type { RegionalWeatherReply } from '../../generated/dashboard';

export function useWeatherModifiers(widget: WidgetSpec, runtime: WidgetRuntime): readonly OptionChoice[] {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const last = track?.branch.lastHour;
  const request = track && last !== undefined
    ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'regional-weather',
      params: { branchId: track.branch.id, firstHour: last, lastHour: last, width: 1, show: 'conditions', modifier: null, regions: [] } })
    : null;
  const { data } = useProduct<RegionalWeatherReply>(runtime.client, request, true, undefined, track?.source);
  return useMemo(() => (data?.modifiers ?? []).map(modifier => ({ value: setToken(modifier.id), label: modifier.name })), [data]);
}
