use serde::{Deserialize,Serialize};
use std::{collections::BTreeSet,fs,net::TcpStream,path::Path,sync::{Arc,Mutex,mpsc::{self,Sender}},thread::{self,JoinHandle},time::{Duration,Instant}};
use crate::{core,native,platform,settings::{self,Config,LaunchTarget}};

const LOG_LIMIT:usize=200;
#[derive(Clone,Serialize,Deserialize)] pub struct Log {pub id:u64,pub at:u64,pub level:String,pub category:String,pub message:String,pub details:String}
#[derive(Serialize,Deserialize)] struct History {schema:u32,records:Vec<Log>}
#[derive(Clone,Serialize)] pub struct Probe {at:u64,latency_ms:Option<f64>}
#[derive(Clone,Serialize)]pub struct TrafficSample{pub at:u64,pub receive_bps:Option<f64>,pub send_bps:Option<f64>,pub received_bytes:Option<u64>,pub sent_bytes:Option<u64>}
#[derive(Clone,Serialize)] pub struct Status {
    pub revision:u64,pub enabled:bool,pub guard_ready:bool,pub proxy_url:String,pub proxy_reachable:bool,
    pub package_count:usize,pub package_version:Option<String>,pub codex_running:bool,pub restart_required:bool,
    pub cli_detected:bool,pub cli_ready:bool,pub autostart:bool,pub launch_desktop:bool,pub backend_ready:bool,
    pub error:Option<String>,pub logs:Vec<Log>,pub helper_pid:u32,pub updated_at:u64,pub started_at:u64,
    pub proxy_latency_ms:Option<f64>,pub probe_history:Vec<Probe>,pub desktop_pid:Option<u32>,pub tcp_connections:Option<usize>,pub helper_memory_bytes:Option<u64>,pub log_storage_error:Option<String>,
    pub launch_target:LaunchTarget,pub cli_workdir:String,pub network_enabled:bool,pub auto_check_updates:bool,pub update_interval_hours:u32,pub update:crate::update::UpdateState,
    pub guard_uptime_ms:u64,pub connectivity_enabled:bool,pub connectivity:crate::connectivity::Connectivity,
    pub traffic:crate::traffic::TrafficSnapshot,
    pub traffic_history:Vec<TrafficSample>,
    pub desktop_operation:crate::desktop_lifecycle::OperationState,pub cli_management:crate::cli_manager::CliState,
    pub update_notice:crate::update_notice::NoticeSnapshot,pub app_update:crate::app_update::Progress,
    pub desktop_proxy:crate::desktop_proxy::ProxyState,
    pub traffic_enabled:bool,pub traffic_service:crate::traffic_service::ServiceSnapshot,
}
impl Status {
    fn new()->Self {Self{revision:0,enabled:true,guard_ready:false,proxy_url:core::DEFAULT_PROXY.into(),proxy_reachable:false,package_count:0,package_version:None,codex_running:false,restart_required:false,cli_detected:false,cli_ready:false,autostart:platform::autostart(),launch_desktop:false,backend_ready:true,error:None,logs:vec![],helper_pid:std::process::id(),updated_at:settings::now(),started_at:settings::now(),proxy_latency_ms:None,probe_history:vec![],desktop_pid:None,tcp_connections:None,helper_memory_bytes:None,log_storage_error:None,launch_target:LaunchTarget::None,cli_workdir:String::new(),network_enabled:true,auto_check_updates:false,update_interval_hours:6,update:crate::update::snapshot(),guard_uptime_ms:0,connectivity_enabled:true,connectivity:crate::connectivity::snapshot(),traffic:crate::traffic::TrafficSnapshot::disabled(),traffic_history:vec![],desktop_operation:crate::desktop_lifecycle::snapshot(),cli_management:crate::cli_manager::snapshot(),update_notice:crate::update_notice::snapshot(),app_update:crate::app_update::snapshot(),desktop_proxy:crate::desktop_proxy::state(None,core::DEFAULT_PROXY,false),traffic_enabled:false,traffic_service:crate::traffic_service::snapshot()}}
    fn sample_traffic(&mut self){
        use crate::traffic::TrafficState;
        let sample=&self.traffic;
        if sample.state==TrafficState::Disabled{self.traffic_history.clear();return}
        if sample.timestamp_ms==0||self.traffic_history.last().is_some_and(|last|last.at==sample.timestamp_ms){return}
        let previous=self.traffic_history.iter().rev().find(|point|point.received_bytes.is_some()||point.sent_bytes.is_some());
        let reset=previous.is_some_and(|point|matches!((sample.received_bytes,point.received_bytes),(Some(current),Some(before))if current<before)||matches!((sample.sent_bytes,point.sent_bytes),(Some(current),Some(before))if current<before));
        if reset||self.traffic_history.last().is_some_and(|point|point.at>sample.timestamp_ms){self.traffic_history.clear();}
        let collecting=sample.state==TrafficState::Collecting;
        let rate=|value:Option<f64>|value.filter(|value|value.is_finite()&&*value>=0.0).filter(|_|collecting);
        self.traffic_history.push(TrafficSample{at:sample.timestamp_ms,receive_bps:rate(sample.receive_bps),send_bps:rate(sample.send_bps),received_bytes:sample.received_bytes.filter(|_|collecting),sent_bytes:sample.sent_bytes.filter(|_|collecting)});
        if self.traffic_history.len()>60{self.traffic_history.remove(0);}
    }
    fn log(&mut self,level:&str,category:&str,message:impl Into<String>,details:impl Into<String>) {
        let at=settings::now();let id=self.logs.last().map_or(at,|last|last.id.saturating_add(1).max(at));
        self.logs.push(Log{id,at,level:level.into(),category:category.into(),message:message.into(),details:details.into()});
        if self.logs.len()>LOG_LIMIT {self.logs.remove(0);}
    }
}
type Reply=Sender<Result<Status,String>>;
#[derive(Default)]pub struct Preferences {pub autostart:Option<bool>,pub launch_target:Option<LaunchTarget>,pub cli_workdir:Option<String>,pub network_enabled:Option<bool>,pub auto_check_updates:Option<bool>,pub update_interval_hours:Option<u32>,pub connectivity_enabled:Option<bool>}
pub enum Message {Enabled(bool,Option<Reply>),TrafficEnabled(bool,Reply),ServicePause(Reply),ServiceFinished(bool,Reply),Notice(String,String,String,String),Proxy(String,Reply),System(Preferences,Reply),Refresh(Reply),Launch(Reply),LaunchCli(Reply),Stop}
pub struct Controller {pub status:Arc<Mutex<Status>>,pub tx:Sender<Message>,pub traffic:Arc<Mutex<crate::traffic::Coordinator>>,worker:Mutex<Option<JoinHandle<()>>>}
impl Controller {
    pub fn new(notify:impl Fn(&Status)+Send+'static)->Self {
        let(tx,rx)=mpsc::channel();let status=Arc::new(Mutex::new(Status::new()));let shared=status.clone();let traffic=Arc::new(Mutex::new(crate::traffic::Coordinator::new()));let sampler=traffic.clone();
        let worker_tx=tx.clone();let worker=thread::spawn(move|| {
            let root=settings::root();let _=fs::create_dir_all(&root);
            let (mut config,mut config_error)=match settings::load(&root) {Ok(c)=>(c,None),Err(e)=>(Config::default(),Some(format!("配置读取失败：{e}")))};
            let mut status=Status::new();
            let _=fs::create_dir_all(root.join("logs"));let history_path=root.join("logs/events-v1.json");
            let history_writable=match load_history(&history_path) {Ok(records)=>{status.logs=records;true},Err(error)=>{status.log_storage_error=Some(error);false}};
            let mut persisted=status.logs.last().map_or(0,|log|log.id);
            if let Some(error)=&config_error {status.enabled=false;status.error=Some(error.clone());}
            let exe=std::env::current_exe().expect("executable path");
            let debug=native::DebugSettings::new();let event=platform::GuardEvent::new(&settings::event_name(&root));
            let mut held=BTreeSet::<String>::new();let mut applied_url=String::new();let mut guard_started=Instant::now();
            status.log("info","生命周期","守护进程已启动",format!("版本 {} · PID {}\n程序：{}\n配置：{}",env!("CARGO_PKG_VERSION"),status.helper_pid,exe.display(),root.display()));
            status.log("info","配置","已读取启动配置",format!("代理：{}\n登录自动启动：{} · 启动目标：{}",config.proxy_url,yes(status.autostart),target_name(config.launch_target)));
            if let Err(error)=platform::remove_legacy_cli_path(&root.join("cli")){status.log("warning","CLI","旧版命令入口的 PATH 项未能移除",error);}
            let mut cli_error=None;let mut first=true;let restarting=std::env::args().any(|arg|arg=="--after-update");let mut auto_desktop_pending=!restarting&&config.launch_target.opens_desktop()&&config_error.is_none();let mut auto_cli_pending=!restarting&&config.launch_target.opens_cli()&&config_error.is_none();let mut cli_checked=Instant::now();let mut update_seen=status.update.checked_at;let mut traffic_applied=false;let mut service_paused=false;
            loop {
                let(mut pending,mut result,mut stopping,mut guard_action,mut manual)=(None,Ok(()),false,false,None);let mut traffic_requested=false;
                if !first {match rx.recv_timeout(Duration::from_secs(2)) {
                    Ok(Message::Enabled(enabled,reply))=>{pending=reply;if enabled&&config_error.is_some(){result=Err("请先保存有效的代理设置。".into());}else{guard_action=true;if enabled&&!status.enabled {guard_started=Instant::now();status.guard_uptime_ms=0;}if status.enabled&&!enabled {status.guard_uptime_ms=guard_started.elapsed().as_millis()as u64;}status.enabled=enabled;status.log("info","守护",if enabled{"已启用代理守护"}else{"已暂停代理守护"},"作用于 ChatGPT 与 Windows Codex CLI；已运行进程保留启动时的环境。");}},
                    Ok(Message::TrafficEnabled(enabled,reply))=>{pending=Some(reply);if config_error.is_some(){result=Err("配置无法读取，请先修复配置。".into());}else{let mut next=config.clone();next.traffic_enabled=enabled;match settings::save(&root,&next){Ok(())=>{config=next;traffic_requested=true;},Err(error)=>result=Err(error)}}},
                    Ok(Message::ServicePause(reply))=>{pending=Some(reply);service_paused=true;traffic_requested=true;},
                    Ok(Message::ServiceFinished(uninstalled,reply))=>{pending=Some(reply);service_paused=false;traffic_requested=true;if uninstalled{config.traffic_enabled=false;if config_error.is_none(){if let Err(error)=settings::save(&root,&config){status.log("warning","配置","辅助服务已卸载，但统计偏好未能保存",error);}}else{status.log("warning","配置","辅助服务已卸载，原配置文件已保留","配置原本无法读取，未覆盖它；本次运行已停止统计。");}}},
                    Ok(Message::Notice(level,category,message,details))=>status.log(&level,&category,message,details),
                    Ok(Message::Proxy(url,reply))=>{pending=Some(reply);match core::endpoint(&url) {Err(e)=>result=Err(e),Ok(endpoint)=>{let mut next=config.clone();next.proxy_url=endpoint.url;match settings::save(&root,&next) {Err(e)=>result=Err(e),Ok(())=>{config=next;config_error=None;guard_action=true;status.log("info","代理","代理配置已保存",format!("地址：{}\n后续启动的目标进程使用新配置。",config.proxy_url));}}}}},
                    Ok(Message::System(patch,reply))=>{pending=Some(reply);if config_error.is_some(){result=Err("请先保存有效的代理设置。".into());}else{result=apply_preferences(&root,&exe,&mut config,&patch);if result.is_ok(){status.log("info","设置","偏好设置已保存",format!("登录自动启动：{}\n启动目标：{}\n更新联网：{} · 自动检查：{} · 间隔：{} 小时\nCLI 目录：{}",yes(platform::autostart()),target_name(config.launch_target),yes(config.network_enabled),yes(config.auto_check_updates),config.update_interval_hours,if config.cli_workdir.is_empty(){"用户目录"}else{"已配置"}));}}},
                    Ok(Message::Refresh(reply))=>{pending=Some(reply);manual=Some(Instant::now());status.log("info","检测","开始手动检测","检测本地代理、ChatGPT 安装包及进程、Windows Codex CLI 入口。");if crate::cli_manager::is_mutating(){result=Err("Codex CLI 正在更新或卸载，入口将在操作结束后自动检测。".into());}else if config_error.is_none(){match install_cli(&root,&exe,&mut config){Ok(ready)=>{status.cli_ready=ready;cli_error=None;},Err(e)=>{status.cli_ready=false;cli_error=Some(format!("CLI 接入失败：{e}"));result=Err(e);}}}else{result=Err("配置无法读取，请先保存有效的代理设置。".into());}status.cli_detected=!settings::detect_cli(&root).is_empty();},
                    Ok(Message::Launch(reply))=>{pending=Some(reply);result=native::open_desktop();if result.is_ok(){status.log("info","ChatGPT","已请求打开 ChatGPT",format!("目标应用包：{}\n代理守护：{}",core::FAMILY,yes(status.guard_ready)));}},
                    Ok(Message::LaunchCli(reply))=>{pending=Some(reply);result=if crate::cli_manager::is_mutating(){Err("Codex CLI 正在更新或卸载，完成后再打开新 CLI。".into())}else{crate::launch::open_cli(&root,&config)};if result.is_ok(){status.log("info","CLI","已请求在终端打开 Codex CLI",format!("专用入口：{}\n工作目录：{}\n代理守护：{}",root.join("cli/codex-proxy-cli.exe").display(),if config.cli_workdir.is_empty(){"用户目录"}else{"已配置"},yes(status.enabled&&status.cli_ready)));}},
                    Ok(Message::Stop)|Err(mpsc::RecvTimeoutError::Disconnected)=>stopping=true,
                    Err(mpsc::RecvTimeoutError::Timeout)=>{},
                }}
                if stopping {
                    crate::flow_log::flush();
                    if let Ok(mut sampler)=sampler.lock(){if let Err(error)=sampler.disable(){status.log("warning","流量","流量采样仍在收尾",error);}status.traffic=sampler.snapshot();}
                    if let Ok(e)=&event {e.set(false);}
                    if let Ok(d)=&debug {for package in &held {if let Err(error)=d.disable(package){status.log("error","守护","释放应用包设置失败",format!("{package}\n{error}"));}}}
                    drop(debug);drop(event);status.enabled=false;status.guard_ready=false;status.backend_ready=false;
                    status.log("info","生命周期","助手已退出","已关闭 CLI 守护标志并释放本进程持有的应用包接口。");
                    publish(&root,&shared,&mut status,&mut persisted,history_writable,&notify);break;
                }
                status.proxy_url=config.proxy_url.clone();status.launch_desktop=config.launch_target.opens_desktop();status.autostart=platform::autostart();status.launch_target=config.launch_target;status.cli_workdir=config.cli_workdir.clone();status.network_enabled=config.network_enabled;status.auto_check_updates=config.auto_check_updates;status.update_interval_hours=config.update_interval_hours;
                status.connectivity_enabled=config.connectivity_enabled;if status.enabled {status.guard_uptime_ms=guard_started.elapsed().as_millis()as u64;}
                let check_at=Instant::now();
                let connection=core::endpoint(&config.proxy_url).ok().is_some_and(|endpoint|TcpStream::connect_timeout(&endpoint.socket,Duration::from_millis(400)).is_ok());
                status.proxy_latency_ms=if connection {Some(check_at.elapsed().as_secs_f64()*1000.0)}else{None};
                status.probe_history.push(Probe{at:settings::now(),latency_ms:status.proxy_latency_ms});if status.probe_history.len()>60 {status.probe_history.remove(0);}
                if first||connection!=status.proxy_reachable {status.log(if connection{"info"}else{"warning"},"代理",if connection{"本地代理连接成功"}else{"本地代理暂不可连接"},format!("地址：{}\n检测：TCP 端口连接；超时上限 400 ms。{}",config.proxy_url,if connection{format!("耗时 {:.2} ms。",status.proxy_latency_ms.unwrap())}else{"将持续检测。".into()}));}
                status.proxy_reachable=connection;
                if !first&&config_error.is_none()&&!crate::cli_manager::is_mutating()&&cli_checked.elapsed()>=Duration::from_secs(10) {
                    let _=thread::Builder::new().name("cli-installation-refresh".into()).spawn(crate::cli_manager::refresh_local);
                    cli_checked=Instant::now();let detected=!settings::detect_cli(&root).is_empty();
                    if !detected {cli_error=None;}
                    if !detected&&status.cli_detected {status.cli_ready=false;status.log("warning","CLI","Windows Codex CLI 已不可用","将继续检测；重新安装后自动准备专用入口。");}
                    status.cli_detected=detected;
                    if detected&&!status.cli_ready {match install_cli(&root,&exe,&mut config){Ok(ready)=>{status.cli_ready=ready;cli_error=None;if ready{status.log("info","CLI","CLI 专用入口已恢复",format!("入口：{}\n代理守护：{}",root.join("cli/codex-proxy-cli.exe").display(),yes(status.enabled)));}},Err(error)=>{let message=format!("CLI 接入失败：{error}");if cli_error.as_ref()!=Some(&message){status.log("warning","CLI","CLI 入口稍后重试",message.clone());}cli_error=Some(message);}}}
                }
                let mut error=config_error.clone().or_else(||cli_error.clone());status.guard_ready=false;
                if !status.enabled {if let Ok(d)=&debug {for package in held.iter().cloned().collect::<Vec<_>>() {match d.disable(&package){Ok(())=>{held.remove(&package);},Err(e)=>error=Some(e)}}}}
                match native::packages() {Err(e)=>error=Some(e),Ok(packages)=>{
                    let old_version=status.package_version.clone();status.package_count=packages.len();status.package_version=packages.last().map(|p|p.version.clone());
                    if first||old_version!=status.package_version {status.log("info","ChatGPT",if packages.is_empty(){"未检测到 ChatGPT"}else{"已识别 ChatGPT 安装包"},format!("应用包：{}\n安装数量：{} · 版本：{}",core::FAMILY,packages.len(),status.package_version.as_deref().unwrap_or("未安装")));}
                    let desired=if status.enabled {packages.iter().map(|p|p.full_name.clone()).collect::<BTreeSet<_>>()}else{BTreeSet::new()};
                    if let Ok(d)=&debug {
                        if applied_url!=config.proxy_url {for package in held.iter().cloned().collect::<Vec<_>>() {match d.disable(&package){Ok(())=>{held.remove(&package);},Err(e)=>error=Some(e)}}if held.is_empty(){applied_url=config.proxy_url.clone();}}
                        for package in held.difference(&desired).cloned().collect::<Vec<_>>() {match d.disable(&package){Ok(())=>{held.remove(&package);},Err(e)=>error=Some(e)}}
                        let block=core::environment_block(&config.proxy_url);
                        if applied_url==config.proxy_url {for package in desired.difference(&held).cloned().collect::<Vec<_>>() {match d.enable(&package,&exe.to_string_lossy(),&block){Ok(())=>{held.insert(package.clone());status.log("info","守护","ChatGPT 桌面端守护已就绪",format!("安装包：{package}\n代理：{}\n范围：该应用包后续启动的进程。",config.proxy_url));},Err(e)=>error=Some(e)}}}
                    }else if let Err(e)=&debug {error=Some(e.clone());}
                    status.guard_ready=status.enabled&&applied_url==config.proxy_url&&!desired.is_empty()&&desired.is_subset(&held);
                }}
                if let Ok(event)=&event {event.set(status.enabled&&status.cli_ready);}else if let Err(e)=&event {error=Some(e.clone());}
                let desktop=native::desktop_info();let pid=desktop.as_ref().map(|d|d.pid);
                if first||pid!=status.desktop_pid {status.log("info","ChatGPT",if pid.is_some(){"ChatGPT 正在运行"}else{"ChatGPT 当前未运行"},pid.map_or("等待应用启动；守护将持续生效。".into(),|pid|format!("主进程 PID：{pid}\n进程树数量：{}",desktop.as_ref().unwrap().pids.len())));}
                status.desktop_pid=pid;status.codex_running=desktop.is_some();
                status.desktop_proxy=crate::desktop_proxy::state(desktop.as_ref(),&config.proxy_url,status.guard_ready);status.restart_required=status.desktop_proxy.state=="restart_required";
                status.tcp_connections=native::tcp_connections(desktop.as_ref().map_or(&[],|d|d.pids.as_slice()));status.helper_memory_bytes=native::memory_bytes();
                if let Ok(mut sampler)=sampler.lock(){
                    let mut targets_valid=true;
                    let desired_traffic=config.traffic_enabled&&config_error.is_none()&&!service_paused;
                    if desired_traffic{match native::traffic_targets_held(&root,&config){Ok(targets)=>{if let Err(error)=sampler.set_targets(targets){targets_valid=false;sampler.fail_targets(error);}},Err(error)=>{targets_valid=false;sampler.fail_targets(format!("无法验证本轮目标进程：{error}"));}}}
                    if traffic_requested||desired_traffic!=traffic_applied{
                        if desired_traffic&&!targets_valid{result=Err("无法验证目标进程，未开始新的流量采样。".into());}
                        else{let action=if desired_traffic{sampler.enable(crate::traffic::EnableMode::Normal)}else{sampler.disable()};traffic_applied=desired_traffic;if let Err(error)=action{result=Err(error)}}
                    }
                    let next=sampler.snapshot();if next.state!=status.traffic.state{status.log(if next.state==crate::traffic::TrafficState::Unavailable{"warning"}else{"info"},"流量","流量采集状态已变化",format!("状态：{:?}\n{}",next.state,next.message.as_deref().unwrap_or("仅统计已验证的目标进程字节。")));}status.traffic=next;
                }
                crate::flow_log::maybe_sample(&status.traffic);status.traffic_enabled=config.traffic_enabled;status.traffic_service=crate::traffic_service::snapshot();
                if status.error!=error {if let Some(e)=&error {status.log("error","守护","守护状态需要检查",e.clone());}else if status.error.is_some(){status.log("info","守护","守护状态已恢复","本轮检测未再出现此前错误。");}status.error=error;}
                if !config.launch_target.opens_desktop() {auto_desktop_pending=false;}
                if !config.launch_target.opens_cli() {auto_cli_pending=false;}
                if auto_desktop_pending&&status.guard_ready {match native::open_desktop(){Ok(())=>status.log("info","ChatGPT","已按启动偏好请求打开 ChatGPT",format!("应用包：{}",core::FAMILY)),Err(e)=>status.log("error","ChatGPT","打开 ChatGPT 失败",e)}auto_desktop_pending=false;}
                if auto_cli_pending&&status.cli_ready&&!crate::cli_manager::is_mutating() {match crate::launch::open_cli(&root,&config){Ok(())=>status.log("info","CLI","已按启动偏好请求打开 Codex CLI",format!("工作目录：{}",if config.cli_workdir.is_empty(){"用户目录"}else{"已配置"})),Err(e)=>status.log("error","CLI","打开 Codex CLI 失败",e)}auto_cli_pending=false;}
                crate::update::maybe_automatic(&config);status.update=crate::update::snapshot();status.app_update=crate::app_update::snapshot();
                crate::connectivity::maybe_automatic(&config);status.connectivity=crate::connectivity::snapshot();
                crate::pricing::maybe_automatic(&config,&worker_tx);
                if status.update.checked_at!=update_seen {
                    update_seen=status.update.checked_at;
                    if status.update.status=="available"&&!status.update.checking {
                        let update=status.update.clone();let notice_tx=worker_tx.clone();
                        // Toast registration/delivery must not delay guard work or its request channel.
                        if thread::Builder::new().name("update-notice-dispatch".into()).spawn(move||match crate::update_notice::maybe_notify(&update){Ok(true)=>{let _=notice_tx.send(Message::Notice("info".into(),"更新".into(),"已向 Windows 提交更新提醒".into(),"实际横幅是否显示取决于 Windows 通知设置与专注模式。".into()));},Ok(false)=>{},Err(error)=>{let _=notice_tx.send(Message::Notice("warning".into(),"更新".into(),"Windows 更新提醒未完成".into(),error));}}).is_err(){status.log("warning","更新","Windows 更新提醒未能启动","可在设置中查看已检测到的新版本。");}
                    }
                    let update=status.update.clone();status.log(if update.error.is_some(){"warning"}else{"info"},"更新","更新检查已完成",update.error.unwrap_or_else(||match update.status.as_str(){"available"=>format!("发现新版本 {}",update.latest.unwrap_or_default()),"current"=>"当前已是最新版本。".into(),"unpublished"=>"尚未发布 CodexProxyTray 正式安装包。".into(),_=>"本次检查未产生可用发布信息。".into()}));
                }
                if let Some(started)=manual {status.log(if result.is_ok()&&status.error.is_none(){"info"}else{"warning"},"检测","手动检测完成",format!("耗时：{} ms\n代理端口：{} · ChatGPT 包：{} · CLI 入口：{}\nChatGPT PID：{}",started.elapsed().as_millis(),if status.proxy_reachable{"可用"}else{"不可用"},status.package_count,if status.cli_ready{"已接入"}else{"未接入"},pid.map_or("未运行".into(),|p|p.to_string())));}
                if guard_action&&result.is_ok(){if let Some(error)=&status.error {result=Err(format!("设置已处理，但守护需要检查：{error}"));}}
                if let Err(error)=&result {status.log("error","操作","操作未完成",error.clone());}
                publish(&root,&shared,&mut status,&mut persisted,history_writable,&notify);
                if let Some(reply)=pending {let _=reply.send(result.map(|_|status.clone()));}
                if first&&config_error.is_none()&&!crate::cli_manager::is_mutating() {
                    match install_cli(&root,&exe,&mut config) {Ok(ready)=>{status.cli_ready=ready;status.log(if ready{"info"}else{"warning"},"CLI",if ready{"Codex CLI 专用入口已就绪"}else{"未检测到 Windows Codex CLI"},if ready{format!("专用入口：{}\n实际程序：{}\n从助手打开 CLI 时使用内部代理入口；普通终端保留官方 npm 入口。",root.join("cli/codex-proxy-cli.exe").display(),config.cli_command.join(" "))}else{"安装后将自动检测并准备专用入口。".into()});},Err(e)=>{let message=format!("CLI 接入失败：{e}");status.log("error","CLI","CLI 入口准备失败",message.clone());cli_error=Some(message.clone());if status.error.is_none(){status.error=Some(message);}}}
                    status.cli_detected=!settings::detect_cli(&root).is_empty();if let Ok(event)=&event {event.set(status.enabled&&status.cli_ready);}
                    publish(&root,&shared,&mut status,&mut persisted,history_writable,&notify);
                }
                first=false;
            }
        });Self{status,tx,traffic,worker:Mutex::new(Some(worker))}
    }
    pub fn snapshot(&self)->Status {let finished=self.worker.lock().unwrap_or_else(|e|e.into_inner()).as_ref().is_some_and(JoinHandle::is_finished);let mut status=self.status.lock().unwrap_or_else(|e|e.into_inner()).clone();status.update=crate::update::snapshot();status.app_update=crate::app_update::snapshot();status.connectivity=crate::connectivity::snapshot();status.traffic_service=crate::traffic_service::snapshot();status.desktop_operation=crate::desktop_lifecycle::snapshot();status.cli_management=crate::cli_manager::snapshot();status.update_notice=crate::update_notice::snapshot();if let Ok(traffic)=self.traffic.lock(){status.traffic=traffic.snapshot();}status.sample_traffic();if finished&&status.backend_ready {status.backend_ready=false;status.enabled=false;status.guard_ready=false;status.error=Some("守护后台已退出，请重新打开助手。".into());}status}
    pub fn stop(&self){let _=self.tx.send(Message::Stop);if let Some(worker)=self.worker.lock().unwrap_or_else(|e|e.into_inner()).take(){let _=worker.join();}}
}
impl Drop for Controller {fn drop(&mut self){self.stop();}}
fn yes(value:bool)->&'static str {if value{"开启"}else{"关闭"}}
fn target_name(target:LaunchTarget)->&'static str {match target{LaunchTarget::None=>"不打开",LaunchTarget::Chatgpt=>"ChatGPT",LaunchTarget::Cli=>"Codex CLI",LaunchTarget::Both=>"ChatGPT 与 Codex CLI"}}
fn apply_preferences(root:&Path,exe:&Path,config:&mut Config,patch:&Preferences)->Result<(),String>{
    let mut next=config.clone();if let Some(target)=patch.launch_target{next.launch_target=target;}if let Some(directory)=&patch.cli_workdir{settings::validate_cli_workdir(directory)?;next.cli_workdir=directory.trim().into();}if let Some(enabled)=patch.network_enabled{next.network_enabled=enabled;}if let Some(automatic)=patch.auto_check_updates{next.auto_check_updates=automatic;}if let Some(hours)=patch.update_interval_hours{settings::validate_update_interval(hours)?;next.update_interval_hours=hours;}if let Some(enabled)=patch.connectivity_enabled{next.connectivity_enabled=enabled;}
    let old=platform::autostart();if let Some(enabled)=patch.autostart{platform::set_autostart(enabled,exe)?;}
    if let Err(error)=settings::save(root,&next){if patch.autostart.is_some(){let _=platform::set_autostart(old,exe);}return Err(error)}*config=next;Ok(())
}
fn load_history(path:&Path)->Result<Vec<Log>,String> {if !path.exists(){return Ok(vec![])}if fs::metadata(path).map_err(|e|e.to_string())?.len()>1024*1024{return Err("历史日志超过读取上限，原文件已保留。".into())}let history:History=serde_json::from_slice(&fs::read(path).map_err(|e|e.to_string())?).map_err(|_|"历史日志格式无效，原文件已保留。")?;if history.schema!=1{return Err("历史日志版本不受支持，原文件已保留。".into())}if history.records.iter().any(|log|!core::valid_log_position(log.id,log.at))||history.records.windows(2).any(|pair|pair[0].id>=pair[1].id){return Err("历史日志编号或时间无效，原文件已保留。".into())}Ok(history.records.into_iter().rev().take(LOG_LIMIT).collect::<Vec<_>>().into_iter().rev().collect())}
fn publish(root:&Path,shared:&Mutex<Status>,status:&mut Status,persisted:&mut u64,history_writable:bool,notify:&impl Fn(&Status)) {
    status.desktop_operation=crate::desktop_lifecycle::snapshot();status.cli_management=crate::cli_manager::snapshot();status.update_notice=crate::update_notice::snapshot();status.sample_traffic();
    let newest=status.logs.last().map_or(0,|log|log.id);
    if history_writable&&newest!=*persisted {match settings::write_json(&root.join("logs/events-v1.json"),&History{schema:1,records:status.logs.clone()}) {Ok(())=>{*persisted=newest;status.log_storage_error=None;},Err(error)=>status.log_storage_error=Some(error)}}
    status.revision+=1;status.updated_at=settings::now();*shared.lock().unwrap_or_else(|e|e.into_inner())=status.clone();
    // Confirmation identity and installation paths are IPC-only; runtime detail
    // endpoints and usage records never enter Status at all.
    let mut saved=status.clone();saved.cli_management.target_token=None;saved.cli_management.install_path=None;
    let _=settings::write_json(&root.join("status.json"),&saved);notify(status);
}
fn install_cli(root:&Path,exe:&Path,config:&mut Config)->Result<bool,String> {
    if crate::cli_manager::is_mutating(){return Err("Codex CLI 管理操作正在进行，暂缓更改专用入口。".into())}
    let detected=settings::detect_cli(root);if detected.is_empty(){return Ok(false)}
    let source=exe.parent().unwrap().join("codex-proxy-cli.exe");if !source.is_file(){return Err("缺少 codex-proxy-cli.exe，请保留完整程序目录。".into())}
    let directory=root.join("cli");fs::create_dir_all(&directory).map_err(|e|e.to_string())?;let target=directory.join("codex-proxy-cli.exe");let bytes=fs::read(source).map_err(|e|e.to_string())?;
    if fs::read(&target).ok().as_deref()!=Some(bytes.as_slice()){let pending=directory.join("codex-proxy-cli.exe.pending");fs::write(&pending,&bytes).map_err(|e|e.to_string())?;if let Err(error)=fs::rename(&pending,&target){let _=fs::remove_file(&pending);return Err(error.to_string());}}
    config.cli_command=detected;settings::save(root,config)?;platform::remove_legacy_cli_path(&directory)?;if config.cli_command.len()==2{if let Some(appdata)=std::env::var_os("APPDATA"){platform::ensure_user_cli_path(&std::path::PathBuf::from(appdata).join("npm"))?;}}Ok(true)
}

#[cfg(test)]mod traffic_history_tests{
    use super::*;
    use crate::traffic::{TrafficSnapshot,TrafficState};
    fn traffic(at:u64,state:TrafficState,bytes:Option<u64>,rate:Option<f64>)->TrafficSnapshot{let mut sample=TrafficSnapshot::disabled();sample.timestamp_ms=at;sample.state=state;sample.received_bytes=bytes;sample.sent_bytes=bytes;sample.receive_bps=rate;sample.send_bps=rate;sample}
    #[test]fn traffic_samples_keep_real_timestamps_gaps_and_new_session_boundaries(){
        let mut status=Status::new();status.traffic=traffic(100,TrafficState::Collecting,Some(1000),Some(10.0));status.sample_traffic();status.sample_traffic();assert_eq!(status.traffic_history.len(),1);
        status.traffic=traffic(200,TrafficState::Unavailable,Some(2000),Some(20.0));status.sample_traffic();assert_eq!(status.traffic_history[1].receive_bps,None);assert_eq!(status.traffic_history[1].received_bytes,None);
        status.traffic=traffic(300,TrafficState::Collecting,Some(3000),Some(30.0));status.sample_traffic();assert_eq!(status.traffic_history.len(),3);assert_eq!(status.traffic_history[2].at,300);
        status.traffic=traffic(400,TrafficState::Collecting,Some(10),Some(1.0));status.sample_traffic();assert_eq!(status.traffic_history.len(),1);assert_eq!(status.traffic_history[0].received_bytes,Some(10));
        status.traffic=traffic(500,TrafficState::Disabled,None,None);status.sample_traffic();assert!(status.traffic_history.is_empty());
    }
    #[test]fn history_is_bounded_and_invalid_rates_do_not_become_measured_zero(){
        let mut status=Status::new();for at in 1..=100{status.traffic=traffic(at,TrafficState::Collecting,Some(at*100),Some(at as f64));status.sample_traffic();}assert_eq!(status.traffic_history.len(),60);assert_eq!(status.traffic_history[0].at,41);
        status.traffic=traffic(101,TrafficState::Collecting,Some(10100),Some(f64::NAN));status.sample_traffic();assert_eq!(status.traffic_history.last().unwrap().receive_bps,None);
        status.traffic=traffic(90,TrafficState::Collecting,Some(10200),Some(10.0));status.sample_traffic();assert_eq!(status.traffic_history.len(),1,"A backwards timestamp starts a separate graph segment.");
    }
}
