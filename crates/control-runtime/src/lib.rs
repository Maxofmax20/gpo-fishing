//! Shared recorder and remote-input contract. No dependency on fishing state.
use serde::{Deserialize,Serialize};
use std::sync::{Arc,Mutex};
use std::time::{Duration,Instant};

pub const STREAM_RATES:[u32;4]=[20,30,60,120];
#[derive(Debug,Clone,Serialize,Deserialize,PartialEq)]
#[serde(tag="action",rename_all="snake_case")]
pub enum InputAction {
    KeyDown{key:String},KeyUp{key:String},
    Move{x:f32,y:f32},
    MouseDown{button:String},MouseUp{button:String},
    Wheel{amount:i32},
}
impl InputAction {
    pub fn validate(&self)->Result<(),String> {
        match self {
            Self::KeyDown{key}|Self::KeyUp{key} if !valid_key(key)=>Err("Unsupported game key".into()),
            Self::Move{x,y} if !x.is_finite()||!y.is_finite()||!(0.0..=1.0).contains(x)||!(0.0..=1.0).contains(y)=>Err("Pointer coordinates must be between 0 and 1".into()),
            Self::MouseDown{button}|Self::MouseUp{button} if !matches!(button.as_str(),"left"|"right")=>Err("Mouse button must be left or right".into()),
            Self::Wheel{amount} if !(-1200..=1200).contains(amount)=>Err("Wheel amount is out of range".into()),
            _=>Ok(()),
        }
    }
}
pub fn valid_key(key:&str)->bool {
    let k=key.to_ascii_lowercase();
    (k.len()==1 && k.as_bytes()[0].is_ascii_alphanumeric())
        || matches!(k.as_str(),"space"|"shift"|"ctrl"|"enter"|"escape"|"backspace"|"delete"|"left"|"right"|"up"|"down")

}
#[derive(Debug,Clone,Serialize,Deserialize,PartialEq)]
pub struct TimedInput {pub at_ms:u64,pub input:InputAction}
pub fn validate_recording(events:&[TimedInput])->Result<(),String> {
    if events.is_empty(){return Err("Recording has no input events".into());}
    if events.len()>100_000{return Err("Recording exceeds 100000 input events".into());}
    let mut last=0;
    for event in events {
        if event.at_ms<last||event.at_ms>86_400_000{return Err("Recording timestamps are invalid".into());}
        event.input.validate()?;last=event.at_ms;
    }
    Ok(())
}
pub trait InputPort:Send+Sync {
    fn focused(&self)->bool;
    fn send(&self,input:&InputAction)->Result<(),String>;
    fn release_all(&self);
}
pub struct Runtime {port:Arc<dyn InputPort>,epoch:Mutex<u64>}
impl Runtime {
    pub fn new(port:Arc<dyn InputPort>)->Self{Self{port,epoch:Mutex::new(0)}}
    pub fn begin(&self)->u64 {let mut epoch=self.epoch.lock().unwrap();*epoch=epoch.wrapping_add(1);self.port.release_all();*epoch}
    pub fn epoch(&self)->u64 {*self.epoch.lock().unwrap()}
    pub fn active(&self,epoch:u64)->bool {self.epoch()==epoch}
    pub fn finish(&self, expected:u64) {let mut epoch=self.epoch.lock().unwrap();if *epoch==expected{*epoch=epoch.wrapping_add(1);self.port.release_all();}}
    pub fn stop(&self) {let mut epoch=self.epoch.lock().unwrap();*epoch=epoch.wrapping_add(1);self.port.release_all();}
    pub fn apply(&self,epoch:u64,input:&InputAction)->Result<(),String>{
        input.validate()?;
        let current=self.epoch.lock().unwrap();
        if *current!=epoch{return Err("Input session was cancelled".into());}
        let release=matches!(input,InputAction::KeyUp{..}|InputAction::MouseUp{..});
        if !release&&!self.port.focused(){self.port.release_all();return Err("Roblox is not the active window".into());}
        self.port.send(input)
    }
    pub fn sleep(&self,epoch:u64,duration:Duration)->bool {
        let end=Instant::now()+duration;
        while Instant::now()<end {
            if !self.active(epoch)||!self.port.focused(){return false;}
            std::thread::sleep(end.saturating_duration_since(Instant::now()).min(Duration::from_millis(5)));
        }
        self.active(epoch)
    }
    pub fn play(&self,epoch:u64,events:&[TimedInput],speed:f32,loops:u32,mut progress:impl FnMut(u32))->Result<(),String>{
        validate_recording(events)?;
        if !speed.is_finite()||!(0.1..=100.0).contains(&speed){return Err("Speed must be between 0.1 and 100".into());}
        struct Finish<'a>(&'a Runtime,u64);
        impl Drop for Finish<'_>{fn drop(&mut self){self.0.finish(self.1);}}
        let _finish=Finish(self,epoch);
        let mut iteration=0u32;
        loop {
            iteration=iteration.saturating_add(1);progress(iteration);
            let started=Instant::now();
            for event in events {
                let target=Duration::from_secs_f64(event.at_ms as f64/1000.0/f64::from(speed));
                if !self.sleep(epoch,target.saturating_sub(started.elapsed())){return Err("Playback stopped or Roblox lost focus".into());}
                self.apply(epoch,&event.input)?;
            }
            if loops!=0&&iteration>=loops{break;}
            if !self.sleep(epoch,Duration::from_millis(1)){return Err("Playback stopped".into());}
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool,AtomicUsize,Ordering};
    struct Port{focus:AtomicBool,events:Mutex<Vec<InputAction>>,releases:AtomicUsize}
    impl InputPort for Port{
        fn focused(&self)->bool{self.focus.load(Ordering::SeqCst)}
        fn send(&self,input:&InputAction)->Result<(),String>{self.events.lock().unwrap().push(input.clone());Ok(())}
        fn release_all(&self){self.releases.fetch_add(1,Ordering::SeqCst);}
    }
    fn setup()->(Runtime,Arc<Port>){let p=Arc::new(Port{focus:AtomicBool::new(true),events:Mutex::new(vec![]),releases:AtomicUsize::new(0)});(Runtime::new(p.clone()),p)}
    fn tap()->Vec<TimedInput>{vec![TimedInput{at_ms:0,input:InputAction::KeyDown{key:"space".into()}},TimedInput{at_ms:10,input:InputAction::KeyUp{key:"space".into()}}]}
    #[test]fn jump_has_down_and_up(){let(rt,p)=setup();let e=rt.begin();rt.play(e,&tap(),10.0,1,|_|{}).unwrap();assert_eq!(p.events.lock().unwrap().len(),2);}
    #[test]fn stop_invalidates_all_queued_inputs(){let(rt,p)=setup();let e=rt.begin();rt.stop();assert!(rt.apply(e,&tap()[0].input).is_err());assert!(p.events.lock().unwrap().is_empty());}
    #[test]fn missing_focus_blocks_down_but_allows_release(){let(rt,p)=setup();let e=rt.begin();p.focus.store(false,Ordering::SeqCst);assert!(rt.apply(e,&tap()[0].input).is_err());assert!(rt.apply(e,&tap()[1].input).is_ok());}
    #[test]fn invalid_recording_has_zero_side_effects(){let(rt,p)=setup();let e=rt.begin();let bad=vec![TimedInput{at_ms:0,input:InputAction::Move{x:f32::NAN,y:0.0}}];assert!(rt.play(e,&bad,1.0,1,|_|{}).is_err());assert!(p.events.lock().unwrap().is_empty());}
    #[test]fn loops_and_speed_do_not_depend_on_fishing(){let(rt,p)=setup();let e=rt.begin();rt.play(e,&tap(),100.0,3,|_|{}).unwrap();assert_eq!(p.events.lock().unwrap().len(),6);}
    #[test]fn non_monotonic_timestamps_rejected(){let mut a=tap();a[0].at_ms=20;assert!(validate_recording(&a).is_err());}
    #[test]fn supported_rates_are_exact(){assert_eq!(STREAM_RATES,[20,30,60,120]);assert!(Duration::from_secs_f64(1.0/120.0)<Duration::from_millis(10));}
}
