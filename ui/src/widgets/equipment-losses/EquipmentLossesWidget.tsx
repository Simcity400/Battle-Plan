import CombatLogDetails from '../land-battles/CombatLogDetails';
import { narrowTableOf } from '../../charts/table/options.ts';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import DatedCountryChip from '../../dashboard/DatedCountryChip';
import { countryDrillTagAt } from '../../dashboard/countryIdentity';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { chooseDrillCountry, drillCountryCoverage, drillCountrySpans, readDrillCountry } from '../country-measure/drillCountry';
import WidgetState from '../../components/WidgetState';
import { useHeldFailure } from '../../dashboard/heldFailures';
import { useContextMenu } from '../../components/ContextMenu';
import { drawBands } from '../../charts/draw';
import DrillHost from '../../charts/drill/DrillHost';
import DrillParts from '../../charts/drill/DrillParts';
import type { LiveDrillParts } from '../../charts/drill/DrillParts';
import { jumpItem } from '../../charts/jump';
import { liveAt, liveKey, valuesOf } from '../../charts/live';
import type { Live, LiveValues } from '../../charts/live';
import { stackExtent } from '../../charts/stack';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import type { SortValue } from '../../charts/table/logic';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import { fitTooltip } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { formatDateTime } from '../../dashboard/calendar';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import { countryFromKey, isSlotKey } from '../../dashboard/countryKey.ts';
import { countryTagAt, useSlotTags } from '../../dashboard/countrySlots.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { resolveWindow } from '../../dashboard/model';
import { NumericReply } from '../../dashboard/numericReply';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { sinceLabel } from '../../dashboard/stretch';
import { entryAt } from '../../dashboard/reads';
import { DETAIL, OPTIONS } from './definition';
import {
  LEVELS,
  atHead,
  formatLosses,
  lossWindow,
  lossKey,
  withShips,
  type LossWindow,
} from './lossesData.ts';
import { lossHistory, lossNumbers, lossParts, lossRows, lossSegments, pinLosses, readLossSelection, type LossRow } from './lossDrill';
import { exactLossHour, stretchWindow, useLossHistory, type LossSelection } from './lossesSource';
import type { LandSource, LossHeadReply, LossesReply } from '../../generated/dashboard';
import { useDrill } from '../../charts/drill/useDrill';
import { useRegion } from '../../dashboard/widgetTime.ts';
import { recordedHour } from '../../dashboard/regionTime.ts';
import FamilyBreakdownHost from '../../families/breakdown/FamilyBreakdownHost.tsx';
import { breakdownViewOf, levelsShown } from '../../families/breakdown/options.ts';
import type { FamilySourceSubscription } from '../../families/contract.ts';
import { equipmentLossesProjection, type EquipmentLossesProjectionOptions } from './equipmentLossesProjection.ts';
import './equipmentLosses.css';

const NO_WARNINGS: string[] = [];
const rowKey = (row: LossRow) => row.id;

function validateHead(value: unknown): LossHeadReply {
  const reply = value as LossHeadReply;
  if (!reply || !Array.isArray(reply.leaves) || !reply.leaves.every(leaf => typeof leaf === 'string') || !Array.isArray(reply.warnings))
    throw new Error('The equipment-loss head reply is malformed.');
  return reply;
}

function LossesView({ widget, runtime, active, countrySlot, onDiagnostics, setOption }: WidgetProps) {
  const [picked, setPicked] = useState<readonly string[] | null | undefined>(undefined);
  const anchor = useRef<HTMLDivElement>(null), menu = useContextMenu('Equipment loss actions');
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryKey(track) ?? undefined;
  const options = resolveOptions(OPTIONS, widget.options);
  const viewMode = breakdownViewOf(options);
  const namesInRing = options.namesInRing !== false;
  const hasSavedCountry = Object.hasOwn(widget.options ?? {}, 'lossCountry');
  const bindingKey = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, tag, widget.time,
    hasSavedCountry, widget.options?.lossCountry, widget.options?.lossSelection, options.show]);
  const bindingOwner = useMemo(() => ({ client: runtime.client, key: bindingKey }), [runtime.client, bindingKey]);
  const drillControl = useDrill(runtime, useRegion(widget), options.show === DETAIL.id ? DETAIL.id : null, undefined, undefined, HOLDERS);
  const drill = drillControl.state;
  useEffect(() => { drillControl.close(); }, [bindingOwner]);
  const speed = usePlaybackMotion(runtime.clock), tokens = useChartTokens(), height = useViewportHeight();
  const followView = widget.time.mode === 'follow';
  const requestedView: LossWindow | null = !track ? null : followView
    ? { first: track.branch.firstHour, hour: track.branch.lastHour, end: track.branch.lastHour, campaign: false, from: null }
    : lossWindow(resolveWindow(widget, track, runtime.tracks, 0), track.branch, widget.time.mode);
  const hourAt = (position: number) => Math.floor((track && resolveWindow(widget, track, runtime.tracks, position)?.lastHour) ?? requestedView?.end ?? 0);
  const saved = options.show === DETAIL.id, open = drillControl.open;
  const catalogue = useCountryGroups(runtime, track, active);
  const origin = requestedView ? followView ? hourAt(runtime.clock.getSnapshot().position) : requestedView.hour : null;
  const originRef = useRef<{ owner: unknown; hour: number } | null>(null);
  const recorded = (hour: number) => recordedHour(track, hour) !== null;
  if (originRef.current?.owner !== bindingOwner || !recorded(originRef.current.hour)) originRef.current = origin === null ? null : { owner: bindingOwner, hour: origin };
  const originHour = originRef.current?.hour ?? null;
  const savedCountry = useMemo(() => readDrillCountry(widget.options?.lossCountry, track, widget.countries),
    [widget.options?.lossCountry, track, widget.countries]);
  const bySlot = !!tag && isSlotKey(tag);
  const slots = useSlotTags(runtime.client, track, active && bySlot);
  const slotsPending = bySlot && !slots;
  const selectedCountry = useMemo(() => {
    if (savedCountry || hasSavedCountry) return savedCountry;
    const originTag = tag && originHour !== null ? countryTagAt(slots, countryFromKey(tag), originHour) : null;
    return catalogue.data && track && originTag && originHour !== null ? chooseDrillCountry(catalogue.data.countryIdentities, track, originTag, originHour) : null;
  }, [savedCountry, hasSavedCountry, catalogue.data, track, tag, slots, bindingOwner, originHour]);
  const actorSpans = useMemo(() => selectedCountry && catalogue.data && track ? drillCountrySpans(catalogue.data.countryIdentities, selectedCountry, track) : null,
    [selectedCountry, catalogue.data, track]);
  const selection: LossSelection | null = useMemo(() => track && selectedCountry && actorSpans ? {
    branchId: track.branch.id, country: selectedCountry.tag, landSource: options.landSource as LandSource,
    identity: { slot: selectedCountry.slot, countryId: selectedCountry.countryId },
  } : null, [track, selectedCountry, actorSpans, options.landSource]);
  const following = drill.breakdown === null ? followView : drillControl.following;
  const pinnedDetail = !following && drill.breakdown !== null ? drill.hour : null;
  const gate = useGate(runtime, widget);
  const product = useLossHistory(runtime.client, drill.breakdown === null ? gate : drillControl.gate, track, selection, requestedView,
    [...(followView ? [] : [requestedView?.hour ?? 0]), ...(pinnedDetail === null ? [] : [Math.floor(pinnedDetail)])], active, open);
  const head = useHeadProduct(runtime, gate, 'equipment-loss-head',
    followView && selection && open ? { country: selection.country, identity: selection.identity } : null,
    { active: active && open, validate: validateHead, structure: reply => reply.leaves.join(',') });
  const retained = useMemo(() => new NumericReply<LossesReply>(), [runtime.client]);
  const identity = JSON.stringify([track?.source.id, track?.source.revision, selection]);
  const numbers = retained.update(identity, product.data, reply => ({ ...reply, ...lossNumbers(reply) }));
  const data = product.data ?? numbers, contentPending = !!numbers && !product.data && !product.error;
  const productError = useHeldFailure(product.error, product.data !== undefined, product.retry);
  const headRoot = head.root;
  const rootRef = useCallback((element: HTMLDivElement | null) => { (anchor as { current: HTMLDivElement | null }).current = element; headRoot(element); }, [headRoot]);
  const named = useMemo(() => new Set(head.data?.leaves ?? []), [head.data]);
  const structureHour = followView ? head.at ?? origin ?? 0 : requestedView?.hour ?? 0;
  const view = requestedView && data ? stretchWindow(data, { ...requestedView, hour: structureHour }) : requestedView;
  const shownReply = useMemo(() => data && view ? followView && head.data ? atHead(data, view.hour, head.live.read(), named) : withShips(data, view.hour) : null,
    [data, view?.hour, followView, head.data, named]);
  const selectionPick = useMemo(() => track && tag ? readLossSelection(widget.options?.lossSelection, track.source.metadata.campaignKey, track.branch.id, tag) : null,
    [widget.options?.lossSelection, track?.source.metadata.campaignKey, track?.branch.id, tag]);
  const selected = useMemo(() => picked === undefined ? selectionPick ? new Set(selectionPick.keys) : null : picked === null ? null : new Set(picked), [picked, selectionPick]);
  const scope = JSON.stringify([identity, widget.time, widget.options?.lossSelection]);
  const pickOwner = useMemo(() => ({ client: runtime.client, scope }), [runtime.client, scope]);
  useEffect(() => { drillControl.close(); setPicked(undefined); }, [pickOwner]);
  const hour = view?.hour ?? 0;
  const detailPickOwner = useMemo(() => ({ pickOwner, selected }), [pickOwner, selected]);
  const detailHour = following ? hour : Math.floor(drill.hour ?? hour);
  const detailBase = view && detailHour >= view.first && detailHour <= view.end ? { ...view, hour: detailHour } : null;
  const detailView = detailBase && data ? stretchWindow(data, detailBase) : detailBase;
  const detailReply = useMemo(() => data && detailView ? following ? shownReply : withShips(data, detailView.hour) : null,
    [data, detailView?.hour, following, shownReply]);
  const shownTag = actorSpans && detailView ? countryDrillTagAt(actorSpans, detailView.hour) : null;
  const countryLabels = useCountryLabelsAt(runtime.client, track, saved && savedCountry && shownTag && detailView ? [{ tag: shownTag, hour: detailView.hour }] : [], active);
  const actorReason = hasSavedCountry && !savedCountry ? 'Choose a country again.'
    : !catalogue.data || slotsPending ? 'Loading country…'
      : !selectedCountry || !actorSpans ? 'Country unavailable at this time.'
        : !shownTag ? 'Country unavailable at this time.' : null;
  const countryHeader = saved && countrySlot ? createPortal(<DatedCountryChip hour={detailView?.hour ?? null}
    label={savedCountry && shownTag && detailView ? countryLabels.labelAt(detailView.hour)(shownTag) : null}
    reason={hasSavedCountry ? actorReason : 'Choose a country again.'} />, countrySlot) : null;
  const rows = useMemo(() => detailReply && detailView && shownTag ? lossRows(detailReply, detailView, selected) : [],
    [detailReply, detailView?.hour, detailView?.from, selected, shownTag]);
  const coverageSpans = useMemo(() => actorSpans ? drillCountryCoverage(actorSpans) : [], [actorSpans]);
  const history = useMemo(() => data && view ? withShips(data, view.end) : null, [data, view?.end]);
  const stack = useMemo(() => history && view ? lossHistory(history, view.first, view.end, view.from, selected, coverageSpans) : null,
    [history, view?.first, view?.end, view?.from, selected, coverageSpans]);
  const known = useMemo(() => stack?.model.hours.map((firstHour, index) => ({ firstHour, value: stack.model.edges[index]?.some(edge => edge.high > edge.low) === true })) ?? [], [stack]);
  const colors = useMemo(() => stack?.keys.map((_key, index) => tokens.categories[index % tokens.categories.length]) ?? [], [stack, tokens.categories]);
  const layers = useMemo<ChartLayer[]>(() => stack ? [{
    draw: (ctx, frame, theme, state) => {
      for (const segment of lossSegments(stack.model, frame.x[0], Math.min(frame.x[1], state.playhead ?? frame.x[1]),
        (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth))) drawBands(ctx, frame, theme, segment.hours, segment.bands, colors, segment);
    }, extent: range => stackExtent(stack.model, range[0], range[1]),
  }] : [], [stack, colors]);
  const scale = data?.amountScale ?? 1;

  const headLive = useMemo(() => {
    if (!followView || !data || !view || !head.data) return null;
    const hourOf = liveAt(runtime.clock, hourAt, at => at);
    let made: { hour: number; cells: LiveValues<number>; reply: LossesReply } | null = null;
    const replyLive: Live<LossesReply> & { hour(): number } = {
      hour: () => hourOf.read(),
      read: () => {
        const at = hourOf.read(), cells = head.live.read();
        if (!made || made.hour !== at || made.cells !== cells) made = { hour: at, cells, reply: atHead(data, at, cells, named) };
        return made.reply;
      },
      subscribe: changed => { const stops = [hourOf.subscribe(changed), head.live.subscribe(changed)]; return () => { for (const stop of stops) stop(); }; },
    };
    return replyLive;
  }, [followView, data, view?.first, view?.end, view?.from, head.data, head.live, named, runtime.clock]);

  const rowsLive = useMemo<Live<LiveValues<SortValue>> | undefined>(() => {
    if (!headLive || !view || !following) return undefined;
    let made: { reply: LossesReply; values: LiveValues<SortValue> } | null = null;
    return { subscribe: headLive.subscribe, read: () => {
      const reply = headLive.read();
      if (made?.reply !== reply) {
        const at = lossRows(reply, { ...view, hour: headLive.hour() }, selected);
        made = { reply, values: valuesOf(new Map(at.flatMap(row => row.exact === null ? [] : [[liveKey(row.id, 'amount'), row.exact] as const])), 'Not known') };
      }
      return made.values;
    } };
  }, [headLive, view?.first, view?.end, view?.from, following, selected]);
  const partsLive = useMemo<LiveDrillParts | undefined>(() => {
    if (!headLive || !view || !following) return undefined;
    let made: { reply: LossesReply; parts: ReturnType<LiveDrillParts['read']> } | null = null;
    return { subscribe: headLive.subscribe, read: () => {
      const reply = headLive.read();
      if (made?.reply !== reply) made = { reply, parts: { parts: partsOf(lossRows(reply, { ...view, hour: headLive.hour() }, selected)) } };
      return made.parts;
    } };
  }, [headLive, view?.first, view?.end, view?.from, following, selected]);
  const liveHead = useMemo(() => following && headLive ? { read: () => headLive.hour(), subscribe: headLive.subscribe } : undefined, [following, headLive]);

  const rowText = (row: LossRow) => row.exact === null ? 'Not known' : formatLosses(Number(row.exact), row.exact, scale);
  const columns = useMemo<TableColumn<LossRow>[]>(() => [
    { key: 'cause', header: 'Cause', label: 'Cause', priority: 1, inTooltip: true, value: row => row.cause },
    { key: 'type', header: 'Equipment type', label: 'Equipment type', priority: 2, inTooltip: true, value: row => row.type },
    { key: 'design', header: 'Design', label: 'Design', value: row => row.design },
    { key: 'amount', header: 'Losses', label: 'Losses', numeric: true, width: 'max-content', value: row => row.exact, render: rowText,
      paint: value => formatLosses(Number(value), typeof value === 'bigint' ? value : BigInt(Math.round(Number(value))), scale) },
  ], [scale]);
  function partsOf(at: readonly LossRow[]) {
    return lossParts(at, scale).map(part => ({ ...part,
      text: part.raw == null ? 'Not known' : formatLosses(Number(part.raw), part.raw, scale),
      color: colors[stack?.keys.indexOf(part.id) ?? -1] }));
  }
  const parts = useMemo(() => partsOf(rows), [rows, scale, colors, stack]);
  const warnings = useMemo(() => [...data?.warnings ?? [], ...head.data?.warnings ?? []], [data, head.data]);
  useEffect(() => { onDiagnostics(contentPending ? ['Equipment-loss content names are updating.'] : warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS); }, [warnings, contentPending, onDiagnostics]);

  const subscriptions = useMemo<readonly FamilySourceSubscription[]>(() => {
    if (!followView || !track || !selection) return [];
    return [{
      name: 'head',
      source: track.source,
      branch: track.branch.id,
      offset: track.axisOffset,
      channel: 'panel',
      product: 'equipment-loss-head',
      query: { kind: 'equipment-loss-head', country: selection.country, identity: selection.identity },
    }];
  }, [followView, track, selection]);

  const levelKey = LEVELS.map(level => options[level] !== false).join();
  const levels = useMemo(() => levelsShown(options, LEVELS), [levelKey]);

  const projectionOptions = useMemo<EquipmentLossesProjectionOptions | null>(() => {
    if (!view) return null;
    return {
      window: view,
      levels,
      useHead: followView,
    };
  }, [view, levels, followView]);

  const staticSources = useMemo(() => {
    if (!data || !track) return null;
    return {
      panels: { losses: data },
      hour: track.branch.firstHour,
      endHour: track.branch.lastHour,
    };
  }, [data, track]);

  const bounds = useMemo<readonly [number, number] | null>(
    () => track ? [track.branch.firstHour, track.branch.lastHour] : null,
    [track],
  );

  const openRows = useCallback((keys: readonly string[] | null) => {
    setPicked(keys);
    drillControl.show(DETAIL.id, hour);
  }, [drillControl, hour]);

  const contextMenuItems = useCallback((_key: string | null, _name: string | null, atHour: number) => {
    if (!track) return [];
    return [
      { label: 'View all losses', run: () => openRows(null) },
      jumpItem(runtime, track.id, atHour),
    ];
  }, [track, runtime, openRows]);

  const unavailable = !track || !tag ? <WidgetState kind="empty" icon="sunburst" message="Load a campaign and choose a country to see its equipment losses." />
    : !view ? <WidgetState kind="outside" />
      : selectionPick === undefined ? <WidgetState kind="unavailable" message="Choose losses again." />
        : (hasSavedCountry && !savedCountry) || (catalogue.data && !slotsPending && (!selectedCountry || !actorSpans))
          ? <WidgetState kind="unavailable" message={actorReason!} />
          : catalogue.error && !catalogue.data ? <WidgetState kind="error" failure={catalogue.error} onRetry={catalogue.retry} />
            : productError ? <WidgetState kind="error" failure={productError} onRetry={product.retry} />
              : !data || !projectionOptions ? <WidgetState kind="loading" label="Loading equipment losses…" /> : null;
  const catalogueNotice = catalogue.error && catalogue.data ? <WidgetState kind="error" failure={catalogue.error} onRetry={catalogue.retry} /> : null;
  if (unavailable || !track || !tag || !view || !data || !projectionOptions) {
    return <div ref={rootRef} className="equipment-losses">{countryHeader}{catalogueNotice}{unavailable}</div>;
  }

  const readHourOf = (at: readonly LossRow[]) => at.reduce<number | null>((latest, row) => row.readHour === null ? latest : Math.max(latest ?? row.readHour, row.readHour), null);
  const main = (
    <div className="equipment-losses-main">
      <FamilyBreakdownHost
        declaration={equipmentLossesProjection}
        options={projectionOptions}
        view={viewMode}
        namesInRing={namesInRing}
        label="Equipment losses"
        subject="equipment losses"
        widget={widget}
        runtime={runtime}
        active={active && (!open || options.drill === 'popout')}
        subscriptions={subscriptions}
        staticSources={staticSources}
        offset={track.axisOffset}
        pin={followView ? null : Math.floor(view.hour)}
        bounds={bounds}
        onActivate={(_key, meta) => {
          if (Array.isArray(meta) && meta.length) openRows(meta as string[]);
        }}
        contextMenuItems={contextMenuItems}
        onSetOption={setOption}
        onDiagnostics={onDiagnostics}
        loadingLabel="Loading equipment losses…"
      />
    </div>
  );

  const detail = !detailView ? <WidgetState kind="outside" /> : <>
    {detailView.since != null && <p className="chart-line-key">Counted {sinceLabel(detailView.since)}, after hours the recording lost.</p>}
    {saved && <p className="chart-line-key">{following ? 'Following playback' : formatDateTime(detailView.hour)} {!following && view.from == null && <button type="button" className="button small"
      onClick={drillControl.follow}>Follow playback</button>}</p>}
    {actorReason && <p className="drill-empty">{actorReason}</p>}
    {selected && <button type="button" className="button small" onClick={() => setPicked(null)}>View all losses</button>}
    <DrillParts parts={parts} live={partsLive} format={value => formatLosses(value * scale, null, scale)} positiveBars virtual active={active && open}
      showShares={!rows.some(row => row.incomplete || row.exact === null || row.exact < 0n)} onPick={id => { const part = parts.find(item => item.id === id); if (part) setPicked(part.keys); }} />
    <div className="equipment-losses-table"><Table rowRevision={row => [row, columns]} label="Equipment losses" rows={rows} columns={columns} rowKey={rowKey} narrow={narrowTableOf(options)}
      live={rowsLive} sort={{ column: 'amount', direction: 'descending' }} cellBars={false} rankMarkers={false} hoursPerSecond={speed} onRowClick={row => setPicked([row.id])}
      menu={row => row.readHour === null ? [] : [jumpItem(runtime, track.id, row.readHour)]}
      tooltip={row => fitTooltip([{ title: `${row.cause} / ${row.type} / ${row.design}`, rows: [{ label: 'Losses', value: rowText(row) }], note: row.note }], height)} /></div>
    <div className="breakdown-chart"><TimeChart label="Equipment losses over time" extent={[view.first, Math.max(view.first + 1, view.end + 1)]}
      branchStart={track.branch.firstHour} playhead={detailView.hour} liveHead={liveHead} playback="cursor" hoursPerSecond={speed} axes={{ xFrom: null, xTo: null, yMin: 0, yMax: null, log: false }} layers={layers}
      active={active && open} revision={stack} pickOwner={detailPickOwner}
      onSummaryView={product.onSummaryView} onHover={product.onHover}
      keyboardHour={product.keyboardHour} keyboardRevision={product.keyboardRevision}
      onNavigateHour={(key, from, x, until) => product.navigate(key, from, x, until, data.causes.filter(cause => cause.loss && cause.unknown === null && cause.source.kind !== 'sunkShips')
        .flatMap(cause => cause.leaves.filter(leaf => !selected || selected.has(lossKey(cause, leaf.design === null ? undefined : data.designs[leaf.design])))
          .map(leaf => [cause.source, leaf.key] as const)),
        (history ?? data).causes.filter(cause => cause.loss && cause.source.kind === 'sunkShips')
          .flatMap(cause => cause.leaves.filter(leaf => !selected || selected.has(lossKey(cause, leaf.design === null ? undefined : data.designs[leaf.design]))))
          .flatMap(leaf => leaf.points.map(point => point.hour)))}
      onPick={at => {
        const selectedHour = Math.floor(Math.min(at, view.end));
        void product.exactAt(selectedHour).then(exact => {
          if (!exact) return;
          const point = lossHistory(withShips(exact, selectedHour), selectedHour, selectedHour, view.from, selected, coverageSpans);
          if (point.model.edges[0]?.some(edge => edge.high > edge.low)) drillControl.dispatch(drillControl.open ? { type: 'pick', hour: selectedHour } : { type: 'open', breakdown: DETAIL.id, hour: selectedHour, follow: false });
        });
      }}
      onContextMenu={(at, x, y) => { const pointed = Math.floor(Math.min(at, view.end)); if (!entryAt(known, pointed)?.value) return;
        const read = readHourOf(lossRows(withShips(data, pointed), { ...view, hour: pointed }, selected));
        if (read !== null) menu.open(x, y, [jumpItem(runtime, track.id, read)]); }}
      tooltip={at => { const pointed = Math.floor(Math.min(at, view.end)); if (!entryAt(known, pointed)?.value) return null;
        return fitTooltip([{ title: 'Equipment losses', subtitle: `${formatDateTime(pointed)}${exactLossHour(data, pointed) ? '' : ' (to the chart\'s bin)'}`,
          rows: lossRows(withShips(data, pointed), { ...view, hour: pointed }, selected).map(row => ({ label: `${row.cause} / ${row.design}`, value: rowText(row) })) }], height); }} /></div>
  </>;
  const pinHeld = drillControl.gate.checkHold(selectedCountry && track ? { tag: selectedCountry.tag, origin: track.branch.firstHour + selectedCountry.originOffset } : null);
  const pinCountry = pinHeld.held;
  return (
    <div ref={rootRef} className="equipment-losses">
      {countryHeader}
      {catalogueNotice}
      {saved ? detail : (
        <DrillHost
          drill={drillControl}
          runtime={runtime}
          mode={options.drill === 'popout' ? 'popout' : 'inline'}
          title="Equipment losses"
          day={following ? 'Following playback' : formatDateTime(detailHour)}
          canFollow={view.from == null}
          pinRefused={pinHeld.refused ?? drillControl.pinRefused}
          onPin={detailView && pinCountry && drillControl.time() ? () => {
            const span = view.from != null ? drillControl.gate.span(view.from, detailView.hour) : null;
            const time = view.from != null ? span && drillControl.time(span, 'range') : drillControl.time();
            if (time) runtime.addWidgetBelow(widget.id, pinLosses(widget, track, pinCountry, time, selected ? [...selected] : null));
          } : undefined}
          anchor={anchor}
          main={main}
        >
          {detail}
        </DrillHost>
      )}
      {menu.menu}
    </div>
  );
}

const HOLDERS = { holders: true };

export default function EquipmentLossesWidget(props: WidgetProps) {
  return <><LossesView {...props} /><CombatLogDetails {...props} view="equipment" /></>;
}
