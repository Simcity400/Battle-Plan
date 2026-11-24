import type { FamilyViewProps, FamilyViewRenderer } from '../../dashboard/definitions';
import type { EquipmentDesignerSelection } from '../../dashboard/gameWindows';
import GameWindowTable, { type GameWindowRow } from '../../families/table/GameWindowTable';
import { rowSource } from '../../families/table/rowSelection.ts';
import type { DesignMeta } from './designsProjection';

const DESIGNER: GameWindowRow<EquipmentDesignerSelection> = {
  kind: 'equipmentDesigner',
  select: (track, key, meta, hour) => {
    const tag = (meta as DesignMeta | undefined)?.tag;
    return tag ? { kind: 'equipmentDesigner', ...rowSource(track, hour), tag, variant: key } : null;
  },
  label: 'Equipment design',
};

function EquipmentDesignsWidget(props: FamilyViewProps) {
  return <GameWindowTable {...props} window={DESIGNER} />;
}

export default { render: EquipmentDesignsWidget } satisfies FamilyViewRenderer;
