import { useMemo } from 'react';
import GameWindowView from './GameWindowView';
import type {
  GameWindowProps,
  IndustrialOrganisationSelection,
  MioPolicySelection,
} from './gameWindows';

export default function IndustrialOrganisationWindow({
  selection,
  active,
  client,
  onDiagnostics,
  onShown,
  onClose,
}: GameWindowProps<IndustrialOrganisationSelection | MioPolicySelection>) {
  const subView = selection.kind === 'mioPolicyChooser'
    ? 'policy'
    : selection.subView;
  const request = useMemo(() => ({
    sourceId: selection.sourceId,
    revision: selection.revision,
    branchId: selection.branchId,
    hour: selection.hour,
    kind: selection.kind,
    target: { organisation: selection.organisation },
    ...(subView ? { subView } : {}),
  }), [selection.sourceId, selection.revision, selection.branchId, selection.hour, selection.kind, selection.organisation, subView]);

  return <GameWindowView
    client={client}
    request={request}
    active={active}
    label={subView === 'policy' ? 'organisation policy' : 'military industrial organisation'}
    onDiagnostics={onDiagnostics}
    onShown={onShown}
    onClose={onClose}
  />;
}
