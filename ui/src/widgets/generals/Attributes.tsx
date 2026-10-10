import { barFraction } from '../../charts/table/logic';

/** The compact value-and-bar grid belongs to this scoreboard. */
export function AttributesHeader({ names }: { names: readonly string[] }) {
  return <span className="leader-attributes" aria-label="Attributes">{names.map((name, i) =>
    <span key={i} title={name}>{name}</span>)}</span>;
}

export function Attributes({ values, maxima, names, bars }: {
  values: readonly (number | null)[]; maxima: readonly number[]; names: readonly string[]; bars: boolean;
}) {
  return <span className="leader-attributes">{values.map((value, i) =>
    <span key={i} aria-label={`${names[i]}: ${value ?? 'Not read'}`}>
      <span>{value ?? '—'}</span>
      {bars && value != null && <span className="leader-attribute-bar" aria-hidden="true">
        <i style={{ width: `${barFraction(value, maxima[i]) * 100}%` }} />
      </span>}
    </span>)}</span>;
}
