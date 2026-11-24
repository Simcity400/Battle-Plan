import { countryFromKey } from '../../dashboard/countryKey.ts';
import { createPortal } from 'react-dom';
import { useCallback, useEffect, useId, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import WidgetState from '../../components/WidgetState';
import ArtFallback from '../../components/ArtFallback';
import { contentTitleHint } from '../../dashboard/contentTitles';
import FollowControl from '../../charts/drill/FollowControl';
import { useDrill } from '../../charts/drill/useDrill';
import { DetailWindowHost } from '../../families/host/DetailWindowHost';
import GateState from '../../dashboard/GateState';
import { dayLabel } from '../../charts/draw';
import { liveAt, liveByColumn, liveKey, mapLive } from '../../charts/live';
import type { Live, LiveValues } from '../../charts/live';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { fitTooltip } from '../../charts/tooltip';
import type { TooltipRow } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { reporting } from '../../dashboard/follow/liveCells.ts';
import {
  claimGameWindowSlot,
  gameWindow,
  releaseGameWindowSlot,
  subscribeGameWindows,
  updateGameWindowSlotPosition,
  windowDayLabel,
  withWindowDiagnostics,
} from '../../dashboard/gameWindows';
import type { IndustrialOrganisationSelection, ShownWindow, WindowPosition } from '../../dashboard/gameWindows';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { resolveWindow } from '../../dashboard/model';
import { nameReadFailure, nameReadText } from '../../dashboard/nameReads.ts';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { failureDetails } from '../../dashboard/queryError.ts';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { DEFAULT_SORT, OPTIONS, POLICY } from './definition';
import {
  cooldownDays,
  cooldownValues,
  exactFundsText,
  fundsRaw,
  fundsText,
  organisationNumbers,
  organisationRevision,
  organisationRows,
  policyText,
  validateOrganisations,
} from './organisationsData';
import type { OrganisationRow } from './organisationsData';
import './industrialOrganisations.css';
import { TimeRegion, useRegion, useRegionCoverage, useRegionHour } from '../../dashboard/widgetTime.ts';

const NO_ROWS: readonly OrganisationRow[] = [];
const NO_WARNINGS: readonly string[] = [];
const NAME_UNAVAILABLE = 'Organisation name unavailable';
const rowOf = (key: string) => key.slice(0, key.lastIndexOf('\u0000'));
const organisationWindow = () => gameWindow('industrialOrganisation');

export default function IndustrialOrganisationsWidget({ widget, runtime, active, headerSlot, onDiagnostics, setOption }: WidgetProps) {
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const sort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = selectedCountryKeys(widget.countries, track)[0] ?? null;
  const head = useHeadProduct(runtime, useGate(runtime, widget), 'industrial-organisations', tag ? { country: countryFromKey(tag) } : null, { active, validate: validateOrganisations });
  const data = head.data;
  const hour = head.at ?? head.hour ?? track?.branch.lastHour ?? 0;
  const slotOwnerId = useId();
  const [inheritedPos, setInheritedPos] = useState<WindowPosition | null>(null);
  const anchor = useRef<HTMLDivElement>(null);
  const drillControl = useDrill(runtime, useRegion(widget));
  const drill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const scope = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, tag]);
  const owner = useMemo(() => ({ client: runtime.client, scope }), [runtime.client, scope]);
  const [stored, setPicked] = useState<{ owner: object; selection: IndustrialOrganisationSelection; title: string } | null>(null);
  const picked = stored?.owner === owner ? stored : null;
  const [windowLines, setWindowLines] = useState<readonly string[]>(NO_WARNINGS);
  const onWindowDiagnostics = useCallback((lines: string[]) =>
    setWindowLines(previous => previous.length === lines.length && previous.every((line, index) => line === lines[index]) ? previous : lines), []);
  const [windowShown, setWindowShown] = useState<ShownWindow | null>(null);
  const closeForHandover = useCallback(() => {
    dispatch({ type: 'close' });
    setWindowLines(NO_WARNINGS);
    setWindowShown(null);
    setInheritedPos(null);
  }, [dispatch]);
  const closeDrill = useCallback(() => {
    releaseGameWindowSlot('industrialOrganisation', slotOwnerId);
    closeForHandover();
  }, [slotOwnerId, closeForHandover]);
  useEffect(() => closeDrill(), [owner, closeDrill]);
  useEffect(() => () => releaseGameWindowSlot('industrialOrganisation', slotOwnerId), [slotOwnerId]);
  const OrgWindow = useSyncExternalStore(subscribeGameWindows, organisationWindow, organisationWindow);
  const windowOpen = drill.breakdown !== null && !!picked && !!OrgWindow;
  const own = useMemo(() => [...data?.warnings ?? NO_WARNINGS, ...data?.detailsMissing ? [data.detailsMissing] : []], [data]);
  const diagnostics = useMemo(() => withWindowDiagnostics(own, windowOpen ? windowLines : NO_WARNINGS), [own, windowLines, windowOpen]);
  useDiagnostics(onDiagnostics, diagnostics);
  const rows = useMemo(() => data ? organisationRows(data) : NO_ROWS, [data]);
  const names = useCountryNamesAt(runtime.client, track, useMemo(() => tag ? [{ hour, tag }] : [], [tag, hour]), active);
  const shown = useShownHour();
  const shownAt = useRef(head.shownAt);
  shownAt.current = head.shownAt;
  const hourAt = useRef((position: number) => position);
  hourAt.current = position => Math.floor((track && resolveWindow(widget, track, runtime.tracks, position)?.lastHour) ?? hour);
  const live = head.live;
  const tableLive = useMemo((): Live<LiveValues<number>> => liveByColumn({
    funds: mapLive(reporting(live, () => shown(shownAt.current())), values => ({ get: (row: string) => values.get(liveKey(row, 'funds')), missing: (row: string) => values.missing(liveKey(row, 'funds')) })),
    policy: liveAt(runtime.clock, position => hourAt.current(position), at => cooldownValues(rows, at)),
  }), [live, shown, runtime.clock, rows]);
  const columns = useMemo<readonly TableColumn<OrganisationRow>[]>(() => {
    const byKey = new Map(rows.map(row => [row.key, row]));
    return [
      { key: 'organisation', header: 'Organisation', label: 'Organisation', width: 'minmax(var(--table-name-min), 2fr)', value: row => row.name ?? NAME_UNAVAILABLE,
        render: row => <span className="mio-name" data-tooltip={contentTitleHint(row.token)}>
          {row.icon ? <img className="mio-icon" src={row.icon} alt="" /> : <span className="mio-icon"><ArtFallback icon="factory" /></span>}
          <b>{row.name ?? NAME_UNAVAILABLE}</b></span> },
      { key: 'size', header: 'Size', label: 'Size', numeric: true, width: 'max-content', renderMissing: true, value: row => row.size, render: row => row.size === null ? '—' : row.size.toString() },
      { key: 'funds', header: 'Funds', label: 'Funds', numeric: true, width: 'max-content', value: () => null, paint: value => fundsText(fundsRaw(value as number)) },
      { key: 'traits', header: 'Traits', label: 'Traits unlocked', numeric: true, width: 'max-content', priority: 2, renderMissing: true,
        value: row => row.traits, render: row => row.traits === null ? '—' : row.traits.toString() },
      { key: 'policy', header: 'Policy', label: 'Policy', width: 'minmax(120px, 1.4fr)', priority: 1, value: () => null,
        paint: (value, key) => { const row = byKey.get(rowOf(key)); return row ? policyText(row, (value as number) > 0 ? value as number : null) : '—'; } },
    ];
  }, [rows]);
  const tooltip = useCallback((row: OrganisationRow) => {
    const { funds, points } = organisationNumbers(live.read(), row.key);
    const shownHour = hourAt.current(runtime.clock.getSnapshot().position);
    const days = cooldownDays(row.cooldownHour, shownHour);
    const lines: TooltipRow[] = [
      { label: 'Size', value: row.size === null ? 'Not recorded' : row.size.toString() },
      { label: 'Funds', value: funds === null ? 'Not recorded' : exactFundsText(funds) },
      { label: 'Unspent points', value: points === null ? 'Not recorded' : points.toString() },
      { label: 'Traits unlocked', value: row.traits === null ? 'Not recorded' : row.traits.toString() },
      { label: 'Policy', value: policyText(row, days), lookupKey: row.policyToken ?? undefined },
    ];
    if (!OrgWindow) lines.push({ label: 'The game’s organisation screen is not available yet', tone: 'muted' });
    return fitTooltip([{ title: row.name ?? NAME_UNAVAILABLE, subtitle: dayLabel(shownHour), rows: lines }], viewport);
  }, [viewport, OrgWindow, live, runtime.clock]);
  const open = (row: OrganisationRow) => {
    if (!track || !tag || !OrgWindow) return;
    const openHour = hourAt.current(runtime.clock.getHour());
    const inherited = claimGameWindowSlot('industrialOrganisation', slotOwnerId, closeForHandover);
    setInheritedPos(inherited);
    setPicked({
      owner,
      title: row.name ?? NAME_UNAVAILABLE,
      selection: {
        kind: 'industrialOrganisation',
        sourceId: track.source.id,
        revision: track.source.revision,
        branchId: track.branch.id,
        hour: openHour,
        organisation: row.key,
        tag: data?.tag ?? countryFromKey(tag),
      },
    });
    dispatch({ type: 'open', breakdown: POLICY.id, hour: openHour, follow: drillControl.followsAt(openHour) });
  };
  const onPositionChange = useCallback((pos: WindowPosition) => {
    updateGameWindowSlotPosition('industrialOrganisation', slotOwnerId, pos);
  }, [slotOwnerId]);
  const drillRegion = drillControl.region;
  const at = useRegionHour(runtime, drillRegion, active && drill.breakdown !== null);
  const drillCoverage = useRegionCoverage(runtime, drillRegion);
  const shut = drillControl.gate.stage.stage !== 'open' && drillControl.gate.stage.stage !== 'unbound';

  if (!track) return <WidgetState kind="empty" icon="factory" message="Load a campaign to see organisations." />;
  if (!tag) return <WidgetState kind="empty" message="Choose a country in widget settings." />;
  const country = nameReadText(names.get(nameKey(hour, tag)));
  const header = data && headerSlot && createPortal(<span className="pill">{rows.length.toLocaleString()} organisations</span>, headerSlot);
  const body = head.read === 'off' ? <WidgetState kind="outside" />
    : head.error ? <WidgetState kind="error" failure={head.error} onRetry={head.retry} />
    : !data ? <WidgetState kind="loading" label="Loading organisations…" />
    : data.missing ? <WidgetState kind="unavailable" message={data.missing} />
    : !rows.length ? <WidgetState kind="empty" icon="factory" message={`${country} has no industrial organisations at this date.`} />
      : <Table rowRevision={organisationRevision} key={`${track.source.id}:${track.branch.id}:${tag}`} label="Industrial organisations" rows={rows} columns={columns}
        rowKey={row => row.key} sort={sort} setOption={setOption} cellBars={options.cellBars === true} density={options.density === 'compact' ? 'compact' : 'comfortable'}
        rowHeight={2 * (tokens.fontSize.title + tokens.fontSize.small)} narrow={narrowTableOf(options)} hoursPerSecond={hoursPerSecond}
        onRowClick={OrgWindow ? open : undefined} tooltip={tooltip} live={tableLive} />;
  const main = <div ref={head.root} className="industrial-organisations-main">{body}</div>;
  const activeRow = picked ? rows.find(row => row.key === picked.selection.organisation) : undefined;
  const title = activeRow?.name ?? picked?.title ?? POLICY.label;
  return <div ref={anchor} data-detail={failureDetails([...names.values()].map(nameReadFailure))} className="industrial-organisations">
    {header}
    {main}
    <DetailWindowHost
      open={windowOpen}
      subject={picked ? picked.selection.organisation : null}
      hour={at}
      title={title}
      day={shut ? '' : windowShown ? windowDayLabel(windowShown) : at === null ? '' : dayLabel(at)}
      placement="center"
      anchor={anchor}
      returnFocus={picked && typeof CSS !== 'undefined' ? `tr[data-table-row="${CSS.escape(picked.selection.organisation)}"]` : undefined}
      initialPosition={inheritedPos}
      settled={windowShown !== null}
      onPositionChange={onPositionChange}
      headerActions={<FollowControl following={drillControl.following} onFollow={drillControl.follow} onPause={drillControl.pause} />}
      className="detail-window-game"
      closeLabel={`Close ${title}`}
      onClose={closeDrill}
    >
      <TimeRegion runtime={runtime} region={drillRegion}>
        {shut ? <GateState gate={drillControl.gate} loading="Loading…" />
          : picked && OrgWindow && at === null ? <WidgetState kind="outside" read={drillCoverage} />
          : picked && OrgWindow && at !== null ? <OrgWindow
            selection={{ ...picked.selection, hour: at }}
            active={active}
            client={runtime.client}
            onDiagnostics={onWindowDiagnostics}
            onShown={setWindowShown}
            onClose={closeDrill}
          />
          : null}
      </TimeRegion>
    </DetailWindowHost>
  </div>;
}
