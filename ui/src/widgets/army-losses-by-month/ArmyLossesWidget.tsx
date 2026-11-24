import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useMemo,  useRef } from 'react';
import WidgetState from '../../components/WidgetState';
import { WidgetCaption } from '../../dashboard/widgetFrame';
import { useContextMenu } from '../../components/ContextMenu';
import DrillHost from '../../charts/drill/DrillHost';
import { useDrill } from '../../charts/drill/useDrill';
import Legend from '../../charts/Legend';
import { CATEGORY_LIMIT } from '../../charts/palette';
import { columnAt, columnStops } from '../../charts/columns';
import { liveKey, valuesOf } from '../../charts/live';
import type { Live, LiveValues } from '../../charts/live';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import type { SortValue } from '../../charts/table/logic';
import { narrowTableOf } from '../../charts/table/options';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer, LiveHead } from '../../charts/TimeChart';
import { useChartTokens } from '../../charts/useChartTokens';
import { hatchStyle } from '../../charts/hatch';
import { jumpItem } from '../../charts/jump';
import type { TooltipRow } from '../../charts/tooltip';
import type { WidgetProps } from '../../dashboard/definitions';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useCountryNameAt } from '../../dashboard/reachedProduct';
import GateState from '../../dashboard/GateState';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useShownHour } from '../../dashboard/widgetPerf';
import { CHANNELS, LEDGERS, LEDGER_LABELS, OTHER, armyChartPresentation, armyColumnsLayer, armyName, chartView, designAmount, formatAmount, hatchKeyText, measureAvailabilityReason, monthArmies, monthBars, monthLabel, monthOf, monthTooltip,
  monthView, resolveArmyLosses, validateArmyLosses } from './armyLossesData';
import type { ArmyLossesData, EquipmentStack, Ledger, ManpowerStack, Measure, MonthArmy, MonthBars } from './armyLossesData';
import type { ArmyLossesReply } from './types';
import { LEDGER_CHOICES, OPTIONS } from './definition';
import './armyLosses.css';
import { SPEC, axisAt } from '../../dashboard/regionTime.ts';
import { useRegion, useRegionHourAt } from '../../dashboard/widgetTime.ts';

const armyKey = (row: MonthArmy) => row.army.reference;
const SORT = { column: 'army', direction: 'ascending' } as const;
const structureOf = (reply: ArmyLossesReply) => JSON.stringify([reply.absent, reply.armies, reply.manpower.map(row => row.key),
  reply.equipment.map(row => [row.key, row.design]), reply.designs, reply.emptyMonths, reply.unknownMonths, reply.availability]);

const DRILL_NEEDS = { holders: true };

export default function ArmyLossesWidget({ widget, runtime, active, setOption, onDiagnostics }: WidgetProps) {
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const shown = useShownHour();
  const following = widget.time.mode === 'follow';
  const mainRegion = useRegion(widget);
  const hourAt = useRegionHourAt(runtime, mainRegion);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const gate = useGate(runtime, widget, SPEC, active, { country: { stored: widget.options?.country, setOption } });
  const country = gate.country!;
  const hour = gate.hour;
  const tag = gate.open ? country.tag : null;
  const countryLabel = useCountryNameAt(runtime.client, track, tag ?? country.chosen, hour, active);
  const countryName = countryLabel.name;
  const nameDetail = countryLabel.nameFailure ? <span data-detail={countryLabel.nameFailure.message}>{countryName}</span> : undefined;
  const options = resolveOptions(OPTIONS, widget.options);
  const measure: Measure = options.measure === 'equipment' ? 'equipment' : 'manpower';
  const ledger = (LEDGER_CHOICES.find(choice => choice.value === options.ledger)?.value ?? 'combat') as Ledger;
  const stack = measure === 'manpower' ? (options.manpowerStack === 'armyGroup' ? 'armyGroup' : 'channel') as ManpowerStack
    : (options.equipmentStack === 'armyGroup' ? 'armyGroup' : 'type') as EquipmentStack;
  const tokens = useChartTokens();
  const state = useHeadProduct(runtime, gate, 'army-losses', tag ? { country: countryFromKey(tag), view: chartView(measure, ledger) } : null,
    { active, validate: validateArmyLosses, structure: structureOf });
  const reply = state.data;
  useDiagnostics(onDiagnostics, reply?.warnings ?? null);
  const first = widget.time.mode === 'range' && window ? Math.floor(window.firstHour) : track?.branch.firstHour ?? 0;
  const structureHour = state.at ?? hour;
  const data = useMemo(() => reply && structureHour !== null ? resolveArmyLosses(reply, structureHour, state.live.read()) : null, [reply, structureHour, state.live]);
  const bars = useMemo(() => data ? monthBars(data, measure, stack, ledger, first, data.hour, CATEGORY_LIMIT) : null,
    [data, measure, stack, ledger, first]);
  const liveData = useMemo<Live<{ data: ArmyLossesData; bars: MonthBars }> | null>(() => {
    if (!reply || structureHour === null) return null;
    let made: { at: number; cells: LiveValues<number>; value: { data: ArmyLossesData; bars: MonthBars } } | null = null;
    return {
      read: () => {
        const cells = state.live.read(), liveAt = following ? hourAt(axisAt(runtime.clock.getSnapshot().position)) : null;
        const at = liveAt == null ? structureHour : Math.floor(liveAt);
        if (!made || made.cells !== cells || made.at !== at) {
          const resolved = resolveArmyLosses(reply, at, cells);
          made = { at, cells, value: { data: resolved, bars: monthBars(resolved, measure, stack, ledger, first, at, CATEGORY_LIMIT) } };
        }
        shown(state.shownAt());
        return made.value;
      },
      subscribe: changed => {
        const stops = [state.live.subscribe(changed), runtime.clock.subscribeFrame(changed)];
        return () => { for (const stop of stops) stop(); };
      },
    };
  }, [reply, state.live, following, structureHour, hourAt, runtime.clock, measure, stack, ledger, first]);
  const colourOf = useMemo(() => new Map(bars?.parts.map((part, index) => [part.key, part.key === OTHER ? tokens.muted : tokens.categories[index % tokens.categories.length]]) ?? []),
    [bars, tokens]);
  const colour = useCallback((key: string) => colourOf.get(key) ?? tokens.muted, [colourOf, tokens]);
  const layers = useMemo<ChartLayer[]>(() => liveData ? [{
    draw: (ctx, frame, theme, drawn) => {
      const now = liveData.read().bars;
      armyColumnsLayer(now.periods, now.parts.map(part => colour(part.key))).draw(ctx, frame, theme, drawn);
    },
    extent: range => { const now = liveData.read().bars; return armyColumnsLayer(now.periods, now.parts.map(part => colour(part.key))).extent?.(range) ?? null; },
  }] : [], [liveData, colour]);
  const chartData = useMemo(() => liveData ? { revision: () => [liveData.read()], subscribe: liveData.subscribe } : undefined, [liveData]);
  const liveHead = useMemo<LiveHead | undefined>(() => following ? { read: () => hourAt(axisAt(runtime.clock.getSnapshot().position)) ?? Number.NaN, subscribe: runtime.clock.subscribeFrame } : undefined,
    [following, hourAt, runtime.clock]);
  const monthAt = (at: number) => { const now = liveData?.read().bars; return now?.periods[columnAt(now.periods, Math.floor(at))] ?? null; };
  const caption = measure === 'manpower' ? 'Manpower lost' : LEDGER_LABELS[ledger];
  const drillControl = useDrill(runtime, useRegion(widget), null, undefined, undefined, DRILL_NEEDS);
  const drill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const month = drill.breakdown === null ? null : Number(drill.breakdown);
  const drillTag = drillControl.gate.country?.tag ?? null;
  const drillState = useHeadProduct(runtime, drillControl.gate, 'army-losses', drillTag && month !== null ? { country: countryFromKey(drillTag), view: monthView(month) } : null,
    { active, validate: validateArmyLosses, structure: structureOf });
  const drillReply = drillState.data;
  const drillHour = drillState.at ?? hour;
  const anchor = useRef<HTMLDivElement>(null);
  const stateRoot = state.root, drillRoot = drillState.root;
  const rootRef = useCallback((element: HTMLDivElement | null) => {
    (anchor as { current: HTMLDivElement | null }).current = element; stateRoot(element); drillRoot(element);
  }, [stateRoot, drillRoot]);
  const menu = useContextMenu('Army losses actions');
  const pinned = following || widget.time.mode === 'pin';
  const pick = (at: number) => dispatch({ type: 'open', breakdown: String(monthOf(at)), hour: at, follow: false, country: gate.country?.holder ?? null });
  const kinds = LEDGERS[ledger];
  const drillData = useMemo(() => drillReply && drillHour !== null ? resolveArmyLosses(drillReply, drillHour, drillState.live.read()) : null, [drillReply, drillHour, drillState.live]);
  const drillRows = useMemo(() => drillData && month !== null ? monthArmies(drillData, month, kinds) : [], [drillData, month, kinds]);
  const drillNow = useMemo(() => {
    if (!drillReply || drillHour === null) return null;
    let made: { cells: LiveValues<number>; data: ArmyLossesData } | null = null;
    return () => {
      const cells = drillState.live.read();
      if (made?.cells !== cells) made = { cells, data: resolveArmyLosses(drillReply, drillHour, cells) };
      return made.data;
    };
  }, [drillReply, drillHour, drillState.live]);
  const drillLive = useMemo<Live<LiveValues<SortValue>> | undefined>(() => {
    if (!drillNow || month === null) return undefined;
    let made: { from: unknown; values: LiveValues<SortValue> } | null = null;
    return { subscribe: drillState.live.subscribe, read: () => {
      const now = drillNow();
      if (made?.from !== now) {
        const values = new Map<string, SortValue>();
        for (const row of monthArmies(now, month, kinds)) {
          if (row.amount !== null) values.set(liveKey(row.army.reference, 'amount'), Number(row.amount));
          row.losses?.forEach((value, index) => values.set(liveKey(row.army.reference, CHANNELS[index].key), value));
        }
        made = { from: now, values: valuesOf(values, 'Not recorded') };
      }
      return made.values;
    } };
  }, [drillNow, drillState.live, month, kinds]);
  const columns = useMemo<readonly TableColumn<MonthArmy>[]>(() => [
    { key: 'army', header: 'Army', label: 'Army', width: 'minmax(var(--table-name-min), 2fr)', value: row => row.army.ordinal,
      text: row => armyName(row.army), render: row => <span data-tooltip={row.army.reference}>{armyName(row.army)}</span> },
    ...(measure === 'manpower' ? CHANNELS.map((channel, index): TableColumn<MonthArmy> => ({ key: channel.key, header: channel.label, label: channel.label,
      numeric: true, width: 'max-content', priority: index + 1, value: row => row.losses?.[index] ?? null, paint: value => formatAmount(Number(value), 'manpower') })) : [
      { key: 'amount', header: caption, label: caption, numeric: true, width: 'max-content', value: row => row.amount,
        render: row => row.amount === null ? 'Not recorded' : designAmount(String(row.amount)), paint: value => designAmount(String(Math.round(Number(value)))) } satisfies TableColumn<MonthArmy>]),
  ], [measure, caption]);
  if (!following) shown(data ? data.hour : null);
  const unavailableReason = data ? measureAvailabilityReason(data, measure, ledger) : null;
  const presentation = bars ? armyChartPresentation(bars.periods, unavailableReason) : null;
  let main, charted = false, chartNote: string | null = null;
  if (!track) main = <WidgetState kind="empty" message="Load a campaign to see army losses." />;
  else if (!gate.open) main = <GateState gate={gate} loading="Loading losses…" choose="Choose a country to see its army losses."
    absent={nameDetail ? 'This country does not exist at this date.' : `${countryName} does not exist at this date.`} detail={nameDetail} />;
  else if (state.error) main = <WidgetState kind="error" failure={state.error} onRetry={state.retry} />;
  else if (!data || !bars || !presentation) main = <WidgetState kind="loading" label="Loading losses…" />;
  else if (data.absent) main = <WidgetState kind="unavailable" message={nameDetail ? 'This country does not exist at this date.' : `${countryName} does not exist at this date.`} detail={nameDetail} />;
  else if (presentation.kind === 'unavailable')
    main = <WidgetState kind="unavailable" message={presentation.message} detail={nameDetail} />;
  else if (presentation.kind === 'empty')
    main = <WidgetState kind="empty" message={`${nameDetail ? 'No recorded army losses' : `${countryName} has no recorded army losses`}${following ? ' yet' : ''}.`} detail={nameDetail} />;
  else {
    charted = true;
    chartNote = presentation.note;
    main = <div className="army-losses-chart">
      <TimeChart label={`${caption} per month`} extent={[first, widget.time.mode === 'range' && window ? Math.floor(window.lastHour) : track.branch.lastHour]}
        branchStart={track.branch.firstHour} playhead={pinned ? hour : null} liveHead={liveHead} playback={pinned ? 'draw-in' : 'cursor'}
        axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }} layers={layers} keyHours={columnStops(bars.periods)} data={chartData}
        fitValues={span => span ? [Math.min(0, span[0]), Math.max(0, span[1])] : null}
        active={active} revision={`${structureHour}:${measure}:${stack}:${ledger}:${tokens.categories.join()}`}
        tooltip={at => { const period = monthAt(at); return period && liveData ? monthTooltip(period, liveData.read().bars.parts, measure, caption, pinned ? liveData.read().data.hour : null, colour) : null; }}
        onPick={at => { const period = monthAt(at); if (period && !period.unknown) pick(Math.floor(at)); }}
        onContextMenu={(at, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, Math.floor(at))])} />
    </div>;
  }
  const legend = bars && bars.parts.length > 1 ? bars.parts.map(part => ({ id: part.key, label: part.label, color: colour(part.key) })) : [];
  const hatchKey = charted && bars ? hatchKeyText(bars.periods) : null;
  return <div ref={rootRef} className="army-losses" data-detail={countryLabel.nameFailure?.message}>{menu.menu}
    <WidgetCaption>{caption} per month</WidgetCaption>
    {country.chosen && country.error && country.historyHeld ? <WidgetState kind="error" failure={country.error} onRetry={country.retry} /> : null}
    <DrillHost drill={drillControl} runtime={runtime} subject={month !== null} mode={options.drill === 'popout' ? 'popout' : 'inline'} title={month === null ? 'Month' : monthLabel(month)}
      day={month === null ? '' : monthLabel(month)} canFollow={false} anchor={anchor}
      onClose={() => dispatch({ type: 'close' })} main={<div className="army-losses-body">{main}{legend.length > 0 && <Legend label={caption} items={legend} />}
        {hatchKey && <p className="army-losses-key"><i aria-hidden="true" style={hatchStyle(tokens)} />{hatchKey}</p>}
        {chartNote && <p className="army-losses-key" role="status">{chartNote}</p>}</div>}>
      {month !== null && track && <div className="army-losses-drill">
        {drillState.error ? <WidgetState kind="error" failure={drillState.error} onRetry={drillState.retry} />
          : !drillData ? <WidgetState kind="loading" label="Loading the month…" />
          : drillRows.length ? <Table rowRevision={row => [row, columns]} label={`${caption} in ${monthLabel(month)}`} rows={drillRows} columns={columns} rowKey={armyKey} sort={SORT}
          cellBars={false} rankMarkers={false} narrow={narrowTableOf(options)} live={drillLive}
          tooltip={row => { const now = drillNow ? monthArmies(drillNow(), month, kinds).find(item => item.army.reference === row.army.reference) ?? row : row;
            return { title: armyName(now.army), rows: [
              ...(now.losses ? CHANNELS.map((channel, index) => ({ label: channel.label, value: formatAmount(now.losses![index], 'manpower') })) : [{ label: 'Manpower', value: 'Not recorded' }]),
              ...(['combat', 'attrition', 'recovered', 'enemy'] as const).flatMap((name): TooltipRow[] => {
                if (now.unknown.some(kind => LEDGERS[name].includes(kind))) return [{ label: LEDGER_LABELS[name], value: 'Not recorded', strong: true, divider: true }];
                const designs = now.designs.filter(design => LEDGERS[name].includes(design.ledger));
                return designs.length ? [{ label: LEDGER_LABELS[name], strong: true, divider: true }, ...designs.map(design => ({ label: design.name,
                  value: design.amountRaw === null ? 'Not recorded' : designAmount(design.amountRaw), indent: true }))] : [];
              }),
            ] }; }}
          menu={() => [jumpItem(runtime, track.id, drill.hour ?? drillData.hour)]} />
          : <WidgetState kind="empty" message="No army kept an entry for this month." />}
      </div>}
    </DrillHost>
  </div>;
}
