import { useRef, useState } from 'react';
import { jumpItem } from '../../charts/jump.ts';
import type { FamilyViewProps, FamilyViewRenderer } from '../../dashboard/definitions';
import { FamilyWidget } from '../../families/host/registry.tsx';
import LandBattleWindow, { type OpenedBattle } from './LandBattleWindow.tsx';
import type { BattleDetailSubject } from './landBattlesProjection.ts';

function LandBattlesView(props: FamilyViewProps) {
  const anchor = useRef<HTMLDivElement>(null);
  const track = props.runtime.tracks.find(item => item.id === props.widget.trackIds[0]);
  const scope = track && JSON.stringify([track.source.id, track.source.revision, track.branch.id]);
  const [opened, setOpened] = useState<OpenedBattle | null>(null);
  const shown = opened && opened.scope === scope ? opened : null;
  return <div ref={anchor} className="fh-detail-anchor">
    {props.manifestDeclaration && <FamilyWidget {...props} family="table" declaration={props.manifestDeclaration}
      selectedKey={shown?.subject.row.key} onActivate={action => {
        if (scope && action.meta) setOpened({ scope, subject: action.meta as BattleDetailSubject, hour: action.hour });
      }} contextMenuItems={hit => hit.meta && track ? [jumpItem(props.runtime, track.id, (hit.meta as BattleDetailSubject).row.hour)] : []} />}
    <LandBattleWindow opened={shown} track={track} client={props.runtime.client} active={props.active} anchor={anchor} onClose={() => setOpened(null)} />
  </div>;
}

export default { render: LandBattlesView } satisfies FamilyViewRenderer;
