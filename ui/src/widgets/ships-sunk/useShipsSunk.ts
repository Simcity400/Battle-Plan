import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions.ts';
import type { WidgetSpec } from '../../dashboard/model.ts';
import { setToken } from '../../dashboard/options.ts';
import { useProduct } from '../../dashboard/products.ts';
import { drawableIcon } from '../../dashboard/artwork.ts';
import { manifestClassChoices, validateShipsSunkReply } from './shipsSunkData.ts';
import { isOrderedSpans } from '../../charts/series.ts';
import { int, object } from '../../dashboard/validation.ts';
import type { ShipsSunkManifestReply, ShipsSunkReply } from '../../generated/dashboard';
import { productRequest } from '../../dashboard/request.ts';

export interface ShipsSunkWindowReply { window: number; indices: number[]; reply: ShipsSunkReply }

const WINDOW_KEYS = 2 ** 32;

export function validateSinkingWindow(value: unknown): ShipsSunkWindowReply {
  const reply = validateShipsSunkReply(value);
  if (!int(reply.window) || reply.window < 0) throw new Error('The ships sunk window is malformed.');
  return { window: reply.window, indices: reply.rows.map((_, index) => reply.window * WINDOW_KEYS + index), reply };
}
export function validateSinkingManifest(value: unknown): ShipsSunkManifestReply {
  const text = (v: unknown) => v === null || typeof v === 'string';
  if (!object(value) || typeof value.branchId !== 'string' || !isOrderedSpans(value.windows) || typeof value.available !== 'boolean'
    || !(value.firstRead === null || int(value.firstRead))
    || !Array.isArray(value.classes) || !value.classes.every(type => object(type) && int(type.token) && text(type.key) && text(type.name) && text(type.icon))
    || !Array.isArray(value.recordedNames) || !value.recordedNames.every(name => typeof name === 'string')
    || !Array.isArray(value.warnings) || !value.warnings.every(warning => typeof warning === 'string')) throw new Error('The ships sunk manifest is malformed.');
  const manifest = value as unknown as ShipsSunkManifestReply;
  for (const type of manifest.classes) type.icon = drawableIcon(type.icon);
  return manifest;
}
export function useShipsSunk(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'ships-sunk-manifest', params: { branchId: track.branch.id } }) : null;
  return { track, ...useProduct<ShipsSunkManifestReply>(runtime.client, request, active, validateSinkingManifest, track?.source) };
}
export function useShipClassChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { track, data } = useShipsSunk(widget, runtime, true);
  return { classes: manifestClassChoices(data, track?.source.metadata.campaignKey ?? '').map(choice => ({ ...choice, value: setToken(choice.value) })) };
}
