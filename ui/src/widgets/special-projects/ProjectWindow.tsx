import { useCallback, useEffect, useLayoutEffect, useMemo, useRef } from 'react';
import type { CSSProperties } from 'react';
import ArtFallback from '../../components/ArtFallback';
import FloatingPanel from '../../components/FloatingPanel';
import type { FloatingAnchor } from '../../components/FloatingPanel';
import WidgetState from '../../components/WidgetState';
import '../../components/Dossier.css';
import { dayLabel } from '../../charts/draw';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useDrawnPanel } from '../../dashboard/DrawnPanel';
import { useHeadProduct } from '../../dashboard/headProduct';
import type { PanelFeed } from '../../dashboard/panelFeed';
import { panelFeed } from '../../dashboard/panelFeed';
import { DetailValues, Effects, Name, Painted } from './ProjectDetails';
import type { Asked } from './ProjectDetails';
import { detailsStructure, detailTexts, effectsStructure, validateProjectDetails, validateProjectEffects } from './projectDetailsData';
import { projectDate, runtimeProject, scientistName } from './specialProjectsData';
import type { ProjectCard } from './specialProjectsData';
import type { ProjectRuntimeDetails, SpecialProjectDetails, SpecialProjectEffects, SpecialProjectsReply } from '../../generated/dashboard';

const STATES = { running: 'In progress', paused: 'Paused', unknown: 'State unknown', completed: 'Completed', cancelled: 'Cancelled' } as const;
const STATE_TONES = { running: '', paused: ' is-paused', unknown: ' is-quiet', completed: ' is-done', cancelled: ' is-quiet' } as const;

type WindowProps = Asked & { reply: SpecialProjectsReply; card: ProjectCard | null; missing: string; feed: PanelFeed | null; anchor: FloatingAnchor;
  onClose: () => void; onWarnings: (warnings: readonly string[]) => void };

export default function ProjectWindow({ reply, card, missing, feed, anchor, onClose, onWarnings, ...asked }: WindowProps) {
  return <FloatingPanel title="Special project" label="special project details" anchor={anchor} onClose={onClose} className="project-window">
    {card ? <div className="dossier is-blueprint">
      <Summary reply={reply} card={card} feed={feed} active={asked.active} clock={asked.runtime.clock} />
      <Runtime {...asked} card={card} onWarnings={onWarnings} />
    </div> : <div className="dossier-body"><WidgetState kind="unavailable" message={missing} /></div>}
  </FloatingPanel>;
}

function Summary({ reply, card, feed, active, clock }: { reply: SpecialProjectsReply; card: ProjectCard; feed: PanelFeed | null; active: boolean; clock: Asked['runtime']['clock'] }) {
  const facts = useRef<HTMLDivElement>(null);
  const { project, run, info } = card;
  const done = run.state === 'completed';
  useDrawnPanel(facts, clock, done ? null : feed, active);
  const person = reply.people[info.scientist];
  const assigned = info.scientistAsOf !== null && done ? `last assigned ${dayLabel(info.scientistAsOf)}` : null;
  return <>
    <header className="dossier-hero">
      <span className="dossier-art is-blueprint">{info.icon ? <img src={info.icon} alt="" /> : <ArtFallback icon="flask" />}</span>
      <div className="dossier-title">
        <h3>{info.name}</h3>
        <div className="dossier-chips">
          {(run.state === 'paused' || run.state === 'unknown') && <span className={`dossier-chip is-state${STATE_TONES[run.state]}`}>{STATES[run.state]}</span>}
          <span className="dossier-chip" title={assigned ?? undefined}>{person.name && (person.portrait ? <img src={person.portrait} alt="" /> : <ArtFallback icon="helmet" />)}
            <span>{scientistName(reply, info)}</span></span>
        </div>
      </div>
    </header>
    <div className={`dossier-progress${done ? ' is-done' : ''}`} ref={facts} data-project={done ? undefined : project.id}
      style={{ '--project-progress': Math.max(0, run.progress) / 100 } as CSSProperties}>
      {done ? <strong className="dossier-progress-value">{run.completion ? `Completed ${projectDate(run.completion)}` : 'Completed'}</strong> : <>
        <strong className="dossier-progress-value" data-paint="progress">{run.progress}%</strong>
        <span className="special-project-progress" aria-hidden="true"><span /></span>
      </>}
    </div>
  </>;
}

function Runtime({ runtime, widget, tag, active, card, onWarnings }: Asked & { card: ProjectCard; onWarnings: (warnings: readonly string[]) => void }) {
  const box = useRef<HTMLDivElement | null>(null);
  const head = useHeadProduct<SpecialProjectDetails>(runtime, widget, 'special-project-details', { country: countryFromKey(tag) },
    { active, validate: validateProjectDetails, structure: detailsStructure });
  const { data, error, retry, root } = head;
  useDrawnPanel(box, runtime.clock, panelFeed('special-project-details', head, detailsStructure, widget, runtime), active);
  const values = useMemo(() => data ? detailTexts(data) : new Map<string, string>(), [data]);
  const warnings = JSON.stringify(data?.warnings ?? []);
  useEffect(() => onWarnings(JSON.parse(warnings) as string[]), [warnings, onWarnings]);
  useEffect(() => () => onWarnings([]), [onWarnings]);
  const ref = useCallback((element: HTMLDivElement | null) => { box.current = element; root(element); }, [root]);
  const match = data ? runtimeProject(data, card) : null;
  return <div className="dossier-body" ref={ref}>
    {error ? <WidgetState kind="error" failure={error} onRetry={retry} /> : !data || !match ? <WidgetState kind="loading" label="Loading project rewards…" />
      : typeof match === 'string' ? <p className="dossier-missing">{match}</p>
        : <DetailValues.Provider value={values}><Rewards runtime={runtime} widget={widget} tag={tag} active={active} project={match} done={card.run.state === 'completed'} /></DetailValues.Provider>}
  </div>;
}

function Rewards({ runtime, widget, tag, active, project, done }: Asked & { project: ProjectRuntimeDetails; done: boolean }) {
  const box = useRef<HTMLDivElement>(null);
  const effects = useHeadProduct<SpecialProjectEffects>(runtime, widget, 'special-project-effects', { country: countryFromKey(tag), owner: project.owner, project: project.id },
    { active, validate: validateProjectEffects, structure: effectsStructure });
  const state = effects.error ? effects.error.message : !effects.data ? 'Loading effects…' : effects.data.unavailable;
  useLayoutEffect(() => { effects.root(box.current); }, [effects.root]);
  useDrawnPanel(box, runtime.clock, panelFeed('special-project-effects', effects, effectsStructure, widget, runtime), active);
  const at = `j:${project.owner}:${project.id}`;
  return <div ref={box} className="project-window-sections">
    {!done && <section className="dossier-section">
      <h4>Pending reward</h4>
      {!project.pendingKnown ? <p className="dossier-missing">Pending reward not recorded.</p> : !project.pending ? <p className="dossier-note">No reward choice pending.</p> : <>
        <p className="project-page-reward"><Name label={project.pending.label} /></p>
        {project.pending.description && <p className="dossier-note">{project.pending.description}</p>}
        {project.pending.unavailable && <p className="dossier-missing">{project.pending.unavailable}</p>}
        <div className="project-page-options">{project.pending.options.map((option, index) => <div className="project-page-option" key={index}>
          <h5><Name label={option.label} /></h5><Effects prefix={String(index)} value={effects.data?.options[index]} state={state} /></div>)}</div>
      </>}
    </section>}
    <section className="dossier-section">
      <h4>Iterations{project.choices && project.choices.length > 0 && <span className="project-window-count">{project.choices.length}</span>}</h4>
      {project.choices === null ? <p className="dossier-missing">Iterations not recorded.</p> : project.choices.length === 0 ? <p className="dossier-note">No iterations yet.</p>
        : <ol className="project-window-choices">{project.choices.map(choice => {
          const gave = effects.data?.iterations.find(item => item.ordinal === choice.ordinal)?.effects;
          return <li key={choice.ordinal}>
            <time>{dayLabel(choice.date)}</time>
            <div className="project-window-choice">
              <p><b><Name label={choice.reward} /></b>{choice.option.name && <><i aria-hidden="true">→</i><Name label={choice.option} /></>}</p>
              <Effects prefix={'iteration:' + choice.ordinal} value={gave} state={effects.data && !gave ? 'Effects of this iteration were not recorded.' : state} />
            </div>
          </li>;
        })}</ol>}
    </section>
    <section className="dossier-section">
      <h4>Project output</h4>
      <Effects prefix="output" value={effects.data?.output} state={state} />
    </section>
    <details className="dossier-section project-page-variables">
      <summary>Project variables <Painted at={`${at}:read`} /></summary>
      {project.variables === null ? <p className="dossier-missing">Variables not recorded.</p> : project.variables.length === 0 ? <p className="dossier-note">No project variables.</p>
        : <dl>{project.variables.map((variable, index) => <div key={index}><dt>{variable.name}</dt><dd><Painted at={`${at}:var:${index}`} /></dd></div>)}</dl>}
      {project.seed !== null && <p className="dossier-note">Variable random seed <Painted at={`${at}:seed`} /></p>}
    </details>
  </div>;
}
