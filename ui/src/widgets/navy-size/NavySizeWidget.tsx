import { useCallback, useRef, useState } from 'react';
import type { FamilyViewProps } from '../../dashboard/definitions';
import { useGate } from '../../dashboard/regionGate';
import { SPEC, type GatedHour } from '../../dashboard/regionTime';
import { DetailWindowHost } from '../../families/host/index.ts';
import { FamilyWidget } from '../../families/host/registry.tsx';
import type { InstalledAction } from '../../families/plan.ts';
import { parseChartInspect } from '../../families/time-chart/hitTest.ts';
import NavyManagement from './NavyManagement';

export default function NavySizeWidget(props: FamilyViewProps) {
  const { widget, runtime, active, view, manifestDeclaration } = props;
  const isLines = manifestDeclaration?.family === 'time-chart';
  const anchorRef = useRef<HTMLDivElement>(null);
  const [detail, setDetail] = useState<{ readonly tag: string; readonly key: string; readonly hour: GatedHour } | null>(null);
  const gate = useGate(runtime, widget, SPEC, active);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);

  const onActivate = useCallback((action: InstalledAction) => {
    if (isLines && parseChartInspect(action.subject) !== null) return;
    const gated = gate.recorded(Math.floor(action.hour));
    if (gated === null) return;
    const meta = action.meta as { readonly tag?: string; readonly key?: string } | null | undefined;
    setDetail({ tag: meta?.tag ?? action.subject, key: meta?.key ?? action.subject, hour: gated });
  }, [gate, isLines]);

  return (
    <div ref={anchorRef} className="fh-frame">
      {manifestDeclaration ? (
        <FamilyWidget
          {...props}
          family={manifestDeclaration.family}
          view={view}
          declaration={manifestDeclaration}
          {...(isLines ? {} : { selectedKey: detail?.key ?? null })}
          onActivate={onActivate}
        />
      ) : null}
      <DetailWindowHost
        open={detail !== null}
        subject={detail?.tag ?? null}
        hour={detail?.hour ?? null}
        title={detail ? `${detail.tag} navy` : 'Navy'}
        anchor={anchorRef}
        returnFocus=".fh-body"
        onClose={() => setDetail(null)}
      >
        {detail && track ? (
          <NavyManagement
            runtime={runtime}
            track={track}
            tag={detail.tag}
            hour={detail.hour}
            active={active}
          />
        ) : null}
      </DetailWindowHost>
    </div>
  );
}
