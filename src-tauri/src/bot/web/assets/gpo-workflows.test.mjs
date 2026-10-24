import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { runInContext } from 'node:vm';
import { afterEach, test } from 'node:test';
import assert from 'node:assert/strict';
const read = name => readFileSync(new URL('../' + name, import.meta.url),'utf8');
const opened=[];
afterEach(() => opened.splice(0).forEach(dom => dom.window.close()));
async function page(name, status={}) {
  const dom=new JSDOM(read(name+'.html'),{url:'http://localhost/',runScripts:'outside-only'}); opened.push(dom);
  const win=dom.window, timers=[], commands=[];
  win.setInterval=(fn,ms)=>{timers.push({fn,ms});return timers.length;};
  const payload={fish:0,fruits:0,runtime_s:0,success_rate:0,pity_fruit:0,pity_legendary:0,state:'paused',is_running:false,paused:true,stream_live:false,recent_catches:[],crafting:{is_crafting:false,crafted_count:0,state:'pending'},...status};
  win.fetch=async(path,options={})=>{
    if(options.method==='POST') commands.push({path,body:JSON.parse(options.body)});
    return {ok:true,status:200,json:async()=>payload};
  };
  runInContext(read('assets/gpo-core.js'),dom.getInternalVMContext());
  runInContext(read('assets/gpo-'+name+'.js'),dom.getInternalVMContext());
  await new Promise(resolve=>setTimeout(resolve,0));
  return {win,timers,commands,payload};
}
test('all five pages have no inline prototype scripts or executable event attributes',()=>{
  for(const name of ['remote','stats','studio','craft','system']) {
    const html=read(name+'.html');
    assert.doesNotMatch(html,/<script(?![^>]*\bsrc=)[^>]*>/i,name+' contains an inline script');
    assert.doesNotMatch(html,/\son(?:click|input|change)="/i,name+' contains an inline event handler');
  }
});
test('statistics runtime is derived only from the real snapshot after repeated timer ticks',async()=>{
  const {win,timers}=await page('stats');
  for(let i=0;i<5;i++) for(const timer of timers) await timer.fn();
  assert.equal(win.document.getElementById('session-stopwatch').textContent,'00:00:00');
});
test('remote fruit key follows the configured slot and Stop sends stop rather than pause',async()=>{
  const {win,commands}=await page('remote',{remote_keys:{fruit1:'9'}});
  win.document.getElementById('sfx-skill-btn').click();
  win.document.getElementById('stop-fishing-btn').click();
  await new Promise(resolve=>setTimeout(resolve,0));
  assert.ok(commands.some(c=>c.path==='/api/key' && c.body.key==='9'));
  assert.ok(commands.some(c=>c.path==='/api/action' && c.body.action==='stop'));
});
test('missing configuration never falls back to a hardcoded fruit key',async()=>{
  const {win,commands}=await page('remote');win.document.getElementById('sfx-skill-btn').click();
  await new Promise(resolve=>setTimeout(resolve,0));
  assert.equal(commands.filter(c=>c.path==='/api/key').length,0);
});
test('crafting sends the target to the PC and never enforces completion from a browser poll',async()=>{
  const {win,commands,timers,payload}=await page('craft');
  const slider=win.document.getElementById('batch-slider');slider.value='17';slider.dispatchEvent(new win.Event('input'));
  win.document.getElementById('start-craft-btn').click();await new Promise(resolve=>setTimeout(resolve,0));
  assert.ok(commands.some(c=>c.path==='/api/craft' && c.body.target_batches===17));
  payload.crafting={is_crafting:true,crafted_count:17,target_batches:17,state:'running'};
  for(const timer of timers) await timer.fn();
  assert.equal(commands.filter(c=>c.path==='/api/craft' && c.body.action==='stop').length,0);
  assert.match(win.document.getElementById('progress-counter').textContent,/batches attempted/);
});
