import type { WidgetProps, WidgetRenderer } from '../../dashboard/definitions';
import type { AgencyUpgradesSelection } from '../../dashboard/gameWindows';
import GameWindowTable, { type GameWindowRow } from '../../families/table/GameWindowTable';
import { rowSource } from '../../families/table/rowSelection.ts';
import { agencyDeclaration, type AgencyMeta } from './agencyProjection';

const UPGRADES: GameWindowRow<AgencyUpgradesSelection> = {
  kind: 'agencyUpgrades',
  select: (track, _key, meta, hour) => {
    const tag = (meta as AgencyMeta | undefined)?.tag;
    return tag ? { kind: 'agencyUpgrades', ...rowSource(track, hour), tag } : null;
  },
  label: 'Intelligence agency',
};

function AgencyComparisonWidget(props: WidgetProps) {
  return <GameWindowTable {...props} declaration={agencyDeclaration} window={UPGRADES} />;
}

export default { render: AgencyComparisonWidget } satisfies WidgetRenderer;
