#![cfg_attr(windows, windows_subsystem = "windows")]
mod core;
#[cfg(windows)] mod native;
#[cfg(windows)] mod settings;
#[cfg(windows)] mod platform;
#[cfg(windows)] mod worker;
#[cfg(windows)] mod update;
#[cfg(windows)] mod launch;
#[cfg(windows)] mod connectivity;
#[cfg(windows)] mod traffic_core;
#[cfg(windows)] mod traffic_etw;
#[cfg(windows)] mod traffic;
#[cfg(windows)] mod traffic_service;

#[cfg(windows)] mod desktop {
    use tauri::{Manager,State,Emitter,tray::{TrayIconBuilder,TrayIconEvent,MouseButton,MouseButtonState}};
    use crate::worker::{Controller,Message,Status,Preferences};
    use std::sync::atomic::{AtomicBool,Ordering};
    static SERVICE_OPERATION:AtomicBool=AtomicBool::new(false);
    struct ServiceOperation;
    impl Drop for ServiceOperation{fn drop(&mut self){SERVICE_OPERATION.store(false,Ordering::Release);}}
    struct UiStartup{ready:AtomicBool,show_requested:AtomicBool,page:std::sync::Mutex<Option<String>>,tray_ready:AtomicBool,tray_requested:AtomicBool}
    fn show(app:&tauri::AppHandle){if let Some(startup)=app.try_state::<UiStartup>(){startup.show_requested.store(true,Ordering::Release);if !startup.ready.load(Ordering::Acquire){return;}}if let Some(window)=app.get_webview_window("main"){let _=window.show();let _=window.unminimize();let _=window.set_focus();}}
    fn show_page(app:&tauri::AppHandle,page:&str){if let Some(startup)=app.try_state::<UiStartup>(){let mut pending=startup.page.lock().unwrap_or_else(|e|e.into_inner());if !startup.ready.load(Ordering::Acquire){*pending=Some(page.into());drop(pending);show(app);return}}show(app);let _=app.emit("navigate",page);}
    fn show_tray(app:&tauri::AppHandle){if let Some(startup)=app.try_state::<UiStartup>(){startup.tray_requested.store(true,Ordering::Release);if !startup.tray_ready.load(Ordering::Acquire){return}}if let Some(window)=app.get_webview_window("tray"){let _=window.show();let _=window.set_focus();}}
    fn toggle_tray(app:&tauri::AppHandle,point:tauri::PhysicalPosition<f64>){if let Some(window)=app.get_webview_window("tray"){
        if window.is_visible().unwrap_or(false){let _=window.hide();return}
        if let Ok(Some(monitor))=window.monitor_from_point(point.x,point.y){let area=monitor.work_area();let scale=monitor.scale_factor();let(x,y,width,height)=crate::core::popup_bounds((point.x.round()as i32,point.y.round()as i32),(area.position.x,area.position.y,area.size.width,area.size.height),((272.0*scale).round()as u32,(380.0*scale).round()as u32),(8.0*scale).round()as i32);let _=window.set_size(tauri::PhysicalSize::new(width,height));let _=window.set_position(tauri::PhysicalPosition::new(x,y));}
        show_tray(app);
    }}
    fn tray_image()->tauri::image::Image<'static>{match crate::native::small_icon_size(){0..=16=>tauri::include_image!("icons/icon-16.png"),17..=20=>tauri::include_image!("icons/icon-20.png"),21..=24=>tauri::include_image!("icons/icon-24.png"),25..=32=>tauri::include_image!("icons/icon-32.png"),33..=40=>tauri::include_image!("icons/icon-40.png"),41..=48=>tauri::include_image!("icons/icon-48.png"),_=>tauri::include_image!("icons/icon-64.png")}}
    #[tauri::command] async fn get_status(state:State<'_,Controller>)->Result<Status,String>{Ok(state.snapshot())}
    #[tauri::command]fn report_ui_error(message:String,window:tauri::WebviewWindow,state:State<'_,Controller>)->Result<(),String>{let message=message.chars().take(2000).collect::<String>();let _=state.tx.send(Message::Notice("error".into(),"界面".into(),"界面加载失败".into(),message.clone()));crate::settings::write_json(&crate::settings::root().join("ui-error.json"),&serde_json::json!({"at":crate::settings::now(),"window":window.label(),"message":message}))}
    #[tauri::command]fn frontend_ready(rendered:Option<bool>,geometry:Option<serde_json::Value>,app:tauri::AppHandle,window:tauri::WebviewWindow,state:State<'_,UiStartup>)->Option<String>{let _=crate::settings::write_json(&crate::settings::root().join(format!("ui-ready-{}.json",window.label())),&serde_json::json!({"at":crate::settings::now(),"window":window.label(),"rendered":rendered.unwrap_or(false),"helper_pid":std::process::id(),"geometry":geometry.filter(|value|value.to_string().len()<=2048)}));if window.label()=="tray"{state.tray_ready.store(true,Ordering::Release);if state.tray_requested.load(Ordering::Acquire){show_tray(&app)}return None}let mut pending=state.page.lock().unwrap_or_else(|e|e.into_inner());if !state.ready.swap(true,Ordering::AcqRel){if state.show_requested.load(Ordering::Acquire){show(&app);}return pending.take()}None}
    #[tauri::command]fn navigate_main(page:String,app:tauri::AppHandle)->Result<(),String>{if !matches!(page.as_str(),"dashboard"|"proxy"|"logs"|"settings"){return Err("未知页面。".into())}if let Some(window)=app.get_webview_window("tray"){let _=window.hide();}show_page(&app,&page);Ok(())}
    #[tauri::command]fn quit_helper(app:tauri::AppHandle){app.exit(0);}
    async fn request(state:State<'_,Controller>,make:impl FnOnce(std::sync::mpsc::Sender<Result<Status,String>>)->Message+Send+'static)->Result<Status,String>{request_to(state.tx.clone(),make).await}
    async fn request_to(tx:std::sync::mpsc::Sender<Message>,make:impl FnOnce(std::sync::mpsc::Sender<Result<Status,String>>)->Message+Send+'static)->Result<Status,String>{tauri::async_runtime::spawn_blocking(move||{let(reply,rx)=std::sync::mpsc::channel();tx.send(make(reply)).map_err(|_|"后台已退出".to_owned())?;rx.recv_timeout(std::time::Duration::from_secs(8)).map_err(|_|"操作仍在处理中，请查看当前状态。".to_owned())?}).await.map_err(|e|e.to_string())?}
    #[tauri::command] async fn set_enabled(enabled:bool,state:State<'_,Controller>)->Result<Status,String>{request(state,move|r|Message::Enabled(enabled,Some(r))).await}
    #[tauri::command] async fn save_proxy(proxy_url:String,state:State<'_,Controller>)->Result<Status,String>{request(state,move|r|Message::Proxy(proxy_url,r)).await}
    #[tauri::command] async fn save_system(autostart:Option<bool>,launch_target:Option<crate::settings::LaunchTarget>,cli_workdir:Option<String>,network_enabled:Option<bool>,auto_check_updates:Option<bool>,update_interval_hours:Option<u32>,connectivity_enabled:Option<bool>,state:State<'_,Controller>)->Result<Status,String>{request(state,move|r|Message::System(Preferences{autostart,launch_target,cli_workdir,network_enabled,auto_check_updates,update_interval_hours,connectivity_enabled},r)).await}
    #[tauri::command] async fn refresh_status(state:State<'_,Controller>)->Result<Status,String>{request(state,Message::Refresh).await}
    #[tauri::command] async fn launch_codex(state:State<'_,Controller>)->Result<Status,String>{request(state,Message::Launch).await}
    #[tauri::command] async fn launch_cli(state:State<'_,Controller>)->Result<Status,String>{request(state,Message::LaunchCli).await}
    #[tauri::command] async fn check_updates()->Result<crate::update::Update,String>{tauri::async_runtime::spawn_blocking(crate::update::check).await.map_err(|_|"更新检查暂不可用。".to_owned())?}
    #[tauri::command] async fn check_connectivity()->Result<crate::connectivity::Connectivity,String>{tauri::async_runtime::spawn_blocking(crate::connectivity::check).await.map_err(|_|"连通检测暂不可用。".to_owned())?}
    #[tauri::command] async fn enable_traffic(state:State<'_,Controller>)->Result<Status,String>{request(state,|reply|Message::TrafficEnabled(true,reply)).await}
    #[tauri::command] async fn disable_traffic(state:State<'_,Controller>)->Result<Status,String>{request(state,|reply|Message::TrafficEnabled(false,reply)).await}
    async fn service_action(state:State<'_,Controller>,install:bool)->Result<crate::traffic_service::ServiceSnapshot,String>{
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
        let app=tauri::Builder::default().manage(UiStartup{ready:AtomicBool::new(false),show_requested:AtomicBool::new(!background),page:std::sync::Mutex::new(None),tray_ready:AtomicBool::new(false),tray_requested:AtomicBool::new(false)}).plugin(tauri_plugin_single_instance::init(|app,args,_|{if args.iter().any(|arg|arg=="--quit"){app.exit(0);}else if args.iter().any(|arg|arg=="--refresh"){let(tx,_)=std::sync::mpsc::channel();let _=app.state::<Controller>().tx.send(Message::Refresh(tx));}else if !args.iter().any(|arg|arg=="--background"){show(app);}}))
            .invoke_handler(tauri::generate_handler![get_status,report_ui_error,frontend_ready,navigate_main,quit_helper,set_enabled,save_proxy,save_system,refresh_status,launch_codex,launch_cli,check_updates,check_connectivity,enable_traffic,disable_traffic,install_traffic_service,uninstall_traffic_service,open_release,open_source,open_logs])
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
                if refreshing {let(tx,_)=std::sync::mpsc::channel();let _=app.state::<Controller>().tx.send(Message::Refresh(tx));}
                if let Some(window)=app.get_webview_window("main"){if let Ok(hwnd)=window.hwnd(){crate::native::set_window_icons(hwnd.0 as *mut std::ffi::c_void);}}
                Ok(())
            })
            .on_window_event(|window,event|{match event{tauri::WindowEvent::Focused(false)if window.label()=="tray"=>{let _=window.hide();},tauri::WindowEvent::CloseRequested{api,..}=>{api.prevent_close();let _=window.hide();},tauri::WindowEvent::ScaleFactorChanged{..}if window.label()=="main"=>{if let Ok(hwnd)=window.hwnd(){crate::native::set_window_icons(hwnd.0 as *mut std::ffi::c_void);}if let Some(tray)=window.app_handle().tray_by_id("main-tray"){let _=tray.set_icon(Some(tray_image()));}},_=>{}}})
            .build(tauri::generate_context!()).expect("CodexProxyTray 启动失败");
        app.run(|handle,event|{if let tauri::RunEvent::Exit=event{if let Some(state)=handle.try_state::<Controller>(){state.stop();}}});
    }
}
#[cfg(windows)]fn main(){let raw=std::env::args_os().skip(1).collect::<Vec<_>>();if let Some(code)=traffic_service::service_entry(&raw){std::process::exit(code)}if let Some(code)=traffic::collector_entry(&raw){std::process::exit(code)}let args=raw.iter().map(|s|s.to_string_lossy().into_owned()).collect::<Vec<_>>();match core::resume_args(&args){Ok(Some(request))=>{if let Err(error)=native::resume(request){let root=settings::root();let _=std::fs::create_dir_all(&root);let _=std::fs::write(root.join("activation-error.txt"),error);std::process::exit(1)}},Err(_)=>std::process::exit(2),Ok(None)=>desktop::run()}}
#[cfg(not(windows))]fn main(){eprintln!("CodexProxyTray desktop targets Windows. Core checks can run on Linux.");}
