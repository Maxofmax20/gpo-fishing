(() => {
'use strict';
const params=new URLSearchParams(location.search);
const token=params.get('token')||sessionStorage.getItem('gpo-dashboard-token')||'';
if(token)sessionStorage.setItem('gpo-dashboard-token',token);
if(params.has('token')){params.delete('token');history.replaceState(null,'',location.pathname+(params.size?'?'+params:''));}
const session=(globalThis.crypto?.randomUUID?.()||Date.now().toString(36)+Math.random().toString(36).slice(2));
const el=id=>document.getElementById(id);
let epoch=null,inputOrder=Promise.resolve(),localEpoch=0;
let held=new Map(),polling=false,streamTimer=null,messageTimer=null;
async function api(path,body){
 const response=await fetch(path+(path.includes('?')?'&':'?')+'session='+encodeURIComponent(session),{method:body?'POST':'GET',headers:{Authorization:'Bearer '+token,...(body?{'Content-Type':'application/json'}:{})},body:body?JSON.stringify(body):undefined,cache:'no-store'});
 const data=await response.json();if(!response.ok||data.ok===false)throw Error(data.error||'Request failed');return data;
}
function message(text){el('message').textContent=text;el('message').classList.add('visible');clearTimeout(messageTimer);messageTimer=setTimeout(()=>el('message').classList.remove('visible'),6000);}
function run(fn){Promise.resolve().then(fn).catch(error=>message(error.message));}
function focus(){return el('auto-focus').checked;}
function input(path,body){
 const expected=localEpoch;const value={...body,epoch};
 inputOrder=inputOrder.catch(()=>{}).then(()=>{if(expected!==localEpoch)return;return api(path,value);});
 return inputOrder;
}

function stream(){
 clearTimeout(streamTimer);if(document.hidden)return;
 const image=el('screen');image.src='/api/stream?token='+encodeURIComponent(token)+'&session='+encodeURIComponent(session)+'&fps='+el('fps').value+'&scale='+el('scale').value;
}
el('screen').onload=()=>{el('screen-error').hidden=true;};
el('screen').onerror=()=>{el('screen-error').hidden=false;streamTimer=setTimeout(stream,1500);};
['fps','scale'].forEach(id=>el(id).onchange=stream);
document.querySelectorAll('[data-control]').forEach(button=>button.onclick=()=>run(async()=>{
 if(button.dataset.control==='stop'||button.dataset.control==='pause')localEpoch++;await api('/api/control',{command:button.dataset.control});await status();if(button.dataset.control==='focus')stream();
}));
document.querySelectorAll('[data-key]').forEach(button=>button.onclick=()=>run(()=>input('/api/tap',{key:button.dataset.key,focus:focus()})));
function bindHold(button,key){
 const up=()=>{const timer=held.get(key);clearInterval(timer);held.delete(key);run(()=>input('/api/input',{input:{action:'key_up',key},focus:false}));};
 button.onpointerdown=event=>{event.preventDefault();button.setPointerCapture(event.pointerId);if(held.has(key))return;const down=()=>run(()=>input('/api/input',{input:{action:'key_down',key},focus:focus()}));down();held.set(key,setInterval(down,250));};
 button.onpointerup=up;button.onpointercancel=up;button.onlostpointercapture=()=>{if(held.has(key))up();};
}
document.querySelectorAll('[data-hold]').forEach(button=>bindHold(button,button.dataset.hold));
for(let slot=1;slot<=9;slot++){const button=document.createElement('button');button.textContent=String(slot);button.setAttribute('aria-label','Equip hotbar slot '+slot);button.onclick=()=>run(()=>input('/api/tap',{key:String(slot),focus:focus()}));el('slots').append(button);}
el('record').onclick=()=>run(async()=>{await api('/api/recorder',{command:'record'});message('Recording web controls');});
el('save').onclick=()=>run(async()=>{await api('/api/recorder',{command:'save',name:''});await recordings();});
el('discard').onclick=()=>run(()=>api('/api/recorder',{command:'discard'}));
el('stop-playback').onclick=()=>run(()=>api('/api/recorder',{command:'stop'}));
el('test-notification').onclick=()=>run(async()=>{await api('/api/notifications/test',{});message('Test delivery succeeded');});
async function recordings(){
 const result=await api('/api/macros');el('recordings').replaceChildren();
 for(const item of result.macros){const row=document.createElement('div');row.className='recording';const title=document.createElement('span');title.textContent=item.name;
 const button=document.createElement('button');button.textContent='Play';button.onclick=()=>run(()=>api('/api/recorder',{command:'play',id:item.id,speed:Number(el('speed').value),loop:Number(el('loops').value)!==1,loops:Number(el('loops').value)}));row.append(title,button);el('recordings').append(row);}
}
async function status(){
 if(polling||document.hidden)return;polling=true;
 try{const data=await api('/api/status');epoch=data.control_epoch;el('connection').textContent=data.window?.is_foreground?'Roblox focused':'Roblox needs focus';el('state').textContent=typeof data.state==='string'?data.state:JSON.stringify(data.state);el('recorder-state').textContent=data.recorder.message||'Ready';el('actual').textContent=data.stream_fps==null?'Waiting for frames':data.stream_fps.toFixed(0)+' FPS delivered';}
 catch(error){el('connection').textContent='Disconnected';}finally{polling=false;}
}
const screen=el('screen');
let pointer=null,mousePulse=null;
function point(event){
 const box=screen.getBoundingClientRect(),nw=screen.naturalWidth,nh=screen.naturalHeight;
 if(!nw||!nh)return null;
 const scale=Math.min(box.width/nw,box.height/nh),w=nw*scale,h=nh*scale;
 const x=(event.clientX-box.left-(box.width-w)/2)/w,y=(event.clientY-box.top-(box.height-h)/2)/h;
 return x>=0&&x<=1&&y>=0&&y<=1?{x,y}:null;
}
screen.style.touchAction='none';
screen.oncontextmenu=event=>event.preventDefault();
screen.onpointerdown=event=>{
 const p=point(event);if(!p)return;event.preventDefault();screen.setPointerCapture(event.pointerId);
 pointer=event.button===2?'right':'left';
 run(()=>input('/api/input',{input:{action:'move',...p},focus:focus()}));
 run(()=>input('/api/input',{input:{action:'mouse_down',button:pointer},focus:false}));
 mousePulse=setInterval(()=>{if(pointer)run(()=>input('/api/input',{input:{action:'mouse_down',button:pointer},focus:false}));},250);
};
let movedAt=0;
screen.onpointermove=event=>{if(!pointer||Date.now()-movedAt<25)return;const p=point(event);if(p){movedAt=Date.now();run(()=>input('/api/input',{input:{action:'move',...p},focus:false}));}};
function mouseUp(){clearInterval(mousePulse);if(pointer){const button=pointer;pointer=null;run(()=>input('/api/input',{input:{action:'mouse_up',button},focus:false}));}}
screen.onpointerup=mouseUp;screen.onpointercancel=mouseUp;screen.onlostpointercapture=mouseUp;
window.addEventListener('blur',mouseUp);
window.addEventListener('blur',()=>{for(const key of held.keys()){clearInterval(held.get(key));run(()=>input('/api/input',{input:{action:'key_up',key},focus:false}));}held.clear();});
document.addEventListener('visibilitychange',()=>{if(document.hidden){el('screen').src='';clearTimeout(streamTimer);window.dispatchEvent(new Event('blur'));}else{stream();status();}});
stream();status();recordings().catch(error=>message(error.message));setInterval(status,1000);
})();
