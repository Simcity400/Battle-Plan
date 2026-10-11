import ArtFallback from '../../components/ArtFallback';
import { CountryCell } from '../../charts/table/Table';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import type { LeaderRow } from './generalsData';
import { leaderName } from './generalsData';
import type { UnitLeaderPortrait } from '../../generated/dashboard';
import { useLayoutEffect, useRef, useState } from 'react';
import { compactSkill } from './skillDisplay';

export function SkillStars({ level, maximum }: { level: number | null; maximum: number | null }) {
  const element = useRef<HTMLSpanElement>(null);
  const [display, setDisplay] = useState({ maximum: null as number | null, compact: true });
  useLayoutEffect(() => {
    const stars = element.current;
    if (!stars || maximum == null) return;
    const measure = () => {
      const style = getComputedStyle(stars);
      const context = document.createElement('canvas').getContext('2d');
      if (!context) return;
      context.font = style.font;
      const starWidth = context.measureText('★').width + (parseFloat(style.letterSpacing) || 0);
      setDisplay({ maximum, compact: compactSkill(maximum, stars.clientWidth, starWidth) });
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(stars);
    return () => observer.disconnect();
  }, [level, maximum]);
  if (level == null) return <span className="leader-missing">Not recorded</span>;
  if (maximum == null) return <span>{level}</span>;
  return <span ref={element} className="leader-stars" aria-label={`Skill ${level} of ${maximum}`}>
    {display.maximum !== maximum || display.compact ? <span aria-hidden="true">★ {level} / {maximum}</span> : <>
      <span aria-hidden="true">{'★'.repeat(Math.max(0, Math.min(level, maximum)))}</span>
      <em aria-hidden="true">{'★'.repeat(Math.max(0, maximum - Math.max(0, level)))}</em>
    </>}
  </span>;
}

export function LeaderPortrait({ art }: { art?: UnitLeaderPortrait }) {
  const portrait = art?.image;
  return <span className="leader-portrait">
    {portrait ? <img src={portrait} alt="" /> : <ArtFallback icon="helmet" />}
  </span>;
}

export function LeaderIdentity({ art, row, country, showCountry = false }: { art?: UnitLeaderPortrait; row: LeaderRow; country: ReturnType<CountryLabel>; showCountry?: boolean }) {
  const rank = row.value.rank || `Rank ${row.value.category}`;
  const countryText = showCountry ? `${country.name} · ${rank}` : rank;
  return <span className="leader-identity"><LeaderPortrait art={art} /><span className="leader-text">
    <strong>{leaderName(row)}</strong><small>{row.value.owner ? <CountryCell {...country} name={countryText} /> : countryText}
      {showCountry && <><span> · </span><SkillStars level={row.value.skill} maximum={row.value.maximumSkill} /></>}
    </small></span></span>;
}
