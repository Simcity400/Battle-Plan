// In-page probe of the acceptance suite: trusted input times, Event Timing, frames and, while a step is
// measured, every widget's visible state per frame. Installed on every document the app loads.
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
  } catch { /* the report says when no long frames were observable */ }
  // Trusted input only: what the suite sends through CDP arrives as the OS would deliver it.
  for (const type of ['pointerdown', 'pointerup', 'pointermove', 'click', 'keydown', 'input', 'change', 'wheel'])
    addEventListener(type, e => { if (e.isTrusted && p.inputs.length < 20000) p.inputs.push({ type, t: e.timeStamp }); }, { capture: true, passive: true });

  const LOADING = '[data-state=loading], .skeleton, [aria-busy="true"], .spinner';
  const loadingText = e => e.matches('[role=status]') && /^Loading\b/.test((e.textContent || '').trim());
  const placeholders = () => [...document.querySelectorAll(`${LOADING}, [role=status]`)].filter(e => (e.matches(LOADING) || loadingText(e)) && shownOnScreen(e)
    && !(p.ignore && e.closest(`article.dash-widget[data-widget="${CSS.escape(p.ignore)}"]`)));
  /** Whether any of `e` shows: not hidden by CSS, and inside the viewport and every ancestor that clips it. */
  const shownOnScreen = e => {
    if (!e.checkVisibility({ visibilityProperty: true, opacityProperty: true })) return false;
    let { left, top, right, bottom } = e.getBoundingClientRect();
    left = Math.max(left, 0); top = Math.max(top, 0); right = Math.min(right, innerWidth); bottom = Math.min(bottom, innerHeight);
    for (let a = e.parentElement; a && right > left && bottom > top; a = a.parentElement) {
      const style = getComputedStyle(a);
      if (a.matches(':popover-open') && style.overflowX === 'visible' && style.overflowY === 'visible' && style.contain.indexOf('paint') < 0) break;
      if (style.overflowX === 'visible' && style.overflowY === 'visible' && style.contain.indexOf('paint') < 0) continue;
      const r = a.getBoundingClientRect();
      left = Math.max(left, r.left); top = Math.max(top, r.top); right = Math.min(right, r.right); bottom = Math.min(bottom, r.bottom);
      // An open popover is in the top layer: nothing outside it clips it.
      if (a.matches(':popover-open')) break;
    }
    return right > left && bottom > top;
  };
  /** Where a visible loading placeholder is: its widget's title, else its own class. */
  const placeholderAt = e => e.closest('article.dash-widget')?.getAttribute('aria-label') ?? (e.className?.toString?.() || e.tagName).slice(0, 60);

  /** Every map state the host sends the page, as the app receives it, with when it arrived. */
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
  /** The first state of map view `id` since epoch `after` that `test` accepts. */
  p.mapState = (id, after, test) => p.mapStates.find(state => state.id === id && (state.drawnAt ?? 0) > after && test(state)) ?? null;
  const ERROR = '[data-state=error], [role=alert]';
  const ids = new WeakMap();
  let serial = 0;
  const idOf = article => { let id = ids.get(article); if (id === undefined) ids.set(article, id = ++serial); return id; };
  /** A widget's state as a user sees it: an error, loading, only an empty or unavailable notice, or content. */
  p.stateOf = article => {
    const error = article.querySelector(ERROR);
    if (error) return { state: 'error', text: (error.textContent || '').trim().slice(0, 200), detail: error.getAttribute('data-detail') };
    if ([...article.querySelectorAll(LOADING)].some(shownOnScreen)) return { state: 'loading' };
    // A chart whose canvas has a draw asked for and not yet run has not shown its content.
    if ([...article.querySelectorAll('canvas')].some(c => c.drawPending && shownOnScreen(c))) return { state: 'loading', drawing: true };
    if ([...article.querySelectorAll('[role=status]')].some(e => /^Loading\b/.test((e.textContent || '').trim()) && shownOnScreen(e))) return { state: 'loading' };
    const content = article.querySelector('.dash-widget-content');
    const notice = content?.querySelector('[data-state=empty], [data-state=unavailable]');
    if (notice && (content.textContent || '').trim() === (notice.textContent || '').trim())
      return { state: notice.getAttribute('data-state'), text: (notice.textContent || '').trim().slice(0, 200) };
    // A body with no text and nothing drawn shows no content, whatever it is made of.
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
  // What a widget shows, as a short string: its size, its text, its drawings' pixels and markup. A setting that
  // changes none of them changed nothing a user can see.
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
    // Its markup: text, bars, classes and inline styles, which is what its own CSS draws from.
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
  /** Every option of the open select menu by its place in the whole list, mounting each window of the virtual list in turn. */
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
  /** Every row of the open content picker, by its value, mounting each window of its virtual list in turn. */
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
    // No rows is a list only when the picker says it has none; loading or failing, it shows no list at all.
    const option = list.querySelector('[role=option]');
    if (!option && !document.querySelector('.content-picker:popover-open p.content-picker-empty:not([role])')) return null;
    const size = option ? Number(option.getAttribute('aria-setsize')) : 0;
    list.scrollTop = 0;
    await twoFrames();
    return { size, rows: [...found.values()].sort((a, b) => a.pos - b.pos) };
  };
  /** Scrolls the open content picker's row of `value` into view; whether it is there. */
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
  /** Every row of the open country picker, mounting each window of its virtual list in turn: its name, tag and whether chosen. */
  p.countryRows = async () => {
    const list = document.querySelector('.country-picker:popover-open .country-picker-list');
    if (!list) return null;
    const found = new Map();
    for (let top = 0; ; top += Math.max(24, list.clientHeight)) {
      list.scrollTop = top;
      await twoFrames();
      for (const o of list.querySelectorAll('[role=option]')) {
        const tag = o.querySelector('small');
        found.set(Number(o.getAttribute('aria-posinset')), { name: tag?.previousElementSibling?.textContent.trim() ?? '', tag: tag?.textContent.trim() ?? '',
          selected: o.getAttribute('aria-selected') === 'true' });
      }
      if (list.scrollTop + list.clientHeight >= list.scrollHeight - 1) break;
    }
    const option = list.querySelector('[role=option]');
    list.scrollTop = 0;
    await twoFrames();
    return { size: option ? Number(option.getAttribute('aria-setsize')) : 0, rows: [...found.values()] };
  };
  /** Scrolls the open country picker's row of `tag` into view; whether it is there. */
  p.revealCountry = async tag => {
    const list = document.querySelector('.country-picker:popover-open .country-picker-list');
    if (!list) return false;
    for (let top = 0; ; top += Math.max(24, list.clientHeight)) {
      list.scrollTop = top;
      await twoFrames();
      const row = [...list.querySelectorAll('[role=option] small')].find(e => e.textContent.trim() === tag);
      if (row) { row.scrollIntoView({ block: 'nearest' }); await twoFrames(); return true; }
      if (list.scrollTop + list.clientHeight >= list.scrollHeight - 1) return false;
    }
  };
  /** Scrolls option `index` of the open select menu into its list's view; whether it is there. */
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
  // A frame's rendering ends before the task its animation callbacks post runs: that is when what it showed painted.
  p.painted = [];
  const afterPaint = new MessageChannel();
  afterPaint.port1.onmessage = ({ data }) => { const entry = p.painted.find(f => f.start === data); if (entry) entry.end = performance.now(); };
  const frame = now => {
    p.painted.push({ start: now, end: null });
    if (p.painted.length > 8000) p.painted.splice(0, 4000);
    afterPaint.port2.postMessage(now);
    p.frames.push(now);
    if (p.frames.length > 8000) p.frames.splice(0, 4000);
    // What this frame paints is sampled after every animation callback, the charts' included: a resize observer's
    // callback runs then, in the same rendering update, so the probe makes one fire each frame it measures.
    p.frameStart = now;
    if ((p.tracking || p.watching || (p.done && p.doneAt === null)) && document.body) {
      if (!sizer.isConnected) { document.body.append(sizer); resized.observe(sizer); }
      sizer.style.width = sizer.style.width === '1px' ? '2px' : '1px';
    }
    requestAnimationFrame(frame);
  };
  // A placeholder a frame painted shows until the next frame paints: its time is from one sampled frame to the next.
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
    if (p.done && p.doneAt === null) { try { if (p.done()) p.doneAt = p.frameStart; } catch { /* the step reports it never finished */ } }
    p.settler?.();
  };
  const sizer = document.createElement('div');
  sizer.setAttribute('aria-hidden', 'true');
  sizer.style.cssText = 'position:fixed;left:0;top:0;width:1px;height:1px;visibility:hidden;pointer-events:none;contain:strict';
  const resized = new ResizeObserver(sampleFrame);
  requestAnimationFrame(frame);

  /** Starts a measured step: what follows is measured from here, and `done`, when given, must hold for it to finish. */
  p.mark = (done, ignore = null) => {
    p.ignore = ignore;
    p.inputs = []; p.events = []; p.loafs = []; p.transitions = []; p.last = new Map(); p.tracking = true;
    p.placeholders = { ms: 0, where: new Set() }; placeholderSince = null;
    p.done = done ? new Function(`return (${done});`) : null; p.doneAt = null;
    p.t0 = performance.now();
    p.t0Epoch = performance.timeOrigin + p.t0;
    return p.t0;
  };
  /** Resolves once every visible widget has left loading for `quiet` ms and `done` holds, or at `timeout`;
   * then reports the step's input, its paint, each touched widget, and when `done` first held. */
  p.settle = (timeout, quiet, only = null) => new Promise(resolve => {
    let since = null;
    const check = () => {
      const now = performance.now();
      // A step that touches one widget waits for it alone; a placeholder elsewhere still counts against it.
      const visible = p.widgets().filter(w => w.visible && (only === null || w.title === only || w.key === only));
      const settled = visible.every(w => w.state !== 'loading') && (!p.done || p.doneAt !== null);
      since = settled ? since ?? now : null;
      if ((settled && now - since >= quiet) || now - p.t0 >= timeout) {
        p.settler = null;
        p.tracking = false;
        // The frames up to now finish rendering, and their stamps arrive, before the report reads them.
        requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(() => resolve(p.report(now, !settled, visible)), 0)));
      }
    };
    // Decided after each frame's sample, which sees what that frame paints, so its last frame is counted too.
    p.settler = check;
  });
  p.report = (now, timedOut, visible) => {
    // When frame `i` finished rendering. A long frame's own entry says exactly; a short one's rendering ended before both
    // the task its callbacks posted and the next frame began, so the earlier of those bounds it.
    const ended = i => {
      const f = p.painted[i];
      if (!f) return null;
      const long = p.loafs.find(l => l.start <= f.start && f.start <= l.start + l.dur);
      if (long) return long.start + long.dur;
      const bounds = [f.end, p.painted[i + 1]?.start].filter(Number.isFinite);
      return bounds.length ? Math.min(...bounds) : null;
    };
    // A change sampled in the frame that started at `t` is on screen once that frame has rendered.
    const shown = t => (t === null || t === undefined ? null : ended(p.painted.findLastIndex(f => f.start <= t)));
    // An input before a frame is handled before its animation callbacks, and shows once that frame has rendered.
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
    // Each input painted when its Event Timing entry says; one whose handlers ran within a frame has none, and showed once
    // the frame after it rendered.
    const paints = p.inputs.map(i => p.events.find(e => e.name === i.type && Math.abs(e.start - i.t) < 1)?.end ?? nextPainted(i.t));
    const painted = paints.some(v => v === null) ? null : Math.max(-Infinity, ...paints);
    return { t0: p.t0, now, timedOut, input: input?.t ?? null, last: lastInput, inputType: input?.type ?? null, inputs: p.inputs.length,
      paint: Number.isFinite(painted) ? painted : null,
      doneAt: shown(p.doneAt), doneAtSeen: p.doneAt !== null, widgets: [...widgets.values()].map(({ loadingFrom, states, ...w }) => w),
      placeholderMs: (placeholderEnd(now), p.placeholders.ms), placeholderAt: [...p.placeholders.where],
      loafs: p.loafs.filter(l => l.dur >= 50 && l.start + l.dur >= p.t0).map(l => ({ dur: Math.round(l.dur), top: l.top })) };
  };
  /** Watches for visible loading placeholders over a window no step measures (playback, map interactions). */
  p.watch = () => { p.watching = true; placeholderSince = null; p.placeholders = { ms: 0, where: new Set() }; };
  p.unwatch = () => { placeholderEnd(performance.now()); p.watching = false; return { ms: p.placeholders.ms, where: [...p.placeholders.where] }; };
  /** Frame intervals between `from` and `to`. */
  p.gaps = (from, to) => { const f = p.frames.filter(t => t >= from && t <= to); const gaps = []; for (let i = 1; i < f.length; i++) gaps.push(f[i] - f[i - 1]); return gaps; };
})();
