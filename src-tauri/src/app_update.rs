//! Explicit, same-user update of this assistant only. The detached installer has
//! no URLs or commands from the frontend, and never elevates or kills user tasks.
use std::{fs,io::{Read,Write},os::windows::process::CommandExt,path::{Path,PathBuf},process::{Child,Command,Stdio},sync::{Mutex,OnceLock},thread,time::{Duration,Instant}};
use serde::{Serialize,Deserialize};
use crate::settings;

const RELEASES:&str="https://github.com/OrangeZh1/CodexProxyTray/releases";
const SCRIPT:&str=include_str!("install_update.ps1");
const MAX_ZIP:u64=128*1024*1024;
#[derive(Clone,Default,Serialize,Deserialize)]#[serde(default)]
pub struct Progress{pub started_at:u64,pub completed_at:u64,pub operation:String,pub busy:bool,pub phase:String,pub version:Option<String>,pub downloaded:u64,pub total:Option<u64>,pub error:Option<String>}
#[derive(Default)]struct Gate{updating:bool,downloading:bool,management:u32}
static GATE:Mutex<Gate>=Mutex::new(Gate{updating:false,downloading:false,management:0});
static VIEW:OnceLock<Mutex<Progress>>=OnceLock::new();
static READY:OnceLock<Mutex<Option<Prepared>>>=OnceLock::new();
#[derive(Serialize,Deserialize)]struct ReadyReceipt{schema:u32,job:String,version:String}
fn read_small(path:&Path,limit:u64)->Option<Vec<u8>>{let file=fs::File::open(path).ok()?;let mut bytes=vec![];file.take(limit+1).read_to_end(&mut bytes).ok()?;(bytes.len()as u64<=limit).then_some(bytes)}
fn ready()->&'static Mutex<Option<Prepared>>{READY.get_or_init(||{
    let loaded=(||{let saved:ReadyReceipt=serde_json::from_slice(&read_small(&settings::root().join("update-download-ready.json"),4096)?).ok()?;
        if saved.schema!=1||!version(&saved.version)||!crate::core::newer_version(&saved.version,env!("CARGO_PKG_VERSION"))||saved.job.split('-').count()!=2||!saved.job.bytes().all(|b|b.is_ascii_digit()||b==b'-'){return None}
        let job=settings::root().join("updates").join(saved.job);if !job.join("verified.json").is_file()||job.join("cancelled.json").exists(){return None}Some(Prepared{job,version:saved.version,_lease:None})})();Mutex::new(loaded)
})}

fn view()->&'static Mutex<Progress>{VIEW.get_or_init(||{let saved=read_small(&settings::root().join("update-install-result.json"),16384).and_then(|bytes|serde_json::from_slice::<Progress>(&bytes).ok());Mutex::new(saved.unwrap_or_default())})}
pub fn snapshot()->Progress{
    let mut state=view().lock().unwrap_or_else(|e|e.into_inner()).clone();
    if !state.busy{let package_version=ready().lock().unwrap_or_else(|e|e.into_inner()).as_ref().map(|package|package.version.clone());if let Some(version)=package_version{state.phase="ready".into();state.version=Some(version);}else if let Some(bytes)=read_small(&settings::root().join("update-install-result.json"),16384){if let Ok(result)=serde_json::from_slice::<Progress>(&bytes){if result.completed_at>=state.started_at&&result.completed_at>state.completed_at&&matches!(result.phase.as_str(),"installed"|"rolled_back"|"error"|"installed_cleanup_pending"){state=result;}}}}
    state
}

pub fn busy()->bool{GATE.lock().unwrap_or_else(|e|e.into_inner()).updating}
enum LeaseKind{Management,Download,Install}
pub struct Lease{kind:LeaseKind}
impl Drop for Lease{fn drop(&mut self){let mut gate=GATE.lock().unwrap_or_else(|e|e.into_inner());match self.kind{LeaseKind::Management=>gate.management=gate.management.saturating_sub(1),LeaseKind::Download=>{gate.downloading=false;view().lock().unwrap_or_else(|e|e.into_inner()).busy=false;},LeaseKind::Install=>{gate.updating=false;view().lock().unwrap_or_else(|e|e.into_inner()).busy=false;}}}}
pub fn management()->Result<Lease,String>{let mut gate=GATE.lock().map_err(|_|"操作状态暂不可用。")?;if gate.updating{return Err("助手正在交接安装，请等待重启完成。".into())}gate.management+=1;Ok(Lease{kind:LeaseKind::Management})}
fn begin(version:&str,operation:&str)->Result<Lease,String>{let mut gate=GATE.lock().map_err(|_|"操作状态暂不可用。")?;if gate.updating||gate.downloading||operation=="install"&&gate.management!=0{return Err("另一项管理操作仍在进行，请稍后更新助手。".into())}let kind=if operation=="download"{gate.downloading=true;LeaseKind::Download}else{gate.updating=true;LeaseKind::Install};*view().lock().unwrap_or_else(|e|e.into_inner())=Progress{started_at:settings::now(),operation:operation.into(),busy:true,phase:if operation=="download"{"downloading"}else{"verifying"}.into(),version:Some(version.into()),..Progress::default()};Ok(Lease{kind})}
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
    if !settings::load(&settings::root()).is_ok_and(|config|config.network_enabled){return Err("联网检测已关闭，更新下载已停止。".into())}
    let mut response=client.get(url).send().map_err(|_|"无法连接 GitHub，请检查本地代理。")?.error_for_status().map_err(|_|"官方更新附件暂不可用，请稍后重试。")?;
    if response.content_length().is_some_and(|n|n>limit){return Err("更新附件超过允许大小。".into())}
    let mut file=fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(|_|"无法创建下载文件。")?;
    if report{view().lock().unwrap_or_else(|e|e.into_inner()).total=response.content_length();}
    let mut checked=Instant::now();let mut bytes=0u64;let mut buffer=[0u8;65536];loop{if checked.elapsed()>=Duration::from_millis(250){if !settings::load(&settings::root()).is_ok_and(|config|config.network_enabled){return Err("联网检测已关闭，更新下载已取消。".into())}checked=Instant::now();}let n=response.read(&mut buffer).map_err(|_|"更新下载中断，尚未替换程序。")?;if n==0{break}bytes+=n as u64;if bytes>limit{return Err("更新附件超过允许大小。".into())}file.write_all(&buffer[..n]).map_err(|_|"更新下载无法写入磁盘。")?;if report{view().lock().unwrap_or_else(|e|e.into_inner()).downloaded=bytes;}}
    if response.content_length().is_some_and(|n|n!=bytes){return Err("更新附件下载不完整。".into())}file.sync_all().map_err(|_|"更新附件未能完整保存。".into())
}
fn powershell()->PathBuf{PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(||"C:\\Windows".into())).join("System32/WindowsPowerShell/v1.0/powershell.exe")}
fn command(job:&Path,phase:&str)->Command{let mut command=Command::new(powershell());command.args(["-NoLogo","-NoProfile","-NonInteractive","-ExecutionPolicy","Bypass","-File"]).arg(job.join("installer.ps1")).args(["-Phase",phase,"-Job"]).arg(job).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).creation_flags(0x08000000);command}
fn wait_prepare(child:&mut Child,job:&Path)->Result<(),String>{let deadline=Instant::now()+Duration::from_secs(45);loop{match child.try_wait(){Ok(Some(status))=>return if status.success(){Ok(())}else{Err(read_small(&job.join("result.json"),16384).and_then(|bytes|serde_json::from_slice::<serde_json::Value>(&bytes).ok()).and_then(|value|value["error"].as_str().map(|text|text.chars().take(1000).collect::<String>())).unwrap_or_else(||"安装包或安装目录校验未通过；原程序和恢复资料已保留。".into()))},Ok(None)=>{},Err(_)=>return Err("无法读取安装校验结果。".into())}if Instant::now()>=deadline{let _=child.kill();let _=child.wait();return Err("安装校验超时，原程序未替换。".into())}thread::sleep(Duration::from_millis(100));}}
pub struct Prepared{job:PathBuf,version:String,_lease:Option<Lease>}
impl Drop for Prepared{fn drop(&mut self){if self._lease.is_some(){let _=fs::write(self.job.join("cancelled.json"),b"{}");let mut state=view().lock().unwrap_or_else(|e|e.into_inner());state.phase="error".into();state.error=Some("更新交接未完成，助手继续运行。".into());}}}

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
    let lease=begin(&expected,"download")?;
    let result:Result<PathBuf,String>=(||{
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
        phase("verifying");let mut child=command(&job,"Prepare").spawn().map_err(|_|"无法启动安装包校验。")?;wait_prepare(&mut child,&job)?;
        if !settings::load(&root).is_ok_and(|c|c.network_enabled){return Err("联网检测已关闭，更新已停止，原程序保持运行。".into())}
        Ok(job)
    })();
    match result{Ok(job)=>Ok(Prepared{job,version:expected,_lease:Some(lease)}),Err(error)=>{let mut state=view().lock().unwrap_or_else(|e|e.into_inner());state.phase="error".into();state.error=Some(error.clone());Err(error)}}
}

pub fn download_update(expected:String)->Result<Progress,String>{
    let existing=ready().lock().map_err(|_|"更新包状态暂不可用。")?.as_ref().map(|package|package.version.clone());
    if let Some(existing)=existing{if existing==expected{return Ok(snapshot())}return Err(format!("已有已校验的 {existing} 更新包，请先完成重启更新。"))}
    let mut package=prepare(expected.clone())?;
    let receipt=ReadyReceipt{schema:1,job:package.job.file_name().ok_or("更新目录无效。")?.to_string_lossy().into(),version:expected};settings::write_json(&settings::root().join("update-download-ready.json"),&receipt)?;
    drop(package._lease.take());*ready().lock().map_err(|_|"更新包状态暂不可用。")?=Some(package);phase("ready");Ok(snapshot())
}
pub fn install_ready(expected:String)->Result<Prepared,String>{
    if !version(&expected)||!crate::core::newer_version(&expected,env!("CARGO_PKG_VERSION")){return Err("目标版本无效，请重新检查更新。".into())}
    let lease=begin(&expected,"install")?;phase("verifying");
    let mut package={let mut current=ready().lock().map_err(|_|"更新包状态暂不可用。")?;if current.as_ref().is_none_or(|package|package.version!=expected){return Err("请先下载并校验此版本的更新包。".into())}current.take().unwrap()};package._lease=Some(lease);
    let mut plan:serde_json::Value=serde_json::from_slice(&read_small(&package.job.join("plan.json"),262144).ok_or("下载记录不可读取，已保留原程序。")?).map_err(|_|"下载记录无效。")?;
    let exe=std::env::current_exe().map_err(|_|"无法核验当前助手。")?;
    if plan["schema"]!=1||plan["product"]!="CodexProxyTray"||plan["version"]!=expected||plan["executable"].as_str().is_none_or(|path|settings::normalized_executable(Path::new(path))!=settings::normalized_executable(&exe)){return Err("安装目录或下载记录已变化，未开始替换。".into())}
    let identity=unsafe{crate::traffic::identity_on(windows::Win32::System::Threading::GetCurrentProcess())}.map_err(|_|"无法核验助手进程。")?;plan["parent_pid"]=serde_json::json!(std::process::id());plan["parent_created"]=serde_json::json!(identity.created_filetime.to_string());settings::write_json(&package.job.join("plan.json"),&plan)?;
    package.handoff()
}

#[cfg(test)]mod tests{use super::*;#[test]fn only_canonical_versions_and_exact_unique_asset_hashes_are_accepted(){assert!(version("0.2.1"));for bad in ["v0.2.1","../0.2.1","0.2.1-beta","01.2.1"]{assert!(!version(bad));}let hash="a".repeat(64);assert_eq!(checksum(&format!("{hash}  package.zip\n"),"package.zip").unwrap(),hash);assert!(checksum(&format!("{hash}  package.zip\n{hash}  package.zip"),"package.zip").is_err());assert!(checksum(&format!("{hash}  ../package.zip"),"package.zip").is_err());assert!(checksum("bogus  package.zip","package.zip").is_err());}}
