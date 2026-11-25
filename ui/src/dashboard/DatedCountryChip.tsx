import { useLayoutEffect, useRef } from 'react';
import { failureText, type Failure } from './queryError.ts';
import { CountryFlagUrl } from '../components/CountryFlag';
import { formatDateTime } from './calendar';
import type { CountryLabel } from './countryFlagsAt';
import type { LiveValue } from '../components/DateTimePicker';

export default function DatedCountryChip({ label, hour, reason, failure, live }: {
  label: ReturnType<CountryLabel> | null; hour: number | null; reason: string | null; failure?: Failure | null; live?: LiveValue;
}) {
  const chip = useRef<HTMLSpanElement>(null);
  const qualified = label !== null && hour !== null && reason === null;
  const named = label?.known === true && !label.nameFailed;
  const failed = failure ?? label?.nameFailure;
  const hasFailure = !!failed || label?.nameFailed === true;
  const errorText = hasFailure ? failureText(failed, "Couldn’t load country name.") : undefined;
  const loading = !hasFailure && qualified && label.known === undefined && !label.nameFailed;
  const unavailable = !qualified || label.nameFailed || label.known === false;
  const note = hasFailure ? errorText : unavailable ? 'Name unavailable' : null;
  const tooltip = (at: number | null) => [note, at === null ? '' : `As of ${formatDateTime(at)}`].filter(Boolean).join(' · ');
  useLayoutEffect(() => {
    if (!live || hour === null) return;
    let shown: number | null | undefined;
    const paint = () => {
      const at = live.read();
      if (at === shown || !chip.current) return;
      shown = at;
      chip.current.setAttribute('data-tooltip', tooltip(at));
    };
    paint();
    return live.subscribe(paint);
  });
  return <span ref={chip} className="dash-widget-country" data-detail={failed?.message ?? label?.nameDetail} data-tooltip={tooltip(hour)}>
    <CountryFlagUrl className="country-flag" url={qualified ? label?.flag : undefined}
      detail={qualified ? label?.flagDetail : undefined} reason={qualified ? label?.flagReason ?? undefined : undefined} pending={qualified ? label?.flagPending : undefined} />
    <span>{hasFailure ? errorText : named && qualified ? label!.name : loading ? 'Loading…' : 'Unavailable'}</span>
  </span>;
}
