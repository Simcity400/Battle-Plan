/** The engine's timing curves and times, with no frame loop: what a drawing worker eases by too. */

/** A CSS `cubic-bezier(x1, y1, x2, y2)` timing curve, as a function of the time fraction `t` in [0, 1]. */
export function bezier(x1: number, y1: number, x2: number, y2: number): (t: number) => number {
  const along = (s: number, p1: number, p2: number) => 3 * (1 - s) * (1 - s) * s * p1 + 3 * (1 - s) * s * s * p2 + s * s * s;
  const slope = (s: number) => 3 * (1 - s) * (1 - s) * x1 + 6 * (1 - s) * s * (x2 - x1) + 3 * s * s * (1 - x2);
  return t => {
    if (t <= 0) return 0;
    if (t >= 1) return 1;
    // The curve's parameter where x reaches t, by Newton's method from t.
    let s = t;
    for (let step = 0; step < 8; step++) {
      const d = slope(s);
      if (Math.abs(d) < 1e-9) break;
      s = Math.min(1, Math.max(0, s - (along(s, x1, x2) - t) / d));
    }
    return along(s, y1, y2);
  };
}

/** CSS's `ease`, the table bars' transition. */
export const cssEase = bezier(0.25, 0.1, 0.25, 1);
/** The rows' glide (`.moving-row`'s transition). */
export const rowEase = bezier(0.2, 0.8, 0.2, 1);

/** How long a row's entry takes: its fade in and its flash out, as `.moving-row`'s entry animation did. */
export const ENTRY_MS = 700;
/** How long a leaving row fades out, as `.moving-row`'s opacity transition did. */
export const EXIT_MS = 450;
