//! Pure traffic attribution rules. No Windows APIs, sockets, packet data or files.
use serde::{Deserialize,Serialize};

pub const TCP_PROVIDER:u128=0x9a280ac0_c8e0_11d1_84e2_00c04fb998a2;
pub const UDP_PROVIDER:u128=0xbf3a50c5_a9c9_4988_a005_2df0b7c80f80;
pub const MAX_TARGETS:usize=256;
#[derive(Clone,Copy,Debug,PartialEq,Eq,Hash,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessIdentity {pub pid:u32,pub created_filetime:u64}
pub fn validate_targets(targets:&[ProcessIdentity])->Result<(),String>{
    if targets.len()>MAX_TARGETS{return Err("目标进程数量超过采样上限。".into())}
    let mut seen=std::collections::HashSet::new();
    for target in targets {if target.pid==0||target.created_filetime==0||!seen.insert(target.pid){return Err("目标进程身份无效或重复。".into())}}
    Ok(())
}
pub fn within_lifetime(created:u64,exited:Option<u64>,event_filetime:u64)->bool{event_filetime>=created&&exited.is_none_or(|ended|event_filetime<=ended)}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum Direction{Send,Receive}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub struct Packet{pub pid:u32,pub bytes:u32,pub direction:Direction}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum Decode{Ignore,Unsupported,Packet(Packet)}
/// Microsoft classic TcpIp/UdpIp v2: only PID and size (first two uint32s) are read.
/// IPv4/IPv6 send=10/26, receive=11/27; the EventId and header PID are not attribution.
pub fn decode(provider:u128,version:u8,opcode:u8,first_fields:&[u8])->Decode{
    if provider!=TCP_PROVIDER&&provider!=UDP_PROVIDER{return Decode::Ignore}
    let direction=match opcode{10|26=>Direction::Send,11|27=>Direction::Receive,_=>return Decode::Ignore};
    if version!=2||first_fields.len()<8{return Decode::Unsupported}
    Decode::Packet(Packet{pid:u32::from_le_bytes(first_fields[0..4].try_into().unwrap()),bytes:u32::from_le_bytes(first_fields[4..8].try_into().unwrap()),direction})
}
#[derive(Clone,Copy,Debug,Default,PartialEq,Eq)]pub struct Totals{pub sent:u64,pub received:u64}
impl Totals{
    pub fn add(&mut self,direction:Direction,bytes:u32)->bool{
        let value=match direction{Direction::Send=>&mut self.sent,Direction::Receive=>&mut self.received};
        match value.checked_add(bytes as u64){Some(next)=>{*value=next;true},None=>false}
    }
}

#[cfg(test)]mod tests{
    use super::*;
    #[test]fn only_documented_v2_network_transfer_events_are_decoded(){
        let fields=[42u32.to_le_bytes(),1200u32.to_le_bytes()].concat();
        for provider in [TCP_PROVIDER,UDP_PROVIDER]{for(opcode,direction)in[(10,Direction::Send),(11,Direction::Receive),(26,Direction::Send),(27,Direction::Receive)]{assert_eq!(decode(provider,2,opcode,&fields),Decode::Packet(Packet{pid:42,bytes:1200,direction}));}}
        assert_eq!(decode(TCP_PROVIDER,1,10,&fields),Decode::Unsupported);assert_eq!(decode(TCP_PROVIDER,2,10,&fields[..7]),Decode::Unsupported);
        assert_eq!(decode(TCP_PROVIDER,2,14,&fields),Decode::Ignore);assert_eq!(decode(0,2,10,&fields),Decode::Ignore);
    }
    #[test]fn identities_and_counter_overflow_fail_closed(){
        assert!(validate_targets(&[ProcessIdentity{pid:2,created_filetime:3}]).is_ok());
        assert!(validate_targets(&[ProcessIdentity{pid:2,created_filetime:3},ProcessIdentity{pid:2,created_filetime:4}]).is_err());
        assert!(validate_targets(&[ProcessIdentity{pid:0,created_filetime:1}]).is_err());
        let mut totals=Totals{sent:u64::MAX,received:0};assert!(!totals.add(Direction::Send,1));assert_eq!(totals.sent,u64::MAX);assert!(totals.add(Direction::Receive,8));assert_eq!(totals.received,8);
        assert!(!within_lifetime(20,Some(30),19));assert!(within_lifetime(20,Some(30),25));assert!(!within_lifetime(20,Some(30),31));assert!(within_lifetime(40,None,40));
    }
}
