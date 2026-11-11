import React from 'react';
import {Button,Select,Switch} from '@fluentui/react-components';
import {Icon} from '@iconify/react';
import icons from '#fluent-icons';

const intervals=[1,3,6,12,24];
const license=`MIT License

Copyright (c) 2026 OrangeZh1

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.`;

function Symbol({name,spin=false}){
  return <Icon icon={icons[name]} className={spin?'symbol spin':'symbol'} aria-hidden="true"/>;
}
function Row({icon,title,description,children}){
  return <div className="setting-row"><Symbol name={icon}/><div className="row-copy"><div className="row-title">{title}</div>{description&&<div className="row-description">{description}</div>}</div><div className="row-control">{children}</div></div>;
}
function checkedTime(value){
  if(!Number.isFinite(value)||!Number.isFinite(new Date(value).getTime()))return '尚未检查';
  return new Date(value).toLocaleString('zh-CN',{timeZone:'Asia/Shanghai',year:'numeric',month:'2-digit',day:'2-digit',hour:'2-digit',minute:'2-digit',second:'2-digit',hour12:false});
}

export function About({status,version,busy=()=>false,writable=false,onPreference,onCheck,onSource,onRelease,onLogs}){
  const networkEnabled=status?.network_enabled!==false;
  const automatic=!!status?.auto_check_updates;
  const interval=intervals.includes(status?.update_interval_hours)?status.update_interval_hours:6;
  const update=status?.update||{status:'unchecked'};
  const manualBusy=busy('check-updates');
  const networkBusy=busy('network-enabled'),automaticBusy=busy('auto-check-updates'),intervalBusy=busy('update-interval');
  const titles={unchecked:'尚未检查更新',current:'当前已是最新版本',available:`发现新版本 ${update.latest||''}`,unpublished:'尚未发布正式版本',error:'检查更新未完成'};
  const title=update.checking?'正在检查更新':Object.hasOwn(titles,update.status)?titles[update.status]:'尚未检查更新';
  const notes=typeof update.notes==='string'?update.notes:'';
  const error=typeof update.error==='string'?update.error:'';
  const action=(id,label,callback)=><Button id={id} className="text-action" appearance="subtle" aria-busy={busy(id)} disabled={typeof callback!=='function'||busy(id)} icon={<Symbol name={busy(id)?'arrow-sync-24-regular':'open-24-regular'} spin={busy(id)}/>} onClick={callback}>{label}</Button>;
  return <div className="about-view">
    <div className="settings-group">
      <Row icon="info-24-regular" title={`版本 ${version}`} description="ChatGPT 桌面端与 Windows Codex CLI 代理助手"><div className="about-links">{action('open-source','GitHub 源码',onSource)}{action('open-release','发布记录',onRelease)}</div></Row>
      <details className="about-license"><summary>MIT 开源许可</summary><pre className="license-text" style={{whiteSpace:'pre-wrap',overflowWrap:'anywhere'}}>{license}</pre></details>
    </div>

    <h2>更新</h2>
    <div className="settings-group">
      <Row icon="plug-connected-24-regular" title="联网检查更新" description="允许助手读取 GitHub 的公开更新清单。"><div className="switch-control">{networkBusy&&<Symbol name="arrow-sync-24-regular" spin/>}<Switch id="network-enabled" aria-label="联网检查更新" aria-busy={networkBusy} checked={networkEnabled} disabled={!writable||typeof onPreference!=='function'||networkBusy} onChange={(_,data)=>onPreference('networkEnabled',data.checked)}/></div></Row>
      <Row icon="arrow-sync-24-regular" title="自动检查更新" description="按设定间隔在后台检查，不自动下载或安装。"><div className="switch-control">{automaticBusy&&<Symbol name="arrow-sync-24-regular" spin/>}<Switch id="auto-check-updates" aria-label="自动检查更新" aria-busy={automaticBusy} checked={automatic} disabled={!writable||typeof onPreference!=='function'||!networkEnabled||automaticBusy} onChange={(_,data)=>onPreference('autoCheckUpdates',data.checked)}/></div></Row>
      <Row icon="clock-24-regular" title="检查间隔" description="可先设置间隔，再开启自动检查。"><div className="switch-control">{intervalBusy&&<Symbol name="arrow-sync-24-regular" spin/>}<Select id="update-interval" aria-label="自动检查更新间隔" aria-busy={intervalBusy} value={String(interval)} icon={<Icon icon={icons['chevron-down-24-regular']} width={16} height={16} aria-hidden="true"/>} disabled={!writable||typeof onPreference!=='function'||!networkEnabled||intervalBusy} onChange={(_,data)=>onPreference('updateIntervalHours',Number(data.value))}>{intervals.map(hours=><option key={hours} value={String(hours)}>{hours} 小时</option>)}</Select></div></Row>
      <Row icon="info-24-regular" title={title} description={<><span>上次检查：{checkedTime(update.checked_at)}</span>{update.checking&&!manualBusy&&<span className="background-check" role="status">后台检查中</span>}</>}><Button id="check-updates" className="text-action" appearance="subtle" aria-busy={manualBusy} disabled={!writable||!networkEnabled||typeof onCheck!=='function'||manualBusy} icon={<Symbol name="arrow-sync-24-regular" spin={manualBusy}/>} onClick={onCheck}>{manualBusy?'正在检查':'检查更新'}</Button></Row>
      {!update.checking&&update.status==='error'&&error&&<div className="release-notes error-text" role="status">{error}</div>}
      {!update.checking&&update.status!=='error'&&notes&&<div className="release-notes" role="status">{notes}</div>}
    </div>

    <h2>本机数据与隐私</h2>
    <div className="settings-group">
      <div className="about-copy">
        <p>配置、启动偏好和最近 200 条分类运行日志保存在本机。</p>
        <p>只采样本地代理端口连接耗时、目标进程树 TCP 连接数与守护进程内存，不记录或上报对话、提示词、用户 CLI 参数。启动参数仅原样转发给已安装的 CLI。</p>
        <p>本应用没有遥测或数据上报。更新请求包含应用名称与版本，不附带本机日志、对话或参数；外部链接由你点击后在浏览器中打开。</p>
        <p>关闭联网检查后，不再发起后续 GitHub 更新检查，已设间隔会保留。目标应用代理和本机 TCP 探测继续运行。</p>
      </div>
      <div className="group-footer"><span>日志保存在本机</span>{action('open-logs','打开日志目录',onLogs)}</div>
    </div>
  </div>;
}
