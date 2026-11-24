import { useEffect, useMemo, useState } from 'react';
import GameWindowView from './GameWindowView';
import type { GameWindowProps, GameWindowView as View, IndustrialOrganisationSelection } from './gameWindows';

const VIEWS: readonly View[] = ['traits', 'history', 'policy'];

export default function IndustrialOrganisationWindow({
  selection,
  generation,
  active,
  client,
  onDiagnostics,
  onShown,
  onClose,
}: GameWindowProps<IndustrialOrganisationSelection>) {
  const [subView, setSubView] = useState<View>(selection.subView ?? 'traits');
  useEffect(() => setSubView(selection.subView ?? 'traits'), [selection.organisation, selection.subView]);
  const request = useMemo(() => ({
    sourceId: selection.sourceId,
    revision: selection.revision,
    branchId: selection.branchId,
    hour: selection.hour,
    kind: selection.kind,
    target: { organisation: selection.organisation },
    subView,
  }), [selection.sourceId, selection.revision, selection.branchId, selection.hour, selection.kind, selection.organisation, subView]);

  return <GameWindowView
    client={client}
    request={request}
    generation={generation}
    active={active}
    label="military industrial organisation"
    onDiagnostics={onDiagnostics}
    onShown={onShown}
    onClose={onClose}
    onView={view => { if ((VIEWS as readonly string[]).includes(view)) setSubView(view as View); }}
  />;
}
