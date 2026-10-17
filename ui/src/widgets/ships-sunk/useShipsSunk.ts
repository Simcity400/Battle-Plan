import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model.ts';
import { setToken } from '../../dashboard/options.ts';
import { useProduct } from '../../dashboard/products.ts';
import type { ShipsSunkBattleTitlesReply, ShipsSunkReply } from '../../generated/dashboard.ts';
import { shipClassChoices, validateShipBattleTitlesReply, validateShipsSunkReply } from './shipsSunkData.ts';
import { isOrderedSpans } from '../../charts/series.ts';
import { isPreparationExtent, preparedPending } from '../../dashboard/preparedPrefix.ts';
import type { PreparedCoverage } from '../../shell/preparation/contract.ts';
import { int, object } from '../../dashboard/validation.ts';

interface SinkingPrefix { coverage?: PreparedCoverage; pending?: readonly { firstHour: number; lastHour: number }[] }
export interface ShipsSunkWindowReply extends SinkingPrefix { window: number; indices: number[]; reply: ShipsSunkReply }
export interface ShipsSunkManifestReply extends SinkingPrefix { windows: { firstHour: number; lastHour: number }[]; classes: ShipsSunkReply }
function sinkingPrefix(value: Record<string, unknown>): SinkingPrefix {
  const pending = preparedPending(value);
  if (value.coverage !== undefined && !isPreparationExtent(value.coverage)) throw new Error('The ships sunk coverage is malformed.');
  return value.coverage === undefined ? {} : { coverage: value.coverage as PreparedCoverage, pending };
}
export function validateSinkingWindow(value: unknown): ShipsSunkWindowReply {
  if (!object(value) || !int(value.window) || value.window < 0 || !Array.isArray(value.indices)
    || !value.indices.every(index => int(index) && index >= 0 && index <= 0xffffffff)
    || new Set(value.indices).size !== value.indices.length) throw new Error('The ships sunk window is malformed.');
  const reply = validateShipsSunkReply(value.reply);
  if (value.indices.length !== reply.rows.length) throw new Error('The ships sunk row indices are malformed.');
  return { window: value.window, indices: value.indices, reply, ...sinkingPrefix(value) };
}
export function validateSinkingManifest(value: unknown): ShipsSunkManifestReply {
  if (!object(value) || !isOrderedSpans(value.windows)) throw new Error('The ships sunk manifest is malformed.');
  return { windows: value.windows, classes: validateShipsSunkReply(value.classes), ...sinkingPrefix(value) };
}
export function useShipsSunk(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'ships-sunk-manifest', params: { branchId: track.branch.id } } : null;
  return { track, ...useProduct<ShipsSunkManifestReply>(runtime.client, request, active, validateSinkingManifest, track?.source) };
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
