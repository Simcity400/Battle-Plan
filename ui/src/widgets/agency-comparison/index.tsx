import type { WidgetProps, WidgetRenderer } from '../../dashboard/definitions';
import type { IntelligenceAgencySelection } from '../../dashboard/gameWindows';
import GameWindowTable, { type GameWindowRow } from '../../families/table/GameWindowTable';
import { agencyDeclaration } from './agencyProjection';

const AGENCY: GameWindowRow<IntelligenceAgencySelection> = {
  kind: 'intelligenceAgency',
  select: (track, key, _meta, hour) => {
    const country = Number(key);
    return Number.isSafeInteger(country) ? { kind: 'intelligenceAgency', sourceId: track.source.id, revision: track.source.revision, branchId: track.branch.id, hour, country } : null;
  },
  label: 'Intelligence agency',
};

function AgencyComparisonWidget(props: WidgetProps) {
  return <GameWindowTable {...props} declaration={agencyDeclaration} window={AGENCY} />;
}

export default { render: AgencyComparisonWidget } satisfies WidgetRenderer;
