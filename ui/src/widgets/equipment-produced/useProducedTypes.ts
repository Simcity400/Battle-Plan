import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import type { EquipmentProducedReply, ProducedType } from '../../generated/dashboard';
import { validateProducedReply } from './producedData';

const NO_TYPES: readonly ProducedType[] = [];

/** Every type produced in the recording, named by its content: a request naming no
 * country answers with the types alone. Units leave `count` out, as a bridge from
 * before the IC count reads them too. */
export function useProducedTypeList(runtime: WidgetRuntime, track: BoundTrack | undefined, active: boolean): readonly ProducedType[] {
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'equipment-produced',
    params: { branchId: track.branch.id, tags: [], top: null, measure: 'total', grouping: 'type', included: [], refits: true } } : null;
  const { data } = useProduct<EquipmentProducedReply>(runtime.client, request, active, validateProducedReply);
  return data?.types ?? NO_TYPES;
}
