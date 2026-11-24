import type { ReactNode } from 'react';
import type { IconName } from '../../components/Icons';
import WidgetState from '../../components/WidgetState';
import type { FailureOrigin } from '../../dashboard/follow/page.ts';
import type { Pipeline } from '../../dashboard/follow/pipeline.ts';
import NoMembersState from '../../dashboard/NoMembersState';
import ReachedCountryChoice from '../../dashboard/ReachedCountryChoice';
import { asFailure, type Failure } from '../../dashboard/queryError.ts';
import { coverageOf } from '../../dashboard/widgetTime.ts';
import type { FamilyBodyStatus } from '../plan.ts';

export interface FamilyBodyStateProps {
  readonly mode: string;
  readonly reason: string | null;
  readonly failureOrigin: FailureOrigin;
  readonly empty: string | null;
  readonly emptyDetail?: string | null;
  readonly emptyKind?: 'empty' | 'unavailable';
  readonly subject: string;
  readonly loadingLabel?: string;
  readonly pipeline: Pipeline | null;
  readonly pipe: number | null;
  readonly onRetryStream: () => void;
  readonly onRetryLocal: () => void;
  readonly localError?: Failure | null;
  readonly emptyAction?: ReactNode;
}

export function renderFamilyStatus(
  status: FamilyBodyStatus,
  options: { readonly subject: string; readonly icon?: IconName; readonly loadingLabel?: string },
): ReactNode {
  switch (status.kind) {
    case 'ready':
      return null;
    case 'unbound':
      return <WidgetState kind="empty" icon={options.icon} message={status.message} />;
    case 'loading':
      return <WidgetState kind="loading" label={status.label || options.loadingLabel || `Loading ${options.subject}…`} />;
    case 'outside':
      return <WidgetState kind="outside" read={status.outside ? coverageOf(status.outside) : undefined} />;
    case 'not-recorded':
      return <WidgetState kind="unavailable" message={status.reason} detail={status.detail} />;
    case 'refused':
      return status.technical !== undefined
        ? <WidgetState kind="unavailable" technical={status.technical} />
        : <WidgetState kind="unavailable" message={status.reason} />;
    case 'failed':
      return <WidgetState kind="error" what={options.subject} failure={status.failure} onRetry={status.onRetry} />;
    case 'choose':
      return <WidgetState kind="nothing-chosen" prompt={status.message} />;
    case 'holders':
      return <ReachedCountryChoice tag={status.tag} holders={[...status.holders]} onPick={status.pick} />;
    case 'absent':
      return <WidgetState kind="unavailable" message={status.message} detail={status.detail} />;
    case 'no-members':
      return <NoMembersState reason={status.reason} />;
    case 'empty':
      return <WidgetState kind={status.unavailable ? 'unavailable' : 'empty'} message={status.message} detail={status.detail} />;
  }
}

export function renderFamilyBodyState(props: FamilyBodyStateProps): ReactNode {
  const {
    mode,
    reason,
    failureOrigin,
    empty,
    emptyDetail,
    emptyKind = 'empty',
    subject,
    loadingLabel,
    pipeline,
    pipe,
    onRetryStream,
    onRetryLocal,
    localError = null,
    emptyAction,
  } = props;

  if (localError) {
    return <WidgetState kind="error" what={subject} failure={localError} onRetry={onRetryLocal} />;
  }
  if (mode === 'outside') return <WidgetState kind="outside" />;
  if (mode === 'refused') return <WidgetState kind="unavailable" technical={reason ?? undefined} />;
  if (mode === 'failed') {
    const failure = (pipeline && pipe !== null ? pipeline.widgetError(pipe) : undefined)
      ?? asFailure(new Error(reason ?? `Couldn’t load ${subject}.`));
    const onRetry = pipeline ? (failureOrigin === 'stream' ? onRetryStream : onRetryLocal) : undefined;
    return <WidgetState kind="error" what={subject} failure={failure} onRetry={onRetry} />;
  }
  if (mode === 'reading') return <WidgetState kind="loading" label={loadingLabel ?? `Loading ${subject}…`} />;
  if (empty) {
    return emptyKind === 'empty'
      ? <WidgetState kind="empty" message={empty} detail={emptyDetail ?? undefined} action={emptyAction} />
      : <WidgetState kind="unavailable" message={empty} detail={emptyDetail ?? undefined} action={emptyAction} />;
  }
  return null;
}
