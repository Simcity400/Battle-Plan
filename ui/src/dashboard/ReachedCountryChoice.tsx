import { Icon } from '../components/Icons';
import { formatDate, formatDateTime, gameDate } from './calendar.ts';
import type { CountryHolder } from './reachedCountry';

const dated = (hour: number) => { const date = gameDate(hour); return date ? formatDate(date) : formatDateTime(hour); };

export default function ReachedCountryChoice({ tag, holders, onPick }: { tag: string | null; holders: readonly CountryHolder[]; onPick: (holder: CountryHolder) => void }) {
  return <div className="widget-state"><Icon name="info" /><span>{tag} named more than one country in this campaign. Which one?</span>
    {holders.map(holder => <button key={holder.origin} type="button" className="button small" onClick={() => onPick(holder)}>
      {tag} from {dated(holder.firstHour)} to {dated(holder.lastHour)}</button>)}</div>;
}
