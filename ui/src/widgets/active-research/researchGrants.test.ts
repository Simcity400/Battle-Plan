import assert from 'node:assert/strict';
import test from 'node:test';
import { researchGrants } from './researchGrants.ts';
test('grant inventory preserves unused grants, duplicate ids, restrictions and empty baselines', () => {
  const grant = {id:-1,ordinal:0,kind:'speed',token:'mod_grant',name:null,uses:3,activeClaims:2,speedBonusRaw:'25000',aheadReductionRaw:'0',costReductionRaw:null,
    technologies:[{token:'mod_technology',name:null,databaseIndex:7},{token:'mod_technology',name:null,databaseIndex:8}],categories:[]};
  const inventory={nextId:-2147483648,grants:[grant,{...grant,ordinal:1,uses:1}],unavailable:null};
  assert.equal(researchGrants(inventory),inventory);
  assert.equal(researchGrants({nextId:0,grants:[],unavailable:null}).grants.length,0);
  assert.throws(()=>researchGrants({...inventory,nextId:2147483648}),/Invalid/);
  assert.throws(()=>researchGrants({...inventory,grants:[{...grant,speedBonusRaw:'9223372036854775808'}]}),/Invalid/);
});
