import { useCallback, useMemo } from 'react';
import ArtFallback from '../../components/ArtFallback';
import DrillParts from '../../charts/drill/DrillParts';
import type { DrillPart, LiveDrillParts } from '../../charts/drill/DrillParts';
import { mapLive } from '../../charts/live';
import LiveText from '../../charts/LiveText';
import type { LiveHour } from './TradeDrillView';
import { dayLabel, drawBands, drawPlayhead, drawValueRule } from '../../charts/draw';
import Legend from '../../charts/Legend';
import type { LegendItem } from '../../charts/Legend';
import { compactStack, compactStackExtent, forEachCompactStackSegment, forEachJoinedSegment } from '../../charts/stack';
import { heldStackSegment } from '../../charts/race/countryRace';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import type { ChartTokens } from '../../charts/tokens';
import type { TooltipRow } from '../../charts/tooltip';
import { formatAmount, formatAxisValue } from '../../charts/valueAxis';
import { MAX_NAME_HOURS } from '../../dashboard/countryFacts';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { lookHour } from '../../dashboard/countryLooks';
import type { NameAt } from '../../dashboard/countryNamesAt';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import type { HourSpan, ResourceInfo } from '../../generated/dashboard';
import { contributionHistory, contributionMeasureExtent, contributionMeasureModel, forEachContributionMeasureSegment } from './tradeDrill';
import { bandGroups, countryParts, countryTotal, countryTrade, divergingExtent, mergedBands, mirrored, partnerFact } from './tradeCountry';
import type { CountryPart, TradeDirection } from './tradeCountry';
import { coverageNote, knownSpanAt, measureNote, resourceName } from './tradeData';
import type { Measure, TradeCountryFact, TradeData } from './tradeData';
import { motionOf } from '../../charts/easing.ts';
import { useChangedState } from '../../dashboard/changedState.ts';

const PLAIN_ROWS = 40;
const DIRECTIONS: readonly TradeDirection[] = ['imports', 'exports'];
const TITLES: Record<TradeDirection, string> = { imports: 'Imports', exports: 'Exports' };
const sameIds = (a: readonly string[], b: readonly string[]) => a === b || a.length === b.length && a.every((id, index) => id === b[index]);

export function resourceLegendItems(resources: readonly ResourceInfo[], colours: ReadonlyMap<number, string>, other: string): LegendItem[] {
  return resources.map(item => ({ id: String(item.id), label: resourceName(item), color: colours.get(item.id) ?? other,
    flag: item.icon ? <img className="trade-resource-icon" src={item.icon} alt="" /> : <ArtFallback className="trade-resource-icon" icon="layers" name={resourceName(item)} /> }));
}

export default function TradeCountryView({ holder, trade, resources, track, runtime, measure, from, hour, live, extent, colours, tokens, active, onPick, onContextMenu }: {
  holder: string; trade: TradeData; resources: ReadonlySet<number> | null; track: BoundTrack; runtime: WidgetRuntime; measure: Measure; from: number;
  hour: number | null; live: LiveHour | null; extent: [number, number];
  colours: ReadonlyMap<number, string>; tokens: ChartTokens; active: boolean; onPick: (hour: number) => void;
  onContextMenu: (hour: number, x: number, y: number) => void;
}) {
  const known: readonly HourSpan[] = trade.reply.known;
  const decimals = trade.reply.decimals;
  const scale = 10 ** decimals;
  const country = useMemo(() => countryTrade(trade, holder, resources), [trade, holder, resources]);
  const [visible, setVisible] = useChangedState<Record<TradeDirection, readonly string[]>>({ imports: [], exports: [] });
  const onVisible = useMemo(() => ({
    imports: (ids: readonly string[]) => setVisible(before => sameIds(before.imports, ids) ? before : { ...before, imports: ids }),
    exports: (ids: readonly string[]) => setVisible(before => sameIds(before.exports, ids) ? before : { ...before, exports: ids }),
  }), []);
  const partsAt = useCallback((at: number) => Object.fromEntries(DIRECTIONS.map(direction => [direction, {
    total: countryTotal(country[direction], measure, from, at, known),
    delivered: measure === 'total' ? null : countryTotal(country[direction], 'total', from, at, known),
    partners: countryParts(country[direction], direction, 'partner', measure, from, at, known),
    resources: countryParts(country[direction], direction, 'resource', measure, from, at, known),
  }])) as Record<TradeDirection, { total: bigint | null; delivered: bigint | null; partners: CountryPart[]; resources: CountryPart[] }>,
  [country, measure, from, known]);
  const parts = useMemo(() => hour == null ? null : partsAt(hour), [hour, partsAt]);

  const factHour = useCallback((fact: TradeCountryFact) => lookHour(track.branch, fact.hour, fact.firstHour)?.hour ?? fact.hour, [track.branch]);
  const facts = useMemo(() => {
    const dated = new Map<string, TradeCountryFact | null>();
    if (parts && hour != null) for (const direction of DIRECTIONS) for (const part of parts[direction].partners) dated.set(`${direction}:${part.id}`, partnerFact(part, direction, hour));
    return dated;
  }, [parts, hour]);
  const wanted = useMemo<NameAt[]>(() => DIRECTIONS.flatMap(direction => visible[direction].flatMap(id => {
    const fact = facts.get(`${direction}:${id}`);
    return fact ? [{ tag: fact.tag, hour: factHour(fact) }] : [];
  })), [visible, facts, factHour]);
  const labels = useCountryLabelsAt(runtime.client, track, wanted, active, Math.max(MAX_NAME_HOURS, new Set(wanted.map(item => item.hour)).size));

  const resourceInfo = useMemo(() => new Map(trade.reply.resources.map(item => [String(item.id), item])), [trade]);
  const amount = useCallback((raw: bigint) => formatAmount(raw, decimals), [decimals]);
  const rowsOf = useCallback((parts: ReturnType<typeof partsAt>) => Object.fromEntries(DIRECTIONS.map(direction => {
    const side = parts[direction];
    const total = side.total == null ? null : Number(side.total) / scale;
    const shown = new Set(visible[direction]);
    const partners = side.partners.map((part): DrillPart => {
      const fact = facts.get(`${direction}:${part.id}`);
      const label = fact ? labels.labelAt(factHour(fact))(fact.tag) : null;
      return { id: part.id, label: label?.name ?? 'Country name unavailable', raw: part.raw, value: Number(part.raw) / scale, text: amount(part.raw),
        countryFlag: label && shown.has(part.id) ? label : fact ? {} : undefined, muted: part.raw === 0n };
    });
    const byResource = side.resources.map((part): DrillPart => {
      const item = resourceInfo.get(part.id);
      return { id: part.id, label: item ? resourceName(item) : 'Resource name unavailable', raw: part.raw, value: Number(part.raw) / scale,
        text: amount(part.raw), icon: item?.icon ?? undefined, color: colours.get(Number(part.id)) ?? tokens.muted, muted: part.raw === 0n };
    });
    return [direction, { total, partners, byResource }];
  })) as Record<TradeDirection, { total: number | null; partners: DrillPart[]; byResource: DrillPart[] }>,
  [visible, facts, labels.labelAt, factHour, amount, scale, resourceInfo, colours, tokens.muted]);
  const rows = useMemo(() => parts && rowsOf(parts), [parts, rowsOf]);
  const liveRows = useMemo(() => live ? mapLive(live, at => rowsOf(partsAt(at))) : null, [live, rowsOf, partsAt]);
  const liveList = useMemo(() => {
    if (!liveRows) return null;
    const list = (direction: TradeDirection, which: 'partners' | 'byResource'): LiveDrillParts => mapLive(liveRows, rows => ({ parts: rows[direction][which], total: rows[direction].total }));
    return { imports: { partners: list('imports', 'partners'), byResource: list('imports', 'byResource') }, exports: { partners: list('exports', 'partners'), byResource: list('exports', 'byResource') } };
  }, [liveRows]);

  const stacks = useMemo(() => measure === 'current' ? {
    imports: compactStack(country.imports.map(flow => contributionHistory(flow, known))),
    exports: compactStack(country.exports.map(flow => contributionHistory(flow, known))),
  } : null, [country, known, measure]);
  const measured = useMemo(() => measure === 'current' ? null : {
    imports: contributionMeasureModel(country.imports, measure, from, known),
    exports: contributionMeasureModel(country.exports, measure, from, known),
  }, [country, measure, from, known]);
  const bands = useMemo(() => Object.fromEntries(DIRECTIONS.map(direction => {
    const groups = bandGroups(country[direction].map(flow => flow.resource.id));
    return [direction, { groups, colors: groups.map(([first]) => colours.get(country[direction][first].resource.id) ?? tokens.muted) }];
  })) as Record<TradeDirection, { groups: [number, number][]; colors: string[] }>, [country, colours, tokens.muted]);
  const layers = useMemo<ChartLayer[]>(() => [{
    draw: (ctx, frame, chartTokens, state) => {
      const end = Math.min(frame.x[1], state.playhead ?? frame.x[1]);
      const minHours = (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth);
      for (const direction of DIRECTIONS) {
        const { groups, colors } = bands[direction];
        const place = <T extends { bands: { low: number[]; high: number[] }[] }>(piece: T) => {
          const merged = mergedBands(piece, groups);
          return direction === 'imports' ? mirrored(merged) : merged;
        };
        if (stacks) forEachJoinedSegment(piece => forEachCompactStackSegment(stacks[direction], frame.x[0], end, minHours, segment => piece(heldStackSegment(segment))),
          held => drawBands(ctx, frame, chartTokens, held.hours, place(held).bands, colors, held));
        else if (measured) forEachJoinedSegment(piece => forEachContributionMeasureSegment(measured[direction], frame.x[0], end, minHours, piece),
          piece => drawBands(ctx, frame, chartTokens, piece.hours, place(piece).bands, colors, piece));
      }
      drawValueRule(ctx, frame, chartTokens, 0);
    },
    extent: x => stacks ? divergingExtent(compactStackExtent(stacks.exports, x[0], x[1]), compactStackExtent(stacks.imports, x[0], x[1]))
      : measured ? divergingExtent(contributionMeasureExtent(measured.exports, x[0], x[1]), contributionMeasureExtent(measured.imports, x[0], x[1])) : null,
  }], [stacks, measured, bands]);
  const keyHours = useMemo(() => {
    const hours = stacks ? [...stacks.imports.hours, ...stacks.exports.hours] : measured ? [...measured.imports.hours, ...measured.exports.hours] : [];
    return [...new Set(hours)].sort((a, b) => a - b);
  }, [stacks, measured]);

  const shownResources = useMemo(() => {
    const ids = new Set([...country.imports, ...country.exports].map(flow => flow.resource.id));
    return trade.reply.resources.filter(item => ids.has(item.id));
  }, [country, trade]);
  const legend = useMemo(() => resourceLegendItems(shownResources, colours, tokens.muted), [shownResources, colours, tokens.muted]);
  const totalLabel = measure === 'total' ? 'Delivered' : measure === 'average' ? 'Average per trading day' : 'Per day';
  const note = hour == null || live ? undefined : coverageNote(trade.reply, measure, from, hour);
  const tooltipRows = useCallback((at: number): TooltipRow[] => DIRECTIONS.flatMap((direction, index) => {
    const total = countryTotal(country[direction], measure, from, at, known);
    const byResource = countryParts(country[direction], direction, 'resource', measure, from, at, known)
      .sort((a, b) => (trade.resourceOrder.get(Number(a.id)) ?? 0) - (trade.resourceOrder.get(Number(b.id)) ?? 0));
    return [{ label: TITLES[direction], value: total == null ? 'Not recorded' : amount(total), strong: true, divider: index > 0 },
      ...byResource.map(part => {
        const item = resourceInfo.get(part.id);
        return { label: item ? resourceName(item) : 'Resource name unavailable', icon: item?.icon ?? undefined,
          swatch: colours.get(Number(part.id)), value: amount(part.raw), indent: true, short: true };
      })];
  }), [country, measure, from, known, trade.resourceOrder, resourceInfo, colours, amount]);

  const empty = (direction: TradeDirection) => `No ${direction} ${measure === 'current' ? 'on this day' : 'in this time'}.`;
  return <div className="trade-country">
    <p className="drill-empty">{measureNote(measure)}{live ? <> · <LiveText live={live.texts} at="day" /></> : hour != null ? (measure === 'current' ? ` · ${dayLabel(hour)}` : ` · ${dayLabel(from)} – ${dayLabel(hour)}`) : ''}
      {resources ? ' · Chosen resources only' : ''}.</p>
    {hour == null || !parts || !rows ? <p className="drill-empty">Choose a day.</p> : liveList && live ? <div className="trade-country-sides">
      {DIRECTIONS.map(direction => <section key={direction} className="trade-country-side" aria-label={TITLES[direction]}>
        <header><strong>{TITLES[direction]}</strong><span className="drill-day">{totalLabel}</span><b><LiveText live={live.texts} at={`${direction}:total`} /></b></header>
        <p className="drill-empty"><LiveText live={live.texts} at={`${direction}:state`} /></p>
        <DrillParts parts={rows[direction].byResource} live={liveList[direction].byResource} positiveBars signedShares active={active} total={rows[direction].total} format={String} />
        <DrillParts parts={rows[direction].partners} live={liveList[direction].partners} positiveBars signedShares active={active} total={rows[direction].total} format={String}
          virtual={rows[direction].partners.length > PLAIN_ROWS} onVisible={onVisible[direction]} />
      </section>)}
    </div> : <div className="trade-country-sides">
      {DIRECTIONS.map(direction => {
        const side = parts[direction];
        const shown = rows[direction];
        return <section key={direction} className="trade-country-side" aria-label={TITLES[direction]}>
          <header><strong>{TITLES[direction]}</strong><span className="drill-day">{totalLabel}</span>
            <b>{side.total == null ? 'Not recorded' : amount(side.total)}</b></header>
          {side.delivered != null && <p className="drill-empty">{amount(side.delivered)} delivered {dayLabel(from)} – {dayLabel(hour)}</p>}
          {side.total == null ? <p className="drill-empty">Trade deals were not recorded {measure === 'current' ? 'on this day' : 'in this time'}.</p>
            : !shown.byResource.length ? <p className="drill-empty">{empty(direction)}</p> : <>
              <DrillParts parts={shown.byResource} positiveBars signedShares active={active} total={shown.total} format={String} />
              <DrillParts parts={shown.partners} positiveBars signedShares active={active} total={shown.total} format={String}
                virtual={shown.partners.length > PLAIN_ROWS} onVisible={onVisible[direction]} />
            </>}
        </section>;
      })}
    </div>}
    {live ? <p className="drill-empty"><LiveText live={live.texts} at="note" /></p> : note && <p className="drill-empty">{note}</p>}
    <Legend always label="Resources" items={legend} windowed={legend.length > tokens.categories.length} />
    <p className="drill-empty">{measureNote(measure)} over time: exports above the line, imports below it.</p>
    <div className="trade-drill-chart">
      <TimeChart label={`${measureNote(measure)} over time, exports above and imports below, by resource`} extent={extent} branchStart={track.branch.firstHour}
        playhead={live ? live.hour() : hour} liveHead={live ?? undefined} playback="cursor" afterPlayhead hoursPerSecond={motionOf(runtime.clock.getSnapshot())}
        axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }} layers={layers} keyHours={keyHours}
        formatValue={(value, ticks) => formatAxisValue(Math.abs(value), ticks.map(Math.abs))}
        active={active} revision={stacks ?? measured} onPick={onPick} onContextMenu={onContextMenu}
        overlay={live ? undefined : (ctx, frame, chartTokens) => { if (hour != null) drawPlayhead(ctx, frame, chartTokens, hour); }}
        tooltip={at => knownSpanAt(known, at) < 0 && measure === 'current'
          ? { title: dayLabel(at), rows: [{ label: 'Trade', value: 'Not recorded', tone: 'muted' }] }
          : { title: dayLabel(at), rows: tooltipRows(at) }} />
    </div>
  </div>;
}

export function countryTexts(trade: TradeData, holder: string, resources: ReadonlySet<number> | null, measure: Measure, from: number) {
  const country = countryTrade(trade, holder, resources);
  const known = trade.reply.known;
  return (at: number) => {
    const texts = new Map<string, string>();
    for (const direction of DIRECTIONS) {
      const total = countryTotal(country[direction], measure, from, at, known);
      const delivered = measure === 'total' ? null : countryTotal(country[direction], 'total', from, at, known);
      const traded = countryParts(country[direction], direction, 'resource', measure, from, at, known).length > 0;
      texts.set(`${direction}:total`, total == null ? 'Not recorded' : formatAmount(total, trade.reply.decimals));
      texts.set(`${direction}:state`, total == null ? `Trade deals were not recorded ${measure === 'current' ? 'on this day' : 'in this time'}.`
        : !traded ? `No ${direction} ${measure === 'current' ? 'on this day' : 'in this time'}.`
        : delivered != null ? `${formatAmount(delivered, trade.reply.decimals)} delivered ${dayLabel(from)} – ${dayLabel(at)}` : '');
    }
    return texts;
  };
}
