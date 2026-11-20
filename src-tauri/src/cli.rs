mod core;
mod settings;
#[cfg(windows)] mod native;
#[cfg(windows)] mod desktop_lifecycle;
#[cfg(windows)] mod traffic_core;
#[cfg(windows)] mod platform;

#[cfg(windows)]
fn separate_session(command:&[String], args:&[std::ffi::OsString])->bool {
    use std::{os::windows::process::CommandExt, process::{Command,Stdio}};
    if !(args.is_empty() || args.first().is_some_and(|arg|arg=="resume" || arg=="fork"))
        || args.iter().any(|arg|arg=="--no-daemon" || arg=="--remote" || arg.to_string_lossy().starts_with("--remote=")) {
        return false;
    }
    // CLI installations predate --no-daemon. Probe the selected installation,
    // without opening a console, before changing an interactive launch.
    Command::new(&command[0]).args(&command[1..]).args(["--no-daemon","--version"])
        .creation_flags(0x08000000).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .status().is_ok_and(|status|status.success())
}

#[cfg(windows)]
fn main(){
    use std::process::{Command,exit};
    let root=settings::root();
    let (config,valid)=match settings::load(&root){Ok(c)=>(c,true),Err(e)=>{eprintln!("CodexProxyTray 配置无法读取，将直接运行原 CLI：{e}");(settings::Config::default(),false)}};
    let detected=settings::detect_cli(&root);
    let command=if !detected.is_empty(){detected}else if settings::command_available(&config.cli_command,&root){config.cli_command.clone()}else{vec![]};
    let terminal_dir=root.join("terminal");
    let terminal_entry=std::env::current_exe().is_ok_and(|exe|
        settings::normalized_executable(&exe)==settings::normalized_executable(&terminal_dir.join("codex.exe")));
    let active=valid&&platform::guard_active(&settings::event_name(&root))
        && (!terminal_entry || platform::terminal_registered(&terminal_dir).unwrap_or(false));
    let args=std::env::args_os().skip(1).collect::<Vec<_>>();
    if args.len()==1&&args[0]=="--proxy-info"{println!("{}",serde_json::json!({"guard_active":active,"proxy_url":if active{Some(&config.proxy_url)}else{None},"target":command,"lookup":settings::cli_lookup_info(&root)}));return}
    let Some(program)=command.first()else{eprintln!("未找到 Codex CLI。请先安装 CLI，CodexProxyTray 会自动检测接入。");exit(127)};
    let mut child=Command::new(program);child.args(&command[1..]);
    if active && separate_session(&command,&args) { child.arg("--no-daemon"); }
    child.args(args);
    if active{for(key,value)in core::proxy_pairs(&config.proxy_url){child.env(key,value);}}
    platform::wait_for_console_child();
    // Command passes arguments and inherited console handles directly; no command shell is involved.
    match child.status(){Ok(status)=>exit(status.code().unwrap_or(1)),Err(e)=>{eprintln!("无法启动 Codex CLI：{e}");exit(1)}}
}
#[cfg(not(windows))]fn main(){eprintln!("The proxy shim targets Windows.");}
