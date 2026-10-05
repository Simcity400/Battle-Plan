import { useEffect, useMemo, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { useContextMenu } from '../../components/ContextMenu';
import type { MenuItem } from '../../components/ContextMenu';
import WindowedRows from '../../charts/WindowedRows';
import type { MovingRow } from '../../charts/rowMotion';
import { ProgressMeter } from '../../charts/meter';
import { jumpItem } from '../../charts/jump';
import { runAt } from '../../charts/series';
import { hideTooltip, showTooltip, tooltipText } from '../../charts/tooltip';
import { useViewportHeight } from '../../charts/useChartTokens';
import { selectedTags } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { groupMembers, groupRuns, groupUnobserved } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useCountriesAt } from '../../dashboard/useCountriesAt';
import type { CountryAt } from '../../dashboard/useCountriesAt';
import type { DecryptionReply } from '../../generated/dashboard';
import { OPTIONS } from './definition';
import { activeBonusDays, amountText, CIPHER_BROKEN, decryptionTooltip, observationDate, progressLabel, progressText, raceRows, validateDecryptionReply, workingStatus } from './decryptionData';
import type { CountrySide, RaceOrder, RaceRow } from './decryptionData';
import './decryption.css';

const NO_WARNINGS: string[] = [];

export default function DecryptionWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = window?.lastHour ?? 0;
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'decryption', params: { branchId: track.branch.id } } : null;
  const { data, error, retry } = useProduct<DecryptionReply>(runtime.client, request, active, validateDecryptionReply);
  const groups = useCountryGroups(runtime, track, active && widget.countries.kind === 'group');
  const selection = useMemo(() => {
    const choice = widget.countries;
    if (choice.kind === 'all') return { tags: null, reason: null };
    if (choice.kind !== 'group') return { tags: new Set(selectedTags(choice, track)), reason: null };
    if (!groups.data) return { tags: new Set<string>(), reason: groups.error ?? 'Loading country groups…' };
    const runs = groupRuns(groups.data, choice.group);
    if (!runs) return { tags: new Set<string>(), reason: 'This group is not in this campaign branch.' };
    const members = groupMembers(runs, choice.membership, hour, track?.branch.lastHour ?? hour, Infinity, groupUnobserved(groups.data, choice.group));
    return { tags: new Set(members ?? []), reason: members == null ? 'Group membership is not recorded at this time.' : null };
  }, [widget.countries, track, groups.data, groups.error, hour]);
  const rows = useMemo(() => data ? raceRows(data, hour, options.order as RaceOrder, selection.tags, options.countrySide as CountrySide) : [],
    [data, hour, options.order, options.countrySide, selection.tags]);
  const items = useMemo(() => rows.map(item => ({ key: item.key, item })), [rows]);
  const [visibleRows, setVisibleRows] = useState<readonly MovingRow<RaceRow>[]>([]);
  const tags = useMemo(() => [...new Set(visibleRows.filter(row => row.phase !== 'leaving')
    .flatMap(({ item }) => [item.tags.ownerTag, item.tags.targetTag]).filter((tag): tag is string => tag != null))], [visibleRows]);
  const countries = useCountriesAt(runtime.client, track, tags, hour, active);
  const menu = useContextMenu('Cipher actions');
  useEffect(() => { onDiagnostics(data?.warnings ?? NO_WARNINGS); return () => onDiagnostics(NO_WARNINGS); }, [data, onDiagnostics]);
  if (!track) return <WidgetState kind="empty" icon="target" message="Load a campaign to see its decryption races." />;
  if (!window || hour < track.branch.firstHour || hour > track.branch.lastHour) return <WidgetState kind="unavailable" message="This time is outside the recorded branch." />;
  if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading decryption…" />;
  const coverage = runAt(data.coverage, hour);
  if (!coverage || (!coverage.complete && !rows.length)) return <WidgetState kind="unavailable"
    message={`Not in this recording${coverage?.reason ? `: ${coverage.reason}` : ' at this time.'}`} />;
  if (selection.reason) return <WidgetState kind="unavailable" message={selection.reason} />;
  const jump = (row: RaceRow, x: number, y: number) => {
    const items: MenuItem[] = [];
    if (row.run.started) items.push({ ...jumpItem(runtime, track.id, row.run.started.hour),
      label: `Jump to the start of the current attempt · ${observationDate(row.run.started, row.hour)}` });
    if (row.run.broken === true && row.broken) items.push({ ...jumpItem(runtime, track.id, row.broken.hour),
      label: `Jump to the break · ${observationDate(row.broken, row.hour)}` });
    if (items.length) menu.open(x, y, items);
  };
  return <div className="decryption" key={JSON.stringify([track.source.id, track.source.revision, track.branch.id])}>
    {coverage.reason && <p className="dash-inline-warning" role="status">{coverage.reason}</p>}
    <WindowedRows items={items} rowHeight={52} padding={8} label="Decryption races" className="decryption-race"
      onVisibleRows={setVisibleRows} empty={<WidgetState kind="empty" message={widget.countries.kind === 'all' ? 'No ciphers being decrypted yet.' : 'No matching decryption pairs at this time.'} />}>
      {row => <RaceCell row={row.item} label={countries.country} active={active && row.phase !== 'leaving'} leaving={row.phase === 'leaving'}
        owner={`decryption:${widget.id}:${row.key}`} onJump={jump} />}
    </WindowedRows>
    {menu.menu}
  </div>;
}

/** Names and flags follow one shared playhead look, retained together while fading. */
function RaceCell({ row, label, active, leaving, owner, onJump }: {
  row: RaceRow; label: CountryAt; active: boolean; leaving: boolean; owner: string;
  onJump: (row: RaceRow, x: number, y: number) => void;
}) {
  const decryptor = row.tags.ownerTag == null ? { name: `Country ${row.pair.ownerId} (tag unavailable)`, flag: undefined } : label(row.tags.ownerTag);
  const target = row.tags.targetTag == null ? { name: `Country ${row.pair.targetId} (tag unavailable)`, flag: undefined } : label(row.tags.targetTag);
  const [shownLabels, setShownLabels] = useState<{ decryptor: ReturnType<CountryAt>; target: ReturnType<CountryAt> } | null>(null);
  useEffect(() => {
    if (!leaving) setShownLabels(previous => previous?.decryptor.name === decryptor.name && previous.target.name === target.name
      && previous.decryptor.flag === decryptor.flag && previous.target.flag === target.flag
      ? previous : { decryptor: { name: decryptor.name, flag: decryptor.flag }, target: { name: target.name, flag: target.flag } });
  }, [leaving, decryptor.name, decryptor.flag, target.name, target.flag]);
  // A fading row keeps its last displayed names and flags from the same look.
  const decryptorName = leaving ? shownLabels?.decryptor.name ?? row.tags.ownerTag ?? decryptor.name : decryptor.name;
  const targetName = leaving ? shownLabels?.target.name ?? row.tags.targetTag ?? target.name : target.name;
  const decryptorFlag = leaving ? shownLabels?.decryptor.flag : decryptor.flag;
  const targetFlag = leaving ? shownLabels?.target.flag : target.flag;
  const height = useViewportHeight();
  const content = useMemo(() => decryptionTooltip(row, decryptorName, targetName, height), [row, decryptorName, targetName, height]);
  const [hover, setHover] = useState<{ x: number; y: number } | null>(null);
  useEffect(() => { if (hover && active) showTooltip(owner, content, hover.x, hover.y); else hideTooltip(owner); }, [hover, active, owner, content]);
  useEffect(() => () => hideTooltip(owner), [owner]);
  const point = (element: HTMLElement) => { const rect = element.getBoundingClientRect(); return { x: rect.right - 8, y: rect.top + rect.height / 2 }; };
  const broken = row.run.broken === true;
  const bonus = activeBonusDays(row.run);
  const live = active && !broken && row.run.decrypting !== 0 && row.run.progressPercent != null;
  return <div className="decryption-row" tabIndex={leaving ? -1 : 0} aria-label={tooltipText(content)}
    onPointerMove={event => setHover({ x: event.clientX, y: event.clientY })} onPointerLeave={() => setHover(null)}
    onFocus={event => setHover(point(event.currentTarget))} onBlur={() => setHover(null)}
    onContextMenu={event => { event.preventDefault(); hideTooltip(owner); onJump(row, event.clientX, event.clientY); }}
    onKeyDown={event => {
      if (event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) {
        event.preventDefault(); const at = point(event.currentTarget); hideTooltip(owner); onJump(row, at.x, at.y);
      } else if (event.key === 'Escape') setHover(null);
    }}>
    <div className="decryption-pair">
      <span className="decryption-flag" aria-hidden="true">{decryptorFlag && <img src={decryptorFlag} alt="" />}</span>
      <small aria-hidden="true">→</small>
      <span className="decryption-flag" aria-hidden="true">{targetFlag && <img src={targetFlag} alt="" />}</span>
      <small className="decryption-target">{targetName}</small>
    </div>
    <div className="decryption-progress" aria-hidden="true">
      <ProgressMeter value={row.run.progressPercent == null ? null : row.run.progressPercent / 100}
        state={broken ? 'done' : live ? 'live' : 'idle'} />
      {broken && row.run.progressPercent == null && <small className="decryption-unknown">{progressText(row.run)}</small>}
    </div>
    <div className="decryption-status">
      <b>{broken ? CIPHER_BROKEN : progressLabel(row.run)}</b>
      <span>{broken ? observationDate(row.broken, row.hour) : workingStatus(row.run)}</span>
      {!broken && row.run.progressPercent == null && <span className="decryption-amount">{amountText(row.run)}</span>}
      {bonus != null && <span className="decryption-badge">{bonus >= 0 ? '+' : ''}{bonus} bonus days</span>}
    </div>
  </div>;
}
