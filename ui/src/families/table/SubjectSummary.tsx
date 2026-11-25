import { useLayoutEffect, useRef } from 'react';
import { ValueState } from '../../components/ValueState';
import TrimmedArt from '../art/TrimmedArt.tsx';
import type { Value } from '../value.ts';
import { formatValueState, valueStateProps } from '../valueFormat.ts';
import type { TableSubjectFigure, TableSubjectSummary } from './model.ts';
import { figureText, type SubjectFigures } from './subjectFigures.ts';

function Text({ value }: { value: Value }) {
  return <ValueState {...valueStateProps(value)} />;
}

function Figure({ figure, text }: { figure: TableSubjectFigure; text: string | null }) {
  const spoken = formatValueState(figure.value).spoken;
  return <div className="tf-subject-figure" data-tooltip={figure.hint}>
    <dt>{figure.label}</dt>
    <dd>{figure.pictures?.length
      ? <>{figure.pictures.map((picture, index) => <span key={index} className="tf-subject-medal" data-tooltip={picture.count ? `${picture.name} ×${picture.count}` : picture.name}>
          {picture.url ? <TrimmedArt src={picture.url} /> : <span className="val-picture-missing" data-tooltip={`${picture.name}: ${picture.missing ?? 'artwork not recorded'}`} />}
          {picture.count ? <span className="tf-subject-times">×{picture.count}</span> : null}
        </span>)}<span className="sr-only">{spoken}</span></>
      : text !== null ? <span data-figure-text="">{text}</span> : <Text value={figure.value} />}</dd>
  </div>;
}

export default function SubjectSummary({ summary, figures, compact }: { summary: TableSubjectSummary; figures: SubjectFigures; compact: boolean }) {
  const list = useRef<HTMLDListElement>(null);
  useLayoutEffect(() => figures.attach(texts => {
    const shown = list.current?.querySelectorAll('[data-figure-text]');
    shown?.forEach((node, index) => {
      const text = texts[index];
      if (text !== undefined && node.textContent !== text) node.textContent = text;
    });
  }), [figures, compact]);
  return <section className="tf-subject" data-compact={compact || undefined} aria-label={formatValueState(summary.name).spoken}>
    <span className="tf-subject-art">{summary.icon ? <TrimmedArt src={summary.icon} /> : <span className="val-picture-missing" data-tooltip="Division artwork not recorded" />}</span>
    <div className="tf-subject-text">
      <strong className="tf-subject-name"><Text value={summary.name} /></strong>
      <div className="tf-subject-line">
        {summary.flag ? <img className="tf-subject-flag" src={summary.flag} alt="" /> : <span className="val-picture-missing is-flag" data-tooltip="Flag not recorded" />}
        <span className="tf-subject-quiet"><Text value={summary.country} /></span>
        <span className="tf-subject-quiet tf-subject-dot" aria-hidden="true">·</span>
        <span className="tf-subject-quiet"><Text value={summary.detail} /></span>
        <span className="tf-subject-level">
          {summary.insignia ? <TrimmedArt src={summary.insignia} /> : summary.level.kind === 'known' ? <span className="val-picture-missing" data-tooltip="Insignia not in your installation" /> : null}
          <Text value={summary.level} />
        </span>
      </div>
    </div>
    {compact ? null : <dl ref={list} className="tf-subject-figures">{summary.figures.map(figure => <Figure key={figure.label} figure={figure} text={figureText(figure)} />)}</dl>}
  </section>;
}
