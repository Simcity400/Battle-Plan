import { useLayoutEffect, useRef } from 'react';
import { CountryFlagUrl } from '../../components/CountryFlag';
import { battleFlagCount, battlePartial, markPartial } from './battleRows';
import type { BattleCountryLabel, BattleRow, BattleSideSummary } from './battleRows';
import type { BattleSide } from '../../generated/dashboard';

const laidOut = (element: HTMLElement) => element.getClientRects().length > 0;

function useFitPainter(element: React.RefObject<HTMLElement | null>, paint: () => void, inputs: readonly unknown[]) {
  const latest = useRef(paint);
  latest.current = paint;
  useLayoutEffect(() => {
    const host = element.current!;
    const measure = () => { if (laidOut(host)) latest.current(); };
    const observer = new ResizeObserver(measure);
    observer.observe(host);
    return () => observer.disconnect();
  }, [element]);
  useLayoutEffect(() => { if (element.current && laidOut(element.current)) latest.current(); }, inputs);
}

function Flags({ side, country }: { side: BattleSide; country: BattleCountryLabel }) {
  const host = useRef<HTMLSpanElement>(null);
  const probe = useRef<HTMLElement>(null);
  const probeMore = useRef<HTMLSpanElement>(null);
  const more = useRef<HTMLSpanElement>(null);
  const count = side.countries.length;
  useFitPainter(host, () => {
    const element = host.current!;
    const gap = parseFloat(getComputedStyle(element).columnGap) || 0;
    const shown = battleFlagCount(count, element.clientWidth, probe.current!.offsetWidth, gap, probeMore.current!.offsetWidth);
    let index = 0;
    for (const flag of element.children as HTMLCollectionOf<HTMLElement>) {
      if (!flag.classList.contains('battle-country')) continue;
      const display = index++ < shown ? '' : 'none';
      if (flag.style.display !== display) flag.style.display = display;
    }
    const tail = more.current!, rest = `+${count - shown}`;
    if (tail.hidden !== shown >= count) tail.hidden = shown >= count;
    if (tail.textContent !== rest) tail.textContent = rest;
  }, [count, side, country]);
  const names = side.countries.map(part => country(part.tag, part.hour).name);
  const title = [side.label, ...names, side.countriesUnknown ? 'Countries unknown' : null].filter(Boolean).join(' · ');
  return <span ref={host} className="battle-flags" title={title} aria-label={title}>
    <span className="battle-probe" aria-hidden="true"><i ref={probe} className="battle-flag-missing" /><span ref={probeMore}>+{count}</span></span>
    {side.countries.map(part => {
      const named = country(part.tag, part.hour);
      return <span className="battle-country" key={part.tag} title={named.name}>
        <CountryFlagUrl className="log-flag" url={named.flag} detail={named.flagDetail} reason={named.flagReason} pending={named.flagPending} fixedSize />
      </span>;
    })}
    <span ref={more} className="battle-flags-more" hidden />
  </span>;
}

export default function BattleSides({ row, country, summary, compact }: {
  row: BattleRow; country: BattleCountryLabel; summary: BattleSideSummary; compact: boolean;
}) {
  const host = useRef<HTMLSpanElement>(null);
  const probe = useRef<HTMLSpanElement>(null);
  const flag = useRef<HTMLElement>(null);
  const fullCount = useRef<HTMLSpanElement>(null);
  const compactCount = useRef<HTMLSpanElement>(null);
  const formatSide = (side: BattleSide, isCompact: boolean) => {
    if (summary.formatSide) return markPartial(summary.formatSide(side, isCompact), side.counts.some(battlePartial));
    const count = side.counts.find(value => value.label === summary.countLabel)
      ?? { label: summary.countLabel, value: null, unknown: summary.unknown };
    return markPartial(summary.formatCount(count, isCompact), battlePartial(count));
  };
  const fullText = row.sides.map(side => formatSide(side, false)).join(` ${summary.separator} `);
  const compactText = row.sides.map(side => formatSide(side, true)).join(` ${summary.separator} `);
  useFitPainter(host, () => {
    if (compact) { compactCount.current!.hidden = false; return; }
    const element = host.current!;
    const gap = parseFloat(getComputedStyle(element).columnGap) || 0;
    const full = probe.current!.offsetWidth + flag.current!.offsetWidth * 2 + gap * 4 <= element.clientWidth;
    if (fullCount.current!.hidden !== !full) fullCount.current!.hidden = !full;
    if (compactCount.current!.hidden !== full) compactCount.current!.hidden = full;
  }, [compact, fullText]);
  return <span ref={host} className="battle-sides-summary" title={fullText}>
    <span className="battle-probe" aria-hidden="true"><span ref={probe}>{fullText}</span><i ref={flag} className="battle-flag-missing" /></span>
    {row.sides[0] && <Flags side={row.sides[0]} country={country} />}
    {!compact && <span ref={fullCount} className="battle-count" hidden>{fullText}</span>}
    <span ref={compactCount} className="battle-count">{compactText}</span>
    {row.sides.slice(1).map((side, index) => <Flags key={index} side={side} country={country} />)}
  </span>;
}
