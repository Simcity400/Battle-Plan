import { useEffect, useMemo, useRef, useState } from 'react';
import DrillParts from '../../charts/drill/DrillParts';
import { dayLabel, drawBands, drawPlayhead } from '../../charts/draw';
import { isMember } from '../../dashboard/groups';
import { buildStack, stackExtent, stackedSegments, compactStackExtent, forEachCompactStackSegment } from '../../charts/stack';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer, ChartPick } from '../../charts/TimeChart';
import type { ChartTokens } from '../../charts/tokens';
import type { PlaybackMode } from '../../charts/view';
import { shownUntil } from '../../charts/view';
import type { PowerBalanceIndex } from '../../dashboard/powerBalance';
import { formatPower, formatRate } from '../../dashboard/powerDecimals';
import type { HourSpan } from '../../generated/dashboard';
import { powerActionDetails, powerPartHistory, powerPartsAt } from './powerDrillData';
import type { PowerBreakdown } from './powerDrillData';
import { powerLedgerModelKey } from './powerPickModel';
import { usePreparedPowerParts } from './preparedPowerParts';
import { summaryStack } from '../../charts/chartSummary';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import { useParityRows } from '../../dashboard/parityRows';

export default function PowerDrill({ client, source, branchId, bindings, countryId, index, digits = 0, pickOwner, presentationReady, mode, hour, visibleThrough, extent, within, countries, playback, branchStart,
  hoursPerSecond, active, tokens, onPick, onContextMenu }: {
  client?: DashboardClient; source?: DashboardSource; branchId?: string; bindings?: ReadonlyMap<string, readonly HourSpan[]>; countryId?: number;
  index: PowerBalanceIndex; digits?: number; pickOwner: unknown; presentationReady: boolean; mode: PowerBreakdown; hour: number | null; visibleThrough: number;
  extent: readonly [number, number]; within: readonly HourSpan[] | null; countries: readonly HourSpan[];
  playback: { playhead: number | null; mode: PlaybackMode; afterPlayhead: boolean; windowHours: number };
  branchStart: number; hoursPerSecond: number; active: boolean; tokens: ChartTokens;
  onPick?: (hour: number) => void; onContextMenu: (hour: number, x: number, y: number) => void;
}) {
  const prepared = usePreparedPowerParts(client, source, branchId, index.reply.tag, countryId, index.currency, mode, bindings, extent, hour, visibleThrough, active);
  const root = useRef<HTMLDivElement>(null);
  const navigation = useRef<AbortController | null>(null);
  const navigationOwner = useRef(prepared.owner); navigationOwner.current = prepared.owner;
  const [keyboard, setKeyboard] = useState<{ owner: typeof prepared.owner; hour: number | null; revision: number }>();
  useEffect(() => () => navigation.current?.abort(), [prepared.owner]);
  const model = useMemo(() => powerLedgerModelKey(index.reply), [index.reply]);
  const membership = useMemo(() => JSON.stringify(within), [within]);
  const identities = useMemo(() => JSON.stringify(countries), [countries]);
  const chartModel = prepared.enabled ? prepared.owner : model;
  const chartOwner = useMemo(() => ({ parent: pickOwner, actor: index.reply.tag, currency: index.currency, mode, model: chartModel, membership, identities }),
    [pickOwner, index.reply.tag, index.currency, mode, chartModel, membership, identities]);
  const partLabel = (label: string) => presentationReady || mode === 'income' ? label : 'Loading…';
  // Daily figures are rates with the game's two decimals; payments are whole amounts.
  const amount = (raw: bigint | null) => raw === null ? 'Not recorded' : mode === 'income' ? formatRate(raw, digits) : formatPower(raw, digits);
  const totalOf = (total: bigint | null) => mode === 'spending' ? 'Spent that day'
    : index.currency === 'Political' ? 'Daily change' : (total ?? 0n) < 0n ? 'Daily deficit' : 'Daily gain';
  const legacyLayers = useMemo(() => prepared.enabled ? [] : powerPartHistory(index, mode, within, countries), [prepared.enabled, index, mode, within, countries]);
  const summaries = prepared.summaries ?? [];
  const total = summaries.find(part => part.id === `${mode}-total`);
  const partsShown = mode === 'income' ? ['income', 'costs', 'wrapping'].flatMap(id => summaries.filter(part => part.id === id))
    : summaries.filter(part => part.id !== `${mode}-total`);
  const layers = prepared.enabled ? partsShown.map(part => ({ id: part.id, label: part.label, runs: part.summary.runs?.map(run => ({ ...run, startsSegment: run.breakBefore })) ?? [] })) : legacyLayers;
  const colors = useMemo(() => new Map(layers.map((layer, at) => [layer.id, tokens.categories[at % tokens.categories.length] ?? tokens.muted])), [layers, tokens]);
  const stack = useMemo(() => buildStack(layers.map(layer => layer.runs)), [layers]);
  const dense = useMemo(() => prepared.enabled && total && partsShown.length && partsShown.some(part => part.summary.runs === undefined)
    ? summaryStack(partsShown.slice(0, -1).map(part => part.summary), total.summary) : null, [prepared.enabled, total, prepared.summaries]); // eslint-disable-line react-hooks/exhaustive-deps
  const until = shownUntil(playback.playhead, playback.mode, playback.afterPlayhead);
  const chartLayers = useMemo<ChartLayer[]>(() => [{
    draw: (ctx, frame, chartTokens, state) => {
      const end = Math.min(frame.x[1], state.playhead ?? frame.x[1]);
      if (prepared.enabled && (!total || total.summary.from !== frame.x[0] || total.summary.to !== end || total.summary.width !== Math.ceil(frame.plotWidth))) return;
      const resolution = (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth) / 2;
      if (dense) {
        forEachCompactStackSegment(dense.model, frame.x[0], end, resolution, segment =>
          drawBands(ctx, frame, chartTokens, segment.hours, segment.bands, layers.map(layer => colors.get(layer.id)!)), dense.sample);
        return;
      }
      for (const segment of stackedSegments(stack, frame.x[0], end, resolution)) {
        drawBands(ctx, frame, chartTokens, segment.hours, segment.bands, layers.map(layer => colors.get(layer.id)!));
      }
    },
    extent: x => dense ? compactStackExtent(dense.model, x[0], Math.min(x[1], until ?? x[1])) : stackExtent(stack, x[0], Math.min(x[1], until ?? x[1])),
  }], [stack, layers, colors, until, dense, prepared.enabled, total]);
  const at = hour === null ? null : Math.floor(hour);
  const exactIndex = at === null ? undefined : prepared.enabled ? prepared.at(at) : index;
  const snapshot = at === null || !exactIndex || !isMember(within ?? undefined, at) ? null : powerPartsAt(exactIndex, mode, at, visibleThrough, countries);
  const parts = (snapshot?.parts ?? []).filter(part => part.raw === null || part.raw !== 0n).map(part => ({
    id: part.id, label: partLabel(part.label), value: part.raw === null ? null : Number(part.raw) / 100000,
    text: amount(part.raw),
    color: colors.get(part.category), muted: part.category === 'income',
  }));
  const absolute = parts.reduce((sum, part) => sum + Math.abs(part.value ?? 0), 0);
  const title = mode === 'income' ? 'Daily income and costs' : 'Spending that day';

  useParityRows(root, () => ({ kind: 'chart', label: `${index.currency} income and causes`, scope: 'displayed-hour', hour: at,
    complete: !!snapshot, total: snapshot?.parts.length ?? 0, rows: (snapshot?.parts ?? []).map(part => ({ id: part.id, raw: part.raw?.toString() ?? null, reason: part.reason ?? null })),
    amountRaw: snapshot?.total?.toString() ?? null, reason: snapshot?.reason ?? null }));
  return <div className="power-drill" ref={root}>
    {at === null ? <p className="drill-empty">Choose a day.</p>
      : prepared.enabled && !exactIndex ? <p className="drill-empty">Loading income and causes...</p>
      : !snapshot ? <p className="drill-empty">This country was not shown at this hour.</p>
        : <>
          <DrillParts parts={parts} format={value => String(value)} total={absolute} showShares={mode === 'spending' && snapshot.total !== null && !snapshot.reason}
            totalRow={{ label: totalOf(snapshot.total), value: snapshot.reason ? `${amount(snapshot.total)} · ${snapshot.reason}` : amount(snapshot.total) }} />
          {presentationReady ? <dl className="power-drill-details">{powerActionDetails(exactIndex!, at, visibleThrough, digits).map((row, position) => <div key={position}>
            <dt>{row.label}</dt>{row.value && <dd>{row.value}</dd>}
          </div>)}</dl> : <p className="drill-empty">Loading names…</p>}
        </>}
    <div className="power-drill-chart">
      <TimeChart label={`${title} over time`} extent={[extent[0], extent[1]]} playhead={playback.playhead} playback={playback.mode}
        afterPlayhead={playback.afterPlayhead} windowHours={playback.windowHours} branchStart={branchStart}
        hoursPerSecond={hoursPerSecond} axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }}
        active={active} layers={chartLayers} revision={`${mode}:${at}:${layers.length}`}
        onSummaryView={prepared.enabled ? prepared.onSummaryView : undefined} onHover={prepared.enabled ? prepared.onHover : undefined}
        onNavigateHour={prepared.enabled ? (key, from, bounds, cutoff) => {
          navigation.current?.abort(); const controller = new AbortController(); navigation.current = controller; const owner = prepared.owner;
          void prepared.navigate(key, from, bounds, cutoff, controller.signal).then(hour => {
            if (!controller.signal.aborted && navigationOwner.current === owner) { prepared.onHover(hour); setKeyboard(previous => ({ owner, hour, revision: (previous?.revision ?? 0) + 1 })); }
          }).catch(() => { if (!controller.signal.aborted) prepared.retry(); });
        } : undefined}
        keyboardHour={keyboard?.owner === prepared.owner ? keyboard.hour : undefined} keyboardRevision={keyboard?.owner === prepared.owner ? keyboard.revision : undefined}
        keyHours={prepared.enabled ? undefined : stack.hours} pickOwner={chartOwner} formatValue={value => String(Math.round(value * 100) / 100)}
        overlay={(ctx, frame, chartTokens) => { if (at !== null && at >= frame.x[0] && at <= frame.x[1]) drawPlayhead(ctx, frame, chartTokens, at); }}
        tooltip={shown => {
          if (!isMember(within ?? undefined, shown)) return { title: dayLabel(shown), rows: [{ label: 'Country outside the shown group', tone: 'muted' }] };
          const reached = prepared.enabled ? prepared.at(shown) : index;
          if (!reached) return { title: dayLabel(shown), rows: [{ label: 'Loading income and causes...', tone: 'muted' }] };
          const values = powerPartsAt(reached, mode, shown, visibleThrough, countries);
          return { title: dayLabel(shown), rows: [...values.parts.map(part => ({ label: partLabel(part.label), swatch: colors.get(part.category),
            value: amount(part.raw), tone: part.raw === null ? 'muted' as const : undefined })),
          { label: totalOf(values.total), value: amount(values.total), strong: true }], note: values.reason };
        }} onPick={onPick ? (picked, context?: ChartPick) => { if (context?.frame) onPick(picked); } : undefined} onContextMenu={onContextMenu} />
    </div>
    {prepared.error && <p className="drill-empty" role="alert">Income and causes could not be loaded. <button type="button" onClick={prepared.retry}>Retry</button></p>}
  </div>;
}
