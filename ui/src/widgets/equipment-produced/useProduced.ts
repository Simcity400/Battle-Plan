import type { GatedHour } from '../../dashboard/regionTime';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { resolveOptions, setToken } from '../../dashboard/options';
import { OPTIONS } from './definition';
import { typeLabel } from './producedData';
import { useProduct } from '../../dashboard/products';
import type { Measure } from './producedData';
import type { ProducedAtReply, ProducedTypesReply } from '../../generated/dashboard';
import { productRequest } from '../../dashboard/request.ts';

const amount = (value: { value?: unknown; reason?: unknown } | null | undefined) => value != null
  && (value.value === null ? typeof value.reason === 'string' : Number.isSafeInteger(value.value));

export function validateTypesReply(data: unknown): ProducedTypesReply {
  const reply = data as ProducedTypesReply;
  const ok = typeof reply === 'object' && reply != null && typeof reply.branchId === 'string' && Array.isArray(reply.types)
    && reply.types.every(type => typeof type.key === 'string' && type.key.length > 0 && (type.name === null || typeof type.name === 'string'))
    && new Set(reply.types.map(type => type.key)).size === reply.types.length
    && (reply.mostDelivered === null || reply.types.some(type => type.key === reply.mostDelivered))
    && Array.isArray(reply.warnings);
  if (!ok) throw new Error('The equipment types reply is malformed.');
  return reply;
}

export function validateAtReply(data: unknown): ProducedAtReply {
  const reply = data as ProducedAtReply;
  const ok = typeof reply === 'object' && reply != null && typeof reply.tag === 'string' && Number.isInteger(reply.hour)
    && typeof reply.recorded === 'boolean' && amount(reply.icTotal)
    && Array.isArray(reply.models) && reply.models.every(model => typeof model.variant === 'string' && typeof model.typeKey === 'string'
      && typeof model.typeRecorded === 'boolean' && amount(model.units) && amount(model.ic))
    && (reply.observation === null || Number.isInteger(reply.observation)) && (reply.read === null || Number.isInteger(reply.read))
    && Array.isArray(reply.warnings);
  if (!ok) throw new Error('The equipment produced reply at an hour is malformed.');
  return reply;
}

export function useProducedTypes(runtime: WidgetRuntime, track: BoundTrack | undefined, refits: boolean, active: boolean) {
  const request = track ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'equipment-produced-types',
    params: { branchId: track.branch.id, refits } }) : null;
  return useProduct<ProducedTypesReply>(runtime.client, request, active, validateTypesReply, track?.source);
}

export function useProducedAt(runtime: WidgetRuntime, track: BoundTrack | undefined, tag: string | null, hour: GatedHour | null,
  measure: Measure, refits: boolean, active: boolean) {
  const request = track && tag && hour != null ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'equipment-produced-at',
    params: { branchId: track.branch.id, country: countryFromKey(tag), hour, measure, refits } }) : null;
  return useProduct<ProducedAtReply>(runtime.client, request, active, validateAtReply, track?.source);
}

export function useProducedTypeChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const types = useProducedTypes(runtime, track, resolveOptions(OPTIONS, widget.options).refits === true, true);
  return { included: (types.data?.types ?? []).map(type => ({ value: setToken(type.key), label: typeLabel(type) })) };
}
