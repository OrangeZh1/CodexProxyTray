import test from 'node:test';
import assert from 'node:assert/strict';
import {createActions,acceptSnapshot,systemPreferences} from '../ui/actions.mjs';
const deferred=()=>{let resolve,reject;const promise=new Promise((a,b)=>{resolve=a;reject=b});return{promise,resolve,reject}};

test('overlapping actions keep their own indicators and clear independently',async()=>{
  const actions=createActions();const save=deferred(),refresh=deferred();
  const saving=actions.run('save',()=>save.promise);
  assert.equal(actions.has('save'),true);assert.equal(actions.has('refresh'),false);assert.equal(actions.has('launch'),false);
  const refreshing=actions.run('refresh',()=>refresh.promise);
  save.resolve();await saving;assert.equal(actions.has('save'),false);assert.equal(actions.has('refresh'),true);
  refresh.resolve();await refreshing;assert.equal(actions.has('refresh'),false);
});
test('failure and cancellation release only the initiating indicator',async()=>{
  const actions=createActions();const update=deferred();const checking=actions.run('update',()=>update.promise);
  await assert.rejects(actions.run('save',async()=>{throw Error('disk error')}));
  assert.equal(actions.has('save'),false);assert.equal(actions.has('update'),true);
  await assert.rejects(actions.run('refresh',async()=>{throw new DOMException('cancelled','AbortError')}));
  assert.equal(actions.has('refresh'),false);assert.equal(actions.has('update'),true);
  update.resolve();await checking;
});
test('background reloads have no action indicator and stale results cannot undo a save',()=>{
  const actions=createActions();const saved={revision:6,proxy_url:'http://localhost:8888'};
  assert.equal(acceptSnapshot(saved,{revision:5,proxy_url:'http://localhost:7890'}),saved);
  assert.equal(actions.has('refresh'),false);assert.equal(actions.has('save'),false);
});

test('fast actions update data immediately and retain only their own visible animation',async()=>{
  const hold=deferred();let requested=0,updated=false;
  const actions=createActions(()=>{},{minimumVisibleMs:650,now:()=>0,wait:ms=>{requested=ms;return hold.promise}});
  const running=actions.run('refresh',async()=>{updated=true});
  await Promise.resolve();await Promise.resolve();
  assert.equal(updated,true);assert.equal(requested,650);assert.equal(actions.has('refresh'),true);assert.equal(actions.has('save'),false);
  hold.resolve();await running;assert.equal(actions.has('refresh'),false);
});

test('independent preference replies preserve another pending switch',()=>{
  const desired={autostart:true,launchTarget:'cli',cliWorkdir:'',networkEnabled:false,connectivityEnabled:true,autoCheckUpdates:true,updateIntervalHours:12};
  const incoming={autostart:false,launch_target:'cli',network_enabled:false,auto_check_updates:true,update_interval_hours:12};
  assert.deepEqual(systemPreferences(desired,incoming,new Set(['autostart'])),desired);
  assert.deepEqual(systemPreferences(desired,incoming,new Set()),{...desired,autostart:false});
  assert.equal(systemPreferences(desired,{...incoming,network_enabled:true},new Set(['networkEnabled'])).networkEnabled,false);
  assert.equal(systemPreferences(desired,{...incoming,connectivity_enabled:false},new Set(['connectivityEnabled'])).connectivityEnabled,true);
});
