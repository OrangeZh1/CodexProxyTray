#![cfg_attr(windows, windows_subsystem = "windows")]
mod core;
#[cfg(windows)] mod native;
#[cfg(windows)] mod settings;
#[cfg(windows)] mod platform;
#[cfg(windows)] mod worker;
#[cfg(windows)] mod update;
#[cfg(windows)] mod launch;

#[cfg(windows)] mod desktop {
    use tauri::{Manager,State,Emitter,menu::{Menu,MenuItem,PredefinedMenuItem},tray::{TrayIconBuilder,TrayIconEvent,MouseButton,MouseButtonState}};
    use crate::worker::{Controller,Message,Status,Preferences};
    fn show(app:&tauri::AppHandle){if let Some(window)=app.get_webview_window("main"){let _=window.show();let _=window.unminimize();let _=window.set_focus();}}
    fn show_page(app:&tauri::AppHandle,page:&str){show(app);let _=app.emit("navigate",page);}
    fn tray_image()->tauri::image::Image<'static>{match crate::native::small_icon_size(){0..=16=>tauri::include_image!("icons/icon-16.png"),17..=20=>tauri::include_image!("icons/icon-20.png"),21..=24=>tauri::include_image!("icons/icon-24.png"),25..=32=>tauri::include_image!("icons/icon-32.png"),33..=40=>tauri::include_image!("icons/icon-40.png"),41..=48=>tauri::include_image!("icons/icon-48.png"),_=>tauri::include_image!("icons/icon-64.png")}}
    #[tauri::command] fn get_status(state:State<'_,Controller>)->Status{state.snapshot()}
    async fn request(state:State<'_,Controller>,make:impl FnOnce(std::sync::mpsc::Sender<Result<Status,String>>)->Message+Send+'static)->Result<Status,String>{let tx=state.tx.clone();tauri::async_runtime::spawn_blocking(move||{let(reply,rx)=std::sync::mpsc::channel();tx.send(make(reply)).map_err(|_|"后台已退出".to_owned())?;rx.recv_timeout(std::time::Duration::from_secs(8)).map_err(|_|"操作仍在处理中，请查看当前状态。".to_owned())?}).await.map_err(|e|e.to_string())?}
    #[tauri::command] async fn set_enabled(enabled:bool,state:State<'_,Controller>)->Result<Status,String>{request(state,move|r|Message::Enabled(enabled,Some(r))).await}
    #[tauri::command] async fn save_proxy(proxy_url:String,state:State<'_,Controller>)->Result<Status,String>{request(state,move|r|Message::Proxy(proxy_url,r)).await}
    #[tauri::command] async fn save_system(autostart:Option<bool>,launch_target:Option<crate::settings::LaunchTarget>,cli_workdir:Option<String>,network_enabled:Option<bool>,auto_check_updates:Option<bool>,update_interval_hours:Option<u32>,state:State<'_,Controller>)->Result<Status,String>{request(state,move|r|Message::System(Preferences{autostart,launch_target,cli_workdir,network_enabled,auto_check_updates,update_interval_hours},r)).await}
    #[tauri::command] async fn refresh_status(state:State<'_,Controller>)->Result<Status,String>{request(state,Message::Refresh).await}
    #[tauri::command] async fn launch_codex(state:State<'_,Controller>)->Result<Status,String>{request(state,Message::Launch).await}
    #[tauri::command] async fn launch_cli(state:State<'_,Controller>)->Result<Status,String>{request(state,Message::LaunchCli).await}
    #[tauri::command] async fn check_updates()->Result<crate::update::Update,String>{tauri::async_runtime::spawn_blocking(crate::update::check).await.map_err(|_|"更新检查暂不可用。".to_owned())?}
    #[tauri::command]fn open_release()->Result<(),String>{crate::native::open(crate::update::RELEASES)}
    #[tauri::command]fn open_source()->Result<(),String>{crate::native::open(crate::update::SOURCE)}
    #[tauri::command]fn open_logs()->Result<(),String>{let path=crate::settings::root().join("logs");std::fs::create_dir_all(&path).map_err(|e|e.to_string())?;crate::native::open(&path.to_string_lossy())}
    pub fn run(){
        let quitting=std::env::args().any(|s|s=="--quit");
        let refreshing=std::env::args().any(|s|s=="--refresh");
        let background=quitting||refreshing||std::env::args().any(|s|s=="--background");
        let app=tauri::Builder::default().plugin(tauri_plugin_single_instance::init(|app,args,_|{if args.iter().any(|arg|arg=="--quit"){app.exit(0);}else if args.iter().any(|arg|arg=="--refresh"){let(tx,_)=std::sync::mpsc::channel();let _=app.state::<Controller>().tx.send(Message::Refresh(tx));}else if !args.iter().any(|arg|arg=="--background"){show(app);}}))
            .invoke_handler(tauri::generate_handler![get_status,set_enabled,save_proxy,save_system,refresh_status,launch_codex,launch_cli,check_updates,open_release,open_source,open_logs])
            .setup(move|app|{
                if quitting {app.handle().exit(0);return Ok(());}
                let status_item=MenuItem::with_id(app,"status","CodexProxyTray · 正在检测",false,None::<&str>)?;
                let proxy_item=MenuItem::with_id(app,"proxy-status","本地代理 · 正在检测",false,None::<&str>)?;
                let show_item=MenuItem::with_id(app,"show","打开仪表盘",true,None::<&str>)?;
                let settings_item=MenuItem::with_id(app,"settings","设置",true,None::<&str>)?;
                let logs_item=MenuItem::with_id(app,"logs","运行日志",true,None::<&str>)?;
                let launch_item=MenuItem::with_id(app,"launch","打开 ChatGPT",true,None::<&str>)?;
                let cli_item=MenuItem::with_id(app,"launch-cli","打开 Codex CLI",true,None::<&str>)?;
                let refresh_item=MenuItem::with_id(app,"refresh","重新检测",true,None::<&str>)?;
                let toggle_item=MenuItem::with_id(app,"toggle","暂停守护",true,None::<&str>)?;
                let quit_item=MenuItem::with_id(app,"quit","退出 CodexProxyTray",true,None::<&str>)?;
                let separator1=PredefinedMenuItem::separator(app)?;let separator2=PredefinedMenuItem::separator(app)?;let separator3=PredefinedMenuItem::separator(app)?;
                let menu=Menu::with_items(app,&[&status_item,&proxy_item,&separator1,&show_item,&settings_item,&logs_item,&separator2,&launch_item,&cli_item,&refresh_item,&toggle_item,&separator3,&quit_item])?;
                TrayIconBuilder::with_id("main-tray").icon(tray_image()).tooltip("CodexProxyTray").menu(&menu).show_menu_on_left_click(false)
                    .on_menu_event(|app,event|{match event.id().as_ref(){"show"=>show_page(app,"dashboard"),"settings"=>show_page(app,"settings"),"logs"=>show_page(app,"logs"),"launch"=>{let(tx,_)=std::sync::mpsc::channel();let _=app.state::<Controller>().tx.send(Message::Launch(tx));},"launch-cli"=>{let(tx,_)=std::sync::mpsc::channel();let _=app.state::<Controller>().tx.send(Message::LaunchCli(tx));},"refresh"=>{let(tx,_)=std::sync::mpsc::channel();let _=app.state::<Controller>().tx.send(Message::Refresh(tx));},"toggle"=>{let state=app.state::<Controller>();let _=state.tx.send(Message::Enabled(!state.snapshot().enabled,None));},"quit"=>app.exit(0),_=>{}}})
                    .on_tray_icon_event(|tray,event|{if let TrayIconEvent::Click{button:MouseButton::Left,button_state:MouseButtonState::Up,..}=event{show_page(tray.app_handle(),"dashboard")}}).build(app)?;
                let handle=app.handle().clone();
                app.manage(Controller::new(move|status|{
                    let label=if !status.enabled{"守护已暂停"}else if status.error.is_some(){"守护需要检查"}else if !status.proxy_reachable{"等待本地代理"}else if status.guard_ready||status.cli_ready{"守护已就绪"}else{"等待应用接入"};
                    let title=format!("CodexProxyTray · {label}");let proxy=format!("代理：{} · {}",status.proxy_url,if status.proxy_reachable{"可连接"}else{"未连接"});let toggle=if status.enabled{"暂停守护"}else{"启用守护"};let can_launch=status.package_count>0;let can_cli=status.cli_ready;
                    let (status_item,proxy_item,toggle_item,launch_item,cli_item,app)=(status_item.clone(),proxy_item.clone(),toggle_item.clone(),launch_item.clone(),cli_item.clone(),handle.clone());
                    let _=handle.run_on_main_thread(move||{let _=status_item.set_text(&title);let _=proxy_item.set_text(&proxy);let _=toggle_item.set_text(toggle);let _=launch_item.set_enabled(can_launch);let _=cli_item.set_enabled(can_cli);if let Some(tray)=app.tray_by_id("main-tray"){let _=tray.set_tooltip(Some(&title));}});
                }));
                if refreshing {let(tx,_)=std::sync::mpsc::channel();let _=app.state::<Controller>().tx.send(Message::Refresh(tx));}
                if let Some(window)=app.get_webview_window("main"){if let Ok(hwnd)=window.hwnd(){crate::native::set_window_icons(hwnd.0 as *mut std::ffi::c_void);}}
                if !background{if let Some(window)=app.get_webview_window("main"){window.show()?;window.set_focus()?;}}
                Ok(())
            })
            .on_window_event(|window,event|{match event{tauri::WindowEvent::CloseRequested{api,..}=>{api.prevent_close();let _=window.hide();},tauri::WindowEvent::ScaleFactorChanged{..}=>{if let Ok(hwnd)=window.hwnd(){crate::native::set_window_icons(hwnd.0 as *mut std::ffi::c_void);}if let Some(tray)=window.app_handle().tray_by_id("main-tray"){let _=tray.set_icon(Some(tray_image()));}},_=>{}}})
            .build(tauri::generate_context!()).expect("CodexProxyTray 启动失败");
        app.run(|handle,event|{if let tauri::RunEvent::Exit=event{if let Some(state)=handle.try_state::<Controller>(){state.stop();}}});
    }
}
#[cfg(windows)]fn main(){let args=std::env::args().skip(1).collect::<Vec<_>>();match core::resume_args(&args){Ok(Some(request))=>{if let Err(error)=native::resume(request){let root=settings::root();let _=std::fs::create_dir_all(&root);let _=std::fs::write(root.join("activation-error.txt"),error);std::process::exit(1)}},Err(_)=>std::process::exit(2),Ok(None)=>desktop::run()}}
#[cfg(not(windows))]fn main(){eprintln!("CodexProxyTray desktop targets Windows. Core checks can run on Linux.");}
