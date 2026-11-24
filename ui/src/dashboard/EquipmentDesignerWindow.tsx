import { useMemo } from 'react';
import GameWindowView from './GameWindowView';
import type { EquipmentDesignerSelection, GameWindowProps } from './gameWindows';

export default function EquipmentDesignerWindow({
  selection,
  generation,
  active,
  client,
  onDiagnostics,
  onShown,
  onClose,
}: GameWindowProps<EquipmentDesignerSelection>) {
  const request = useMemo(() => ({
    sourceId: selection.sourceId,
    revision: selection.revision,
    branchId: selection.branchId,
    hour: selection.hour,
    kind: 'equipmentDesigner',
    target: {
      tag: selection.tag,
      variant: selection.variant,
    },
  }), [selection.sourceId, selection.revision, selection.branchId, selection.hour, selection.tag, selection.variant]);

  return <GameWindowView
    client={client}
    request={request}
    generation={generation}
    active={active}
    label="equipment designer"
    onDiagnostics={onDiagnostics}
    onShown={onShown}
    onClose={onClose}
  />;
}
