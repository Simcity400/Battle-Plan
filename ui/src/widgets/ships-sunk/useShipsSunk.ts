import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { setToken } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import type { ShipsSunkBattleTitlesReply, ShipsSunkReply } from '../../generated/dashboard';
import { shipClassChoices, validateShipBattleTitlesReply, validateShipsSunkReply } from './shipsSunkData';

export function useShipsSunk(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'ships-sunk', params: { branchId: track.branch.id } } : null;
  return { track, ...useProduct<ShipsSunkReply>(runtime.client, request, active, validateShipsSunkReply) };
}

export function useShipClassChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { track, data } = useShipsSunk(widget, runtime, true);
  return { classes: shipClassChoices(data, track?.source.metadata.campaignKey ?? '').map(choice => ({ ...choice, value: setToken(choice.value) })) };
}

export function useShipsSunkBattleTitles(runtime: WidgetRuntime, track: BoundTrack | undefined, rows: readonly number[], active: boolean) {
  const request = track && rows.length ? { sourceId: track.source.id, revision: track.source.revision, product: 'ships-sunk-battle-titles',
    params: { branchId: track.branch.id, rows } } : null;
  return useProduct<ShipsSunkBattleTitlesReply>(runtime.client, request, active, validateShipBattleTitlesReply);
}
