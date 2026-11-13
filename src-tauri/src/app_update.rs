//! Explicit, same-user update of this assistant only. The detached installer has
//! no URLs or commands from the frontend, and never elevates or kills user tasks.
use std::{fs,io::{Read,Write},os::windows::{fs::MetadataExt,process::CommandExt},path::{Path,PathBuf},process::{Child,Command,Stdio},sync::{Mutex,OnceLock},thread,time::{Duration,Instant}};
use serde::{Serialize,Deserialize};
use crate::settings;

const RELEASES:&str="https://github.com/OrangeZh1/CodexProxyTray/releases";
const SCRIPT:&str=include_str!("install_update.ps1");
const MAX_ZIP:u64=128*1024*1024;
const FAILURE_FILE:&str="update-download-failure-v1.json";
const FAILURE_PENDING:&str="update-download-failure-v1.pending.json";
#[derive(Clone,Default,Serialize,Deserialize)]#[serde(default)]
pub struct Progress{pub started_at:u64,pub completed_at:u64,pub operation:String,pub busy:bool,pub phase:String,pub version:Option<String>,pub downloaded:u64,pub total:Option<u64>,pub error:Option<String>}
#[derive(Default)]struct Gate{updating:bool,downloading:bool,management:u32}
static GATE:Mutex<Gate>=Mutex::new(Gate{updating:false,downloading:false,management:0});
static VIEW:OnceLock<Mutex<Progress>>=OnceLock::new();
static READY:OnceLock<Mutex<Option<Prepared>>>=OnceLock::new();
#[derive(Serialize,Deserialize)]struct ReadyReceipt{schema:u32,job:String,version:String}
#[derive(Clone,Copy,Serialize,Deserialize)]#[serde(rename_all="snake_case")]enum DownloadStage{Configuration,Client,Workspace,Script,Checksums,Manifest,Package,Plan,Prepare,Network,ReadyReceipt}
#[derive(Serialize,Deserialize)]#[serde(deny_unknown_fields)]struct DownloadFailure{schema:u32,product:String,started_at:u64,completed_at:u64,version:String,phase:String,error:String,downloaded:u64,total:Option<u64>,job:Option<String>,failure_stage:DownloadStage}
impl DownloadFailure{
    fn valid(&self,now:u64)->bool{self.schema==1&&self.product=="CodexProxyTray"&&self.phase=="error"&&self.started_at>0&&self.completed_at>=self.started_at&&self.completed_at<=now.saturating_add(300_000)&&version(&self.version)&&!self.error.trim().is_empty()&&self.error.chars().count()<=1500&&self.downloaded<=MAX_ZIP&&self.total.is_none_or(|total|total<=MAX_ZIP)&&self.job.as_ref().is_none_or(|job|job.len()<=64&&job.split('-').count()==2&&job.split('-').all(|part|!part.is_empty()&&part.bytes().all(|byte|byte.is_ascii_digit())))}
    fn progress(&self)->Progress{Progress{started_at:self.started_at,completed_at:self.completed_at,operation:"download".into(),busy:false,phase:self.phase.clone(),version:Some(self.version.clone()),downloaded:self.downloaded,total:self.total,error:Some(self.error.clone())}}
}
fn read_failure(path:&Path,now:u64)->Result<Option<DownloadFailure>,String>{
    let metadata=match fs::symlink_metadata(path){Ok(metadata)=>metadata,Err(error)if error.kind()==std::io::ErrorKind::NotFound=>return Ok(None),Err(_)=>return Err("最近下载结果无法读取，原文件已保留。".into())};
    if !metadata.is_file()||metadata.file_attributes()&0x400!=0||metadata.len()>16384{return Err("最近下载结果超出读取范围，原文件已保留。".into())}
    let bytes=read_small(path,16384).ok_or("最近下载结果无法读取，原文件已保留。")?;let record:DownloadFailure=serde_json::from_slice(&bytes).map_err(|_|"最近下载结果格式无效，原文件已保留。")?;
    if !record.valid(now){return Err("最近下载结果版本或时间无效，原文件已保留。".into())}Ok(Some(record))
}
fn load_failure(root:&Path,now:u64)->Result<Option<DownloadFailure>,String>{
    let complete=read_failure(&root.join(FAILURE_FILE),now)?;let pending=read_failure(&root.join(FAILURE_PENDING),now)?;
    Ok(match(complete,pending){(Some(complete),Some(pending))=>Some(if pending.completed_at>complete.completed_at{pending}else{complete}),(complete,pending)=>complete.or(pending)})
}
fn save_failure(root:&Path,record:&DownloadFailure,now:u64)->Result<(),String>{
    if !record.valid(now){return Err("本次下载结果无效，未覆盖已有记录。".into())}if load_failure(root,now)?.is_some_and(|previous|previous.completed_at>record.completed_at){return Err("已有更近期的下载结果，未覆盖它。".into())}
    let bytes=serde_json::to_vec(record).map_err(|_|"下载结果编码失败。")?;if bytes.len()>16384{return Err("下载结果超过保存上限。".into())}
    let pending=root.join(FAILURE_PENDING);let mut file=fs::OpenOptions::new().write(true).create_new(true).open(&pending).map_err(|_|"下载结果有未完成写入或目录不可写，原文件已保留。")?;file.write_all(&bytes).and_then(|_|file.sync_all()).map_err(|_|"下载结果未能完整保存，恢复材料已保留。")?;drop(file);
    if load_failure(root,now)?.is_some_and(|previous|previous.completed_at>record.completed_at){return Err("已有更近期的下载结果，未覆盖它。".into())}
    fs::rename(pending,root.join(FAILURE_FILE)).map_err(|_|"下载结果替换失败，恢复材料已保留。".into())
}
fn prefer_completed(install:Progress,failure:Option<Progress>)->Progress{match failure{Some(failure)if failure.completed_at>install.completed_at=>failure,_=>install}}
fn saved_progress(root:&Path)->Result<Progress,String>{
    let now=settings::now();let install=read_small(&root.join("update-install-result.json"),16384).and_then(|bytes|serde_json::from_slice::<Progress>(&bytes).ok()).filter(|progress|!progress.busy&&progress.completed_at>0&&progress.completed_at>=progress.started_at&&progress.completed_at<=now.saturating_add(300_000)&&progress.version.as_deref().is_some_and(version)&&matches!(progress.phase.as_str(),"installed"|"rolled_back"|"error"|"installed_cleanup_pending")).unwrap_or_default();
    Ok(prefer_completed(install,load_failure(root,now)?.filter(|failure|crate::core::newer_version(&failure.version,env!("CARGO_PKG_VERSION"))).map(|failure|failure.progress())))
}
fn read_small(path:&Path,limit:u64)->Option<Vec<u8>>{let file=fs::File::open(path).ok()?;let mut bytes=vec![];file.take(limit+1).read_to_end(&mut bytes).ok()?;(bytes.len()as u64<=limit).then_some(bytes)}
fn ready()->&'static Mutex<Option<Prepared>>{READY.get_or_init(||{
    let loaded=(||{let saved:ReadyReceipt=serde_json::from_slice(&read_small(&settings::root().join("update-download-ready.json"),4096)?).ok()?;
        if saved.schema!=1||!version(&saved.version)||!crate::core::newer_version(&saved.version,env!("CARGO_PKG_VERSION"))||saved.job.split('-').count()!=2||!saved.job.bytes().all(|b|b.is_ascii_digit()||b==b'-'){return None}
        let job=settings::root().join("updates").join(saved.job);if !job.join("verified.json").is_file()||job.join("cancelled.json").exists(){return None}Some(Prepared{job,version:saved.version,_lease:None})})();Mutex::new(loaded)
})}

fn view()->&'static Mutex<Progress>{VIEW.get_or_init(||Mutex::new(saved_progress(&settings::root()).unwrap_or_else(|error|Progress{phase:"error".into(),error:Some(error),..Progress::default()})))}
pub fn snapshot()->Progress{
    let mut state=view().lock().unwrap_or_else(|e|e.into_inner()).clone();
    if !state.busy{let package_version=ready().lock().unwrap_or_else(|e|e.into_inner()).as_ref().map(|package|package.version.clone());if let Some(version)=package_version{state.phase="ready".into();state.version=Some(version);state.error=None;}else if let Ok(result)=saved_progress(&settings::root()){if result.completed_at>=state.started_at&&result.completed_at>state.completed_at&&matches!(result.phase.as_str(),"installed"|"rolled_back"|"error"|"installed_cleanup_pending"){state=result;}}}
    state
}

pub fn busy()->bool{GATE.lock().unwrap_or_else(|e|e.into_inner()).updating}
enum LeaseKind{Management,Download,Install}
pub struct Lease{kind:LeaseKind}
impl Drop for Lease{fn drop(&mut self){let mut gate=GATE.lock().unwrap_or_else(|e|e.into_inner());match self.kind{LeaseKind::Management=>gate.management=gate.management.saturating_sub(1),LeaseKind::Download=>{gate.downloading=false;view().lock().unwrap_or_else(|e|e.into_inner()).busy=false;},LeaseKind::Install=>{gate.updating=false;view().lock().unwrap_or_else(|e|e.into_inner()).busy=false;}}}}
pub fn management()->Result<Lease,String>{let mut gate=GATE.lock().map_err(|_|"操作状态暂不可用。")?;if gate.updating{return Err("助手正在交接安装，请等待重启完成。".into())}gate.management+=1;Ok(Lease{kind:LeaseKind::Management})}
fn begin(version:&str,operation:&str)->Result<Lease,String>{let mut gate=GATE.lock().map_err(|_|"操作状态暂不可用。")?;if gate.updating||gate.downloading||operation=="install"&&gate.management!=0{return Err("另一项管理操作仍在进行，请稍后更新助手。".into())}let kind=if operation=="download"{gate.downloading=true;LeaseKind::Download}else{gate.updating=true;LeaseKind::Install};*view().lock().unwrap_or_else(|e|e.into_inner())=Progress{started_at:settings::now(),operation:operation.into(),busy:true,phase:if operation=="download"{"downloading"}else{"verifying"}.into(),version:Some(version.into()),..Progress::default()};Ok(Lease{kind})}
fn phase(value:&str){view().lock().unwrap_or_else(|e|e.into_inner()).phase=value.into();}
fn download_failed(error:String,job:Option<String>,failure_stage:DownloadStage)->String{
    let mut state=view().lock().unwrap_or_else(|e|e.into_inner());state.phase="error".into();state.busy=false;state.completed_at=settings::now();state.error=Some(error.chars().take(1000).collect());
    if let Some(version)=&state.version{let record=DownloadFailure{schema:1,product:"CodexProxyTray".into(),started_at:state.started_at,completed_at:state.completed_at,version:version.clone(),phase:"error".into(),error:state.error.clone().unwrap_or_default(),downloaded:state.downloaded,total:state.total,job,failure_stage};let root=settings::root();let history=save_job_failure(&root,&record);let recent=save_failure(&root,&record,state.completed_at);if let Err(storage)=recent{state.error=Some(format!("{error} {storage} 重开后可能无法显示本次失败，请保留更新目录。"));}else if let Err(storage)=history{state.error=Some(format!("{error} {storage}"));}}
    state.error.clone().unwrap_or(error)
}
fn save_job_failure(root:&Path,record:&DownloadFailure)->Result<(),String>{
    if !record.valid(settings::now()){return Err("本次更新历史结果无效，未覆盖已有文件。".into())}
    let Some(job)=&record.job else{return Ok(())};let path=root.join("updates").join(job);let metadata=fs::symlink_metadata(&path).map_err(|_|"本次更新目录无法读取，历史结果未写入。")?;
    if !metadata.is_dir()||metadata.file_attributes()&0x400!=0{return Err("本次更新目录身份变化，历史结果未写入。".into())}
    let mut file=fs::OpenOptions::new().write(true).create_new(true).open(path.join("download-failure.json")).map_err(|_|"本次更新历史结果已存在或不可写，原文件已保留。")?;
    let bytes=serde_json::to_vec(record).map_err(|_|"本次更新历史结果编码失败。")?;file.write_all(&bytes).and_then(|_|file.sync_all()).map_err(|_|"本次更新历史结果未完整保存，目录已保留。".into())
}
fn version(value:&str)->bool{!value.starts_with('v')&&crate::core::version_parts(value).is_some()}
fn checksum(text:&str,name:&str)->Result<String,String>{
    let mut found=None;for line in text.lines(){let Some((hash,file))=line.split_once("  ")else{return Err("官方校验清单格式无效。".into())};if hash.len()!=64||!hash.bytes().all(|b|b.is_ascii_hexdigit())||file.contains(['/', '\\']){return Err("官方校验清单无效。".into())}if file==name{if found.is_some(){return Err("官方校验清单包含重复文件。".into())}found=Some(hash.to_ascii_lowercase());}}
    found.ok_or_else(||"官方校验清单缺少安装包。".into())
}
fn client(config:&settings::Config)->Result<reqwest::blocking::Client,String>{
    if !config.network_enabled{return Err("联网检测已关闭，未下载更新。".into())}
    reqwest::blocking::Client::builder().no_proxy().proxy(reqwest::Proxy::all(&config.proxy_url).map_err(|_|"本地代理设置无效。")?).connect_timeout(Duration::from_secs(12)).timeout(Duration::from_secs(180)).user_agent(concat!("CodexProxyTray/",env!("CARGO_PKG_VERSION")))
        .redirect(crate::update::official_redirect()).build().map_err(|_|"更新下载暂不可用。".into())
}
fn download(client:&reqwest::blocking::Client,url:&str,path:&Path,limit:u64,report:bool)->Result<(),String>{
    if !settings::load(&settings::root()).is_ok_and(|config|config.network_enabled){return Err("联网检测已关闭，更新下载已停止。".into())}
    let mut response=crate::update::official_get(client,url)?.error_for_status().map_err(|error|error.status().map_or_else(||"官方更新附件暂不可用，请稍后重试。".into(),|status|format!("官方更新附件暂不可用（HTTP {}），尚未替换程序。",status.as_u16())))?;
    let expected_length=response.content_length();
    if expected_length.is_some_and(|n|n>limit){return Err("更新附件超过允许大小。".into())}
    let mut file=fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(|_|"无法创建下载文件。")?;
    if report{view().lock().unwrap_or_else(|e|e.into_inner()).total=expected_length;}
    let mut checked=Instant::now();let mut bytes=0u64;let mut buffer=[0u8;65536];loop{if checked.elapsed()>=Duration::from_millis(250){if !settings::load(&settings::root()).is_ok_and(|config|config.network_enabled){return Err("联网检测已关闭，更新下载已取消。".into())}checked=Instant::now();}let n=response.read(&mut buffer).map_err(|_|"更新下载中断，尚未替换程序。")?;if n==0{break}bytes+=n as u64;if bytes>limit{return Err("更新附件超过允许大小。".into())}file.write_all(&buffer[..n]).map_err(|_|"更新下载无法写入磁盘。")?;if report{view().lock().unwrap_or_else(|e|e.into_inner()).downloaded=bytes;}}
    if expected_length.is_some_and(|n|n!=bytes){return Err("更新附件下载不完整。".into())}file.sync_all().map_err(|_|"更新附件未能完整保存。".into())
}
fn powershell()->PathBuf{PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(||"C:\\Windows".into())).join("System32/WindowsPowerShell/v1.0/powershell.exe")}
fn command(job:&Path,phase:&str)->Command{let mut command=Command::new(powershell());let inherited=std::env::var_os("PSModulePath");command.env("CODEX_PROXY_UPDATER_MODULES_PRESENT",if inherited.is_some(){"1"}else{"0"}).env("CODEX_PROXY_UPDATER_MODULES",inherited.unwrap_or_default());let policy=std::env::var_os("PSExecutionPolicyPreference");command.env("CODEX_PROXY_UPDATER_POLICY_PRESENT",if policy.is_some(){"1"}else{"0"}).env("CODEX_PROXY_UPDATER_POLICY",policy.unwrap_or_default());command.args(["-NoLogo","-NoProfile","-NonInteractive","-ExecutionPolicy","Bypass","-File"]).arg(job.join("installer.ps1")).args(["-Phase",phase,"-Job"]).arg(job).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).creation_flags(0x08000000);command}
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
    let mut job_name=None;let mut stage=DownloadStage::Configuration;
    let result:Result<PathBuf,String>=(||{
        let root=settings::root();let config=settings::load(&root).map_err(|_|"无法读取现有配置，未下载更新。")?;stage=DownloadStage::Client;let client=client(&config)?;stage=DownloadStage::Workspace;
        let job=root.join("updates").join(format!("{}-{}",std::process::id(),settings::now()));fs::create_dir_all(job.parent().unwrap()).and_then(|_|fs::create_dir(&job)).map_err(|_|"无法创建更新工作目录。")?;
        job_name=job.file_name().and_then(|name|name.to_str()).map(str::to_owned);stage=DownloadStage::Script;
        fs::write(job.join("installer.ps1"),format!("\u{feff}{SCRIPT}")).map_err(|_|"无法准备安装程序。")?;
        let base=format!("{RELEASES}/download/v{expected}");let name=format!("CodexProxyTray-v{expected}-windows-x64.zip");
        stage=DownloadStage::Checksums;download(&client,&format!("{base}/SHA256SUMS.txt"),&job.join("release-sums.txt"),65536,false).map_err(|error|format!("获取更新校验清单未完成：{error}"))?;
        let sums=fs::read_to_string(job.join("release-sums.txt")).map_err(|_|"官方校验清单不可读取。")?;let zip_hash=checksum(&sums,&name)?;let manifest_hash=checksum(&sums,"CodexProxyTray-update.json")?;
        stage=DownloadStage::Manifest;download(&client,&format!("{base}/CodexProxyTray-update.json"),&job.join("manifest.json"),65536,false).map_err(|error|format!("获取更新说明未完成：{error}"))?;
        stage=DownloadStage::Package;download(&client,&format!("{base}/{name}"),&job.join("package.zip"),MAX_ZIP,true).map_err(|error|format!("下载安装包未完成：{error}"))?;stage=DownloadStage::Plan;
        let exe=std::env::current_exe().map_err(|_|"无法确定当前安装位置。")?;
        let identity=unsafe{crate::traffic::identity_on(windows::Win32::System::Threading::GetCurrentProcess())}.map_err(|_|"无法核验助手进程。")?;
        settings::write_json(&job.join("plan.json"),&serde_json::json!({"schema":1,"product":"CodexProxyTray","version":expected,"zip_sha256":zip_hash,"manifest_sha256":manifest_hash,"executable":exe,"data_root":root,"parent_pid":std::process::id(),"parent_created":identity.created_filetime.to_string()}))?;
        stage=DownloadStage::Prepare;phase("verifying");let mut child=command(&job,"Prepare").spawn().map_err(|_|"无法启动安装包校验。")?;wait_prepare(&mut child,&job)?;stage=DownloadStage::Network;
        if !settings::load(&root).is_ok_and(|c|c.network_enabled){return Err("联网检测已关闭，更新已停止，原程序保持运行。".into())}
        Ok(job)
    })();
    match result{Ok(job)=>Ok(Prepared{job,version:expected,_lease:Some(lease)}),Err(error)=>Err(download_failed(error,job_name,stage))}
}

pub fn download_update(expected:String)->Result<Progress,String>{
    let existing=ready().lock().map_err(|_|"更新包状态暂不可用。")?.as_ref().map(|package|package.version.clone());
    if let Some(existing)=existing{if existing==expected{return Ok(snapshot())}return Err(format!("已有已校验的 {existing} 更新包，请先完成重启更新。"))}
    let mut package=prepare(expected.clone())?;
    let receipt=ReadyReceipt{schema:1,job:package.job.file_name().ok_or("更新目录无效。")?.to_string_lossy().into(),version:expected};if let Err(error)=settings::write_json(&settings::root().join("update-download-ready.json"),&receipt){let job=Some(receipt.job);drop(package);return Err(download_failed(error,job,DownloadStage::ReadyReceipt))}
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

#[cfg(test)]mod tests{
    use super::*;
    #[test]#[ignore="Requires a fresh isolated GitHub-hosted process and public HTTPS access"]
    fn published_update_download_probe(){
        const TEST:&str="app_update::tests::published_update_download_probe";const ZIP_HASH:&str="ce4cb875325eda9f0872ef304bd156c6baa99d0908b67021e79dc0952dc2ca2f";
        assert_eq!(std::env::var("GITHUB_ACTIONS").as_deref(),Ok("true"));assert_eq!(std::env::var("RUNNER_ENVIRONMENT").as_deref(),Ok("github-hosted"));let args=std::env::args().collect::<Vec<_>>();assert!(args.iter().any(|value|value==TEST)&&args.iter().any(|value|value=="--exact")&&args.iter().any(|value|value=="--test-threads=1"));
        let boundary=PathBuf::from(std::env::var_os("RUNNER_TEMP").expect("runner temp")).canonicalize().unwrap();let result=PathBuf::from(std::env::var_os("CODEX_PROXY_PUBLIC_DOWNLOAD_PROBE_RESULT").expect("explicit result path"));assert!(result.is_absolute()&&result.file_name()==Some(std::ffi::OsStr::new("public-download-probe.json"))&&result.parent().unwrap().canonicalize().unwrap().starts_with(&boundary));
        let mut output=fs::OpenOptions::new().write(true).create_new(true).open(result).expect("fresh public download evidence");let started=Instant::now();let root=boundary.join(format!("CodexProxyTray-public-download-{}-{}",std::process::id(),settings::now()));fs::create_dir(&root).unwrap();let local=root.join("local");let data=local.join("CodexProxyTray");fs::create_dir_all(&data).unwrap();
        struct Restore(Option<std::ffi::OsString>);impl Drop for Restore{fn drop(&mut self){match &self.0{Some(value)=>std::env::set_var("LOCALAPPDATA",value),None=>std::env::remove_var("LOCALAPPDATA")}}}
        let restore=Restore(std::env::var_os("LOCALAPPDATA"));std::env::set_var("LOCALAPPDATA",&local);
        let mut sums_verified=false;let mut manifest_verified=false;let mut failure=None;
        let outcome=(||->Result<(),String>{
            let config=settings::Config{network_enabled:true,auto_check_updates:false,connectivity_enabled:false,traffic_enabled:false,..settings::Config::default()};settings::save(&data,&config)?;
            let client=reqwest::blocking::Client::builder().no_proxy().connect_timeout(Duration::from_secs(12)).timeout(Duration::from_secs(60)).user_agent(concat!("CodexProxyTray/",env!("CARGO_PKG_VERSION"))).redirect(crate::update::official_redirect()).build().map_err(|_|"公开下载验收客户端无法创建。")?;
            let base="https://github.com/OrangeZh1/CodexProxyTray/releases/download/v0.2.2";
            download(&client,&format!("{base}/SHA256SUMS.txt"),&root.join("sums.txt"),16384,false)?;let sums=fs::read_to_string(root.join("sums.txt")).map_err(|_|"公开校验清单无法读取。")?;
            if checksum(&sums,"CodexProxyTray-v0.2.2-windows-x64.zip")?!=ZIP_HASH{return Err("固定 v0.2.2 ZIP 校验条目不匹配。".into())}sums_verified=true;
            download(&client,&format!("{base}/CodexProxyTray-update.json"),&root.join("manifest.json"),16384,false)?;let bytes=read_small(&root.join("manifest.json"),16384).ok_or("公开清单超过读取上限。")?;let manifest:serde_json::Value=serde_json::from_slice(&bytes).map_err(|_|"公开更新清单格式无效。")?;
            if manifest["schema"]!=1||manifest["product"]!="CodexProxyTray"||manifest["version"]!="0.2.2"{return Err("固定公开更新清单身份不匹配。".into())}manifest_verified=true;Ok(())
        })();if let Err(error)=outcome{failure=Some(error.chars().take(1000).collect::<String>());}drop(restore);
        let cleanup=(||->Result<(),String>{
            if !root.canonicalize().map_err(|_|"临时目录不可核验。")?.starts_with(&boundary){return Err("临时目录边界变化，已保留。".into())}
            for path in [data.join("config.json"),root.join("sums.txt"),root.join("manifest.json")]{if path.exists(){let metadata=fs::symlink_metadata(&path).map_err(|_|"临时文件不可核验。")?;if !metadata.is_file()||metadata.file_attributes()&0x400!=0{return Err("临时文件身份变化，已保留。".into())}fs::remove_file(path).map_err(|_|"临时文件清理未完成。")?;}}
            for path in [&data,&local,&root]{fs::remove_dir(path).map_err(|_|"临时目录含未确认材料，已保留。")?;}Ok(())
        })();let cleaned=cleanup.is_ok();if let Err(error)=cleanup{if failure.is_none(){failure=Some(error);}}
        let report=serde_json::json!({"schema":1,"status":if failure.is_none(){"passed"}else{"failed"},"scope":"Cloud direct fixed public HTTPS GET; no user proxy, accounts, install or local TLS-root diagnosis","reference_version":"0.2.2","zip_sha256_expected":ZIP_HASH,"sums_verified":sums_verified,"manifest_verified":manifest_verified,"cleanup":if cleaned{"passed"}else{"retained_for_diagnostics"},"elapsed_ms":started.elapsed().as_millis().min(u128::from(u64::MAX))as u64,"error":failure});let bytes=serde_json::to_vec(&report).unwrap();assert!(bytes.len()<=16384);output.write_all(&bytes).and_then(|_|output.sync_all()).expect("durable public download evidence");assert_eq!(report["status"],"passed","public download probe failed");
    }
    fn failure(now:u64)->DownloadFailure{DownloadFailure{schema:1,product:"CodexProxyTray".into(),started_at:now-100,completed_at:now,version:"9.0.0".into(),phase:"error".into(),error:"connection failed".into(),downloaded:0,total:None,job:Some("2-1".into()),failure_stage:DownloadStage::Checksums}}
    fn temporary(name:&str)->PathBuf{let suffix=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();let root=std::env::temp_dir().join(format!("codex-app-update-{name}-{}-{suffix}",std::process::id()));fs::create_dir(&root).unwrap();root}
    #[test]fn latest_download_failure_survives_reload_without_overwriting_install_history(){
        let root=temporary("failure-state");let now=settings::now();let installed=serde_json::json!({"schema":1,"phase":"installed","version":"0.2.1","completed_at":now-1000,"busy":false,"recovery":"old-owned-job"});let install_path=root.join("update-install-result.json");fs::write(&install_path,serde_json::to_vec(&installed).unwrap()).unwrap();let original=fs::read(&install_path).unwrap();
        fs::create_dir_all(root.join("updates/2-1")).unwrap();let record=failure(now);save_job_failure(&root,&record).unwrap();save_failure(&root,&record,now).unwrap();let reopened=saved_progress(&root).unwrap();assert_eq!(reopened.phase,"error");assert_eq!(reopened.version.as_deref(),Some("9.0.0"));assert_eq!(reopened.started_at,now-100);assert_eq!(reopened.completed_at,now);assert!(!reopened.busy);assert_eq!(fs::read(&install_path).unwrap(),original);
        let history=root.join("updates/2-1/download-failure.json");let history_bytes=fs::read(&history).unwrap();assert!(save_job_failure(&root,&record).is_err());assert_eq!(fs::read(&history).unwrap(),history_bytes);
        let failed_bytes=fs::read(root.join(FAILURE_FILE)).unwrap();fs::write(&install_path,serde_json::to_vec(&serde_json::json!({"schema":1,"phase":"installed","version":"9.0.0","completed_at":now+1,"busy":false})).unwrap()).unwrap();assert_eq!(saved_progress(&root).unwrap().phase,"installed");assert_eq!(fs::read(root.join(FAILURE_FILE)).unwrap(),failed_bytes);assert_eq!(fs::read(history).unwrap(),history_bytes);
        assert!(root.starts_with(std::env::temp_dir()));fs::remove_dir_all(root).unwrap();
    }
    #[test]fn failure_state_rejects_unknown_corrupt_and_incomplete_overwrites(){
        let root=temporary("failure-boundaries");let now=settings::now();let record=failure(now);let mut value=serde_json::to_value(&record).unwrap();value["unknown"]=serde_json::json!(true);let unknown=serde_json::to_vec(&value).unwrap();let path=root.join(FAILURE_FILE);fs::write(&path,&unknown).unwrap();assert!(load_failure(&root,now).is_err());assert!(save_failure(&root,&record,now).is_err());assert_eq!(fs::read(&path).unwrap(),unknown);
        for invalid in [b"{broken".to_vec(),vec![b'x';16385]]{fs::write(&path,&invalid).unwrap();assert!(save_failure(&root,&record,now).is_err());assert_eq!(fs::read(&path).unwrap(),invalid);}fs::remove_file(&path).unwrap();
        let pending=root.join(FAILURE_PENDING);let preserved=serde_json::to_vec(&record).unwrap();fs::write(&pending,&preserved).unwrap();assert_eq!(load_failure(&root,now).unwrap().unwrap().progress().phase,"error");assert!(save_failure(&root,&record,now).is_err());assert_eq!(fs::read(pending).unwrap(),preserved);
        let mut invalid=failure(now);invalid.started_at=now+1;assert!(!invalid.valid(now));invalid=failure(now);invalid.completed_at=now+300001;assert!(!invalid.valid(now));invalid=failure(now);invalid.job=Some("../private".into());assert!(!invalid.valid(now));
        assert!(root.starts_with(std::env::temp_dir()));fs::remove_dir_all(root).unwrap();
    }
#[test]fn only_canonical_versions_and_exact_unique_asset_hashes_are_accepted(){assert!(version("0.2.1"));for bad in ["v0.2.1","../0.2.1","0.2.1-beta","01.2.1"]{assert!(!version(bad));}let hash="a".repeat(64);assert_eq!(checksum(&format!("{hash}  package.zip\n"),"package.zip").unwrap(),hash);assert!(checksum(&format!("{hash}  package.zip\n{hash}  package.zip"),"package.zip").is_err());assert!(checksum(&format!("{hash}  ../package.zip"),"package.zip").is_err());assert!(checksum("bogus  package.zip","package.zip").is_err());}}
