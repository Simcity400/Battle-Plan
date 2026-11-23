export const OUTCOME_RANK = Object.freeze({ content: 0, placeholder: 1, refusal: 2, error: 3 });

export function mergeWidgetOutcomes(widgets, observed) {
  return widgets.map(widget => {
    const seen = observed?.[widget.id];
    if (!seen) return widget;
    if ((OUTCOME_RANK[seen.outcome] ?? 0) > (OUTCOME_RANK[widget.outcome ?? 'content'] ?? 0)) {
      return { ...widget, outcome: seen.outcome, detail: seen.detail, notMigrated: seen.notMigrated };
    }
    return widget;
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

export function createOutcomeWatcher({ grid, playHost, isPlaying, classify, observe, requestFrame, cancelFrame, waitForPlaying = false }) {
  const rank = { content: 0, placeholder: 1, refusal: 2, error: 3 };
  const worst = new Map();
  let active = !waitForPlaying || isPlaying();
  let stopped = false;
  let rafId = 0;
  let playStop = null;
  let gridStop = null;
  const recordArticle = a => {
    const id = a?.dataset?.widget;
    if (!id) return;
    const next = classify(a);
    const prev = worst.get(id);
    if (!prev || (rank[next.outcome] ?? 0) > (rank[prev.outcome] ?? 0)) worst.set(id, next);
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
      if (gridStop) { gridStop(); gridStop = null; }
      return Object.fromEntries(worst);
    },
  };
}

export function buildWidgetClassifierExpr(classifySnapshot, refusalPattern) {
  return `(a => {
    const classify = ${classifySnapshot.toString()};
    const refusal = new RegExp(${JSON.stringify(refusalPattern.source)}, 'i');
    const placeholder = a.querySelector('.dash-empty[role="status"]');
    const unavailable = a.querySelector('[data-state="unavailable"]');
    const error = a.querySelector('[data-state="error"], [role="alert"]');
    const empty = a.querySelector('[data-state="empty"]');
    const probe = window.__e2e?.stateOf?.(a) ?? null;
    return classify({
      placeholderText: placeholder?.textContent ?? null,
      unavailableText: unavailable ? (unavailable.getAttribute('data-tooltip')?.trim() || unavailable.textContent || 'unavailable') : null,
      errorDetail: error ? (error.getAttribute('data-detail')?.trim() || error.textContent || 'error') : null,
      emptyText: empty?.textContent ?? null,
      probeState: probe?.state ?? null,
      probeDrawing: Boolean(probe?.drawing),
    }, refusal);
  })`;
}
