import type { WidgetProps, WidgetRenderer } from '../../dashboard/definitions';
import type { MioPolicySelection } from '../../dashboard/gameWindows';
import GameWindowTable, { type GameWindowRow } from '../../families/table/GameWindowTable';
import { organisationsDeclaration, type OrganisationMeta } from './organisationsProjection';

const POLICIES: GameWindowRow<MioPolicySelection> = {
  kind: 'mioPolicyChooser',
  select: (track, key, _meta, hour) => ({
    kind: 'mioPolicyChooser', sourceId: track.source.id, revision: track.source.revision, branchId: track.branch.id, hour, organisation: key,
  }),
  title: meta => (meta as OrganisationMeta | undefined)?.name ?? 'Policies',
};

function IndustrialOrganisationsWidget(props: WidgetProps) {
  return <GameWindowTable {...props} declaration={organisationsDeclaration} window={POLICIES} />;
}

export default { render: IndustrialOrganisationsWidget } satisfies WidgetRenderer;
