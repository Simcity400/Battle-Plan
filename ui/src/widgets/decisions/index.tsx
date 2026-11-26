import { useCallback, useMemo, useRef, useState } from 'react';
import type { FamilyViewProps, FamilyViewRenderer } from '../../dashboard/definitions';
import type { InstalledAction } from '../../families/contract.ts';
import { FamilyWidget } from '../../families/host/registry.tsx';
import DecisionDetailWindow, { type DecisionDetailSubject } from './DecisionDetailWindow.tsx';
import { useEventDetail, useTooltipText } from '../event-choices/eventDetail.ts';
import { withHoverText } from '../event-feed/hoverText.ts';

function DecisionsView(props: FamilyViewProps) {
  const anchor = useRef<HTMLDivElement>(null);
  const track = props.runtime.tracks.find(item => item.id === props.widget.trackIds[0]);
  const scope = track ? `${track.source.id}:${track.source.revision}:${track.branch.id}` : '';
  const [opened, setOpened] = useState<{ readonly scope: string; readonly subject: DecisionDetailSubject } | null>(null);
  const shown = opened && opened.scope === scope ? opened.subject : null;
  const detail = useEventDetail(props.runtime, props.widget.trackIds[0], 'decisions-tail', shown ? { key: shown.key, hour: shown.startedHour, asOfHour: shown.asOfHour } : null, shown !== null);
  const hover = useTooltipText(props.runtime, props.widget.trackIds[0], 'decisions-tail', props.widget.time.mode === 'follow', props.active);
  const options = useMemo(() => withHoverText(props.options, hover.texts), [props.options, hover.texts]);

  const onActivate = useCallback((action: InstalledAction) => {
    const meta = action.meta as DecisionDetailSubject | null | undefined;
    if (!meta || typeof meta !== 'object' || typeof meta.key !== 'string') return;
    setOpened({ scope, subject: { ...meta, key: action.subject || meta.key, asOfHour: meta.asOfHour ?? action.hour } });
  }, [scope]);

  return (
    <div ref={anchor} className="fh-detail-anchor">
      {props.manifestDeclaration ? <FamilyWidget
        {...props}
        options={options}
        onTooltipRow={hover.onTooltipRow}
        family={props.manifestDeclaration.family}
        declaration={props.manifestDeclaration}
        selectedKey={shown?.key ?? null}
        onActivate={onActivate}
      /> : null}
      <DecisionDetailWindow subject={shown} detail={detail} anchor={anchor} onClose={() => setOpened(null)} />
    </div>
  );
}

export default { render: DecisionsView } satisfies FamilyViewRenderer;
