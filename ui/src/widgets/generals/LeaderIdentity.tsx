import ArtFallback from '../../components/ArtFallback';
import { CountryCell } from '../../charts/table/Table';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import type { UnitLeaderPortrait } from './types';
import { useLayoutEffect, useRef, useState } from 'react';
import { compactSkill } from './skillDisplay';
import type { ReactNode } from 'react';
import type { LeaderRole } from '../../generated/dashboard';

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
  }, [level == null, maximum]);
  if (level == null) return <span className="leader-missing">Not recorded</span>;
  if (maximum == null) return <span>{level}</span>;
  return <span ref={element} className="leader-stars" aria-label={`Skill ${level} of ${maximum}`}>
    {display.maximum !== maximum || display.compact ? <span aria-hidden="true">★ {level} / {maximum}</span> : <>
      <span aria-hidden="true">{'★'.repeat(Math.max(0, Math.min(level, maximum)))}</span>
      <em aria-hidden="true">{'★'.repeat(Math.max(0, maximum - Math.max(0, level)))}</em>
    </>}
  </span>;
}

export function LeaderPortrait({ art, name }: { art?: Pick<UnitLeaderPortrait, 'image' | 'missing'> | Error; name?: string }) {
  const portrait = art instanceof Error ? undefined : art?.image;
  const [failed, setFailed] = useState<string>();
  const broken = !!portrait && failed === portrait;
  return <span className="leader-portrait" data-detail={art instanceof Error ? undefined : art?.missing ?? undefined}>
    {portrait && !broken ? <img src={portrait} alt="" onError={() => setFailed(portrait)} /> : <ArtFallback icon="helmet" name={name} failure={art instanceof Error ? art : undefined}
      reason={broken ? 'Portrait could not be drawn.' : art instanceof Error || art?.missing == null ? undefined : 'Portrait unavailable.'} />}
  </span>;
}

export interface Identified { name: string; role: LeaderRole; skill: number | null }

export function leaderIdentityProps(row: Identified, country: ReturnType<CountryLabel>) {
  const rank = row.role.rank ?? `Rank ${row.role.category}`;
  return { name: row.name, rank, hasOwner: !!row.role.owner, level: row.skill, maximum: row.role.maximumSkill,
    countryName: country.name, nameFailure: country.nameFailure, flag: country.flag, flagReason: country.flagReason,
    flagDetail: country.flagDetail, flagPending: country.flagPending };
}

export function leaderPortraitProps(art?: UnitLeaderPortrait | Error) {
  return { image: art instanceof Error ? undefined : art?.image, portraitFailure: art instanceof Error ? art : undefined,
    portraitMissing: art instanceof Error ? undefined : art?.missing };
}

type IdentityProps = ReturnType<typeof leaderIdentityProps> & ReturnType<typeof leaderPortraitProps> & { showCountry?: boolean };

function identityContent({ name, rank, hasOwner, level, maximum, countryName, nameFailure, flag, flagReason, flagDetail, flagPending, showCountry = false }: IdentityProps, portrait: ReactNode) {
  const countryText = showCountry ? `${countryName} · ${rank}` : rank;
  return <span className="leader-identity">{portrait}<span className="leader-text">
    <strong>{name}</strong><small>{hasOwner ? <CountryCell flag={flag} flagReason={flagReason} flagDetail={flagDetail} flagPending={flagPending} name={countryText} nameFailure={nameFailure} /> : countryText}
      {showCountry && <><span> · </span><SkillStars level={level} maximum={maximum} /></>}
    </small></span></span>;
}

export function LeaderIdentity(props: IdentityProps) {
  return identityContent(props, <LeaderPortrait name={props.name} art={props.portraitFailure ?? { image: props.image ?? null, missing: props.portraitMissing ?? null }} />);
}
