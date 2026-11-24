import type { WidgetProps, WidgetRenderer } from '../../dashboard/definitions';
import type { IndustrialOrganisationSelection } from '../../dashboard/gameWindows';
import GameWindowTable, { type GameWindowRow } from '../../families/table/GameWindowTable';
import { organisationsDeclaration } from './organisationsProjection';

const ORGANISATION: GameWindowRow<IndustrialOrganisationSelection> = {
  kind: 'industrialOrganisation',
  select: (track, key, _meta, hour) => ({
    kind: 'industrialOrganisation', sourceId: track.source.id, revision: track.source.revision, branchId: track.branch.id, hour, organisation: key,
  }),
  label: 'Industrial organisation',
};

function IndustrialOrganisationsWidget(props: WidgetProps) {
  return <GameWindowTable {...props} declaration={organisationsDeclaration} window={ORGANISATION} />;
}

export default { render: IndustrialOrganisationsWidget } satisfies WidgetRenderer;
