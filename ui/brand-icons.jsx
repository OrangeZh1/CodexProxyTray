import React from 'react';
import chatgpt from './chatgpt.svg';
import github from './github.svg';

// Canonical vendor SVG paths remain in the source assets. Render them as icons
// so the selected application theme, rather than the OS favicon theme, sets fill.
const marks=Object.fromEntries(Object.entries({chatgpt,github}).map(([name,source])=>[name,{viewBox:/viewBox="([^"]+)"/.exec(source)?.[1],paths:[...source.matchAll(/<path\b[^>]*\bd="([^"]+)"[^>]*\/?\s*>/g)].map(match=>({d:match[1],fillRule:/fill-rule="([^"]+)"/.exec(match[0])?.[1],clipRule:/clip-rule="([^"]+)"/.exec(match[0])?.[1]}))}]));
export function BrandIcon({name,className='symbol'}){const mark=marks[name];return <svg className={className} viewBox={mark.viewBox} fill="currentColor" aria-hidden="true">{mark.paths.map((path,index)=><path key={index} {...path}/>)}</svg>;}
