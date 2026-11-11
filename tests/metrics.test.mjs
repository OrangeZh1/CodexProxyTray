import {test} from 'node:test';
import assert from 'node:assert/strict';
import {duration,probePath} from '../ui/metrics.mjs';
test('chart uses measured timestamps and leaves gaps for failed probes',()=>{
  const path=probePath([{at:100,latency_ms:1},{at:200,latency_ms:null},{at:300,latency_ms:2},{at:500,latency_ms:1}]);
  assert.equal((path.match(/M/g)||[]).length,2);assert.equal((path.match(/L/g)||[]).length,1);assert.match(path,/L600\.0,/);
  assert.equal(probePath([{at:100,latency_ms:null}]),'');assert.equal(probePath([]),'');
  assert.equal(duration(3661000),'01:01:01');assert.equal(duration(undefined),'—');
});
