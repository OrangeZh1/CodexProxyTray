import React from 'react';

export function StatusLabel({color='subtle',className='',children,...props}){
  const tone=['success','warning','danger'].includes(color)?color:'neutral';
  return <span className={`status-label status-${tone} ${className}`.trim()} data-status={tone} {...props}><span className="status-label-dot" aria-hidden="true"/><span>{children}</span></span>;
}
