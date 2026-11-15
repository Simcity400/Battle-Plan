import { countryNameDetails } from '../../charts/tooltip.ts';
import { useCallback, useMemo, useRef, useState } from 'react';
import DrillParts from '../../charts/drill/DrillParts';
import type { DrillPart, LiveDrillParts } from '../../charts/drill/DrillParts';
import { dayLabel, drawBands, drawPlayhead } from '../../charts/draw';
import { mapLive } from '../../charts/live';
import type { Live } from '../../charts/live';
import LiveText from '../../charts/LiveText';
import type { LiveTexts } from '../../charts/LiveText';
import { usNumber } from '../../charts/numberFormat';
import { compactStack, compactStackExtent, forEachCompactStackSegment, forEachJoinedSegment } from '../../charts/stack';
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
import { contributionAmounts, contributionHistory, contributionMeasureExtent, contributionMeasureModel, forEachContributionMeasureSegment } from './tradeDrill';
import { coverageNote, flowCountryAt, formatDays, knownHours, knownSpanAt, measureAmount, measureNote, resourceName } from './tradeData';
import type { Flow, Measure, TradeData, TradeCountryFact } from './tradeData';
import { TOOLTIP_LIMIT } from './tradeTooltip';
import { motionOf } from '../../charts/easing.ts';

/** The drill's hour each frame while it follows playback, with its texts. */
export type LiveHour = Live<number> & { hour(): number; texts: LiveTexts };

/** The complete native holder/resource inventory, and the same measure over time. While it follows
 * playback the amounts and the playhead are painted at the head each frame (`live`); `hour` is then
 * the hour its names are read at, which moves with the look grid, not each hour. */
export default function TradeDrill({ flows, trade, track, runtime, measure, from, hour, live, extent, known, colours, tokens, identityNote, active, onPick, onContextMenu }: {
  flows: readonly Flow[]; trade: TradeData; track: BoundTrack; runtime: WidgetRuntime; measure: Measure; from: number;
  hour: number | null; live: LiveHour | null; extent: [number, number]; known: readonly HourSpan[];
  colours: ReadonlyMap<number, string>; tokens: ChartTokens; identityNote?: string; active: boolean; onPick: (hour: number) => void;
  onContextMenu: (hour: number, x: number, y: number) => void;
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
  const nameDetails = useCallback((flow: Flow, at: number, exact = false) => countryNameDetails(...(['seller', 'buyer'] as const).map(side => {
    const fact = flowCountryAt(flow, side, at);
    return fact ? labels.labelAt(factHour(fact, exact))(fact.tag) : undefined;
  })), [labels.labelAt, factHour]);
  const ask = useCallback((at: number, selected: readonly Flow[]) => {
    const key = JSON.stringify([at, selected.map(flow => flow.key)]);
    if (key === hoverKey.current) return;
    hoverKey.current = key;
    queueMicrotask(() => setHovered({ hour: at, flows: selected }));
  }, []);
  const scale = 10 ** trade.reply.decimals;
  // The parts at an hour: names and flags as read at `hour`, amounts at `at`.
  const partsAt = useCallback((at: number) => {
    const amounts = contributionAmounts(flows, measure, from, at, known);
    const total = !flows.length || amounts.some(item => item.raw == null) ? null : measureAmount(flows, measure, from, at);
    const named_ = hour ?? at;
    const parts = amounts.map(({ flow, raw }): DrillPart => {
      const seller = visibleSet.has(flow.key) ? flowCountryAt(flow, 'seller', named_) : null;
      return { ...nameDetails(flow, named_), id: flow.key,
        label: named(flow, named_), value: raw == null ? null : Number(raw) / scale, raw,
        text: raw == null ? 'Not recorded' : formatAmount(raw, trade.reply.decimals, trade.reply.decimals),
        shareText: raw == null || total == null || total === 0n ? '—'
          : `${formatAmount(rescale(raw * (total < 0n ? -100n : 100n), total < 0n ? -total : total, 1), 1, 1)}%`,
        color: colours.get(flow.resource.id) ?? tokens.muted,
        countryFlag: seller ? labels.labelAt(factHour(seller, false))(seller.tag) : undefined };
    });
    return { parts, total: total == null ? null : Number(total) / scale,
      totalText: total == null ? 'Not recorded' : formatAmount(total, trade.reply.decimals, trade.reply.decimals) };
  }, [flows, measure, from, known, hour, visibleSet, nameDetails, named, scale, trade.reply.decimals, colours, tokens.muted, labels.labelAt, factHour]);
  const shown = useMemo(() => hour == null || live ? null : partsAt(hour), [hour, live, partsAt]);
  const liveParts = useMemo<LiveDrillParts | undefined>(() => live ? mapLive(live, partsAt) : undefined, [live, partsAt]);
  const stack = useMemo(() => measure === 'current' ? compactStack(flows.map(flow => contributionHistory(flow, known))) : null, [flows, known, measure]);
  const measured = useMemo(() => measure === 'current' ? null : contributionMeasureModel(flows, measure, from, known), [flows, measure, from, known]);
  const colors = useMemo(() => flows.map(flow => colours.get(flow.resource.id) ?? tokens.muted), [flows, colours, tokens.muted]);
  const layers = useMemo<ChartLayer[]>(() => [{
    draw: (ctx, frame, chartTokens, state) => {
      const end = Math.min(frame.x[1], state.playhead ?? frame.x[1]);
      const minHours = (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth);
      if (stack) forEachJoinedSegment(piece => forEachCompactStackSegment(stack, frame.x[0], end, minHours, segment => piece(heldStackSegment(segment))),
        held => drawBands(ctx, frame, chartTokens, held.hours, held.bands, colors, held));
      else if (measured) forEachJoinedSegment(piece => forEachContributionMeasureSegment(measured, frame.x[0], end, minHours, piece),
        piece => drawBands(ctx, frame, chartTokens, piece.hours, piece.bands, colors, piece));
    },
    extent: x => stack ? compactStackExtent(stack, x[0], x[1]) : measured ? contributionMeasureExtent(measured, x[0], x[1]) : null,
  }], [stack, measured, colors]);
  const noteAt = useCallback((at: number) => {
    const missingGroupHours = knownHours(trade.reply.known, from, at) - knownHours(known, from, at);
    return [coverageNote(trade.reply, measure, from, at), measure !== 'current' && missingGroupHours > 0 ? `Group membership is missing for ${formatDays(missingGroupHours)}.` : null]
      .filter(Boolean).join(' ');
  }, [trade.reply, measure, from, known]);
  const note = hour == null || live ? '' : noteAt(hour);
  const parts = shown?.parts ?? (live ? partsAt(live.hour()).parts : []);
  return <div className="trade-drill">
    <p className="drill-empty">{measureNote(measure)}{live ? <> · <LiveText live={live.texts} at="day" /></> : hour != null ? ` · ${dayLabel(hour)}` : ''}. {usNumber.format(flows.length)} trades.</p>
    {identityNote && <p className="drill-empty">{identityNote}</p>}
    {!flows.length && <p className="drill-empty">No trades match this selection.</p>}
    {hour == null && !live ? <p className="drill-empty">Choose a day.</p>
      : <DrillParts parts={parts} live={liveParts} virtual active={active} onVisible={setVisible} positiveBars signedShares
        total={shown?.total ?? null} format={value => String(value)}
        totalRow={{ label: 'Total', value: shown?.totalText ?? 'Not recorded' }} />}
    {live ? <p className="drill-empty"><LiveText live={live.texts} at="note" /></p> : note && <p className="drill-empty">{note}</p>}
    <p className="drill-empty">{measureNote(measure)} over time.</p>
    <div className="trade-drill-chart">
      <TimeChart label={`Selected trade contributors, ${measureNote(measure).toLowerCase()} over time`} extent={extent} branchStart={track.branch.firstHour}
        playhead={live ? live.hour() : hour} liveHead={live ?? undefined} playback="cursor" afterPlayhead hoursPerSecond={motionOf(runtime.clock.getSnapshot())}
        axes={{ xFrom: null, xTo: null, yMin: 0, yMax: null, log: false }} layers={layers} keyHours={stack?.hours ?? measured?.hours}
        active={active} revision={stack ?? measured} onPick={onPick} onContextMenu={onContextMenu}
        overlay={live ? undefined : (ctx, frame, chartTokens) => { if (hour != null) drawPlayhead(ctx, frame, chartTokens, hour); }}
        tooltip={at => {
          // An unrecorded hour hides only its own amounts: totals so far still add the hours before it.
          if (measure === 'current' && knownSpanAt(known, at) < 0) return { title: dayLabel(at), rows: [{ label: 'Trade contributions', value: 'Not recorded', tone: 'muted' }] };
          const amountsAt = contributionAmounts(flows, measure, from, at, known);
          if (amountsAt.some(item => item.raw == null)) return { title: dayLabel(at), rows: [{ label: 'Trade contributions', value: 'Not recorded', tone: 'muted' }] };
          const values = amountsAt.sort((a, b) => {
            const first = a.raw! < 0n ? -a.raw! : a.raw!;
            const second = b.raw! < 0n ? -b.raw! : b.raw!;
            return first === second ? a.flow.key.localeCompare(b.flow.key) : first > second ? -1 : 1;
          });
          const listed = values.slice(0, TOOLTIP_LIMIT);
          ask(at, listed.map(item => item.flow));
          return { title: dayLabel(at), rows: [...listed.map(({ flow, raw }) => ({ ...nameDetails(flow, at, true), label: named(flow, at, true),
            icon: flow.resource.icon ?? undefined, swatch: colours.get(flow.resource.id), value: formatAmount(raw!, trade.reply.decimals, trade.reply.decimals), short: true })),
          { label: measure === 'total' ? 'Delivered total' : measure === 'average' ? 'Average per trading day' : 'Total per day',
            value: formatAmount(measureAmount(flows, measure, from, at), trade.reply.decimals, trade.reply.decimals), strong: true }],
          note: values.length > listed.length ? `${usNumber.format(values.length - listed.length)} more trades` : undefined };
        }} />
    </div>
  </div>;
}

/** The texts a following trade view paints: its day and the coverage note at the head. */
export function tradeTexts(trade: TradeData, measure: Measure, from: number, known: readonly HourSpan[]) {
  return (at: number) => {
    const missingGroupHours = knownHours(trade.reply.known, from, at) - knownHours(known, from, at);
    const note = [coverageNote(trade.reply, measure, from, at), measure !== 'current' && missingGroupHours > 0 ? `Group membership is missing for ${formatDays(missingGroupHours)}.` : null]
      .filter(Boolean).join(' ');
    return new Map([['day', measure === 'current' ? dayLabel(at) : `${dayLabel(from)} – ${dayLabel(at)}`], ['note', note], ['caption-day', dayLabel(at)]]);
  };
}
