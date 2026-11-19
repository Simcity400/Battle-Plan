// The browser's own trace of a playing window: what the page's main thread spends a frame on beyond the app's own
// painting (style, layout, pre-paint, paint, compositing, script, garbage collection), what runs off it (raster, GPU),
// and which elements force style and layout. Taken over the DevTools protocol (`Tracing`, devtools.timeline categories).

/** What the trace records: DevTools' performance panel's own categories, with invalidation tracking for what dirtied style and layout. */
export const TRACE_CATEGORIES = [
  'devtools.timeline', 'disabled-by-default-devtools.timeline', 'disabled-by-default-devtools.timeline.frame',
  'disabled-by-default-devtools.timeline.stack', 'disabled-by-default-devtools.timeline.invalidationTracking',
  'toplevel', 'v8.execute', 'blink.user_timing', 'latencyInfo',
];
/** The marks the page sets at the window's ends: the trace is cut to the hours played. */
export const TRACE_MARKS = { from: 'e2e-trace-from', to: 'e2e-trace-to' };
const COMPLETE_MS = 120_000;

/** Starts tracing on `cdp` (`send`, `on` returning an unsubscribe); the result stops it and gives every event. */
export async function startTrace(cdp, categories = TRACE_CATEGORIES) {
  const events = [];
  let complete;
  const completed = new Promise(resolve => { complete = resolve; });
  const stops = [cdp.on('Tracing.dataCollected', ({ value }) => { for (const event of value) events.push(event); }),
    cdp.on('Tracing.tracingComplete', () => complete())];
  await cdp.send('Tracing.start', { transferMode: 'ReportEvents', traceConfig: { recordMode: 'recordAsMuchAsPossible', includedCategories: categories } });
  let stopped = null;
  return () => stopped ??= (async () => {
    try {
      await cdp.send('Tracing.end');
      let timer;
      await Promise.race([completed, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`the trace did not complete within ${COMPLETE_MS / 1000} s`)), COMPLETE_MS); })])
        .finally(() => clearTimeout(timer));
      return events;
    } finally { for (const stop of stops) stop?.(); }
  })();
}

/** Main-thread work by kind, by the event names DevTools' performance panel files them under. A nested event's time is
 * its own kind's; what is left of an event after its children is its own (`self`). */
const KINDS = {
  style: ['UpdateLayoutTree', 'RecalculateStyles', 'ParseAuthorStyleSheet'],
  layout: ['Layout'],
  prePaint: ['PrePaint', 'UpdateLayer', 'UpdateLayerTree', 'Layerize', 'HitTest', 'ComputeIntersections', 'IntersectionObserverController::computeIntersections'],
  paint: ['Paint', 'PaintImage', 'PaintSetup', 'Decode Image', 'Decode LazyPixelRef', 'Draw LazyPixelRef'],
  composite: ['CompositeLayers', 'Commit', 'ProxyMain::BeginMainFrame::commit'],
  script: ['FunctionCall', 'EvaluateScript', 'v8.compile', 'v8.compileModule', 'v8.evaluateModule', 'v8.run', 'V8.Execute', 'v8.callFunction', 'TimerFire',
    'FireAnimationFrame', 'FireIdleCallback', 'EventDispatch', 'RunMicrotasks', 'XHRReadyStateChange', 'XHRLoad', 'CompileScript', 'CompileCode', 'V8.CompileCode',
    'HandlePostMessage', 'v8.newInstance'],
};
const KIND_OF = new Map(Object.entries(KINDS).flatMap(([kind, names]) => names.map(name => [name, kind])));
const GC = /^(MinorGC|MajorGC|V8\.GC|BlinkGC|CppGC|ThreadState::)/;
export const kindOf = name => KIND_OF.get(name) ?? (GC.test(name) ? 'gc' : 'other');
const RASTER = new Set(['RasterTask', 'ImageDecodeTask', 'RasterizerTaskImpl::RunOnWorkerThread']);
const INVALIDATIONS = { LayoutInvalidationTracking: 'layout', StyleRecalcInvalidationTracking: 'style', StyleInvalidatorInvalidationTracking: 'style',
  ScheduleStyleInvalidationTracking: 'style' };
const thread = event => `${event.pid}:${event.tid}`;

/** Spans from complete events and begin/end pairs, each with its args (an end's merged over its begin's). */
export function traceSpans(events) {
  const spans = [], open = new Map();
  for (const event of events) {
    if (event.ph === 'X') spans.push({ ...event, dur: event.dur ?? 0 });
    else if (event.ph === 'B') { const key = thread(event); if (!open.has(key)) open.set(key, []); open.get(key).push(event); }
    else if (event.ph === 'E') {
      const begun = open.get(thread(event))?.pop();
      if (begun) spans.push({ ...begun, ph: 'X', dur: event.ts - begun.ts, args: { ...begun.args, ...event.args } });
    }
  }
  return spans;
}

/** Each span's own time, its children's taken out, and its depth on its thread. */
export function selfTimes(spans) {
  const byThread = new Map();
  for (const span of spans) { const key = thread(span); if (!byThread.has(key)) byThread.set(key, []); byThread.get(key).push(span); }
  for (const list of byThread.values()) {
    list.sort((a, b) => a.ts - b.ts || b.dur - a.dur);
    const stack = [];
    for (const span of list) {
      while (stack.length && stack.at(-1).ts + stack.at(-1).dur <= span.ts) stack.pop();
      span.self = span.dur;
      span.depth = stack.length;
      const parent = stack.at(-1);
      if (parent) parent.self -= Math.min(span.dur, parent.ts + parent.dur - span.ts);
      stack.push(span);
    }
  }
  return spans;
}

const ms = microseconds => Math.round(microseconds / 10) / 100;
const top = (counts, limit) => [...counts.values()].sort((a, b) => b.count - a.count || b.ms - a.ms).slice(0, limit)
  .map(entry => ({ ...entry, ms: ms(entry.ms), nodes: entry.nodes ? [...entry.nodes].slice(0, 4) : undefined }));

/** A playing window's trace summarised: its frames, the main thread's busy time and its time by kind, raster and GPU
 * time off it (all in ms over the window), and what dirtied style and forced layout. The window is the page's marks
 * (`TRACE_MARKS`), else the whole trace. */
export function summariseTrace(events, marks = TRACE_MARKS) {
  const names = new Map();
  for (const event of events) if (event.ph === 'M' && event.name === 'thread_name') names.set(thread(event), event.args?.name ?? '');
  const markAt = name => events.find(event => event.name === name && String(event.cat ?? '').includes('user_timing'))?.ts ?? null;
  const spans = selfTimes(traceSpans(events));
  let from = markAt(marks.from), to = markAt(marks.to);
  const marked = from !== null && to !== null && to > from;
  if (!marked) {
    from = Infinity; to = -Infinity;
    for (const event of events) if (event.ts > 0) { from = Math.min(from, event.ts); to = Math.max(to, event.ts + (event.dur ?? 0)); }
    for (const span of spans) to = Math.max(to, span.ts + span.dur);
  }
  const inside = event => event.ts >= from && event.ts <= to;
  // The page's main thread: the renderer's main thread busiest in the window (a pop-out has its own renderer).
  const busy = new Map();
  for (const span of spans) if (inside(span) && span.depth === 0 && names.get(thread(span)) === 'CrRendererMain') busy.set(thread(span), (busy.get(thread(span)) ?? 0) + span.dur);
  const main = [...busy].sort((a, b) => b[1] - a[1])[0]?.[0] ?? null;
  if (main === null) return { marked, seconds: ms(to - from) / 1000, frames: null, reason: 'the trace holds no renderer main thread in the window' };
  const [pid] = main.split(':');
  const kinds = { style: 0, layout: 0, prePaint: 0, paint: 0, composite: 0, script: 0, gc: 0, other: 0 };
  let mainBusy = 0, raster = 0, gpu = 0, compositor = 0, dirtyObjects = 0, layouts = 0;
  const forced = new Map(), invalidated = new Map(), roots = new Map(), scripts = new Map();
  for (const span of spans) {
    if (!inside(span)) continue;
    const key = thread(span), name = names.get(key) ?? '';
    if (key === main) {
      if (span.depth === 0) mainBusy += span.dur;
      const kind = kindOf(span.name);
      kinds[kind] += Math.max(0, span.self);
      if (span.name === 'Layout') {
        layouts++;
        dirtyObjects += span.args?.beginData?.dirtyObjects ?? 0;
        for (const root of span.args?.endData?.layoutRoots ?? []) if (root?.nodeId != null) {
          const held = roots.get(root.nodeId) ?? { node: root.nodeId, count: 0, ms: 0 };
          held.count++; held.ms += span.dur; roots.set(root.nodeId, held);
        }
      }
      const stack = (span.name === 'Layout' || span.name === 'UpdateLayoutTree') && span.args?.beginData?.stackTrace;
      if (stack?.length) {
        const frame = stack[0], cause = `${span.name === 'Layout' ? 'layout' : 'style'} forced by ${frame.functionName || '(anonymous)'} (${String(frame.url ?? '').split('/').pop()}:${(frame.lineNumber ?? 0) + 1})`;
        const held = forced.get(cause) ?? { cause, count: 0, ms: 0 };
        held.count++; held.ms += span.dur; forced.set(cause, held);
      }
      // The script that took longest at the top of a task: what runs between frames as well as in them.
      if (kind === 'script' && span.depth <= 1) {
        const data = span.args?.data ?? {};
        const what = `${span.name}${data.type ? ` ${data.type}` : ''}${data.functionName ? ` ${data.functionName}` : ''}${data.url ? ` (${String(data.url).split('/').pop()}${data.lineNumber != null ? `:${data.lineNumber + 1}` : ''})` : ''}`;
        const held = scripts.get(what) ?? { what, count: 0, ms: 0 };
        held.count++; held.ms += span.dur; scripts.set(what, held);
      }
    } else if (span.name === 'GPUTask') gpu += span.dur;
    else if (RASTER.has(span.name)) raster += Math.max(0, span.self);
    else if (name === 'Compositor' && key.startsWith(`${pid}:`) && span.depth === 0) compositor += span.dur;
  }
  for (const event of events) {
    const kind = INVALIDATIONS[event.name];
    if (!kind || !inside(event)) continue;
    const data = event.args?.data ?? {};
    const cause = `${kind}: ${data.nodeName ?? 'a node'}${data.reason ? ` (${data.reason})` : ''}`;
    const held = invalidated.get(cause) ?? { cause, count: 0, ms: 0, nodes: new Set() };
    held.count++;
    if (data.nodeId != null) held.nodes.add(data.nodeId);
    invalidated.set(cause, held);
  }
  const frameEvents = events.filter(event => event.name === 'BeginMainThreadFrame' && thread(event) === main && inside(event)).length;
  const compositorThread = [...names].find(([key, name]) => name === 'Compositor' && key.startsWith(`${pid}:`))?.[0];
  const drawn = compositorThread ? events.filter(event => event.name === 'BeginFrame' && thread(event) === compositorThread && inside(event)).length : 0;
  return {
    marked, seconds: ms(to - from) / 1000, frames: frameEvents || drawn || null, framesFrom: frameEvents ? 'BeginMainThreadFrame' : drawn ? 'BeginFrame' : null,
    main: ms(mainBusy), kinds: Object.fromEntries(Object.entries(kinds).map(([kind, value]) => [kind, ms(value)])),
    raster: ms(raster), gpu: ms(gpu), compositor: ms(compositor), layouts, dirtyObjects,
    forced: top(forced, 6), invalidated: top(invalidated, 8), layoutRoots: top(roots, 6), scripts: top(scripts, 6),
  };
}

/** The node ids a summary names, for `describeNodes`. */
export const summaryNodes = summary => [...new Set([...(summary.invalidated ?? []).flatMap(entry => entry.nodes ?? []), ...(summary.layoutRoots ?? []).map(entry => entry.node)])].slice(0, 24);

/** Each node's element and the widget it is in, where it is still in the page: `backendNodeId`s are the trace's node ids. */
export async function describeNodes(cdp, ids) {
  const described = {};
  if (!ids.length) return described;
  const DESCRIBE = `function () { const element = this.nodeType === 1 ? this : this.parentElement; if (!element) return null;
    const widget = element.closest('[data-widget]');
    return { element: element.tagName.toLowerCase() + [...element.classList].slice(0, 3).map(name => '.' + name).join(''),
      widget: widget ? widget.getAttribute('aria-label') || widget.dataset.widget : null, connected: element.isConnected }; }`;
  try { await cdp.send('DOM.enable'); } catch { /* resolving may still work */ }
  for (const id of ids) {
    try {
      const { object } = await cdp.send('DOM.resolveNode', { backendNodeId: id });
      const { result } = await cdp.send('Runtime.callFunctionOn', { objectId: object.objectId, functionDeclaration: DESCRIBE, returnByValue: true });
      await cdp.send('Runtime.releaseObject', { objectId: object.objectId }).catch(() => {});
      described[id] = result.value ?? null;
    } catch { described[id] = null; }
  }
  try { await cdp.send('DOM.disable'); } catch { /* already off */ }
  return described;
}

/** A summary's figures a frame, and its causes as text naming each node's widget where it is known. */
export function perFrame(summary) {
  const frames = summary.frames;
  const each = value => (frames ? Math.round(value / frames * 100) / 100 : null);
  const node = id => { const look = summary.described?.[id]; return look ? `${look.element}${look.widget ? ` in ${look.widget}` : ''}` : null; };
  const causes = [
    ...(summary.forced ?? []).map(entry => `${entry.cause} ×${entry.count}`),
    ...(summary.invalidated ?? []).map(entry => `${entry.cause}${entry.nodes?.map(node).filter(Boolean).length ? ` [${[...new Set(entry.nodes.map(node).filter(Boolean))].join(', ')}]` : ''} ×${entry.count}`),
    ...(summary.layoutRoots ?? []).map(entry => `layout root ${node(entry.node) ?? `node ${entry.node}`} ×${entry.count}`),
  ];
  return { main: each(summary.main ?? 0), ...Object.fromEntries(Object.entries(summary.kinds ?? {}).map(([kind, value]) => [kind, each(value)])),
    raster: each(summary.raster ?? 0), gpu: each(summary.gpu ?? 0), compositor: each(summary.compositor ?? 0), causes };
}
