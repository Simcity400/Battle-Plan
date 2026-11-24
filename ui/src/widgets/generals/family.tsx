import { useCallback, useEffect, useRef } from 'react';
import FollowControl from '../../charts/drill/FollowControl';
import { useDrill } from '../../charts/drill/useDrill';
import { Icon } from '../../components/Icons';
import { formatDateTime } from '../../dashboard/calendar';
import type { FamilyViewProps, FamilyViewRenderer } from '../../dashboard/definitions';
import GateState from '../../dashboard/GateState';
import { TimeRegion, useRegion, useRegionHour } from '../../dashboard/widgetTime.ts';
import { DetailWindowHost } from '../../families/host/index.ts';
import { FamilyWidget } from '../../families/host/registry.tsx';
import type { InstalledAction } from '../../families/plan.ts';
import CareerView from '../general-career/CareerView';
import { pinnedCareer } from './generalsData.ts';

function GeneralsFamilyWidget(props: FamilyViewProps) {
  const { widget, runtime, active, manifestDeclaration } = props;
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const anchor = useRef<HTMLDivElement>(null);
  const drill = useDrill(runtime, useRegion(widget));
  const reference = drill.state.breakdown;
  const drillAt = useRegionHour(runtime, drill.region, active && reference !== null);
  const { dispatch, close } = drill;
  useEffect(() => dispatch({ type: 'close' }), [track?.source.id, track?.source.revision, track?.branch.id, dispatch]);

  const onActivate = useCallback((action: InstalledAction) => drill.show(action.subject, Math.floor(action.hour)), [drill]);
  const pin = useCallback(() => {
    const time = drill.time();
    if (!track || reference === null || !time) return;
    const history = time.value.mode === 'pin' ? drill.gate.span(track.branch.firstHour, drill.hour ?? Number.NaN) : null;
    const kept = history ? drill.time(history) : time;
    if (kept) runtime.addWidgetBelow(widget.id, pinnedCareer(widget, props.options.growWithPlayback === true, reference, track, kept));
  }, [drill, track, reference, runtime, widget, props.options.growWithPlayback]);

  const shut = drill.gate.stage.stage !== 'open' && drill.gate.stage.stage !== 'unbound';
  const pinAction = drill.pinRefused
    ? <button type="button" className="icon-button small" aria-disabled="true" aria-label={`Can’t pin: ${drill.pinRefused}`} data-tooltip={drill.pinRefused}><Icon name="pin" /></button>
    : <button type="button" className="icon-button small" aria-label="Pin as widget" data-tooltip="Pin as widget" onClick={pin}><Icon name="pin" /></button>;

  return <div ref={anchor} className="fh-frame">
    {manifestDeclaration ? <FamilyWidget
      {...props}
      family="table"
      declaration={manifestDeclaration}
      selectedKey={reference}
      onActivate={onActivate}
    /> : null}
    <DetailWindowHost
      open={reference !== null}
      subject={reference}
      hour={drillAt}
      title="General career"
      day={shut || drillAt === null ? '' : formatDateTime(drillAt)}
      anchor={anchor}
      returnFocus=".fh-body"
      headerActions={<>
        <FollowControl following={drill.following} onFollow={drill.follow} onPause={drill.pause} />
        {!shut && pinAction}
      </>}
      onClose={close}
    >
      <TimeRegion runtime={runtime} region={drill.region}>
        {shut ? <GateState gate={drill.gate} loading="Loading…" />
          : reference !== null && <CareerView {...props} grow={props.options.growWithPlayback === true} time={drill.region.action} reference={reference} onPick={drill.pick} />}
      </TimeRegion>
    </DetailWindowHost>
  </div>;
}

export default { render: GeneralsFamilyWidget } satisfies FamilyViewRenderer;
