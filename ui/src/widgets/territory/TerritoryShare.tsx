import { useCallback, useMemo } from 'react';
import { dayLabel } from '../../charts/draw';
import { usNumber } from '../../charts/numberFormat';
import type { raceSeriesIndex } from '../../charts/race/series';
import Sunburst from '../../charts/sunburst/Sunburst';
import { sliceTooltip } from '../../charts/sunburst/layout';
import { shareTooltip } from '../../charts/sunburst/share';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import type { SunburstKey, SunburstNode, SunburstText } from '../../charts/sunburst/layout';
import type { FactionTotalsReply, TerritoryMeasure, TerritoryReply, TerritoryRule } from '../../generated/dashboard';
import { useChartTokens } from '../../charts/useChartTokens';
import { formatTerritory, MEASURES, territoryNote } from './territoryData';
import { territoryShareReading, territoryWorldModel } from './territoryShareData';

interface TerritoryShareProps {
  series: ReturnType<typeof raceSeriesIndex>;
  worldReply: TerritoryReply;
  measure: TerritoryMeasure;
  rule: TerritoryRule;
  hour: number;
  count: number;
  nameOf: (key: string, hour?: number) => string;
  countryFlagOf: (key: string) => NonNullable<SunburstKey['countryFlag']>;
  colourOf: (key: string) => string;
  factions?: { reply: FactionTotalsReply };
  unavailableReason?: string | null;
  expectedKeys?: readonly string[];
  membershipNote?: (key: string) => string | null;
  nameReason?: (key: string) => string | null;
  hoverLabel?: CountryLabel;
  onHover?: (key: string) => void;
  onPick?: (hour: number, key: string) => void;
  pickOwner?: unknown;
  active: boolean;
  hoursPerSecond: number | undefined;
  onContextMenu: (x: number, y: number) => void;
}

const SETTINGS = { growth: 'share', labels: true, legend: true, breadcrumb: false, dimming: true } as const;
const REST = JSON.stringify(['rest-of-world']);
const entityKey = (key: string) => JSON.stringify(['entity', key]);
const countryKey = (node: SunburstNode): string => JSON.parse(node.key)[1];

/** Territory supplies complete readings; the shared sunburst owns every shape. */
export default function TerritoryShare(props: TerritoryShareProps) {
  const { worldReply, measure, rule, series, hour, count, factions, nameOf, countryFlagOf, colourOf, hoverLabel, onHover } = props;
  const tokens = useChartTokens();
  const measureName = MEASURES.find(item => item.value === measure)!.label;
  const root = useMemo(() => ({ id: 'territory-world', label: measureName }), [measureName]);
  const world = useMemo(() => territoryWorldModel(worldReply, measure, rule), [worldReply, measure, rule]);
  const reading = useMemo(() => territoryShareReading(series, world, hour, count,
    key => {
      const note = props.membershipNote?.(key);
      return { id: entityKey(key), label: note ? `${nameOf(key, hour)} · ${note}` : nameOf(key, hour), countryFlag: factions ? undefined : countryFlagOf(key), colour: colourOf(key) };
    },
    { id: REST, label: 'Rest of world', colour: tokens.muted }, factions?.reply, props.unavailableReason, props.expectedKeys),
  [series, world, hour, count, nameOf, countryFlagOf, colourOf, tokens.muted, factions?.reply, props.unavailableReason, props.expectedKeys, props.membershipNote]);
  const text = useMemo<SunburstText>(() => ({ format: (value, exact) => exact == null ? formatTerritory(value) : usNumber.format(exact),
    unit: measure === 'area' ? worldReply.areaUnit ?? '' : measure === 'victoryPoints' ? 'victory points' : measure,
    valueLabel: measureName, emptyLabel: 'No share to show.', missingLabel: reading.missingLabel }), [measure, worldReply.areaUnit, measureName, reading.missingLabel]);
  const tooltip = useCallback((node: SunburstNode, focus: SunburstNode, words: SunburstText) => {
    const rest = node.key === REST;
    const label = !rest ? hoverLabel?.(countryKey(node)) : undefined;
    return shareTooltip(sliceTooltip(node, focus, words), label,
      `${dayLabel(hour)} · ${rule === 'owned' ? 'Owned' : 'Controlled'}`,
      [!rest && props.nameReason?.(countryKey(node)), !rest && props.membershipNote?.(countryKey(node)), territoryNote(measure)].filter(Boolean).join(' ') || undefined);
  }, [hoverLabel, hour, rule, measure, props.membershipNote, props.nameReason]);
  const hover = useCallback((node: SunburstNode) => { if (node.key !== REST) onHover?.(countryKey(node)); }, [onHover]);
  const context = useCallback((_node: SunburstNode | null, x: number, y: number) => props.onContextMenu(x, y), [props.onContextMenu]);
  const canPick = useCallback((node: SunburstNode) => !!props.onPick && node.key.startsWith('["entity",')
    && series.series.has(countryKey(node)), [props.onPick, series]);
  const pick = useCallback((node: SunburstNode) => {
    if (!canPick(node)) return false;
    props.onPick!(hour, countryKey(node)); return true;
  }, [canPick, props.onPick, hour]);
  return <Sunburst label={`Territory share, ${measureName}, ${dayLabel(hour)}`} items={reading.items} root={root} text={text}
    settings={SETTINGS} summaryNotes rings={1} fold={false} unavailableReason={reading.unavailableReason}
    stretch={reading.stretch == null ? null : `${worldReply.branchId}:${measure}:${rule}:${reading.stretch}`}
    tooltip={tooltip} onHover={hover} onPick={props.onPick ? pick : undefined} canPick={canPick} pickOwner={props.pickOwner}
    onContextMenu={context} active={props.active} hoursPerSecond={props.hoursPerSecond} footnote={territoryNote(measure)} />;
}
