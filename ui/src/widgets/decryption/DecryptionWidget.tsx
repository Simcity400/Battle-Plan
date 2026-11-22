import { useCallback, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { setAttr, setStyle, setText } from '../../charts/domText';
import type { HTMLAttributes } from 'react';
import WidgetState from '../../components/WidgetState';
import { CountryFlagUrl } from '../../components/CountryFlag';
import { useContextMenu } from '../../components/ContextMenu';
import type { MenuItem } from '../../components/ContextMenu';
import WindowedRows from '../../charts/WindowedRows';
import type { MovingRow } from '../../charts/rowMotion';
import type { Live, LiveValues } from '../../charts/live';
import { jumpItem } from '../../charts/jump';
import { tooltipText } from '../../charts/tooltip';
import type { TooltipContent } from '../../charts/tooltip';
import { useRowTooltips } from '../../charts/useRowTooltips';
import { useViewportHeight } from '../../charts/useChartTokens';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch } from '../../dashboard/groups';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { shownHour, useIntelLabels } from '../../dashboard/IntelCountry';
import { tagsOf } from '../../dashboard/intelligence';
import { resolveOptions } from '../../dashboard/options';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { OPTIONS } from './definition';
import { activeBonusDays, amountText, bonusText, CIPHER_BROKEN, cipherNumbers, decryptionTooltip, observationDate, pairKey, percentOf,
  progressLabel, progressText, raceRows, raceStructure, validateDecryption, workingStatus } from './decryptionData';
import type { CountrySide, RaceOrder } from './decryptionData';
import './decryption.css';
import type { CipherPair, DecryptionReply } from '../../generated/dashboard';

const NO_ROWS: readonly MovingRow<CipherPair>[] = [];
const PARAMS = {} as const;

export default function DecryptionWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const head = useHeadProduct(runtime, useGate(runtime, widget), 'decryption', track ? PARAMS : null,
    { active, validate: validateDecryption, structure: raceStructure });
  useShownEachFrame(runtime.clock, head.shownAt, head.live);
  const hour = head.at ?? shownHour(runtime, widget, head) ?? 0;
  const groups = useCountryGroups(runtime, track, active && widget.countries.kind !== 'all');
  const selection = useMemo(() => countryMatch(widget.countries, selectedCountryKeys(widget.countries, track), groups.data, track?.branch.lastHour ?? hour),
    [widget.countries, track, groups.data, hour]);
  const data = head.data;
  const rows = useMemo(() => data && selection.ready ? raceRows(data, options.order as RaceOrder, options.countrySide as CountrySide, hour,
    widget.countries.kind === 'all' ? undefined : selection.matches) : [], [data, options.order, options.countrySide, hour, selection, widget.countries.kind]);
  const items = useMemo(() => rows.map(item => ({ key: pairKey(item), item })), [rows]);
  const [visible, setVisible] = useState<readonly MovingRow<CipherPair>[]>(NO_ROWS);
  const pushed = head.read === 'followed';
  const tags = useMemo(() => tagsOf((pushed ? rows : visible.map(row => row.item)).flatMap(pair => [pair.owner, pair.target])), [pushed, rows, visible]);
  const label = useIntelLabels(runtime, widget, tags, hour, active, pushed);
  const labelOf = useCallback((tag: string | null) => tag === null ? null : label(tag), [label]);
  const menu = useContextMenu('Cipher actions');
  const height = useViewportHeight();
  const byKey = useMemo(() => new Map(rows.map(row => [pairKey(row), row])), [rows]);
  const live = head.live;
  const content = useCallback((key: string) => {
    const pair = byKey.get(key);
    return pair && data ? decryptionTooltip(pair, data, cipherNumbers(live.read(), pair), labelOf(pair.owner.tag), labelOf(pair.target.tag), height) : null;
  }, [byKey, data, live, labelOf, height]);
  const jump = (key: string, x: number, y: number) => {
    const pair = byKey.get(key);
    if (!track || !pair) return;
    const entries: MenuItem[] = [];
    if (pair.started) entries.push({ ...jumpItem(runtime, track.id, pair.started.hour),
      label: `Jump to the start of the current attempt · ${observationDate(pair.started)}` });
    if (pair.broken === true && pair.brokenAt) entries.push({ ...jumpItem(runtime, track.id, pair.brokenAt.hour),
      label: `Jump to the break · ${observationDate(pair.brokenAt)}` });
    if (entries.length) menu.open(x, y, entries);
  };
  const tips = useRowTooltips<string>({ owner: `decryption:${widget.id}`, active, content,
    revision: `${hour}:${track?.source.revision}:${track?.branch.id}`, onMenu: jump });
  useDiagnostics(onDiagnostics, data ? data.warnings : null);
  if (!track) return <WidgetState kind="empty" icon="target" message="Load a campaign to see its decryption races." />;
  if (head.read === 'off') return <WidgetState kind="outside" />;
  if (head.error && !data) return <WidgetState kind="error" failure={head.error} onRetry={head.retry} />;
  const groupFailure = widget.countries.kind !== 'all' && groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return groupFailure;
  const body = () => {
    if (!data || !selection.ready) return <WidgetState kind="loading" label="Loading decryption…" />;
    if (data.missing) return <WidgetState kind="unavailable" message="Decryption data is unavailable at this time." detail={data.missing} />;
    if (selection.reason) return <WidgetState kind="unavailable" message={selection.reason} />;
    return <>
      {data.attemptsUnknown && <p className="chart-line-key">{data.attemptsUnknown}</p>}
      <WindowedRows items={items} rowHeight={52} padding={8} label="Decryption races" className="decryption-race" onVisibleRows={pushed ? undefined : setVisible}
        empty={<WidgetState kind="empty" message={widget.countries.kind === 'all' ? 'No ciphers being decrypted yet.' : 'No matching decryption pairs at this time.'} />}>
        {row => <RaceCell pair={row.item} reply={data} live={live} owner={labelOf(row.item.owner.tag)} target={labelOf(row.item.target.tag)}
          active={active && row.phase !== 'leaving'} leaving={row.phase === 'leaving'}
          rowProps={row.phase === 'leaving' ? undefined : tips.rowProps(row.key)} descriptionId={tips.descriptionId(row.key)}
          content={row.phase === 'leaving' ? null : content(row.key)} />}
      </WindowedRows>
    </>;
  };
  return <div ref={head.root} className="decryption" key={`${track.source.id}:${track.source.revision}:${track.branch.id}`}>
    {groupFailure}
    {body()}
    {menu.menu}
  </div>;
}

function RaceCell({ pair, reply, live, owner, target, active, leaving, rowProps, descriptionId, content }: {
  pair: CipherPair; reply: DecryptionReply; live: Live<LiveValues<number>>; owner: ReturnType<CountryLabel> | null; target: ReturnType<CountryLabel> | null;
  active: boolean; leaving: boolean; rowProps: HTMLAttributes<HTMLDivElement> | undefined; descriptionId: string; content: TooltipContent | null;
}) {
  const broken = pair.broken === true;
  const known = pair.thresholdRaw !== null;
  const fill = useRef<HTMLElement>(null), meter = useRef<HTMLDivElement>(null), percent = useRef<HTMLElement>(null);
  const unknown = useRef<HTMLElement>(null), amount = useRef<HTMLSpanElement>(null), bonus = useRef<HTMLSpanElement>(null);
  const view = () => {
    const numbers = cipherNumbers(live.read(), pair);
    const shown = percentOf(pair, numbers);
    return { numbers, percent: shown, bonus: activeBonusDays(pair, numbers) };
  };
  useLayoutEffect(() => {
    const paint = () => {
      const now = view();
      setStyle(fill.current, 'width', `${now.percent ?? 0}%`);
      if (meter.current) {
        if (now.percent === null) { if (meter.current.hasAttribute('aria-valuenow')) meter.current.removeAttribute('aria-valuenow'); setAttr(meter.current, 'aria-valuetext', 'Not recorded'); }
        else { setAttr(meter.current, 'aria-valuenow', String(now.percent / 100)); if (meter.current.hasAttribute('aria-valuetext')) meter.current.removeAttribute('aria-valuetext'); }
      }
      setText(percent.current, progressLabel(now.percent));
      setText(unknown.current, progressText(now.percent, now.numbers));
      setText(amount.current, amountText(now.numbers));
      setText(bonus.current, bonusText(now.bonus));
    };
    paint();
    return live.subscribe(paint);
  }, [live, pair]);
  const now = view();
  const state = broken ? 'done' : active && pair.decrypting !== 0 && known ? 'live' : 'idle';
  return <div className="decryption-row" {...rowProps} tabIndex={leaving ? -1 : 0}>
    {content && <span id={descriptionId} className="sr-only">{tooltipText(content)}</span>}
    <div className="decryption-pair">
      <span className="decryption-flag">{owner && <CountryFlagUrl url={owner.flag} detail={owner.flagDetail} reason={owner.flagReason} pending={owner.flagPending} />}</span>
      <small aria-hidden="true">→</small>
      <span className="decryption-flag">{target && <CountryFlagUrl url={target.flag} detail={target.flagDetail} reason={target.flagReason} pending={target.flagPending} />}</span>
      <small className="decryption-target">{target?.name ?? pair.target.tag ?? `Country ${pair.target.id}`}</small>
    </div>
    <div className="decryption-progress" aria-hidden="true">
      <div ref={meter} className={`progress-meter is-${state} has-sweep`} role="meter" aria-label="Progress" aria-valuemin={0} aria-valuemax={1}
        aria-valuenow={now.percent === null ? undefined : now.percent / 100} aria-valuetext={now.percent === null ? 'Not recorded' : undefined}>
        <i ref={fill} style={{ width: `${now.percent ?? 0}%` }} />
      </div>
      {broken && !known && <small ref={unknown} className="decryption-unknown">{progressText(now.percent, now.numbers)}</small>}
    </div>
    <div className="decryption-status">
      <b ref={broken ? undefined : percent}>{broken ? CIPHER_BROKEN : progressLabel(now.percent)}</b>
      <span>{broken ? observationDate(pair.brokenAt, reply.attemptsUnknown === null ? 'Not recorded' : 'Not known') : workingStatus(pair)}</span>
      {!broken && !known && <span ref={amount} className="decryption-amount">{amountText(now.numbers)}</span>}
      {broken && pair.bonusActive !== 0 && <span ref={bonus} className="pill warning">{bonusText(now.bonus)}</span>}
    </div>
  </div>;
}
