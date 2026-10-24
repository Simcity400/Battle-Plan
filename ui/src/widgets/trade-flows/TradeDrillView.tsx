import type { useTradeFlows } from './useTradeFlows';
import { useCallback, useMemo, useRef, useState } from 'react';
import DrillParts from '../../charts/drill/DrillParts';
import type { DrillPart } from '../../charts/drill/DrillParts';
import { dayLabel, drawBands, drawPlayhead } from '../../charts/draw';
import { drawApproximateSpans, summaryFits } from '../../charts/chartSummary';
import { usNumber } from '../../charts/numberFormat';
import { compactStack, compactStackExtent, forEachCompactStackSegment } from '../../charts/stack';
import { heldStackSegment } from '../../charts/race/countryRace';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import type { ChartTokens } from '../../charts/tokens';
import { formatAmount, rescale } from '../../charts/valueAxis';
import { MAX_NAME_HOURS } from '../../dashboard/countryFacts';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { lookHour } from '../../dashboard/countryLooks';
import type { NameAt } from '../../dashboard/countryNamesAt';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import type { HourSpan } from '../../generated/dashboard';
import { preparedContributionStack, contributionAmounts, contributionHistory, contributionMeasureExtent, contributionMeasureModel, forEachContributionMeasureSegment } from './tradeDrill';
import { coverageNote, flowCountryAt, formatDays, knownHours, knownSpanAt, measureAmount, measureNote, resourceName } from './tradeData';
import type { Flow, Measure, TradeData, TradeCountryFact } from './tradeData';
import { TOOLTIP_LIMIT } from './tradeTooltip';

/** The complete native holder/resource inventory, and the same measure over time. */
export default function TradeDrill({ prepared, flows, trade, track, runtime, measure, from, hour, extent, playhead, known, colours, tokens, identityNote, active, onPick, onContextMenu }: {
  flows: readonly Flow[]; trade: TradeData; track: BoundTrack; runtime: WidgetRuntime; measure: Measure; from: number;
  hour: number | null; extent: [number, number]; playhead: number | null; known: readonly HourSpan[];
  colours: ReadonlyMap<number, string>; tokens: ChartTokens; identityNote?: string; active: boolean; onPick: (hour: number) => void;
  onContextMenu: (hour: number, x: number, y: number) => void;
  prepared?: Pick<ReturnType<typeof useTradeFlows>, 'onSummaryView' | 'onHover' | 'onNavigateHour' | 'keyboardHour' | 'keyboardRevision' | 'hasExactHour'>;
}) {
  const [visible, setVisible] = useState<readonly string[]>([]);
  const [hovered, setHovered] = useState<{ hour: number; flows: readonly Flow[] } | null>(null);
  const hoverKey = useRef('');
  const visibleSet = useMemo(() => new Set(visible), [visible]);
  const factHour = useCallback((fact: TradeCountryFact, exact: boolean) => exact ? fact.hour
    : lookHour(track.branch, fact.hour, fact.firstHour)?.hour ?? fact.hour, [track.branch]);
  const wanted = useMemo<NameAt[]>(() => {
    const facts: NameAt[] = [];
    const add = (flow: Flow, at: number, exact: boolean) => {
      for (const side of ['seller', 'buyer'] as const) {
        const fact = flowCountryAt(flow, side, at);
        if (fact) facts.push({ tag: fact.tag, hour: factHour(fact, exact) });
      }
    };
    if (hour != null) for (const flow of flows) if (visibleSet.has(flow.key)) add(flow, hour, false);
    if (hovered) for (const flow of hovered.flows) add(flow, hovered.hour, true);
    return facts;
  }, [flows, hour, visibleSet, hovered, factHour]);
  const labels = useCountryLabelsAt(runtime.client, track, wanted, active, Math.max(MAX_NAME_HOURS, new Set(wanted.map(item => item.hour)).size));
  const named = useCallback((flow: Flow, at: number, exact = false) => {
    const name = (side: 'seller' | 'buyer') => {
      const fact = flowCountryAt(flow, side, at);
      return fact ? labels.labelAt(factHour(fact, exact))(fact.tag).name : 'Unknown country';
    };
    return `${name('seller')} → ${name('buyer')} · ${resourceName(flow.resource)}`;
  }, [labels.labelAt, factHour]);
  const ask = useCallback((at: number, selected: readonly Flow[]) => {
    const key = JSON.stringify([at, selected.map(flow => flow.key)]);
    if (key === hoverKey.current) return;
    hoverKey.current = key;
    queueMicrotask(() => setHovered({ hour: at, flows: selected }));
  }, []);
  const pending = hour != null && !!prepared?.hasExactHour && !prepared.hasExactHour(hour);
  const amounts = useMemo(() => hour == null || pending ? [] : contributionAmounts(flows, measure, from, hour, known), [flows, measure, from, hour, known, pending]);
  const total = hour == null || !flows.length || amounts.some(item => item.raw == null) ? null : measureAmount(flows, measure, from, hour);
  const scale = 10 ** trade.reply.decimals;
  const parts = useMemo<DrillPart[]>(() => amounts.map(({ flow, raw }) => {
    const seller = visibleSet.has(flow.key) ? flowCountryAt(flow, 'seller', hour!) : null;
    return { id: flow.key,
    label: named(flow, hour!), value: raw == null ? null : Number(raw) / scale, raw,
    text: raw == null ? 'Not recorded' : formatAmount(raw, trade.reply.decimals, trade.reply.decimals),
    shareText: raw == null || total == null || total === 0n ? '—'
      : `${formatAmount(rescale(raw * (total < 0n ? -100n : 100n), total < 0n ? -total : total, 1), 1, 1)}%`,
    color: colours.get(flow.resource.id) ?? tokens.muted,
    countryFlag: seller ? labels.labelAt(factHour(seller, false))(seller.tag) : undefined,
  }; }), [amounts, named, hour, scale, trade.reply.decimals, total, colours, tokens.muted, visibleSet, labels.labelAt, factHour]);
  const dense = useMemo(() => preparedContributionStack(flows), [flows]);
  const stack = useMemo(() => measure === 'current' ? compactStack(flows.map(flow => contributionHistory(flow, known))) : null, [flows, known, measure]);
  const measured = useMemo(() => measure === 'current' ? null : contributionMeasureModel(flows, measure, from, known), [flows, measure, from, known]);
  const colors = useMemo(() => flows.map(flow => colours.get(flow.resource.id) ?? tokens.muted), [flows, colours, tokens.muted]);
  const layers = useMemo<ChartLayer[]>(() => [{
    draw: (ctx, frame, chartTokens, state) => {
      const end = Math.min(frame.x[1], state.playhead ?? frame.x[1]);
      const minHours = (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth);
      if (dense && !summaryFits(frame, dense, end)) return;
      if (dense) drawApproximateSpans(ctx, frame, dense.approximate, end, () => forEachCompactStackSegment(dense.model, frame.x[0], end, minHours, piece => {
        const geometry = measure === 'current' ? heldStackSegment(piece) : piece;
        drawBands(ctx, frame, chartTokens, geometry.hours, geometry.bands, colors, geometry);
      }, dense.sample));
      else if (stack) forEachCompactStackSegment(stack, frame.x[0], end, minHours, piece => {
        const held = heldStackSegment(piece);
        drawBands(ctx, frame, chartTokens, held.hours, held.bands, colors, held);
      });
      else if (measured) forEachContributionMeasureSegment(measured, frame.x[0], end, minHours,
        piece => drawBands(ctx, frame, chartTokens, piece.hours, piece.bands, colors, piece));
    },
    extent: x => dense ? compactStackExtent(dense.model, x[0], x[1]) : stack ? compactStackExtent(stack, x[0], x[1]) : measured ? contributionMeasureExtent(measured, x[0], x[1]) : null,
  }], [dense, stack, measured, colors, measure]);
  const missingGroupHours = hour == null ? 0 : knownHours(trade.reply.known, from, hour) - knownHours(known, from, hour);
  const note = hour == null ? undefined : [coverageNote(trade.reply, measure, from, hour),
    measure !== 'current' && missingGroupHours > 0 ? `Group membership is missing for ${formatDays(missingGroupHours)}.` : null]
    .filter(Boolean).join(' ') || undefined;
  return <div className="trade-drill">
    <p className="drill-empty">{measureNote(measure)}{hour != null ? ` · ${dayLabel(hour)}` : ''}. {usNumber.format(flows.length)} trades.</p>
    {identityNote && <p className="drill-empty">{identityNote}</p>}
    {!flows.length && <p className="drill-empty">No trades match this selection.</p>}
    {hour == null ? <p className="drill-empty">Choose a day.</p> : pending ? <p className="drill-empty">Loading trade amounts…</p>
      : <DrillParts parts={parts} virtual active={active} onVisible={setVisible} positiveBars signedShares
        total={total == null ? null : Number(total) / scale} format={value => String(value)}
        totalRow={{ label: 'Total', value: total == null ? 'Not recorded' : formatAmount(total, trade.reply.decimals, trade.reply.decimals) }} />}
    {note && <p className="drill-empty">{note}</p>}
    <p className="drill-empty">{measureNote(measure)} over time.</p>
    {!!dense?.approximate.length && <p className="chart-line-key"><i className="is-dashed" aria-hidden="true" />Approximate</p>}
    <div className="trade-drill-chart">
      <TimeChart label={`Selected trade contributors, ${measureNote(measure).toLowerCase()} over time`} extent={extent} branchStart={track.branch.firstHour}
        playhead={playhead} playback="cursor" afterPlayhead hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond}
        axes={{ xFrom: null, xTo: null, yMin: 0, yMax: null, log: false }} layers={layers} onSummaryView={prepared?.onSummaryView} onHover={prepared?.onHover}
        onNavigateHour={prepared?.onNavigateHour} keyboardHour={prepared?.keyboardHour} keyboardRevision={prepared?.keyboardRevision} keyHours={stack?.hours ?? measured?.hours}
        active={active} revision={dense ?? stack ?? measured} onPick={onPick} onContextMenu={onContextMenu}
        overlay={(ctx, frame, chartTokens) => { if (hour != null) drawPlayhead(ctx, frame, chartTokens, hour); }}
        tooltip={at => {
          if (prepared?.hasExactHour && !prepared.hasExactHour(at)) return { title: dayLabel(at), rows: [{ label: 'Trade amounts', value: 'Loading...', tone: 'muted' }] };
          if (knownSpanAt(known, at) < 0) return { title: dayLabel(at), rows: [{ label: 'Trade contributions', value: 'Not recorded', tone: 'muted' }] };
          const values = contributionAmounts(flows, measure, from, at, known).sort((a, b) => {
            const first = a.raw! < 0n ? -a.raw! : a.raw!;
            const second = b.raw! < 0n ? -b.raw! : b.raw!;
            return first === second ? a.flow.key.localeCompare(b.flow.key) : first > second ? -1 : 1;
          });
          const listed = values.slice(0, TOOLTIP_LIMIT);
          ask(at, listed.map(item => item.flow));
          return { title: dayLabel(at), rows: [...listed.map(({ flow, raw }) => ({ label: named(flow, at, true),
            icon: flow.resource.icon ?? undefined, swatch: colours.get(flow.resource.id), value: formatAmount(raw!, trade.reply.decimals, trade.reply.decimals), short: true })),
          { label: measure === 'total' ? 'Delivered total' : measure === 'average' ? 'Average per trading day' : 'Total per day',
            value: formatAmount(measureAmount(flows, measure, from, at), trade.reply.decimals, trade.reply.decimals), strong: true }],
          note: values.length > listed.length ? `${usNumber.format(values.length - listed.length)} more trades` : undefined };
        }} />
    </div>
  </div>;
}
