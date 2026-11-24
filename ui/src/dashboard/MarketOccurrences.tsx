import { useCallback, useMemo, useState, type ReactNode } from 'react';
import WidgetState from '../components/WidgetState';
import { jumpItem } from '../charts/jump';
import type { LogDensity } from '../charts/log/logData';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack, WidgetSpec } from './model';
import { loadProduct, useProduct } from './products';
import type { Gate } from './regionGate';
import type { BoundWindow } from './regionTime';
import { forwardRequest } from './request.ts';
import type { LogCut } from './useLogPages';
import { ledgerFacets, ledgerSelection, MARKET_LEDGER_TAIL, type LedgerItem } from './marketLedger.ts';
import type { FamilySources } from '../families/contract.ts';
import FamilyLogHost from '../families/log/FamilyLogHost.tsx';
import type { LogControlsSpec, LogSourceSubscription } from '../families/log/declaration.ts';
import { LOG_MIN_TAIL_LIMIT } from '../families/log/layout.ts';
import type { CountryScopeFilter } from '../families/log/scope.ts';
import { isLogTailReply, type LogTailReplyWire } from '../families/log/tailReply.ts';
import { useLogLooks } from '../families/log/useLogLooks.ts';
import { marketLedgerProjection, type MarketLedgerProjectionOptions } from '../widgets/market-ledger/marketLedgerProjection.ts';

export interface MarketOccurrencesProps {
  runtime: WidgetRuntime;
  track: BoundTrack;
  gate: Gate;
  kinds: readonly number[];
  tags: readonly string[] | CountryScopeFilter | null;
  span: BoundWindow | null;
  cut: LogCut;
  active: boolean;
  label: string;
  density: LogDensity;
  months: boolean;
  onOpen?: (item: LedgerItem, hour: number) => void;
  empty?: ReactNode;
  widget?: WidgetSpec;
  headerSlot?: HTMLElement | null;
  controls?: LogControlsSpec | null;
  onDiagnostics?: (warnings: string[]) => void;
}

const NO_ROW_TAGS: readonly string[] = [];

export default function MarketOccurrences({
  runtime,
  track,
  gate,
  kinds,
  tags,
  span,
  cut,
  active,
  label,
  density,
  months,
  onOpen,
  empty,
  widget,
  headerSlot = null,
  controls = null,
  onDiagnostics,
}: MarketOccurrencesProps) {
  const facets = useMemo(() => ledgerFacets(kinds, tags), [kinds, tags]);
  const [limit, setLimit] = useState(LOG_MIN_TAIL_LIMIT);
  const [rowTags, setRowTags] = useState<readonly string[]>(NO_ROW_TAGS);
  const follow = cut.follow;
  const followOffset = cut.follow ? cut.offset : 0;
  const cutHour = cut.follow ? null : cut.hour;
  const spanFirst = span?.firstHour ?? null;
  const spanLast = span?.lastHour ?? null;
  const pinHour = follow ? null : (cutHour ?? spanLast ?? track.branch.lastHour);
  const fromParam = spanFirst !== null && spanFirst > track.branch.firstHour ? spanFirst : null;

  const looks = useLogLooks(runtime, track, track.branch.firstHour, track.branch.lastHour, active && kinds.length > 0, rowTags);

  const syntheticWidget = useMemo<WidgetSpec>(() => widget ?? ({
    id: `market-occurrences:${track.id}:${label}`,
    kind: 'market-ledger',
    configVersion: 1,
    title: label,
    trackIds: [track.id],
    countries: { kind: 'all' },
    time: follow ? { mode: 'follow' } : { mode: 'campaign' },
    options: {},
    layout: { x: 0, y: 0, w: 6, h: 12 },
  }), [widget, track.id, label, follow]);

  const tailParams = useMemo(() => {
    if (!kinds.length || spanFirst === null || spanLast === null) return null;
    return {
      branchId: track.branch.id,
      facets,
      ...(fromParam !== null ? { from: fromParam } : {}),
      limit,
      countFacets: [],
    };
  }, [kinds.length, spanFirst, spanLast, track.branch.id, facets, fromParam, limit]);

  const queryKey = useMemo(
    () => JSON.stringify([track.source.id, track.source.revision, track.branch.id, facets, fromParam, pinHour, follow ? 1 : 0, spanFirst, spanLast]),
    [track.source.id, track.source.revision, track.branch.id, facets, fromParam, pinHour, follow, spanFirst, spanLast],
  );

  const followPlan = useMemo(() => {
    if (!follow || !tailParams || !active) {
      return { subs: [] as LogSourceSubscription[], refused: null as string | null };
    }
    const checked = gate.checkFollow({ panel: MARKET_LEDGER_TAIL, ...tailParams });
    if (checked.refused) return { subs: [], refused: checked.refused };
    if (!checked.query) return { subs: [], refused: null };
    return {
      subs: [{
        name: 'tail',
        source: track.source,
        branch: track.branch.id,
        offset: followOffset,
        query: checked.query,
        channel: 'panel' as const,
        product: MARKET_LEDGER_TAIL,
      }],
      refused: null,
    };
  }, [follow, followOffset, tailParams, active, gate.checkFollow, track.source, track.branch.id]);

  const staticReq = useMemo(() => {
    if (follow || !tailParams || pinHour === null) return null;
    const clamped = gate.recorded(pinHour);
    if (clamped === null) return null;
    return forwardRequest(track.source, MARKET_LEDGER_TAIL, { ...tailParams, hour: clamped });
  }, [follow, tailParams, pinHour, gate.recorded, track.source]);

  const staticReply = useProduct<LogTailReplyWire>(
    runtime.client,
    staticReq,
    active && !follow && !!staticReq,
    value => {
      if (!isLogTailReply(value)) throw new Error('The market ledger tail reply is malformed.');
      return value;
    },
    track.source,
  );

  const staticSources = useMemo(() => {
    if (follow || !staticReply.data || pinHour === null) return null;
    return {
      sources: { answers: new Map(), panels: new Map<string, unknown>([['tail', staticReply.data]]), live: new Map() },
      hour: pinHour,
    };
  }, [follow, staticReply.data, pinHour]);

  const exactSources = useCallback(async (hour: number, askLimit: number): Promise<FamilySources | null> => {
    if (!tailParams) return null;
    const clamped = gate.recorded(hour);
    if (clamped === null) return null;
    const controller = new AbortController();
    const reply = await loadProduct<LogTailReplyWire>(
      runtime.client,
      forwardRequest(track.source, MARKET_LEDGER_TAIL, { ...tailParams, hour: clamped, limit: askLimit }),
      controller.signal,
      value => {
        if (!isLogTailReply(value)) throw new Error('The market ledger tail reply is malformed.');
        return value;
      },
      track.source,
    );
    return { answers: new Map(), panels: new Map<string, unknown>([['tail', reply]]), live: new Map() };
  }, [tailParams, gate.recorded, runtime.client, track.source]);

  const onLoadScrollPage = useCallback(async (
    frozenHour: number,
    offset: number,
    pageSize: number,
    _baseOrdinal: number,
    total: number,
    signal?: AbortSignal,
  ) => {
    if (!tailParams || total <= 0 || offset >= total) return null;
    const clamped = gate.recorded(frozenHour);
    if (clamped === null) return null;
    const controller = new AbortController();
    const activeSignal = signal ?? controller.signal;
    return loadProduct<LogTailReplyWire>(
      runtime.client,
      forwardRequest(track.source, MARKET_LEDGER_TAIL, {
        ...tailParams,
        hour: clamped,
        offset: String(offset),
        limit: pageSize,
      }),
      activeSignal,
      value => {
        if (!isLogTailReply(value)) throw new Error('The market ledger tail reply is malformed.');
        return value;
      },
      track.source,
    );
  }, [tailParams, gate.recorded, runtime.client, track.source]);

  const projectionOptions = useMemo<MarketLedgerProjectionOptions>(() => ({
    looks,
    fromHour: fromParam ?? track.branch.firstHour,
    ...(typeof empty === 'string' ? { emptyMessage: empty } : {}),
  }), [looks, fromParam, track.branch.firstHour, empty]);

  const onActivate = useCallback((_key: string, meta?: unknown, hour?: number) => {
    if (!onOpen || !meta || hour === undefined) return;
    const item = meta as LedgerItem;
    if (ledgerSelection(item) !== null) onOpen(item, hour);
  }, [onOpen]);

  const contextMenuItems = useCallback((_key: string, hour: number, meta?: unknown) => {
    const item = meta as LedgerItem | undefined;
    const canOpen = !!onOpen && !!item && ledgerSelection(item) !== null;
    return [
      ...(canOpen ? [{ label: 'Show contract', run: () => onOpen(item, hour) }] : []),
      jumpItem(runtime, track.id, hour),
    ];
  }, [onOpen, runtime, track.id]);

  const bodyBlockedState = !kinds.length
    ? <WidgetState kind="empty" icon="history" message="Choose the occurrences to show in the widget settings." />
    : followPlan.refused
      ? <WidgetState kind="unavailable" message={followPlan.refused} />
      : !follow && staticReply.error
        ? <WidgetState kind="error" failure={staticReply.error} onRetry={staticReply.retry} />
        : null;

  const bounds = useMemo(
    () => (spanFirst !== null && spanLast !== null ? [spanFirst, spanLast] as const : null),
    [spanFirst, spanLast],
  );

  return (
    <FamilyLogHost
      projection={marketLedgerProjection}
      options={projectionOptions}
      label={label}
      subject="market activity"
      widget={syntheticWidget}
      runtime={runtime}
      active={active}
      density={density}
      months={months}
      twoLine
      subscriptions={followPlan.subs}
      staticSources={staticSources}
      queryKey={queryKey}
      exactSources={exactSources}
      onLoadScrollPage={onLoadScrollPage}
      onLimitChange={setLimit}
      onRowTags={setRowTags}
      offset={followOffset}
      pin={pinHour}
      bounds={bounds}
      controls={controls}
      onActivate={onOpen ? onActivate : undefined}
      contextMenuItems={contextMenuItems}
      headerSlot={headerSlot}
      onDiagnostics={onDiagnostics}
      bodyBlockedState={bodyBlockedState}
    />
  );
}
