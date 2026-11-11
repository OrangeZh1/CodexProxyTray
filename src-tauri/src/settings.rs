use serde::{Deserialize, Deserializer, Serialize};
use std::{path::{Path,PathBuf}, fs, time::{SystemTime,UNIX_EPOCH}};
use crate::core::{endpoint,DEFAULT_PROXY};

#[derive(Clone,Copy,Debug,Default,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="snake_case")]
pub enum LaunchTarget {#[default] None,Chatgpt,Cli,Both}
impl LaunchTarget {
    pub fn opens_desktop(self)->bool {matches!(self,Self::Chatgpt|Self::Both)}
    pub fn opens_cli(self)->bool {matches!(self,Self::Cli|Self::Both)}
}

#[derive(Clone,Serialize,Deserialize)]
#[serde(try_from="ConfigInput")]
pub struct Config {
    pub proxy_url:String,
    pub launch_target:LaunchTarget,
    pub cli_workdir:String,
    pub network_enabled:bool,
    pub auto_check_updates:bool,
    pub update_interval_hours:u32,
    pub cli_command:Vec<String>,
}
impl Default for Config {fn default()->Self{Self{proxy_url:DEFAULT_PROXY.into(),launch_target:LaunchTarget::None,cli_workdir:String::new(),network_enabled:true,auto_check_updates:false,update_interval_hours:6,cli_command:vec![]}}}

// Missing launch_target is a legacy config; an explicit null/unknown value is invalid.
fn launch_target_field<'de,D:Deserializer<'de>>(deserializer:D)->Result<Option<LaunchTarget>,D::Error>{LaunchTarget::deserialize(deserializer).map(Some)}
#[derive(Deserialize)]
#[serde(default,deny_unknown_fields)]
struct ConfigInput {
    proxy_url:String,
    #[serde(default,deserialize_with="launch_target_field")]
    launch_target:Option<LaunchTarget>,
    launch_desktop:bool,
    cli_workdir:String,
    network_enabled:bool,
    auto_check_updates:bool,
    update_interval_hours:u32,
    cli_command:Vec<String>,
}
impl Default for ConfigInput {fn default()->Self{let config=Config::default();Self{proxy_url:config.proxy_url,launch_target:None,launch_desktop:false,cli_workdir:config.cli_workdir,network_enabled:config.network_enabled,auto_check_updates:config.auto_check_updates,update_interval_hours:config.update_interval_hours,cli_command:config.cli_command}}}
impl TryFrom<ConfigInput> for Config {
    type Error=String;
    fn try_from(input:ConfigInput)->Result<Self,String>{
        validate_update_interval(input.update_interval_hours)?;
        cli_workdir_path(&input.cli_workdir)?;
        Ok(Self{proxy_url:endpoint(&input.proxy_url)?.url,launch_target:input.launch_target.unwrap_or(if input.launch_desktop{LaunchTarget::Chatgpt}else{LaunchTarget::None}),cli_workdir:input.cli_workdir.trim().into(),network_enabled:input.network_enabled,auto_check_updates:input.auto_check_updates,update_interval_hours:input.update_interval_hours,cli_command:input.cli_command})
    }
}
pub fn validate_update_interval(hours:u32)->Result<(),String>{if matches!(hours,1|3|6|12|24){Ok(())}else{Err("自动检查间隔只能为 1、3、6、12 或 24 小时。".into())}}

fn cli_workdir_path(value:&str)->Result<Option<PathBuf>,String>{
    let value=value.trim();if value.is_empty(){return Ok(None)}
    let path=PathBuf::from(value);
    if value.contains('\0')||!path.is_absolute(){return Err("Codex CLI 工作目录需要使用有效的完整路径。".into())}
    Ok(Some(path))
}
pub fn validate_cli_workdir(value:&str)->Result<PathBuf,String>{
    let path=if let Some(path)=cli_workdir_path(value)?{path}else{
        #[cfg(windows)] let home=std::env::var_os("USERPROFILE");
        #[cfg(not(windows))] let home=std::env::var_os("HOME");
        PathBuf::from(home.filter(|path|!path.is_empty()).ok_or("无法确定用户目录，请设置 Codex CLI 工作目录。")?)
    };
    if !path.is_absolute(){return Err("Codex CLI 工作目录需要使用完整路径。".into())}
    match fs::metadata(&path){Ok(info)if info.is_dir()=>Ok(path),Ok(_)=>Err("Codex CLI 工作目录不是文件夹。".into()),Err(error)=>Err(format!("Codex CLI 工作目录不存在或无法访问：{error}"))}
}
pub fn cli_directory(config:&Config)->Result<PathBuf,String>{validate_cli_workdir(&config.cli_workdir)}
pub fn now()->u64{SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64}
pub fn root()->PathBuf { PathBuf::from(std::env::var_os("LOCALAPPDATA").expect("LOCALAPPDATA is required")).join("CodexProxyTray") }
pub fn load(root:&Path)->Result<Config,String>{let path=root.join("config.json");match fs::read(path){Ok(bytes)=>serde_json::from_slice(&bytes).map_err(|error|error.to_string()),Err(error)if error.kind()==std::io::ErrorKind::NotFound=>Ok(Config::default()),Err(error)=>Err(error.to_string())}}
pub fn write_json(path:&Path,value:&impl Serialize)->Result<(),String>{let temporary=path.with_extension("json.tmp");fs::write(&temporary,serde_json::to_vec_pretty(value).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;fs::rename(temporary,path).map_err(|e|e.to_string())}
pub fn save(root:&Path,config:&Config)->Result<(),String>{
    validate_update_interval(config.update_interval_hours)?;cli_workdir_path(&config.cli_workdir)?;
    let mut next=config.clone();next.proxy_url=endpoint(&next.proxy_url)?.url;next.cli_workdir=next.cli_workdir.trim().into();
    write_json(&root.join("config.json"),&next)
}
pub fn event_name(root:&Path)->String{let mut hash=0xcbf29ce484222325u64;for b in root.to_string_lossy().to_lowercase().bytes(){hash^=b as u64;hash=hash.wrapping_mul(0x100000001b3)}format!("Local\\CodexProxyTray.Guard.{hash:016x}")}
fn owned_path(path:&Path,root:&Path)->bool{
    // Canonicalization handles junctions/aliases; Windows path lookup is case insensitive.
    let path=path.canonicalize().unwrap_or_else(|_|path.to_path_buf()).to_string_lossy().replace('/',"\\").to_lowercase();
    let root=root.canonicalize().unwrap_or_else(|_|root.to_path_buf()).to_string_lossy().replace('/',"\\").to_lowercase();
    path.starts_with(&(root.trim_end_matches('\\').to_owned()+"\\"))
}
pub fn command_available(command:&[String],root:&Path)->bool{
    let Some(program)=command.first().map(PathBuf::from)else{return false};
    if !program.is_file()||owned_path(&program,root){return false}
    if program.file_name().is_some_and(|name|name.to_string_lossy().eq_ignore_ascii_case("node.exe")){
        return command.get(1).is_some_and(|script|Path::new(script).is_file());
    }
    true
}
pub fn detect_cli(root:&Path)->Vec<String>{
    let npm=PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join("npm/node_modules/@openai/codex/bin/codex.js");
    let paths=std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect::<Vec<_>>();
    if npm.is_file(){if let Some(node)=paths.iter().map(|p|p.join("node.exe")).find(|p|p.is_file()){return vec![node.to_string_lossy().into(),npm.to_string_lossy().into()]}}
    for path in paths{let exe=path.join("codex.exe");if exe.is_file()&&!owned_path(&exe,root){return vec![exe.to_string_lossy().into()]}}
    vec![]
}
pub fn cli_lookup_info(root:&Path)->serde_json::Value {
    let appdata=PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default());
    let script=appdata.join("npm/node_modules/@openai/codex/bin/codex.js");
    let paths=std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect::<Vec<_>>();
    serde_json::json!({"config_root":root,"config_exists":root.join("config.json").is_file(),"npm_script":script,"npm_script_exists":script.is_file(),"node_candidates":paths.iter().take(12).map(|p|{let node=p.join("node.exe");serde_json::json!({"path":node,"exists":node.is_file()})}).collect::<Vec<_>>()})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_startup_migrates_without_rewriting_or_losing_preferences(){
        let old:Config=serde_json::from_str(r#"{"launch_desktop":true,"proxy_url":"http://localhost:17890/","cli_command":["node.exe","codex.js"]}"#).unwrap();
        assert_eq!(old.launch_target,LaunchTarget::Chatgpt);assert_eq!(old.proxy_url,"http://localhost:17890");assert_eq!(old.cli_command,vec!["node.exe","codex.js"]);
        assert!(old.network_enabled);assert!(!old.auto_check_updates);assert_eq!(old.update_interval_hours,6);
        let encoded=serde_json::to_value(&old).unwrap();assert!(encoded.get("launch_desktop").is_none());assert_eq!(encoded["launch_target"],"chatgpt");
        let explicit:Config=serde_json::from_str(r#"{"launch_target":"none","launch_desktop":true}"#).unwrap();assert_eq!(explicit.launch_target,LaunchTarget::None);
        for (target,desktop,cli) in [(LaunchTarget::None,false,false),(LaunchTarget::Chatgpt,true,false),(LaunchTarget::Cli,false,true),(LaunchTarget::Both,true,true)]{assert_eq!(target.opens_desktop(),desktop);assert_eq!(target.opens_cli(),cli);}
        for invalid in [r#"{"launch_target":null}"#,r#"{"launch_target":"unknown"}"#,r#"{"launch_desktop":null}"#,r#"{"update_interval_hours":2}"#,r#"{"unknown":true}"#]{assert!(serde_json::from_str::<Config>(invalid).is_err(),"{invalid}");}
        for hours in [1,3,6,12,24]{assert!(validate_update_interval(hours).is_ok());}
    }
    #[test]
    fn removed_workdir_does_not_block_unrelated_preferences_or_allow_invalid_writes(){
        let suffix=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root=std::env::temp_dir().join(format!("codex-proxy-tray-settings-{}-{suffix}",std::process::id()));fs::create_dir(&root).unwrap();
        let project=root.join("project");fs::create_dir(&project).unwrap();assert_eq!(validate_cli_workdir(project.to_str().unwrap()).unwrap(),project);
        fs::remove_dir(&project).unwrap();
        let mut config=Config::default();config.cli_workdir=project.to_str().unwrap().into();config.network_enabled=false;
        save(&root,&config).unwrap();let loaded=load(&root).unwrap();assert!(!loaded.network_enabled);assert_eq!(loaded.cli_workdir,config.cli_workdir);assert!(cli_directory(&loaded).is_err());assert!(!project.exists());
        let path=root.join("config.json");let original=fs::read(&path).unwrap();config.update_interval_hours=2;assert!(save(&root,&config).is_err());assert_eq!(fs::read(&path).unwrap(),original);
        config.update_interval_hours=6;config.cli_workdir="relative-project".into();assert!(save(&root,&config).is_err());assert_eq!(fs::read(&path).unwrap(),original);
        fs::write(&path,b"{broken").unwrap();assert!(load(&root).is_err());assert_eq!(fs::read(&path).unwrap(),b"{broken");
        fs::remove_file(path).unwrap();fs::remove_dir(root).unwrap();
    }
}
