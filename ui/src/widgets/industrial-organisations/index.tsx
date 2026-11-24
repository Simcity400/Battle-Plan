import type { WidgetProps, WidgetRenderer } from '../../dashboard/definitions';
import type { MioPolicySelection } from '../../dashboard/gameWindows';
import GameWindowTable, { type GameWindowRow } from '../../families/table/GameWindowTable';
import { organisationsDeclaration } from './organisationsProjection';

const POLICIES: GameWindowRow<MioPolicySelection> = {
  kind: 'mioPolicyChooser',
  select: (track, key, _meta, hour) => ({
    kind: 'mioPolicyChooser', sourceId: track.source.id, revision: track.source.revision, branchId: track.branch.id, hour, organisation: key,
  }),
  label: 'Organisation policies',
};

function IndustrialOrganisationsWidget(props: WidgetProps) {
  return <GameWindowTable {...props} declaration={organisationsDeclaration} window={POLICIES} />;
}

export default { render: IndustrialOrganisationsWidget } satisfies WidgetRenderer;
