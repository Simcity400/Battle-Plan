import { useCallback, useEffect, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import FactionTotalsView from '../../charts/race/FactionTotalsView';
import AggregateTotalsView, { aggregated } from '../../charts/race/AggregateTotalsView';
import MeasureRace from '../../charts/race/MeasureRace';
import Sunburst from '../../charts/sunburst/Sunburst';
import { useChartTokens } from '../../charts/useChartTokens';
import { useCountryLooks } from '../../dashboard/countryLooks';
import type { WidgetProps } from '../../dashboard/definitions';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { resolveOptions } from '../../dashboard/options';
import type { OptionValue } from '../../dashboard/options';
import { AREA_UNIT, formatTerritory, MEASURES, measureOf, ruleOf, territoryMeasureLabel, territoryNote, territorySeriesMeasure } from '../../dashboard/territoryMeasure.ts';
import type { TerritoryMeasure, TerritoryRule } from '../../dashboard/territoryMeasure';
import { toHex } from '../../shell/appearance/color';
import { shareItems, shareStructure, validateShareReply } from './territoryData.ts';
import { mapLive } from '../../charts/live.ts';
import { useShownHour } from '../../dashboard/widgetPerf';
import { OPTIONS } from './definition';
import '../../charts/race/countryRace.css';

const NO_WARNINGS: string[] = [];
const SHARE_SETTINGS = { growth: 'share', labels: true, legend: true, breadcrumb: false, dimming: true } as const;

export default function TerritoryWidget(props: WidgetProps) {
  const options = resolveOptions(OPTIONS, props.widget.options);
  const measure = measureOf(options.measure), rule = ruleOf(options.rule);
  if (props.widget.countries.kind === 'factions') return <FactionTotalsView {...props} options={options} kind="territory"
    measure={territorySeriesMeasure(measure, rule)} label="Territory" measureLabel={territoryMeasureLabel(measure, rule)}
    note={territoryNote(measure)} format={formatTerritory} />;
  if (aggregated(props.widget.countries) && options.view !== 'share') return <AggregateTotalsView {...props} options={options} kind="territory"
    measure={territorySeriesMeasure(measure, rule)} label="Territory" measureLabel={territoryMeasureLabel(measure, rule)}
    note={territoryNote(measure)} format={formatTerritory} />;
  if (options.view === 'share' && rule !== 'occupied') return <TerritoryShareView {...props} measure={measure} rule={rule} />;
  return <TerritoryRaceView {...props} options={options} measure={measure} rule={rule} />;
}

interface Chosen { measure: TerritoryMeasure; rule: TerritoryRule }

const formatRaw = (raw: string) => formatTerritory(Number(raw));

function TerritoryRaceView({ widget, runtime, active, onDiagnostics, options, measure, rule }: WidgetProps & Chosen & { options: Readonly<Record<string, OptionValue>> }) {
  const family = useMemo(() => ({ kind: 'territory', measure: territorySeriesMeasure(measure, rule) }), [measure, rule]);
  const onWarnings = useCallback((warnings: readonly string[]) => onDiagnostics(warnings.length ? [...warnings] : NO_WARNINGS), [onDiagnostics]);
  return <MeasureRace widget={widget} runtime={runtime} active={active} measure={family} decimals={0} view="lines"
    options={options} label={`Territory, ${territoryMeasureLabel(measure, rule)}`} icon="map" format={formatRaw} formatValue={formatTerritory}
    note={territoryNote(measure)} onWarnings={onWarnings} />;
}

function TerritoryShareView({ widget, runtime, active, onDiagnostics, measure, rule }: WidgetProps & Chosen) {
  const tokens = useChartTokens();
  const shown = useShownHour();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const name = territorySeriesMeasure(measure, rule);
  const count = widget.countries.kind === 'group' && widget.countries.group.type === 'top' ? widget.countries.group.count : 10;
  const share = useHeadProduct(runtime, useGate(runtime, widget), 'territory-share', track ? { measure: name } : null,
    { active, validate: validateShareReply, structure: reply => shareStructure(reply, count) });
  const hour = share.at ?? (track ? Math.floor(runtime.clock.getHour()) + track.axisOffset : 0);
  const shownTags = useMemo(() => share.data?.entries.slice(0, count).map(entry => entry.tag) ?? [], [share.data, count]);
  const looks = useCountryLooks(runtime.client, track, shownTags, hour, active);
  const measureName = MEASURES.find(item => item.value === measure)!.label;
  const look = useCallback((tag: string) => {
    const colour = looks.color(tag);
    return { id: JSON.stringify(['entity', tag]), label: looks.name(tag), countryFlag: { flag: looks.flag(tag), flagPending: looks.flagPending(tag), flagReason: looks.flagReason(tag) },
      ...(colour ? { colour: toHex({ r: colour[0], g: colour[1], b: colour[2] }) } : {}) };
  }, [looks]);
  const rest = useMemo(() => ({ id: JSON.stringify(['rest']), label: 'Rest of the world', colour: tokens.muted }), [tokens.muted]);
  const reading = useMemo(() => share.data ? shareItems(share.data, count, share.live.read(), look, rest) : null, [share.data, share.live, count, look, rest]);
  const live = useMemo(() => share.data ? mapLive(share.live, values => {
    shown(share.shownAt());
    const made = shareItems(share.data!, count, values, look, rest);
    return { items: made.items, unavailableReason: made.unavailableReason };
  }) : undefined, [share.data, share.live, share.shownAt, count, look, rest, shown]);
  useEffect(() => { if (!share.data) shown(null); }, [share.data, shown]);
  const text = useMemo(() => ({ format: (value: number) => formatTerritory(value), unit: measure === 'area' ? AREA_UNIT : measure === 'victoryPoints' ? 'victory points' : measure,
    valueLabel: measureName, emptyLabel: 'No share to show.', missingLabel: 'Not recorded' }), [measure, measureName]);
  const diagnosticKey = JSON.stringify([...(share.data?.warnings ?? []), ...(share.data?.unnamed ? ['Some territory is held by countries no tag names at this hour; it counts in the rest of the world.'] : []),
    ...(share.reason ? [`Territory share is asked by the fallback clock: ${share.reason}`] : [])]);
  useEffect(() => {
    const warnings = JSON.parse(diagnosticKey) as string[];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [diagnosticKey, onDiagnostics]);
  if (!track) return <WidgetState kind="empty" icon="map" message="Load a campaign to see its territory." />;
  if (share.error) return <WidgetState kind="error" failure={share.error} onRetry={share.retry} />;
  return <div className="country-race" ref={share.root}><div className="country-race-chart">
    {!reading ? <WidgetState kind="loading" label="Loading territory share…" />
      : <Sunburst label={`Territory share, ${measureName}, ${dayLabel(hour)}`} items={reading.items} root={{ id: 'territory-world', label: measureName }}
        text={text} settings={SHARE_SETTINGS} summaryNotes rings={1} fold={false} unavailableReason={reading.unavailableReason}
        active={active} footnote={territoryNote(measure)} live={live} />}
  </div></div>;
}
