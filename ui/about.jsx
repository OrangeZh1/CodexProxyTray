import React from 'react';
import {Button,Switch,Dialog,DialogTrigger,DialogSurface,DialogBody,DialogTitle,DialogContent,DialogActions} from '@fluentui/react-components';
import {Icon} from '@iconify/react';
import icons from '#fluent-icons';
import {BrandIcon} from './brand-icons.jsx';
import {Choice} from './choice.jsx';

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
  if(name==='github')return <BrandIcon name="github"/>;
  return <Icon icon={icons[name]} className={spin?'symbol spin':'symbol'} aria-hidden="true"/>;
}
function Row({icon,title,description,children}){
  return <div className="setting-row"><Symbol name={icon}/><div className="row-copy"><div className="row-title">{title}</div>{description&&<div className="row-description">{description}</div>}</div><div className="row-control">{children}</div></div>;
}
function checkedTime(value){
  if(!Number.isFinite(value)||!Number.isFinite(new Date(value).getTime()))return '尚未检查';
  return new Date(value).toLocaleString('zh-CN',{timeZone:'Asia/Shanghai',year:'numeric',month:'2-digit',day:'2-digit',hour:'2-digit',minute:'2-digit',second:'2-digit',hour12:false});
}
export function LicenseText(){return <pre className="license-text">{license}</pre>;}
function LicenseDialog(){
  return <Dialog><DialogTrigger disableButtonEnhancement><Button className="license-link" appearance="transparent" aria-haspopup="dialog">MIT 许可</Button></DialogTrigger><DialogSurface className="license-dialog"><DialogBody><DialogTitle>MIT 开源许可</DialogTitle><DialogContent className="license-content"><LicenseText/></DialogContent><DialogActions><DialogTrigger disableButtonEnhancement><Button appearance="subtle">关闭</Button></DialogTrigger></DialogActions></DialogBody></DialogSurface></Dialog>;
}

export function ReleaseNotes({notes}){
  const inline=text=>text.split(/(\*\*[^*]+\*\*|`[^`]+`)/g).map((part,index)=>part.startsWith('**')?<strong key={index}>{part.slice(2,-2)}</strong>:part.startsWith('`')?<code key={index}>{part.slice(1,-1)}</code>:part);
  return <div className="release-content">{notes.split(/\r?\n/).map((line,index)=>/^#{1,6}\s/.test(line)?<h3 key={index}>{inline(line.replace(/^#{1,6}\s+/,''))}</h3>:/^\s*[-*]\s/.test(line)?<div className="release-item" key={index}>{inline(line.replace(/^\s*[-*]\s+/,''))}</div>:line.trim()?<p key={index}>{inline(line)}</p>:null)}</div>;
}

export function About({version,busy=()=>false,onSource,onRelease,onLogs,onPrivacy,onDisclaimer}){
  const action=(id,label,callback)=><Button id={id} className="text-action" appearance="subtle" aria-busy={busy(id)} disabled={typeof callback!=='function'||busy(id)} icon={<Symbol name={busy(id)?'arrow-sync-24-regular':id==='open-source'?'github':'open-24-regular'} spin={busy(id)}/>} onClick={callback}>{label}</Button>;
  return <div className="about-view">
    <header className="about-identity"><img src="icon.svg" alt=""/><h2>CodexProxyTray</h2><div className="about-version"><span>版本 {version} · Windows x64</span><LicenseDialog/>{action('open-source','GitHub 源码',onSource)}</div><p>为 ChatGPT 与 Codex CLI 单独接入本地代理。</p></header>
    <section className="product-intro" aria-label="产品介绍"><h2>只为 ChatGPT 和 Codex 连接代理</h2><p>接入已有的本地 HTTP / mixed 端口，省去反复配置启动环境。无需为了这两个应用开启 TUN 或系统全局代理。</p><ul><li><strong><Symbol name="plug-connected-24-regular"/>独立代理</strong><span>为目标进程传入代理，不修改系统代理和全局环境变量。</span></li><li><strong><Symbol name="shield-checkmark-24-regular"/>持续守护</strong><span>保留 Windows 应用包身份，跟踪升级，提供 CLI 专用入口。</span></li><li><strong><Symbol name="pulse-24-regular"/>连接诊断</strong><span>集中查看连接状态、目标流量与日志，帮助定位启动和代理问题。</span></li></ul><p className="product-boundary">需自备可用代理，不提供节点。社区开源工具，与 OpenAI 无隶属关系。</p></section>

    <PrivacyInfo busy={busy} onLogs={onLogs} onRead={onPrivacy}/><div className="settings-group about-disclaimer"><button type="button" className="about-detail-entry" onClick={onDisclaimer}><Symbol name="document-error-24-regular"/>免责声明<Symbol name="chevron-right-20-regular"/></button></div>
  </div>;
}

export function UpdateSettings({status,busy=()=>false,writable=false,onPreference,onCheck}){
  const networkEnabled=status?.network_enabled!==false;
  const automatic=!!status?.auto_check_updates;
  const interval=intervals.includes(status?.update_interval_hours)?status.update_interval_hours:6;
  const update=status?.update||{status:'unchecked'};
  const manualBusy=busy('check-updates');
  const networkBusy=busy('network-enabled'),automaticBusy=busy('auto-check-updates'),intervalBusy=busy('update-interval');
  const titles={unchecked:'尚未检查更新',current:'当前已是最新版本',available:`发现新版本 ${update.latest||''}`,unpublished:'暂无公开发布版本',error:'检查更新未完成'};
  const title=update.checking?'正在检查更新':Object.hasOwn(titles,update.status)?titles[update.status]:'尚未检查更新';
  const notes=typeof update.notes==='string'?update.notes:'';
  const error=typeof update.error==='string'?update.error:'';
  const action=(id,label,callback)=><Button id={id} className="text-action" appearance="subtle" aria-busy={busy(id)} disabled={typeof callback!=='function'||busy(id)} icon={<Symbol name={busy(id)?'arrow-sync-24-regular':id==='open-source'?'github':'open-24-regular'} spin={busy(id)}/>} onClick={callback}>{label}</Button>;
  return <>

    <div className="settings-group">
      <Row icon="plug-connected-24-regular" title="允许联网检测" description="用于版本检查、网页探测和模型价格同步。"><div className="switch-control">{networkBusy&&<Symbol name="arrow-sync-24-regular" spin/>}<Switch id="network-enabled" aria-label="允许联网检测" aria-busy={networkBusy} checked={networkEnabled} disabled={!writable||typeof onPreference!=='function'||networkBusy} onChange={(_,data)=>onPreference('networkEnabled',data.checked)}/></div></Row>
      <Row icon="arrow-sync-24-regular" title="自动检查更新" description="按设定间隔在后台检查，不自动下载或安装。"><div className="switch-control">{automaticBusy&&<Symbol name="arrow-sync-24-regular" spin/>}<Switch id="auto-check-updates" aria-label="自动检查更新" aria-busy={automaticBusy} checked={automatic} disabled={!writable||typeof onPreference!=='function'||!networkEnabled||automaticBusy} onChange={(_,data)=>onPreference('autoCheckUpdates',data.checked)}/></div></Row>
      <Row icon="clock-24-regular" title="检查间隔" description="可先设置间隔，再开启自动检查。"><div className="switch-control">{intervalBusy&&<Symbol name="arrow-sync-24-regular" spin/>}<Choice id="update-interval" label="自动检查更新间隔" busy={intervalBusy} value={String(interval)} options={intervals.map(hours=>({value:String(hours),label:`${hours} 小时`}))} disabled={!writable||typeof onPreference!=='function'||!networkEnabled||intervalBusy} onChange={value=>onPreference('updateIntervalHours',Number(value))}/></div></Row>
      <Row icon={update.status==='available'?'arrow-download-24-regular':update.status==='current'?'checkmark-circle-24-regular':update.status==='error'?'warning-24-regular':'arrow-sync-24-regular'} title={title} description={<><span>上次检查：{checkedTime(update.checked_at)}</span>{update.checking&&!manualBusy&&<span className="background-check" role="status">后台检查中</span>}</>}><Button id="check-updates" className="text-action" appearance="subtle" aria-busy={manualBusy} disabled={!writable||!networkEnabled||typeof onCheck!=='function'||manualBusy} icon={<Symbol name={manualBusy?'arrow-sync-24-regular':'search-24-regular'} spin={manualBusy}/>} onClick={onCheck}>{manualBusy?'正在检查':'检查更新'}</Button></Row>
      {!update.checking&&update.status==='error'&&error&&<div className="release-notes error-text" role="status">{error}</div>}
      {!update.checking&&['current','available'].includes(update.status)&&notes&&<div className="group-footer update-notes-entry"><span>{update.status==='available'?`版本 ${update.latest||''} 的更新内容`:'查看此版本的更新内容'}</span><Dialog><DialogTrigger disableButtonEnhancement><Button appearance="subtle" aria-haspopup="dialog">查看更新说明</Button></DialogTrigger><DialogSurface className="release-dialog app-dialog"><DialogBody><DialogTitle>更新说明{update.latest?` · ${update.latest}`:''}</DialogTitle><DialogContent><ReleaseNotes notes={notes}/></DialogContent><DialogActions><DialogTrigger disableButtonEnhancement><Button appearance="secondary">关闭</Button></DialogTrigger></DialogActions></DialogBody></DialogSurface></Dialog></div>}
    </div>

  </>;
}

export function PrivacyInfo({busy=()=>false,onLogs,onRead}){
  const action=(id,label,callback)=><Button id={id} className="text-action" appearance="subtle" aria-busy={busy(id)} disabled={typeof callback!=='function'||busy(id)} icon={<Symbol name={busy(id)?'arrow-sync-24-regular':id==='open-source'?'github':'open-24-regular'} spin={busy(id)}/>} onClick={callback}>{label}</Button>;
  return <>
    <h2>本机数据与隐私</h2>
    <div className="settings-group">
      <ul className="privacy-summary"><li><strong>本机保存</strong> 配置、启动偏好和最近 200 条日志。</li><li><strong>不记录内容</strong> 不保存对话、提示词或用户 CLI 参数；可选流量统计只计字节。</li><li><strong>联网可控</strong> 版本检查、网页探测与价格同步可关闭，没有遥测或数据上报。</li></ul>
      <button type="button" className="about-detail-entry privacy-entry" onClick={onRead}><Symbol name="lock-closed-24-regular"/>完整数据与隐私说明<Symbol name="chevron-right-20-regular"/></button>
      <div className="group-footer"><span>日志保存在本机</span>{action('open-logs','打开日志目录',onLogs)}</div>
    </div>
  </>;
}

export function Disclaimer(){
  return <div className="disclaimer-copy settings-group"><p className="disclaimer-lead">请在了解当地法律、所在组织的网络规定和目标服务条款后使用本软件。</p><h2>工具与服务范围</h2><p>CodexProxyTray 是本地启动与代理环境管理工具，适用于合法、获得授权的网络连接和技术诊断。不提供 VPN、代理节点、跨境线路或账号，也不销售、推荐或担保第三方网络服务。</p><h2>依法合规使用</h2><p>在中国境内进行国际联网，应遵守《中华人民共和国计算机信息网络国际联网管理暂行规定》等现行规定，通过符合法律要求的网络和国际出入口信道联网。不得利用本软件自行建立、使用不符合规定的信道，或从事未经许可的跨境电信业务经营及其他违法活动。</p><h2>第三方与使用风险</h2><p>代理客户端、线路、ChatGPT、Codex CLI 及其账号均由对应服务提供方负责。请自行核实使用授权、数据处理方式与服务条款；本软件不保证线路安全、账号可用或所有连接问题均可解决。修改设置或关闭应用前，请先保存工作。</p><h2>数据与责任</h2><p>本软件的数据处理方式见“关于 → 本机数据与隐私”。项目依据 MIT 许可按现状提供，不附带对适销性、特定用途适用性等的保证；责任范围以适用法律和许可为准。本声明不排除依法不得排除的责任，也不使任何违法行为取得合法性。</p><p className="disclaimer-footnote">如不能确认某项使用是否合法或获得授权，请先停止该项使用，并咨询相关主管部门或合格专业人士。</p></div>;
}

export function PrivacyDetails(){return <div className="settings-group privacy-full"><div className="about-copy">
        <p>配置、启动偏好和最近 200 条分类运行日志保存在本机。</p>
        <p>本机采样包括代理端口连接耗时、目标进程树 TCP 连接数与守护进程内存。用量统计在本机读取 Codex 会话日志，只提取时间、模型、来源和 token 等用量元数据；解析时忽略对话内容，不保存或展示原文。不记录或上报对话、提示词、用户 CLI 参数；启动参数仅原样转发给已安装的 CLI。</p>
        <p>进程流量统计默认关闭，只按目标进程 ID 计数字节，不抓取内容、不保留连接 IP 或端口。打开实时连接详情时，会临时读取目标进程的当前 TCP 地址、端口和状态，用于本机展示，不写入日志或历史。安装或卸载辅助服务时请求 Windows 授权，正常开启、关闭统计不再反复授权。</p>
        <p>联网用于读取 GitHub 公开更新清单、用户手动发起的 npm 官方 CLI 更新检查与包管理、下载 models.dev 公开模型价格目录，以及经当前代理探测 ChatGPT 网页。价格目录经当前本地代理获取并缓存在本机，不向 models.dev 发送本机模型列表或用量记录。网页探测不使用 Cookie，只回显公网出口 IP、国家代码与 HTTP 耗时，不验证账号或对话。</p>
        <p>本应用没有遥测或数据上报。请求以应用名称与版本作标识，不附带本机日志、对话或参数；外部链接由你点击后在浏览器中打开。</p>
        <p>关闭允许联网检测后，停止后续助手外部请求，已设更新间隔会保留。目标应用代理、本机 TCP 探测和本机用量读取继续运行。用量费用按匹配到的公开参考价或你设置的覆盖价估算，不代表订阅账单；未匹配的模型不补造单价。</p>
      </div></div>;}
