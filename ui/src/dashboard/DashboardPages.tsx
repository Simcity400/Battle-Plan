import { useEffect, useLayoutEffect, useMemo, useState, type ReactNode } from 'react';
import { LiveMotion } from '../charts/liveMotion.ts';
import { busyCount } from '../shell/busy.ts';
import '../widgets';
import { laneModules, manifestFamilyView, resolveWidgetActiveView } from '../widgets/manifest.ts';
import { viewKey } from '../families/manifest.ts';
import type { ProjectionKey } from '../draw/laneMessages';
import { preloadOtherWidgetRenderers } from '../widgets/rendererLoaders.ts';
import DashboardGrid, { bodiesWaiting, LiveDashboardGrid } from './DashboardGrid';
import PreparedPages, { type PreparedPage } from './PreparedPages';
import type { WidgetRuntime } from './definitions';
import { prepareLanes } from './follow/pipeline.ts';
import { SharedSurfaceContext, SharedSurfaceHost } from './follow/sharedSurface.ts';
import { usePipeline } from './follow/useFollowed.ts';
import type { DashboardTemplate, GridLayouts, WidgetSpec } from './model';
import { preparingClient } from './prepare.ts';
import { pageClock } from './widgetClock.ts';

interface Props {
  active: boolean;
  viewport: HTMLElement | null;
  stripPageId?: string;
  template: DashboardTemplate;
  runtime: WidgetRuntime;
  editing: boolean;
  preparing: boolean;
  preparePages: () => readonly PreparedPage[];
  empty: ReactNode;
  onLayouts: (layouts: GridLayouts) => void;
  onApply: (widget: WidgetSpec, view?: boolean) => void;
  onRemove: (id: string) => void;
}

const noop = () => {};
const drawPrepared = (page: PreparedPage) =>
  <DashboardGrid prepared template={page.template} runtime={page.runtime} editing={false} onLayouts={noop} onApply={noop} onRemove={noop} />;

export default function DashboardPages({ active, viewport, stripPageId, runtime, template, editing, preparing, preparePages, empty, ...actions }: Props) {
  const [drawn, setDrawn] = useState(false);
  useEffect(() => {
    if (!active || !viewport || drawn) return;
    let settled = false;
    let frame = requestAnimationFrame(function check() {
      const ready = bodiesWaiting() === 0 && busyCount() === 0;
      if (ready && settled) {
        preloadOtherWidgetRenderers();
        setDrawn(true);
        return;
      }
      settled = ready;
      frame = requestAnimationFrame(check);
    });
    return () => cancelAnimationFrame(frame);
  }, [active, viewport, template.widgets, drawn]);

  const { client, clock } = runtime;
  const pipeline = usePipeline(active ? clock : null, active ? client : null);
  const [sharedHost, setSharedHost] = useState<SharedSurfaceHost | null>(null);
  useLayoutEffect(() => {
    if (!active || !pipeline || !viewport) {
      setSharedHost(null);
      return;
    }
    const host = new SharedSurfaceHost(pipeline, viewport);
    setSharedHost(host);
    return () => {
      host.dispose();
      setSharedHost(null);
    };
  }, [active, pipeline, viewport]);
  const laneClient = useMemo(() => preparingClient(client), [client]);
  const pages = useMemo(() => drawn ? preparePages().map(page => ({
    ...page,
    runtime: { ...page.runtime, client: laneClient },
  })) : [], [drawn, preparePages, laneClient]);
  const laneKinds = [...new Set(pages.flatMap(page => page.template.widgets.map(widget => widget.kind)))].sort().join(' ');
  const needs = useMemo(() => {
    if (drawn) return laneModules(laneKinds.split(' '));
    const projections = new Map<string, ProjectionKey>();
    for (const { kind, options } of template.widgets) {
      const view = resolveWidgetActiveView(kind, options);
      const entry = manifestFamilyView(kind, view);
      if (entry) projections.set(viewKey(kind, view), [entry.family, kind, view]);
    }
    return { families: [...new Set([...projections.values()].map(([family]) => family))], projections: [...projections.values()] };
  }, [drawn, laneKinds, template.widgets]);
  useEffect(() => { if (active) prepareLanes(pageClock(clock), needs); }, [active, clock, needs]);

  return <>
    {active && (template.widgets.length
      ? <SharedSurfaceContext.Provider value={sharedHost}><LiveMotion.Provider value={clock.getMotion}><LiveDashboardGrid stripPageId={stripPageId} template={template} runtime={runtime} editing={editing} {...actions} /></LiveMotion.Provider></SharedSurfaceContext.Provider>
      : empty)}
    <PreparedPages pages={pages} clock={clock} client={client} viewport={viewport}
      enabled={active && drawn && preparing} draw={drawPrepared} mounting={bodiesWaiting} />
  </>;
}
