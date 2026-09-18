import { lazy, Suspense, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { FormEvent, RefObject } from 'react';
import {
  desktopAvailable, errorMessage, getCountrySeries, getReplayMetadata,
  recordingName, requestCancelled,
} from '../api';
import type { CountrySeriesRequest, ViewerState } from '../api';
import { Icon } from '../Icons';
import {
  countryLabel, countryOptions, coverageLabels, emptyMemory, formatHour,
  formatMetric, metrics, parseRange, retainedSamples,
} from './model';
import type { AnalyticsMemory } from './model';
import { formatDateTime } from './calendar';
import DateTimePicker from './DateTimePicker';

const HistoryChart = lazy(() => import('./HistoryChart'));

interface Props {
  path: string | null;
  memory: RefObject<AnalyticsMemory | null>;
  viewer: ViewerState | null;
  viewerRunning: boolean;
  viewerReady: boolean;
  opening: boolean;
  openError: string | null;
  onOpen: () => void;
  onViewOnMap: (branch: string, hour: number, revision: string) => Promise<void>;
}

export default function Analytics({ path, memory, viewer, viewerRunning, viewerReady, opening, openError, onOpen, onViewOnMap }: Props) {
  const [data, setData] = useState<AnalyticsMemory>(() => memory.current?.path === path ? memory.current : emptyMemory(path));
  const current = useRef(data);
  const scroll = useRef<HTMLDivElement>(null);
  const scrollTop = useRef(data.scrollTop);
  const activeRequest = useRef<AbortController | null>(null);
  const [metadataRetry, setMetadataRetry] = useState(0);
  const [seriesRetry, setSeriesRetry] = useState(0);
  const [metadataError, setMetadataError] = useState<string | null>(null);
  const [seriesError, setSeriesError] = useState<string | null>(null);
  const [loadState, setLoadState] = useState<'idle' | 'loading' | 'cancelled' | 'error'>('idle');
  const [seeking, setSeeking] = useState(false);
  const [seekError, setSeekError] = useState<string | null>(null);
  const [showIntervals, setShowIntervals] = useState(false);
  const viewerAvailable = viewerRunning && viewerReady;
  const viewerUnavailableReason = viewerRunning ? 'Native viewer is not ready' : 'Native viewer is not running';

  const update = useCallback((change: (previous: AnalyticsMemory) => AnalyticsMemory) => {
    const next = { ...change(current.current), scrollTop: scrollTop.current };
    current.current = next;
    memory.current = next;
    setData(next);
  }, [memory]);

  useLayoutEffect(() => {
    memory.current = current.current;
    if (scroll.current) scroll.current.scrollTop = scrollTop.current;
    return () => { memory.current = { ...current.current, scrollTop: scrollTop.current }; };
  }, [memory]);

  useEffect(() => {
    if (!path || !desktopAvailable || current.current.metadata) return;
    const controller = new AbortController();
    let active = true;
    setMetadataError(null);
    void getReplayMetadata(path, controller.signal).then((metadata) => {
      if (!active) return;
      update((previous) => {
        const branch = metadata.branches.find((item) => item.id === previous.branchId)
          ?? metadata.branches.find((item) => viewer?.path === path && viewer.revision === metadata.revision && item.id === viewer.branch)
          ?? metadata.branches[0];
        const options = countryOptions(metadata.countries, branch?.id ?? '');
        const country = options.find((item) => item.id === previous.countryId)
          ?? options.find((item) => item.id === metadata.playerCountryId)
          ?? options[0];
        const priorRange = previous.branchId === branch?.id ? previous.range : null;
        const range = branch ? priorRange && priorRange.firstHour >= branch.firstHour && priorRange.lastHour <= branch.lastHour
          ? priorRange : { firstHour: branch.firstHour, lastHour: branch.lastHour } : null;
        return {
          ...previous, metadata, branchId: branch?.id ?? '', countryId: country?.id ?? '', range,
          fromInput: range ? String(range.firstHour) : '', toInput: range ? String(range.lastHour) : '',
        };
      });
    }).catch((cause: unknown) => {
      if (active && !requestCancelled(cause)) setMetadataError(errorMessage(cause));
    });
    return () => { active = false; controller.abort(); };
    // The native displayed branch is an initial preference, never a live query driver.
  }, [path, metadataRetry, update]);

  const metadata = data.metadata;
  const branch = metadata?.branches.find((item) => item.id === data.branchId);
  const request: CountrySeriesRequest | null = metadata && data.branchId && data.countryId && data.range ? {
    branchId: data.branchId, countryId: data.countryId, maxPoints: 1200,
    firstHour: data.range.firstHour, lastHour: data.range.lastHour,
  } : null;
  const queryKey = request && path ? JSON.stringify([path, request]) : null;
  const series = data.seriesKey === queryKey ? data.series : null;
  const countries = useMemo(() => countryOptions(
    metadata?.countries ?? [], data.branchId,
    data.series?.branchId === data.branchId ? data.series.countries : [],
  ), [metadata, data.branchId, data.series]);

  useEffect(() => {
    if (!path || !queryKey || !request || !desktopAvailable) return;
    if (current.current.seriesKey === queryKey && current.current.series) {
      setLoadState('idle');
      return;
    }
    const controller = new AbortController();
    activeRequest.current = controller;
    let active = true;
    setLoadState('loading');
    setSeriesError(null);
    setSeekError(null);
    void getCountrySeries(path, request, controller.signal).then((result) => {
      if (!active) return;
      if (result.revision !== current.current.metadata?.revision) {
        setSeriesError('This recording changed after its details were loaded. Refresh the recording to read its current history.');
        setLoadState('error');
        return;
      }
      const samples = retainedSamples(result);
      update((previous) => ({
        ...previous, series: result, seriesKey: queryKey,
        selectedHour: samples.some((sample) => sample.hour === previous.selectedHour)
          ? previous.selectedHour : samples.at(-1)?.hour ?? null,
      }));
      setLoadState('idle');
    }).catch((cause: unknown) => {
      if (!active) return;
      if (requestCancelled(cause)) {
        setLoadState('cancelled');
      } else {
        setSeriesError(errorMessage(cause));
        setLoadState('error');
      }
    }).finally(() => {
      if (activeRequest.current === controller) activeRequest.current = null;
    });
    return () => { active = false; controller.abort(); };
    // queryKey contains every serialized query argument and the recording path.
  }, [path, queryKey, seriesRetry, update]);

  const samples = useMemo(() => series ? retainedSamples(series) : [], [series]);
  const selected = samples.find((sample) => sample.hour === data.selectedHour) ?? samples.at(-1) ?? null;
  const metric = metrics.find((item) => item.key === data.metric)!;
  const rangeResult = branch ? parseRange(data.fromInput, data.toInput, branch) : null;
  const rangeError = typeof rangeResult === 'string' ? rangeResult : null;
  const selectedCountry = series?.country ?? countries.find((item) => item.id === data.countryId);
  const notices = [...new Set([...(metadata?.warnings ?? []), ...(series?.warnings ?? [])])];
  const selectedMetricMissing = series?.points.reduce((sum, point) => sum + point.missingMetricHours[data.metric], 0) ?? 0;

  const selectHour = useCallback((hour: number) => {
    update((previous) => ({ ...previous, selectedHour: hour }));
  }, [update]);

  function applyRange(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!rangeResult || typeof rangeResult === 'string') return;
    update((previous) => ({ ...previous, range: rangeResult }));
  }

  function refreshRecording() {
    activeRequest.current?.abort();
    setMetadataError(null);
    setSeriesError(null);
    setSeekError(null);
    update((previous) => ({ ...previous, metadata: null, series: null, seriesKey: null }));
    setMetadataRetry((value) => value + 1);
  }

  async function seekSelected() {
    if (!selected || !series || seeking) return;
    setSeeking(true);
    setSeekError(null);
    try {
      await onViewOnMap(series.branchId, selected.hour, series.revision);
    } catch (cause) {
      setSeekError(errorMessage(cause));
    } finally {
      setSeeking(false);
    }
  }

  return (
    <div className="analytics-scroll" ref={scroll} onScroll={(event) => {
      scrollTop.current = event.currentTarget.scrollTop;
    }}>
      <div className="analytics-content">
        <header className="page-heading">
          <div>
            <span className="eyebrow">Campaign history</span>
            <h1>Analytics</h1>
            <p className="page-description">Explore a country's recorded history, one branch at a time.</p>
          </div>
          {path && <div className="recording-badge" title={path}>
            <span className="file-name">{metadata?.name ?? recordingName(path)}</span>
            <span className="file-detail">{metadata ? `Format ${metadata.format} · ${metadata.branches.length} ${metadata.branches.length === 1 ? 'branch' : 'branches'}` : 'Reading recording details…'}</span>
            {desktopAvailable && metadata && <button type="button" className="text-button recording-refresh" onClick={refreshRecording}>Refresh recording</button>}
          </div>}
        </header>

        {!desktopAvailable && <div className="browser-notice" role="note">
          Browser preview. Opening recordings, querying history and controlling the native map require the desktop app.
        </div>}
        {openError && <div className="notice error" role="alert">
          <p>{openError}</p>
          <button type="button" className="button small" disabled={opening} onClick={onOpen}>Retry opening</button>
        </div>}

        {!path ? (
          <div className="empty-state">
            <div className="placeholder-icon"><Icon name="chart" /></div>
            <h2>A closer look at your campaign</h2>
            <p>Open a recording to explore political power, command power, stability, war support and convoys over time.</p>
            <button type="button" className="button primary" disabled={!desktopAvailable || opening} onClick={onOpen}>
              <Icon name="folder" />Open recording
            </button>
          </div>
        ) : metadataError ? (
          <div className="empty-state" role="alert">
            <h2>The recording could not be read</h2>
            <p className="error-copy">{metadataError}</p>
            <div className="button-row">
              <button type="button" className="button primary" onClick={() => setMetadataRetry((value) => value + 1)}><Icon name="refresh" />Retry</button>
              <button type="button" className="button" onClick={onOpen} disabled={opening}><Icon name="folder" />Open another recording</button>
            </div>
          </div>
        ) : !metadata ? (
          <div className="empty-state" role="status">
            <span className="spinner" />
            <h2 className="loading-title">Reading recording details</h2>
            <p>Finding recorded branches and country identities.</p>
          </div>
        ) : (
          <>
            <form className="control-panel" onSubmit={applyRange} noValidate>
              <div className="control-grid">
                <label className="field"><span>Country</span>
                  <select value={data.countryId} disabled={!countries.length && !data.countryId} onChange={(event) => {
                    const countryId = event.currentTarget.value;
                    update((previous) => ({ ...previous, countryId }));
                  }}>
                    {!countries.length && !data.countryId && <option value="">No countries available</option>}
                    {data.countryId && !countries.some((country) => country.id === data.countryId) && (
                      <option value={data.countryId}>Country ID {data.countryId} · Not in current catalogue</option>
                    )}
                    {countries.map((country) => <option key={country.id} value={country.id}>{countryLabel(country)}</option>)}
                  </select>
                </label>
                <label className="field"><span>Branch</span>
                  <select value={data.branchId} disabled={!metadata.branches.length} onChange={(event) => {
                    const next = metadata.branches.find((item) => item.id === event.currentTarget.value);
                    if (!next) return;
                    update((previous) => ({
                      ...previous, branchId: next.id, fromInput: String(next.firstHour), toInput: String(next.lastHour),
                      range: { firstHour: next.firstHour, lastHour: next.lastHour },
                    }));
                  }}>
                    {!metadata.branches.length && <option value="">No branches available</option>}
                    {metadata.branches.map((item) => <option key={item.id} value={item.id}>Branch {item.id}</option>)}
                  </select>
                </label>
                <DateTimePicker key={`from-${branch?.id}-${branch?.firstHour}-${branch?.lastHour}`} label="From"
                  value={data.fromInput === '' ? null : Number(data.fromInput)}
                  otherValue={data.toInput === '' ? null : Number(data.toInput)}
                  min={branch?.firstHour ?? 0} max={branch?.lastHour ?? 0} disabled={!branch} invalid={!!rangeError}
                  onChange={(hour) => update((previous) => ({ ...previous, fromInput: String(hour) }))} />
                <DateTimePicker key={`to-${branch?.id}-${branch?.firstHour}-${branch?.lastHour}`} label="To"
                  value={data.toInput === '' ? null : Number(data.toInput)}
                  otherValue={data.fromInput === '' ? null : Number(data.fromInput)}
                  min={branch?.firstHour ?? 0} max={branch?.lastHour ?? 0} disabled={!branch} invalid={!!rangeError}
                  onChange={(hour) => update((previous) => ({ ...previous, toInput: String(hour) }))} />
                <button type="submit" className="button apply-range" disabled={!branch || !!rangeError}>Apply range</button>
              </div>
              <div className="control-footnote">
                <span id="date-range-help">{branch ? `Available · ${formatDateTime(branch.firstHour)} — ${formatDateTime(branch.lastHour)}. ` : ''}Dates and times are inclusive.</span>
                {branch && <button type="button" className="text-button" onClick={() => {
                  update((previous) => ({
                    ...previous, fromInput: String(branch.firstHour), toInput: String(branch.lastHour),
                    range: { firstHour: branch.firstHour, lastHour: branch.lastHour },
                  }));
                }}>Full branch</button>}
              </div>
              <p id="range-error" className="form-error" role={rangeError ? 'alert' : undefined} hidden={!rangeError}>{rangeError}</p>
            </form>

            {!branch || (!countries.length && !data.countryId) ? <div className="empty-state">
              <h2>{!branch ? 'No recorded branches' : 'No country identities available'}</h2>
              <p>{!branch ? 'This recording does not contain a branch to analyse.' : 'The inspected snapshots did not provide readable country identities. Their absence is not treated as an empty country history.'}</p>
              <button type="button" className="button" disabled={opening} onClick={onOpen}><Icon name="folder" />Open another recording</button>
            </div> : !series ? (
              <div className="empty-state" role={seriesError ? 'alert' : 'status'}>
                {loadState === 'loading' && <span className="spinner" />}
                <h2 className="loading-title">{seriesError ? 'History could not be read' : loadState === 'cancelled' ? 'History request cancelled' : 'Reading country history'}</h2>
                <p className={seriesError ? 'error-copy' : ''}>{seriesError ?? (loadState === 'cancelled' ? 'Your country and range selections are retained.' : 'Reading the selected branch and preserving unavailable hours and recorded extrema.')}</p>
                {loadState === 'loading' ? <button type="button" className="button quiet" onClick={() => {
                  activeRequest.current?.abort();
                  setLoadState('cancelled');
                }}>Cancel</button> : <div className="button-row">
                  <button type="button" className="button" onClick={() => setSeriesRetry((value) => value + 1)}><Icon name="refresh" />Retry history</button>
                  {seriesError && <button type="button" className="button quiet" onClick={refreshRecording}>Refresh recording</button>}
                </div>}
              </div>
            ) : (
              <>
                <div className="section-heading">
                  <h2>{selectedCountry ? countryLabel(selectedCountry) : `Country ${data.countryId}`}</h2>
                  <p>{selected ? `Selected sample · ${formatDateTime(selected.hour)}` : 'No recorded country samples in this range'}</p>
                </div>
                <div className="metric-grid" role="group" aria-label="Metric to chart">
                  {metrics.map((item) => (
                    <button type="button" key={item.key} className="metric-card" aria-pressed={data.metric === item.key}
                      onClick={() => update((previous) => ({ ...previous, metric: item.key }))}>
                      <span className="metric-label">{item.label}</span>
                      <span className="metric-value" title={formatMetric(selected?.values[item.key], series.valueScale, item.key)}>
                        {selected?.values[item.key] == null ? '—' : formatMetric(selected.values[item.key], series.valueScale, item.key)}
                      </span>
                      <span className="metric-subtitle">{selected?.values[item.key] == null ? 'Unavailable at this sample' : data.metric === item.key ? 'Showing history' : 'View history'}</span>
                    </button>
                  ))}
                </div>

                {seekError && <div className="notice error" role="alert"><p>{seekError}</p><div className="button-row">
                  <button type="button" className="button small" onClick={refreshRecording}>Refresh analysis</button>
                  <button type="button" className="button small quiet" onClick={() => void seekSelected()}>Retry map</button>
                </div></div>}

                <div className="history-card">
                  <div className="chart-heading">
                    <div><h2>{metric.label}</h2><p>Analysis range · {formatDateTime(series.firstHour)} — {formatDateTime(series.lastHour)}</p></div>
                    <div className="chart-legend" aria-label="Chart legend">
                      <span><i className="legend-line" />Recorded observations</span>
                      {series.downsampled && <span><i className="legend-line range" />Interval low / high</span>}
                    </div>
                  </div>
                  <Suspense fallback={<div className="empty-state compact" role="status"><span className="spinner" /><p className="loading-title">Preparing chart…</p></div>}>
                    <HistoryChart series={series} metric={data.metric} onSelectHour={selectHour} />
                  </Suspense>
                  <div className="chart-caption">
                    <span>{series.downsampled ? 'The line joins retained observations; dashed bounds show exact interval extrema. The intervening path is not retained.' : 'Select a plotted observation to inspect it. Missing values break the line.'}</span>
                    <span>{formatHour(series.points.length)} {series.downsampled ? 'intervals' : 'points'}</span>
                  </div>
                  <div className="sample-bar">
                    <div><span className="sample-label">Selected recorded sample</span><span className="sample-hour">{selected ? formatDateTime(selected.hour) : 'Unavailable in this range'}</span></div>
                    <div className="sample-controls">
                      <label className="sr-only" htmlFor="recorded-sample">Recorded sample to inspect</label>
                      <select id="recorded-sample" disabled={!samples.length} value={selected?.hour ?? ''} onChange={(event) => selectHour(Number(event.currentTarget.value))}>
                        {!samples.length && <option value="">No recorded samples</option>}
                        {samples.map((sample) => <option key={sample.hour} value={sample.hour}>{formatDateTime(sample.hour)}</option>)}
                      </select>
                      <button type="button" className="button primary" disabled={!selected || seeking} onClick={() => void seekSelected()}>
                        {seeking ? <span className="spinner" /> : <Icon name="map" />}{seeking ? 'Opening map…' : 'View on map'}
                      </button>
                    </div>
                  </div>
                </div>

                <div className="coverage-strip" aria-label="Recording coverage for the analysis range">
                  <span><strong>{formatHour(series.coverage.recordedHours)}</strong> recorded hours</span>
                  {!!series.coverage.gapHours && <span className="coverage-warning"><strong>{formatHour(series.coverage.gapHours)}</strong> gap hours</span>}
                  {!!series.coverage.unavailableHours && <span className="coverage-warning"><strong>{formatHour(series.coverage.unavailableHours)}</strong> unavailable hours</span>}
                  {!!series.coverage.absentHours && <span><strong>{formatHour(series.coverage.absentHours)}</strong> country-absent hours</span>}
                  {!!selectedMetricMissing && <span className="coverage-warning">{formatHour(selectedMetricMissing)} recorded hours missing {metric.label.toLowerCase()}</span>}
                </div>

                <details className="recording-notes" onToggle={(event) => setShowIntervals(event.currentTarget.open)}>
                  <summary>Exact values and recording gaps · {formatHour(series.points.length)} intervals</summary>
                  <p>Values below retain the recorded precision. An interval's minimum and maximum are not assigned an invented sample hour.</p>
                  {showIntervals && <div className="data-table-wrap">
                    <table className="data-table">
                      <caption className="sr-only">{metric.label}: retained first and last observations, exact interval extrema, and coverage.</caption>
                      <thead><tr><th scope="col">Date & time</th><th scope="col">Coverage</th><th scope="col">First observation</th><th scope="col">Last observation</th><th scope="col">Low</th><th scope="col">High</th><th scope="col">Notes</th></tr></thead>
                      <tbody>{series.points.map((point, index) => <tr key={`${point.firstHour}-${index}`}>
                        <td title={`Engine hours ${point.firstHour}–${point.lastHour}`}>{formatDateTime(point.firstHour)}{point.lastHour !== point.firstHour ? ` — ${formatDateTime(point.lastHour)}` : ''}</td>
                        <td>{coverageLabels[point.status]}</td>
                        <td>{point.first ? `${formatMetric(point.first.values[data.metric], series.valueScale, data.metric)} at ${formatDateTime(point.first.hour)}` : 'Unavailable'}</td>
                        <td>{point.last ? `${formatMetric(point.last.values[data.metric], series.valueScale, data.metric)} at ${formatDateTime(point.last.hour)}` : 'Unavailable'}</td>
                        <td>{formatMetric(point.min[data.metric], series.valueScale, data.metric)}</td>
                        <td>{formatMetric(point.max[data.metric], series.valueScale, data.metric)}</td>
                        <td>{point.reason ?? (point.missingMetricHours[data.metric] ? `${formatHour(point.missingMetricHours[data.metric])} hours missing this metric` : '—')}</td>
                      </tr>)}</tbody>
                    </table>
                  </div>}
                </details>
              </>
            )}

            <details className="recording-notes">
              <summary>Recording details{notices.length ? ` · ${notices.length} ${notices.length === 1 ? 'note' : 'notes'}` : ''}</summary>
              <p>{metadata.contentVerified ? 'The recording carries a verified content identity from capture time.' : 'The recording does not carry a verified content identity from capture time.'} Analytics does not check local map assets.</p>
              <p>The country list reflects observed snapshots and may grow after a branch is read.</p>
              {branch && <p>Branch {branch.id} · Session {branch.sessionId}. {branch.parentId ? `Probable parent: branch ${branch.parentId}${branch.parentHour != null ? ` at ${formatDateTime(branch.parentHour)}` : ''}.` : 'Parentage is unknown.'}</p>}
              {!!(series?.sampleBoundary ?? branch?.sampleBoundary) && <p>Sample boundary: {series?.sampleBoundary ?? branch?.sampleBoundary}</p>}
              {notices.length > 0 && <ul>{notices.map((notice) => <li key={notice}>{notice}</li>)}</ul>}
            </details>
          </>
        )}

        <div className="map-position">
          <Icon name="map" />
          <span>{viewer?.path && viewer.hour != null ? <>
            <strong>Map</strong> · {formatDateTime(viewer.hour)} · Branch {viewer.branch ?? 'unknown'}
            {viewer.path !== path && ` · ${recordingName(viewer.path)}`}
            {!viewerAvailable && ` · ${viewerUnavailableReason}`}
          </> : <><strong>Map</strong> · {viewerAvailable ? 'No hour displayed yet' : viewerUnavailableReason}</>}</span>
        </div>
      </div>
    </div>
  );
}
