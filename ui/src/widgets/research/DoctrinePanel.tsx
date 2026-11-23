import { useGate } from '../../dashboard/regionGate';
import { axisAt, SPEC } from '../../dashboard/regionTime';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useMemo, useState } from 'react';
import type { ComponentType } from 'react';
import type { WidgetProps } from '../../dashboard/definitions';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import { useHeadHour } from '../army-size/headHour';
import { useProduct } from '../../dashboard/products';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import WidgetState from '../../components/WidgetState';
import type { DoctrineReply, DoctrineReward } from '../../generated/dashboard';
import { validateDoctrines, masteryText } from './doctrineData';
import './doctrines.css';
import { productRequest } from '../../dashboard/request.ts';

export function ResearchWithDoctrines({ body: Body, ...props }: WidgetProps & { body: ComponentType<WidgetProps> }) {
  const [researchWarnings, setResearchWarnings] = useState<string[]>([]);
  const [doctrineWarnings, setDoctrineWarnings] = useState<string[]>([]);
  const research = useCallback((next: string[]) => setResearchWarnings(old => JSON.stringify(old) === JSON.stringify(next) ? old : next), []);
  const doctrines = useCallback((next: string[]) => setDoctrineWarnings(old => JSON.stringify(old) === JSON.stringify(next) ? old : next), []);
  const warnings = useMemo(() => [...new Set([...researchWarnings, ...doctrineWarnings])], [researchWarnings, doctrineWarnings]);
  useDiagnostics(props.onDiagnostics, warnings);
  return <div className="research-with-doctrines">
    <div className="research-primary"><Body {...props} onDiagnostics={research} /></div>
    <DoctrinePanel {...props} onDiagnostics={doctrines} />
  </div>;
}

function DoctrinePanel({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const [open, setOpen] = useState(false);
  const clock = runtime.clock;
  const playbackHour = useHeadHour(clock, active && open && widget.time.mode === 'follow', at => clock.getSnapshot().playing ? 'playing' : String(Math.floor(at)));
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryKey(track);
  const gate = useGate(runtime, widget, SPEC, active && open);
  const hour = widget.time.mode === 'follow' ? gate.hourAt(axisAt(playbackHour)) : gate.hour;
  const request = track && tag && hour !== null ? productRequest({
    sourceId: track.source.id, revision: track.source.revision, product: 'doctrines', params: { branchId: track.branch.id, country: countryFromKey(tag), hour },
  }) : null;
  const { data, error, retry } = useProduct<DoctrineReply>(runtime.client, request, active && open, validateDoctrines, track?.source);
  const diagnostics = useMemo(() => data?.warnings ?? [], [data]);
  useDiagnostics(onDiagnostics, diagnostics);
  return <details className="doctrine-panel" open={open} onToggle={event => setOpen(event.currentTarget.open)}>
    <summary>Doctrines <span>Choices, mastery and bonuses</span></summary>
    {open && <div className="doctrine-body">
      {!request ? <p role="status">Choose a recorded country and time to see its doctrines.</p>
        : error ? <WidgetState kind="error" failure={error} onRetry={retry} />
        : !data ? <p role="status">Loading doctrines…</p>
        : <>
          {data.folders.map(folder => <section key={folder.ordinal} className="doctrine-folder" aria-label={folder.name}>
            <h3>{folder.name}</h3><p>{folder.grandDoctrine ?? 'No grand doctrine selected'}</p>
            {folder.authoredCost && <p>Base cost: {folder.authoredCost}</p>}
            {folder.benefits && <p className="doctrine-benefits">{folder.benefits}</p>}
            {folder.milestones.map((reward, index) => <Reward key={index} reward={reward} milestone />)}
            {folder.tracks.map(item => <div className="doctrine-track" key={item.ordinal}>
              <h4>{item.name}</h4><p>{item.subDoctrine ?? 'No subdoctrine selected'}</p>
              {item.authoredCost && <p>Base cost: {item.authoredCost}</p>}
              {item.benefits && <p className="doctrine-benefits">{item.benefits}</p>}
              <dl><div><dt>Rewards unlocked</dt><dd>{item.rewards}</dd></div>
                <div><dt>Mastery</dt><dd>{masteryText(item.mastery)}</dd></div>
                <div><dt>Mastery bank</dt><dd>{masteryText(item.masteryBank)}</dd></div>
                <div><dt>Daily mastery gain</dt><dd>{masteryText(item.dailyMastery)}</dd></div>
                <div><dt>Unit contribution</dt><dd>{masteryText(item.unitDailyMastery)}</dd></div>
              </dl>
              {item.rewardRequirements.length > 0 && <p>Mastery per reward: {item.rewardRequirements.map(masteryText).join(' · ')}</p>}
              {item.rewardBenefits.map((reward, index) => <Reward key={index} reward={reward} />)}
              {item.commanders.length > 0 && <details><summary>Commander contributions</summary><dl>{item.commanders.map((fact, index) => <div key={index}><dt>{fact.name}</dt><dd>{fact.value}</dd></div>)}</dl></details>}
            </div>)}
            {folder.completed.length > 0 && <div><h4>Completed subdoctrines</h4><ul>{folder.completed.map((name, index) => <li key={index}>{name}</li>)}</ul></div>}
          </section>)}
          {!data.folders.length && <p>No doctrine folders recorded for this country.</p>}
          {data.inventory.map((item, index) => <details key={`${item.kind}:${index}`}><summary>{item.kind} {item.ordinal + 1}</summary>
            <dl>{item.fields.map((fact, index) => <div key={index}><dt>{fact.name}</dt><dd>{fact.value || '—'}</dd></div>)}</dl>
          </details>)}
        </>}
    </div>}
  </details>;
}

function Reward({ reward, milestone = false }: { reward: DoctrineReward; milestone?: boolean }) {
  return <details className="doctrine-reward"><summary>{reward.name}{reward.unlocked === true ? ' · Unlocked' : reward.unlocked === false ? ' · Locked' : ''}</summary>
    <p>{milestone ? 'Complete this track to unlock.' : `Mastery required: ${masteryText(reward.mastery)}`}</p>
    {reward.description && <p className="doctrine-benefits">{reward.description}</p>}
  </details>;
}
