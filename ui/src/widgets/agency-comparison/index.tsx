import type { FamilyViewProps, FamilyViewRenderer } from '../../dashboard/definitions';
import type { IntelligenceAgencySelection } from '../../dashboard/gameWindows';
import GameWindowTable, { type GameWindowRow } from '../../families/table/GameWindowTable';

const AGENCY: GameWindowRow<IntelligenceAgencySelection> = {
  kind: 'intelligenceAgency',
  select: (track, key, _meta, hour) => {
    const country = Number(key);
    return Number.isSafeInteger(country) ? { kind: 'intelligenceAgency', sourceId: track.source.id, revision: track.source.revision, branchId: track.branch.id, hour, country } : null;
  },
  label: 'Intelligence agency',
};

function AgencyComparisonWidget(props: FamilyViewProps) {
  return <GameWindowTable {...props} window={AGENCY} />;
}

export default { render: AgencyComparisonWidget } satisfies FamilyViewRenderer;
