import React from 'react';
import {Dropdown,Option} from '@fluentui/react-components';

export function Choice({id,label,value,options,onChange,disabled=false,busy=false}){
  const selected=options.find(option=>option.value===value);
  return <Dropdown className="setting-select" button={{id,'aria-label':label,'aria-busy':busy}} listbox={{className:'setting-options'}} value={selected?.label||''} selectedOptions={selected?[value]:[]} disabled={disabled} onOptionSelect={(_,data)=>{if(options.some(option=>option.value===data.optionValue))onChange(data.optionValue);}}>
    {options.map(option=><Option key={option.value} value={option.value}>{option.label}</Option>)}
  </Dropdown>;
}
