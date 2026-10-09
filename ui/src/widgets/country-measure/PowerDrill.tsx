import { useMemo } from 'react';
import DrillParts from '../../charts/drill/DrillParts';
import { dayLabel, drawBands, drawPlayhead } from '../../charts/draw';
import { isMember } from '../../dashboard/groups';
import { buildStack, stackExtent, stackedSegments } from '../../charts/stack';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer, ChartPick } from '../../charts/TimeChart';
import type { ChartTokens } from '../../charts/tokens';
import type { PlaybackMode } from '../../charts/view';
import { shownUntil } from '../../charts/view';
import type { PowerBalanceIndex } from '../../dashboard/powerBalance';
import { powerBalanceRows } from '../../dashboard/powerBalance';
import { fixed5Text } from '../../dashboard/ideaTooltip';
import { spendText } from '../../dashboard/politicsSpending';
import type { HourSpan } from '../../generated/dashboard';
import { powerPartHistory, powerPartsAt } from './powerDrillData';
import type { PowerBreakdown } from './powerDrillData';
import { powerLedgerModelKey } from './powerPickModel';

export default function PowerDrill({ index, pickOwner, presentationReady, mode, hour, visibleThrough, extent, within, countries, playback, branchStart,
  hoursPerSecond, active, tokens, onPick, onContextMenu }: {
  index: PowerBalanceIndex; pickOwner: unknown; presentationReady: boolean; mode: PowerBreakdown; hour: number | null; visibleThrough: number;
  extent: readonly [number, number]; within: readonly HourSpan[] | null; countries: readonly HourSpan[];
  playback: { playhead: number | null; mode: PlaybackMode; afterPlayhead: boolean; windowHours: number };
  branchStart: number; hoursPerSecond: number; active: boolean; tokens: ChartTokens;
  onPick?: (hour: number) => void; onContextMenu: (hour: number, x: number, y: number) => void;
}) {
  const model = useMemo(() => powerLedgerModelKey(index.reply), [index.reply]);
  const membership = useMemo(() => JSON.stringify(within), [within]);
  const identities = useMemo(() => JSON.stringify(countries), [countries]);
  const chartOwner = useMemo(() => ({ parent: pickOwner, actor: index.reply.tag, currency: index.currency, mode, model, membership, identities }),
    [pickOwner, index.reply.tag, index.currency, mode, model, membership, identities]);
  const partLabel = (label: string) => presentationReady || mode === 'income' ? label : 'Not identified';
  const layers = useMemo(() => powerPartHistory(index, mode, within, countries), [index, mode, within, countries]);
  const colors = useMemo(() => new Map(layers.map((layer, at) => [layer.id, tokens.categories[at % tokens.categories.length] ?? tokens.muted])), [layers, tokens]);
  const stack = useMemo(() => buildStack(layers.map(layer => layer.runs)), [layers]);
  const until = shownUntil(playback.playhead, playback.mode, playback.afterPlayhead);
  const chartLayers = useMemo<ChartLayer[]>(() => [{
    draw: (ctx, frame, chartTokens, state) => {
      const end = Math.min(frame.x[1], state.playhead ?? frame.x[1]);
      const resolution = (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth) / 2;
      for (const segment of stackedSegments(stack, frame.x[0], end, resolution)) {
        drawBands(ctx, frame, chartTokens, segment.hours, segment.bands, layers.map(layer => colors.get(layer.id)!));
      }
    },
    extent: x => stackExtent(stack, x[0], Math.min(x[1], until ?? x[1])),
  }], [stack, layers, colors, until]);
  const at = hour === null ? null : Math.floor(hour);
  const snapshot = at === null || !isMember(within ?? undefined, at) ? null : powerPartsAt(index, mode, at, visibleThrough, countries);
  const parts = (snapshot?.parts ?? []).filter(part => part.raw !== null && part.raw !== 0n).map(part => ({
    id: part.id, label: partLabel(part.label), value: Number(part.raw!) / 100000,
    text: mode === 'income' ? `${part.raw! > 0n ? '+' : ''}${fixed5Text(part.raw!, -1)}` : spendText(part.raw),
    color: colors.get(part.category), muted: part.category === 'income',
  }));
  const absolute = parts.reduce((sum, part) => sum + Math.abs(part.value), 0);
  const title = mode === 'income' ? 'Daily income and costs' : 'Spending that day';
  const totalLabel = mode === 'spending' ? 'Total costs' : index.currency === 'Command' && (snapshot?.total ?? 0n) < 0n ? 'Daily deficit' : 'Net daily income';

  return <div className="power-drill">
    {at === null ? <p className="drill-empty">Click the chart to choose a day.</p>
      : !snapshot ? <p className="drill-empty">This country was not shown at this hour.</p>
        : <>
          <DrillParts parts={parts} format={value => String(value)} total={absolute} showShares={snapshot.total !== null && !snapshot.reason}
            totalRow={{ label: totalLabel, value: spendText(snapshot.total) }} />
          {snapshot.reason && <p className="drill-empty">{presentationReady || mode === 'income' ? snapshot.reason : 'The recorded payment total is not known.'}</p>}
          {snapshot.parts.filter(part => part.raw === null).map(part => <p className="drill-empty" key={part.id}>{partLabel(part.label)}: Not recorded.</p>)}
          {(presentationReady ? snapshot.parts : []).filter(part => part.reason).map(part => <p className="drill-empty" key={`reason:${part.id}`}>{part.label}: {part.reason}</p>)}
          {parts.some(part => part.value < 0) && <p className="drill-empty">Shares use the absolute recorded components.</p>}
          {mode === 'income' && <p className="drill-empty">Income sources were not recorded. Other / not identified retains the exact native aggregate.</p>}
          {presentationReady ? <dl className="power-drill-details">{powerBalanceRows(index, at, visibleThrough).map((row, position) => <div key={position}>
            <dt>{row.label}</dt>{row.value && <dd>{row.value}</dd>}
          </div>)}</dl> : <p className="drill-empty">Content labels are loading; recorded amounts remain available.</p>}
        </>}
    <div className="power-drill-chart">
      <TimeChart label={`${title} over time`} extent={[extent[0], extent[1]]} playhead={playback.playhead} playback={playback.mode}
        afterPlayhead={playback.afterPlayhead} windowHours={playback.windowHours} branchStart={branchStart}
        hoursPerSecond={hoursPerSecond} axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }}
        active={active} layers={chartLayers} revision={`${mode}:${at}:${layers.length}`}
        keyHours={stack.hours} pickOwner={chartOwner} formatValue={value => String(Math.round(value * 100) / 100)}
        overlay={(ctx, frame, chartTokens) => { if (at !== null && at >= frame.x[0] && at <= frame.x[1]) drawPlayhead(ctx, frame, chartTokens, at); }}
        tooltip={shown => {
          if (!isMember(within ?? undefined, shown)) return { title: dayLabel(shown), rows: [{ label: 'Country outside the shown group', tone: 'muted' }] };
          const values = powerPartsAt(index, mode, shown, visibleThrough, countries);
          return { title: dayLabel(shown), rows: [...values.parts.map(part => ({ label: partLabel(part.label), swatch: colors.get(part.category),
            value: spendText(part.raw), tone: part.raw === null ? 'muted' as const : undefined })),
          { label: mode === 'spending' ? 'Total costs' : index.currency === 'Command' && (values.total ?? 0n) < 0n ? 'Daily deficit' : 'Net daily income', value: spendText(values.total), strong: true }], note: (values.reason && (!presentationReady && mode === 'spending' ? 'The recorded payment total is not known.' : values.reason)) ?? (onPick ? 'Click to show this day.' : undefined) };
        }} onPick={onPick ? (picked, context?: ChartPick) => { if (context?.frame) onPick(picked); } : undefined} onContextMenu={onContextMenu} />
    </div>
  </div>;
}
