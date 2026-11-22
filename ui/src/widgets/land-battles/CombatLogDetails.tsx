import { useGate } from '../../dashboard/regionGate';
import { SPEC } from '../../dashboard/regionTime';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useMemo, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { useProduct } from '../../dashboard/products';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import { countryMatch, countryMatchFilter } from '../../dashboard/groups';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { formatDateTime } from '../../dashboard/calendar';
import { useShownHour } from '../../dashboard/widgetPerf';
import { useHeadHour } from '../army-size/headHour.ts';
import type { CombatLogReply as Reply, LogDetailTable } from '../../generated/dashboard';
import { validateCombatLog } from './combatLogData.ts';
import './combatLog.css';
import { productRequest } from '../../dashboard/request.ts';

const EQUIPMENT_KINDS = ['Combat losses', 'Other attrition', 'Combat attrition', 'Training attrition', 'Recovered', 'Enemy losses'];
type View = 'manpower' | 'equipment' | 'battles';

export default function CombatLogDetails(props: WidgetProps & { view: View }) {
  const [open, setOpen] = useState(false);
  return <details className="combat-log-details" onToggle={event => setOpen(event.currentTarget.open)}>
    <summary>Game's retained Combat Log</summary>{open && <CombatLogSnapshot {...props} />}
  </details>;
}

export function CombatLogSnapshot({ widget, runtime, active, view }: WidgetProps & { view: View }) {
  const following = widget.time.mode === 'follow';
  const settled = useHeadHour(runtime.clock, active && following, at => runtime.clock.getSnapshot().playing ? 'playing' : String(Math.floor(at)));
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, settled);
  const hour = window ? Math.floor(window.lastHour) : null;
  const inBranch = !!track && hour !== null && hour >= track.branch.firstHour && hour <= track.branch.lastHour;
  const groups = useCountryGroups(runtime, track, active && widget.countries.kind !== 'all');
  const fallbackKey = JSON.stringify(selectedCountryKeys(widget.countries, track));
  const fallback = useMemo<string[]>(() => JSON.parse(fallbackKey), [fallbackKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const countries = useMemo(() => countryMatchFilter(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const gate = useGate(runtime, widget, SPEC, active);
  const at = gate.recorded(hour);
  const request = track && inBranch && at !== null && match.ready && !match.reason ? productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'combat-log-history', params: { branchId: track.branch.id, hour: at, countries: { countries: countries.tags?.map(countryFromKey) ?? null,
      membership: countries.membership?.flatMap(run => { const held = gate.span(run.firstHour, run.lastHour); return held ? [{ ...run, ...held }] : []; }) ?? null,
      unobserved: countries.unobserved.flatMap(span => gate.span(span.firstHour, span.lastHour) ?? []) } } }) : null;
  const state = useProduct<Reply>(runtime.client, request, active, validateCombatLog, track?.source);
  const shown = useShownHour();
  shown(state.data?.hour ?? null);
  if (!inBranch) return <WidgetState kind="outside" />;
  const groupFailure = widget.countries.kind !== 'all' && groups.error ? <WidgetState kind="error" what="country selection" failure={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return groupFailure;
  if (match.reason) return <>{groupFailure}<WidgetState kind="unavailable" message={match.reason} /></>;
  if (!match.ready) return <>{groupFailure}<WidgetState kind="loading" label="Loading selected countries" /></>;
  if (!state.data) return <>{groupFailure}{state.error ? <WidgetState kind="error" what="retained Combat Log" failure={state.error} onRetry={state.retry} />
    : <WidgetState kind="loading" label="Loading retained Combat Log" />}</>;
  return <>{groupFailure}<CombatLogTables data={state.data} view={view} /></>;
}

function Missing({ source }: { source: string | null }) {
  return source ? <p role="status">Unavailable - {source}</p> : null;
}

export function DetailTables({ tables }: { tables: readonly LogDetailTable[] }) {
  return <>{tables.map((table, i) => <table key={i}><caption>{table.title}</caption>
    <thead><tr>{table.columns.map((column, j) => <th key={j}>{column}</th>)}</tr></thead>
    <tbody>{table.rows.map((row, j) => <tr key={j}>{row.map((cell, k) => <td key={k}>{cell}</td>)}</tr>)}</tbody>
  </table>)}</>;
}

export function CombatLogTables({ data, view }: { data: Reply; view: View }) {
  const availability = data.availability;
  return <section className="combat-log-snapshot" aria-label="Retained Combat Log">
    <p>{formatDateTime(data.hour)} - Entries the game still retains at this hour for the selected countries.</p>
    <details><summary>Retained group membership</summary><Missing source={availability.groups} />
      {!availability.groups && <table><caption>Manager groups</caption><thead><tr><th>Country / identity</th><th>Group identity</th><th>Order</th></tr></thead>
        <tbody>{data.groups.map((g, i) => <tr key={i}><td>{g.tag}</td><td>{g.group}</td><td>{g.ordinal}</td></tr>)}</tbody></table>}
    </details>
    {view === 'manpower' && <><Missing source={availability.manpower} />{!availability.manpower && <>
      <p>Native manpower counters, in their stored order. These monthly group totals have their own admission and retention rules.</p>
      <table><caption>Manpower losses</caption><thead><tr><th>Country</th><th>First write</th><th>Group</th><th>Order</th><th>Reason</th><th>Ground</th><th>Air</th><th>Navy</th></tr></thead><tbody>
        {data.manpower.map((r, i) => <tr key={i}><td>{r.tag}</td><td>{formatDateTime(r.date)}</td><td>{r.group}</td><td>{r.ordinal}</td><td>{r.reason}</td>{r.losses.map((n, j) => <td key={j}>{n.toLocaleString()}</td>)}</tr>)}
      </tbody></table>{!data.manpower.length && <p>No retained manpower months.</p>}</>}</>}
    {view === 'equipment' && <><Missing source={availability.months} /><Missing source={availability.ownEquipment} /><Missing source={availability.enemyEquipment} />{!availability.months && <>
      <p>Enemy losses are equipment this group dealt. Own quantities use their latest sliced acquisition.</p>
      <table><caption>Retained equipment months</caption><thead><tr><th>Country</th><th>First write</th><th>Group</th><th>Month order</th><th>Ledger / reason</th><th>Allow zero</th><th>Design / order</th><th>Amount</th></tr></thead><tbody>
        {data.equipment.map((r, i) => <tr key={i}><td>{r.tag}</td><td>{formatDateTime(r.date)}</td><td>{r.group}</td><td>{r.month}</td><td>{EQUIPMENT_KINDS[r.kind] ?? `Ledger ${r.kind}`} / {r.reason}</td><td>{String(r.allowZero)}</td>
          <td>{r.missingSource ? `Unavailable - ${r.missingSource}` : r.variant ? `${r.name ?? `Name unavailable (${r.variant})`} - ${r.ordinal}` : 'Month entry; no amount rows at this sample'}</td><td>{r.amountRaw === null ? '?' : fixed5(r.amountRaw)}</td></tr>)}
      </tbody></table>{!data.equipment.length && <p>No retained equipment months.</p>}</>}</>}
    {view === 'battles' && <>
      <Missing source={availability.templates} />{!availability.templates && <table><caption>Division-template combat statistics</caption><thead><tr><th>Country</th><th>First write</th><th>Group</th><th>Order</th><th>Template / identity</th><th>Wins</th><th>Battles</th></tr></thead><tbody>
        {data.templates.map((r, i) => <tr key={i}><td>{r.tag}</td><td>{formatDateTime(r.date)}</td><td>{r.group}</td><td>{r.ordinal}</td><td>{r.name ? `${r.name} (${r.template})` : `Name unavailable (${r.template})`}</td><td>{r.wins}</td><td>{r.total}</td></tr>)}
      </tbody></table>}
      <Missing source={availability.battles} />{!availability.battles && <>
        <table><caption>Retained battle references</caption><thead><tr><th>Country</th><th>Date</th><th>Group</th><th>Record</th><th>Province ID</th><th>Group side</th><th>Attacker losses</th><th>Defender losses</th><th>Result</th></tr></thead><tbody>
          {data.battles.map((r, i) => <tr key={i}><td>{r.tag}</td><td>{formatDateTime(r.date)}</td><td>{r.group}</td><td><details><summary>{r.id} - reference {r.ordinal + 1}</summary>
            <p>{r.references} native references; stored flags: {r.flags.join(', ')}</p><DetailTables tables={r.details} />
          </details></td><td>{r.province}</td><td>{r.attacker ? 'Attacker' : 'Defender'}</td><td>{r.manpower[0].toLocaleString()}</td><td>{r.manpower[1].toLocaleString()}</td><td>{r.flags[0] ? 'Defender won' : 'Attacker won'}</td></tr>)}
        </tbody></table>{!data.battles.length && <p>No retained battle references.</p>}
      </>}
      <Missing source={availability.active} /><Missing source={availability.terminal} />
      {!!data.active.length && <table><caption>Observers containing selected countries</caption><thead><tr><th>Combat</th><th>Side</th><th>Progress</th><th>Damage</th><th>Activity groups</th><th>Leader entries</th></tr></thead><tbody>
        {data.active.map((r, i) => <tr key={i}><td><details><summary>{r.combat}{r.terminal ? ' (ended)' : ''}</summary><Missing source={r.lossSource} /><DetailTables tables={r.details} /></details></td>
          <td>{r.attacker ? 'Attacker' : 'Defender'}</td><td>{fixed5(r.progressRaw)}</td><td>{fixed5(r.damageRaw)}</td><td>{r.activityGroups}</td><td>{r.leaders}</td></tr>)}
      </tbody></table>}
    </>}
    {data.warnings.map((warning, i) => <p className="chart-line-key" key={i}>{warning}</p>)}
  </section>;
}

function fixed5(raw: string): string {
  const value = BigInt(raw), magnitude = value < 0n ? -value : value;
  const fraction = (magnitude % 100000n).toString().padStart(5, '0').replace(/0+$/, '');
  return `${value < 0n ? '-' : ''}${magnitude / 100000n}${fraction ? `.${fraction}` : ''}`;
}
