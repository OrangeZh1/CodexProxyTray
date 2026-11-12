//! Closed, coarse event vocabulary. Never accept user text or operation arguments.
use serde::Deserialize;
use std::sync::{Mutex,mpsc::Sender};
use crate::worker::Message;

#[derive(Clone,Copy,Deserialize)]#[serde(rename_all="snake_case")]
pub enum UiEvent{AppearanceChanged,WindowPinOn,WindowPinOff,WindowPinFailed,UsageSettingsChanged,UsageSyncStarted,UsageSyncSuccess,UsageSyncFailed,PricesSyncStarted,PricesSyncSuccess,PricesSyncFailed,CurrencyChanged}
pub fn note(tx:&Sender<Message>,level:&'static str,category:&'static str,message:&'static str,details:&'static str)->Result<(),String>{tx.send(Message::Notice(level.into(),category.into(),message.into(),details.into())).map_err(|_|"后台日志暂不可用。".into())}
pub fn ui(tx:&Sender<Message>,event:UiEvent)->Result<(),String>{
    let(level,category,message)=match event{
        UiEvent::AppearanceChanged=>("info","外观","已调整界面主题"),UiEvent::WindowPinOn=>("info","窗口","已启用窗口置顶"),UiEvent::WindowPinOff=>("info","窗口","已取消窗口置顶"),UiEvent::WindowPinFailed=>("warning","窗口","窗口置顶操作未完成"),
        UiEvent::UsageSettingsChanged=>("info","用量","已调整用量显示设置"),UiEvent::UsageSyncStarted=>("info","用量","开始手动同步用量记录"),UiEvent::UsageSyncSuccess=>("info","用量","手动用量同步已结束"),UiEvent::UsageSyncFailed=>("warning","用量","手动用量同步未完成"),
        UiEvent::PricesSyncStarted=>("info","价格","开始同步模型参考价格"),UiEvent::PricesSyncSuccess=>("info","价格","模型参考价格同步已结束"),UiEvent::PricesSyncFailed=>("warning","价格","模型参考价格同步未完成"),UiEvent::CurrencyChanged=>("info","用量","已切换费用显示币种"),
    };note(tx,level,category,message,"")
}
#[derive(Clone,Copy)]pub enum ReadState{Usage,Prices,Exchange,Memory,Connections,DesktopTarget,Connectivity}
static HEALTH:Mutex<[Option<bool>;7]>=Mutex::new([None;7]);
// First successes are silent; repeated errors do not fill the log every poll.
fn transition(state:&mut Option<bool>,failed:bool)->Option<bool>{let previous=state.replace(failed);if previous==Some(failed)||!failed&&previous.is_none(){None}else{Some(failed)}}
pub fn changed(key:ReadState,failed:bool)->Option<bool>{let mut states=HEALTH.lock().unwrap_or_else(|error|error.into_inner());transition(&mut states[key as usize],failed)}
pub fn read_state(tx:&Sender<Message>,key:ReadState,failed:bool,category:&'static str,failure:&'static str,recovery:&'static str){if let Some(failed)=changed(key,failed){let _=note(tx,if failed{"warning"}else{"info"},category,if failed{failure}else{recovery},"");}}
#[cfg(test)]mod tests{
    use super::*;
    #[test]fn closed_ui_vocabulary_rejects_free_text_and_never_carries_arguments(){
        assert!(serde_json::from_str::<UiEvent>("\"window_pin_on\"").is_ok());assert!(serde_json::from_str::<UiEvent>("\"PRIVATE_SESSION_TEXT\"").is_err());assert!(serde_json::from_str::<UiEvent>("{\"event\":\"usage_sync_success\",\"private\":\"secret\"}").is_err());
        let(tx,rx)=std::sync::mpsc::channel();ui(&tx,UiEvent::UsageSettingsChanged).unwrap();match rx.recv().unwrap(){Message::Notice(_,_,message,details)=>{assert_eq!(message,"已调整用量显示设置");assert!(details.is_empty());},_=>panic!("Expected the static coarse notice")}
    }
    #[test]fn polling_success_is_silent_and_error_recovery_is_recorded_once(){let mut state=None;let events=[false,false,true,true,true,false,false].into_iter().map(|failed|transition(&mut state,failed)).collect::<Vec<_>>();assert_eq!(events,vec![None,None,Some(true),None,None,Some(false),None]);}
}
