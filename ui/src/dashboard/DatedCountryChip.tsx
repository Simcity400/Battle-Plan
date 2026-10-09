import { CountryFlagUrl } from '../components/CountryFlag';
import { formatDateTime } from './calendar';
import type { CountryLabel } from './countryFlagsAt';

/** Callers resolve the native country and supply the label at the displayed hour. */
export default function DatedCountryChip({ label, hour, reason }: {
  label: ReturnType<CountryLabel> | null; hour: number | null; reason: string | null;
}) {
  const qualified = label !== null && hour !== null && reason === null;
  const named = label?.known === true && !label.nameFailed;
  const unavailable = reason ?? (hour === null || !label ? 'Selected country unavailable' : label.nameFailed
    ? 'The country name request failed.' : label.known === false ? 'The country name is unavailable at this date.'
      : !named ? 'The country name is loading.' : null);
  const date = hour === null ? '' : `Country shown at ${formatDateTime(hour)}.`;
  return <span className="dash-widget-country" data-tooltip={[unavailable, date].filter(Boolean).join(' ')}>
    <CountryFlagUrl className="country-flag" url={qualified ? label?.flag : undefined}
      reason={qualified ? label?.flagReason ?? undefined : reason ?? undefined} pending={qualified ? label?.flagPending : undefined} />
    <span>{named && qualified ? label!.name : 'Selected country unavailable'}</span>
  </span>;
}
