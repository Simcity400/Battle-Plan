import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { setToken } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import type { ShipsSunkBattleTitlesReply, ShipsSunkReply } from '../../generated/dashboard';
import { shipClassChoices, validateShipBattleTitlesReply, validateShipsSunkReply } from './shipsSunkData';
import { isOrderedSpans } from '../../charts/series';
import { int, object } from '../../dashboard/validation';

export interface ShipsSunkWindowReply { window: number; indices: number[]; reply: ShipsSunkReply }
export interface ShipsSunkManifestReply { windows: { firstHour: number; lastHour: number }[]; classes: ShipsSunkReply }
export function validateSinkingWindow(value: unknown): ShipsSunkWindowReply {
  if (!object(value) || !int(value.window) || value.window < 0 || !Array.isArray(value.indices)
    || !value.indices.every(index => int(index) && index >= 0 && index <= 0xffffffff)
    || new Set(value.indices).size !== value.indices.length) throw new Error('The ships sunk window is malformed.');
  const reply = validateShipsSunkReply(value.reply);
  if (value.indices.length !== reply.rows.length) throw new Error('The ships sunk row indices are malformed.');
  return { window: value.window, indices: value.indices, reply };
}
export function validateSinkingManifest(value: unknown): ShipsSunkManifestReply {
  if (!object(value) || !isOrderedSpans(value.windows)) throw new Error('The ships sunk manifest is malformed.');
  return { windows: value.windows, classes: validateShipsSunkReply(value.classes) };
}
export function useShipsSunk(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'ships-sunk-manifest', params: { branchId: track.branch.id } } : null;
  return { track, ...useProduct<ShipsSunkManifestReply>(runtime.client, request, active, validateSinkingManifest) };
}
export function useShipClassChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { track, data } = useShipsSunk(widget, runtime, true);
  return { classes: shipClassChoices(data?.classes, track?.source.metadata.campaignKey ?? '').map(choice => ({ ...choice, value: setToken(choice.value) })) };
}
export function useShipsSunkBattleTitles(runtime: WidgetRuntime, track: BoundTrack | undefined, rows: readonly number[], active: boolean) {
  const request = track && rows.length ? { sourceId: track.source.id, revision: track.source.revision, product: 'ships-sunk-battle-titles',
    params: { branchId: track.branch.id, rows } } : null;
  return useProduct<ShipsSunkBattleTitlesReply>(runtime.client, request, active, validateShipBattleTitlesReply);
}
