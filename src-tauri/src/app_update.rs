//! Explicit, same-user update of this assistant only. The detached installer has
//! no URLs or commands from the frontend, and never elevates or kills user tasks.
use std::{fs,io::{Read,Write},os::windows::process::CommandExt,path::{Path,PathBuf},process::{Child,Command,Stdio},sync::{Mutex,OnceLock},thread,time::{Duration,Instant}};
use serde::{Serialize,Deserialize};
use crate::settings;

const RELEASES:&str="https://github.com/OrangeZh1/CodexProxyTray/releases";
const SCRIPT:&str=include_str!("install_update.ps1");
const MAX_ZIP:u64=128*1024*1024;
#[derive(Clone,Default,Serialize,Deserialize)]#[serde(default)]
pub struct Progress{pub busy:bool,pub phase:String,pub version:Option<String>,pub downloaded:u64,pub total:Option<u64>,pub error:Option<String>}
#[derive(Default)]struct Gate{updating:bool,management:u32}
static GATE:Mutex<Gate>=Mutex::new(Gate{updating:false,management:0});
static VIEW:OnceLock<Mutex<Progress>>=OnceLock::new();
fn view()->&'static Mutex<Progress>{VIEW.get_or_init(||{let saved=fs::read(settings::root().join("update-install-result.json")).ok().filter(|b|b.len()<=16384).and_then(|bytes|serde_json::from_slice::<Progress>(&bytes).ok());Mutex::new(saved.unwrap_or_default())})}
pub fn snapshot()->Progress{view().lock().unwrap_or_else(|e|e.into_inner()).clone()}
pub fn busy()->bool{GATE.lock().unwrap_or_else(|e|e.into_inner()).updating}
pub struct Lease{update:bool}
impl Drop for Lease{fn drop(&mut self){let mut gate=GATE.lock().unwrap_or_else(|e|e.into_inner());if self.update{gate.updating=false;view().lock().unwrap_or_else(|e|e.into_inner()).busy=false;}else{gate.management=gate.management.saturating_sub(1);}}}
pub fn management()->Result<Lease,String>{let mut gate=GATE.lock().map_err(|_|"操作状态暂不可用。")?;if gate.updating{return Err("助手正在准备更新，请等待完成。".into())}gate.management+=1;Ok(Lease{update:false})}
fn begin(version:&str)->Result<Lease,String>{let mut gate=GATE.lock().map_err(|_|"操作状态暂不可用。")?;if gate.updating||gate.management!=0{return Err("另一项管理操作仍在进行，请稍后更新助手。".into())}gate.updating=true;*view().lock().unwrap_or_else(|e|e.into_inner())=Progress{busy:true,phase:"downloading".into(),version:Some(version.into()),..Progress::default()};Ok(Lease{update:true})}
fn phase(value:&str){view().lock().unwrap_or_else(|e|e.into_inner()).phase=value.into();}
fn version(value:&str)->bool{!value.starts_with('v')&&crate::core::version_parts(value).is_some()}
fn checksum(text:&str,name:&str)->Result<String,String>{
    let mut found=None;for line in text.lines(){let Some((hash,file))=line.split_once("  ")else{return Err("官方校验清单格式无效。".into())};if hash.len()!=64||!hash.bytes().all(|b|b.is_ascii_hexdigit())||file.contains(['/', '\\']){return Err("官方校验清单无效。".into())}if file==name{if found.is_some(){return Err("官方校验清单包含重复文件。".into())}found=Some(hash.to_ascii_lowercase());}}
    found.ok_or_else(||"官方校验清单缺少安装包。".into())
}
fn client(config:&settings::Config)->Result<reqwest::blocking::Client,String>{
    if !config.network_enabled{return Err("联网检测已关闭，未下载更新。".into())}
    reqwest::blocking::Client::builder().no_proxy().proxy(reqwest::Proxy::all(&config.proxy_url).map_err(|_|"本地代理设置无效。")?).connect_timeout(Duration::from_secs(12)).timeout(Duration::from_secs(180)).user_agent(concat!("CodexProxyTray/",env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::custom(|attempt|{let url=attempt.url();if attempt.previous().len()<5&&url.scheme()=="https"&&matches!(url.host_str(),Some("github.com"|"release-assets.githubusercontent.com")){attempt.follow()}else{attempt.error("Unexpected update redirect")}})).build().map_err(|_|"更新下载暂不可用。".into())
}
fn download(client:&reqwest::blocking::Client,url:&str,path:&Path,limit:u64,report:bool)->Result<(),String>{
    let mut response=client.get(url).send().map_err(|_|"无法连接 GitHub，请检查本地代理。")?.error_for_status().map_err(|_|"官方更新附件暂不可用，请稍后重试。")?;
    if response.content_length().is_some_and(|n|n>limit){return Err("更新附件超过允许大小。".into())}
    let mut file=fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(|_|"无法创建下载文件。")?;
    if report{view().lock().unwrap_or_else(|e|e.into_inner()).total=response.content_length();}
    let mut bytes=0u64;let mut buffer=[0u8;65536];loop{let n=response.read(&mut buffer).map_err(|_|"更新下载中断，尚未替换程序。")?;if n==0{break}bytes+=n as u64;if bytes>limit{return Err("更新附件超过允许大小。".into())}file.write_all(&buffer[..n]).map_err(|_|"更新下载无法写入磁盘。")?;if report{view().lock().unwrap_or_else(|e|e.into_inner()).downloaded=bytes;}}
    if response.content_length().is_some_and(|n|n!=bytes){return Err("更新附件下载不完整。".into())}file.sync_all().map_err(|_|"更新附件未能完整保存。".into())
}
fn powershell()->PathBuf{PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(||"C:\\Windows".into())).join("System32/WindowsPowerShell/v1.0/powershell.exe")}
fn command(job:&Path,phase:&str)->Command{let mut command=Command::new(powershell());command.args(["-NoLogo","-NoProfile","-NonInteractive","-ExecutionPolicy","Bypass","-File"]).arg(job.join("installer.ps1")).args(["-Phase",phase,"-Job"]).arg(job).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).creation_flags(0x08000000);command}
fn wait_prepare(child:&mut Child)->Result<(),String>{let deadline=Instant::now()+Duration::from_secs(45);loop{match child.try_wait(){Ok(Some(status))=>return if status.success(){Ok(())}else{Err("安装包或安装目录校验未通过；原程序和恢复资料已保留。".into())},Ok(None)=>{},Err(_)=>return Err("无法读取安装校验结果。".into())}if Instant::now()>=deadline{let _=child.kill();let _=child.wait();return Err("安装校验超时，原程序未替换。".into())}thread::sleep(Duration::from_millis(100));}}
pub struct Prepared{job:PathBuf,_lease:Lease}
impl Drop for Prepared{fn drop(&mut self){let _=fs::write(self.job.join("cancelled.json"),b"{}");let mut state=view().lock().unwrap_or_else(|e|e.into_inner());state.phase="error".into();state.error=Some("更新交接未完成，助手继续运行。".into());}}
impl Prepared{
    pub fn handoff(self)->Result<Self,String>{
        // The script signals readiness only after holding the installation lock.
        let mut child=command(&self.job,"Apply").spawn().map_err(|_|"无法启动后台安装程序。")?;
        let deadline=Instant::now()+Duration::from_secs(10);loop{if self.job.join("handoff-ready.json").is_file(){phase("restarting");return Ok(self)}if child.try_wait().map_err(|_|"安装交接状态不可读。")?.is_some(){return Err("后台安装程序未接受交接；助手继续运行。".into())}if Instant::now()>=deadline{return Err("后台安装交接超时；助手继续运行，安装程序会自行退出。".into())}thread::sleep(Duration::from_millis(100));}
    }
}
pub fn prepare(expected:String)->Result<Prepared,String>{
    if !version(&expected)||!crate::core::newer_version(&expected,env!("CARGO_PKG_VERSION")){return Err("请先检查并选择可用的新版本。".into())}
    let checked=crate::update::snapshot();if checked.status!="available"||checked.latest.as_deref()!=Some(&expected){return Err("更新信息已变化，请重新检查。".into())}
    let lease=begin(&expected)?;
    let result=(||{
        let root=settings::root();let config=settings::load(&root).map_err(|_|"无法读取现有配置，未下载更新。")?;let client=client(&config)?;
        let job=root.join("updates").join(format!("{}-{}",std::process::id(),settings::now()));fs::create_dir_all(job.parent().unwrap()).and_then(|_|fs::create_dir(&job)).map_err(|_|"无法创建更新工作目录。")?;
        fs::write(job.join("installer.ps1"),format!("\u{feff}{SCRIPT}")).map_err(|_|"无法准备安装程序。")?;
        let base=format!("{RELEASES}/download/v{expected}");let name=format!("CodexProxyTray-v{expected}-windows-x64.zip");
        download(&client,&format!("{base}/SHA256SUMS.txt"),&job.join("release-sums.txt"),65536,false)?;
        let sums=fs::read_to_string(job.join("release-sums.txt")).map_err(|_|"官方校验清单不可读取。")?;let zip_hash=checksum(&sums,&name)?;let manifest_hash=checksum(&sums,"CodexProxyTray-update.json")?;
        download(&client,&format!("{base}/CodexProxyTray-update.json"),&job.join("manifest.json"),65536,false)?;
        download(&client,&format!("{base}/{name}"),&job.join("package.zip"),MAX_ZIP,true)?;
        let exe=std::env::current_exe().map_err(|_|"无法确定当前安装位置。")?;
        let identity=unsafe{crate::traffic::identity_on(windows::Win32::System::Threading::GetCurrentProcess())}.map_err(|_|"无法核验助手进程。")?;
        settings::write_json(&job.join("plan.json"),&serde_json::json!({"schema":1,"product":"CodexProxyTray","version":expected,"zip_sha256":zip_hash,"manifest_sha256":manifest_hash,"executable":exe,"data_root":root,"parent_pid":std::process::id(),"parent_created":identity.created_filetime.to_string()}))?;
        phase("verifying");let mut child=command(&job,"Prepare").spawn().map_err(|_|"无法启动安装包校验。")?;wait_prepare(&mut child)?;
        if !settings::load(&root).is_ok_and(|c|c.network_enabled){return Err("联网检测已关闭，更新已停止，原程序保持运行。".into())}
        Ok(job)
    })();
    match result{Ok(job)=>Ok(Prepared{job,_lease:lease}),Err(error)=>{let mut state=view().lock().unwrap_or_else(|e|e.into_inner());state.phase="error".into();state.error=Some(error.clone());Err(error)}}
}

#[cfg(test)]mod tests{use super::*;#[test]fn only_canonical_versions_and_exact_unique_asset_hashes_are_accepted(){assert!(version("0.2.1"));for bad in ["v0.2.1","../0.2.1","0.2.1-beta","01.2.1"]{assert!(!version(bad));}let hash="a".repeat(64);assert_eq!(checksum(&format!("{hash}  package.zip\n"),"package.zip").unwrap(),hash);assert!(checksum(&format!("{hash}  package.zip\n{hash}  package.zip"),"package.zip").is_err());assert!(checksum(&format!("{hash}  ../package.zip"),"package.zip").is_err());assert!(checksum("bogus  package.zip","package.zip").is_err());}}
