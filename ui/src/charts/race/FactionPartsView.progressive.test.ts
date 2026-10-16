import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { plotFrame } from '../frame.ts';
import type { ChartLayer } from '../TimeChart';
import type { ChartSummary } from '../chartSummary';
import type { ChartTokens } from '../tokens';

test('faction member bands use the shared approximate union and drawing style', async () => {
  const state = { layers: [] as ChartLayer[] };
  const host = globalThis as typeof globalThis & { __progressiveFactionBands?: typeof state };
  host.__progressiveFactionBands = state;
  const replacements: Record<string,string> = {
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/dashboard/countryNamesAt.ts': 'export const useCountryFactsRead = () => new Map();',
    '/dashboard/useCountriesAt.ts': 'export const STYLES = 0;',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({labelAt: () => tag => ({name: tag})});',
    '/dashboard/playback.ts': 'export const usePlaybackSpeed = () => 0;',
    '/charts/useChartTokens.ts': "export const useChartTokens = () => ({muted:'#888',surface:'#000',countryColours:'fixed'});",
    '/charts/race/factionPartColours.ts': `export const factionColourPlan = () => ({wanted:[],selection:new Map(),maxHours:0});
      export const factionColourKey = () => null; export const factionCountryColours = () => new Map(); export const heldFactionColours = () => new Map();`,
    '/charts/TimeChart.tsx': 'export default props => {globalThis.__progressiveFactionBands.layers = props.layers; return null;};',
    '/charts/drill/DrillParts.tsx': 'export default () => null;',
  };
  const vite = await createServer({ configFile:false, optimizeDeps:{noDiscovery:true}, server:{middlewareMode:true,watch:null,hmr:false,ws:false},
    plugins:[{name:'progressive-band-host',enforce:'pre',transform(_source,id) {
      return Object.entries(replacements).find(([suffix]) => id.replaceAll('\\','/').endsWith(suffix))?.[1];
    }}] });
  try {
    const { default: View } = await vite.ssrLoadModule('/ui/src/charts/race/FactionPartsView.tsx');
    const bucket = (firstHour:number,lastHour:number) => ({firstHour,lastHour,firstValue:2,lastValue:2,lastValueHour:firstHour,
      minHour:firstHour,minValue:2,maxHour:firstHour,maxValue:2,firstBreak:false,hasBreak:false});
    const total:ChartSummary = {from:0,to:31,width:64,buckets:[bucket(0,15),bucket(16,31)]};
    const member:ChartSummary = {...total,approximate:[{firstHour:16,lastHour:31}]};
    renderToStaticMarkup(createElement(View, {model:{parts:[{id:'A',runs:[{firstHour:0,lastHour:31,raw:2n,value:2}],
      counted:[{firstHour:0,lastHour:31}],witnesses:[{firstHour:0,lastHour:31,tag:'A'}]}],census:[],total:[{firstHour:0,lastHour:31,value:2}],remainder:[],unidentified:0},
      runtime:{client:{},clock:{}},track:{source:{id:'source'},branch:{id:'1'}},hour:5,following:false,extent:[0,31],active:true,
      pickOwner:{},onPick(){},label:'Faction',formatValue:String,totalSummary:total,summaries:{A:member}}));
    const strokes:{alpha:number;dash:number[]}[] = [];
    let alpha=1,dash:number[]=[];
    const saved:{alpha:number;dash:number[]}[]=[];
    const context={save(){saved.push({alpha,dash});},restore(){({alpha,dash}=saved.pop()!);},
      get globalAlpha(){return alpha;},set globalAlpha(value:number){alpha=value;},
      setLineDash(value:number[]){dash=value;},beginPath(){},rect(){},clip(){},moveTo(){},lineTo(){},closePath(){},fill(){},
      stroke(){strokes.push({alpha,dash});}} as unknown as CanvasRenderingContext2D;
    state.layers[0].draw(context,plotFrame(64,160,{left:0,right:0,top:0,bottom:0},[0,31],[0,4],false),
      {surface:'#000',lineWidth:{series:2}} as ChartTokens,{playhead:null} as Parameters<ChartLayer['draw']>[3]);
    assert.ok(strokes.some(stroke=>stroke.alpha===1 && stroke.dash.length===0));
    assert.ok(strokes.some(stroke=>stroke.alpha===0.55 && stroke.dash.join(',')==='4,4'), 'a member-only rough span reaches the real band renderer');
    assert.equal(alpha,1);assert.deepEqual(dash,[]);
  } finally { await vite.close(); delete host.__progressiveFactionBands; }
});
