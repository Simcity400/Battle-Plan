import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import DatedCountryChip from '../../dashboard/DatedCountryChip';
import { countryDrillTagAt } from '../../dashboard/countryIdentity';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { chooseDrillCountry, drillCountryCoverage, drillCountrySpans, readDrillCountry } from '../country-measure/drillCountry';
import WidgetState from '../../components/WidgetState';
import { useContextMenu } from '../../components/ContextMenu';
import { drawBands } from '../../charts/draw';
import DrillHost from '../../charts/drill/DrillHost';
import DrillParts from '../../charts/drill/DrillParts';
import { CLOSED, reduceDrill } from '../../charts/drill/drillState';
import type { DrillState } from '../../charts/drill/drillState';
import { jumpItem } from '../../charts/jump';
import Legend from '../../charts/Legend';
import { stackExtent } from '../../charts/stack';
import Sunburst from '../../charts/sunburst/Sunburst';
import type { SunburstText } from '../../charts/sunburst/Sunburst';
import { levelsShown, sunburstSettings } from '../../charts/sunburst/sunburstOptions';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import { fitTooltip } from '../../charts/tooltip';
import { mix } from '../../shell/appearance/color';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { formatDateTime } from '../../dashboard/calendar';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { NumericReply } from '../../dashboard/numericReply';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour, usePlaybackSpeed } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { entryAt } from '../../dashboard/reads';
import type { LossesReply } from '../../generated/dashboard';
import { DETAIL, OPTIONS } from './definition';
import { LEVELS, coverageLegend, destructionLegend, formatLosses, lossDetails, lossItems, lossesTotal, lossesFootnote,
  lossWindow, unknownCauses, validateLossesReply } from './lossesData';
import { lossHistory, lossNumbers, lossParts, lossRows, lossSegments, pinLosses, readLossSelection } from './lossDrill';
import type { LossRow } from './lossDrill';
import './equipmentLosses.css';

const ROOT = { id: 'all', label: 'All losses' }, NO_WARNINGS: string[] = [];
export default function EquipmentLossesWidget({ widget, runtime, active, countrySlot, onDiagnostics }: WidgetProps) {
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
  const view = track ? lossWindow(resolved, track.branch, widget.time.mode) : null;
  const saved = options.show === DETAIL.id, open = saved || drill.breakdown !== null;
  const origin = useRef<{ owner: typeof bindingOwner; hour: number } | null>(null);
  if (origin.current?.owner !== bindingOwner) origin.current = null;
  if (view && !origin.current) origin.current = { owner: bindingOwner, hour: view.hour };
  const catalogue = useCountryGroups(runtime, track, active);
  const savedCountry = useMemo(() => readDrillCountry(widget.options?.lossCountry, track, widget.countries),
    [widget.options?.lossCountry, track, widget.countries]);
  const selectedCountry = useMemo(() => savedCountry ?? (hasSavedCountry ? null
    : catalogue.data && track && tag && origin.current ? chooseDrillCountry(catalogue.data.countryIdentities, track, tag, origin.current.hour) : null),
    [savedCountry, hasSavedCountry, catalogue.data, track, tag, bindingOwner]);
  const actorSpans = useMemo(() => selectedCountry && catalogue.data && track ? drillCountrySpans(catalogue.data.countryIdentities, selectedCountry, track) : null,
    [selectedCountry, catalogue.data, track]);
  const request = track && selectedCountry && actorSpans ? { sourceId: track.source.id, revision: track.source.revision, product: 'equipment-losses',
    params: { branchId: track.branch.id, tag: selectedCountry.tag, landSource: options.landSource,
      country: { slot: selectedCountry.slot, countryId: selectedCountry.countryId, originHour: track.branch.firstHour + selectedCountry.originOffset } } } : null;
  const product = useProduct<LossesReply>(runtime.client, request, active, validateLossesReply);
  const retained = useMemo(() => new NumericReply<LossesReply>(), [runtime.client]);
  const identity = JSON.stringify(request), numbers = retained.update(identity, product.data, lossNumbers);
  const data = product.data ?? numbers, contentPending = !!numbers && !product.data && !product.error;
  const selection = useMemo(() => track && tag ? readLossSelection(widget.options?.lossSelection, track.source.metadata.campaignKey, track.branch.id, tag) : null,
    [widget.options?.lossSelection, track?.source.metadata.campaignKey, track?.branch.id, tag]);
  const selected = useMemo(() => picked === undefined ? selection ? new Set(selection.keys) : null : picked === null ? null : new Set(picked), [picked, selection]);
  const scope = JSON.stringify([identity, widget.time, widget.options?.lossSelection]);
  const pickOwner = useMemo(() => ({ client: runtime.client, scope, numbers }), [runtime.client, scope, numbers]);
  useEffect(() => { setDrill(CLOSED); setPicked(undefined); }, [pickOwner]);
  const hour = view?.hour ?? 0;
  const detailPickOwner = useMemo(() => ({ pickOwner, selected }), [pickOwner, selected]);
  const following = drill.breakdown !== null ? drill.follow : widget.time.mode === 'follow';
  const detailHour = Math.floor(following ? playback + (track?.axisOffset ?? 0) : drill.hour ?? hour);
  const detailView = view && track && detailHour >= view.first && detailHour <= view.end ? { ...view, hour: Math.floor(detailHour) } : null;
  const shownTag = actorSpans && detailView ? countryDrillTagAt(actorSpans, detailView.hour) : null;
  const countryLabels = useCountryLabelsAt(runtime.client, track, saved && savedCountry && shownTag && detailView ? [{ tag: shownTag, hour: detailView.hour }] : [], active);
  const actorReason = hasSavedCountry && !savedCountry ? 'The saved native country selection is invalid.'
    : !catalogue.data ? 'The selected native country identity is loading.'
      : !selectedCountry || !actorSpans ? 'The selected native country identity is unavailable at its origin.'
        : !shownTag ? 'The selected native country identity is not recorded at this moment.' : null;
  const countryHeader = saved && countrySlot ? createPortal(<DatedCountryChip hour={detailView?.hour ?? null}
    label={savedCountry && shownTag && detailView ? countryLabels.labelAt(detailView.hour)(shownTag) : null}
    reason={hasSavedCountry ? actorReason : 'This breakdown has no saved native country selection.'} />, countrySlot) : null;
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
      for (const segment of lossSegments(stack.model, frame.x[0], Math.min(frame.x[1], state.playhead ?? frame.x[1]),
        (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth))) drawBands(ctx, frame, theme, segment.hours, segment.bands, colors, segment);
    }, extent: range => stackExtent(stack.model, range[0], range[1]),
  }] : [], [stack, colors]);
  const detailsHour = useRef(hour);
  useLayoutEffect(() => { detailsHour.current = hour; }, [hour]);
  const scale = data?.amountScale ?? 1;
  const text = useMemo<SunburstText>(() => ({ format: (value, exact) => formatLosses(value, exact, scale), unit: 'units', valueLabel: 'Losses', shares: !total.incomplete,
    describe: node => data ? lossDetails(node, data, detailsHour.current) : [] }), [data, scale, total.incomplete]);
  const rowText = (row: LossRow) => row.exact === null ? 'Not known' : `${row.incomplete ? 'Recorded subtotal: ' : ''}${formatLosses(Number(row.exact), row.exact, scale)}`;
  const columns = useMemo<TableColumn<LossRow>[]>(() => [
    { key: 'cause', header: 'Cause', label: 'Cause', value: row => row.cause },
    { key: 'type', header: 'Equipment type', label: 'Equipment type', value: row => row.type },
    { key: 'design', header: 'Design', label: 'Design', value: row => row.design },
    { key: 'amount', header: 'Signed losses', label: 'Signed losses', numeric: true, width: 'max-content', value: row => row.exact, render: rowText },
    { key: 'read', header: 'Change read', label: 'Change read', width: 'max-content', value: row => row.readHour,
      render: row => row.readHour === null ? 'Not recorded' : formatDateTime(row.readHour) },
  ], [scale]);
  const parts = useMemo(() => lossParts(rows, scale).map(part => ({ ...part,
    text: part.raw == null ? 'Not known' : `${part.incomplete ? 'Recorded subtotal: ' : ''}${formatLosses(Number(part.raw), part.raw, scale)}`,
    color: colors[stack?.keys.indexOf(part.id) ?? -1] })), [rows, scale, colors, stack]);
  useEffect(() => { onDiagnostics(contentPending ? ['Equipment-loss content names are updating.'] : data?.warnings ?? NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS); }, [data, contentPending, onDiagnostics]);

  const unavailable = !track || !tag ? <WidgetState kind="empty" icon="sunburst" message="Load a campaign and choose a country to see its equipment losses." />
    : !view ? <WidgetState kind="unavailable" message="This time is outside the campaign's recorded branch." />
      : selection === undefined ? <WidgetState kind="unavailable" message="This loss selection belongs to another campaign, branch or country." />
        : (hasSavedCountry && !savedCountry) || (catalogue.data && (!selectedCountry || !actorSpans))
          ? <WidgetState kind="unavailable" message={actorReason!} />
          : catalogue.error ? <WidgetState kind="error" message={catalogue.error} onRetry={catalogue.retry} />
            : product.error ? <WidgetState kind="error" message={product.error} onRetry={product.retry} />
              : !data ? <WidgetState kind="loading" label="Loading equipment losses..." /> : null;
  if (unavailable || !track || !tag || !view || !data) return <>{countryHeader}{unavailable}</>;
  const openRows = (keys: readonly string[] | null) => { setPicked(keys); setDrill(state => reduceDrill(state, { type: 'open', breakdown: DETAIL.id,
    hour, playing: widget.time.mode === 'follow' && runtime.clock.getSnapshot().playing, playhead: widget.time.mode === 'follow' ? hour : null })); };
  const coverage = coverageLegend(data, view, tokens), coverageNote = data.landSource === 'destruction' ? data.landLowerBound : null;
  const airCoverage = data.landSource === 'combatLog' ? destructionLegend(data, view, tokens) : [];
  const unknown = !settings.legend || options.causeLevel === false ? unknownCauses(data, view.hour).map(cause => ({ ...cause, color: mix(tokens.surface, tokens.text, 0.1) })) : [];
  const coverageBody = <div className="equipment-losses-coverage">
    {(coverage.length > 0 || coverageNote) && <section aria-label="Loss coverage">
      {coverage.length > 0 && <h3>{data.landSource === 'destruction' ? 'Coverage' : 'Land coverage'} / {view.campaign ? 'included hours' : formatDateTime(view.hour)}</h3>}
      <Legend items={coverage} label="Loss coverage states" always />{coverageNote && <p className="sunburst-footnote">{coverageNote}</p>}
    </section>}
    {airCoverage.length > 0 && <section aria-label="Air destruction coverage"><h3>Air destruction coverage</h3><Legend items={airCoverage} label="Air destruction coverage states" always /></section>}
    {unknown.length > 0 && <section aria-label="Unknown loss causes"><h3>Unknown causes</h3><Legend items={unknown} label="Unknown loss causes and reasons" always /></section>}
  </div>;
  const main = <div className="equipment-losses-main">
    <p className="chart-line-key">{total.incomplete ? 'Recorded signed subtotal' : 'Signed losses'}: {total.exact === null ? 'Not known' : formatLosses(total.value ?? 0, total.exact, scale)}</p>
    <Sunburst label="Equipment losses" items={items} root={ROOT} levels={levelsShown(options, LEVELS)} text={text} settings={settings}
      authoritativeTotal={total} pickOwner={pickOwner} hoursPerSecond={speed} active={active && (!open || options.drill === 'popout')}
      footnote={lossesFootnote(data, view)} legendExtra={coverageBody}
      canPick={node => node.children.length === 0 && node.counted.size > 0}
      onPick={node => { if (node.children.length || !node.counted.size) return false; openRows([...node.counted]); return true; }}
      onContextMenu={(node, x, y) => { const chosen = lossRows(data, view, node ? node.counted : null);
        const read = chosen.reduce<number | null>((latest, row) => row.readHour === null ? latest : Math.max(latest ?? row.readHour, row.readHour), null);
        menu.open(x, y, [{ label: 'Recorded losses', run: () => openRows(node ? [...node.counted] : null) }, ...(read === null ? [] : [jumpItem(runtime, track.id, read)])]); }} />
    <button type="button" className="button small" onClick={() => openRows(null)}>All recorded losses and signed corrections</button>
    {contentPending && <p className="chart-line-key">Equipment-loss content names are updating.</p>}
  </div>;
  const detail = !detailView ? <WidgetState kind="unavailable" message="The picked moment is outside the selected recorded range." /> : <>
    {saved && <p className="chart-line-key">{formatDateTime(detailView.hour)} {!following && view.from == null && <button type="button" className="button small"
      onClick={() => setDrill(state => ({ ...state, breakdown: DETAIL.id, follow: true, hour: null }))}>Follow playback</button>}</p>}
    {actorReason && <p className="drill-empty">{actorReason}</p>}
    {selected && <button type="button" className="button small" onClick={() => setPicked(null)}>All recorded losses</button>}
    <DrillParts parts={parts} format={value => formatLosses(value * scale, null, scale)} positiveBars virtual active={active && open}
      showShares={!rows.some(row => row.incomplete || row.exact === null || row.exact < 0n)} onPick={id => { const part = parts.find(part => part.id === id); if (part) setPicked(part.keys); }} />
    <div className="equipment-losses-table"><Table label="Exact recorded equipment losses" rows={rows} columns={columns} rowKey={row => row.id}
      sort={{ column: 'amount', direction: 'descending' }} cellBars={false} rankMarkers={false} hoursPerSecond={speed} onRowClick={row => setPicked([row.id])}
      menu={row => row.readHour === null ? [] : [jumpItem(runtime, track.id, row.readHour)]}
      tooltip={row => fitTooltip([{ title: `${row.cause} / ${row.type} / ${row.design}`, rows: [{ label: 'Signed losses', value: rowText(row) },
        ...(row.readHour === null ? [] : [{ label: 'Change read', value: formatDateTime(row.readHour) }])], note: row.note }], height)} /></div>
    <div className="breakdown-chart"><TimeChart label="Recorded positive equipment losses over time" extent={[view.first, Math.max(view.first + 1, view.end + 1)]}
      branchStart={track.branch.firstHour} playhead={detailView.hour} playback="cursor" hoursPerSecond={speed} axes={{ xFrom: null, xTo: null, yMin: 0, yMax: null, log: false }} layers={layers}
      active={active && open} revision={stack} pickOwner={detailPickOwner} keyHours={stack?.model.hours.filter(hour => hour <= view.end)}
      onPick={h => { const selectedHour = Math.floor(Math.min(h, view.end)); if (entryAt(known, selectedHour)?.value)
        setDrill(state => ({ ...state, breakdown: DETAIL.id, follow: false, hour: selectedHour })); }}
      onContextMenu={(h, x, y) => { const at = Math.floor(Math.min(h, view.end)); if (!entryAt(known, at)?.value) return;
        const read = lossRows(data, { ...view, hour: at }, selected).reduce<number | null>((latest, row) => row.readHour === null ? latest : Math.max(latest ?? row.readHour, row.readHour), null);
        if (read !== null) menu.open(x, y, [jumpItem(runtime, track.id, read)]); }}
      tooltip={h => { const at = Math.floor(Math.min(h, view.end)); if (!entryAt(known, at)?.value) return null;
        return fitTooltip([{ title: 'Recorded equipment losses', subtitle: formatDateTime(at), rows: lossRows(data, { ...view, hour: at }, selected)
          .map(row => ({ label: `${row.cause} / ${row.design}`, value: `${rowText(row)}${row.readHour === null ? '' : ` / Change read ${formatDateTime(row.readHour)}`}` })),
          note: 'Only positive amounts draw bands. Corrections remain exact selectable rows.' }], height); }} /></div>
    <p className="chart-line-key">Only positive amounts have geometry. Select any exact signed or zero row to see its history.</p>
  </>;
  return <div ref={anchor} className="equipment-losses">{countryHeader}{saved ? detail : <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={open}
    title="Recorded equipment losses" day={formatDateTime(detailHour)} following={following} canFollow={view.from == null}
    onClose={() => setDrill(CLOSED)} onFollow={() => setDrill(state => ({ ...state, breakdown: DETAIL.id, follow: true, hour: null }))}
    onPin={detailView ? () => runtime.addWidgetBelow(widget.id,
      pinLosses(widget, track, tag, detailView.hour, following, view.from, selected ? [...selected] : null, selectedCountry)) : undefined}
    anchor={anchor} main={main}>{detail}</DrillHost>}{menu.menu}</div>;
}
