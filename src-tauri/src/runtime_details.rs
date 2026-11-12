//! On-demand, read-only runtime details. Endpoints remain in the returned view only;
//! this module never records them in status, logs, caches, or files.
use serde::Serialize;
use std::{collections::BTreeMap,ffi::c_void,net::{Ipv4Addr,Ipv6Addr,SocketAddrV6},ptr};
use crate::{native,settings,traffic::{OwnedHandle,open_identity},traffic_core::ProcessIdentity};

const ROW_LIMIT:usize=200;
const MAX_TABLE_BYTES:u32=16*1024*1024;
#[derive(Serialize)]pub struct ConnectionRow{pub pid:u32,pub process_label:String,pub local:String,pub remote:String,pub state:&'static str}
#[derive(Serialize)]pub struct ConnectionSnapshot{
    pub sampled_at:u64,pub read_started_at:u64,pub rows:Vec<ConnectionRow>,pub target_count:usize,
    pub matched_total:usize,pub row_limit:usize,pub truncated:bool,pub scope_note:&'static str,
}
#[derive(Serialize)]pub struct MemoryRow{
    pub pid:u32,pub process_label:String,pub role:&'static str,
    pub working_set_bytes:Option<u64>,pub peak_working_set_bytes:Option<u64>,pub private_bytes:Option<u64>,pub error:Option<String>,
}
#[derive(Serialize)]pub struct MemorySnapshot{
    pub sampled_at:u64,pub read_started_at:u64,pub rows:Vec<MemoryRow>,pub working_set_bytes:Option<u64>,
    pub peak_working_set_bytes:Option<u64>,pub private_bytes:Option<u64>,pub complete:bool,pub scope_note:&'static str,
}
type Handle=*mut c_void;
#[link(name="iphlpapi")]extern "system"{fn GetExtendedTcpTable(table:Handle,length:*mut u32,order:i32,family:u32,kind:u32,reserved:u32)->u32;}
#[link(name="kernel32")]extern "system"{
    fn CreateToolhelp32Snapshot(flags:u32,pid:u32)->Handle;
    fn Process32FirstW(snapshot:Handle,entry:*mut ProcessEntry)->i32;
    fn Process32NextW(snapshot:Handle,entry:*mut ProcessEntry)->i32;
    fn CloseHandle(handle:Handle)->i32;
    fn GetLastError()->u32;
    fn GetSystemTimeAsFileTime(time:*mut u64);
    fn WaitForSingleObject(handle:Handle,timeout:u32)->u32;
}
#[link(name="psapi")]extern "system"{fn GetProcessMemoryInfo(process:Handle,counters:*mut MemoryCounters,size:u32)->i32;}
#[repr(C)]struct ProcessEntry{size:u32,usage:u32,pid:u32,heap:usize,module:u32,threads:u32,parent:u32,priority:i32,flags:u32,exe:[u16;260]}
#[repr(C)]struct MemoryCounters{size:u32,faults:u32,peak:usize,working:usize,peak_paged:usize,paged:usize,peak_nonpaged:usize,nonpaged:usize,pagefile:usize,peak_pagefile:usize,private:usize}
struct VerifiedProcess{identity:ProcessIdentity,image:String,handle:OwnedHandle}
fn process(pid:u32)->Option<VerifiedProcess>{
    let(identity,image,_)=unsafe{native::process_identity(pid)}?;
    let handle=open_identity(identity).ok()??;Some(VerifiedProcess{identity,image,handle})
}
fn unchanged(process:&VerifiedProcess)->bool{
    unsafe{WaitForSingleObject(process.handle.0.0,0)==258&&native::process_identity(process.identity.pid).is_some_and(|(identity,image,_)|identity==process.identity&&image==process.image)}
}
fn label(image:&str)->String{image.rsplit(['\\','/']).next().unwrap_or("未知进程").to_owned()}
fn tcp_table(family:u32)->Result<Vec<u8>,String>{unsafe{
    let mut length=0;let initial=GetExtendedTcpTable(ptr::null_mut(),&mut length,1,family,5,0);
    if !matches!(initial,0|122){return Err(format!("无法读取 TCP 连接表（Windows 错误 {initial}）。"))}
    for _ in 0..3{
        if !(4..=MAX_TABLE_BYTES).contains(&length){return Err("TCP 连接表长度超出允许范围。".into())}
        let mut bytes=vec![0u8;length as usize];let result=GetExtendedTcpTable(bytes.as_mut_ptr().cast(),&mut length,1,family,5,0);
        if result==122{continue}if result!=0{return Err(format!("无法读取 TCP 连接表（Windows 错误 {result}）。"))}
        if length as usize>bytes.len(){return Err("TCP 连接表返回长度无效。".into())}bytes.truncate(length as usize);return Ok(bytes)
    }Err("连接表正在快速变化，请稍后重新打开详情。".into())
}}
fn number(bytes:&[u8],offset:usize)->u32{u32::from_le_bytes(bytes[offset..offset+4].try_into().unwrap())}
fn port(bytes:&[u8],offset:usize)->u16{u16::from_be_bytes(bytes[offset..offset+2].try_into().unwrap())}
fn parse_tcp(bytes:&[u8],ipv6:bool,targets:&BTreeMap<u32,String>,rows:&mut Vec<ConnectionRow>)->Result<usize,String>{
    let size=if ipv6{56}else{24};let count=bytes.get(..4).map(|value|u32::from_le_bytes(value.try_into().unwrap())as usize).ok_or("TCP 连接表不完整。")?;
    let end=count.checked_mul(size).and_then(|value|value.checked_add(4)).filter(|&value|value<=bytes.len()).ok_or("TCP 连接表的记录长度无效。")?;
    let mut matched=0;
    for row in bytes[4..end].chunks_exact(size){
        let(state_offset,pid_offset)=if ipv6{(48,52)}else{(0,20)};if number(row,state_offset)!=5{continue}
        let pid=number(row,pid_offset);let Some(process_label)=targets.get(&pid)else{continue};matched+=1;if rows.len()>=ROW_LIMIT{continue}
        let(local,remote)=if ipv6{
            let address=|offset,scope_offset,port_offset|{let ip=Ipv6Addr::from(<[u8;16]>::try_from(&row[offset..offset+16]).unwrap());let scope=u32::from_be_bytes(row[scope_offset..scope_offset+4].try_into().unwrap());SocketAddrV6::new(ip,port(row,port_offset),0,scope).to_string()};
            (address(0,16,20),address(24,40,44))
        }else{(format!("{}:{}",Ipv4Addr::from(<[u8;4]>::try_from(&row[4..8]).unwrap()),port(row,8)),format!("{}:{}",Ipv4Addr::from(<[u8;4]>::try_from(&row[12..16]).unwrap()),port(row,16)))};
        rows.push(ConnectionRow{pid,process_label:process_label.clone(),local,remote,state:"established"});
    }Ok(matched)
}
pub fn connections()->Result<ConnectionSnapshot,String>{
    let read_started_at=settings::now();let root=settings::root();let config=settings::load(&root).map_err(|_|"配置无法读取，未查询目标连接。".to_owned())?;
    let targets=native::traffic_targets(&root,&config)?;let before=targets.into_iter().filter_map(|identity|process(identity.pid).filter(|process|process.identity==identity)).collect::<Vec<_>>();
    let ipv4=tcp_table(2)?;let ipv6=tcp_table(23)?;
    // Both table reads are bracketed by live PID + creation + executable checks.
    // Holding the process handles also prevents the old process object disappearing.
    let verified=before.iter().filter(|process|unchanged(process)).map(|process|(process.identity.pid,label(&process.image))).collect::<BTreeMap<_,_>>();
    let mut rows=vec![];let matched_total=parse_tcp(&ipv4,false,&verified,&mut rows)?+parse_tcp(&ipv6,true,&verified,&mut rows)?;
    Ok(ConnectionSnapshot{sampled_at:settings::now(),read_started_at,target_count:verified.len(),truncated:matched_total>rows.len(),rows,matched_total,row_limit:ROW_LIMIT,scope_note:"仅显示已验证的 ChatGPT / Codex CLI 目标进程当前已建立的 TCP 连接；不包含 UDP、DNS 域名、规则或历史连接。地址只按需显示，不写入日志或保存。"})
}
struct ProcessRow{pid:u32,parent:u32,webview:bool}
fn process_rows()->Result<Vec<ProcessRow>,String>{unsafe{
    let raw=CreateToolhelp32Snapshot(2,0);if raw as isize == -1{return Err("无法读取助手子进程列表。".into())}
    // This snapshot handle is closed on every path, including a failed first read.
    let result=(||{let mut entry:ProcessEntry=std::mem::zeroed();entry.size=std::mem::size_of::<ProcessEntry>()as u32;
        if Process32FirstW(raw,&mut entry)==0{return Err("无法读取助手子进程列表。".to_owned())}let mut rows=vec![];
        loop{let end=entry.exe.iter().position(|&c|c==0).unwrap_or(entry.exe.len());rows.push(ProcessRow{pid:entry.pid,parent:entry.parent,webview:String::from_utf16_lossy(&entry.exe[..end]).eq_ignore_ascii_case("msedgewebview2.exe")});if Process32NextW(raw,&mut entry)==0{break}}
        if GetLastError()!=18{return Err("助手子进程列表读取不完整。".into())}Ok(rows)
    })();CloseHandle(raw);result
}}
fn memory_row(process:&VerifiedProcess,helper:bool)->Option<MemoryRow>{unsafe{
    if !unchanged(process){return None}let mut counters:MemoryCounters=std::mem::zeroed();let size=std::mem::size_of::<MemoryCounters>()as u32;counters.size=size;
    let good=GetProcessMemoryInfo(process.handle.0.0,&mut counters,size)!=0;let code=GetLastError();if !unchanged(process){return None}
    Some(MemoryRow{pid:process.identity.pid,process_label:label(&process.image),role:if helper{"helper"}else{"webview2"},working_set_bytes:good.then_some(counters.working as u64),peak_working_set_bytes:good.then_some(counters.peak as u64),private_bytes:good.then_some(counters.private as u64),error:(!good).then(||format!("无法读取此进程内存（Windows 错误 {code}）。"))})
}}
fn sum(rows:&[MemoryRow],value:impl Fn(&MemoryRow)->Option<u64>)->Option<u64>{rows.iter().try_fold(0u64,|total,row|total.checked_add(value(row)?))}
fn memory_complete(rows:&[MemoryRow],expected:usize,selection_complete:bool)->bool{selection_complete&&rows.len()==expected&&rows.iter().all(|row|row.error.is_none())}
pub fn memory()->Result<MemorySnapshot,String>{
    let read_started_at=settings::now();let mut snapshot_started=0;unsafe{GetSystemTimeAsFileTime(&mut snapshot_started)};
    let helper_pid=std::process::id();let helper=process(helper_pid).ok_or("无法验证助手进程。")?;let own_image=std::env::current_exe().map_err(|_|"无法验证助手路径。")?;
    if helper.image!=settings::normalized_executable(&own_image){return Err("助手进程路径已变化，未读取其他应用内存。".into())}
    let candidates=process_rows()?;let mut selected=BTreeMap::new();let mut selection_complete=true;selected.insert(helper_pid,helper);
    loop{let mut added=false;for row in candidates.iter().filter(|row|row.webview){
        if selected.contains_key(&row.pid){continue}let Some(parent)=selected.get(&row.parent)else{continue};if !unchanged(parent){continue}
        if let Some(child)=process(row.pid){if eligible_child(parent.identity,child.identity,snapshot_started,&child.image){if selected.len()>=128{return Err("助手 WebView2 进程数超出详情读取上限。".into())}selected.insert(row.pid,child);added=true;}}else{selection_complete=false;}
    }if !added{break}}
    let rows=selected.values().filter_map(|process|memory_row(process,process.identity.pid==helper_pid)).collect::<Vec<_>>();
    if !rows.iter().any(|row|row.pid==helper_pid){return Err("助手进程状态发生变化，请重新打开详情。".into())}
    let working_set_bytes=sum(&rows,|row|row.working_set_bytes);let peak_working_set_bytes=sum(&rows,|row|row.peak_working_set_bytes);let private_bytes=sum(&rows,|row|row.private_bytes);
    Ok(MemorySnapshot{sampled_at:settings::now(),read_started_at,complete:memory_complete(&rows,selected.len(),selection_complete),rows,working_set_bytes,peak_working_set_bytes,private_bytes,scope_note:"范围仅为助手与当前可验证的 WebView2 子进程，不包含 ChatGPT 或 Codex CLI。工作集求和可能重复计算共享页；峰值为各进程历史峰值之和，并非整组同时发生的峰值。私有提交量来自 Windows，不代表 Rust / JavaScript 堆或 GC 指标。"})
}
fn eligible_child(parent:ProcessIdentity,child:ProcessIdentity,snapshot_started:u64,image:&str)->bool{
    child.pid!=parent.pid&&child.created_filetime>=parent.created_filetime&&child.created_filetime<=snapshot_started&&label(image).eq_ignore_ascii_case("msedgewebview2.exe")
}

#[cfg(test)]mod tests{
    use super::*;
    #[test]fn parses_ipv4_ownership_state_and_network_ports(){
        let mut bytes=vec![0u8;4+24*3];bytes[..4].copy_from_slice(&3u32.to_le_bytes());
        for(index,pid,state)in[(0,42u32,5u32),(1,99,5),(2,42,2)]{let row=&mut bytes[4+index*24..4+(index+1)*24];row[..4].copy_from_slice(&state.to_le_bytes());row[4..8].copy_from_slice(&[127,0,0,1]);row[8..10].copy_from_slice(&53000u16.to_be_bytes());row[12..16].copy_from_slice(&[203,0,113,10]);row[16..18].copy_from_slice(&443u16.to_be_bytes());row[20..24].copy_from_slice(&pid.to_le_bytes());}
        let mut rows=vec![];assert_eq!(parse_tcp(&bytes,false,&BTreeMap::from([(42,"ChatGPT.exe".into())]),&mut rows).unwrap(),1);assert_eq!(rows.len(),1);assert_eq!(rows[0].local,"127.0.0.1:53000");assert_eq!(rows[0].remote,"203.0.113.10:443");assert_eq!(rows[0].state,"established");
        assert!(parse_tcp(&bytes[..bytes.len()-1],false,&BTreeMap::new(),&mut vec![]).is_err());
    }
    #[test]fn parses_ipv6_without_dns_and_bounds_visible_rows(){
        let mut bytes=vec![0u8;4+56];bytes[..4].copy_from_slice(&1u32.to_le_bytes());let row=&mut bytes[4..];row[0..16].copy_from_slice(&Ipv6Addr::LOCALHOST.octets());row[20..22].copy_from_slice(&1234u16.to_be_bytes());row[24..40].copy_from_slice(&"fe80::1".parse::<Ipv6Addr>().unwrap().octets());row[40..44].copy_from_slice(&3u32.to_be_bytes());row[44..46].copy_from_slice(&443u16.to_be_bytes());row[48..52].copy_from_slice(&5u32.to_le_bytes());row[52..56].copy_from_slice(&7u32.to_le_bytes());
        let targets=BTreeMap::from([(7,"codex.exe".into())]);let mut rows=vec![];assert_eq!(parse_tcp(&bytes,true,&targets,&mut rows).unwrap(),1);assert_eq!(rows[0].local,"[::1]:1234");assert_eq!(rows[0].remote,"[fe80::1%3]:443");
        for _ in 0..ROW_LIMIT+5{assert_eq!(parse_tcp(&bytes,true,&targets,&mut rows).unwrap(),1)}assert_eq!(rows.len(),ROW_LIMIT);
    }
    #[test]fn webview_selection_excludes_other_apps_and_reused_parent_times(){
        let parent=ProcessIdentity{pid:1,created_filetime:100};let child=ProcessIdentity{pid:2,created_filetime:110};
        assert!(eligible_child(parent,child,120,r"C:\Runtime\msedgewebview2.exe"));assert!(!eligible_child(parent,child,120,r"C:\Apps\ChatGPT.exe"));assert!(!eligible_child(parent,child,120,r"C:\Apps\codex.exe"));
        assert!(!eligible_child(parent,ProcessIdentity{pid:2,created_filetime:90},120,"msedgewebview2.exe"));assert!(!eligible_child(parent,child,105,"msedgewebview2.exe"));
        let row=MemoryRow{pid:1,process_label:"fixture".into(),role:"helper",working_set_bytes:Some(10),peak_working_set_bytes:Some(20),private_bytes:None,error:Some("unavailable".into())};assert_eq!(sum(&[row],|row|row.private_bytes),None);
    }
    #[test]fn inaccessible_or_exited_children_never_produce_a_complete_memory_sample(){
        let rows=vec![MemoryRow{pid:1,process_label:"fixture".into(),role:"helper",working_set_bytes:Some(10),peak_working_set_bytes:Some(20),private_bytes:Some(8),error:None}];
        assert!(memory_complete(&rows,1,true));assert!(!memory_complete(&rows,2,true),"An exited selected child is still a partial sample");assert!(!memory_complete(&rows,1,false),"A child whose identity could not be verified is not a complete process selection");
    }
}
