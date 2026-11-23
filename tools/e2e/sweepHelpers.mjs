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
  await throttle(rate);
  try {
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
