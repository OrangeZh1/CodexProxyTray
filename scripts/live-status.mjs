import { createUpdateCheck } from './update-check.mjs';
import { setGlobalProxyFromEnv } from 'node:http';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { readFile, access } from 'node:fs/promises';
import { join } from 'node:path';
import { createConnection } from 'node:net';
const exec=promisify(execFile);
const root=join(process.env.LOCALAPPDATA,'CodexProxyTray');
let revision=0, cached, checked=0, pendingStatus, pendingUpdate;
const socketCheck=(host,port)=>new Promise(resolve=>{const socket=createConnection({host,port});const finish=value=>{socket.destroy();resolve(value)};socket.setTimeout(600,()=>finish(false));socket.once('connect',()=>finish(true));socket.once('error',()=>finish(false))});

async function nativeStatus(){
  try{
    const native=JSON.parse(await readFile(join(root,'status.json'),'utf8'));
    const age=Date.now()-native.updated_at;
    if(!Number.isInteger(native.helper_pid)||native.helper_pid<=0||!Number.isFinite(age)||age< -2000||age>=10000||!Array.isArray(native.logs)||typeof native.backend_ready!=='boolean')return null;
    process.kill(native.helper_pid,0);
    return {...native,read_only:true};
  }catch{return null;}
}

async function queryOsStatus(){
  let config={proxy_url:'http://127.0.0.1:7890',launch_desktop:false};
  let configError=null;
  try{
    const saved=JSON.parse(await readFile(join(root,'config.json'),'utf8'));
    if(!saved||typeof saved!=='object'||Array.isArray(saved))throw Error('Invalid configuration');
    config={...config,...saved};
    if(typeof config.proxy_url!=='string'||typeof config.launch_desktop!=='boolean')throw Error('Invalid configuration');
  }catch(error){if(error.code!=='ENOENT')configError='代理配置无法读取。';}
  const command=`$p=Get-AppxPackage -Name OpenAI.Codex | Select-Object -First 1; $r=@(Get-CimInstance Win32_Process -Filter "Name='ChatGPT.exe'" -ErrorAction SilentlyContinue | Where-Object {$_.ExecutablePath -like '*OpenAI.Codex_*' -and $_.CommandLine -notmatch ' --type='}).Count -gt 0; $a=(Get-ItemProperty -LiteralPath 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Run' -Name CodexProxyTray -ErrorAction SilentlyContinue).CodexProxyTray; [pscustomobject]@{package_count=$(if($p){1}else{0});package_version=$(if($p){[string]$p.Version}else{$null});codex_running=$r;autostart=[bool]$a} | ConvertTo-Json -Compress`;
  const {stdout}=await exec('C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe',['-NoLogo','-NoProfile','-NonInteractive','-Command',command],{windowsHide:true,timeout:8000,maxBuffer:65536});
  const desktop=JSON.parse(stdout.replace(/^\uFEFF/,''));
  const endpoint=typeof config.proxy_url==='string'?/^http:\/\/(127\.0\.0\.1|localhost|\[::1\]):(\d{1,5})\/?$/.exec(config.proxy_url.trim()):null;
  const validEndpoint=endpoint&&Number(endpoint[2])>0&&Number(endpoint[2])<=65535;
  if(!validEndpoint)configError='代理配置无效。';
  const proxy_reachable=!configError&&validEndpoint?await socketCheck(endpoint[1].replace(/^\[|\]$/g,''),Number(endpoint[2])):false;
  let cli_detected=false;try{await access(join(process.env.APPDATA,'npm/node_modules/@openai/codex/bin/codex.js'));cli_detected=true}catch{}
  cached={...desktop,proxy_url:configError?'配置无效':config.proxy_url,proxy_reachable:!!proxy_reachable,enabled:false,guard_ready:false,restart_required:false,cli_detected,cli_ready:false,launch_desktop:!!config.launch_desktop,backend_ready:false,read_only:true,error:configError,logs:[{at:Date.now(),level:configError?'error':'info',message:configError||`实时检测：ChatGPT ${desktop.codex_running?'正在运行':'尚未运行'}，本地代理${proxy_reachable?'可以连接':'暂不可连接'}。`}]};
  checked=Date.now();return cached;
}

export async function liveStatus(force=false){
  let data=await nativeStatus();
  if(!data){
    if(cached&&!force&&Date.now()-checked<4000)data=cached;
    else{
      // All preview clients share one probe; a manual refresh joins an in-flight read.
      pendingStatus??=queryOsStatus().finally(()=>{pendingStatus=undefined;});
      data=await pendingStatus;
    }
    // The native worker may have started while the slower OS probe was running.
    data=await nativeStatus()||data;
  }
  return {...data,read_only:true,revision:++revision};
}

export const checkUpdates = createUpdateCheck(async (url, options) => {
  // Only this development server's HTTP client changes; no OS or process proxy variables are written.
  let proxy = 'http://127.0.0.1:7890';
  try { proxy = JSON.parse(await readFile(join(root,'config.json'),'utf8')).proxy_url; } catch {}
  if (!/^http:\/\/(127\.0\.0\.1|localhost|\[::1\]):\d{1,5}\/?$/.test(proxy)) throw Error('代理设置无效。');
  setGlobalProxyFromEnv({HTTP_PROXY:proxy,HTTPS_PROXY:proxy,NO_PROXY:'localhost,127.0.0.1,::1'});
  return fetch(url,options);
});
