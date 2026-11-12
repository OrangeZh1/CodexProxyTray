import React,{useState} from 'react';
import manifest from './flags/manifest.json';

const available=new Set(manifest.codes.map(code=>code.toUpperCase()));
let regionNames;
try{regionNames=new Intl.DisplayNames(['zh-CN'],{type:'region',fallback:'none'});}catch{}

// Local SVG assets avoid Windows' inconsistent support for regional flag emoji.
// showName=false keeps the compact country code; showCode adds it after a name.
export function CountryFlag({code,className='',showName=true,showCode=false}){
  const normalized=typeof code==='string'&&/^[a-z]{2}$/i.test(code.trim())?code.trim().toUpperCase():'';
  const known=available.has(normalized);
  const [failed,setFailed]=useState(null);
  const name=known&&showName?regionNames?.of(normalized)||normalized:normalized;
  const label=name?(showCode&&name!==normalized?`${name} ${normalized}`:name):'—';
  return <span className={`country-flag ${className}`.trim()} style={{display:'inline-flex',alignItems:'center',gap:6,verticalAlign:'middle'}}>
    {known&&failed!==normalized&&<img src={`flags/${normalized.toLowerCase()}.svg`} width="20" height="15" alt="" aria-hidden="true" draggable={false} onError={()=>setFailed(normalized)} style={{display:'block',flexShrink:0,objectFit:'cover',borderRadius:2}}/>}
    <span>{label}</span>
  </span>;
}
