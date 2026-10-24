import AxeBuilder from "@axe-core/playwright";
import { test, expect } from "@playwright/test";
import { readFile, writeFile } from "node:fs/promises";
import { resolve, extname } from "node:path";
const root = resolve("src-tauri/src/bot/web");
const pages = ["remote", "stats", "studio", "craft", "system"];
for (const width of [320, 360, 768, 1280]) for (const name of pages) {
  test(`${name} at ${width}px loads offline assets, fits and reports failures`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    const errors: string[] = [];
    page.on("pageerror", e => errors.push(e.message));
    let offline = false;
    await page.route("http://gpo.test/**", async route => {
      const url = new URL(route.request().url());
      if (url.pathname.startsWith("/api/")) {
        const authenticated = url.pathname === "/api/stream" ? url.searchParams.get("token") === "synthetic-test-token" : route.request().headers()["authorization"] === "Bearer synthetic-test-token";
        if (!authenticated) { await route.fulfill({ status: 401, body: "Unauthorized" }); return; }
        if (offline) { await route.abort(); return; }
        if (url.pathname === "/api/action") { await route.fulfill({ status: 400, json: { ok: false, message: "Test command rejected" } }); return; }
        if (url.pathname === "/api/stream") { await route.abort(); return; }
        await route.fulfill({ json: { ok: true, state: "paused", paused: true, is_running: false, stream_live: false,
          version: "test", fish: 0, fruits: 0, runtime_s: 0, macros: [], timers: [],
          settings: {}, total: {}, crafting: { is_crafting: false, crafted_count: 0 } } }); return;
      }
      const pageName = url.pathname === "/" ? "remote" : url.pathname.slice(1);
      const relative = url.pathname.startsWith("/assets/") ? url.pathname.slice(1) : `${pages.includes(pageName) ? pageName : "remote"}.html`;
      const file = resolve(root, relative);
      if (!file.startsWith(root + "/")) { await route.abort(); return; }
      const mime: Record<string,string> = { ".css": "text/css", ".js": "text/javascript", ".html": "text/html", ".woff2": "font/woff2" };
      await route.fulfill({ body: await readFile(file), contentType: mime[extname(file)] || "application/octet-stream" });
    });
    await page.goto(`http://gpo.test/${name === "remote" ? "" : name}?token=synthetic-test-token`);
    expect(page.url()).not.toContain("token=");
    expect(await page.evaluate(() => sessionStorage.getItem("gpo-dashboard-token"))).toBe("synthetic-test-token");
    await page.waitForTimeout(250);
    if (width === 360) await page.screenshot({ path: `test-results/${name}-phone.png`, fullPage: true });
    const accessibility = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa"]).analyze();
    if (width === 360) await writeFile(`test-results/${name}-accessibility.json`, JSON.stringify(accessibility.violations, null, 2));
    expect(accessibility.violations.filter(v => ["critical", "serious"].includes(v.impact || "")), JSON.stringify(accessibility.violations.map(v => ({id: v.id, nodes: v.nodes.map(n => n.target)})))).toEqual([]);
    expect(errors).toEqual([]);
    await expect(page.locator("body")).not.toContainText("undefined");
    await expect(page.locator("body")).not.toContainText("NaN");
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await expect(page.locator('[data-nav]')).toHaveCount(5);
    await expect(page.locator('script[src*="tailwind.js"]')).toHaveCount(0);
    const message = await page.evaluate(async () => {
      try { await (window as any).gpoAction("invalid"); return "incorrect success"; }
      catch (e) { return String(e); }
    });
    expect(message).toContain("Test command rejected");
    offline = true;
    await page.evaluate(async () => { try { await (window as any).gpoGet("/api/status"); } catch {} });
    await expect(page.locator("#gpo-offline-banner")).toBeVisible();
    offline = false;
    await page.evaluate(async () => { await (window as any).gpoGet("/api/status"); });
    await expect(page.locator("#gpo-offline-banner")).toBeHidden();
    expect(errors).toEqual([]);
    const next = name === "stats" ? "/" : "/stats";
    await page.locator(`[data-nav="${next}"]`).click();
    await expect(page).toHaveURL(`http://gpo.test${next}`);
    expect(await page.evaluate(() => sessionStorage.getItem("gpo-dashboard-token"))).toBe("synthetic-test-token");
    await page.waitForTimeout(100);
    expect(errors).toEqual([]);
  });
}
