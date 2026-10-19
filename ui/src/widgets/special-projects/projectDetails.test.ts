import assert from 'node:assert/strict';
import test from 'node:test';
import { validateProjectDetails } from './projectDetailsData.ts';
const person = {assigned:true,id:7,name:'Scientist',portrait:null,missing:null};
const label = {token:'mod_project',name:'Project'};
test('project details preserve duplicate traits, supporting assignments, pending choices and exact variables', () => {
  const reply = {hour:24,breakthroughs:[{specialization:label,points:-12345,readHour:24}],scientists:[{character:7,person,active:253,injuryDays:-1,injuryHour:24,visible:true,visibilityHour:24,stateRefusals:{},
    skills:[{specialization:label,level:-3,experienceRaw:'9223372036854775807'}],traits:[label,label]}],
    programs:[{id:3,province:9,readHour:24,facility:{speedRaw:'87654',unavailable:null,sources:[{kind:'Scientist trait',unit:'factor',label,modifier:label,assignment:1,scientistReference:'30064771145',valueRaw:'123456'}]},assignments:[{ordinal:0,reference:'30064771145',person,target:5,current:-2,timed:1},{ordinal:1,reference:'30064771145',person,target:5,current:2,timed:0}]}],
    projects:[{owner:5,id:1,label,readHour:24,pendingKnown:true,pending:{label,description:null,options:[{label,effects:{text:'Output',refusals:{}}}],unavailable:null},
      choices:[{ordinal:0,date:23,reward:label,option:label,province:-1,progress:12345,characterReference:'18446744073709551615',country:5,scientist:7,state:null}],seed:4294967295,variables:[{name:'mod_array^0',valueRaw:'-9223372036854775808'}],output:{text:'Effect',refusals:{}}}],unavailable:[]};
  assert.equal(validateProjectDetails(reply),reply);
  assert.throws(() => validateProjectDetails({...reply,programs:[{...reply.programs[0],facility:{...reply.programs[0].facility,speedRaw:'9223372036854775808'}}]}),/Invalid/);
  assert.throws(() => validateProjectDetails({...reply,projects:[{...reply.projects[0],choices:[{...reply.projects[0].choices[0],characterReference:'18446744073709551616'}]}]}),/Invalid/);
  assert.throws(() => validateProjectDetails({...reply,breakthroughs:[{...reply.breakthroughs[0],readHour:25}]}),/Invalid/);
  assert.throws(() => validateProjectDetails({...reply,scientists:[{...reply.scientists[0],visibilityHour:25}]}),/Invalid/);
  assert.throws(() => validateProjectDetails({...reply,programs:[{...reply.programs[0],readHour:25}]}),/Invalid/);
  assert.throws(() => validateProjectDetails({...reply,projects:[{...reply.projects[0],pendingKnown:false}]}),/Invalid/);
  assert.throws(() => validateProjectDetails({...reply,scientists:[{...reply.scientists[0],skills:[{specialization:label,level:1,experienceRaw:'9223372036854775808'}]}]}),/Invalid/);
});
