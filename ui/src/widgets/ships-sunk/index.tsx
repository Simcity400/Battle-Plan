import type { FamilyViewProps, FamilyViewRenderer } from '../../dashboard/definitions';
import { jumpItem } from '../../charts/jump.ts';
import { FamilyWidget } from '../../families/host/registry.tsx';

function ShipsSunkView(props: FamilyViewProps) {
  const track = props.runtime.tracks.find(item => item.id === props.widget.trackIds[0]);
  return props.manifestDeclaration ? <FamilyWidget {...props} family="table" declaration={props.manifestDeclaration}
    contextMenuItems={hit => track && typeof hit.meta === 'number' ? [jumpItem(props.runtime, track.id, hit.meta)] : []}
  /> : null;
}

export default { render: ShipsSunkView } satisfies FamilyViewRenderer;
