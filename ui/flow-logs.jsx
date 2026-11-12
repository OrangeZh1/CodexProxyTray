import React,{useEffect,useRef,useState} from 'react';
import {Button,Switch,Input} from '@fluentui/react-components';
import {Icon} from '@iconify/react';
import icons from '#fluent-icons';
import {logDate,logTime} from './logs.mjs';

const bytes=(value,rate=false)=>{if(!Number.isFinite(value)||value<0)return '—';let number=value,index=0;while(number>=1024&&index<3){number/=1024;index++;}return `${new Intl.NumberFormat('zh-CN',{maximumFractionDigits:1}).format(number)} ${['B','KB','MB','GB'][index]}${rate?'/s':''}`;};
export function FlowLogs({invoke,writable}){
  const [data,setData]=useState(null),[error,setError]=useState(null),[refreshBusy,setRefreshBusy]=useState(false),[saveBusy,setSaveBusy]=useState(false),[query,setQuery]=useState('');
  const active=useRef(true),readPending=useRef(null),writePending=useRef(false),reader=useRef(null);
  useEffect(()=>{active.current=true;let disposed=false;
    async function read(){if(disposed||writePending.current||typeof invoke!=='function')return;if(readPending.current)return readPending.current;
      const pending=invoke('get_flow_logs').then(value=>{if(!value||!Array.isArray(value.entries)||!Array.isArray(value.samples))throw Error('连接日志数据格式无效。');if(!disposed){setData(value);setError(value.error||null);}return value;}).catch(failure=>{if(!disposed)setError(String(failure?.message||failure));}).finally(()=>{if(readPending.current===pending)readPending.current=null;});readPending.current=pending;return pending;
    }
    reader.current=read;read();const timer=setInterval(read,5000);return()=>{disposed=true;active.current=false;clearInterval(timer);reader.current=null;};
  },[invoke]);
  async function refresh(){if(refreshBusy)return;setRefreshBusy(true);try{await reader.current?.();}finally{if(active.current)setRefreshBusy(false);}}
  async function toggle(enabled){if(writePending.current||!writable)return;writePending.current=true;setSaveBusy(true);try{await readPending.current;const value=await invoke('set_flow_logging',{enabled});if(active.current){setData(value);setError(value.error||null);}}catch(failure){if(active.current)setError(String(failure?.message||failure));}finally{writePending.current=false;if(active.current)setSaveBusy(false);}}
  const rows=(data?.entries||[]).filter(row=>`${row.remote} ${row.local} ${row.process} ${row.pid}`.toLowerCase().includes(query.trim().toLowerCase()));
  const samples=[...(data?.samples||[])].reverse().slice(0,20);
  return <div className="flow-logs"><div className="settings-group"><div className="setting-row"><div className="row-copy"><div className="row-title">本机连接与流量日志</div><div className="row-description">开启后在本机保留最近 300 条连接与流量采样。包含进程、地址和端口；不记录通信内容，不上传。</div></div><Switch id="flow-log-enabled" aria-label="记录本机连接与流量日志" aria-busy={saveBusy} input={{'aria-busy':saveBusy}} disabled={!writable||saveBusy} checked={!!data?.enabled} onChange={(_,value)=>toggle(value.checked)}/></div><div className="group-footer"><span>{data?.enabled?'每 5 秒采样；短于采样间隔的连接可能遗漏。':'记录已关闭，已有历史保留在本机。'}</span><Button id="flow-log-refresh" appearance="subtle" aria-busy={refreshBusy} disabled={!invoke||refreshBusy} icon={<Icon className={refreshBusy?'symbol spin':'symbol'} icon={icons['arrow-sync-24-regular']}/>} onClick={refresh}>刷新</Button></div></div>
    {error&&<p className="error-text" role="alert">{error}</p>}
    <div className="flow-log-heading"><h2>连接记录</h2><Input id="flow-log-search" aria-label="搜索连接记录" placeholder="搜索进程或地址" value={query} onChange={(_,value)=>setQuery(value.value)}/></div>
    <p className="metric-detail">TCP 地址来自 Windows 已验证的目标进程。未提供域名、代理规则、节点及单连接字节；流量采样为所有目标进程合计。</p>
    <ol className="flow-log-list">{rows.length?rows.map(row=><li key={`${row.pid}:${row.created}:${row.local}:${row.remote}`}><div className="flow-log-top"><time>{logDate(row.first_at)} {logTime(row.first_at)}</time><span>{data.enabled&&row.present?'本次采样可见':'后续采样未见'}</span></div><strong>{row.remote}</strong><p>{row.process} · PID {row.pid} · TCP</p><p>本地 {row.local}<span>最后可见 {logTime(row.last_at)}</span></p></li>):<li className="empty-state">{data?.enabled?'尚未采样到目标连接':'开启记录后在此显示连接历史'}</li>}</ol>
    {data?.truncated&&<p className="warning-text">当前记录达到上限，或本轮连接列表未完整覆盖；仅保留最近记录。</p>}
    <h2>流量采样</h2><p className="metric-detail">采集服务可用并开启流量统计后记录。— 表示未获得有效计量，累计值从本次采集会话开始。</p>
    <div className="flow-sample-scroll"><table className="flow-samples"><thead><tr><th>时间</th><th>上传速率</th><th>下载速率</th><th>累计上传</th><th>累计下载</th></tr></thead><tbody>{samples.map(row=><tr key={row.at}><td>{logTime(row.at)}</td><td>{bytes(row.send_bps,true)}</td><td>{bytes(row.receive_bps,true)}</td><td>{bytes(row.sent_bytes)}</td><td>{bytes(row.received_bytes)}</td></tr>)}</tbody></table>{!samples.length&&<p className="empty-state">暂无有效流量采样</p>}</div>
  </div>;
}
