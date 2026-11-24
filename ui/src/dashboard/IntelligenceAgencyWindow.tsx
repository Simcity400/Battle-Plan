import { useMemo } from 'react';
import GameWindowView from './GameWindowView';
import type { GameWindowProps, IntelligenceAgencySelection } from './gameWindows';

export default function IntelligenceAgencyWindow({
  selection,
  active,
  client,
  onDiagnostics,
  onShown,
  onClose,
}: GameWindowProps<IntelligenceAgencySelection>) {
  const request = useMemo(() => ({
    sourceId: selection.sourceId,
    revision: selection.revision,
    branchId: selection.branchId,
    hour: selection.hour,
    kind: selection.kind,
    target: { country: selection.country },
  }), [selection.sourceId, selection.revision, selection.branchId, selection.hour, selection.kind, selection.country]);

  return <GameWindowView
    client={client}
    request={request}
    active={active}
    label="intelligence agency"
    onDiagnostics={onDiagnostics}
    onShown={onShown}
    onClose={onClose}
  />;
}
