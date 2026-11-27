(() => {
  if (window.__e2e) return;
  const p = window.__e2e = { inputs: [], events: [], loafs: [], frames: [], transitions: [], tracking: false, done: null, doneAt: null,
    mapStates: [], mapModes: {}, popoutStates: [], placeholders: { ms: 0, where: new Set() } };
  try {
    new PerformanceObserver(list => {
      for (const e of list.getEntries()) p.events.push({ name: e.name, start: e.startTime, end: e.startTime + e.duration, run: e.processingEnd - e.processingStart, id: e.interactionId });
    }).observe({ type: 'event', buffered: true, durationThreshold: 16 });
  } catch { p.noEventTiming = true; }
  try {
    new PerformanceObserver(list => {
      for (const e of list.getEntries()) p.loafs.push({ start: e.startTime, dur: e.duration,
        top: (e.scripts || []).slice().sort((a, b) => b.duration - a.duration).slice(0, 3)
          .map(s => `${Math.round(s.duration)} ms ${s.invoker || ''} ${s.sourceFunctionName || ''} ${(s.sourceURL || '').split('/').pop()}:${s.sourceCharPosition}`) });
    }).observe({ type: 'long-animation-frame', buffered: true });
  } catch {   }
  for (const type of ['pointerdown', 'pointerup', 'pointermove', 'click', 'keydown', 'input', 'change', 'wheel'])
    addEventListener(type, e => { if (e.isTrusted && p.inputs.length < 20000) p.inputs.push({ type, t: e.timeStamp }); }, { capture: true, passive: true });

  const LOADING = '[data-state=loading], .skeleton, [aria-busy="true"], .spinner, .is-loading';
  const LIBRARY_OPENING = '.sidebar-row.library-row';
  const loadingText = e => e.matches('[role=status]') && /^Loading\b/.test((e.textContent || '').trim());
  const visuallyHidden = (e, style = getComputedStyle(e)) => {
    if (e.classList?.contains('sr-only') || e.classList?.contains('fh-sr-only')) return true;
    const inset = /^inset\(\s*([^)]+?)\s*\)$/.exec((style?.clipPath || '').trim());
    if (inset) {
      const parts = inset[1].split(/\s+round\b/i)[0].trim().split(/\s+/);
      if (parts.length >= 1 && parts.length <= 4 && parts.every(v => /^[+-]?(?:\d+\.?\d*|\.\d+)%$/.test(v))) {
        const [t, r = t, b = t, l = r] = parts.map(parseFloat);
        if (t + b >= 100 || l + r >= 100) return true;
      }
    }
    const clip = /^rect\(\s*([+-]?(?:\d+\.?\d*|\.\d+))px,\s*([+-]?(?:\d+\.?\d*|\.\d+))px,\s*([+-]?(?:\d+\.?\d*|\.\d+))px,\s*([+-]?(?:\d+\.?\d*|\.\d+))px\s*\)$/.exec((style?.clip || '').trim());
    return Boolean(clip && (style.position === 'absolute' || style.position === 'fixed')
      && (Number(clip[1]) >= Number(clip[3]) || Number(clip[4]) >= Number(clip[2])));
  };
  p.visuallyHidden = visuallyHidden;
  const placeholders = () => [...document.querySelectorAll(`${LOADING}, [role=status]`)].filter(e => (e.matches(LOADING) || loadingText(e)) && shownOnScreen(e)
    && !e.closest(LIBRARY_OPENING)
    && !(p.ignore && e.closest(`article.dash-widget[data-widget="${CSS.escape(p.ignore)}"]`)));
  const shownOnScreen = e => {
    if (!e.checkVisibility({ visibilityProperty: true, opacityProperty: true })) return false;
    if (visuallyHidden(e)) return false;
    let { left, top, right, bottom } = e.getBoundingClientRect();
    left = Math.max(left, 0); top = Math.max(top, 0); right = Math.min(right, innerWidth); bottom = Math.min(bottom, innerHeight);
    for (let a = e.parentElement; a && right > left && bottom > top; a = a.parentElement) {
      const style = getComputedStyle(a);
      if (visuallyHidden(a, style)) return false;
      if (a.matches(':popover-open') && style.overflowX === 'visible' && style.overflowY === 'visible' && style.contain.indexOf('paint') < 0) break;
      if (style.overflowX === 'visible' && style.overflowY === 'visible' && style.contain.indexOf('paint') < 0) continue;
      const r = a.getBoundingClientRect();
      left = Math.max(left, r.left); top = Math.max(top, r.top); right = Math.min(right, r.right); bottom = Math.min(bottom, r.bottom);
      if (a.matches(':popover-open')) break;
    }
    return right > left && bottom > top;
  };
  const placeholderAt = e => e.closest('article.dash-widget')?.getAttribute('aria-label') ?? (e.className?.toString?.() || e.tagName).slice(0, 60);

  p.listenMaps = () => {
    if (p.mapsListening) return true;
    const tauri = window.__TAURI_INTERNALS__;
    const handler = tauri.transformCallback(event => {
      const at = performance.timeOrigin + performance.now();
      for (const state of event.payload) p.mapStates.push({ ...state, receivedAt: at });
      if (p.mapStates.length > 40000) p.mapStates.splice(0, 20000);
    });
    const modes = tauri.transformCallback(event => { p.mapModes[event.payload.viewId] = event.payload.modes; });
    const popout = tauri.transformCallback(event => { p.popoutStates.push({ ...event.payload, receivedAt: performance.timeOrigin + performance.now() }); });
    p.mapsListening = Promise.all([
      tauri.invoke('plugin:event|listen', { event: 'dashboard-map-state', target: { kind: 'Any' }, handler }),
      tauri.invoke('plugin:event|listen', { event: 'dashboard-map-modes', target: { kind: 'Any' }, handler: modes }),
      tauri.invoke('plugin:event|listen', { event: 'popout-state', target: { kind: 'Any' }, handler: popout }),
    ]).then(() => true);
    return p.mapsListening;
  };
  p.mapState = (id, after, test) => p.mapStates.find(state => state.id === id && (state.drawnAt ?? 0) > after && test(state)) ?? null;
  const ERROR = '[data-state=error], [role=alert]';
  const ids = new WeakMap();
  let serial = 0;
  const idOf = article => { let id = ids.get(article); if (id === undefined) ids.set(article, id = ++serial); return id; };
  p.stateOf = article => {
    const error = article.querySelector(ERROR);
    if (error) return { state: 'error', text: (error.textContent || '').trim().slice(0, 200), detail: error.getAttribute('data-detail') };
    if ([...article.querySelectorAll(LOADING)].some(shownOnScreen)) return { state: 'loading' };
    if ([...article.querySelectorAll('canvas')].some(c => c.drawPending && shownOnScreen(c))) return { state: 'loading', drawing: true };
    if ([...article.querySelectorAll('[role=status]')].some(e => /^Loading\b/.test((e.textContent || '').trim()) && shownOnScreen(e))) return { state: 'loading' };
    const content = article.querySelector('.dash-widget-content');
    const notice = content?.querySelector('[data-state=empty], [data-state=unavailable], [data-state=outside], [data-state=not-recorded], [data-state=nothing-chosen], [data-state=no-members]');
    if (notice && (content.textContent || '').trim() === (notice.textContent || '').trim())
      return { state: notice.getAttribute('data-state'), text: (notice.textContent || '').trim().slice(0, 200) };
    if (!(content?.textContent || '').trim() && !content?.querySelector('canvas, svg, img')) return { state: 'blank' };
    return { state: 'content' };
  };
  p.widgets = () => {
    const scroller = document.querySelector('.dash-grid-scroll');
    const view = scroller ? scroller.getBoundingClientRect() : { top: 0, bottom: innerHeight, left: 0, right: innerWidth };
    return [...document.querySelectorAll('article.dash-widget')].map(article => {
      const r = article.getBoundingClientRect();
      const visible = r.width > 0 && r.height > 0 && r.bottom > view.top && r.top < view.bottom && r.right > view.left && r.left < view.right;
      const kind = [...article.classList].find(c => c.startsWith('kind-'))?.slice(5) ?? null;
      return { id: idOf(article), key: article.dataset.widget ?? null, kind, title: article.getAttribute('aria-label'), visible, ...p.stateOf(article) };
    });
  };
  const sample = document.createElement('canvas');
  sample.width = sample.height = 24;
  const sampler = sample.getContext('2d', { willReadFrequently: true });
  const hash = (h, value) => Math.imul(h ^ value, 16777619);
  const hashText = (h, text) => { for (let i = 0; i < text.length; i++) h = hash(h, text.charCodeAt(i)); return hash(h, 1); };
  p.signature = key => {
    const article = document.querySelector(`article.dash-widget[data-widget="${CSS.escape(key)}"]`);
    if (!article) return null;
    const r = article.getBoundingClientRect();
    let h = hashText(2166136261, `${Math.round(r.width)}x${Math.round(r.height)}|${article.className}|${article.getAttribute('style') ?? ''}`);
    const content = article.querySelector('.dash-widget-content');
    h = hashText(h, content?.innerHTML ?? '');
    for (const canvas of content?.querySelectorAll('canvas') ?? []) {
      h = hashText(h, `${canvas.width}x${canvas.height}`);
      try {
        sampler.clearRect(0, 0, 24, 24);
        sampler.drawImage(canvas, 0, 0, 24, 24);
        for (const value of sampler.getImageData(0, 0, 24, 24).data) h = hash(h, value);
      } catch { h = hashText(h, 'unreadable'); }
    }
    for (const img of content?.querySelectorAll('img') ?? []) h = hashText(h, img.currentSrc);
    return (h >>> 0).toString(36);
  };
  const twoFrames = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  const selectList = () => document.querySelector('.select-menu:popover-open .select-options');
  p.selectOptions = async () => {
    const list = selectList();
    if (!list) return null;
    const found = new Map();
    for (let top = 0; ; top += Math.max(32, list.clientHeight)) {
      list.scrollTop = top;
      await twoFrames();
      for (const o of list.querySelectorAll('[role=option][data-option-index]'))
        found.set(Number(o.dataset.optionIndex), { index: Number(o.dataset.optionIndex), label: o.textContent.trim(), disabled: o.disabled,
          selected: o.getAttribute('aria-selected') === 'true', reason: o.disabled ? o.getAttribute('data-tooltip') : null });
      if (list.scrollTop + list.clientHeight >= list.scrollHeight - 1) break;
    }
    const size = Number(list.querySelector('[role=option]')?.getAttribute('aria-setsize') ?? found.size);
    list.scrollTop = 0;
    await twoFrames();
    return { size, options: [...found.values()].sort((a, b) => a.index - b.index) };
  };
  const pickerList = () => document.querySelector('.content-picker:popover-open .content-picker-list');
  p.pickerRows = async () => {
    const list = pickerList();
    if (!list) return null;
    const found = new Map();
    for (let top = 0; ; top += Math.max(24, list.clientHeight)) {
      list.scrollTop = top;
      await twoFrames();
      for (const o of list.querySelectorAll('[role=option][data-value]'))
        found.set(o.dataset.value, { value: o.dataset.value, label: o.textContent.trim(), pos: Number(o.getAttribute('aria-posinset')),
          selected: o.getAttribute('aria-selected') === 'true', disabled: o.getAttribute('aria-disabled') === 'true' });
      if (list.scrollTop + list.clientHeight >= list.scrollHeight - 1) break;
    }
    const option = list.querySelector('[role=option]');
    if (!option && !document.querySelector('.content-picker:popover-open p.content-picker-empty:not([role])')) return null;
    const size = option ? Number(option.getAttribute('aria-setsize')) : 0;
    list.scrollTop = 0;
    await twoFrames();
    return { size, rows: [...found.values()].sort((a, b) => a.pos - b.pos) };
  };
  p.revealPickerRow = async value => {
    const list = pickerList();
    if (!list) return false;
    const selector = `[role=option][data-value="${CSS.escape(value)}"]`;
    for (let top = 0; ; top += Math.max(24, list.clientHeight)) {
      list.scrollTop = top;
      await twoFrames();
      const row = list.querySelector(selector);
      if (row) { row.scrollIntoView({ block: 'nearest' }); await twoFrames(); return true; }
      if (list.scrollTop + list.clientHeight >= list.scrollHeight - 1) return false;
    }
  };
  const countryRowInfo = o => {
    const unnamed = o.querySelector('.country-picker-unnamed');
    if (unnamed) {
      const tag = [...unnamed.childNodes].filter(n => n.nodeType === 3).map(n => n.textContent).join('').trim();
      return { name: '', tag, selected: o.getAttribute('aria-selected') === 'true' };
    }
    const tagEl = o.querySelector('small');
    const nameEl = o.querySelector('[data-overflow-label]') ?? tagEl?.previousElementSibling ?? null;
    return { name: nameEl?.textContent.trim() ?? '', tag: tagEl?.textContent.trim() ?? '', selected: o.getAttribute('aria-selected') === 'true' };
  };
  p.countryRowInfo = countryRowInfo;
  p.countryRows = async () => {
    const list = document.querySelector('.country-picker:popover-open .country-picker-list');
    if (!list) return null;
    const found = new Map();
    for (let top = 0; ; top += Math.max(24, list.clientHeight)) {
      list.scrollTop = top;
      await twoFrames();
      for (const o of list.querySelectorAll('[role=option]')) {
        found.set(Number(o.getAttribute('aria-posinset')), countryRowInfo(o));
      }
      if (list.scrollTop + list.clientHeight >= list.scrollHeight - 1) break;
    }
    const option = list.querySelector('[role=option]');
    list.scrollTop = 0;
    await twoFrames();
    return { size: option ? Number(option.getAttribute('aria-setsize')) : 0, rows: [...found.values()] };
  };
  p.revealCountry = async tag => {
    const list = document.querySelector('.country-picker:popover-open .country-picker-list');
    if (!list) return false;
    for (let top = 0; ; top += Math.max(24, list.clientHeight)) {
      list.scrollTop = top;
      await twoFrames();
      const row = [...list.querySelectorAll('[role=option]')].find(o => countryRowInfo(o).tag === tag);
      if (row) { row.scrollIntoView({ block: 'nearest' }); await twoFrames(); return true; }
      if (list.scrollTop + list.clientHeight >= list.scrollHeight - 1) return false;
    }
  };
  p.revealOption = async index => {
    const list = selectList();
    if (!list) return false;
    for (let tries = 0; tries < 4; tries++) {
      const option = list.querySelector(`[data-option-index="${index}"]`);
      if (option) { option.scrollIntoView({ block: 'nearest' }); await twoFrames(); return true; }
      list.scrollTop = Math.max(0, index * 32 - list.clientHeight / 2);
      await twoFrames();
    }
    return false;
  };
  p.painted = [];
  const afterPaint = new MessageChannel();
  afterPaint.port1.onmessage = ({ data }) => {
    const entry = p.painted.find(f => f.start === data); if (entry) entry.end = performance.now();
    if (data === p.frameStart) sampleFrame();
  };
  let loopId = 0;
  let recordingFrames = false;
  const shouldRun = () => Boolean(p.tracking || p.watching || p.settler || recordingFrames);
  const startLoop = () => {
    if (loopId || !shouldRun()) return;
    loopId = requestAnimationFrame(frame);
  };
  const stopLoopIfIdle = () => {
    if (!shouldRun() && loopId) { cancelAnimationFrame(loopId); loopId = 0; }
  };
  const frame = now => {
    loopId = 0;
    p.painted.push({ start: now, end: null });
    if (p.painted.length > 8000) p.painted.splice(0, 4000);
    afterPaint.port2.postMessage(now);
    p.frames.push(now);
    if (p.frames.length > 8000) p.frames.splice(0, 4000);
    p.frameStart = now;
    if (shouldRun()) loopId = requestAnimationFrame(frame);
  };
  let placeholderSince = null;
  const placeholderEnd = at => { if (placeholderSince !== null) p.placeholders.ms += at - placeholderSince; placeholderSince = null; };
  const sampleFrame = () => {
    if (p.tracking || p.watching) {
      const shown = placeholders();
      placeholderEnd(p.frameStart);
      if (shown.length) {
        placeholderSince = p.frameStart;
        for (const e of shown) if (p.placeholders.where.size < 8) p.placeholders.where.add(placeholderAt(e));
      }
    }
    if (p.tracking) {
      for (const w of p.widgets()) {
        const key = `${w.visible ? 'v' : 'h'}:${w.state}`;
        if (p.last.get(w.id) !== key) { p.last.set(w.id, key); p.transitions.push({ ...w, at: p.frameStart }); }
      }
    }
    if (p.done && p.doneAt === null) { try { if (p.done()) p.doneAt = p.frameStart; } catch {   } }
    if (p.alt && p.altAt === null) { try { if (p.alt()) p.altAt = p.frameStart; } catch {   } }
    p.settler?.();
  };

  p.mark = (done, ignore = null, alt = null, altQuiet = 0) => {
    p.ignore = ignore;
    p.inputs = []; p.events = []; p.loafs = []; p.transitions = []; p.last = new Map(); p.tracking = true;
    p.placeholders = { ms: 0, where: new Set() }; placeholderSince = null;
    p.done = done ? new Function(`return (${done});`) : null; p.doneAt = null;
    p.alt = alt ? new Function(`return (${alt});`) : null; p.altAt = null; p.altQuiet = altQuiet;
    p.t0 = performance.now();
    p.t0Epoch = performance.timeOrigin + p.t0;
    startLoop();
    return p.t0;
  };
  p.inFlight = () => { try { return window.__busy ? window.__busy.kinds() : null; } catch { return null; } };
  p.prepared = timeout => new Promise(resolve => {
    const start = performance.now(); let idle = 0;
    const check = () => {
      const now = performance.now(), kinds = p.inFlight(), preparing = window.__busy?.preparing?.();
      const known = kinds !== null && Number.isFinite(preparing);
      idle = known && preparing === 0 && Object.values(kinds).every(n => n === 0) ? idle + 1 : 0;
      if (!known || idle >= 2 || now - start >= timeout) resolve({ at: performance.timeOrigin + now, complete: known && idle >= 2 });
      else requestAnimationFrame(check);
    };
    check();
  });
  p.perfContext = () => ({ enabled: !!window.__perf, page: document.querySelector('.sidebar-page-list [aria-current=page]')?.textContent.trim() ?? null,
    hour: document.querySelector('input.header-timeline')?.value ?? null, viewport: [innerWidth, innerHeight, devicePixelRatio] });
  p.settle = (timeout, quiet, only = null, over) => new Promise(resolve => {
    let since = null, idle = 0;
    const check = () => {
      const now = performance.now();
      const visible = p.widgets().filter(w => w.visible && (only === null || w.title === only || w.key === only));
      const finished = !p.done || p.doneAt !== null;
      const kinds = p.inFlight();
      const busy = kinds === null ? null : Object.values(kinds).reduce((sum, n) => sum + n, 0);
      const settled = visible.every(w => w.state !== 'loading') && (finished || p.altAt !== null) && placeholders().length === 0;
      since = settled ? since ?? now : null;
      idle = settled && busy === 0 ? idle + 1 : 0;
      const wait = finished ? quiet : Math.max(quiet, p.altQuiet);
      if (over({ settled, busy, idle, heldMs: since === null ? 0 : now - since, wait }) || now - p.t0 >= timeout) {
        p.settler = null;
        p.tracking = false;
        const pending = busy > 0 ? kinds : null;
        requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(() => resolve({ ...p.report(now, !settled || pending !== null, visible),
          inFlight: pending, busySignal: busy !== null }), 0)));
      }
    };
    p.settler = check;
    startLoop();
  });
  p.report = (now, timedOut, visible) => {
    const ended = i => {
      const f = p.painted[i];
      if (!f) return null;
      const long = p.loafs.find(l => l.start <= f.start && f.start <= l.start + l.dur);
      if (long) return long.start + long.dur;
      const bounds = [f.end, p.painted[i + 1]?.start].filter(Number.isFinite);
      return bounds.length ? Math.min(...bounds) : null;
    };
    const shown = t => (t === null || t === undefined ? null : ended(p.painted.findLastIndex(f => f.start <= t)));
    const nextPainted = t => ended(p.painted.findIndex(f => f.start > t));
    const widgets = new Map();
    for (const t of p.transitions) {
      const w = widgets.get(t.id) ?? { id: t.id, key: t.key, kind: t.kind, title: t.title, first: null, ready: null, loadingFrom: null, loadingMax: 0, loaded: false, errors: [], states: [] };
      widgets.set(t.id, w);
      if (w.loadingFrom !== null && (t.state !== 'loading' || !t.visible)) { w.loadingMax = Math.max(w.loadingMax, t.at - w.loadingFrom); w.loadingFrom = null; }
      if (t.visible && t.state === 'loading') { w.loadingFrom = t.at; w.loaded = true; }
      if (t.visible && t.state !== 'loading') { w.seen = true; if (w.first === null) w.first = shown(t.at); if (w.loaded) w.ready = shown(t.at); }
      if (t.state === 'error') w.errors.push(t.detail ? `${t.text} (${t.detail})` : t.text);
      if (t.state === 'empty' || t.state === 'unavailable') w.notice = { state: t.state, text: t.text };
    }
    for (const v of visible) {
      const w = widgets.get(v.id);
      if (!w) continue;
      w.end = v.state; w.visible = true;
      if (w.loadingFrom !== null) w.loadingMax = Math.max(w.loadingMax, now - w.loadingFrom);
    }
    const input = p.inputs.find(i => i.type !== 'pointermove') ?? p.inputs[0] ?? null;
    const lastInput = p.inputs.length ? p.inputs[p.inputs.length - 1].t : null;
    const paints = p.inputs.map(i => p.events.find(e => e.name === i.type && Math.abs(e.start - i.t) < 1)?.end ?? nextPainted(i.t));
    const painted = paints.some(v => v === null) ? null : Math.max(-Infinity, ...paints);
    return { t0: p.t0, now, timedOut, input: input?.t ?? null, last: lastInput, inputType: input?.type ?? null, inputs: p.inputs.length,
      paint: Number.isFinite(painted) ? painted : null,
      doneAt: shown(p.doneAt), doneAtSeen: p.doneAt !== null, altAt: p.altAt, widgets: [...widgets.values()].map(({ loadingFrom, states, ...w }) => w),
      placeholderMs: (placeholderEnd(now), p.placeholders.ms), placeholderAt: [...p.placeholders.where], placeholderStill: [...new Set(placeholders().map(placeholderAt))],
      loafs: p.loafs.filter(l => l.dur >= 50 && l.start + l.dur >= p.t0).map(l => ({ dur: Math.round(l.dur), top: l.top })) };
  };
  p.watch = () => { p.watching = true; placeholderSince = null; p.placeholders = { ms: 0, where: new Set() }; startLoop(); };
  p.unwatch = () => { placeholderEnd(performance.now()); p.watching = false; stopLoopIfIdle(); return { ms: p.placeholders.ms, where: [...p.placeholders.where], still: [...new Set(placeholders().map(placeholderAt))] }; };
  p.startFrames = () => { recordingFrames = true; startLoop(); };
  p.stopFrames = () => { recordingFrames = false; stopLoopIfIdle(); };
  p.gaps = (from, to) => { const f = p.frames.filter(t => t >= from && t <= to); const gaps = []; for (let i = 1; i < f.length; i++) gaps.push(f[i] - f[i - 1]); return gaps; };
})();
