import { useCallback, useMemo, useState } from 'react';
import type { WidgetProps } from '../../dashboard/definitions';
import { formatDateTime } from '../../dashboard/calendar';
import type { NavalBattlesReply, NavalBattlesRankingReply, NavalBattleDetailsReply } from '../../generated/dashboard';
import { useProduct } from '../../dashboard/products';
import BattleLogView from '../battles/BattleLogView';
import type { BattleViewAdapter } from '../battles/BattleLogView';
import type { BattleMeasure, BattleSideSummary } from '../battles/battleRows';
import { battleValueText } from '../battles/battleRows';
import { definition } from './definition';
import { navalBattleRows, rankedNavalBattleRows,
  validateNavalBattlesManifest, validateNavalBattlesReply, validateNavalBattlesRanking,
  navalBattleSize, navalBattleMeasureText, NAVAL_BATTLE_SIZE_LABELS } from './navalBattlesData.ts';
import type { NavalBattleRow } from './navalBattlesData';
import { navalDetailKey, navalFleetDescription, navalDetailTooltip, validateNavalBattleDetails } from './navalBattleDetails';

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
  validateRanking: validateNavalBattlesRanking, rows: navalBattleRows, rankedRows: rankedNavalBattleRows,
  labels: { place: 'Battle', date: 'Date', start: 'Started', end: 'Ended', winner: 'Winner',
    ongoing: 'Ongoing', unknownPlace: 'Unnamed province', unknownStart: 'Start not recorded' },
  measures: MEASURES, resultMeasure: MEASURES[0], compactSides: SIDES,
  formatLoss: (value, row) => `${battleValueText(value)}${value.value !== null && row.end === null ? ' so far' : ''}`,
};
export default function NavalBattlesWidget(props: WidgetProps) {
  const track = props.runtime.tracks.find(item => item.id === props.widget.trackIds[0]);
  const identity = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id]) : null;
  const [visible, setVisible] = useState<{ identity: string | null; rows: { id: string; hour: number }[] }>({ identity: null, rows: [] });
  const onVisibleRows = useCallback((rows: readonly NavalBattleRow[]) => {
    const next = rows.map(row => ({ id: row.id, hour: row.observedHour }));
    setVisible(previous => previous.identity === identity && JSON.stringify(previous.rows) === JSON.stringify(next)
      ? previous : { identity, rows: next });
  }, [identity]);
  const request = track && visible.identity === identity && visible.rows.length ? {
    sourceId: track.source.id, revision: track.source.revision, product: 'naval-battle-details',
    params: { branchId: track.branch.id, battles: visible.rows },
  } : null;
  const details = useProduct<NavalBattleDetailsReply>(props.runtime.client, request, props.active, validateNavalBattleDetails);
  const enriched = useMemo(() => {
    const rows = new Map(details.data?.rows.map(row => [navalDetailKey(row.id, row.hour), row]));
    const detail = (row: NavalBattleRow) => rows.get(navalDetailKey(row.id, row.observedHour));
    return { ...adapter, onVisibleRows, description: (row: NavalBattleRow) => navalFleetDescription(detail(row)),
      tooltipRows: (row: NavalBattleRow) => navalDetailTooltip(detail(row), details.error),
      diagnostics: [...(details.data?.warnings ?? []), ...(details.error ? [`Battle details: ${details.error.message}`] : [])],
    };
  }, [onVisibleRows, details.data, details.error]);
  return <BattleLogView {...props} adapter={enriched} />;
}
