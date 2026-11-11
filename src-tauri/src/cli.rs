mod core;
mod settings;
#[cfg(windows)] mod native;
#[cfg(windows)] mod traffic_core;
#[cfg(windows)] mod platform;

#[cfg(windows)]
fn main(){
    use std::process::{Command,exit};
    let root=settings::root();
    let (config,valid)=match settings::load(&root){Ok(c)=>(c,true),Err(e)=>{eprintln!("CodexProxyTray 配置无法读取，将直接运行原 CLI：{e}");(settings::Config::default(),false)}};
    let detected=settings::detect_cli(&root);
    let command=if !detected.is_empty(){detected}else if settings::command_available(&config.cli_command,&root){config.cli_command.clone()}else{vec![]};
    let active=valid&&platform::guard_active(&settings::event_name(&root));
    let args=std::env::args_os().skip(1).collect::<Vec<_>>();
    if args.len()==1&&args[0]=="--proxy-info"{println!("{}",serde_json::json!({"guard_active":active,"proxy_url":if active{Some(&config.proxy_url)}else{None},"target":command,"lookup":settings::cli_lookup_info(&root)}));return}
    let Some(program)=command.first()else{eprintln!("未找到 Codex CLI。请先安装 CLI，再在 CodexProxyTray 中重新检测。");exit(127)};
    let mut child=Command::new(program);child.args(&command[1..]).args(args);
    if active{for(key,value)in core::proxy_pairs(&config.proxy_url){child.env(key,value);}}
    platform::wait_for_console_child();
    // Command passes arguments and inherited console handles directly; no command shell is involved.
    match child.status(){Ok(status)=>exit(status.code().unwrap_or(1)),Err(e)=>{eprintln!("无法启动 Codex CLI：{e}");exit(1)}}
}
#[cfg(not(windows))]fn main(){eprintln!("The proxy shim targets Windows.");}
