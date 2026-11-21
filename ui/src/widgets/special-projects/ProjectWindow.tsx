import { useCallback, useEffect, useLayoutEffect, useMemo, useRef } from 'react';
import type { CSSProperties } from 'react';
import ArtFallback from '../../components/ArtFallback';
import FloatingPanel from '../../components/FloatingPanel';
import type { FloatingAnchor } from '../../components/FloatingPanel';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import { contentTitle } from '../../dashboard/contentTitles';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useDrawnPanel } from '../../dashboard/DrawnPanel';
import { useHeadProduct } from '../../dashboard/headProduct';
import type { PanelFeed } from '../../dashboard/panelFeed';
import { panelFeed } from '../../dashboard/panelFeed';
import { DetailValues, Effects, Name, Painted } from './ProjectDetails';
import type { Asked } from './ProjectDetails';
import { detailsStructure, detailTexts, effectsStructure, validateProjectDetails, validateProjectEffects } from './projectDetailsData';
import { Scientist } from './projectParts';
import { iterationMark, projectDate, runtimeProject } from './specialProjectsData';
import type { ProjectCard } from './specialProjectsData';
import type { ProjectRuntimeDetails, SpecialProjectDetails, SpecialProjectEffects, SpecialProjectsReply } from '../../generated/dashboard';

const STATES = { running: 'In progress', paused: 'Paused', unknown: 'State unknown', completed: 'Completed', cancelled: 'Cancelled' } as const;

type WindowProps = Asked & { reply: SpecialProjectsReply; card: ProjectCard | null; missing: string; feed: PanelFeed | null; anchor: FloatingAnchor;
  onClose: () => void; onWarnings: (warnings: readonly string[]) => void };

export default function ProjectWindow({ reply, card, missing, feed, anchor, onClose, onWarnings, ...asked }: WindowProps) {
  return <FloatingPanel title="Special project" label="special project details" anchor={anchor} onClose={onClose} className="project-window">
    <div className="project-page">
      {card ? <>
        <Summary reply={reply} card={card} feed={feed} active={asked.active} clock={asked.runtime.clock} />
        <Runtime {...asked} card={card} onWarnings={onWarnings} />
      </> : <WidgetState kind="unavailable" message={missing} />}
    </div>
  </FloatingPanel>;
}

function Summary({ reply, card, feed, active, clock }: { reply: SpecialProjectsReply; card: ProjectCard; feed: PanelFeed | null; active: boolean; clock: Asked['runtime']['clock'] }) {
  const facts = useRef<HTMLDListElement>(null);
  const { project, run, info } = card;
  const done = run.state === 'completed';
  useDrawnPanel(facts, clock, done ? null : feed, active);
  const text = run.iterations === null ? null : reply.iterationTexts[run.iterations] ?? null;
  return <>
    <header className="project-page-head">
      <span className="blueprint-tile project-page-art">{info.icon ? <img src={info.icon} alt="" /> : <ArtFallback icon="flask" />}</span>
      <div className="project-page-title">
        <h3>{info.name}</h3>
        <p>{contentTitle(info.specializationName, info.specialization, 'Type name unavailable')}</p>
        <span className="special-project-who"><Scientist reply={reply} info={info} />
          {info.scientistAsOf !== null && done && <small>Last assigned {dayLabel(info.scientistAsOf)}</small>}</span>
      </div>
      <span className={`project-page-state is-${run.state}`}>{STATES[run.state]}</span>
    </header>
    <dl className="project-page-facts" ref={facts} data-project={done ? undefined : project.id} style={{ '--project-progress': Math.max(0, run.progress) / 100 } as CSSProperties}>
      {!done && <div className="project-page-progress"><dt>Prototype progress</dt>
        <dd><span className="special-project-progress" aria-hidden="true"><span /></span><strong data-paint="progress">{run.progress}%</strong></dd></div>}
      <div><dt>Iterations</dt><dd><strong data-paint={done ? undefined : 'iterations'}>{iterationMark(run.iterations)}</strong></dd></div>
      <div><dt>Started</dt><dd>{projectDate(project.start)}</dd></div>
      {run.completion && <div><dt>Completed</dt><dd>{projectDate(run.completion)}</dd></div>}
    </dl>
    {text && <p className="project-page-iteration-text">{text}</p>}
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
  return <div className="project-page-body" ref={ref}>
    {error ? <WidgetState kind="error" failure={error} onRetry={retry} /> : !data || !match ? <WidgetState kind="loading" label="Loading project rewards…" />
      : typeof match === 'string' ? <p className="project-page-missing">{match}</p>
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
  return <div ref={box} className="project-page-sections">
    {!done && <section className="project-page-section">
      <h4>Prototype reward</h4>
      {!project.pendingKnown ? <p className="project-page-missing">Pending reward not recorded.</p> : !project.pending ? <p className="project-page-quiet">No reward choice pending.</p> : <>
        <p className="project-page-reward"><Name label={project.pending.label} /></p>
        {project.pending.description && <p className="project-page-quiet">{project.pending.description}</p>}
        {project.pending.unavailable && <p className="project-page-missing">{project.pending.unavailable}</p>}
        <div className="project-page-options">{project.pending.options.map((option, index) => <div className="project-page-option" key={index}>
          <h5><Name label={option.label} /></h5><Effects prefix={String(index)} value={effects.data?.options[index]} state={state} /></div>)}</div>
      </>}
    </section>}
    <section className="project-page-section">
      <h4>Chosen rewards</h4>
      {project.choices === null ? <p className="project-page-missing">Chosen rewards not recorded.</p> : project.choices.length === 0 ? <p className="project-page-quiet">No rewards chosen yet.</p>
        : <ol className="project-page-choices">{project.choices.map(choice => <li key={choice.ordinal}>
          <time>{dayLabel(choice.date)}</time><span><Name label={choice.reward} /></span><strong><Name label={choice.option} /></strong></li>)}</ol>}
    </section>
    <section className="project-page-section">
      <h4>Project output</h4>
      <Effects prefix="output" value={effects.data?.output} state={state} />
    </section>
    <details className="project-page-variables">
      <summary>Project variables <Painted at={`${at}:read`} /></summary>
      {project.variables === null ? <p className="project-page-missing">Variables not recorded.</p> : project.variables.length === 0 ? <p className="project-page-quiet">No project variables.</p>
        : <dl>{project.variables.map((variable, index) => <div key={index}><dt>{variable.name}</dt><dd><Painted at={`${at}:var:${index}`} /></dd></div>)}</dl>}
      {project.seed !== null && <p className="project-page-quiet">Variable random seed <Painted at={`${at}:seed`} /></p>}
    </details>
  </div>;
}
