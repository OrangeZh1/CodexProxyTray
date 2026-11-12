#![cfg_attr(windows, windows_subsystem = "windows")]
mod core;
#[cfg(windows)] mod native;
#[cfg(windows)] mod desktop_lifecycle;
#[cfg(windows)] mod desktop_proxy;
#[cfg(windows)] mod cli_manager;
#[cfg(windows)] mod runtime_details;
#[cfg(windows)] mod update_notice;
#[cfg(windows)] mod usage;
#[cfg(windows)] mod pricing;
#[cfg(windows)] mod exchange_rate;
#[cfg(windows)] mod event_log;
#[cfg(windows)] mod folder_picker;
#[cfg(windows)] mod settings;
#[cfg(windows)] mod platform;
#[cfg(windows)] mod worker;
#[cfg(windows)] mod update;
#[cfg(windows)] mod app_update;
#[cfg(windows)] mod flow_log;
#[cfg(windows)] mod launch;
#[cfg(windows)] mod connectivity;
#[cfg(windows)] mod traffic_core;
#[cfg(windows)] mod traffic_etw;
#[cfg(windows)] mod traffic;
#[cfg(windows)] mod traffic_service;

#[cfg(windows)] mod desktop {
    use tauri::{Manager,State,Emitter,tray::{TrayIconBuilder,TrayIconEvent,MouseButton,MouseButtonState}};
    use crate::worker::{Controller,Message,Status,Preferences};
    use std::sync::atomic::{AtomicBool,AtomicU32,Ordering};
    static SERVICE_OPERATION:AtomicBool=AtomicBool::new(false);
    struct ServiceOperation;
    impl Drop for ServiceOperation{fn drop(&mut self){SERVICE_OPERATION.store(false,Ordering::Release);}}
    struct UiStartup{ready:AtomicBool,show_requested:AtomicBool,page:std::sync::Mutex<Option<String>>,tray_ready:AtomicBool,tray_requested:AtomicBool,tray_height:AtomicU32,tray_point:std::sync::Mutex<Option<tauri::PhysicalPosition<f64>>>}
    fn show(app:&tauri::AppHandle){if let Some(startup)=app.try_state::<UiStartup>(){startup.show_requested.store(true,Ordering::Release);if !startup.ready.load(Ordering::Acquire){return;}}if let Some(window)=app.get_webview_window("main"){let _=window.show();let _=window.unminimize();let _=window.set_focus();}}
    fn show_page(app:&tauri::AppHandle,page:&str){if let Some(startup)=app.try_state::<UiStartup>(){let mut pending=startup.page.lock().unwrap_or_else(|e|e.into_inner());if !startup.ready.load(Ordering::Acquire){*pending=Some(page.into());drop(pending);show(app);return}}show(app);let _=app.emit("navigate",page);}
    fn show_tray(app:&tauri::AppHandle){if let Some(startup)=app.try_state::<UiStartup>(){if !startup.tray_ready.load(Ordering::Acquire){startup.tray_requested.store(true,Ordering::Release);return}startup.tray_requested.store(false,Ordering::Release);}if let Some(window)=app.get_webview_window("tray"){let _=window.show();let _=window.set_focus();}}
    fn position_tray(app:&tauri::AppHandle){if let(Some(window),Some(startup))=(app.get_webview_window("tray"),app.try_state::<UiStartup>()){let point=*startup.tray_point.lock().unwrap_or_else(|error|error.into_inner());if let Some(point)=point{if let Ok(Some(monitor))=window.monitor_from_point(point.x,point.y){let area=monitor.work_area();let scale=monitor.scale_factor();let height=startup.tray_height.load(Ordering::Acquire);let(x,y,width,height)=crate::core::popup_bounds((point.x.round()as i32,point.y.round()as i32),(area.position.x,area.position.y,area.size.width,area.size.height),((272.0*scale).round()as u32,(f64::from(height)*scale).round()as u32),(8.0*scale).round()as i32);let _=window.set_size(tauri::PhysicalSize::new(width,height));let _=window.set_position(tauri::PhysicalPosition::new(x,y));}}}}
    fn toggle_tray(app:&tauri::AppHandle,point:tauri::PhysicalPosition<f64>){if let Some(window)=app.get_webview_window("tray"){if window.is_visible().unwrap_or(false){let _=window.hide();return}if let Some(startup)=app.try_state::<UiStartup>(){*startup.tray_point.lock().unwrap_or_else(|error|error.into_inner())=Some(point);}position_tray(app);show_tray(app);}}
    fn tray_image()->tauri::image::Image<'static>{match crate::native::small_icon_size(){0..=16=>tauri::include_image!("icons/icon-16.png"),17..=20=>tauri::include_image!("icons/icon-20.png"),21..=24=>tauri::include_image!("icons/icon-24.png"),25..=32=>tauri::include_image!("icons/icon-32.png"),33..=40=>tauri::include_image!("icons/icon-40.png"),41..=48=>tauri::include_image!("icons/icon-48.png"),_=>tauri::include_image!("icons/icon-64.png")}}
    #[tauri::command] async fn get_status(state:State<'_,Controller>)->Result<Status,String>{Ok(state.snapshot())}
    #[tauri::command]fn report_ui_error(message:String,window:tauri::WebviewWindow,state:State<'_,Controller>)->Result<(),String>{let message=message.chars().take(2000).collect::<String>();let _=state.tx.send(Message::Notice("error".into(),"界面".into(),"界面加载失败".into(),message.clone()));crate::settings::write_json(&crate::settings::root().join("ui-error.json"),&serde_json::json!({"at":crate::settings::now(),"window":window.label(),"helper_pid":std::process::id(),"message":message}))}
    #[tauri::command]fn record_ui_event(event:crate::event_log::UiEvent,state:State<'_,Controller>)->Result<(),String>{crate::event_log::ui(&state.tx,event)}
    #[tauri::command]fn frontend_ready(rendered:Option<bool>,geometry:Option<serde_json::Value>,app:tauri::AppHandle,window:tauri::WebviewWindow,state:State<'_,UiStartup>)->Option<String>{if window.label()=="tray"{if let Some(height)=geometry.as_ref().and_then(|value|value.get("tray_height")).and_then(serde_json::Value::as_u64){state.tray_height.store(height.clamp(200,600)as u32,Ordering::Release);position_tray(&app);}}let _=crate::settings::write_json(&crate::settings::root().join(format!("ui-ready-{}.json",window.label())),&serde_json::json!({"at":crate::settings::now(),"window":window.label(),"rendered":rendered.unwrap_or(false),"helper_pid":std::process::id(),"geometry":geometry.filter(|value|value.to_string().len()<=32768)}));if window.label()=="tray"{state.tray_ready.store(true,Ordering::Release);if state.tray_requested.load(Ordering::Acquire){show_tray(&app)}return None}let mut pending=state.page.lock().unwrap_or_else(|e|e.into_inner());if !state.ready.swap(true,Ordering::AcqRel){if state.show_requested.load(Ordering::Acquire){show(&app);}return pending.take()}None}
    #[tauri::command]fn navigate_main(page:String,app:tauri::AppHandle)->Result<(),String>{if !matches!(page.as_str(),"dashboard"|"proxy"|"logs"|"settings"){return Err("未知页面。".into())}if let Some(window)=app.get_webview_window("tray"){let _=window.hide();}show_page(&app,&page);Ok(())}
    fn request_quit(app:&tauri::AppHandle)->Result<(),String>{
        if crate::app_update::busy(){return Err("助手正在交接更新，请等待重启完成。".into())}
        if crate::cli_manager::is_mutating(){return Err("Codex CLI 正在更新或卸载，请等待操作完成后退出助手。".into())}
        if crate::desktop_lifecycle::in_progress(){return Err("ChatGPT 关闭或重启操作仍在进行，请等待完成后退出助手。".into())}
        app.exit(0);Ok(())
    }
    #[tauri::command]fn quit_helper(app:tauri::AppHandle)->Result<(),String>{request_quit(&app)}
    async fn request(state:State<'_,Controller>,make:impl FnOnce(std::sync::mpsc::Sender<Result<Status,String>>)->Message+Send+'static)->Result<Status,String>{request_to(state.tx.clone(),make).await}
    async fn request_to(tx:std::sync::mpsc::Sender<Message>,make:impl FnOnce(std::sync::mpsc::Sender<Result<Status,String>>)->Message+Send+'static)->Result<Status,String>{let _lease=crate::app_update::management()?;tauri::async_runtime::spawn_blocking(move||{let(reply,rx)=std::sync::mpsc::channel();tx.send(make(reply)).map_err(|_|"后台已退出".to_owned())?;rx.recv_timeout(std::time::Duration::from_secs(8)).map_err(|_|"操作仍在处理中，请查看当前状态。".to_owned())?}).await.map_err(|e|e.to_string())?}
    #[tauri::command] async fn set_enabled(enabled:bool,state:State<'_,Controller>)->Result<Status,String>{request(state,move|r|Message::Enabled(enabled,Some(r))).await}
    #[tauri::command] async fn save_proxy(proxy_url:String,state:State<'_,Controller>)->Result<Status,String>{request(state,move|r|Message::Proxy(proxy_url,r)).await}
    #[tauri::command] async fn save_system(autostart:Option<bool>,launch_target:Option<crate::settings::LaunchTarget>,cli_workdir:Option<String>,network_enabled:Option<bool>,auto_check_updates:Option<bool>,update_interval_hours:Option<u32>,connectivity_enabled:Option<bool>,state:State<'_,Controller>)->Result<Status,String>{request(state,move|r|Message::System(Preferences{autostart,launch_target,cli_workdir,network_enabled,auto_check_updates,update_interval_hours,connectivity_enabled},r)).await}
    #[tauri::command]async fn choose_cli_workdir(initial_directory:Option<String>,window:tauri::WebviewWindow,state:State<'_,Controller>)->Result<Option<String>,String>{
        if window.label()!="main"{return Err("请在主窗口设置中选择工作目录。".into())}let owner=window.hwnd().map_err(|_|"无法取得设置主窗口。".to_owned())?.0 as usize;
        let _=crate::event_log::note(&state.tx,"info","CLI","收到工作目录选择请求","");let result=tauri::async_runtime::spawn_blocking(move||crate::folder_picker::choose(initial_directory,owner)).await.unwrap_or_else(|_|Err("文件夹选择未能完成。".into()));
        let _=crate::event_log::note(&state.tx,if result.is_err(){"warning"}else{"info"},"CLI",match &result{Ok(Some(_))=>"已选择 CLI 工作目录，尚未保存",Ok(None)=>"已取消 CLI 工作目录选择",Err(_)=>"CLI 工作目录选择未完成"},"");result
    }
    #[tauri::command] async fn refresh_status(state:State<'_,Controller>)->Result<Status,String>{request(state,Message::Refresh).await}
    #[tauri::command] async fn launch_codex(state:State<'_,Controller>)->Result<Status,String>{request(state,Message::Launch).await}
    #[tauri::command] async fn launch_cli(state:State<'_,Controller>)->Result<Status,String>{request(state,Message::LaunchCli).await}
    #[tauri::command]async fn capture_desktop_target(state:State<'_,Controller>)->Result<crate::desktop_lifecycle::Target,String>{
        let result=tauri::async_runtime::spawn_blocking(crate::desktop_lifecycle::capture).await.unwrap_or_else(|_|Err("无法读取当前 ChatGPT 实例，请重新打开确认框。".into()));
        crate::event_log::read_state(&state.tx,crate::event_log::ReadState::DesktopTarget,result.is_err(),"ChatGPT","ChatGPT 目标核验未完成","ChatGPT 目标核验已恢复");result
    }
    #[tauri::command]async fn manage_desktop(target:crate::desktop_lifecycle::Target,restart:bool,state:State<'_,Controller>)->Result<crate::desktop_lifecycle::Outcome,String>{
        let _lease=crate::app_update::management()?;
        let _=crate::event_log::note(&state.tx,"info","ChatGPT",if restart{"收到 ChatGPT 正常重启请求"}else{"收到 ChatGPT 正常关闭请求"},"");
        let result=tauri::async_runtime::spawn_blocking(move||crate::desktop_lifecycle::perform(target,restart)).await.unwrap_or_else(|_|Err("ChatGPT 操作未能完成，请查看当前状态；助手未强制结束应用。".into()));
        let _=crate::event_log::note(&state.tx,if result.is_ok(){"info"}else{"warning"},"ChatGPT",if result.is_err(){"ChatGPT 正常关闭或重启未完成"}else if restart{"已请求重新打开 ChatGPT"}else{"目标 ChatGPT 已正常退出"},"助手使用正常关闭请求；未强制结束进程。");result
    }
    fn log_cli_result(tx:&std::sync::mpsc::Sender<Message>,result:&Result<crate::cli_manager::CliState,String>,success:&'static str){let failed=result.as_ref().map_or(true,|view|view.error.is_some());let _=crate::event_log::note(tx,if failed{"warning"}else{"info"},"CLI",if failed{"Codex CLI 管理操作未完成"}else{success},"");}
    #[tauri::command]async fn check_cli_updates(state:State<'_,Controller>)->Result<crate::cli_manager::CliState,String>{
        let _=crate::event_log::note(&state.tx,"info","CLI","收到 Codex CLI 版本检查请求","");let result=tauri::async_runtime::spawn_blocking(crate::cli_manager::check).await.unwrap_or_else(|_|Err("CLI 更新检查未能完成。".into()));log_cli_result(&state.tx,&result,"Codex CLI 版本检查已结束");result
    }
    #[tauri::command]async fn update_cli(target_token:String,state:State<'_,Controller>)->Result<crate::cli_manager::CliState,String>{
        let _=crate::event_log::note(&state.tx,"info","CLI","收到 Codex CLI 更新请求","");let result=tauri::async_runtime::spawn_blocking(move||crate::cli_manager::update(target_token)).await.unwrap_or_else(|_|Err("CLI 更新操作未能完成，请先查看当前状态后再操作。".into()));log_cli_result(&state.tx,&result,"Codex CLI 更新操作已结束");result
    }
    #[tauri::command]async fn uninstall_cli(target_token:String,state:State<'_,Controller>)->Result<crate::cli_manager::CliState,String>{
        let _=crate::event_log::note(&state.tx,"info","CLI","收到 Codex CLI 卸载请求","");let result=tauri::async_runtime::spawn_blocking(move||crate::cli_manager::uninstall(target_token)).await.unwrap_or_else(|_|Err("CLI 卸载操作未能完成，请先查看当前状态后再操作。".into()));log_cli_result(&state.tx,&result,"Codex CLI 卸载操作已结束");result
    }
    #[tauri::command]async fn get_runtime_connections(state:State<'_,Controller>)->Result<crate::runtime_details::ConnectionSnapshot,String>{
        let result=tauri::async_runtime::spawn_blocking(crate::runtime_details::connections).await.unwrap_or_else(|_|Err("连接详情读取失败，请稍后重新打开。".into()));crate::event_log::read_state(&state.tx,crate::event_log::ReadState::Connections,result.is_err(),"连接","目标连接详情暂不可读取","目标连接详情读取已恢复");result
    }
    #[tauri::command]async fn get_runtime_memory(state:State<'_,Controller>)->Result<crate::runtime_details::MemorySnapshot,String>{
        let result=tauri::async_runtime::spawn_blocking(crate::runtime_details::memory).await.unwrap_or_else(|_|Err("内存详情读取失败，请稍后重新打开。".into()));crate::event_log::read_state(&state.tx,crate::event_log::ReadState::Memory,result.as_ref().map_or(true,|view|!view.complete),"内存","助手内存详情暂不可完整读取","助手内存详情读取已恢复");result
    }
    #[tauri::command]async fn get_usage(start_ms:u64,end_ms:u64,state:State<'_,Controller>)->Result<crate::usage::UsageSnapshot,String>{
        let result=tauri::async_runtime::spawn_blocking(move||crate::usage::read(start_ms,end_ms)).await.unwrap_or_else(|_|Err("用量统计暂不可用，请稍后重新打开。".into()));crate::event_log::read_state(&state.tx,crate::event_log::ReadState::Usage,result.as_ref().map_or(true,|view|view.partial),"用量","用量扫描暂不可用或当前仅为部分结果","用量扫描已恢复完整结果");result
    }
    #[tauri::command]async fn get_usage_prices(force:Option<bool>,state:State<'_,Controller>)->Result<crate::pricing::Snapshot,String>{
        let force=force.unwrap_or(false);if force{let _=crate::event_log::ui(&state.tx,crate::event_log::UiEvent::PricesSyncStarted);}
        let result=tauri::async_runtime::spawn_blocking(move||crate::pricing::read(force)).await.unwrap_or_else(|_|Err("价格同步暂不可用。".into()));
        if force{let failed=result.as_ref().map_or(true,|view|view.error.is_some());let _=crate::event_log::changed(crate::event_log::ReadState::Prices,failed);let _=crate::event_log::ui(&state.tx,if failed{crate::event_log::UiEvent::PricesSyncFailed}else{crate::event_log::UiEvent::PricesSyncSuccess});}
        else{crate::pricing::record_auto_result(&state.tx,&result);}result
    }
    #[tauri::command]async fn set_usage_price_sync(enabled:bool,state:State<'_,Controller>)->Result<crate::pricing::SyncSetting,String>{let result=tauri::async_runtime::spawn_blocking(move||crate::pricing::set_sync(enabled)).await.unwrap_or_else(|_|Err("参考价自动同步设置未能保存。".into()));let _=crate::event_log::note(&state.tx,if result.is_ok(){"info"}else{"warning"},"价格",if result.is_err(){"参考价自动同步设置未能保存"}else if enabled{"已启用模型参考价自动同步"}else{"已关闭模型参考价自动同步"},"");result}
    #[tauri::command]async fn reload_usage_prices(state:State<'_,Controller>)->Result<crate::pricing::Snapshot,String>{let result=tauri::async_runtime::spawn_blocking(crate::pricing::reload).await.unwrap_or_else(|_|Err("本机参考价文件未能重新载入。".into()));let failed=result.as_ref().map_or(true,|value|value.error.is_some());let _=crate::event_log::note(&state.tx,if failed{"warning"}else{"info"},"价格",if failed{"本机参考价文件未能重新载入"}else{"已重新载入本机参考价文件"},"");result}
    #[tauri::command]async fn open_usage_prices_dir(state:State<'_,Controller>)->Result<(),String>{let result=tauri::async_runtime::spawn_blocking(crate::pricing::open_directory).await.unwrap_or_else(|_|Err("本机价格目录未能打开。".into()));let _=crate::event_log::note(&state.tx,if result.is_ok(){"info"}else{"warning"},"价格",if result.is_ok(){"已请求打开本机价格目录"}else{"本机价格目录未能打开"},"");result}
    #[tauri::command]async fn get_usage_exchange_rate(state:State<'_,Controller>)->Result<crate::exchange_rate::Snapshot,String>{
        let result=tauri::async_runtime::spawn_blocking(crate::exchange_rate::read).await.unwrap_or_else(|_|Err("参考汇率查询暂不可用。".into()));
        if !result.as_ref().is_ok_and(|view|!view.network_enabled){crate::event_log::read_state(&state.tx,crate::event_log::ReadState::Exchange,result.as_ref().map_or(true,|view|view.error.is_some()||view.rate.is_none()),"汇率","USD/CNY 参考汇率自动获取未完成","USD/CNY 参考汇率当前可读取");}result
    }
    #[tauri::command]async fn download_app_update(expected_version:String,window:tauri::WebviewWindow)->Result<crate::app_update::Progress,String>{if window.label()!="main"{return Err("请从主窗口下载助手更新。".into())}tauri::async_runtime::spawn_blocking(move||crate::app_update::download_update(expected_version)).await.map_err(|_|"更新下载未能完成。".to_owned())?}
    #[tauri::command]async fn install_app_update(expected_version:String,window:tauri::WebviewWindow,app:tauri::AppHandle)->Result<(),String>{
        if window.label()!="main"{return Err("请从主窗口更新助手。".into())}
        let result=tauri::async_runtime::spawn_blocking(move||crate::app_update::install_ready(expected_version)).await.map_err(|_|"助手更新未能完成。".to_owned())??;
        std::mem::forget(result);app.exit(0);Ok(())
    }
    #[tauri::command]async fn install_cli(target_token:String,state:State<'_,Controller>)->Result<crate::cli_manager::CliState,String>{
        let result=tauri::async_runtime::spawn_blocking(move||crate::cli_manager::install(target_token)).await.unwrap_or_else(|_|Err("CLI 安装未能完成，请查看当前状态。".into()));log_cli_result(&state.tx,&result,"Codex CLI 安装已完成");result
    }
    #[tauri::command]async fn get_flow_logs()->Result<crate::flow_log::Snapshot,String>{tauri::async_runtime::spawn_blocking(||Ok(crate::flow_log::snapshot())).await.map_err(|_|"连接日志读取失败。".to_owned())?}
    #[tauri::command]async fn set_flow_logging(enabled:bool)->Result<crate::flow_log::Snapshot,String>{let _lease=crate::app_update::management()?;tauri::async_runtime::spawn_blocking(move||crate::flow_log::set_enabled(enabled)).await.map_err(|_|"连接日志设置未能保存。".to_owned())?}
    #[tauri::command] async fn check_updates()->Result<crate::update::Update,String>{tauri::async_runtime::spawn_blocking(crate::update::check).await.map_err(|_|"更新检查暂不可用。".to_owned())?}
    #[tauri::command] async fn check_connectivity(state:State<'_,Controller>)->Result<crate::connectivity::Connectivity,String>{let result=tauri::async_runtime::spawn_blocking(crate::connectivity::check).await.unwrap_or_else(|_|Err("连通检测暂不可用。".into()));crate::event_log::read_state(&state.tx,crate::event_log::ReadState::Connectivity,result.as_ref().map_or(true,|view|view.error.is_some()||matches!(view.status.as_str(),"blocked"|"error")),"连通","连通检测暂未取得可用结果","连通检测现可读取有效结果");result}
    #[tauri::command] async fn enable_traffic(state:State<'_,Controller>)->Result<Status,String>{request(state,|reply|Message::TrafficEnabled(true,reply)).await}
    #[tauri::command] async fn disable_traffic(state:State<'_,Controller>)->Result<Status,String>{request(state,|reply|Message::TrafficEnabled(false,reply)).await}
    async fn service_action(state:State<'_,Controller>,install:bool)->Result<crate::traffic_service::ServiceSnapshot,String>{
        let _lease=crate::app_update::management()?;
        if SERVICE_OPERATION.swap(true,Ordering::AcqRel){return Err("另一个辅助服务操作仍在进行。".into())}let _operation=ServiceOperation;let tx=state.tx.clone();
        if let Err(error)=request_to(tx.clone(),Message::ServicePause).await{let _=request_to(tx.clone(),|reply|Message::ServiceFinished(false,reply)).await;return Err(error)}
        let mut result=tauri::async_runtime::spawn_blocking(move||if install{crate::traffic_service::install()}else{crate::traffic_service::uninstall()}).await.unwrap_or_else(|_|Err("辅助服务操作未能完成。".into()));
        let removed=!install&&result.is_ok();if let Err(error)=request_to(tx.clone(),move|reply|Message::ServiceFinished(removed,reply)).await{if result.is_ok(){result=Err(error);}}
        let _=tx.send(Message::Notice(if result.is_ok(){"info"}else{"error"}.into(),"流量辅助服务".into(),if install{"辅助服务安装操作已结束"}else{"辅助服务卸载操作已结束"}.into(),result.as_ref().map(|_|"已重新读取 Windows 服务状态。".to_owned()).unwrap_or_else(|error|error.clone())));result
    }
    #[tauri::command]async fn install_traffic_service(state:State<'_,Controller>)->Result<crate::traffic_service::ServiceSnapshot,String>{service_action(state,true).await}
    #[tauri::command]async fn uninstall_traffic_service(state:State<'_,Controller>)->Result<crate::traffic_service::ServiceSnapshot,String>{service_action(state,false).await}
    #[tauri::command]fn open_release()->Result<(),String>{crate::native::open(crate::update::RELEASES)}
    #[tauri::command]fn open_source()->Result<(),String>{crate::native::open(crate::update::SOURCE)}
    #[tauri::command]fn open_logs()->Result<(),String>{let path=crate::settings::root().join("logs");std::fs::create_dir_all(&path).map_err(|e|e.to_string())?;crate::native::open(&path.to_string_lossy())}
    pub fn run(){
        let previous_hook=std::panic::take_hook();std::panic::set_hook(Box::new(move|info|{let root=crate::settings::root();let _=std::fs::create_dir_all(&root);let _=std::fs::write(root.join("startup-error.txt"),info.to_string());previous_hook(info);}));
        let quitting=std::env::args().any(|s|s=="--quit");
        let refreshing=std::env::args().any(|s|s=="--refresh");
        let background=quitting||refreshing||std::env::args().any(|s|s=="--background");
        let app=tauri::Builder::default().manage(UiStartup{ready:AtomicBool::new(false),show_requested:AtomicBool::new(!background),page:std::sync::Mutex::new(None),tray_ready:AtomicBool::new(false),tray_requested:AtomicBool::new(false),tray_height:AtomicU32::new(380),tray_point:std::sync::Mutex::new(None)}).plugin(tauri_plugin_single_instance::init(|app,args,_|{if args.iter().any(|arg|arg=="--quit"){if let Err(error)=request_quit(app){let _=app.state::<Controller>().tx.send(Message::Notice("warning".into(),"生命周期".into(),"退出助手已暂缓".into(),error));}}else if args.iter().any(|arg|arg=="--refresh"){let(tx,_)=std::sync::mpsc::channel();let _=app.state::<Controller>().tx.send(Message::Refresh(tx));}else if !args.iter().any(|arg|arg=="--background"){show(app);}}))
            .invoke_handler(tauri::generate_handler![get_status,report_ui_error,record_ui_event,frontend_ready,navigate_main,quit_helper,set_enabled,save_proxy,save_system,choose_cli_workdir,refresh_status,launch_codex,launch_cli,capture_desktop_target,manage_desktop,check_cli_updates,install_cli,update_cli,uninstall_cli,get_runtime_connections,get_runtime_memory,get_usage,get_usage_prices,set_usage_price_sync,reload_usage_prices,open_usage_prices_dir,get_usage_exchange_rate,check_updates,download_app_update,install_app_update,get_flow_logs,set_flow_logging,check_connectivity,enable_traffic,disable_traffic,install_traffic_service,uninstall_traffic_service,open_release,open_source,open_logs])
            .setup(move|app|{
                if quitting {app.handle().exit(0);return Ok(());}
                TrayIconBuilder::with_id("main-tray").icon(tray_image()).tooltip("CodexProxyTray").show_menu_on_left_click(false)
                    .on_tray_icon_event(|tray,event|{if let TrayIconEvent::Click{button,button_state:MouseButtonState::Up,position,..}=event{match button{MouseButton::Left=>show_page(tray.app_handle(),"dashboard"),MouseButton::Right=>toggle_tray(tray.app_handle(),position),_=>{}}}}).build(app)?;
                let handle=app.handle().clone();
                app.manage(Controller::new(move|status|{
                    let label=if !status.enabled{"守护已暂停"}else if status.error.is_some(){"守护需要检查"}else if !status.proxy_reachable{"等待本地代理"}else if status.guard_ready||status.cli_ready{"守护已就绪"}else{"等待应用接入"};
                    let title=format!("CodexProxyTray · {label}");let app=handle.clone();
                    let _=handle.run_on_main_thread(move||{if let Some(tray)=app.tray_by_id("main-tray"){let _=tray.set_tooltip(Some(&title));}});
                }));
                // Initial local discovery; the worker silently refreshes it every 10 seconds.
                let _=std::thread::Builder::new().name("cli-installation-inspect".into()).spawn(||{let _=crate::cli_manager::inspect();});
                if refreshing {let(tx,_)=std::sync::mpsc::channel();let _=app.state::<Controller>().tx.send(Message::Refresh(tx));}
                if let Some(window)=app.get_webview_window("main"){if let Ok(hwnd)=window.hwnd(){crate::native::set_window_icons(hwnd.0 as *mut std::ffi::c_void);}}
                Ok(())
            })
            .on_window_event(|window,event|{match event{tauri::WindowEvent::Focused(false)if window.label()=="tray"=>{let _=window.hide();},tauri::WindowEvent::CloseRequested{api,..}=>{api.prevent_close();let _=window.hide();},tauri::WindowEvent::ScaleFactorChanged{..}if window.label()=="main"=>{if let Ok(hwnd)=window.hwnd(){crate::native::set_window_icons(hwnd.0 as *mut std::ffi::c_void);}if let Some(tray)=window.app_handle().tray_by_id("main-tray"){let _=tray.set_icon(Some(tray_image()));}},_=>{}}})
            .build(tauri::generate_context!()).expect("CodexProxyTray 启动失败");
        app.run(|handle,event|{match event{tauri::RunEvent::ExitRequested{api,..}if crate::cli_manager::is_mutating()||crate::desktop_lifecycle::in_progress()=>api.prevent_exit(),tauri::RunEvent::Exit=>{if let Some(state)=handle.try_state::<Controller>(){state.stop();}},_=>{}}});
    }
}
#[cfg(windows)]fn main(){let raw=std::env::args_os().skip(1).collect::<Vec<_>>();if let Some(code)=traffic_service::service_entry(&raw){std::process::exit(code)}if let Some(code)=traffic::collector_entry(&raw){std::process::exit(code)}let args=raw.iter().map(|s|s.to_string_lossy().into_owned()).collect::<Vec<_>>();match core::resume_args(&args){Ok(Some(request))=>{if let Err(error)=native::resume(request){let root=settings::root();let _=std::fs::create_dir_all(&root);let _=std::fs::write(root.join("activation-error.txt"),error);std::process::exit(1)}},Err(_)=>std::process::exit(2),Ok(None)=>desktop::run()}}
#[cfg(not(windows))]fn main(){eprintln!("CodexProxyTray desktop targets Windows. Core checks can run on Linux.");}
