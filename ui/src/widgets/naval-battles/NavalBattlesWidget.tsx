import type { WidgetProps } from '../../dashboard/definitions';
import { formatDateTime } from '../../dashboard/calendar';
import BattleLogView from '../battles/BattleLogView';
import type { BattleViewAdapter } from '../battles/BattleLogView';
import type { BattleMeasure, BattleSideSummary } from '../battles/battleRows';
import { battleValueText } from '../battles/battleRows';
import { definition } from './definition';
import { navalBattleChanges, navalBattleRow, navalBattleRows, navalBattleState, navalRunDetails, navalWindowPlaces, rankedNavalBattleRows,
  validateNavalBattlesManifest, validateNavalBattlesReply, validateNavalBattlesRanking,
  navalBattleSize, navalBattleMeasureText, NAVAL_BATTLE_SIZE_LABELS } from './navalBattlesData.ts';
import type { NavalBattleRow } from './navalBattlesData';
import { navalFleetDescription, navalDetailTooltip, validateNavalBattleDetails } from './navalBattleDetails';
import type { NavalBattleDetail, NavalBattlesRankingReply, NavalBattlesReply } from '../../generated/dashboard';

const MEASURES = NAVAL_BATTLE_SIZE_LABELS.map<BattleMeasure<NavalBattleRow>>(label => ({
  label, title: label, value: row => navalBattleSize(row, label), format: navalBattleMeasureText,
  hint: label === 'Peak ships' ? 'Most ships at once.'
    : label === 'Length' ? undefined : 'Convoys shown separately.',
  tooltipFormat: (value, row) => `${navalBattleMeasureText(value, row)}${label === 'Peak ships' && row.peakHour !== null ? ` at ${formatDateTime(row.peakHour)}` : ''}`,
}));
const SIDES: BattleSideSummary = { label: 'Sides', countLabel: 'Ships', separator: 'v', unknown: 'Not recorded',
  formatCount: (value, compact) => `${battleValueText(value)}${compact || value.value === null ? '' : value.value === 1 ? ' ship' : ' ships'}` };
const adapter: BattleViewAdapter<NavalBattleRow, NavalBattlesReply, NavalBattlesRankingReply> = {
  definition, product: 'naval-battles', validateManifest: validateNavalBattlesManifest, validateWindow: validateNavalBattlesReply,
  validateRanking: validateNavalBattlesRanking, rows: navalBattleRows, row: navalBattleRow, changes: navalBattleChanges, state: navalBattleState, windowPlaces: navalWindowPlaces,
  rankedRows: rankedNavalBattleRows,
  labels: { place: 'Battle', date: 'Date', start: 'Started', end: 'Ended', winner: 'Winner',
    ongoing: 'Ongoing', unknownPlace: 'Unnamed province', unrecordedPlace: 'Province not recorded', unknownStart: 'Start not recorded' },
  measures: MEASURES, resultMeasure: MEASURES[0], compactSides: SIDES,
  formatLoss: (value, row) => `${battleValueText(value)}${value.value !== null && row.end === null ? ' so far' : ''}`,
  details: {
    product: 'naval-battle-details', validate: validateNavalBattleDetails, held: navalRunDetails,
    ask: (row, following) => ({ id: row.id, hour: following ? row.runHour : row.observedHour }),
    description: (detail, read) => navalFleetDescription(detail as NavalBattleDetail | undefined, read),
    tooltipRows: (detail, error) => navalDetailTooltip(detail as NavalBattleDetail | undefined, error),
  },
};
export default function NavalBattlesWidget(props: WidgetProps) {
  return <BattleLogView {...props} adapter={adapter} />;
}
