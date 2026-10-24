import { beforeEach, expect, it, vi } from "vitest";
const mocks=vi.hoisted(()=>({check:vi.fn(),toast:vi.fn(),invoke:vi.fn()}));
vi.mock("@tauri-apps/plugin-updater",()=>({check:mocks.check}));
vi.mock("@tauri-apps/api/core",()=>({invoke:mocks.invoke}));
vi.mock("./store",()=>({showToast:mocks.toast}));
import {useUpdater} from "./updater";
beforeEach(()=>{
 vi.clearAllMocks();
 mocks.invoke.mockResolvedValue("handoff-one");
 useUpdater.setState({available:null,busy:false,message:null,dismissedVersion:null});
});
it("checking alone never downloads, installs or arms resume",async()=>{
 const update={version:"6.0.0",download:vi.fn(),install:vi.fn(),close:vi.fn()};
 mocks.check.mockResolvedValue(update);
 await useUpdater.getState().check();
 expect(update.download).not.toHaveBeenCalled();
 expect(update.install).not.toHaveBeenCalled();
 expect(mocks.invoke).not.toHaveBeenCalled();
});
it("downloads while fishing then prepares a safe checkpoint before installation",async()=>{
 const order:string[]=[];
 const messages:(string|null)[]=[];
 const update={version:"6.0.0",download:vi.fn(async(progress:any)=>{
   order.push("download");progress({event:"Started",data:{contentLength:100}});
   messages.push(useUpdater.getState().message);
   progress({event:"Progress",data:{chunkLength:40}});messages.push(useUpdater.getState().message);
   progress({event:"Finished"});messages.push(useUpdater.getState().message);
 }),install:vi.fn(async()=>{order.push("install");})};
 mocks.invoke.mockImplementation(async(name)=>{order.push(name);return "one";});
 useUpdater.setState({available:update as any});
 await useUpdater.getState().install();
 expect(order).toEqual(["download","update_prepare","install"]);
 expect(messages).toEqual(["Downloading 0%","Downloading 40%","Download complete; verifying…"]);
 expect(mocks.invoke).toHaveBeenCalledWith("update_prepare",{version:"6.0.0"});
 expect(useUpdater.getState().busy).toBe(false);
});
it("a failed download leaves the running session untouched",async()=>{
 const update={version:"6.0.0",download:vi.fn().mockRejectedValue(Error("offline")),install:vi.fn()};
 useUpdater.setState({available:update as any});
 await useUpdater.getState().install();
 expect(mocks.invoke).not.toHaveBeenCalled();expect(update.install).not.toHaveBeenCalled();
 expect(useUpdater.getState().message).toContain("offline");expect(useUpdater.getState().busy).toBe(false);
});
it("Stop during boundary wait cancels installation",async()=>{
 const update={version:"6.0.0",download:vi.fn(),install:vi.fn()};
 mocks.invoke.mockRejectedValueOnce(Error("Cancelled by Stop"));
 useUpdater.setState({available:update as any});
 await useUpdater.getState().install();
 expect(update.install).not.toHaveBeenCalled();expect(useUpdater.getState().message).toContain("Cancelled");
});
it("installer failure revokes the checkpoint and requests old-session recovery",async()=>{
 const update={version:"6.0.0",download:vi.fn(),install:vi.fn().mockRejectedValue(Error("installer failed"))};
 useUpdater.setState({available:update as any});
 await useUpdater.getState().install();
 expect(mocks.invoke).toHaveBeenCalledWith("update_cancel",{requestId:"handoff-one"});
 expect(useUpdater.getState().busy).toBe(false);
});
