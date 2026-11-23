import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { CountryFlagUrl, countryFlagUnavailable } from '../../components/CountryFlag';
import LogList from '../../charts/log/LogList';
import type { LogRow } from '../../charts/log/logData';
import DrillHost from '../../charts/drill/DrillHost';
import { useDrill } from '../../charts/drill/useDrill';
import { tooltipText } from '../../charts/tooltip';
import type { TooltipContent } from '../../charts/tooltip';
import { useChartTokens } from '../../charts/useChartTokens';
import { dayLabel } from '../../charts/draw';
import { usNumber } from '../../charts/numberFormat';
import { useContentRevision } from '../../dashboard/contentRevision';
import { loadProduct, useProduct } from '../../dashboard/products';
import { useKeyedReads } from '../../dashboard/keyedReads';
import { resolveOptions } from '../../dashboard/options';
import type { BattleLogProps } from '../battles/BattleLog';
import type { BattleViewContext } from '../battles/BattleLogView';
import { battleValueText, rowsById } from '../battles/battleRows';
import type { BattleCountryLabel } from '../battles/battleRows';
import { battleOutcome, battleTooltip, heldTerrain, lengthText, missingTerrain, sideLook, validateLandBattleTerrain } from './landBattleCourse';
import type { HeldTerrain, SideLook } from './landBattleCourse';
import { landManpowerLost } from './landBattlesData.ts';
import type { LandBattleRow } from './landBattlesData';
import { definition } from './definition';
import LandBattleDrill from './LandBattleDrill';
import './landBattles.css';
import { useRegion } from '../../dashboard/widgetTime.ts';
import { productRequest } from '../../dashboard/request.ts';

type Props = BattleLogProps<LandBattleRow> & { context: BattleViewContext };
const NO_PLACES: readonly { provinceId: number; hour: number }[] = [];
const SIDES = ['Attackers', 'Defenders'] as const;
const FLAGS = 3;

function listRows(rows: readonly LandBattleRow[], view: Props['view'], logRows: readonly LogRow<LandBattleRow>[]): ReadonlyArray<LogRow<LandBattleRow>> {
  return view === 'log' ? logRows
    : rows.map(item => ({ key: item.id, id: item.id, hour: item.start?.hour ?? item.place.hour, by: item.start?.bound === 'by', item }));
}

export function Flags({ look, max = FLAGS }: { look: SideLook; max?: number }) {
  const shown = look.countries.length > max ? look.countries.slice(0, max - 1) : look.countries;
  return <span className="lb-flags">
    {look.unknown && !look.countries.length && <CountryFlagUrl className="log-flag" reason={look.unknown} />}
    { }
    {shown.map(part => <CountryFlagUrl key={part.tag} className="log-flag" url={part.flag} detail={part.flagDetail} reason={part.flagReason} pending={part.flagPending} />)}
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

const lossWidth = (value: number, larger: number) => larger > 0 && value > 0 ? `${Math.max(3, value / larger * 50)}%` : '0';

export function paintBattle(element: HTMLElement, row: LandBattleRow, terrain: string | null) {
  const held = battleParts(element);
  const { values, bars, sub } = held;
  const lost = landManpowerLost(row);
  if (lost && values.length === 2 && bars.length === 2 && (lost[0] !== held.lost[0] || lost[1] !== held.lost[1])) {
    held.lost = lost;
    const larger = Math.max(lost[0], lost[1]);
    lost.forEach((value, side) => {
      const text = usNumber.format(value);
      if (values[side].textContent !== text) values[side].textContent = text;
      const width = lossWidth(value, larger);
      if (bars[side].style.width !== width) bars[side].style.width = width;
    });
  }
  const line = subLine(row);
  if (sub && (line !== held.line || terrain !== held.terrain)) {
    held.line = line; held.terrain = terrain;
    const text = subText(row, terrain);
    if (sub.textContent !== text) sub.textContent = text;
  }
}

interface BattleParts { values: readonly HTMLElement[]; bars: readonly HTMLElement[]; sub: HTMLElement | null;
  lost: readonly number[]; line: string | null; terrain: string | null | undefined }
const parts = new WeakMap<HTMLElement, BattleParts>();
function battleParts(element: HTMLElement): BattleParts {
  let held = parts.get(element);
  if (!held || (held.sub && !element.contains(held.sub)) || (held.values[0] && !element.contains(held.values[0]))) {
    held = { values: [...element.querySelectorAll<HTMLElement>('.lb-losses .log-value')], bars: [...element.querySelectorAll<HTMLElement>('.lb-losses .lb-bar')],
      sub: element.querySelector<HTMLElement>('.lb-place small'), lost: [], line: null, terrain: undefined };
    parts.set(element, held);
  }
  return held;
}

function shownCells(element: HTMLElement): boolean {
  const holder = element.parentElement;
  if (!holder || holder.hidden) return false;
  const row = holder.classList.contains('log-row') ? holder : holder.parentElement;
  return row?.dataset.reveal !== 'ahead';
}

function subLine(row: LandBattleRow): string | null {
  const hours = row.length.value;
  if (hours === null || row.end === null && hours < 24) return null;
  return `${hours < 24 ? hours : `d${Math.floor(hours / 24)}`}${row.length.atLeast === true ? '+' : ''}`;
}

function subText(row: LandBattleRow, terrain: string | null) {
  return [terrain, row.end !== null || row.length.value !== null && row.length.value >= 24
    ? row.length.value === null ? null : lengthText(row.length.value, row.length.atLeast === true, false) : null].filter(Boolean).join(' · ');
}

function Losses({ row, colours }: { row: LandBattleRow; colours: readonly string[] }) {
  const values = row.sides.map(side => side.losses.find(value => value.label === 'Manpower lost') ?? null);
  if (values.some(value => value === null || value.value === null)) {
    const reason = values.find(value => value && value.value === null);
    return <span className="lb-losses is-unknown" title={reason && reason.value === null ? reason.unknown : 'Casualties not recorded'}>
      <span className="log-value is-muted">—</span><span className="lb-bars"><i className="lb-hatch" /></span><span className="log-value is-muted">—</span></span>;
  }
  const [attacker, defender] = values.map(value => value!.value as number);
  const larger = Math.max(attacker, defender);
  const width = (value: number) => lossWidth(value, larger);
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

function flagElement(url: string | undefined, reason: string | undefined, detail: string | undefined, pending: boolean | undefined): HTMLElement {
  const unavailable = (failed: boolean) => {
    const element = document.createElement('span'), tooltip = countryFlagUnavailable(reason, pending, failed).text;
    element.className = 'country-flag country-flag-unavailable log-flag';
    element.setAttribute('role', 'img');
    if (detail !== undefined) element.dataset.detail = detail;
    element.setAttribute('aria-label', tooltip);
    element.dataset.tooltip = tooltip;
    return element;
  };
  if (!url) return unavailable(false);
  const image = document.createElement('img');
  image.className = 'country-flag-art log-flag';
  image.src = url;
  image.alt = '';
  image.addEventListener('error', () => image.replaceWith(unavailable(true)), { once: true });
  return image;
}

const element = (tag: string, className?: string, text?: string) => {
  const made = document.createElement(tag);
  if (className) made.className = className;
  if (text !== undefined) made.textContent = text;
  return made;
};

function flagsElement(look: SideLook, max = FLAGS): HTMLElement {
  const shown = look.countries.length > max ? look.countries.slice(0, max - 1) : look.countries;
  const flags = element('span', 'lb-flags');
  if (look.unknown && !look.countries.length) flags.append(flagElement(undefined, look.unknown, undefined, false));
  for (const part of shown) flags.append(flagElement(part.flag, part.flagReason, part.flagDetail, part.flagPending));
  if (look.countries.length > shown.length) flags.append(element('span', 'lb-more', `+${look.countries.length - shown.length}`));
  return flags;
}

function battleCellsElement(item: LandBattleRow, values: LandBattleRow, sides: SideLook[], terrain: string | null, pending: boolean, keyedPlace: string | null,
  live: boolean, colours: readonly string[], onOpen: () => void): HTMLElement {
  const outcome = battleOutcome(item);
  const cells = element('span', 'lb-cells');
  cells.dataset.battle = item.id;
  cells.addEventListener('click', onOpen);
  const place = element('span', 'lb-place'), name = element('b', 'log-text');
  if (item.place.name === null && keyedPlace !== null) {
    const keyed = element('span', pending ? 'is-loading' : undefined, placeName(item, pending));
    keyed.dataset.keyed = 'place';
    keyed.dataset.keyedKey = keyedPlace;
    name.append(keyed);
  } else name.append(placeName(item, pending));
  place.append(name, element('small', undefined, subText(values, terrain)));
  const side = (index: number) => {
    const look = sides[index], state = outcome.kind !== 'won' ? null : outcome.side === index ? 'winner' : 'loser';
    const names = look.countries.map(part => part.name).concat(look.unknown ? [look.unknown] : []);
    const label = look.name ?? (look.unknown && !look.countries.length ? 'Not recorded' : null);
    const made = element('span', `lb-side lb-side-${index}${state ? ` is-${state}` : ''}${look.unknown ? ' is-unknown' : ''}`);
    made.title = `${SIDES[index]}: ${names.join(', ')}`;
    made.append(flagsElement(look));
    if (label) made.append(element('span', 'lb-side-name', label));
    return made;
  };
  const losses = (() => {
    const known = values.sides.map(entry => entry.losses.find(value => value.label === 'Manpower lost') ?? null);
    if (known.some(value => value === null || value.value === null)) {
      const reason = known.find(value => value && value.value === null);
      const made = element('span', 'lb-losses is-unknown');
      made.title = reason && reason.value === null ? reason.unknown : 'Casualties not recorded';
      const bars = element('span', 'lb-bars');
      bars.append(element('i', 'lb-hatch'));
      made.append(element('span', 'log-value is-muted', '—'), bars, element('span', 'log-value is-muted', '—'));
      return made;
    }
    const [attacker, defender] = known.map(value => value!.value as number);
    const larger = Math.max(attacker, defender);
    const made = element('span', 'lb-losses'), bars = element('span', 'lb-bars');
    bars.setAttribute('aria-hidden', 'true');
    const bar = (value: number, side: string, colour: string) => {
      const made = element('i', `lb-bar is-${side}`);
      made.style.width = lossWidth(value, larger);
      made.style.background = colour;
      return made;
    };
    bars.append(bar(attacker, 'attacker', colours[0]), element('i', 'lb-axis'), bar(defender, 'defender', colours[1]));
    made.append(element('span', 'log-value', usNumber.format(attacker)), bars, element('span', 'log-value', usNumber.format(defender)));
    return made;
  })();
  const result = (() => {
    if (outcome.kind === 'ongoing') {
      const made = element('span', `lb-result${live ? ' is-live' : ''}`);
      if (live) { const dot = element('i', 'live-dot'); dot.setAttribute('aria-hidden', 'true'); made.append(dot); }
      made.append('Ongoing');
      return made;
    }
    if (outcome.kind === 'unknown') { const made = element('span', 'lb-result is-unknown', outcome.text); made.title = outcome.text; return made; }
    const look = sides[outcome.side];
    const made = element('span', 'lb-result');
    made.title = `${look.countries.map(part => part.name).join(', ') || SIDES[outcome.side]} won`;
    made.append(flagsElement(look, 2));
    if (look.name) made.append(element('span', 'lb-side-name', look.name));
    made.append(element('span', 'lb-won', 'won'));
    return made;
  })();
  cells.append(place, side(0), losses, side(1), result);
  return cells;
}

interface CellsProps {
  stateKey: string;
  item: LandBattleRow;
  values: LandBattleRow;
  sides: SideLook[];
  terrain: string | null;
  pending: boolean;
  keyedPlace: string | null;
  rank: number | undefined;
  measure: string | null;
  view: Props['view'];
  live: boolean;
  painted: boolean;
  colours: readonly string[];
  onOpen: (stateKey: string) => void;
}

const sameLook = (a: SideLook, b: SideLook) => a.name === b.name && a.unknown === b.unknown && a.countries.length === b.countries.length
  && a.countries.every((part, index) => { const other = b.countries[index]; return part.tag === other.tag && part.name === other.name && part.flag === other.flag
    && part.flagReason === other.flagReason && part.flagDetail === other.flagDetail && part.flagPending === other.flagPending; });

function sameCells(a: CellsProps, b: CellsProps): boolean {
  if (a.stateKey !== b.stateKey || a.item.place.name !== b.item.place.name || a.pending !== b.pending || a.keyedPlace !== b.keyedPlace || a.rank !== b.rank
    || a.measure !== b.measure || a.view !== b.view || a.live !== b.live || a.painted !== b.painted || a.colours !== b.colours || a.onOpen !== b.onOpen
    || a.sides.length !== b.sides.length || !a.sides.every((look, index) => sameLook(look, b.sides[index]))) return false;
  if (a.painted) return true;
  const [lostA, lostB] = [landManpowerLost(a.values), landManpowerLost(b.values)];
  return subText(a.values, a.terrain) === subText(b.values, b.terrain) && (lostA === lostB || !!lostA && !!lostB && lostA[0] === lostB[0] && lostA[1] === lostB[1]);
}

const BattleCells = memo(function BattleCells({ stateKey, item, values, sides, terrain, pending, keyedPlace, rank, measure, view, live, colours, onOpen }: CellsProps) {
  const outcome = battleOutcome(item);
  const state = (index: number) => outcome.kind !== 'won' ? null : outcome.side === index ? 'winner' : 'loser';
  const cells = useRef<HTMLSpanElement>(null);
  useLayoutEffect(() => { if (cells.current) parts.delete(cells.current); });
  return <span ref={cells} className="lb-cells" data-battle={item.id} onClick={() => onOpen(stateKey)}>
    <span className="lb-place">
      <b className="log-text">{view === 'biggest' && <span className="lb-rank">{rank}</span>}
        {item.place.name === null && keyedPlace !== null
          ? <span className={pending ? 'is-loading' : undefined} data-keyed="place" data-keyed-key={keyedPlace}>{placeName(item, pending)}</span>
          : placeName(item, pending)}</b>
      { }
      <small>{subText(values, terrain)}</small></span>
    <Side look={sides[0]} index={0} state={state(0)} />
    <Losses row={values} colours={colours} />
    <Side look={sides[1]} index={1} state={state(1)} />
    {view === 'biggest' && <span className="log-value lb-measure">{measure}</span>}
    <Result row={item} looks={sides} live={live} />
  </span>;
}, sameCells);

export default function LandBattleList(props: Props) {
  const { rows, view, log, country, live, menu, context, label, empty, onVisibleRows } = props;
  const listRegion = useRegion(context.widget);
  const itemDrill = useDrill(context.runtime, listRegion);
  const tokens = useChartTokens();
  const colours = useMemo(() => [tokens.categories[0], tokens.categories[1]], [tokens]);
  const options = resolveOptions(definition.options, context.widget.options);
  const measure = context.rankedMeasure;
  const owner = JSON.stringify([context.track.source.id, context.track.source.revision, context.track.branch.id]);
  const [openId, setOpenId] = useState<{ client: typeof context.runtime.client; owner: string; id: string; row: LandBattleRow; hour: number } | null>(null);
  const anchor = useRef<HTMLDivElement>(null);
  const terrainRef = useRef<(province: number) => string | null>(() => null);
  const content = useContentRevision(context.runtime.client);
  const held = useRef<HeldTerrain | null>(null);
  held.current = heldTerrain(held.current, context.runtime.client, context.track.source, content);
  const terrains = held.current;
  const drawn = useRef<readonly number[]>([]);
  const [asked, setAsked] = useState<{ held: HeldTerrain; provinces: readonly number[] } | null>(null);
  const following = useRef(context.heldPlaces !== null);
  following.current = context.heldPlaces !== null;
  const readTerrain = useCallback((places: readonly { provinceId: number; hour: number }[], signal: AbortSignal) => loadProduct(context.runtime.client,
    productRequest({ sourceId: context.track.source.id, revision: context.track.source.revision, product: 'land-battle-terrain',
      params: { provinces: places.map(place => place.provinceId) }, asksFrom: Math.min(...places.map(place => place.hour)) }), signal, validateLandBattleTerrain, context.track.source)
    .then(reply => ({ values: reply.provinces.map(entry => [String(entry.provinceId), entry] as const), warnings: reply.warnings })),
  [context.runtime.client, context.track.source]);
  const terrainReads = useKeyedReads(JSON.stringify([context.track.source.id, context.track.source.revision, content]), context.heldPlaces ?? NO_PLACES,
    context.active, place => String(place.provinceId), readTerrain);
  for (const entry of terrainReads.reads.values.values()) terrains.names.set(entry.provinceId, entry);
  const provinces = following.current ? [] : asked?.held === terrains ? asked.provinces : missingTerrain(terrains, drawn.current);
  const terrainRequest = provinces.length ? productRequest({ sourceId: context.track.source.id, revision: context.track.source.revision,
    product: 'land-battle-terrain', params: { provinces } }) : null;
  const terrainReply = useProduct(context.runtime.client, terrainRequest, context.active, validateLandBattleTerrain);
  for (const entry of terrainReply.data?.provinces ?? []) terrains.names.set(entry.provinceId, entry);
  const terrainOf = (province: number) => terrainReads.reads.values.get(String(province))?.name ?? terrains.names.get(province)?.name ?? null;
  terrainRef.current = terrainOf;
  const ranks = useMemo(() => new Map(rows.map((row, index) => [row.id, index + 1])), [rows]);
  const now = (() => {
    const at = context.headHour();
    return at === null ? null : rowsById(context.rowsAt(at)) as ReadonlyMap<string, LandBattleRow>;
  })();
  const atHead = (row: LandBattleRow) => now?.get(row.id) ?? row;
  useLayoutEffect(() => {
    if (context.headHour() === null) return;
    let painted = Number.NaN;
    const paint = () => {
      const at = context.headHour();
      if (at === null || at === painted || !anchor.current) return;
      painted = at;
      const read = new Map<string, LandBattleRow | null>();
      for (const element of anchor.current.querySelectorAll<HTMLElement>('[data-battle]')) {
        if (!shownCells(element)) continue;
        const id = element.dataset.battle!;
        let row = read.get(id);
        if (row === undefined) read.set(id, row = context.rowAt(id, at) as LandBattleRow | null);
        if (row) paintBattle(element, row, terrainRef.current(row.provinceId));
      }
    };
    paint();
    const frames = context.runtime.clock.subscribeFrame(paint);
    const seeks = context.runtime.clock.subscribe(() => { painted = Number.NaN; paint(); });
    return () => { frames(); seeks(); };
  }, [context]);
  const logRows = useMemo(() => listRows(rows, view, props.logRows), [rows, view, props.logRows]);
  const looks = useCallback((row: LandBattleRow) => row.sides.map(side => sideLook(side, country as BattleCountryLabel)), [country]);
  const keyed = props.keyed;
  const bindRow = useMemo(() => view !== 'log' ? undefined : (row: LogRow<LandBattleRow>) => {
    const item = row.item, at = context.headHour();
    const head = at === null ? item : (context.rowAt(item.id, at) as LandBattleRow | null) ?? item;
    const values = !landManpowerLost(head) === !landManpowerLost(item) ? head : item;
    return battleCellsElement(item, values, looks(item), terrainRef.current(item.provinceId), context.placePending(item),
      item.place.name === null && keyed ? keyed.place(item) : null, live, colours, () => opener.current(item));
  }, [view, context, looks, keyed, live, colours]);
  const tooltip = useCallback((row: LandBattleRow): TooltipContent => {
    const at = context.headHour();
    const shown = at === null ? row : (rowsById(context.rowsAt(at)).get(row.id) as LandBattleRow | undefined) ?? row;
    return battleTooltip(shown, looks(shown), terrainOf(row.provinceId), colours, context.placePending(row));
  },
    [looks, colours, context.placePending, terrains, terrainReply.data]);
  const visible = useCallback((shown: readonly LogRow<LandBattleRow>[]) => {
    onVisibleRows(shown.map(row => row.item));
    drawn.current = shown.map(row => row.item.provinceId);
    const current = held.current;
    if (!current || following.current) return;
    const wanted = missingTerrain(current, drawn.current);
    setAsked(previous => previous?.held === current && (wanted.length === 0
      || previous.provinces.length === wanted.length && previous.provinces.every((value, index) => value === wanted[index])) ? previous : { held: current, provinces: wanted });
  }, [onVisibleRows]);
  const open = (row: LandBattleRow) => { setOpenId({ client: context.runtime.client, owner, id: row.id, row, hour: context.shownHour }); itemDrill.attach(row.id); };
  const drawnItems = useMemo(() => new Map<string, LandBattleRow>(), [logRows]);
  const opener = useRef(open);
  opener.current = open;
  const items = useRef(drawnItems);
  items.current = drawnItems;
  const openState = useCallback((stateKey: string) => { const row = items.current.get(stateKey); if (row) opener.current(row); }, []);
  const opened = openId && openId.client === context.runtime.client && openId.owner === owner ? openId : null;
  const listed = opened ? rows.find(row => row.id === opened.id) : undefined;
  useEffect(() => {
    if (listed) setOpenId(previous => previous && previous.id === listed.id && (previous.row !== listed || previous.hour !== context.shownHour)
      ? { ...previous, row: listed, hour: context.shownHour } : previous);
  }, [listed, context.shownHour]);
  const current = opened ? listed ?? (opened.hour <= context.shownHour ? opened.row : null) : null;
  const heldAt = opened && !listed && current ? opened.hour : null;
  const measureText = (row: LandBattleRow) => {
    const value = measure === 'Length' ? row.length : measure === 'Peak size' ? (row.peakUnknown ? null : row.peak.value) : row.casualties;
    if (value === null) return '—';
    if (typeof value === 'number') return usNumber.format(value);
    if (value.value === null) return '—';
    return measure === 'Length' ? lengthText(value.value, value.atLeast === true, false) : battleValueText(value);
  };
  const columns = view === 'log' ? 'minmax(96px, 1.5fr) minmax(56px, 190px) minmax(128px, 168px) minmax(56px, 190px) minmax(68px, 1fr)'
    : 'minmax(96px, 1.5fr) minmax(56px, 190px) minmax(128px, 168px) minmax(56px, 190px) max-content minmax(68px, 1fr)';
  const list = <div className="lb-view" ref={anchor}>
    <LogList anchorRows rows={logRows} label={label} density={log.density} months={view === 'log' && log.months}
      animate={view === 'log' && live && log.animate} playhead={view === 'log' ? props.reveal?.hour() : undefined} reveal={view === 'log' ? props.reveal : undefined} states={view === 'log'}
      columns={columns} onVisible={visible} bindRow={bindRow}
      menu={row => [{ label: 'Open battle', icon: 'chevron-right', run: () => open(row.item) }, ...menu(row.item)]} empty={empty}
      describe={row => tooltipText(tooltip(row.item))} tooltip={row => [tooltip(row.item)]}
      renderRow={row => {
        const item = row.item, head = atHead(item);
        drawnItems.set(row.key, item);
        const values = !landManpowerLost(head) === !landManpowerLost(item) ? head : item;
        return <BattleCells stateKey={row.key} item={item} values={values} sides={looks(item)} terrain={terrainOf(item.provinceId)} pending={context.placePending(item)}
          keyedPlace={item.place.name === null && props.keyed ? props.keyed.place(item) : null} rank={view === 'biggest' ? ranks.get(item.id) : undefined}
          measure={view === 'biggest' ? measureText(item) : null} view={view} live={live} painted={now !== null} colours={colours} onOpen={openState} />;
      }} />
  </div>;
  const drill = options.drill === 'popout' ? 'popout' : 'inline';
  return <DrillHost drill={itemDrill} runtime={context.runtime} subject={current !== null} mode={drill} title={current ? placeName(current, context.placePending(current)) : ''}
    day={dayLabel(context.shownHour)} canFollow={false} onClose={() => setOpenId(null)} anchor={anchor} main={list}>
    {current && <LandBattleDrill row={current} context={context} colours={colours} looks={looks(current)} terrain={terrainOf(current.provinceId)} heldAt={heldAt} />}
  </DrillHost>;
}
