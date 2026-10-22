import { useCallback, useMemo, useReducer, useRef } from 'react';
import WidgetState from '../../components/WidgetState';
import { useContextMenu } from '../../components/ContextMenu';
import { Segmented } from '../../components/Segmented';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, reduceDrill } from '../../charts/drill/drillState';
import Legend from '../../charts/Legend';
import { CATEGORY_LIMIT } from '../../charts/palette';
import { columnAt, columnStops, columnsLayer } from '../../charts/columns';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf } from '../../charts/table/options';
import { useTableHour } from '../../charts/table/useTableHour';
import TimeChart from '../../charts/TimeChart';
import { useChartTokens } from '../../charts/useChartTokens';
import { jumpItem } from '../../charts/jump';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useCountryNameAt, useReachedCountry, useReachedProduct } from '../../dashboard/reachedProduct';
import ReachedCountryChoice from '../../dashboard/ReachedCountryChoice';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { CHANNELS, LEDGERS, LEDGER_LABELS, OTHER, armyName, designAmount, formatAmount, monthArmies, monthBars, monthLabel, monthOf, monthTooltip, validateArmyLosses } from './armyLossesData';
import type { EquipmentStack, Ledger, ManpowerStack, Measure, MonthArmy } from './armyLossesData';
import { LEDGER_CHOICES, MEASURES, OPTIONS } from './definition';
import './armyLosses.css';

const armyKey = (row: MonthArmy) => row.army.reference;
const SORT = { column: 'army', direction: 'ascending' } as const;
/** Monthly bars keep their wider gaps, and a month's small losses still show beside large ones. */
const BAR_STYLE = { spacing: 'proportional', thinBands: true } as const;

export default function ArmyLossesWidget({ widget, runtime, active, setOption, onDiagnostics }: WidgetProps) {
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const playback = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const window = track && resolveWindow(widget, track, runtime.tracks, playback);
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const country = useReachedCountry(widget, runtime, hour, active, widget.options?.country, setOption);
  const tag = country.tag;
  const countryName = useCountryNameAt(runtime.client, track, tag ?? country.chosen, hour, active);
  const options = resolveOptions(OPTIONS, widget.options);
  const measure: Measure = options.measure === 'equipment' ? 'equipment' : 'manpower';
  const ledger = (LEDGER_CHOICES.find(choice => choice.value === options.ledger)?.value ?? 'combat') as Ledger;
  const stack = measure === 'manpower' ? (options.manpowerStack === 'armyGroup' ? 'armyGroup' : 'channel') as ManpowerStack
    : (options.equipmentStack === 'armyGroup' ? 'armyGroup' : 'type') as EquipmentStack;
  const tokens = useChartTokens();
  const request = track && window && tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'army-losses',
    params: { branchId: track.branch.id, hour, tag } } : null;
  const state = useReachedProduct(runtime.client, track?.source, request, JSON.stringify([track?.branch.id, country.chosen]), active, validateArmyLosses);
  const data = state.data;
  useDiagnostics(onDiagnostics, data?.warnings ?? null);
  const first = widget.time.mode === 'range' && window ? Math.floor(window.firstHour) : track?.branch.firstHour ?? 0;
  const bars = useMemo(() => data ? monthBars(data, measure, stack, ledger, first, Math.min(hour, data.hour), CATEGORY_LIMIT) : null,
    [data, measure, stack, ledger, first, hour]);
  const colour = useCallback((key: string) => {
    if (key === OTHER) return tokens.muted;
    const index = bars?.parts.findIndex(part => part.key === key) ?? -1;
    return tokens.categories[Math.max(0, index) % tokens.categories.length];
  }, [bars, tokens]);
  // Every month's bands are the parts in order, so each band's colour is its part's.
  const layers = useMemo(() => bars ? [columnsLayer(bars.periods, bars.parts.map(part => colour(part.key)), BAR_STYLE)] : [], [bars, colour]);
  const monthAt = (at: number) => bars?.periods[columnAt(bars.periods, Math.floor(at))] ?? null;
  const caption = measure === 'manpower' ? 'Manpower lost' : LEDGER_LABELS[ledger];
  const [drill, dispatch] = useReducer(reduceDrill, CLOSED);
  const anchor = useRef<HTMLDivElement>(null);
  const menu = useContextMenu('Army losses actions');
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const pick = (at: number) => dispatch({ type: 'open', breakdown: String(monthOf(at)), hour: at, playing: false, playhead: null });
  const month = drill.breakdown === null ? null : Number(drill.breakdown);
  const drillRows = useMemo(() => data && month !== null ? monthArmies(data, month) : [], [data, month]);
  const kinds = LEDGERS[ledger];
  const columns = useMemo<readonly TableColumn<MonthArmy>[]>(() => [
    { key: 'army', header: 'Army', label: 'Army', width: 'minmax(var(--table-name-min), 2fr)', value: row => row.army.ordinal,
      text: row => armyName(row.army), render: row => <span data-tooltip={row.army.reference}>{armyName(row.army)}</span> },
    ...(measure === 'manpower' ? CHANNELS.map((channel, index): TableColumn<MonthArmy> => ({ key: channel.key, header: channel.label, label: channel.label,
      numeric: true, width: 'max-content', priority: index + 1, value: row => row.losses?.[index] ?? null })) : [
      { key: 'amount', header: caption, label: caption, numeric: true, width: 'max-content', value: row => row.designs.filter(design => kinds.includes(design.ledger))
        .reduce((sum, design) => sum + BigInt(design.amountRaw), 0n), render: row => designAmount(String(row.designs.filter(design => kinds.includes(design.ledger))
        .reduce((sum, design) => sum + BigInt(design.amountRaw), 0n))) } satisfies TableColumn<MonthArmy>]),
  ], [measure, caption, kinds]);
  let main;
  if (!track) main = <WidgetState kind="empty" message="Load a campaign to see army losses." />;
  else if (!window) main = <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  else if (!country.chosen) main = <WidgetState kind="empty" message="Choose a country to see its army losses." />;
  else if (country.error && !country.historyHeld) main = <WidgetState kind="error" detail={country.error} onRetry={country.retry} />;
  else if (country.holders) main = <ReachedCountryChoice tag={country.chosen} holders={country.holders} onPick={country.pick} />;
  else if (country.reason) main = <WidgetState kind="unavailable" message={country.reason} />;
  else if (country.absent) main = <WidgetState kind="unavailable" message={`${countryName} does not exist at this date.`} />;
  else if (!tag) main = <WidgetState kind="loading" label="Loading losses…" />;
  else if (state.error) main = <WidgetState kind="error" detail={state.error} onRetry={state.retry} />;
  else if (!data || !bars) main = <WidgetState kind="loading" label="Loading losses…" />;
  else if (data.absent) main = <WidgetState kind="unavailable" message={`${countryName} does not exist at this date.`} />;
  else if (data.availability.armies || data.availability.manpower && measure === 'manpower')
    main = <WidgetState kind="unavailable" message="The combat log wasn’t recorded at this time." />;
  else if (!bars.periods.some(period => !period.unknown))
    main = <WidgetState kind="empty" message={`${countryName} has no recorded army losses${widget.time.mode === 'follow' ? ' yet' : ''}.`} />;
  else main = <div className="army-losses-chart">
    <TimeChart label={`${caption} per month`} extent={[first, widget.time.mode === 'range' ? Math.floor(window.lastHour) : track.branch.lastHour]}
      branchStart={track.branch.firstHour} playhead={following ? hour : null} playback="cursor"
      axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }} layers={layers} keyHours={columnStops(bars.periods)}
      fitValues={shown => shown ? [Math.min(0, shown[0]), Math.max(0, shown[1])] : null}
      active={active} revision={`${data.hour}:${measure}:${stack}:${ledger}:${tokens.categories.join()}`}
      tooltip={at => { const period = monthAt(at); return period ? monthTooltip(period, bars.parts, measure, caption, following ? hour : null, colour) : null; }}
      onPick={at => { const period = monthAt(at); if (period && !period.unknown) pick(Math.floor(at)); }}
      onContextMenu={(_at, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, Math.min(hour, data.hour))])} />
  </div>;
  const legend = bars && bars.parts.length > 1 ? bars.parts.map(part => ({ id: part.key, label: part.label, color: colour(part.key) })) : [];
  return <div ref={anchor} className="army-losses">{menu.menu}
    {setOption && <div className="army-losses-caption"><span>{caption} per month</span>
      <Segmented value={measure} options={MEASURES} label="Measure" onChange={value => setOption('measure', value)} />
      {measure === 'equipment' && <Segmented value={ledger} options={LEDGER_CHOICES} label="Ledger" onChange={value => setOption('ledger', value)} />}
    </div>}
    {/* Outside the drill, so an open breakdown never hides a later failure. */}
    {country.chosen && country.error && country.historyHeld ? <WidgetState kind="error" detail={country.error} onRetry={country.retry} /> : null}
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={month !== null} title={month === null ? 'Month' : monthLabel(month)}
      day={month === null ? '' : monthLabel(month)} following={false} canFollow={false} anchor={anchor}
      onClose={() => dispatch({ type: 'close' })} onFollow={() => {}} main={<div className="army-losses-body">{main}{legend.length > 0 && <Legend label={caption} items={legend} />}</div>}>
      {month !== null && data && track && <div className="army-losses-drill">
        {drillRows.length ? <Table label={`${caption} in ${monthLabel(month)}`} rows={drillRows} columns={columns} rowKey={armyKey} sort={SORT}
          cellBars={false} rankMarkers={false} narrow={narrowTableOf(options)}
          tooltip={row => ({ title: armyName(row.army), rows: [
            ...(row.losses ? CHANNELS.map((channel, index) => ({ label: channel.label, value: formatAmount(row.losses![index], 'manpower') })) : [{ label: 'Manpower', value: 'Not recorded' }]),
            ...(['combat', 'attrition', 'recovered', 'enemy'] as const).flatMap(name => {
              const designs = row.designs.filter(design => LEDGERS[name].includes(design.ledger));
              return designs.length ? [{ label: LEDGER_LABELS[name], strong: true, divider: true }, ...designs.map(design => ({ label: design.name, value: designAmount(design.amountRaw), indent: true }))] : [];
            }),
          ] })}
          menu={() => [jumpItem(runtime, track.id, Math.min(hour, data.hour))]} />
          : <WidgetState kind="empty" message="No army kept an entry for this month." />}
      </div>}
    </DrillHost>
  </div>;
}
