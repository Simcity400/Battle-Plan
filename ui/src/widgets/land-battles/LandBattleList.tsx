import { useCallback, useContext, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { CountryFlagUrl } from '../../components/CountryFlag';
import { Segmented } from '../../components/Segmented';
import LogList from '../../charts/log/LogList';
import type { LogRow } from '../../charts/log/logData';
import DrillHost from '../../charts/drill/DrillHost';
import { HeaderActionsSlot } from '../../charts/ZoomReset';
import { tooltipText } from '../../charts/tooltip';
import type { TooltipContent } from '../../charts/tooltip';
import { useChartTokens } from '../../charts/useChartTokens';
import { dayLabel } from '../../charts/draw';
import { usNumber } from '../../charts/numberFormat';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useProduct } from '../../dashboard/products';
import { resolveOptions } from '../../dashboard/options';
import type { BattleLogProps } from '../battles/BattleLog';
import type { BattleViewContext } from '../battles/BattleLogView';
import { battleLogRows, battleValueText } from '../battles/battleRows';
import type { BattleCountryLabel } from '../battles/battleRows';
import { battleOutcome, battleTooltip, heldTerrain, lengthText, missingTerrain, sideLook, validateLandBattleTerrain } from './landBattleCourse';
import type { HeldTerrain, SideLook } from './landBattleCourse';
import type { LandBattleRow } from './landBattlesData';
import { definition, VIEWS } from './definition';
import LandBattleDrill from './LandBattleDrill';
import './landBattles.css';

type Props = BattleLogProps<LandBattleRow> & { context: BattleViewContext };
const SIDES = ['Attackers', 'Defenders'] as const;
/** Flags drawn per side before "+N"; every country stays in the hint and tooltip. */
const FLAGS = 3;

export function Flags({ look, max = FLAGS }: { look: SideLook; max?: number }) {
  const shown = look.countries.length > max ? look.countries.slice(0, max - 1) : look.countries;
  return <span className="lb-flags">
    {look.unknown && !look.countries.length && <CountryFlagUrl className="log-flag" reason={look.unknown} />}
    {shown.map(part => <CountryFlagUrl key={`${part.tag}@${part.hour}`} className="log-flag" url={part.flag} reason={part.flagReason} pending={part.flagPending} />)}
    {look.countries.length > shown.length && <span className="lb-more">+{look.countries.length - shown.length}</span>}
  </span>;
}

function Side({ look, index, state }: { look: SideLook; index: number; state: 'winner' | 'loser' | null }) {
  const names = look.countries.map(part => part.name).concat(look.unknown ? [look.unknown] : []);
  const label = look.name ?? (look.unknown && !look.countries.length ? 'Not recorded' : null);
  return <span className={`lb-side lb-side-${index}${state ? ` is-${state}` : ''}${look.unknown ? ' is-unknown' : ''}`} title={`${SIDES[index]}: ${names.join(', ')}`}>
    <Flags look={look} />{label && <span className="lb-side-name">{label}</span>}
  </span>;
}

/** Each side's manpower lost; the larger loss fills its half, so the bars show the split and the numbers the size. */
function Losses({ row, colours }: { row: LandBattleRow; colours: readonly string[] }) {
  const values = row.sides.map(side => side.losses.find(value => value.label === 'Manpower lost') ?? null);
  if (values.some(value => value === null || value.value === null)) {
    const reason = values.find(value => value && value.value === null);
    return <span className="lb-losses is-unknown" title={reason && reason.value === null ? reason.unknown : 'Casualties not recorded'}>
      <span className="log-value is-muted">—</span><span className="lb-bars"><i className="lb-hatch" /></span><span className="log-value is-muted">—</span></span>;
  }
  const [attacker, defender] = values.map(value => value!.value as number);
  const larger = Math.max(attacker, defender);
  const width = (value: number) => larger > 0 && value > 0 ? `${Math.max(3, value / larger * 50)}%` : '0';
  return <span className="lb-losses">
    <span className="log-value">{usNumber.format(attacker)}</span>
    <span className="lb-bars" aria-hidden="true"><i className="lb-bar is-attacker" style={{ width: width(attacker), background: colours[0] }} />
      <i className="lb-axis" /><i className="lb-bar is-defender" style={{ width: width(defender), background: colours[1] }} /></span>
    <span className="log-value">{usNumber.format(defender)}</span>
  </span>;
}

function Result({ row, looks, live }: { row: LandBattleRow; looks: SideLook[]; live: boolean }) {
  const outcome = battleOutcome(row);
  if (outcome.kind === 'ongoing') return <span className={`lb-result${live ? ' is-live' : ''}`}>{live && <i className="live-dot" aria-hidden="true" />}Ongoing</span>;
  if (outcome.kind === 'unknown') return <span className="lb-result is-unknown" title={outcome.text}>{outcome.text}</span>;
  const look = looks[outcome.side];
  return <span className="lb-result" title={`${look.countries.map(part => part.name).join(', ') || SIDES[outcome.side]} won`}>
    <Flags look={look} max={2} />{look.name && <span className="lb-side-name">{look.name}</span>}<span className="lb-won">won</span></span>;
}

function placeName(row: LandBattleRow, pending: boolean) {
  return row.place.name ?? (pending ? 'Loading…' : 'Unnamed province');
}

/** Both views share one row: place, attackers, manpower lost each side, defenders, [ranked measure], result.
 * Biggest's measure column is what the ranking actually ranked by. */
export default function LandBattleList(props: Props) {
  const { rows, view, log, country, live, menu, context, label, empty, onVisibleRows } = props;
  const tokens = useChartTokens();
  const colours = useMemo(() => [tokens.categories[0], tokens.categories[1]], [tokens]);
  const actions = useContext(HeaderActionsSlot);
  const options = resolveOptions(definition.options, context.widget.options);
  // The ranking's own measure: an older recording without casualties ranks by peak size.
  const measure = context.rankedMeasure;
  const [openId, setOpenId] = useState<{ id: string; row: LandBattleRow } | null>(null);
  const anchor = useRef<HTMLDivElement>(null);
  // Names already read stay while the next drawn rows load, so scrolling never blanks a terrain;
  // new content or another recording starts again and asks for every drawn province.
  const content = useContentRevision(context.runtime.client);
  const held = useRef<HeldTerrain | null>(null);
  held.current = heldTerrain(held.current, context.runtime.client, context.track.source, content);
  const terrains = held.current;
  const drawn = useRef<readonly number[]>([]);
  const [asked, setAsked] = useState<{ held: HeldTerrain; provinces: readonly number[] } | null>(null);
  const provinces = asked?.held === terrains ? asked.provinces : missingTerrain(terrains, drawn.current);
  const terrainRequest = provinces.length ? { sourceId: context.track.source.id, revision: context.track.source.revision,
    product: 'land-battle-terrain', params: { provinces } } : null;
  const terrainReply = useProduct(context.runtime.client, terrainRequest, context.active, validateLandBattleTerrain);
  for (const entry of terrainReply.data?.provinces ?? []) terrains.names.set(entry.provinceId, entry);
  const terrainOf = (province: number) => terrains.names.get(province)?.name ?? null;
  const ranks = useMemo(() => new Map(rows.map((row, index) => [row.id, index + 1])), [rows]);
  const logRows = useMemo<LogRow<LandBattleRow>[]>(() => view === 'log' ? battleLogRows(rows, live)
    // Ranked rows keep the server's order and show each battle's start.
    : rows.map(item => ({ key: item.id, id: item.id, hour: item.start?.hour ?? item.place.hour, by: item.start?.bound === 'by', item })), [rows, live, view]);
  const looks = useCallback((row: LandBattleRow) => row.sides.map(side => sideLook(side, country as BattleCountryLabel)), [country]);
  const tooltip = useCallback((row: LandBattleRow): TooltipContent => battleTooltip(row, looks(row), terrainOf(row.provinceId), colours, context.placePending(row)),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [looks, colours, context.placePending, terrains, terrainReply.data]);
  const visible = useCallback((shown: readonly LogRow<LandBattleRow>[]) => {
    onVisibleRows(shown.map(row => row.item));
    drawn.current = shown.map(row => row.item.provinceId);
    const current = held.current;
    if (!current) return;
    const wanted = missingTerrain(current, drawn.current);
    setAsked(previous => previous?.held === current && (wanted.length === 0
      || previous.provinces.length === wanted.length && previous.provinces.every((value, index) => value === wanted[index])) ? previous : { held: current, provinces: wanted });
  }, [onVisibleRows]);
  const open = (row: LandBattleRow) => setOpenId({ id: row.id, row });
  const current = openId ? rows.find(row => row.id === openId.id) ?? openId.row : null;
  const measureText = (row: LandBattleRow) => {
    const value = measure === 'Length' ? row.length : measure === 'Peak size' ? (row.peakUnknown ? null : row.peak.value) : row.casualties;
    if (value === null) return '—';
    if (typeof value === 'number') return usNumber.format(value);
    if (value.value === null) return '—';
    return measure === 'Length' ? lengthText(value.value, value.atLeast === true, false) : battleValueText(value);
  };
  const columns = view === 'log' ? 'minmax(96px, 1.5fr) minmax(56px, 190px) minmax(128px, 168px) minmax(56px, 190px) minmax(68px, 1fr)'
    : 'minmax(96px, 1.5fr) minmax(56px, 190px) minmax(128px, 168px) minmax(56px, 190px) max-content minmax(68px, 1fr)';
  const header = actions && createPortal(<Segmented value={view} options={VIEWS} onChange={value => context.setOption?.('view', value)} label="Land battles view" />, actions);
  const list = <div className="lb-view" ref={anchor}>
    <LogList anchorRows rows={logRows} label={label} density={log.density} months={view === 'log' && log.months}
      animate={view === 'log' && live && log.animate} columns={columns} onVisible={visible}
      menu={row => [{ label: 'Open battle', icon: 'chevron-right', run: () => open(row.item) }, ...menu(row.item)]} empty={empty}
      describe={row => tooltipText(tooltip(row.item))} tooltip={row => [tooltip(row.item)]}
      renderRow={row => {
        const item = row.item, sides = looks(item), outcome = battleOutcome(item);
        const state = (index: number) => outcome.kind !== 'won' ? null : outcome.side === index ? 'winner' : 'loser';
        const terrain = terrainOf(item.provinceId);
        const sub = [terrain, item.end !== null || item.length.value !== null && item.length.value >= 24
          ? item.length.value === null ? null : lengthText(item.length.value, item.length.atLeast === true, false) : null].filter(Boolean).join(' · ');
        return <span className="lb-cells" onClick={() => open(item)}>
          <span className="lb-place">
            <b className={`log-text${item.place.name === null && context.placePending(item) ? ' is-loading' : ''}`}>
              {view === 'biggest' && <span className="lb-rank">{ranks.get(item.id)}</span>}{placeName(item, context.placePending(item))}</b>
            {sub && <small>{sub}</small>}</span>
          <Side look={sides[0]} index={0} state={state(0)} />
          <Losses row={item} colours={colours} />
          <Side look={sides[1]} index={1} state={state(1)} />
          {view === 'biggest' && <span className="log-value lb-measure">{measureText(item)}</span>}
          <Result row={item} looks={sides} live={live} />
        </span>;
      }} />
  </div>;
  const drill = options.drill === 'popout' ? 'popout' : 'inline';
  return <>{header}<DrillHost mode={drill} open={current !== null} title={current ? placeName(current, context.placePending(current)) : ''}
    day={dayLabel(context.shownHour)} following={false} canFollow={false} onFollow={() => {}} onClose={() => setOpenId(null)} anchor={anchor} main={list}>
    {current && <LandBattleDrill row={current} context={context} colours={colours} looks={looks(current)} terrain={terrainOf(current.provinceId)} />}
  </DrillHost></>;
}

