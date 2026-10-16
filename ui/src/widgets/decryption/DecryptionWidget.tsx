import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { HTMLAttributes } from 'react';
import WidgetState from '../../components/WidgetState';
import { CountryFlagUrl } from '../../components/CountryFlag';
import { useContextMenu } from '../../components/ContextMenu';
import type { MenuItem } from '../../components/ContextMenu';
import WindowedRows from '../../charts/WindowedRows';
import type { MovingRow } from '../../charts/rowMotion';
import { ProgressMeter } from '../../charts/meter';
import { jumpItem } from '../../charts/jump';
import { runAt } from '../../charts/series';
import { tooltipText } from '../../charts/tooltip';
import type { TooltipContent } from '../../charts/tooltip';
import { useRowTooltips } from '../../charts/useRowTooltips';
import { useViewportHeight } from '../../charts/useChartTokens';
import { selectedTags } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useParityRows } from '../../dashboard/parityRows';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import type { DecryptionReply } from '../../generated/dashboard';
import { OPTIONS } from './definition';
import { activeBonusDays, amountText, CIPHER_BROKEN, decryptionTooltip, observationDate, progressLabel, progressText, raceRows, validateDecryptionReply, workingStatus } from './decryptionData';
import type { CountrySide, RaceOrder, RaceRow } from './decryptionData';
import { useDecryptionCountries } from './decryptionCountries';
import type { CipherCountries } from './decryptionCountries';
import './decryption.css';

const NO_WARNINGS: string[] = [];

export default function DecryptionWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = window?.lastHour ?? 0;
  const first = track ? track.branch.firstHour + Math.floor((hour - track.branch.firstHour) / 24) * 24 : hour;
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'decryption', params: runtime.client.preparedQuery
    ? { branchId: track.branch.id, firstHour: first, lastHour: Math.min(first + 23, track.branch.lastHour) }
    : { branchId: track.branch.id } } : null;
  const { data, error, retry } = useProduct<DecryptionReply>(runtime.client, request, active, validateDecryptionReply, track?.source);
  const groups = useCountryGroups(runtime, track, active && widget.countries.kind !== 'all');
  const selection = useMemo(() => {
    return countryMatch(widget.countries, selectedTags(widget.countries, track), groups.data, track?.branch.lastHour ?? hour);
  }, [widget.countries, track, groups.data, groups.error, hour]);
  const rows = useMemo(() => data && selection.ready ? raceRows(data, hour, options.order as RaceOrder, null, options.countrySide as CountrySide,
    widget.countries.kind === 'all' ? undefined : selection.matches) : [],
    [data, hour, options.order, options.countrySide, selection, widget.countries.kind]);
  const items = useMemo(() => rows.map(item => ({ key: item.key, item })), [rows]);
  const root = useRef<HTMLDivElement>(null);
  useParityRows(root, () => ({ kind: 'table', label: 'Decryption values',
    complete: Boolean(data && selection.ready), total: rows.length,
    rows: rows.map(row => ({ key: row.key, hour: row.hour, ownerId: row.pair.ownerId, targetId: row.pair.targetId,
      run: row.run, tags: row.tags, broken: row.broken })),
  }));
  const [visibleRows, setVisibleRows] = useState<readonly MovingRow<RaceRow>[]>([]);
  const countries = useDecryptionCountries(runtime.client, track, rows, visibleRows, active);
  const menu = useContextMenu('Cipher actions');
  const height = useViewportHeight();
  const byKey = useMemo(() => new Map(rows.map(row => [row.key, row])), [rows]);
  const content = useCallback((key: string) => {
    const row = byKey.get(key);
    if (!row) return null;
    const labels = countries.pair(row);
    return decryptionTooltip(row, labels.owner, labels.target, height);
  }, [byKey, countries, height]);
  const jump = (key: string, x: number, y: number) => {
    const row = byKey.get(key);
    if (!track || !row) return;
    const items: MenuItem[] = [];
    if (row.run.started) items.push({ ...jumpItem(runtime, track.id, row.run.started.hour),
      label: `Jump to the start of the current attempt · ${observationDate(row.run.started, row.hour)}` });
    if (row.run.broken === true && row.broken) items.push({ ...jumpItem(runtime, track.id, row.broken.hour),
      label: `Jump to the break · ${observationDate(row.broken, row.hour)}` });
    if (items.length) menu.open(x, y, items);
  };
  const tips = useRowTooltips<string>({ owner: `decryption:${widget.id}`, active, content,
    revision: `${hour}:${track?.source.revision}:${track?.branch.id}:${countries.revision}`, onMenu: jump });
  useEffect(() => { onDiagnostics(data?.warnings ?? NO_WARNINGS); return () => onDiagnostics(NO_WARNINGS); }, [data, onDiagnostics]);
  if (!track) return <WidgetState kind="empty" icon="target" message="Load a campaign to see its decryption races." />;
  if (!window || hour < track.branch.firstHour || hour > track.branch.lastHour) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (widget.countries.kind !== 'all' && groups.error) return <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} />;
  if (!data || !selection.ready) return <WidgetState kind="loading" label="Loading decryption…" />;
  const coverage = runAt(data.coverage, hour);
  if (!coverage || (!coverage.complete && !rows.length)) return <WidgetState kind="unavailable"
    message="Decryption data is unavailable at this time." />;
  if (selection.reason) return <WidgetState kind="unavailable" message={selection.reason} />;
  return <div ref={root} className="decryption" key={JSON.stringify([track.source.id, track.source.revision, track.branch.id])}>
    <WindowedRows items={items} rowHeight={52} padding={8} label="Decryption races" className="decryption-race"
      onVisibleRows={setVisibleRows} empty={<WidgetState kind="empty" message={widget.countries.kind === 'all' ? 'No ciphers being decrypted yet.' : 'No matching decryption pairs at this time.'} />}>
      {row => <RaceCell row={row.item} labels={countries.pair(row.item)} active={active && row.phase !== 'leaving'} leaving={row.phase === 'leaving'}
        rowProps={row.phase === 'leaving' ? undefined : tips.rowProps(row.key)} descriptionId={tips.descriptionId(row.key)}
        content={row.phase === 'leaving' ? null : content(row.key)} />}
    </WindowedRows>
    {menu.menu}
  </div>;
}

/** Country labels retain their supplying native segment while fading. */
function RaceCell({ row, labels, active, leaving, rowProps, descriptionId, content }: {
  row: RaceRow; labels: CipherCountries; active: boolean; leaving: boolean;
  rowProps: HTMLAttributes<HTMLDivElement> | undefined; descriptionId: string; content: TooltipContent | null;
}) {
  const decryptor = labels.owner;
  const target = labels.target;
  const [shownLabels, setShownLabels] = useState<{ decryptor: ReturnType<CountryLabel>; target: ReturnType<CountryLabel> } | null>(null);
  useEffect(() => {
    if (!leaving) setShownLabels(previous => previous?.decryptor.name === decryptor.name && previous.target.name === target.name
      && previous.decryptor.flag === decryptor.flag && previous.target.flag === target.flag
      && previous.decryptor.flagReason === decryptor.flagReason && previous.target.flagReason === target.flagReason
      && previous.decryptor.flagPending === decryptor.flagPending && previous.target.flagPending === target.flagPending
      ? previous : {
        decryptor: { name: decryptor.name, flag: decryptor.flag, flagReason: decryptor.flagReason, flagPending: decryptor.flagPending },
        target: { name: target.name, flag: target.flag, flagReason: target.flagReason, flagPending: target.flagPending },
      });
  }, [leaving, decryptor.name, decryptor.flag, decryptor.flagReason, decryptor.flagPending, target.name, target.flag, target.flagReason, target.flagPending]);
  // A fading row keeps its last displayed names and flags from the same look.
  const shown = leaving ? shownLabels ?? { decryptor, target } : { decryptor, target };
  const broken = row.run.broken === true;
  const bonus = activeBonusDays(row.run);
  const live = active && !broken && row.run.decrypting !== 0 && row.run.progressPercent != null;
  return <div className="decryption-row" {...rowProps} tabIndex={leaving ? -1 : 0}>
    {content && <span id={descriptionId} className="sr-only">{tooltipText(content)}</span>}
    <div className="decryption-pair">
      <span className="decryption-flag">{row.tags.ownerTag !== null && <CountryFlagUrl url={shown.decryptor.flag} reason={shown.decryptor.flagReason} pending={shown.decryptor.flagPending} />}</span>
      <small aria-hidden="true">→</small>
      <span className="decryption-flag">{row.tags.targetTag !== null && <CountryFlagUrl url={shown.target.flag} reason={shown.target.flagReason} pending={shown.target.flagPending} />}</span>
      <small className="decryption-target">{shown.target.name}</small>
    </div>
    <div className="decryption-progress" aria-hidden="true">
      <ProgressMeter value={row.run.progressPercent == null ? null : row.run.progressPercent / 100}
        state={broken ? 'done' : live ? 'live' : 'idle'} sweep />
      {broken && row.run.progressPercent == null && <small className="decryption-unknown">{progressText(row.run)}</small>}
    </div>
    <div className="decryption-status">
      <b>{broken ? CIPHER_BROKEN : progressLabel(row.run)}</b>
      <span>{broken ? observationDate(row.broken, row.hour) : workingStatus(row.run)}</span>
      {!broken && row.run.progressPercent == null && <span className="decryption-amount">{amountText(row.run)}</span>}
      {bonus != null && <span className="pill warning">{bonus >= 0 ? '+' : ''}{bonus} bonus days</span>}
    </div>
  </div>;
}
