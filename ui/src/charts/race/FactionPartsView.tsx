import { useCallback, useMemo, useRef, useState } from 'react';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useCountryFactsRead } from '../../dashboard/countryNamesAt';
import { MAX_NAME_HOURS } from '../../dashboard/countryFacts';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { STYLES } from '../../dashboard/useCountriesAt';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { usePlaybackSpeed } from '../../dashboard/playback';
import DrillParts from '../drill/DrillParts';
import type { DrillPart } from '../drill/DrillParts';
import { dayLabel, drawBands, drawPlayhead } from '../draw';
import { runAt } from '../series';
import { compactStack, compactStackExtent, forEachCompactStackSegment } from '../stack';
import { drawApproximateSpans, summaryStack } from '../chartSummary';
import type { ChartSummary } from '../chartSummary';
import TimeChart from '../TimeChart';
import type { ChartLayer, TimeChartProps } from '../TimeChart';
import { useParityRows } from '../../dashboard/parityRows';
import { useChartTokens } from '../useChartTokens';
import { heldStackSegment } from './countryRace';
import { factionPartShare, factionPartWitness } from './factionParts';
import type { FactionParts } from './factionParts';
import { factionColourKey, factionColourPlan, factionCountryColours, heldFactionColours } from './factionPartColours';
import './factionParts.css';

/** The same full member inventory serves Faction strength and Territory. Names
 * and flags are requested only for mounted rows at their proven identity hour. */
export default function FactionPartsView({ model, runtime, track, hour, following, extent, active, pickOwner, onPick,
  label, formatValue, summaries, totalSummary, onSummaryView, onReachHour, navigation }: {
  model: FactionParts; runtime: WidgetRuntime; track: BoundTrack; hour: number; following: boolean;
  extent: readonly [number, number]; active: boolean; pickOwner: unknown; onPick: (hour: number) => void;
  label: string; formatValue: (value: number) => string;
  summaries?: Readonly<Record<string, ChartSummary>>; totalSummary?: ChartSummary;
  onSummaryView?: TimeChartProps['onSummaryView']; onReachHour?: (hour: number | null) => void;
  navigation?: Pick<TimeChartProps, 'onNavigateHour' | 'keyboardHour' | 'keyboardRevision'>;
}) {
  const tokens = useChartTokens();
  const hoursPerSecond = usePlaybackSpeed(runtime.clock);
  const [visible, setVisible] = useState<readonly string[]>([]);
  const onVisible = useCallback((ids: readonly string[]) => setVisible(before => before.length === ids.length
    && before.every((id, index) => id === ids[index]) ? before : ids), []);
  const byId = useMemo(() => new Map(model.parts.map(part => [part.id, part])), [model]);
  const wanted = useMemo(() => visible.flatMap(id => {
    const part = byId.get(id);
    const witness = part && factionPartWitness(part, hour);
    return witness ? [witness] : [];
  }), [visible, byId, hour]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active,
    Math.max(MAX_NAME_HOURS, new Set(wanted.map(item => item.hour)).size));
  const fixed = tokens.countryColours === 'fixed';
  const plan = useMemo(() => factionColourPlan(model, track.branch, hour, fixed), [model, track.branch, hour, fixed]);
  const styles = useCountryFactsRead(runtime.client, track, plan.wanted, active, STYLES, plan.maxHours);
  const colourKey = useMemo(() => factionColourKey(plan, styles), [plan, styles]);
  const corrected = useMemo(() => colourKey == null ? null : factionCountryColours(colourKey, tokens.surface, fixed), [colourKey, tokens.surface, fixed]);
  const content = useContentRevision(runtime.client);
  const context = JSON.stringify([track.source.id, track.branch.id, content, tokens.surface, fixed]);
  const held = useRef({ context, client: runtime.client, colours: new Map<string, string>(), selection: plan.selection });
  if (held.current.context !== context || held.current.client !== runtime.client) held.current = {
    context, client: runtime.client, colours: new Map(), selection: plan.selection,
  };
  const colors = useMemo(() => {
    const colours = corrected ?? heldFactionColours(held.current.colours, held.current.selection, plan.selection, styles);
    held.current.colours = colours; held.current.selection = plan.selection;
    return colours;
  }, [corrected, plan, styles, context, runtime.client]);
  const bandColors = useMemo(() => [...model.parts.map(part => colors.get(part.id) ?? tokens.muted), tokens.muted], [model, colors, tokens.muted]);
  const stack = useMemo(() => compactStack([...model.parts.map(part => part.runs.filter(run => run.raw != null)), model.remainder]), [model]);
  const dense = useMemo(() => totalSummary && (!totalSummary.runs || totalSummary.approximate?.length) && summaries && model.parts.every(part => summaries[part.id])
    ? summaryStack(model.parts.map(part => summaries[part.id]), totalSummary) : null, [model, summaries, totalSummary]);
  const layers = useMemo<ChartLayer[]>(() => [{
    draw: (ctx, frame, chartTokens, state) => {
      const end = Math.min(frame.x[1], state.playhead ?? frame.x[1]);
      if (totalSummary) {
        if (totalSummary.from !== frame.x[0] || totalSummary.to !== end || totalSummary.width !== Math.ceil(frame.plotWidth)) return;
        if (!totalSummary.runs || totalSummary.approximate?.length) {
          if (dense) drawApproximateSpans(ctx, frame, dense.approximate, end, () => {
            forEachCompactStackSegment(dense.model, frame.x[0], end, 0, segment => {
              const held = heldStackSegment(segment);
              drawBands(ctx, frame, chartTokens, held.hours, held.bands, bandColors, held);
            }, dense.sample);
          });
          return;
        }
      }
      drawApproximateSpans(ctx, frame, totalSummary?.approximate, end, () => {
        forEachCompactStackSegment(stack, frame.x[0], end, (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth), segment => {
          const held = heldStackSegment(segment);
          drawBands(ctx, frame, chartTokens, held.hours, held.bands, bandColors, held);
        });
      });
    },
    extent: x => totalSummary && (!totalSummary.runs || totalSummary.approximate?.length) ? dense ? compactStackExtent(dense.model, x[0], Math.min(x[1], following ? hour : x[1])) : null
      : compactStackExtent(stack, x[0], Math.min(x[1], following ? hour : x[1])),
  }], [stack, bandColors, following, hour, totalSummary, dense]);
  const whole = runAt(model.total, hour);
  const total = whole ? BigInt(whole.value) : 0n;
  let unknown = 0;
  const rows: DrillPart[] = model.parts.filter(part => runAt(part.counted, hour)).map(part => {
    const run = runAt(part.runs, hour);
    const raw = run?.raw ?? null;
    if (raw == null) unknown++;
    const witness = factionPartWitness(part, hour);
    const country = witness ? labelAt(witness.hour)(witness.tag) : undefined;
    const name = country?.name ?? 'Country name unavailable';
    return { id: part.id, label: name, countryFlag: country ?? {},
      value: raw == null ? null : Number(raw), raw,
      text: raw == null ? 'Not recorded' : Number.isSafeInteger(Number(raw)) ? formatValue(Number(raw)) : raw.toLocaleString(),
      color: colors.get(part.id) ?? tokens.muted, shareText: factionPartShare(raw, total) };
  });
  const nameCounts = new Map<string, number>();
  for (const row of rows) nameCounts.set(row.label, (nameCounts.get(row.label) ?? 0) + 1);
  for (const row of rows) if ((nameCounts.get(row.label) ?? 0) > 1) {
    const part = byId.get(row.id)!;
    const tag = factionPartWitness(part, hour)?.tag ?? part.witnesses[0]?.tag;
    if (tag && row.label !== tag) row.label += ` (${tag})`;
  }
  const remainder = runAt(model.remainder, hour)?.raw;
  const parityRoot = useRef<HTMLDivElement>(null);
  useParityRows(parityRoot, () => ({ kind: 'table', label: 'Faction member values', complete: !!whole, hour, total: rows.length,
    rows: rows.map(row => ({ id: row.id, value: row.raw?.toString() ?? null })) }));
  return <div ref={parityRoot} className="faction-parts">
    {!whole && <p className="drill-empty">Faction total unavailable on {dayLabel(hour)}.</p>}
    <DrillParts parts={rows} virtual active={active} onVisible={onVisible} signedShares total={whole?.value ?? null} showShares={whole !== null} format={formatValue}
        unattributed={remainder == null ? undefined : { label: 'Not identified', value: Number(remainder), raw: remainder,
          text: Number.isSafeInteger(Number(remainder)) ? formatValue(Number(remainder)) : remainder.toLocaleString(),
          color: tokens.muted, shareText: factionPartShare(remainder, total) }}
        totalRow={{ label: 'Total', value: whole ? formatValue(whole.value) : 'Not recorded' }} />
    {(unknown > 0 || model.unidentified > 0) && <p className="drill-empty">Some member values are unavailable.</p>}
    <div className="faction-parts-history">
      <TimeChart label={`${label}: members over time`} extent={[extent[0], extent[1]]} branchStart={track.branch.firstHour}
        playhead={following ? hour : null} playback="cursor" afterPlayhead={false} windowHours={0} hoursPerSecond={hoursPerSecond}
        active={active} revision={stack} pickOwner={pickOwner} layers={layers} formatValue={formatValue}
        axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }} onPick={onPick}
        onSummaryView={onSummaryView} onHover={onReachHour} {...navigation}
        overlay={(ctx, frame, chartTokens) => drawPlayhead(ctx, frame, chartTokens, hour)}
        tooltip={at => ({ title: dayLabel(at), rows: [{ label: 'Total', value: runAt(model.total, at)
          ? formatValue(runAt(model.total, at)!.value) : 'Not recorded' }] })} />
    </div>
  </div>;
}
