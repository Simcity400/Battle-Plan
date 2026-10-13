import { CountryFlag, CountryFlagUrl } from '../components/CountryFlag';
import { formatDateTime } from './calendar';
import type { CountryLabel } from './countryFlagsAt';

/** Callers resolve the native country and supply the label at the displayed hour. */
export default function DatedCountryChip({ label, hour, reason }: {
  label: ReturnType<CountryLabel> | null; hour: number | null; reason: string | null;
}) {
  const qualified = label !== null && hour !== null && reason === null;
  const named = label?.known === true && !label.nameFailed;
  const loading = qualified && label.known === undefined && !label.nameFailed;
  const unavailable = !qualified || label.nameFailed || label.known === false;
  const date = hour === null ? '' : `As of ${formatDateTime(hour)}`;
  return <span className="dash-widget-country" data-tooltip={[unavailable ? 'Name unavailable' : null, date].filter(Boolean).join(' · ')}>
    <CountryFlagUrl className="country-flag" url={qualified ? label?.flag : undefined}
      reason={qualified ? label?.flagReason ?? undefined : undefined} pending={qualified ? label?.flagPending : undefined} />
    <span>{named && qualified ? label!.name : loading ? 'Loading…' : 'Unavailable'}</span>
  </span>;
}

/** The header of a breakdown that has no country yet: an unchosen selection. */
export function ChooseCountryChip() {
  return <span className="dash-widget-country"><CountryFlag /><span>Choose a country</span></span>;
}
