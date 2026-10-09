import assert from 'node:assert/strict';
import test from 'node:test';
import { canvasPage } from '../../../tools/canvas-page.ts';

test('share Canvas pixels retain a subpixel spike, a fractional gap and a wholly empty gap', async () => {
  const { result } = await canvasPage(`
import { shareStack, shareLayer } from '/charts/share.ts';
import { plotFrame } from '/charts/frame.ts';
import { tokensFromValues } from '/charts/tokens.ts';
try {
  const ctx = document.querySelector('canvas').getContext('2d');
  const runs = Array.from({length:2000},(_,hour)=>({firstHour:hour,lastHour:hour,counts:hour===71?[100,0,0]:[0,0,100]}))
    .filter(run=>run.firstHour!==1001&&(run.firstHour<500||run.firstHour>540));
  const looks=['#ff0000','#ffbf00','#00ff00'].map((color,i)=>({id:String(i),label:String(i),color,alpha:i===2?.85:1}));
  const frame=plotFrame(200,100,{left:0,right:0,top:0,bottom:0},[0,1999],[0,100],false);
  const layer=shareLayer(shareStack(runs,3),looks,[0,1999]);
  const tokens=tokensFromValues(()=> 'theme','font');
  layer.draw(ctx,frame,tokens,{playhead:null});
  const around=hour=>Array.from(ctx.getImageData(Math.floor(frame.toX(hour))-1,50,4,1).data);
  const recorded={spike:around(71).some((value,i)=>i%4===0&&value>0),
    gap:around(1001).some((value,i)=>i%4===3&&value<210),
    empty:ctx.getImageData(Math.floor(frame.toX(520)),50,1,1).data[3]};
  ctx.clearRect(0,0,840,360);layer.draw(ctx,frame,tokens,{playhead:70});
  const hasRed=()=>Array.from(ctx.getImageData(0,0,200,100).data).some((value,i)=>i%4===0&&value>0);
  const future=hasRed();
  ctx.clearRect(0,0,840,360);layer.draw(ctx,frame,tokens,{playhead:72});
  finish({...recorded,future,revealed:hasRed()});
}catch(error){finish({error:String(error.stack||error)});}
`);
  assert.equal(result.spike, true);
  assert.equal(result.gap, true);
  assert.equal(result.empty, 0);
  assert.equal(result.future, false, 'the future spike cannot colour a cached pixel before the playhead');
  assert.equal(result.revealed, true);
});
