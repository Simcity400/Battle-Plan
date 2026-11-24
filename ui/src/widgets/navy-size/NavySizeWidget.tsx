import { useCallback, useRef, useState } from 'react';
import type { WidgetProps } from '../../dashboard/definitions';
import { useGate } from '../../dashboard/regionGate';
import { SPEC, type GatedHour } from '../../dashboard/regionTime';
import { barsHostAdapter } from '../../families/bars/hostAdapter.ts';
import { DeclaredFamilyWidget, DetailWindowHost } from '../../families/host/index.ts';
import type { InstalledAction } from '../../families/plan.ts';
import { parseChartInspect } from '../../families/time-chart/hitTest.ts';
import { timeChartHostAdapter } from '../../families/time-chart/hostAdapter.ts';
import NavyManagement from './NavyManagement';
import { navyLinesDeclaration } from './navyLinesProjection.ts';
import { navySizeMixDeclaration } from './navySizeMixProjection.ts';
import { navySizeRaceDeclaration } from './navySizeRaceProjection.ts';

export default function NavySizeWidget(props: WidgetProps) {
  const { widget, runtime, active } = props;
  const rawView = widget.options?.view;
  const view = rawView === 'lines' ? 'lines' : rawView === 'mix' ? 'mix' : 'race';
  const isLines = view === 'lines';
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
      {isLines ? (
        <DeclaredFamilyWidget
          {...props}
          adapter={timeChartHostAdapter}
          view="lines"
          declaration={navyLinesDeclaration}
          onActivate={onActivate}
        />
      ) : view === 'mix' ? (
        <DeclaredFamilyWidget
          {...props}
          adapter={barsHostAdapter}
          view="mix"
          declaration={navySizeMixDeclaration}
          selectedKey={detail?.key ?? null}
          onActivate={onActivate}
        />
      ) : (
        <DeclaredFamilyWidget
          {...props}
          adapter={barsHostAdapter}
          view="race"
          declaration={navySizeRaceDeclaration}
          selectedKey={detail?.key ?? null}
          onActivate={onActivate}
        />
      )}
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
