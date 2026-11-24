import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState.tsx';
import type { WidgetProps } from '../../dashboard/definitions';
import type { TiledLine } from '../../dashboard/tiles/view';
import { useSpriteArtResolutions } from '../../dashboard/useSpriteArtwork.ts';
import type { ArtResolution } from '../art/types.ts';
import { renderFamilyStatus, useFamilyPlan, useFamilyTheme } from '../host/index.ts';
import type { SerializedTiledLine, TimeChartBinding, TimeChartWidgetDeclaration } from './declaration.ts';
import FamilyTimeChartHost from './FamilyTimeChartHost.tsx';
import { spriteHandleKey } from './laneView.ts';

let nextTiledRev = 1;
const tiledRevCache = new WeakMap<TiledLine, { readonly rev: number; readonly serialized: SerializedTiledLine }>();

function serializeTiledLine(source: string, line: TiledLine): { readonly rev: number; readonly serialized: SerializedTiledLine } {
  const cached = tiledRevCache.get(line);
  if (cached && cached.serialized.source === source) return cached;
  const rev = nextTiledRev++;
  const serialized: SerializedTiledLine = {
    rev,
    source,
    tag: line.tag,
    head: line.head,
    decimals: line.decimals,
    binHours: line.summary.tiles?.binHours ?? 1,
    from: line.summary.from,
    to: line.summary.to,
    width: line.summary.width,
    buckets: line.summary.buckets,
    ...(line.summary.runs ? { runs: line.summary.runs } : {}),
    missing: line.missing,
    parts: line.parts,
    marks: line.marks,
  };
  const entry = { rev, serialized };
  tiledRevCache.set(line, entry);
  return entry;
}

export interface DeclaredTimeChartWidgetProps<TProjectionOptions, TAux> extends WidgetProps {
  readonly declaration: TimeChartWidgetDeclaration<TProjectionOptions, TAux>;
  readonly onActivate?: (inspectKey: string) => void;
  readonly extraWarnings?: readonly string[];
}

export default function DeclaredTimeChartWidget<TProjectionOptions, TAux>({
  declaration,
  widget,
  runtime,
  active,
  onDiagnostics,
  setOption,
  onActivate,
  extraWarnings,
}: DeclaredTimeChartWidgetProps<TProjectionOptions, TAux>) {
  const [hoverHour, setHoverHour] = useState<number | null>(null);
  const [demandedSprites, setDemandedSprites] = useState<readonly { readonly key: string; readonly frame: number }[]>([]);
  const demandedSigRef = useRef('');
  const widthRef = useRef(0);

  const onArtDemands = useCallback((demands: readonly { readonly key: string; readonly frame: number }[]) => {
    const merged = new Map<string, { readonly key: string; readonly frame: number }>();
    for (const d of demands) merged.set(`${d.key}#${d.frame}`, d);
    const list = [...merged.values()].sort((a, b) => a.key.localeCompare(b.key) || a.frame - b.frame);
    const sig = list.map(d => `${d.key}#${d.frame}`).join(',');
    if (sig === demandedSigRef.current) return;
    demandedSigRef.current = sig;
    setDemandedSprites(prev => {
      const next = new Map(prev.map(d => [`${d.key}#${d.frame}`, d]));
      for (const d of list) next.set(`${d.key}#${d.frame}`, d);
      return [...next.values()];
    });
  }, []);

  const plan = useFamilyPlan<TAux, undefined>({
    kind: declaration.kind,
    subject: declaration.subject,
    emptyCampaign: declaration.emptyCampaign,
    icon: declaration.icon,
    optionFields: declaration.optionFields,
    countries: declaration.countries,
    aux: declaration.aux,
    sources: declaration.sources,
    widget,
    runtime,
    active,
    hoverHour,
    setOption,
  });

  const { theme } = useFamilyTheme(undefined, plan.displayedCountryLooks);

  const resolveSprite = useSpriteArtResolutions(runtime, plan.track, demandedSprites, active && demandedSprites.length > 0);
  const spriteResolutions = useMemo<Readonly<Record<string, ArtResolution>> | undefined>(() => {
    if (!demandedSprites.length) return undefined;
    const out: Record<string, ArtResolution> = {};
    for (const s of demandedSprites) {
      out[spriteHandleKey({ kind: 'sprite', key: s.key, frame: s.frame })] = resolveSprite(s);
    }
    return out;
  }, [demandedSprites, resolveSprite]);

  const extent = useMemo<readonly [number, number]>(() => {
    const span = plan.displayedCtx?.staticSpan;
    if (span) return span;
    if (plan.track) return [plan.track.branch.firstHour, plan.track.branch.lastHour];
    return [0, 1];
  }, [plan.displayedCtx?.staticSpan, plan.track?.branch.firstHour, plan.track?.branch.lastHour]);

  const onViewportWidth = useCallback((width: number) => {
    widthRef.current = width;
    if (!plan.track || width <= 0) return;
    const scale = typeof devicePixelRatio === 'number' && devicePixelRatio > 1 ? Math.min(2, devicePixelRatio) : 1;
    plan.onSeriesViewport({
      from: extent[0],
      to: extent[1],
      end: extent[1],
      width: Math.max(1, Math.round((width - 60) * scale)),
    });
  }, [plan.track, extent, plan.onSeriesViewport]);

  useEffect(() => {
    if (widthRef.current > 0) onViewportWidth(widthRef.current);
  }, [onViewportWidth]);

  const headSourceBySeries = useMemo(() => {
    if (!plan.plan) return undefined;
    const map: Record<string, { readonly answerName: string; readonly tags: readonly string[] }> = {};
    for (const entry of plan.plan.entries) {
      if (entry.read.shape === 'pixel-binned' && entry.read.headQuery) {
        map[entry.name] = { answerName: entry.name, tags: entry.read.tags };
      }
    }
    return Object.keys(map).length ? map : undefined;
  }, [plan.plan]);

  const projectionOptions = useMemo(() => {
    if (!plan.displayedCtx) return null;
    return declaration.projectionOptions(plan.displayedCtx, {
      extent,
      surface: theme?.surface ?? '#111111',
    });
  }, [plan.displayedCtx, declaration, extent, theme?.surface]);

  const buildBinding = useCallback((width: number, height: number, sourceIds: Readonly<Record<string, number>>): TimeChartBinding<TProjectionOptions> | null => {
    if (!plan.track || projectionOptions === null) return null;
    const tiledLines: SerializedTiledLine[] = [];
    const keepTiled: number[] = [];
    const tiledBySource: Record<string, Record<string, number>> = {};

    for (const [sourceName, lines] of Object.entries(plan.tiledSeries)) {
      const byTag: Record<string, number> = {};
      for (const [tag, line] of lines) {
        const entry = serializeTiledLine(sourceName, line);
        keepTiled.push(entry.rev);
        byTag[tag] = entry.rev;
        tiledLines.push(entry.serialized);
      }
      tiledBySource[sourceName] = byTag;
    }

    return {
      family: 'time-chart',
      kind: declaration.kind,
      options: projectionOptions,
      sources: sourceIds,
      extent,
      width,
      height,
      theme,
      identitySnapshot: plan.identitySnapshot,
      ...(spriteResolutions ? { spriteResolutions } : {}),
      ...(tiledLines.length ? { tiledLines } : {}),
      keepTiled,
      tiledBySource,
      ...(headSourceBySeries ? { headSourceBySeries } : {}),
      ...(plan.staticBundle ? { staticPanels: plan.staticBundle.panels, staticAnswers: plan.staticBundle.answers } : {}),
      ...(Object.keys(plan.hovers).length ? { hovers: plan.hovers } : {}),
    };
  }, [plan.track, plan.tiledSeries, plan.staticBundle, plan.hovers, plan.identitySnapshot, spriteResolutions, projectionOptions, declaration.kind, extent, theme, headSourceBySeries]);

  const customUnavailable = plan.displayedCtx && declaration.unavailable
    ? declaration.unavailable(plan.displayedCtx, plan.staticBundle?.hour ?? plan.moment)
    : null;

  const combinedWarnings = useMemo(() => {
    const fromDecl = plan.displayedCtx && declaration.warnings ? declaration.warnings(plan.displayedCtx, plan.followRefused) : [];
    const all = [...plan.groupWarnings, ...fromDecl, ...(extraWarnings ?? [])];
    return all.length ? all : undefined;
  }, [plan.displayedCtx, declaration.warnings, plan.followRefused, plan.groupWarnings, extraWarnings]);

  const blockedState = plan.bodyStatus.kind !== 'ready'
    ? renderFamilyStatus(plan.bodyStatus, { subject: declaration.subject, icon: declaration.icon, loadingLabel: plan.loadingLabel })
    : customUnavailable
      ? <WidgetState kind="unavailable" message={customUnavailable} />
      : null;

  const spritesSig = useMemo(
    () => (spriteResolutions ? Object.entries(spriteResolutions).map(([k, r]) => `${k}:${r.status}`).join(',') : ''),
    [spriteResolutions],
  );
  const hoversSig = useMemo(() => JSON.stringify(Object.entries(plan.hovers).map(([k, v]) => [k, v.hour])), [plan.hovers]);

  return (
    <FamilyTimeChartHost
      declarationKind={declaration.kind}
      label={declaration.label}
      subject={declaration.subject}
      widgetId={widget.id}
      widgetKind={widget.kind}
      trackId={plan.track?.id ?? ''}
      runtime={runtime}
      active={active}
      subscriptions={plan.subscriptions}
      offset={plan.offset}
      pin={plan.pin}
      bounds={plan.bounds}
      buildBinding={buildBinding}
      bindingVersion={`${plan.tiledRevision}:${spritesSig}:${hoversSig}:${theme?.surface ?? ''}:${plan.identitySnapshot ? 1 : 0}:${Object.keys(plan.identitySnapshot?.drawnFlags ?? {}).length}`}
      blockedState={blockedState}
      extraWarnings={combinedWarnings}
      onViewportWidth={onViewportWidth}
      onViewportSize={plan.measureViewport}
      onHoverHour={setHoverHour}
      onIdentityDemands={plan.onIdentityDemands}
      onArtDemands={onArtDemands}
      onActivate={onActivate}
      onDiagnostics={onDiagnostics}
      loadingLabel={plan.loadingLabel}
    />
  );
}
