import {webDarkTheme,webLightTheme} from '@fluentui/react-components';

export const themeKey='color-theme';
export const themeModes=['system','light','dark'];
export const systemThemeQuery='(prefers-color-scheme: dark)';
export function readThemeMode(storage){
  try {const value=(storage??globalThis.localStorage)?.getItem(themeKey);return themeModes.includes(value)?value:'system';}
  catch{return 'system';}
}
export function resolveTheme(mode,systemDark){return mode==='dark'||mode==='system'&&systemDark?'dark':'light';}
export function prefersDark(){try{return !!globalThis.matchMedia?.(systemThemeQuery).matches;}catch{return false;}}
export function applyDocumentTheme(value){
  if(typeof document==='undefined')return;
  document.documentElement.dataset.theme=value;
  document.documentElement.style.colorScheme=value;
}
const fontFamilyBase='"Segoe UI Variable", "Segoe UI", "Microsoft YaHei UI", sans-serif';
export const themes={
  dark:{...webDarkTheme,fontFamilyBase,colorNeutralBackground1:'#242a35',colorNeutralBackground2:'#202631',colorNeutralForeground1:'#edf0f6',colorNeutralForeground2:'#b6c0d1',colorBrandBackground:'#0f6cbd',colorBrandBackgroundHover:'#115ea3',colorBrandForeground1:'#75bbff',colorStrokeFocus2:'#66b3ff'},
  light:{...webLightTheme,fontFamilyBase,colorBrandForeground1:'#005fb8',colorStrokeFocus2:'#0067c0'},
};
