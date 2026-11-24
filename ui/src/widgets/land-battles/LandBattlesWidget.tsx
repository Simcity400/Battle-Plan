import type { WidgetProps } from '../../dashboard/definitions';
import { formatDateTime } from '../../dashboard/calendar';
import BattleLogView from '../battles/BattleLogView';
import type { BattleViewAdapter } from '../battles/BattleLogView';
import { battleState, battleValueText } from '../battles/battleRows';
import type { BattleLabels, BattleMeasure, BattleSideSummary } from '../battles/battleRows';
import { definition } from './definition';
import LandBattleList from './LandBattleList';
import CombatLogDetails from './CombatLogDetails';
import { rankedLandBattleRows, landBattleChanges, landBattleRow, landBattleRows, landWindowPlaces, validateLandBattlesReply, validateLandBattlesManifest, validateLandBattlesRanking,
  casualtyRankingNote, landCasualtiesMissing, landCasualtyText, landDivisionText, landLossText } from './landBattlesData.ts';
import type { LandBattleRow, LandBattlesRanking, LandBattlesWindow } from './landBattlesData';

const LABELS: BattleLabels = { place: 'Battle', date: 'Date', start: 'Started', end: 'Ended', winner: 'Winner',
  ongoing: 'Ongoing', unknownPlace: 'Unnamed province', unrecordedPlace: 'Province not recorded', unknownStart: 'Start not recorded' };
const SIDES: BattleSideSummary = { label: 'Sides', countLabel: 'Divisions (incl. reserves)', separator: 'v',
  unknown: 'Not in this recording', formatCount: landDivisionText };
const MEASURES: readonly BattleMeasure<LandBattleRow>[] = [
  { label: 'Casualties', title: 'Casualties', value: row => row.casualties,
    format: (value, row) => landCasualtyText(value, row), tooltipFormat: (value, row) => landCasualtyText(value, row, true) },
  { label: 'Peak size', title: 'Peak divisions', hint: 'Most divisions at once, including reserves.',
    value: row => row.peakUnknown ? { label: 'Peak size', value: null, unknown: 'Roster not recorded' }
      : { label: 'Peak size', value: row.peak.value, atLeast: row.start?.bound === 'by' || row.end !== null && 'unknown' in row.end },
    format: value => battleValueText(value), tooltipFormat: (value, row) => value.value === null ? value.unknown : `${battleValueText(value)} at ${formatDateTime(row.peak.hour)}` },
  { label: 'Length', title: 'Length', value: row => row.length, format: value => value.value === null ? value.unknown : `${battleValueText(value)} h` },
];

const rankedBy = (measure: string) => `Ranked by ${measure === 'Peak size' ? 'most divisions at once' : measure === 'Length' ? 'length' : 'casualties, both sides'}.`;

const adapter: BattleViewAdapter<LandBattleRow, LandBattlesWindow, LandBattlesRanking> = {
  definition, product: 'land-battles', manifestProduct: 'land-battles-manifest', rankingProduct: 'land-battles-ranking', landPlaceNames: true, validateManifest: validateLandBattlesManifest, validateWindow: validateLandBattlesReply,
  validateRanking: validateLandBattlesRanking, rows: landBattleRows, row: landBattleRow, changes: landBattleChanges, windowPlaces: landWindowPlaces, rankedRows: rankedLandBattleRows,
  state: row => battleState(row, true),
  labels: LABELS, measures: MEASURES, compactSides: SIDES, formatLoss: landLossText, view: LandBattleList,
  summary: (rows, ranking, measure, view) => {
    const casualtyNote = view === 'biggest' && measure === 'Casualties' ? casualtyRankingNote(ranking)
      : landCasualtiesMissing(rows) ? 'Casualties: Not in this recording.' : null;
    return casualtyNote ?? (view === 'biggest' ? rankedBy(ranking?.effectiveMeasure ?? measure) : '');
  },
};
export default function LandBattlesWidget(props: WidgetProps) {
  return <><BattleLogView {...props} adapter={adapter} /><CombatLogDetails {...props} view="battles" /></>;
}
