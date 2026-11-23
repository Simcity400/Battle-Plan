import type { ReactNode } from 'react';
import WidgetState from '../../components/WidgetState';
import type { FailureOrigin } from '../../dashboard/follow/page.ts';
import type { Pipeline } from '../../dashboard/follow/pipeline.ts';
import { asFailure, type Failure } from '../../dashboard/queryError.ts';

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
  if (empty) return <WidgetState kind={emptyKind} message={empty} detail={emptyDetail ?? undefined} />;
  return null;
}
