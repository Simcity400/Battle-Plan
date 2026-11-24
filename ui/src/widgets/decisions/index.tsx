import { useCallback, useRef, useState } from 'react';
import type { FamilyViewProps, FamilyViewRenderer } from '../../dashboard/definitions';
import type { InstalledAction } from '../../families/contract.ts';
import { FamilyWidget } from '../../families/host/registry.tsx';
import DecisionDetailWindow, { type DecisionDetailSubject } from './DecisionDetailWindow.tsx';

function DecisionsView(props: FamilyViewProps) {
  const anchor = useRef<HTMLDivElement>(null);
  const track = props.runtime.tracks.find(item => item.id === props.widget.trackIds[0]);
  const scope = track ? `${track.source.id}:${track.source.revision}:${track.branch.id}` : '';
  const [opened, setOpened] = useState<{ readonly scope: string; readonly subject: DecisionDetailSubject } | null>(null);
  const shown = opened && opened.scope === scope ? opened.subject : null;

  const onActivate = useCallback((action: InstalledAction) => {
    const meta = action.meta as DecisionDetailSubject | null | undefined;
    if (!meta || typeof meta !== 'object' || typeof meta.key !== 'string') return;
    setOpened({ scope, subject: { ...meta, key: action.subject || meta.key, asOfHour: action.hour } });
  }, [scope]);

  return (
    <div ref={anchor} className="fh-detail-anchor">
      {props.manifestDeclaration ? <FamilyWidget
        {...props}
        family={props.manifestDeclaration.family}
        declaration={props.manifestDeclaration}
        selectedKey={shown?.key ?? null}
        onActivate={onActivate}
      /> : null}
      <DecisionDetailWindow subject={shown} anchor={anchor} onClose={() => setOpened(null)} />
    </div>
  );
}

export default { render: DecisionsView } satisfies FamilyViewRenderer;
