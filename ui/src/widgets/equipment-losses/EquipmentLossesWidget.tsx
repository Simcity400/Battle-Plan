import CombatLogDetails from '../land-battles/CombatLogDetails';
import { narrowTableOf } from '../../charts/table/options.ts';
import { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import DatedCountryChip from '../../dashboard/DatedCountryChip';
import { countryDrillTagAt } from '../../dashboard/countryIdentity';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { chooseDrillCountry, drillCountryCoverage, drillCountrySpans, readDrillCountry } from '../country-measure/drillCountry';
import WidgetState from '../../components/WidgetState';
import { useContextMenu } from '../../components/ContextMenu';
import { dayLabel, drawBands } from '../../charts/draw';
import { drawApproximateSpans } from '../../charts/chartSummary';
import { PreparedKey } from '../../charts/PreparedKey';
import { PreparedLogCoverage } from '../../charts/log/LogList';
import DrillHost from '../../charts/drill/DrillHost';
import DrillParts from '../../charts/drill/DrillParts';
import { CLOSED, reduceDrill } from '../../charts/drill/drillState';
import type { DrillState } from '../../charts/drill/drillState';
import { jumpItem } from '../../charts/jump';
import { stackExtent } from '../../charts/stack';
import Sunburst from '../../charts/sunburst/Sunburst';
import type { SunburstText } from '../../charts/sunburst/Sunburst';
import { levelsShown, sunburstSettings } from '../../charts/sunburst/sunburstOptions';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import { fitTooltip } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { formatDateTime } from '../../dashboard/calendar';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { NumericReply } from '../../dashboard/numericReply';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour, usePlaybackSpeed } from '../../dashboard/playback';
import { preparedLossHour, preparedLossWindow, usePreparedLosses } from './preparedLosses';
import type { PreparedLosses } from './preparedLosses';
import { entryAt } from '../../dashboard/reads';
import type { LossesParams } from '../../generated/dashboard';
import { DETAIL, OPTIONS } from './definition';
import { LEVELS, coverageLegend, destructionLegend, formatLosses, lossItems, lossesTotal, lossesFootnote,
  lossWindow, lossKey, unknownCauses } from './lossesData';
import { lossHistory, lossNumbers, lossParts, lossRows, lossSegments, pinLosses, readLossSelection } from './lossDrill';
import type { LossRow } from './lossDrill';
import { useParityRows } from '../../dashboard/parityRows';
import { lossesParity } from './lossesParity';
import './equipmentLosses.css';

const ROOT = { id: 'all', label: 'All losses' }, NO_WARNINGS: string[] = [];
const rowKey = (row: LossRow) => row.id;
function OriginalEquipmentLossesWidget({ widget, runtime, active, countrySlot, onDiagnostics }: WidgetProps) {
  const [localDrill, setLocalDrill] = useState<{ owner: unknown; state: DrillState } | null>(null);
  const [picked, setPicked] = useState<readonly string[] | null | undefined>(undefined);
  const anchor = useRef<HTMLDivElement>(null), menu = useContextMenu('Equipment loss actions');
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  const options = resolveOptions(OPTIONS, widget.options), settings = sunburstSettings(options);
  const hasSavedCountry = Object.hasOwn(widget.options ?? {}, 'lossCountry');
  const bindingKey = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, tag, widget.time,
    hasSavedCountry, widget.options?.lossCountry, widget.options?.lossSelection, options.show]);
  const bindingOwner = useMemo(() => ({ client: runtime.client, key: bindingKey }), [runtime.client, bindingKey]);
  const drill = localDrill?.owner === bindingOwner ? localDrill.state : CLOSED;
  const setDrill = (next: DrillState | ((state: DrillState) => DrillState)) => setLocalDrill(before => ({ owner: bindingOwner,
    state: typeof next === 'function' ? next(before?.owner === bindingOwner ? before.state : CLOSED) : next }));
  const playback = usePlaybackHour(runtime.clock, active && (widget.time.mode === 'follow' || drill.breakdown !== null && drill.follow));
  const speed = usePlaybackSpeed(runtime.clock), tokens = useChartTokens(), height = useViewportHeight();
  const resolved = track ? resolveWindow(widget, track, runtime.tracks, playback) : null;
  const requestedView = track ? lossWindow(resolved, track.branch, widget.time.mode) : null;
  const saved = options.show === DETAIL.id, open = saved || drill.breakdown !== null;
  const origin = useRef<{ owner: typeof bindingOwner; hour: number } | null>(null);
  if (origin.current?.owner !== bindingOwner) origin.current = null;
  if (requestedView && !origin.current) origin.current = { owner: bindingOwner, hour: requestedView.hour };
  const catalogue = useCountryGroups(runtime, track, active);
  const savedCountry = useMemo(() => readDrillCountry(widget.options?.lossCountry, track, widget.countries),
    [widget.options?.lossCountry, track, widget.countries]);
  const selectedCountry = useMemo(() => savedCountry ?? (hasSavedCountry ? null
    : catalogue.data && track && tag && origin.current ? chooseDrillCountry(catalogue.data.countryIdentities, track, tag, origin.current.hour) : null),
    [savedCountry, hasSavedCountry, catalogue.data, track, tag, bindingOwner]);
  const actorSpans = useMemo(() => selectedCountry && catalogue.data && track ? drillCountrySpans(catalogue.data.countryIdentities, selectedCountry, track) : null,
    [selectedCountry, catalogue.data, track]);
  const following = drill.breakdown !== null ? drill.follow : widget.time.mode === 'follow';
  const detailHour = Math.floor(following ? playback + (track?.axisOffset ?? 0) : drill.hour ?? requestedView?.hour ?? 0);
  const nativeSelection: LossesParams | null = track && selectedCountry && actorSpans ? {
    branchId: track.branch.id, tag: selectedCountry.tag, landSource: options.landSource as LossesParams['landSource'],
    country: { slot: selectedCountry.slot, countryId: selectedCountry.countryId, originHour: track.branch.firstHour + selectedCountry.originOffset },
  } : null;
  const product = usePreparedLosses(runtime.client, track, nativeSelection, requestedView, detailHour, active, open);
  const request = product.request;
  const retained = useMemo(() => new NumericReply<PreparedLosses>(), [runtime.client]);
  const identity = JSON.stringify([track?.source.id, track?.source.revision, nativeSelection]);
  const numbers = retained.update(identity, product.data, reply => ({ ...reply, ...lossNumbers(reply) }));
  const data = product.data ?? numbers, contentPending = !!numbers && !product.data && !product.error;
  // The window the reply answers: before preparation reaches the requested
  // one, through its prepared prefix's last hour.
  const view = requestedView && data ? preparedLossWindow(data, requestedView) : requestedView;
  const pendingRange = !!requestedView && !!data && !view;
  const selection = useMemo(() => track && tag ? readLossSelection(widget.options?.lossSelection, track.source.metadata.campaignKey, track.branch.id, tag) : null,
    [widget.options?.lossSelection, track?.source.metadata.campaignKey, track?.branch.id, tag]);
  const selected = useMemo(() => picked === undefined ? selection ? new Set(selection.keys) : null : picked === null ? null : new Set(picked), [picked, selection]);
  const scope = JSON.stringify([identity, widget.time, widget.options?.lossSelection]);
  const pickOwner = useMemo(() => ({ client: runtime.client, scope }), [runtime.client, scope]);
  useEffect(() => { setDrill(CLOSED); setPicked(undefined); }, [pickOwner]);
  const hour = view?.hour ?? 0;
  const detailPickOwner = useMemo(() => ({ pickOwner, selected }), [pickOwner, selected]);
  const shownDetailHour = data && !preparedLossHour(data, detailHour) ? data.preparedDetailHour : detailHour;
  const detailView = view && track && detailHour >= view.first && detailHour <= (requestedView?.end ?? view.end)
    ? { ...view, hour: Math.min(shownDetailHour, view.end) } : null;
  const shownTag = actorSpans && detailView ? countryDrillTagAt(actorSpans, detailView.hour) : null;
  const countryLabels = useCountryLabelsAt(runtime.client, track, saved && savedCountry && shownTag && detailView ? [{ tag: shownTag, hour: detailView.hour }] : [], active);
  const actorReason = hasSavedCountry && !savedCountry ? 'Choose a country again.'
    : !catalogue.data ? 'Loading country…'
      : !selectedCountry || !actorSpans ? 'Country unavailable at this time.'
        : !shownTag ? 'Country unavailable at this time.' : null;
  const countryHeader = saved && countrySlot ? createPortal(<DatedCountryChip hour={detailView?.hour ?? null}
    label={savedCountry && shownTag && detailView ? countryLabels.labelAt(detailView.hour)(shownTag) : null}
    reason={hasSavedCountry ? actorReason : 'Choose a country again.'} />, countrySlot) : null;
  const items = useMemo(() => data && view ? lossItems(data, view) : [], [data, view?.first, view?.hour, view?.end, view?.from]);
  const total = useMemo(() => lossesTotal(items), [items]);
  const rows = useMemo(() => data && detailView && shownTag ? lossRows(data, detailView, selected) : [], [data, detailView?.hour, view?.first, view?.end, view?.from, selected, shownTag]);
  const coverageSpans = useMemo(() => actorSpans ? drillCountryCoverage(actorSpans) : [], [actorSpans]);
  const stack = useMemo(() => data && view ? lossHistory(data, view.first, view.end, view.from, selected, coverageSpans) : null,
    [data, view?.first, view?.end, view?.from, selected, coverageSpans]);
  const known = useMemo(() => stack?.model.hours.map((firstHour, index) => ({ firstHour, value: stack.model.edges[index]?.some(edge => edge.high > edge.low) === true })) ?? [], [stack]);
  const colors = useMemo(() => stack?.keys.map((_key, index) => tokens.categories[index % tokens.categories.length]) ?? [], [stack, tokens.categories]);
  const layers = useMemo<ChartLayer[]>(() => stack ? [{
    draw: (ctx, frame, theme, state) => {
      const to = Math.min(view?.end ?? frame.x[1], frame.x[1], state.playhead ?? frame.x[1]);
      const from = Math.min(to, Math.max(view?.first ?? frame.x[0], frame.x[0]));
      if (!data || data.preparedFrom !== from || data.preparedTo !== to
        || data.preparedWidth !== Math.ceil(frame.plotWidth)) return;
      drawApproximateSpans(ctx, frame, data.prefix?.approximate, state.playhead, () => {
        for (const segment of lossSegments(stack.model, frame.x[0], Math.min(frame.x[1], state.playhead ?? frame.x[1]),
          (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth))) drawBands(ctx, frame, theme, segment.hours, segment.bands, colors, segment);
      });
    }, extent: range => stackExtent(stack.model, range[0], range[1]),
  }] : [], [stack, colors, data, view?.first, view?.end]);
  const scale = data?.amountScale ?? 1;
  const text = useMemo<SunburstText>(() => ({ format: (value, exact) => formatLosses(value, exact, scale), unit: 'units', valueLabel: 'Losses', shares: !total.incomplete }), [scale, total.incomplete]);
  const rowText = (row: LossRow) => row.exact === null ? 'Not known' : `${row.incomplete ? 'Subtotal: ' : ''}${formatLosses(Number(row.exact), row.exact, scale)}`;
  const columns = useMemo<TableColumn<LossRow>[]>(() => [
    { key: 'cause', header: 'Cause', label: 'Cause', priority: 1, inTooltip: true, value: row => row.cause },
    { key: 'type', header: 'Equipment type', label: 'Equipment type', priority: 2, inTooltip: true, value: row => row.type },
    { key: 'design', header: 'Design', label: 'Design', value: row => row.design },
    { key: 'amount', header: 'Losses', label: 'Losses', numeric: true, width: 'max-content', value: row => row.exact, render: rowText },
  ], [scale]);
  const parts = useMemo(() => lossParts(rows, scale).map(part => ({ ...part,
    text: part.raw == null ? 'Not known' : `${part.incomplete ? 'Subtotal: ' : ''}${formatLosses(Number(part.raw), part.raw, scale)}`,
    color: colors[stack?.keys.indexOf(part.id) ?? -1] })), [rows, scale, colors, stack]);
  useEffect(() => { onDiagnostics(contentPending ? ['Equipment-loss content names are updating.'] : data?.warnings ?? NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS); }, [data, contentPending, onDiagnostics]);

  const unavailable = !track || !tag ? <WidgetState kind="empty" icon="sunburst" message="Load a campaign and choose a country to see its equipment losses." />
    : !view ? pendingRange ? <WidgetState kind="loading" label="This range is still being prepared." />
      : <WidgetState kind="unavailable" message="This time is outside the recording." />
      : selection === undefined ? <WidgetState kind="unavailable" message="Choose losses again." />
        : (hasSavedCountry && !savedCountry) || (catalogue.data && (!selectedCountry || !actorSpans))
          ? <WidgetState kind="unavailable" message={actorReason!} />
          : catalogue.error ? <WidgetState kind="error" detail={catalogue.error} onRetry={catalogue.retry} />
            : product.error ? <WidgetState kind="error" detail={product.error} onRetry={product.retry} />
              : !data ? <><PreparedLogCoverage coverage={request ? null : undefined} />
                <WidgetState kind="loading" label="Loading equipment losses..." /></> : null;
  if (import.meta.env.MODE === 'parity') {
    useParityRows(saved ? null : anchor, () => lossesParity({
      campaignKey: track?.source.metadata.campaignKey, request, data, window: view,
      items, total, levels: levelsShown(options, LEVELS), settings,
      complete: unavailable === null && !!track && !!tag && !!view && !!data,
    }));
  }
  if (unavailable || !track || !tag || !view || !data) return <>{countryHeader}{unavailable}</>;
  const openRows = (keys: readonly string[] | null) => { setPicked(keys); setDrill(state => reduceDrill(state, { type: 'open', breakdown: DETAIL.id,
    hour, playing: widget.time.mode === 'follow' && runtime.clock.getSnapshot().playing, playhead: widget.time.mode === 'follow' ? hour : null })); };
  const coverage = coverageLegend(data, view, tokens);
  const lostThrough = [data.prefix?.pending.length && requestedView && view.hour < requestedView.hour
    ? `Lost through ${dayLabel(view.hour)}; the rest is still being prepared.` : null,
    data.prefix?.approximate?.length ? 'Approximate while sinking dates and equipment types are still being prepared.' : null].filter(Boolean).join(' ');
  const airCoverage = data.landSource === 'combatLog' ? destructionLegend(data, view, tokens) : [];
  const incomplete = total.incomplete || [...coverage, ...airCoverage].some(item => item.label === 'Incomplete')
    || data.causes.some(cause => cause.loss && cause.source.kind === 'destruction')
      && (data.destructionCountedFrom === null || view.hour < data.destructionCountedFrom);
  const status = data.prefix?.approximate?.length ? 'Approximate' : total.exact === null ? 'Unavailable' : incomplete ? 'Incomplete' : 'Complete';
  const limitations = incomplete ? unknownCauses(data, view.hour) : [];
  const coverageBody = limitations.length ? <details className="equipment-losses-completeness"><summary>{status}</summary>
    {limitations.map(item => <p key={item.id}>{item.label}</p>)}</details> : <span className="equipment-losses-completeness">{status}</span>;
  const main = <div className="equipment-losses-main">
    <p className="chart-line-key">{total.incomplete ? 'Subtotal' : 'Losses'}: {total.exact === null ? 'Not known' : formatLosses(total.value ?? 0, total.exact, scale)}</p>
    <Sunburst label="Equipment losses" items={items} root={ROOT} levels={levelsShown(options, LEVELS)} text={text} settings={settings}
      authoritativeTotal={total} pickOwner={pickOwner} hoursPerSecond={speed} active={active && (!open || options.drill === 'popout')}
      footnote={[lostThrough, lossesFootnote(data, view)].filter(Boolean).join(' ')} legendExtra={coverageBody}
      canPick={node => node.children.length === 0 && node.counted.size > 0}
      onPick={node => { if (node.children.length || !node.counted.size) return false; openRows([...node.counted]); return true; }}
      onContextMenu={(node, x, y) => { const chosen = lossRows(data, view, node ? node.counted : null);
        const read = chosen.reduce<number | null>((latest, row) => row.readHour === null ? latest : Math.max(latest ?? row.readHour, row.readHour), null);
        menu.open(x, y, [{ label: 'Losses', run: () => openRows(node ? [...node.counted] : null) }, ...(read === null ? [] : [jumpItem(runtime, track.id, read)])]); }} />
    <button type="button" className="button small" onClick={() => openRows(null)}>View all losses</button>
  </div>;
  const detail = !detailView ? <WidgetState kind="unavailable" message="This time is outside the recording." /> : <>
    <PreparedKey approximate={data.prefix?.approximate} pending={data.prefix?.pending} />
    {saved && <p className="chart-line-key">{formatDateTime(detailView.hour)} {!following && view.from == null && <button type="button" className="button small"
      onClick={() => setDrill(state => ({ ...state, breakdown: DETAIL.id, follow: true, hour: null }))}>Follow playback</button>}</p>}
    {actorReason && <p className="drill-empty">{actorReason}</p>}
    {selected && <button type="button" className="button small" onClick={() => setPicked(null)}>View all losses</button>}
    <DrillParts parts={parts} format={value => formatLosses(value * scale, null, scale)} positiveBars virtual active={active && open}
      showShares={!rows.some(row => row.incomplete || row.exact === null || row.exact < 0n)} onPick={id => { const part = parts.find(part => part.id === id); if (part) setPicked(part.keys); }} />
    <div className="equipment-losses-table"><Table label="Equipment losses" rows={rows} columns={columns} rowKey={rowKey} narrow={narrowTableOf(options)}
      sort={{ column: 'amount', direction: 'descending' }} cellBars={false} rankMarkers={false} hoursPerSecond={speed} onRowClick={row => setPicked([row.id])}
      menu={row => row.readHour === null ? [] : [jumpItem(runtime, track.id, row.readHour)]}
      tooltip={row => fitTooltip([{ title: `${row.cause} / ${row.type} / ${row.design}`, rows: [{ label: 'Losses', value: rowText(row) }], note: row.note }], height)} /></div>
    <div className="breakdown-chart"><TimeChart label="Equipment losses over time" extent={[view.first, Math.max(view.first + 1, view.end + 1)]}
      branchStart={track.branch.firstHour} playhead={detailView.hour} playback="cursor" hoursPerSecond={speed} axes={{ xFrom: null, xTo: null, yMin: 0, yMax: null, log: false }} layers={layers}
      active={active && open} revision={stack} pickOwner={detailPickOwner}
      onSummaryView={product.onSummaryView} onHover={product.onHover}
      keyboardHour={product.keyboardHour} keyboardRevision={product.keyboardRevision}
      onNavigateHour={(key, from, x, until) => product.navigate(key, from, x, until, data.causes.filter(cause => cause.loss && cause.unknown === null)
        .flatMap(cause => cause.leaves.filter(leaf => !selected || selected.has(lossKey(cause, leaf.design === null ? undefined : data.designs[leaf.design])))
          .map(leaf => [cause.source, leaf.design === null ? null : data.designs[leaf.design].variant] as const)))}
      onPick={h => {
        const selectedHour = Math.floor(Math.min(h, view.end));
        void product.exactAt(selectedHour).then(exact => {
          if (!exact) return;
          const at = lossHistory(exact, selectedHour, selectedHour, view.from, selected, coverageSpans);
          if (at.model.edges[0]?.some(edge => edge.high > edge.low)) setDrill(state => ({ ...state, breakdown: DETAIL.id, follow: false, hour: selectedHour }));
        });
      }}
      onContextMenu={(h, x, y) => { const at = Math.floor(Math.min(h, view.end)); if (!preparedLossHour(data, at) || !entryAt(known, at)?.value) return;
        const read = lossRows(data, { ...view, hour: at }, selected).reduce<number | null>((latest, row) => row.readHour === null ? latest : Math.max(latest ?? row.readHour, row.readHour), null);
        if (read !== null) menu.open(x, y, [jumpItem(runtime, track.id, read)]); }}
      tooltip={h => { const at = Math.floor(Math.min(h, view.end)); if (!preparedLossHour(data, at) || !entryAt(known, at)?.value) return null;
        return fitTooltip([{ title: 'Equipment losses', subtitle: formatDateTime(at), rows: lossRows(data, { ...view, hour: at }, selected)
          .map(row => ({ label: `${row.cause} / ${row.design}`, value: rowText(row) })) }], height); }} /></div>
  </>;
  return <div ref={anchor} className="equipment-losses">{countryHeader}
    <PreparedLogCoverage coverage={request && !product.data && !product.error ? null : undefined} />
    {saved ? detail : <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={open}
    title="Equipment losses" day={formatDateTime(detailHour)} following={following} canFollow={view.from == null}
    onClose={() => setDrill(CLOSED)} onFollow={() => setDrill(state => ({ ...state, breakdown: DETAIL.id, follow: true, hour: null }))}
    onPin={detailView ? () => runtime.addWidgetBelow(widget.id,
      pinLosses(widget, track, tag, detailView.hour, following, view.from, selected ? [...selected] : null, selectedCountry)) : undefined}
    anchor={anchor} main={main}>{detail}</DrillHost>}{menu.menu}</div>;
}

export default function EquipmentLossesWidget(props: WidgetProps) { return <><OriginalEquipmentLossesWidget {...props} /><CombatLogDetails {...props} view="equipment" /></>; }
