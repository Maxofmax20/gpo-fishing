import AxeBuilder from "@axe-core/playwright";
import {test,expect} from "@playwright/test";
import {readFile} from "node:fs/promises";
import {resolve,extname} from "node:path";
const root=resolve("src-tauri/src/bot/web");
for(const width of [320,360,768,1280]){
 test("rebuilt remote controls at "+width,async({page})=>{
  await page.setViewportSize({width,height:900});
  const errors:string[]=[],calls:{path:string,body:any}[]=[];let offline=false,failSave=false;
  const notifications:Record<string,any>={provider:"telegram",progress_every_n:10,remote_enabled:true};
  for(const key of ["enabled","progress","fruit_drop","spawn","purchase","recovery","legendary_only","pity_zero_only","send_screenshot","send_catch_screenshot","send_drop_screenshot","crop_fruit_screenshot","disconnect_alert","bait_alert"])notifications[key]=false;
  page.on("pageerror",e=>errors.push(e.message));
  await page.route("http://gpo.test/**",async route=>{
   const url=new URL(route.request().url());
   if(url.pathname.startsWith("/api/")){
    if(url.pathname==="/api/stream"){await route.abort();return;}
    expect(route.request().headers().authorization).toBe("Bearer synthetic-test-token");
    if(offline){await route.abort();return;}
    if(url.pathname==="/api/notifications/settings"){
      if(route.request().method()==="POST"){
        const body=route.request().postDataJSON();calls.push({path:url.pathname,body});
        if(failSave){await route.fulfill({status:400,json:{ok:false,error:"Disk save failed"}});return;}
        for(const [key,value]of Object.entries(body.changes)){expect(body.expected[key]).toBe(notifications[key]);notifications[key]=value;}
      }
      await route.fulfill({json:{ok:true,settings:notifications,telegram_configured:true}});return;
    }
    if(url.pathname==="/api/notifications/status"){await route.fulfill({json:{ok:true,delivered:0,pending:0,failed:0}});return;}
    if(route.request().method()==="POST"){
      calls.push({path:url.pathname,body:route.request().postDataJSON()});
      await route.fulfill({json:{ok:true}});return;
    }
    await route.fulfill({json:{ok:true,control_epoch:7,state:"Paused",window:{is_foreground:true},recorder:{message:"Ready"},stream_fps:28,macros:[{id:"r2_test",name:"My recording"}]}});return;
   }
   const file=resolve(root,url.pathname.startsWith("/assets/")?url.pathname.slice(1):"index.html");
   await route.fulfill({body:await readFile(file),contentType:({".html":"text/html",".css":"text/css",".js":"text/javascript"} as Record<string,string>)[extname(file)]});
  });
  await page.goto("http://gpo.test/?token=synthetic-test-token");
  await expect(page.locator("#connection")).toHaveText("Roblox focused");
  expect(page.url()).not.toContain("token=");
  expect(await page.locator("#fps option").allTextContents()).toEqual(["20","30","60","120"]);
  await page.getByRole("button",{name:"Jump · Space"}).click();
  await expect.poll(()=>calls.some(c=>c.path==="/api/tap"&&c.body.key==="space"&&c.body.epoch===7)).toBe(true);
  await page.getByRole("button",{name:"Camera Right",exact:true}).hover();await page.mouse.down();
  await expect.poll(()=>calls.some(c=>c.path==="/api/camera"&&c.body.direction==="right"&&c.body.epoch===7)).toBe(true);
  await page.mouse.up();
  await page.getByRole("checkbox",{name:"Enable notifications",exact:true}).check();
  await expect.poll(()=>notifications.enabled).toBe(true);
  await expect(page.locator("#notification-fields")).toBeEnabled();
  await page.reload();
  await expect(page.getByRole("checkbox",{name:"Enable notifications",exact:true})).toBeChecked();
  failSave=true;
  await page.getByRole("checkbox",{name:"Fishing progress",exact:true}).check();
  await expect(page.getByRole("checkbox",{name:"Fishing progress",exact:true})).not.toBeChecked();
  failSave=false;
  await page.getByRole("button",{name:"Play",exact:true}).click();
  await expect.poll(()=>calls.some(c=>c.path==="/api/recorder"&&c.body.command==="play"&&c.body.id==="r2_test")).toBe(true);
  await page.locator("#fps").selectOption("120");
  await expect(page.locator("#screen")).toHaveAttribute("src",/fps=120/);
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  const audit=await new AxeBuilder({page}).withTags(["wcag2a","wcag2aa"]).analyze();
  expect(audit.violations.filter(v=>["critical","serious"].includes(v.impact||""))).toEqual([]);
  offline=true;
  await expect(page.locator("#connection")).toHaveText("Disconnected",{timeout:4000});
  offline=false;
  await expect(page.locator("#connection")).toHaveText("Roblox focused",{timeout:4000});
  expect(errors).toEqual([]);
 });
}
