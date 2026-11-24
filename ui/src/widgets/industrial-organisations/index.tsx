import type { FamilyViewProps, FamilyViewRenderer } from '../../dashboard/definitions';
import type { IndustrialOrganisationSelection } from '../../dashboard/gameWindows';
import GameWindowTable, { type GameWindowRow } from '../../families/table/GameWindowTable';
import { rowSource } from '../../families/table/rowSelection.ts';

const ORGANISATION: GameWindowRow<IndustrialOrganisationSelection> = {
  kind: 'industrialOrganisation',
  select: (track, key, _meta, hour) => ({
    kind: 'industrialOrganisation', ...rowSource(track, hour), organisation: key,
  }),
  label: 'Industrial organisation',
};

function IndustrialOrganisationsWidget(props: FamilyViewProps) {
  return <GameWindowTable {...props} window={ORGANISATION} />;
}

export default { render: IndustrialOrganisationsWidget } satisfies FamilyViewRenderer;
