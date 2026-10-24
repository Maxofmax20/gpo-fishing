//! Bounded local fruit-result outbox. Credentials are read only at delivery time,
//! never serialized here. Provider acknowledgements prevent known-success replay;
//! an uncertain remote acknowledgement can still cause a duplicate.
use serde::{Deserialize,Serialize};
use std::path::{Path,PathBuf};
use crate::webhook::Notification;

const MAX_RECORD_BYTES:u64=8*1024*1024;
const MAX_TOTAL_BYTES:u64=32*1024*1024;
const MAX_RECORDS:usize=128;
const MAX_AGE_MS:u64=24*60*60*1000;

#[derive(Clone,Serialize,Deserialize)]
pub struct Record {
    pub id:String, pub created_ms:u64, pub pity_zero:bool,
    pub notification:Notification, pub discord_done:bool,
    pub telegram_done:bool, pub completed:bool,
}
pub struct Outbox {dir:PathBuf}
fn now()->u64 {crate::events::now_ms()}
impl Outbox {
    pub fn new(dir:PathBuf)->Self {Self{dir}}
    fn path(&self,id:&str)->Result<PathBuf,String> {
        if id.is_empty() || id.len()>256 {return Err("Invalid outcome identifier".into());}
        // Fixed stable digest avoids path traversal and version-dependent hashing.
        let hash=id.bytes().fold(14695981039346656037u64,|h,b|(h^b as u64).wrapping_mul(1099511628211));
        Ok(self.dir.join(format!("{hash:016x}.json")))
    }
    fn read(path:&Path)->Option<Record> {
        let len=path.metadata().ok()?.len();
        if len>MAX_RECORD_BYTES {return None;}
        serde_json::from_slice(&std::fs::read(path).ok()?).ok()
    }
    fn records(&self)->Vec<(PathBuf,Record,u64)> {
        let mut records=Vec::new();
        if let Ok(entries)=std::fs::read_dir(&self.dir) {
            for entry in entries.flatten().take(MAX_RECORDS+1) {
                let path=entry.path();
                if path.extension().and_then(|s|s.to_str())!=Some("json") {continue;}
                if let Some(record)=Self::read(&path) {
                    records.push((path,record,entry.metadata().map_or(0,|m|m.len())));
                }
            }
        }
        records
    }
    pub fn pending(&self)->Vec<Record> {
        self.records().into_iter().map(|(_,r,_)|r)
            .filter(|r|!r.completed && now().saturating_sub(r.created_ms)<MAX_AGE_MS)
            .take(MAX_RECORDS).collect()
    }
    pub fn load_or_create(&self,id:&str,pity_zero:bool,notification:&Notification)->Result<Record,String> {
        let path=self.path(id)?;
        if let Some(record)=Self::read(&path) {
            if record.id!=id {return Err("Outcome identifier collision".into());}
            return Ok(record);
        }
        if path.exists() {return Err("Existing outcome record is unreadable; retained for review".into());}
        let record=Record{id:id.into(),created_ms:now(),pity_zero,notification:notification.clone(),discord_done:false,telegram_done:false,completed:false};
        self.save(&record)?;
        Ok(record)
    }
    pub fn save(&self,record:&Record)->Result<(),String> {
        let path=self.path(&record.id)?;
        let bytes=serde_json::to_vec(record).map_err(|_|"Could not serialize outcome")?;
        if bytes.len() as u64>MAX_RECORD_BYTES {return Err("Outcome exceeds storage budget".into());}
        std::fs::create_dir_all(&self.dir).map_err(|_|"Could not create outbox")?;
        let records=self.records();
        let old_size=path.metadata().map_or(0,|m|m.len());
        let total:u64=records.iter().map(|(_,_,size)|size).sum();
        let mut count=records.len();let mut used=total.saturating_sub(old_size);
        // Prune only our completed delivery receipts, never pending outcomes,
        // user catches, recordings, datasets, models or configuration.
        let mut completed:Vec<_>=records.into_iter().filter(|(p,r,_)|r.completed && *p!=path).collect();
        completed.sort_by_key(|(_,r,_)|r.created_ms);
        for (file,_,size) in completed {
            if (old_size>0 || count<MAX_RECORDS) && used+bytes.len() as u64<=MAX_TOTAL_BYTES {break;}
            if std::fs::remove_file(file).is_ok() {count=count.saturating_sub(1);used=used.saturating_sub(size);}
        }
        if (old_size==0 && count>=MAX_RECORDS) || used+bytes.len() as u64>MAX_TOTAL_BYTES {
            return Err("Outbox storage budget reached; pending records retained".into());
        }
        let temp=path.with_extension("tmp");
        std::fs::write(&temp,bytes).map_err(|_|"Could not write outcome checkpoint")?;
        std::fs::rename(temp,path).map_err(|_|"Could not commit outcome checkpoint")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture()->(Outbox,PathBuf) {
        static NEXT:std::sync::atomic::AtomicU64=std::sync::atomic::AtomicU64::new(0);
        let dir=std::env::temp_dir().join(format!("gpo-outbox-{}-{}",std::process::id(),NEXT.fetch_add(1,std::sync::atomic::Ordering::Relaxed)));
        (Outbox::new(dir.clone()),dir)
    }
    fn notification()->Notification {
        Notification{title:"Stored".into(),desc:"Fruit result".into(),color:0,fields:vec![],photo:Some(vec![1,2,3]),custom_telegram_html:None}
    }
    #[test]
    fn retry_survives_restart_and_preserves_per_provider_acknowledgement() {
        let (outbox,dir)=fixture();
        let mut record=outbox.load_or_create("workflow-a",true,&notification()).unwrap();
        record.discord_done=true;outbox.save(&record).unwrap();
        let restarted=Outbox::new(dir.clone());
        let pending=restarted.pending();assert_eq!(pending.len(),1);
        assert!(pending[0].discord_done);assert!(!pending[0].telegram_done);
        record.telegram_done=true;record.completed=true;record.notification.photo=None;
        restarted.save(&record).unwrap();
        assert!(restarted.pending().is_empty());
        assert!(restarted.load_or_create("workflow-a",true,&notification()).unwrap().completed);
        let text=std::fs::read_to_string(restarted.path("workflow-a").unwrap()).unwrap();
        assert!(!text.contains("telegram_bot_token"));assert!(!text.contains("webhook_url"));
        let _=std::fs::remove_dir_all(dir);
    }
    #[test]
    fn identifiers_cannot_escape_outbox_and_expired_pending_work_is_not_replayed() {
        let (outbox,dir)=fixture();
        let mut record=outbox.load_or_create("../../private",true,&notification()).unwrap();
        assert_eq!(outbox.path(&record.id).unwrap().parent(),Some(dir.as_path()));
        record.created_ms=0;outbox.save(&record).unwrap();assert!(outbox.pending().is_empty());
        assert!(outbox.load_or_create("",true,&notification()).is_err());
        let _=std::fs::remove_dir_all(dir);
    }
    #[test]
    fn unreadable_existing_records_are_retained_without_overwrite() {
        let (outbox,dir)=fixture();std::fs::create_dir_all(&dir).unwrap();
        let path=outbox.path("a").unwrap();std::fs::write(&path,b"broken").unwrap();
        assert!(outbox.load_or_create("a",true,&notification()).is_err());
        assert_eq!(std::fs::read(path).unwrap(),b"broken");
        let _=std::fs::remove_dir_all(dir);
    }
}
