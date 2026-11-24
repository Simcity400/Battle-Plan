import { useCallback, useMemo, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import { useProduct } from '../../dashboard/products';
import { useGate } from '../../dashboard/regionGate';
import { productRequest } from '../../dashboard/request.ts';
import { cardHostAdapter } from '../../families/card/hostAdapter.ts';
import type { InstalledAction } from '../../families/contract.ts';
import DeclaredFamilyWidget from '../../families/host/DeclaredFamilyWidget.tsx';
import { DetailWindowHost } from '../../families/host/index.ts';
import type { SpecialProjectsReply } from '../../generated/dashboard';
import ProjectDetails from './ProjectDetails';
import { ProjectDossier } from './ProjectWindow';
import { specialProjectsDeclaration } from './specialProjectsProjection.ts';
import { shownCard, unobservedReason, validateSpecialProjectsReply } from './specialProjectsData';

const NO_THROUGH = () => null;
const NO_WARNINGS = () => {};

export default function CurrentProjectsView(props: WidgetProps) {
  const { widget, runtime, active } = props;
  const hostRef = useRef<HTMLDivElement>(null);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = track ? selectedCountryKeys(widget.countries, track)[0] ?? null : null;
  const [opened, setOpened] = useState<{ readonly on: string; readonly id: string; readonly hour: number } | null>(null);
  const on = track && tag ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag]) : '';
  const shown = opened && opened.on === on ? opened : null;
  const onActivate = useCallback((action: InstalledAction) => {
    if (!on || typeof action.meta !== 'string' || !action.meta) return;
    setOpened({ on, id: action.meta, hour: action.hour });
  }, [on]);
  const gate = useGate(runtime, widget);
  const recorded = shown ? gate.recorded(shown.hour) : null;
  const request = useMemo(() => (shown && track && tag && recorded !== null
    ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'special-projects',
      params: { branchId: track.branch.id, country: countryFromKey(tag), hour: recorded } })
    : null), [shown, track, tag, recorded]);
  const reply = useProduct<SpecialProjectsReply>(runtime.client, request, active && !!shown, validateSpecialProjectsReply, track?.source);
  const data = reply.data;
  const card = shown && data ? shownCard(data, shown.hour, shown.id) : null;
  return <div ref={hostRef} className="fh-detail-anchor">
    <DeclaredFamilyWidget {...props} view="live" adapter={cardHostAdapter} declaration={specialProjectsDeclaration} onActivate={onActivate} />
    <DetailWindowHost
      open={Boolean(shown && tag)}
      subject={shown ? `${tag}:${shown.id}` : null}
      hour={shown?.hour ?? null}
      title={card?.info.name ?? 'Special project'}
      anchor={hostRef}
      onClose={() => setOpened(null)}
    >
      {shown && tag && !data && <WidgetState kind={reply.error ? 'error' : 'loading'} failure={reply.error} onRetry={reply.retry} label="Loading the project…" />}
      {shown && tag && data && <>
        <ProjectDossier runtime={runtime} widget={widget} tag={data.tag} active={active} reply={data} card={card} hour={shown.hour}
          through={NO_THROUGH} feed={null} missing={card ? '' : unobservedReason(data, shown.hour) || 'This project is not in progress or completed at this time.'} />
        <ProjectDetails runtime={runtime} widget={widget} tag={data.tag} active={active} onWarnings={NO_WARNINGS} />
      </>}
    </DetailWindowHost>
  </div>;
}
