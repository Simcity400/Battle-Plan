import type { MilitaryAirMetric, MilitaryAirReply } from '../generated/dashboard';
import type { OptionChoices, WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import { resolveWindow } from './model';
import { useProduct } from './products';
import { airCauseName, validateMilitaryAirReply } from './militaryAirLedger';

export function useMilitaryAirLedger(widget: WidgetSpec, runtime: WidgetRuntime, family: MilitaryAirReply['family'], active: boolean,
  query?: { regions?: readonly number[]; causes?: readonly number[]; metric: MilitaryAirMetric }) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const top = query && widget.countries.kind === 'group' && widget.countries.group.type === 'top' ? { count: widget.countries.group.count, metric: query.metric } : null;
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'military-air-ledger', params: { branchId: track.branch.id, family,
    regions: query?.regions ?? [], causes: query?.causes ?? [], fromHour: widget.time.mode === 'range' ? window?.firstHour ?? null : null, top } } : null;
  return { track, ...useProduct<MilitaryAirReply>(runtime.client, request, active, validateMilitaryAirReply, track?.source) };
}

export function useAirLossChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data } = useMilitaryAirLedger(widget, runtime, 'airLosses', true);
  return {
    regions: (data?.regions ?? []).map(region => ({ value: region.id, label: region.name ?? `Region ${region.id} (name not recorded)` })),
    causes: [...new Set(data?.leaves.map(leaf => leaf.cause) ?? [])].sort((a, b) => a - b).map(cause => ({ value: String(cause), label: airCauseName(cause) })),
  };
}
