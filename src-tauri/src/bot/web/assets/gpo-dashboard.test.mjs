import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";
import { afterEach, describe, it } from "node:test";
import assert from "node:assert/strict";
import { runInContext } from "node:vm";

const asset = (name) => readFileSync(new URL(`../${name}`, import.meta.url), "utf8");
const open = [];
async function dashboard(streamLive = true) {
  const dom = new JSDOM(asset("remote.html"), { url: "http://localhost/", runScripts: "outside-only" });
  open.push(dom);
  const win = dom.window;
  win.setInterval = (() => 0);
  win.fetch = async () => ({ ok: true, json: async () => ({ stream_live: streamLive, state: "idle", fish: 0 }) });
  runInContext(asset("assets/gpo-core.js"), dom.getInternalVMContext());
  runInContext(asset("assets/gpo-remote.js"), dom.getInternalVMContext());
  // jsdom dispatches DOMContentLoaded once asynchronously after construction.
  await new Promise((resolve) => setTimeout(resolve, 0));
  return win;
}
afterEach(() => { open.splice(0).forEach((dom) => dom.window.close()); });

describe("remote dashboard connection evidence", () => {
  it("shares slow reads, passes a deadline, and allows a fresh read after completion", async () => {
    const dom = new JSDOM("<main></main>", { url: "http://localhost/", runScripts: "outside-only" });
    open.push(dom);
    const win = dom.window;
    let complete;
    let calls = 0;
    win.fetch = async (_url, options) => {
      calls++;
      assert.ok(options.signal, "slow requests need an abort deadline");
      return new Promise(resolve => { complete = () => resolve({ ok: true, json: async () => ({ fish: calls }) }); });
    };
    runInContext(asset("assets/gpo-core.js"), dom.getInternalVMContext());
    const first = win.gpoGet("/api/status");
    const second = win.gpoGet("/api/status");
    assert.equal(calls, 1);
    complete();
    const results = await Promise.all([first, second]);
    assert.equal(results[0].data.fish, 1);
    assert.equal(results[1].data.fish, 1);
    const third = win.gpoGet("/api/status");
    assert.equal(calls, 2);
    complete();
    assert.equal((await third).data.fish, 2);
  });

  it("does not retain a failed read when the backend reconnects", async () => {
    const win = await dashboard();
    win.fetch = async () => { throw new Error("disconnected"); };
    await assert.rejects(win.gpoGet("/api/status"), /backend unreachable/);
    win.fetch = async () => ({ ok: true, json: async () => ({ fish: 7 }) });
    assert.equal((await win.gpoGet("/api/status")).data.fish, 7);
  });

  it("removes LIVE and covers stale pixels when the backend disconnects", async () => {
    const win = await dashboard();
    assert.ok(win.document.getElementById("header-live-pill")?.textContent.includes("LIVE"));
    win.gpoNoteOnline(false);
    assert.ok(win.document.getElementById("header-live-pill")?.textContent.includes("OFFLINE"));
    assert.equal(win.document.getElementById("pwr-btn").disabled, true);
    assert.equal(win.document.getElementById("main-macro-btn").disabled, true);
    assert.ok(win.document.getElementById("gpo-stream-overlay")?.textContent.includes("Dashboard disconnected"));
    const warning = win.document.getElementById("gpo-offline-banner");
    assert.equal(warning.nextElementSibling?.tagName, "MAIN");
    assert.equal(warning.getAttribute("aria-live"), "polite");
    win.gpoNoteOnline(true);
    assert.equal(warning.style.display, "none");
    assert.ok(win.document.getElementById("header-live-pill")?.textContent.includes("LIVE"));
    assert.equal(win.document.getElementById("main-macro-btn").disabled, false);
  });

  it("does not label a connected dashboard with no captured game as LIVE", async () => {
    const win = await dashboard(false);
    assert.ok(win.document.getElementById("header-live-pill")?.textContent.includes("NO STREAM"));
    assert.ok(win.document.getElementById("gpo-stream-overlay")?.textContent.includes("Roblox not detected"));
  });

  it("keeps failed stream pixels hidden and explains how to retry", async () => {
    const win = await dashboard();
    const img = win.document.getElementById("gpo-stream");
    img.dispatchEvent(new win.Event("error"));
    assert.equal(img.style.display, "none");
    assert.ok(win.document.getElementById("header-live-pill")?.textContent.includes("NO STREAM"));
    assert.ok(win.document.getElementById("gpo-stream-overlay")?.textContent.includes("Stream unavailable"));
  });

  it("replaces initial CONNECTING pills using backend health", async () => {
    const win = await dashboard();
    const pill = win.document.createElement("div");
    pill.setAttribute("data-live-pill", "");
    pill.innerHTML = "<span>CONNECTING</span>";
    win.document.body.appendChild(pill);
    win.gpoNoteOnline(false);
    assert.ok(pill.textContent.includes("OFFLINE"));
    win.gpoNoteOnline(true);
    assert.ok(pill.textContent.includes("LIVE"));
  });
});
