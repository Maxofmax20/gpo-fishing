import AxeBuilder from "@axe-core/playwright";
import {test,expect} from "@playwright/test";
import {readFile} from "node:fs/promises";
import {resolve,extname} from "node:path";
const root=resolve("src-tauri/src/bot/web");
for(const width of [320,360,768,1280]){
 test("rebuilt remote controls at "+width,async({page})=>{
  await page.setViewportSize({width,height:900});
  const errors:string[]=[],calls:{path:string,body:any}[]=[];let offline=false;
  page.on("pageerror",e=>errors.push(e.message));
  await page.route("http://gpo.test/**",async route=>{
   const url=new URL(route.request().url());
   if(url.pathname.startsWith("/api/")){
    if(url.pathname==="/api/stream"){await route.abort();return;}
    expect(route.request().headers().authorization).toBe("Bearer synthetic-test-token");
    if(offline){await route.abort();return;}
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
