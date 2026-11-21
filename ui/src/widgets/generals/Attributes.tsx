import { barFraction } from '../../charts/table/logic';

export function attributesHeader({ names }: { names: readonly string[] }) {
  return <span className="leader-attributes" aria-label="Attributes">{names.map((name, i) =>
    <span key={i} title={name}>{name}</span>)}</span>;
}

export function attributeCells({ values, maxima, names, bars }: {
  values: readonly (number | null)[]; maxima: readonly number[]; names: readonly string[]; bars: boolean;
}) {
  return <span className="leader-attributes">{values.map((value, i) =>
    <span key={i} aria-label={`${names[i]}: ${value ?? 'Not recorded'}`}>
      <span>{value ?? '—'}</span>
      {bars && value != null && <span className="leader-attribute-bar" aria-hidden="true">
        <i style={{ width: `${barFraction(value, maxima[i]) * 100}%` }} />
      </span>}
    </span>)}</span>;
}
