import { useCallback, useEffect, useMemo, useRef } from 'react';
import { useDrill } from '../../charts/drill/useDrill';
import { Icon } from '../../components/Icons';
import { formatDateTime } from '../../dashboard/calendar';
import { subjectToken } from '../../dashboard/chosen.ts';
import type { FamilyViewProps, FamilyViewRenderer } from '../../dashboard/definitions';
import GateState from '../../dashboard/GateState';
import { TimeRegion, useRegion, useRegionHour } from '../../dashboard/widgetTime.ts';
import { DetailWindowHost } from '../../families/host/index.ts';
import { FamilyWidget } from '../../families/host/registry.tsx';
import type { InstalledAction } from '../../families/plan.ts';
import { DOSSIER_VIEW, dossierDeclaration } from '../general-career/dossierProjection.ts';
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

  const career = useMemo(() => {
    if (!track || reference === null) return null;
    const options = { leader: subjectToken(track.source.metadata.campaignKey, reference) };
    return { widget: { ...widget, id: `${widget.id}:career:${reference}`, kind: 'general-career' }, options };
  }, [widget, track, reference]);

  const shut = drill.gate.stage.stage !== 'open' && drill.gate.stage.stage !== 'unbound';
  const pinAction = drill.pinRefused
    ? <button type="button" className="icon-button small" aria-disabled="true" aria-label={`Can’t pin: ${drill.pinRefused}`} data-tooltip={drill.pinRefused}><Icon name="pin" /></button>
    : <button type="button" className="icon-button small" aria-label="Pin as widget" data-tooltip="Pin as widget" onClick={pin}><Icon name="pin" /></button>;
  const followAction = (
    <button
      type="button"
      className="button small quiet"
      aria-label="Follow playback"
      aria-pressed={drill.following}
      disabled={drill.following && !drill.pause}
      onClick={drill.following && drill.pause ? drill.pause : drill.follow}
    >
      Follow
    </button>
  );

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
        {followAction}
        {!shut && pinAction}
      </>}
      onClose={close}
    >
      <TimeRegion runtime={runtime} region={drill.region}>
        {shut ? <GateState gate={drill.gate} loading="Loading…" />
          : career && <div className="fh-frame" style={{ minHeight: 320 }}>
            <FamilyWidget
              {...props}
              widget={career.widget}
              options={career.options}
              family="card"
              view={DOSSIER_VIEW}
              declaration={dossierDeclaration}
            />
          </div>}
      </TimeRegion>
    </DetailWindowHost>
  </div>;
}

export default { render: GeneralsFamilyWidget } satisfies FamilyViewRenderer;
