import { useLayoutEffect, useRef, useState } from 'react';
import { CountryFlagUrl } from '../../components/CountryFlag';
import type { BattleSide } from '../../generated/dashboard';
import { battleFlagCount } from './battleRows';
import type { BattleCountryLabel, BattleRow, BattleSideSummary } from './battleRows';

function Flags({ side, country }: { side: BattleSide; country: BattleCountryLabel }) {
  const host = useRef<HTMLSpanElement>(null);
  const probe = useRef<HTMLElement>(null);
  const more = useRef<HTMLSpanElement>(null);
  const [shown, setShown] = useState(0);
  const count = side.countries.length;
  useLayoutEffect(() => {
    const element = host.current!;
    const measure = () => {
      const gap = parseFloat(getComputedStyle(element).columnGap) || 0;
      const width = element.clientWidth;
      setShown(battleFlagCount(count, width, probe.current!.offsetWidth, gap, more.current!.offsetWidth));
    };
    measure();
    const observer = new ResizeObserver(measure);
    [element, probe.current!, more.current!].forEach(item => observer.observe(item));
    return () => observer.disconnect();
  }, [count]);
  const names = side.countries.map(part => country(part.tag, part.hour).name);
  const title = [side.label, ...names, side.countriesUnknown ? 'Countries unknown' : null].filter(Boolean).join(' · ');
  return <span ref={host} className="battle-flags" title={title} aria-label={title}>
    <span className="battle-probe" aria-hidden="true"><i ref={probe} className="battle-flag-missing" /><span ref={more}>+{count}</span></span>
    {side.countries.slice(0, shown).map(part => {
      const named = country(part.tag, part.hour);
      return <span className="battle-country" key={JSON.stringify([part.tag, part.hour])} title={named.name}>
        <CountryFlagUrl className="log-flag" url={named.flag} detail={named.flagDetail} reason={named.flagReason} pending={named.flagPending} fixedSize />
      </span>;
    })}
    {shown < count && <span className="battle-flags-more">+{count - shown}</span>}
  </span>;
}

/** A measured flag/count pair; labels and unit wording are supplied by the caller. */
export default function BattleSides({ row, country, summary, compact }: {
  row: BattleRow; country: BattleCountryLabel; summary: BattleSideSummary; compact: boolean;
}) {
  const host = useRef<HTMLSpanElement>(null);
  const probe = useRef<HTMLSpanElement>(null);
  const flag = useRef<HTMLElement>(null);
  const [full, setFull] = useState(false);
  const counts = row.sides.map(side => side.counts.find(value => value.label === summary.countLabel)
    ?? { label: summary.countLabel, value: null, unknown: summary.unknown });
  const fullText = counts.map(value => summary.formatCount(value, false)).join(` ${summary.separator} `);
  useLayoutEffect(() => {
    if (compact) return;
    const element = host.current!;
    const measure = () => {
      const gap = parseFloat(getComputedStyle(element).columnGap) || 0;
      setFull(probe.current!.offsetWidth + flag.current!.offsetWidth * 2 + gap * 4 <= element.clientWidth);
    };
    measure();
    const observer = new ResizeObserver(measure);
    [element, probe.current!, flag.current!].forEach(item => observer.observe(item));
    return () => observer.disconnect();
  }, [compact, fullText]);
  return <span ref={host} className="battle-sides-summary" title={fullText}>
    <span className="battle-probe" aria-hidden="true"><span ref={probe}>{fullText}</span><i ref={flag} className="battle-flag-missing" /></span>
    {row.sides[0] && <Flags side={row.sides[0]} country={country} />}
    <span className="battle-count">{!compact && full ? fullText : counts.map(value => summary.formatCount(value, true)).join(` ${summary.separator} `)}</span>
    {row.sides.slice(1).map((side, index) => <Flags key={index} side={side} country={country} />)}
  </span>;
}
