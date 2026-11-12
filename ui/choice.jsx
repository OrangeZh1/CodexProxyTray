import React,{useEffect,useRef,useState} from 'react';
import {Dropdown,Option} from '@fluentui/react-components';

export function Choice({id,label,value,options,onChange,disabled=false,busy=false}){
  const selected=options.find(option=>option.value===value);
  const [open,setOpen]=useState(false),[closing,setClosing]=useState(false),[keyboard,setKeyboard]=useState(false),listbox=useRef(null);
  useEffect(()=>{if(!closing)return;let live=true;const finish=()=>{if(live){setOpen(false);setClosing(false);}},animations=(listbox.current?.getAnimations?.()||[]).filter(animation=>animation.animationName==='popup-exit'&&!['finished','idle'].includes(animation.playState));if(!animations.length){finish();return;}Promise.allSettled(animations.map(animation=>animation.finished)).then(finish);return()=>{live=false;};},[closing]);
  return <Dropdown className={`setting-select${open&&!closing?' is-open':''}`} open={open} onOpenChange={(event,data)=>{if(data.open){setKeyboard(event.type==='keydown');setClosing(false);setOpen(true);}else if(open)setClosing(true);}} button={{id,'aria-label':label,'aria-busy':busy,'aria-expanded':open&&!closing,onPointerDown:()=>setKeyboard(false),onKeyDown:()=>setKeyboard(true)}} listbox={{ref:listbox,className:`setting-options${closing?' is-closing':''}`,'data-pointer-open':!keyboard||undefined,'aria-hidden':closing||undefined,inert:closing||undefined}} value={selected?.label||''} selectedOptions={selected?[value]:[]} disabled={disabled} onOptionSelect={(_,data)=>{if(!closing&&options.some(option=>option.value===data.optionValue))onChange(data.optionValue);}}>
    {options.map(option=><Option key={option.value} value={option.value} text={option.label}><span className="choice-label">{option.label}</span></Option>)}
  </Dropdown>;
}
