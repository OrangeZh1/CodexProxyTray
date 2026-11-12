import React from 'react';
import {Button,Input,Tooltip} from '@fluentui/react-components';
import {Icon} from '@iconify/react';
import icons from '#fluent-icons';
import {Choice} from './choice.jsx';
import './log-filters.css';

const levels=[{value:'all',label:'全部级别'},{value:'warning',label:'警告'},{value:'error',label:'错误'}];
const periods=[{value:'all',label:'全部日期'},{value:'today',label:'今天'},{value:'yesterday',label:'昨天'},{value:'week',label:'近 7 天'},{value:'custom',label:'自选日期'}];
const Symbol=({name})=><Icon icon={icons[name]} className="log-filter-symbol" aria-hidden="true"/>;

export function LogFilters({level='all',period='all',query='',start='',end='',count,total,onLevel,onPeriod,onQuery,onStart,onEnd,onReset}){
  const custom=period==='custom';
  const invalid=custom&&!!start&&!!end&&start>end;
  const active=level!=='all'||period!=='all'||!!query;
  const known=Number.isInteger(count)&&count>=0&&Number.isInteger(total)&&total>=0;
  return <div className="log-filter-panel" role="search" aria-label="日志筛选"><div className="log-filter-controls">
    <Choice id="log-level-choice" label="日志级别" value={level} options={levels} onChange={value=>onLevel?.(value)}/>
    <Choice id="log-period-choice" label="日期范围" value={period} options={periods} onChange={value=>onPeriod?.(value)}/>
    <Input className="log-filter-query" id="log-query" aria-label="搜索日志" placeholder="搜索日志" type="search" value={query} contentBefore={<Symbol name="search-24-regular"/>} onChange={(_,data)=>onQuery?.(data.value)}/>
    <Tooltip content="重置筛选" relationship="label"><Button id="reset-log-filters" className="log-filter-reset" appearance="subtle" aria-label="重置筛选" disabled={!active} icon={<Symbol name="filter-dismiss-24-regular"/>} onClick={()=>onReset?.()}/></Tooltip>
  </div>{custom&&<div className="log-filter-custom"><label><span>开始日期</span><Input id="log-start-date" aria-label="日志开始日期" type="date" value={start} max={end||undefined} aria-invalid={invalid||undefined} onChange={(_,data)=>onStart?.(data.value)}/></label><label><span>结束日期</span><Input id="log-end-date" aria-label="日志结束日期" type="date" value={end} min={start||undefined} aria-invalid={invalid||undefined} onChange={(_,data)=>onEnd?.(data.value)}/></label></div>}{known&&<div className="log-filter-count" aria-live="polite">显示 {count} / {total} 条</div>}</div>;
}
