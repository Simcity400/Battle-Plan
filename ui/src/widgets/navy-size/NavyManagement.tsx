import { useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import type { GatedHour } from '../../dashboard/regionTime';
import { productRequest } from '../../dashboard/request.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { NavalManagementReply } from '../../generated/dashboard';
import { formatNavalAmount, validateNavalManagement } from './navalManagementData';
import './navyManagement.css';

export interface NavyManagementProps {
  readonly runtime: WidgetRuntime;
  readonly track: BoundTrack;
  readonly tag: string;
  readonly hour: GatedHour;
  readonly active: boolean;
}

export default function NavyManagement({ runtime, track, tag, hour, active }: NavyManagementProps) {
  const [open, setOpen] = useState(true);
  const request = open ? productRequest({
    sourceId: track.source.id,
    revision: track.source.revision,
    product: 'naval-management',
    params: { branchId: track.branch.id, country: countryFromKey(tag), hour },
  }) : null;
  const { data, error, retry } = useProduct<NavalManagementReply>(runtime.client, request, active && open, validateNavalManagement, track.source);
  return (
    <details className="navy-management" open={open} onToggle={event => setOpen(event.currentTarget.open)}>
      <summary>Repairs, refits and reinforcement</summary>
      {open ? (
        <div className="navy-management-content">
          {error ? <WidgetState kind="error" what="naval management" failure={error} onRetry={retry} />
            : !data ? <WidgetState kind="loading" label="Loading naval orders…" />
              : (
                <>
                  <p>Standing orders for fleets held by this country on {formatDateOnly(hour)}.</p>
                  {data.warnings.length > 0 ? <p role="status">Some naval details are unavailable. {data.warnings.join(' ')}</p> : null}
                  {data.sections.map(section => (
                    <section key={section.key}>
                      <h4>{section.title}</h4>
                      <ul>
                        {section.lines.map((line, index) => (
                          <li key={index}>{line.text}{line.amount ? formatNavalAmount(line.amount) : null}</li>
                        ))}
                      </ul>
                    </section>
                  ))}
                  {!data.sections.length ? <p>No naval management entries at this date.</p> : null}
                </>
              )}
        </div>
      ) : null}
    </details>
  );
}
