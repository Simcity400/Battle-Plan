export const OUTCOME_RANK = Object.freeze({ loading: -1, content: 0, 'loading-recovered': 1, outside: 2, 'not-recorded': 2, refusal: 3, error: 4 });

export async function restoreCountHour(hour, session, seek) {
  const line = await session.timeline();
  await seek((hour - line.min) / (line.max - line.min));
  if ((await session.timeline()).value !== hour) throw new Error('unmeasured: could not restore the fixed starting hour after free-running playback');
}

export function mergeWidgetOutcomes(widgets, observed) {
  return widgets.map(widget => {
    const seen = observed?.[widget.id];
    if (!seen) return widget;
    const worse = (OUTCOME_RANK[seen.outcome] ?? 0) > (OUTCOME_RANK[widget.outcome ?? 'content'] ?? 0);
    return { ...widget, ...(worse ? { outcome: seen.outcome, detail: seen.detail } : {}),
      ...(seen.loading ? { loading: seen.loading } : {}) };
  });
}

export async function runThrottled(throttle, rate, fn) {
  try {
    await throttle(rate);
    return await fn();
  } finally {
    await throttle(1).catch(() => {});
  }
}

export async function measureIdleCost({ pausePlayback, waitPlayButton, setSpeed, clickPlay, now, frames, onFailure }) {
  try {
    await pausePlayback();
    await waitPlayButton();
    await setSpeed(1);
    await clickPlay();
    const from = await now();
    await frames(120);
    const to = await now();
    await pausePlayback();
    return { from, to };
  } catch (error) {
    onFailure?.(error);
    await pausePlayback().catch(() => {});
    return undefined;
  }
}

export function createOutcomeWatcher({ grid, playHost, isPlaying, classify, observe, requestFrame, cancelFrame, readHour = () => null, waitForPlaying = false }) {
  const rank = { content: 0, 'loading-recovered': 1, outside: 2, 'not-recorded': 2, refusal: 3, error: 4 };
  const worst = new Map();
  const loading = new Map();
  const firstLoading = new Map();
  let active = !waitForPlaying || isPlaying();
  let stopped = false;
  let rafId = 0;
  let loadingFrame = 0;
  let playStop = null;
  let gridStop = null;
  const remember = (id, next) => {
    const prev = worst.get(id);
    if (!prev || (rank[next.outcome] ?? 0) > (rank[prev.outcome] ?? 0)) worst.set(id, next);
  };
  const countLoading = () => {
    loadingFrame = 0;
    for (const pending of loading.values()) pending.frames++;
    scheduleLoading();
  };
  const scheduleLoading = () => {
    if (!stopped && loading.size && !loadingFrame && requestFrame) loadingFrame = requestFrame(countLoading);
  };
  const finishLoading = (id, { firstHour, frames, observation }) => {
    const window = { firstHour, lastHour: readHour(), frames, ...(observation ? { observation } : {}) };
    if (!firstLoading.has(id)) firstLoading.set(id, window);
    return window;
  };
  const recordArticle = a => {
    const id = a?.dataset?.widget;
    if (!id) return;
    const next = classify(a, true);
    if (next.outcome === 'loading') {
      if (!loading.has(id)) loading.set(id, { firstHour: readHour(), frames: 0, detail: next.detail, observation: next.observation });
      scheduleLoading();
      return;
    }
    const pending = loading.get(id);
    if (pending) {
      loading.delete(id);
      if (!loading.size && loadingFrame) {
        cancelFrame?.(loadingFrame);
        loadingFrame = 0;
      }
      const { firstHour, lastHour, frames } = finishLoading(id, pending);
      if (next.outcome === 'content') {
        const span = Number.isFinite(firstHour) && Number.isFinite(lastHour) ? ` across ${Math.abs(lastHour - firstHour)} game hours` : '';
        remember(id, { outcome: 'loading-recovered', detail: `Loading recovered after ${frames} ${frames === 1 ? 'frame' : 'frames'}${span}` });
      }
    }
    remember(id, next);
  };
  const scanAll = () => {
    if (!active || !grid) return;
    for (const a of grid.querySelectorAll('article.dash-widget')) recordArticle(a);
  };
  const stopActivation = () => {
    if (rafId) { cancelFrame?.(rafId); rafId = 0; }
    if (playStop) { playStop(); playStop = null; }
  };
  const activate = () => {
    if (active) return;
    active = true;
    stopActivation();
    scanAll();
  };
  const onGridMutations = records => {
    if (!active) {
      if (isPlaying()) activate();
      return;
    }
    const touched = new Set();
    const collect = node => {
      if (!node) return;
      const el = node.nodeType === 1 ? node : node.parentElement;
      if (!el) return;
      const host = el.closest?.('article.dash-widget');
      if (host) { touched.add(host); return; }
      if (typeof el.querySelectorAll === 'function') {
        for (const a of el.querySelectorAll('article.dash-widget')) touched.add(a);
      }
    };
    for (const rec of records ?? []) {
      collect(rec.target);
      for (const added of rec.addedNodes ?? []) collect(added);
    }
    for (const a of touched) recordArticle(a);
  };
  if (grid) gridStop = observe(grid, { subtree: true, childList: true, attributes: true, characterData: true }, onGridMutations);
  if (active) {
    scanAll();
  } else {
    const checkPlaying = () => {
      if (!stopped && !active && isPlaying()) activate();
    };
    if (playHost) playStop = observe(playHost, { subtree: true, childList: true, attributes: true, attributeFilter: ['aria-label'] }, checkPlaying);
    const poll = () => {
      rafId = 0;
      if (stopped || active) return;
      if (isPlaying()) { activate(); return; }
      rafId = requestFrame ? requestFrame(poll) : 0;
    };
    if (requestFrame) rafId = requestFrame(poll);
  }
  return {
    end() {
      stopped = true;
      stopActivation();
      if (active || isPlaying()) { active = true; scanAll(); }
      if (loadingFrame) { cancelFrame?.(loadingFrame); loadingFrame = 0; }
      for (const [id, pending] of loading) {
        finishLoading(id, pending);
        remember(id, { outcome: 'error', detail: pending.detail ?? 'still loading' });
      }
      if (gridStop) { gridStop(); gridStop = null; }
      return Object.fromEntries([...worst].map(([id, outcome]) => [id, firstLoading.has(id) ? { ...outcome, loading: firstLoading.get(id) } : outcome]));
    },
  };
}

export function buildWidgetClassifierExpr(classifySnapshot) {
  return `((a, tracking = false) => {
    const classify = ${classifySnapshot.toString()};
    const readDetail = el => el ? (el.getAttribute('data-detail')?.trim() || el.getAttribute('data-tooltip')?.trim() || el.textContent?.trim() || null) : null;
    const bodyState = a.querySelector('.widget-state[data-state]:not(.is-section)') ?? a.querySelector('[data-state]');
    const domState = bodyState?.getAttribute('data-state') ?? null;
    const unavailable = a.querySelector('[data-state="unavailable"]');
    const error = a.querySelector('[data-state="error"], [role="alert"]');
    const outside = a.querySelector('[data-state="outside"]');
    const notRecorded = a.querySelector('[data-state="not-recorded"]');
    const empty = a.querySelector('[data-state="empty"]');
    const probe = window.__e2e?.stateOf?.(a, tracking) ?? null;
    const result = classify({
      domState,
      domDetail: readDetail(bodyState) ?? (domState === 'loading' ? bodyState?.getAttribute('aria-label') : null),
      unavailableText: unavailable ? (readDetail(unavailable) || 'unavailable') : null,
      errorDetail: error ? (readDetail(error) || 'error') : null,
      outsideText: outside ? (readDetail(outside) || 'outside') : null,
      notRecordedText: notRecorded ? (readDetail(notRecorded) || 'not-recorded') : null,
      emptyText: empty?.textContent ?? null,
      probeState: probe?.state ?? null,
      probeDrawing: Boolean(probe?.drawing),
    }, tracking);
    if (result.outcome !== 'loading') return result;
    const element = domState === 'loading' ? bodyState : probe?.element;
    const host = element?.closest('.fh-frame') ?? a.querySelector('.fh-frame');
    const diagnostics = host?.playbackDiagnostics;
    const selector = element ? element.localName + [...element.classList].map(name => '.' + window.CSS.escape(name)).join('') : null;
    return { ...result, observation: {
      source: domState === 'loading' ? 'body-state' : 'probe',
      selector: element?.closest('.fh-overlay') ? '.fh-overlay ' + selector : selector,
      text: element?.textContent?.trim() || element?.getAttribute('aria-label') || null,
      domState,
      bodyStatus: diagnostics?.bodyStatus ?? null,
      metaMode: diagnostics?.metaMode ?? null,
      installedBinding: diagnostics?.installedBinding ?? null,
    } };
  })`;
}

export const QUICK_CATALOGUE_DEFAULT_COUNT = 5;

export const catalogueItemKey = card => `${card.view ? 'view' : 'widget'}/${card.title}`;

export function selectQuickCatalogueCards(cards, history, count = QUICK_CATALOGUE_DEFAULT_COUNT) {
  const recorded = new Set(history?.slowest ?? []);
  const matched = cards.flatMap((card, index) => (recorded.has(catalogueItemKey(card)) ? [index] : []));
  if (matched.length) return { indices: new Set(matched), source: 'history' };
  const available = cards.flatMap((card, index) => (card.unavailable ? [] : [index]));
  const pool = available.length ? available : cards.map((_, index) => index);
  const target = Math.min(Math.max(0, count), pool.length);
  if (!target) return { indices: new Set(), source: 'default' };
  const picked = target === pool.length ? pool
    : target === 1 ? [pool[0]]
    : Array.from({ length: target }, (_, i) => pool[Math.round(i * (pool.length - 1) / (target - 1))]);
  return { indices: new Set(picked), source: 'default' };
}

export function describeUncleanCataloguePage({ widgets = 0, dirty = false, popovers = [] } = {}) {
  const issues = [];
  if (widgets > 0) issues.push(`${widgets} ${widgets === 1 ? 'widget' : 'widgets'} left on page`);
  if (dirty) issues.push('layout still has unsaved changes');
  if (popovers.length > 0) issues.push(`popover stayed open (${popovers.join(', ')})`);
  return issues.length ? `page not clean after card: ${issues.join('; ')}` : null;
}

export async function finishCatalogueCard(item, { cleanup, readState, recover, fail }) {
  let failed = false;
  try {
    await cleanup();
  } catch (error) {
    failed = true;
    await fail(item, 'cleanup', error?.message ?? String(error));
  }
  const unclean = describeUncleanCataloguePage(await readState());
  if (!unclean) return;
  if (!failed) await fail(item, 'cleanup', unclean);
  await recover();
  const remaining = describeUncleanCataloguePage(await readState());
  if (remaining) throw new Error(`cannot continue catalogue after ${item}: ${remaining}`);
}
