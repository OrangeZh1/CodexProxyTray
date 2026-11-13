import test from 'node:test';
import assert from 'node:assert/strict';
import {build} from 'esbuild';
import {createRequire} from 'node:module';
import {runInNewContext} from 'node:vm';
import React from 'react';
import {renderToStaticMarkup} from 'react-dom/server';
import {SSRProvider} from '@fluentui/react-components';
import {root,iconsPlugin} from '../scripts/build-ui.mjs';
import {createActions} from '../ui/actions.mjs';

// Render the real JSX in memory; no generated test bundle is left in the tree.
const bundled=await build({absWorkingDir:root,stdin:{contents:"export {CliManagement} from './ui/management.jsx'; export {UpdateSettings} from './ui/about.jsx';",resolveDir:root,sourcefile:'management-test-entry.mjs',loader:'js'},bundle:true,write:false,platform:'node',format:'cjs',target:'node24',packages:'external',jsx:'automatic',loader:{'.svg':'text'},plugins:[iconsPlugin]});
const module={exports:{}};
runInNewContext(bundled.outputFiles[0].text,{module,exports:module.exports,require:createRequire(import.meta.url),console,process});
const {CliManagement,UpdateSettings}=module.exports;
const managed={source:'npm_user_global',supported:true,target_token:'fixture-token',current:'0.168.0',latest:'0.169.0',checked_at:1,operation:null};
const render=(data=managed,busy=()=>false)=>renderToStaticMarkup(React.createElement(SSRProvider,null,React.createElement(CliManagement,{data,writable:true,networkEnabled:true,busy})));
const control=(html,id)=>html.match(new RegExp(`<button\\b[^>]*\\bid="${id}"[^>]*>`))?.[0]||'';

test('CLI management shows the latest number once without repeating the current version',()=>{
  const html=render({...managed,current:'0.169.0'});
  assert.match(html,/<small>最新版本<\/small><strong>0\.169\.0<\/strong>/);
  assert.equal((html.match(/0\.169\.0/g)||[]).length,1);
  assert.doesNotMatch(html,/<small>当前版本|cli-version-arrow|→/);
  assert.equal((html.match(/已是最新/g)||[]).length,1);
  const newer=render();assert.doesNotMatch(newer,/0\.168\.0/);assert.match(newer,/可更新/);assert.match(newer,/id="cli-update"/);
});

test('unchecked and failed checks never substitute the installed version for the latest',()=>{
  const unchecked=render({...managed,latest:null});
  assert.match(unchecked,/<small>最新版本<\/small><strong>尚未检查<\/strong>/);
  assert.equal((unchecked.match(/尚未检查/g)||[]).length,1);assert.doesNotMatch(unchecked,/0\.168\.0|已是最新|cli-version-result/);
  const failed=render({...managed,error:'fixture connection failure'});
  assert.match(failed,/<small>上次查到版本<\/small><strong>0\.169\.0<\/strong>/);assert.match(failed,/检查未完成/);assert.doesNotMatch(failed,/已是最新|>可更新</);assert.match(control(failed,'cli-update'),/\bdisabled(?:=|\s|>)/);
  const unknown=render({...managed,latest:null,error:'fixture connection failure'});
  assert.match(unknown,/<small>最新版本<\/small><strong>尚未检查<\/strong>/);assert.doesNotMatch(unknown,/上次查到|0\.168\.0/);
  const stale=render({...managed,checked_at:null});assert.match(stale,/上次查到版本/);assert.match(stale,/请检查更新/);assert.match(control(stale,'cli-update'),/\bdisabled(?:=|\s|>)/);
});

test('a delayed check owns one status and only its initiating button spins until success or rejection',async()=>{
  const actions=createActions();
  for(const outcome of ['success','failure','cancel']){
    let resolve,reject;const delayed=new Promise((a,b)=>{resolve=a;reject=b});
    const pending=actions.run('cli-check',()=>delayed);
    const html=render({...managed,operation:'check',progress:'正在检查 Codex CLI 最新版本。'},name=>actions.has(name));
    assert.equal((html.match(/正在检查/g)||[]).length,1);assert.doesNotMatch(html,/operation-progress/);
    assert.match(control(html,'cli-check'),/aria-busy="true"/);
    for(const id of ['cli-update','cli-uninstall'])assert.match(control(html,id),/aria-busy="false"/);
    if(outcome==='success'){resolve();await pending;}else{reject(outcome==='cancel'?new DOMException('fixture cancelled','AbortError'):Error('fixture failed'));await assert.rejects(pending);}
    const settled=render(managed,name=>actions.has(name));
    assert.match(control(settled,'cli-check'),/aria-busy="false"/);assert.doesNotMatch(settled,/正在检查/);
  }
  const background=render({...managed,operation:'check'});assert.match(background,/正在检查/);assert.match(control(background,'cli-check'),/aria-busy="false"/);
  const updating=render({...managed,operation:'update',progress:'fixture update progress'},name=>name==='cli-update');
  assert.match(updating,/operation-progress[^>]*>fixture update progress/);assert.match(control(updating,'cli-update'),/aria-busy="true"/);assert.match(control(updating,'cli-check'),/aria-busy="false"/);
});

test('assistant updates keep only the latest version and a permanent notes action in the same footer',()=>{
  const noop=()=>{},view=(update,busy=()=>false)=>renderToStaticMarkup(React.createElement(SSRProvider,null,React.createElement(UpdateSettings,{status:{network_enabled:true,update},version:'0.2.4',writable:true,busy,onPreference:noop,onCheck:noop,onDownload:noop,onRelease:noop})));
  const footer=html=>html.slice(html.indexOf('<div class="group-footer update-version-comparison"'),html.indexOf('</div></div>',html.indexOf('<div class="group-footer update-version-comparison"'))+12);
  for(const update of [{status:'unchecked'},{status:'current',checking:true,latest:'0.3.0'},{status:'current',latest:'0.3.0',notes:'### fixture release body'},{status:'available',latest:'0.3.0',notes:'### fixture release body'},{status:'error',latest:'0.3.0',error:'fixture failure'}]){
    const html=view(update),line=footer(html);
    assert.match(line,/查看更新说明/);assert.equal((line.match(/查看更新说明/g)||[]).length,1);assert.match(line,/update-footer-actions/);
    assert.doesNotMatch(html,/当前版本|→|update-notes-entry|0\.2\.4|fixture release body/);
    if(update.latest)assert.equal((line.match(/0\.3\.0/g)||[]).length,1);else assert.match(line,/最新版本 <strong>尚未检查/);
  }
  const checking=view({status:'current',checking:true,latest:'0.3.0'},name=>name==='check-updates');
  assert.match(control(checking,'check-updates'),/aria-busy="true"/);assert.match(control(checking,'open-release'),/aria-busy="false"/);
  for(const id of ['network-enabled','auto-check-updates','update-interval'])assert.match(checking.match(new RegExp(`<[^>]+\\bid="${id}"[^>]*>`))?.[0]||'',/aria-busy="false"/);
  const saving=view({status:'unchecked'},name=>name==='update-interval');assert.match(control(saving,'check-updates'),/aria-busy="false"/);assert.match(control(saving,'update-interval'),/aria-busy="true"/);
});
