import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";
import { test } from "node:test";
import assert from "node:assert/strict";
import { runInContext } from "node:vm";
const source = readFileSync(new URL("gpo-core.js", import.meta.url), "utf8");
function open(fetch) {
  const dom = new JSDOM("<main></main>", { url: "http://localhost/", runScripts: "outside-only" });
  dom.window.fetch = fetch;
  runInContext(source, dom.getInternalVMContext());
  return dom;
}
test("repeated in-flight toggle sends once, preserves caller input, and can run again", async () => {
  let finish, count = 0;
  const dom = open((_url, options) => {
    count++;
    assert.ok(options.signal);
    return new Promise(resolve => { finish = () => resolve({ ok: true, json: async () => ({ ok: true }) }); });
  });
  try {
    const extra = { enabled: true };
    const a = dom.window.gpoAction("set_collection", extra);
    const b = dom.window.gpoAction("set_collection", extra);
    assert.equal(count, 1);
    assert.deepEqual(extra, { enabled: true });
    finish(); await Promise.all([a, b]);
    const c = dom.window.gpoAction("set_collection", extra);
    assert.equal(count, 2); finish(); await c;
  } finally { dom.window.close(); }
});
test("invalid JSON and rejected commands are never shown as success", async () => {
  const dom = open(async () => ({ ok: true, json: async () => { throw Error("not JSON"); } }));
  try {
    await assert.rejects(dom.window.gpoAction("start"), /bad backend response/);
    dom.window.fetch = async () => ({ ok: false, status: 400, json: async () => ({ ok: false, message: "Unknown action" }) });
    await assert.rejects(dom.window.gpoAction("invalid"), /Unknown action/);
  } finally { dom.window.close(); }
});
