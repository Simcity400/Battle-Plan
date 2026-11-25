import { useEffect, useMemo, useSyncExternalStore } from 'react';
import { jumpItem } from '../../charts/jump.ts';
import { setTooltipDetailRows, tooltipStore } from '../../charts/tooltip.ts';
import type { FamilyViewProps, FamilyViewRenderer } from '../../dashboard/definitions';
import { useProduct } from '../../dashboard/products.ts';
import { useGate } from '../../dashboard/regionGate.ts';
import { SPEC } from '../../dashboard/regionTime.ts';
import { productRequest } from '../../dashboard/request.ts';
import { FamilyWidget } from '../../families/host/registry.tsx';
import { navalDetailTooltip, validateNavalBattleDetails } from './navalBattleDetails.ts';

function BattleTooltip({ props, scrollId }: { props: FamilyViewProps; scrollId: string }) {
  const gate = useGate(props.runtime, props.widget, SPEC, props.active);
  const track = gate.track;
  const request = useMemo(() => {
    if (!track) return null;
    const [, id, shown] = JSON.parse(scrollId) as [string, string, number];
    const hour = gate.recorded(shown);
    if (hour === null) return null;
    return productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'naval-battle-details',
      params: { branchId: track.branch.id, battles: [{ id, hour }] } });
  }, [scrollId, track, gate]);
  const read = useProduct(props.runtime.client, request, props.active, validateNavalBattleDetails, track?.source);
  useEffect(() => {
    setTooltipDetailRows(scrollId, navalDetailTooltip(read.data?.rows[0], read.error));
  }, [scrollId, read.data, read.error]);
  return null;
}

function NavalBattlesView(props: FamilyViewProps) {
  const track = props.runtime.tracks.find(item => item.id === props.widget.trackIds[0]);
  const scope = track && JSON.stringify([props.widget.id, track.source.id, track.source.revision, track.branch.id]);
  const subject = () => {
    const id = tooltipStore.get()?.content.scrollId;
    return scope && id?.startsWith('[' + JSON.stringify(scope) + ',') ? id : null;
  };
  const scrollId = useSyncExternalStore(tooltipStore.subscribe, subject, subject);
  return props.manifestDeclaration ? <>
    {scrollId && <BattleTooltip props={props} scrollId={scrollId} />}
    <FamilyWidget {...props} family="table" declaration={props.manifestDeclaration}
    contextMenuItems={hit => track && typeof hit.meta === 'number' ? [jumpItem(props.runtime, track.id, hit.meta)] : []} />
  </> : null;
}

export default { render: NavalBattlesView } satisfies FamilyViewRenderer;
