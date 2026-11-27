import { contentTitle } from '../../dashboard/contentTitles.ts';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { resolveOptions, setToken } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { RefusedError } from '../../dashboard/queryError';
import { productRequest } from '../../dashboard/request.ts';
import type { ProducedTypesReply } from '../../generated/dashboard';
import { OPTIONS } from './definition';

function validateTypesReply(data: unknown): ProducedTypesReply {
  const reply = data as ProducedTypesReply;
  const ok = typeof reply === 'object' && reply != null && typeof reply.branchId === 'string' && Array.isArray(reply.types)
    && reply.types.every(type => typeof type.key === 'string' && type.key.length > 0 && (type.name === null || typeof type.name === 'string'))
    && new Set(reply.types.map(type => type.key)).size === reply.types.length
    && (reply.mostDelivered === null || reply.types.some(type => type.key === reply.mostDelivered))
    && (reply.typesMissing === null || typeof reply.typesMissing === 'string')
    && Array.isArray(reply.warnings);
  if (!ok) throw new Error('The equipment types reply is malformed.');
  return reply;
}

export function useProducedTypeChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const refits = resolveOptions(OPTIONS, widget.options).refits === true;
  const request = track ? productRequest({
    sourceId: track.source.id,
    revision: track.source.revision,
    product: 'equipment-produced-types',
    params: { branchId: track.branch.id, refits },
  }) : null;
  const types = useProduct<ProducedTypesReply>(runtime.client, request, true, validateTypesReply, track?.source);
  const choices = (types.data?.types ?? []).map(type => ({
    value: setToken(type.key),
    label: contentTitle(type.name, type.key, 'Equipment type'),
  }));
  const error = types.data?.typesMissing ? new RefusedError(types.data.typesMissing) : types.error;
  return { included: error ? { choices, error, retry: types.retry } : choices };
}
