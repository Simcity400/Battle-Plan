import { useMemo, useState } from 'react';
import { Select } from '../../components/Select';
import type { BoundTrack } from '../../dashboard/model';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { useProduct } from '../../dashboard/products';
import type { DesignDetailReply, DesignSection } from '../../generated/dashboard';

export function validateDesignDetail(value: unknown): DesignDetailReply {
  const reply = value as DesignDetailReply;
  if (!reply || typeof reply.branchId !== 'string' || !Number.isInteger(reply.hour)
    || !Array.isArray(reply.designs) || !Array.isArray(reply.sections) || !Array.isArray(reply.warnings)
    || reply.designs.some(row => typeof row.id !== 'string' || !/^\d+$/.test(row.id) || typeof row.name !== 'string')
    || reply.sections.some(section => typeof section.title !== 'string' || !Array.isArray(section.fields)
      || section.fields.some(field => typeof field.label !== 'string' || (field.value !== null && typeof field.value !== 'string')
        || (field.refusal !== null && typeof field.refusal !== 'string') || (field.value === null) === (field.refusal === null)))) {
    throw new Error('Invalid equipment design detail reply');
  }
  return reply;
}

export function DesignSections({ sections }: { sections: readonly DesignSection[] }) {
  return <>{sections.map((section, index) => <section key={index} className="equipment-design-section">
    <h4>{section.title}</h4>
    {!section.fields.length ? <p className="is-muted">None</p> : <dl>{section.fields.map((field, index) => <div key={index}>
      <dt>{field.label}</dt><dd className={field.refusal === null ? undefined : 'is-muted'}>{field.value ?? `Unavailable: ${field.refusal}`}</dd>
    </div>)}</dl>}
  </section>)}</>;
}

/** The complete saved-design roster is requested only while this section is open.
 * Selected-unit and pair queries use full reference strings throughout. */
export default function EquipmentDesignDetail({ runtime, track, hour, active }: {
  runtime: WidgetRuntime; track: BoundTrack; hour: number; active: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [target, setTarget] = useState('');
  const [source, setSource] = useState('');
  const [ship, setShip] = useState('');
  const [wing, setWing] = useState('');
  const [observer, setObserver] = useState('');
  const [targetCountry, setTargetCountry] = useState('');
  const [fog, setFog] = useState('');
  const valid = (value: string) => /^\d+$/.test(value) ? value : null;
  const request = { sourceId: track.source.id, revision: track.source.revision, product: 'equipment-design-detail', params: {
    branchId: track.branch.id, hour: Math.floor(hour), variant: valid(target), fromVariant: valid(source), ship: valid(ship), wing: valid(wing),
    observer: valid(observer) && Number(observer) <= 4294967295 ? Number(observer) : null, fog: fog === '' ? null : fog === 'on',
    targetCountry: valid(targetCountry) && Number(targetCountry) <= 4294967295 ? Number(targetCountry) : null,
  } };
  const detail = useProduct<DesignDetailReply>(runtime.client, open ? request : null, active && open, validateDesignDetail);
  const choices = useMemo(() => [{ value: '', label: 'Choose a saved design' }, ...(detail.data?.designs ?? []).map(row => ({
    value: row.id, label: `${row.name} · ${row.creator}${row.obsolete ? ' · Obsolete' : ''}`,
  }))], [detail.data?.designs]);
  return <details className="equipment-design-detail" open={open} onToggle={event => setOpen(event.currentTarget.open)}>
    <summary>Equipment designs</summary>
    {open && <>
      <div className="equipment-design-choices"><label>Design <Select label="Saved equipment design" value={target} options={choices} onChange={setTarget} /></label>
        <label>Compare from <Select label="Source equipment design" value={source} options={choices} onChange={setSource} /></label>
        <label>Ship reference <input aria-label="Ship reference" inputMode="numeric" value={ship} onChange={event => setShip(event.target.value)} /></label>
        <label>Wing reference <input aria-label="Wing reference" inputMode="numeric" value={wing} onChange={event => setWing(event.target.value)} /></label>
        <label>Observer country reference <input aria-label="Observer country reference" inputMode="numeric" value={observer} onChange={event => setObserver(event.target.value)} /></label>
        <label>Target country (blank uses design creator) <input aria-label="Target country reference" inputMode="numeric" value={targetCountry} onChange={event => setTargetCountry(event.target.value)} /></label>
        <label>Reveal query fog <Select label="Reveal query fog" value={fog} onChange={setFog} options={[
          { value: '', label: 'Recorded game switch' }, { value: 'on', label: 'Fog enabled' }, { value: 'off', label: 'Fog disabled' },
        ]} /></label></div>
      {(ship && !valid(ship) || wing && !valid(wing)) && <p>Use the unit's complete numeric reference.</p>}
      {detail.error ? <p role="alert">{detail.error} <button type="button" onClick={detail.retry}>Retry</button></p>
        : !detail.data ? <p>Loading equipment designs…</p>
          : <>{detail.data.warnings.map((warning, index) => <p key={index} className="is-muted">{warning}</p>)}
            <DesignSections sections={detail.data.sections} /></>}
    </>}
  </details>;
}
