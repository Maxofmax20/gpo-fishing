import {useEffect,useState} from "react";
import {Circle,Play,Square,Save,Globe,Trash2} from "lucide-react";
import {api} from "../lib/ipc";
import {showToast} from "../lib/store";
import ConfirmModal from "../components/ConfirmModal";
import type {CustomMacro,RecorderStatus} from "../lib/types";

export default function Macros(){
 const [items,setItems]=useState<CustomMacro[]>([]);
 const [status,setStatus]=useState<RecorderStatus|null>(null);
 const [name,setName]=useState("");
 const [speed,setSpeed]=useState(1),[loops,setLoops]=useState(1);
 const [pending,setPending]=useState(false);
 const [deleting,setDeleting]=useState<CustomMacro|null>(null);
 async function refresh(){const [list,state]=await Promise.all([api.macroList(),api.macroStatus()]);setItems(list);setStatus(state);}
 useEffect(()=>{let active=true;const poll=()=>{if(!document.hidden&&active)refresh().catch(e=>showToast("error",String(e)));};poll();const timer=setInterval(poll,500);return()=>{active=false;clearInterval(timer);};},[]);
 async function act(action:()=>Promise<unknown>){if(pending)return;setPending(true);try{await action();await refresh();}catch(e){showToast("error",String(e));}finally{setPending(false);}}
 const draft=status?.record_mode!=null;
 const button="inline-flex items-center justify-center gap-2 rounded-xl border border-line px-3 py-2.5 text-sm hover:bg-white/5 disabled:opacity-40";
 return <div className="p-4 grid gap-4">
  <section className="rounded-2xl border border-line bg-bg-elev p-4 grid gap-3">
   <div><h2 className="text-base font-semibold">Game recorder</h2><p className="text-sm text-fg-dim mt-1">Pause fishing, record your actions in Roblox, then save and replay them.</p></div>
   <input className="rounded-xl border border-line bg-bg px-3 py-2" aria-label="Recording name" placeholder="Recording name" value={name} onChange={e=>setName(e.target.value)}/>
   <div className="flex gap-2 flex-wrap">
    <button className={button+" bg-accent-soft"} disabled={pending||draft||status?.is_playing} onClick={()=>act(()=>api.macroRecord("start",undefined,"pc"))}><Circle size={15}/>Record PC</button>
    <button className={button} disabled={pending||!draft} onClick={()=>act(()=>api.macroRecord("stop",name))}><Save size={15}/>Stop and save</button>
    <button className={button} disabled={pending||!draft} onClick={()=>act(()=>api.macroRecord("cancel"))}>Discard draft</button>
    <button className={button} onClick={()=>api.webDashboardUrl().then(api.openUrl).catch(e=>showToast("error",String(e)))}><Globe size={15}/>Web recorder</button>
   </div>
   <p role="status" className="text-sm text-fg-dim">{status?.message||"Ready"}{draft?" · "+status?.recorded_steps_count+" events":""}</p>
  </section>
  <section className="rounded-2xl border border-line bg-bg-elev p-4 grid gap-3">
   <div className="flex items-center justify-between"><h2 className="font-semibold">Playback</h2><button className={button+" text-bad"} disabled={!status?.is_playing} onClick={()=>api.macroPlay("stop").catch(e=>showToast("error",String(e)))}><Square size={15}/>Stop playback</button></div>
   <div className="flex flex-wrap gap-3"><label className="grid gap-1 text-sm">Speed<select className="rounded-xl bg-bg border border-line p-2" value={speed} onChange={e=>setSpeed(Number(e.target.value))}>{[1,2,5,10].map(n=><option key={n} value={n}>{n}×</option>)}</select></label>
   <label className="grid gap-1 text-sm">Repeat<select className="rounded-xl bg-bg border border-line p-2" value={loops} onChange={e=>setLoops(Number(e.target.value))}><option value={1}>Once</option><option value={10}>10 loops</option><option value={0}>Until stopped</option></select></label></div>
   {items.length===0&&<p className="text-fg-dim">Your saved recordings will appear here.</p>}
   {items.map(item=><div key={item.id} className="rounded-xl border border-line p-3 flex gap-2 items-center"><div className="flex-1 min-w-0"><div className="truncate font-medium">{item.name}</div><small className="text-fg-dim">{item.steps.length} steps · existing recordings preserved</small></div><button className={button} disabled={pending||status?.is_playing||draft} onClick={()=>act(()=>api.macroPlay("play",item.id,loops!==1,speed,loops))} aria-label={"Play "+item.name}><Play size={15}/></button><button className={button} disabled={pending||status?.is_playing} aria-label={"Delete "+item.name} onClick={()=>setDeleting(item)}><Trash2 size={15}/></button></div>)}
  </section>
  <ConfirmModal open={deleting!==null} title="Delete recording?" body={"Remove "+(deleting?.name||"this recording")+" from your saved list?"} confirmLabel="Delete" busy={pending} onCancel={()=>setDeleting(null)} onConfirm={()=>{if(deleting)void act(async()=>{await api.macroDelete(deleting.id);setDeleting(null);});}}/>
 </div>;
}
