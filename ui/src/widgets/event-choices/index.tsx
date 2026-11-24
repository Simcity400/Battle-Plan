import { useCallback, useRef, useState } from 'react';
import type { WidgetProps, WidgetRenderer } from '../../dashboard/definitions';
import type { InstalledAction } from '../../families/contract.ts';
import { FamilyWidget } from '../../families/host/registry.tsx';
import EventDetailWindow, { type EventDetailSubject } from './EventDetailWindow.tsx';
import { eventChoicesDeclaration } from './eventChoicesProjection.ts';

function EventChoicesView(props: WidgetProps) {
  const anchor = useRef<HTMLDivElement>(null);
  const track = props.runtime.tracks.find(item => item.id === props.widget.trackIds[0]);
  const scope = track ? `${track.source.id}:${track.source.revision}:${track.branch.id}` : '';
  const [opened, setOpened] = useState<{ readonly scope: string; readonly subject: EventDetailSubject } | null>(null);
  const shown = opened && opened.scope === scope ? opened.subject : null;

  const onActivate = useCallback((action: InstalledAction) => {
    const meta = action.meta as EventDetailSubject | null | undefined;
    if (!meta || typeof meta !== 'object' || typeof meta.key !== 'string') return;
    setOpened({ scope, subject: meta });
  }, [scope]);

  return (
    <div ref={anchor} className="fh-detail-anchor">
      <FamilyWidget
        {...props}
        family="table"
        declaration={eventChoicesDeclaration}
        selectedKey={shown?.key ?? null}
        onActivate={onActivate}
      />
      <EventDetailWindow subject={shown} anchor={anchor} onClose={() => setOpened(null)} />
    </div>
  );
}

export default { render: EventChoicesView } satisfies WidgetRenderer;
