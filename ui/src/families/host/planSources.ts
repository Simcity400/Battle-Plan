import type { WidgetSpec } from '../../dashboard/model';
import type { PlaybackController } from '../../dashboard/playback';
import { watchBoundary } from '../../dashboard/widgetTime.ts';
import { planSignature } from '../broker.ts';
import type { CountryPolicy, FamilyPlanContext, FamilyViewportRequest, SourcePlan } from '../plan.ts';
import type { OptionValue } from '../../dashboard/options';

export function gatedByCountry(policy: CountryPolicy, options: Readonly<Record<string, OptionValue>>): boolean {
  if (policy.mode !== 'single') return false;
  const subject = policy.subjectOption === undefined ? undefined : options[policy.subjectOption];
  return subject === undefined || subject === null || subject === '';
}

export function plannedRangeSpan(
  timeMode: WidgetSpec['time']['mode'],
  branch: { readonly firstHour: number; readonly lastHour: number },
  window: { readonly firstHour: number; readonly lastHour: number } | null,
): readonly [number, number] | null {
  if (timeMode !== 'range' || !window) return null;
  const first = Math.max(Math.floor(window.firstHour), branch.firstHour);
  const last = Math.min(Math.floor(window.lastHour), branch.lastHour);
  return first <= last ? [first, last] as const : null;
}

export function plannedSignature(
  plan: SourcePlan | null,
  staticSpan: readonly [number, number] | null,
): string {
  const base = planSignature(plan);
  if (!base || !staticSpan) return base;
  return `${base}|range:${staticSpan[0]}:${staticSpan[1]}`;
}

export function planFamilySources<TAux>(
  baseCtx: Omit<FamilyPlanContext<TAux>, 'staticSpan'>,
  window: { readonly firstHour: number; readonly lastHour: number } | null,
  inputsReady: boolean,
  buildSources: (ctx: FamilyPlanContext<TAux>, view: FamilyViewportRequest) => SourcePlan,
  viewport: { readonly following: boolean; readonly width?: number; readonly height?: number },
): {
  readonly ctx: FamilyPlanContext<TAux>;
  readonly rangeSpan: readonly [number, number] | null;
  readonly plan: SourcePlan | null;
  readonly sig: string;
} {
  const rangeSpan = plannedRangeSpan(baseCtx.widget.time.mode, baseCtx.branch, window);
  const ctx: FamilyPlanContext<TAux> = { ...baseCtx, staticSpan: rangeSpan };
  const plan = inputsReady
    ? buildSources(ctx, {
        following: viewport.following,
        ...(viewport.width !== undefined ? { width: viewport.width } : {}),
        ...(viewport.height !== undefined ? { height: viewport.height } : {}),
        staticSpan: rangeSpan,
      })
    : null;
  return { ctx, rangeSpan, plan, sig: plannedSignature(plan, rangeSpan) };
}

export function watchPlanValid(
  clock: PlaybackController,
  trackOffset: number,
  valid: readonly [number, number] | undefined,
  onCross: () => void,
): (() => void) | undefined {
  if (!valid) return undefined;
  const [lo, hi] = valid;
  const now = Math.floor(clock.getHour()) + trackOffset;
  if (now < lo || now > hi) {
    onCross();
    return undefined;
  }
  if (lo === -Infinity && hi === Infinity) return undefined;
  return watchBoundary(clock, () => [lo - trackOffset, hi + 1 - trackOffset], onCross);
}
