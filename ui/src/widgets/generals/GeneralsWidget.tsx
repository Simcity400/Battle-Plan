import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useDrill } from '../../charts/drill/useDrill';
import { CLOSED } from '../../charts/drill/drillState';
import { formatDateTime } from '../../dashboard/calendar';
import type { WidgetProps } from '../../dashboard/definitions';
import { useRegion, useRegionHour } from '../../dashboard/widgetTime.ts';
import { DetailWindowHost } from '../../families/host/DetailWindowHost.tsx';
import DeclaredTableWidget from '../../families/table/DeclaredTableWidget.tsx';
import CareerView from '../general-career/CareerView';
import { pinnedCareer } from './generalsData';
import { generalsDeclaration } from './generalsProjection.ts';

export default function GeneralsWidget(props: WidgetProps) {
  const { widget, runtime, active } = props;
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const owner = useMemo(() => ({}), [runtime.client, track?.source.id, track?.source.revision, track?.branch.id]);
  const currentOwner = useRef(owner);
  currentOwner.current = owner;
  const [drillOwner, setDrillOwner] = useState<object | null>(null);
  const drillControl = useDrill(runtime, useRegion(widget));
  const savedDrill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const drill = drillOwner === owner ? savedDrill : CLOSED;
  const drillRegion = drillControl.region;
  const drillAt = useRegionHour(runtime, drillRegion, active && drill.breakdown != null);
  const anchor = useRef<HTMLDivElement>(null);

  useEffect(() => {
    dispatch({ type: 'close' });
    setDrillOwner(null);
  }, [owner, dispatch]);

  const onActivate = useCallback((reference: string, _meta?: unknown, hour?: number) => {
    if (currentOwner.current !== owner) return;
    setDrillOwner(owner);
    drillControl.show(reference, hour ?? track?.branch.lastHour ?? 0);
  }, [owner, drillControl, track?.branch.lastHour]);

  const pinAction = drill.breakdown && track && drillControl.time()
    ? <button
      type="button"
      className="button small quiet"
      onClick={() => {
        const time = drillControl.time();
        const history = time?.value.mode === 'pin' ? drillControl.gate.span(track.branch.firstHour, drillControl.hour ?? Number.NaN) : null;
        const kept = history ? drillControl.time(history) : time;
        if (currentOwner.current === owner && kept) runtime.addWidgetBelow(widget.id, pinnedCareer(widget, drill.breakdown!, track, kept));
      }}
    >
      Pin as widget
    </button>
    : null;

  return <div ref={anchor} className="generals-widget">
    <DeclaredTableWidget
      declaration={generalsDeclaration}
      {...props}
      onActivate={onActivate}
    />
    <DetailWindowHost
      open={drill.breakdown != null}
      subject={drill.breakdown}
      hour={drillAt}
      title="General career"
      day={drillAt === null ? '' : formatDateTime(drillAt)}
      anchor={anchor}
      headerActions={pinAction}
      onClose={() => { if (currentOwner.current === owner) dispatch({ type: 'close' }); }}
    >
      {drill.breakdown && <CareerView
        {...props}
        time={drillRegion.action}
        reference={drill.breakdown}
        onPick={h => { if (currentOwner.current === owner) dispatch({ type: 'pick', hour: h }); }}
      />}
    </DetailWindowHost>
  </div>;
}
