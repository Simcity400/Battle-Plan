import { useCallback, useMemo, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import type { FamilyViewProps } from '../../dashboard/definitions';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import { useProduct } from '../../dashboard/products';
import { useGate } from '../../dashboard/regionGate';
import { at, SPEC } from '../../dashboard/regionTime';
import { productRequest } from '../../dashboard/request.ts';
import type { InstalledAction } from '../../families/contract.ts';
import { DetailWindowHost } from '../../families/host/index.ts';
import type { SpecialProjectsReply } from '../../generated/dashboard';
import ProjectDetails from './ProjectDetails';
import { ProjectDossier } from './ProjectWindow';
import { shownCard, unobservedReason, validateSpecialProjectsReply } from './specialProjectsData';
import { FamilyWidget } from '../../families/host/registry.tsx';
import { openedProject } from './specialProjectsProjection.ts';

const NO_WARNINGS = () => {};

export default function CurrentProjectsView(props: FamilyViewProps) {
  const { widget, runtime, active } = props;
  const hostRef = useRef<HTMLDivElement>(null);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = track ? selectedCountryKeys(widget.countries, track)[0] ?? null : null;
  const [opened, setOpened] = useState<{ readonly on: string; readonly tag: string; readonly id: string; readonly hour: number } | null>(null);
  const on = track && tag ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag]) : '';
  const shown = opened && opened.on === on ? opened : null;
  const onActivate = useCallback((action: InstalledAction) => {
    const project = typeof action.meta === 'string' ? openedProject(action.meta) : null;
    if (!on || !project) return;
    setOpened({ on, tag: project.country, id: project.id, hour: action.hour });
  }, [on, tag]);
  const action = useMemo(() => shown ? at(shown.hour) : SPEC, [shown?.hour]);
  const gate = useGate(runtime, widget, action, active && !!shown);
  const recorded = shown ? gate.recorded(shown.hour) : null;
  const request = useMemo(() => (shown && track && recorded !== null
    ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'special-projects',
      params: { branchId: track.branch.id, country: countryFromKey(shown.tag), hour: recorded } })
    : null), [shown, track, recorded]);
  const reply = useProduct<SpecialProjectsReply>(runtime.client, request, active && !!shown, validateSpecialProjectsReply, track?.source);
  const data = reply.data;
  const card = shown && data ? shownCard(data, shown.hour, shown.id) : null;
  return <div ref={hostRef} className="fh-detail-anchor">
    {props.manifestDeclaration ? <FamilyWidget {...props} family={props.manifestDeclaration.family} declaration={props.manifestDeclaration} onActivate={onActivate} /> : null}
    <DetailWindowHost
      open={shown !== null}
      subject={shown ? `${shown.tag}:${shown.id}` : null}
      hour={shown?.hour ?? null}
      title={card?.info.name ?? 'Special project'}
      anchor={hostRef}
      onClose={() => setOpened(null)}
    >
      {shown && (!data || recorded === null) && <WidgetState kind={reply.error ? 'error' : 'loading'} failure={reply.error} onRetry={reply.retry} label="Loading the project…" />}
      {shown && data && recorded !== null && <>
        <ProjectDossier runtime={runtime} widget={widget} tag={shown.tag} active={active} readHour={recorded} reply={data} card={card} hour={shown.hour}
          missing={card ? '' : unobservedReason(data, shown.hour) || 'This project is not in progress or completed at this time.'} />
        <ProjectDetails runtime={runtime} widget={widget} tag={shown.tag} active={active} readHour={recorded} onWarnings={NO_WARNINGS} />
      </>}
    </DetailWindowHost>
  </div>;
}
