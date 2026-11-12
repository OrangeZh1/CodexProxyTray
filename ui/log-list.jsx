import React,{useState} from 'react';
import {Button,Dialog,DialogSurface,DialogBody,DialogTitle,DialogContent,DialogActions} from '@fluentui/react-components';
import {Icon} from '@iconify/react';
import icons from '#fluent-icons';
import {StatusLabel} from './status-label.jsx';
import {logDate,logTime} from './logs.mjs';

export function EventList({logs,compact=false,emptyLabel='暂无日志'}){
  const [selected,setSelected]=useState(null),[open,setOpen]=useState(false);
  return <><ol className={`logs-list ${compact?'compact-events':''}`}>
    {logs.length?logs.map((log,index)=><li key={log.id||`${log.at}-${index}`} className={`log-entry level-${log.level}`}><button type="button" className="log-open" aria-label={`查看日志：${log.message}`} onClick={()=>{setSelected(log);setOpen(true);}}><time dateTime={new Date(log.at).toISOString()}>{logTime(log.at)}</time><Icon icon={icons[log.level==='error'||log.level==='warning'?'warning-24-regular':'checkmark-circle-24-regular']} className="symbol" aria-hidden="true"/><div className="log-copy"><div className="log-message"><span className="log-category">{log.category||'运行'}</span>{log.message}</div>{!compact&&log.details&&<div className="log-details log-preview">{log.details}</div>}</div><Icon className="log-chevron" icon={icons['chevron-right-20-regular']} aria-hidden="true"/></button></li>):<li className="empty-state">{emptyLabel}</li>}
  </ol><Dialog open={open} onOpenChange={(_,data)=>setOpen(data.open)}><DialogSurface className="log-detail-dialog app-dialog"><DialogBody><DialogTitle>{selected?.message||'日志详情'}</DialogTitle><DialogContent>{selected&&<><dl className="log-detail-meta"><div><dt>时间</dt><dd>{logDate(selected.at)} {logTime(selected.at)}</dd></div><div><dt>分类</dt><dd>{selected.category||'运行'}</dd></div><div><dt>级别</dt><dd><StatusLabel color={selected.level==='error'?'danger':selected.level==='warning'?'warning':'subtle'}>{{info:'信息',warning:'警告',error:'错误'}[selected.level]||selected.level}</StatusLabel></dd></div></dl><div className="log-detail-text">{selected.details||'没有附加详情。'}</div></>}</DialogContent><DialogActions><Button appearance="secondary" onClick={()=>setOpen(false)}>关闭</Button></DialogActions></DialogBody></DialogSurface></Dialog></>;
}
