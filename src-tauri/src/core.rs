use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

pub const FAMILY: &str = "OpenAI.Codex_2p2nqsd0c76g0";
pub const DEFAULT_PROXY: &str = "http://127.0.0.1:7890";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint { pub url: String, pub socket: SocketAddr }

pub fn endpoint(input: &str) -> Result<Endpoint, String> {
    let invalid = || "请使用带端口的本机 HTTP 代理，例如 http://127.0.0.1:7890。".to_owned();
    let text = input.trim();
    let address = text.strip_prefix("http://").ok_or_else(invalid)?;
    let address = address.strip_suffix('/').unwrap_or(address);
    if address.chars().any(|c| c.is_whitespace() || c.is_control() || "/\\@?#".contains(c)) { return Err(invalid()); }
    let (host, port) = address.rsplit_once(':').ok_or_else(invalid)?;
    let port: u16 = port.parse().map_err(|_| invalid())?;
    if port == 0 { return Err(invalid()); }
    let ip = match host {
        "localhost" | "127.0.0.1" => IpAddr::V4(Ipv4Addr::LOCALHOST),
        "[::1]" => IpAddr::V6(Ipv6Addr::LOCALHOST),
        _ => return Err(invalid()),
    };
    Ok(Endpoint { url: format!("http://{host}:{port}"), socket: SocketAddr::new(ip, port) })
}

pub fn proxy_pairs(url: &str) -> Vec<(&'static str, String)> {
    let mut entries = vec![
        ("HTTP_PROXY", url.into()), ("HTTPS_PROXY", url.into()), ("ALL_PROXY", url.into()),
        ("NO_PROXY", "localhost,127.0.0.1,::1".into()),
        ("CODEX_PROXY_TRAY", format!("v1:{url}")),
    ];
    entries.sort_by_key(|entry| entry.0);
    entries
}

pub fn environment_block(url: &str) -> Vec<u16> {
    let mut block = Vec::new();
    for (key, value) in proxy_pairs(url) { block.extend(format!("{key}={value}").encode_utf16()); block.push(0); }
    block.push(0);
    block
}

#[derive(Debug, PartialEq, Eq)]
pub struct ResumeRequest { pub pid: u32, pub tid: u32 }
pub fn resume_args(args: &[String]) -> Result<Option<ResumeRequest>, String> {
    if !args.iter().any(|s| s == "-p" || s == "-tid") { return Ok(None); }
    if args.len() != 4 { return Err("Invalid activation arguments".into()); }
    let (mut pid, mut tid) = (0, 0);
    for pair in args.chunks_exact(2) {
        let id = pair[1].parse::<u32>().map_err(|_| "Invalid process/thread ID")?;
        if id == 0 { return Err("Invalid process/thread ID".into()); }
        match pair[0].as_str() { "-p" if pid == 0 => pid = id, "-tid" if tid == 0 => tid = id, _ => return Err("Invalid activation arguments".into()) }
    }
    if pid == 0 || tid == 0 { return Err("Missing process/thread ID".into()); }
    Ok(Some(ResumeRequest { pid, tid }))
}

pub fn version_parts(value: &str) -> Option<[u32; 3]> {
    let pieces = value.strip_prefix('v').unwrap_or(value).split('.').collect::<Vec<_>>();
    if pieces.len() != 3 || pieces.iter().any(|s| s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) || s.len() > 1 && s.starts_with('0')) { return None; }
    Some([pieces[0].parse().ok()?, pieces[1].parse().ok()?, pieces[2].parse().ok()?])
}
pub fn newer_version(latest: &str, current: &str) -> bool {
    match (version_parts(latest), version_parts(current)) { (Some(a), Some(b)) => a > b, _ => false }
}

pub fn descendant_pids(rows: &[(u32,u32)], roots: &[u32]) -> Vec<u32> {
    let mut found = roots.iter().copied().collect::<std::collections::BTreeSet<_>>();
    loop { let before=found.len(); for &(pid,parent) in rows { if found.contains(&parent) { found.insert(pid); } } if found.len()==before { break; } }
    found.into_iter().collect()
}
pub fn count_tcp_rows(bytes:&[u8], row_size:usize, state_offset:usize, pid_offset:usize, pids:&[u32]) -> Option<usize> {
    let number=|at:usize| -> Option<u32> {Some(u32::from_ne_bytes(bytes.get(at..at.checked_add(4)?)?.try_into().ok()?))};
    let count=number(0)? as usize;
    if row_size<4 || state_offset.checked_add(4)?>row_size || pid_offset.checked_add(4)?>row_size || count>bytes.len().saturating_sub(4)/row_size {return None}
    let mut established=0;
    for index in 0..count { let at=4+index*row_size; if number(at+state_offset)?==5 && pids.contains(&number(at+pid_offset)?) { established+=1; } }
    Some(established)
}
pub fn valid_log_position(id:u64,at:u64)->bool { id>0&&id<=9_007_199_254_740_991&&at<=8_640_000_000_000_000 }

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn only_loopback_without_credentials_or_extra_paths() {
        for good in [DEFAULT_PROXY, "http://localhost:1234/", "http://[::1]:8080"] { assert!(endpoint(good).is_ok(), "{good}"); }
        for bad in ["https://localhost:7890", "http://example.com:7890", "http://user:pass@localhost:7890", "http://localhost:7890/path", "http://localhost:0", "http://localhost:65536", "http://localhost:7890//", "http://localhost:7890?x=1", "http://localhost:7890\0"] { assert!(endpoint(bad).is_err(), "{bad:?}"); }
    }
    #[test] fn block_is_sorted_double_terminated_and_scoped() {
        let b = environment_block(DEFAULT_PROXY); assert_eq!(&b[b.len()-2..], &[0,0]);
        let entries: Vec<String> = b.split(|v| *v == 0).filter(|s| !s.is_empty()).map(|s| String::from_utf16(s).unwrap()).collect();
        let mut sorted = entries.clone(); sorted.sort(); assert_eq!(entries, sorted); assert_eq!(entries.len(), 5);
        assert!(entries.iter().any(|s| s == "NO_PROXY=localhost,127.0.0.1,::1"));
    }
    #[test] fn resume_requires_both_ids_and_rejects_ambiguous_input() {
        let args = |s: &str| s.split_whitespace().map(str::to_owned).collect::<Vec<_>>();
        assert_eq!(resume_args(&args("-p 100 -tid 200")).unwrap(), Some(ResumeRequest { pid: 100, tid: 200 }));
        assert!(resume_args(&args("--background")).unwrap().is_none());
        for bad in ["-p 0 -tid 200", "-p 1 -p 2", "-tid 123", "-p 1 -tid 2 --bad"] { assert!(resume_args(&args(bad)).is_err()); }
    }
    #[test] fn release_version_comparison_uses_numbers() { assert!(newer_version("v0.10.0", "0.9.0")); assert!(!newer_version("v0.1.0", "0.1.0")); assert!(!newer_version("v0.2.0-beta", "0.1.0")); }
    #[test] fn release_versions_require_three_unambiguous_numbers() { for value in ["1", "1.0", "vv1.0.0", "01.0.0", "+1.0.0", "1.2.3.4", "4294967296.0.0"] { assert!(version_parts(value).is_none(), "{value}"); } }
    #[test] fn telemetry_counts_only_owned_established_tcp_rows() {
        for (size,state,pid) in [(24,0,20),(56,48,52)] {
            let mut data=vec![0;4+3*size];data[..4].copy_from_slice(&3u32.to_ne_bytes());
            for (i,(status,owner)) in [(5u32,42u32),(2,42),(5,99)].into_iter().enumerate() {data[4+i*size+state..8+i*size+state].copy_from_slice(&status.to_ne_bytes());data[4+i*size+pid..8+i*size+pid].copy_from_slice(&owner.to_ne_bytes());}
            assert_eq!(count_tcp_rows(&data,size,state,pid,&[42]),Some(1));assert_eq!(count_tcp_rows(&data[..data.len()-1],size,state,pid,&[42]),None);
        }
        assert_eq!(descendant_pids(&[(2,1),(3,2),(4,99),(1,3)],&[1]),vec![1,2,3]);
    }
    #[test] fn history_rejects_overflowing_ids_and_unrenderable_dates(){assert!(valid_log_position(1,0));assert!(!valid_log_position(u64::MAX,1));assert!(!valid_log_position(1,u64::MAX));}
}
