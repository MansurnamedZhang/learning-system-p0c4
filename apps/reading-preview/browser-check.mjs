import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";
import readline from "node:readline";

const require = createRequire(import.meta.url);
const bundled = process.env.USERPROFILE && path.join(
  process.env.USERPROFILE, ".cache", "codex-runtimes", "codex-primary-runtime",
  "dependencies", "node", "node_modules", "playwright",
);
let playwright;
try {
  playwright = require("playwright");
} catch (error) {
  if (!bundled) throw error;
  playwright = require(bundled);
}

const url = process.env.KW_PREVIEW_URL;
const ownerCode = process.env.KW_OWNER_CODE;
const visitorCode = process.env.KW_VISITOR_CODE;
const evidence = process.env.KW_PREVIEW_EVIDENCE;
assert.match(url, /^http:\/\/127\.0\.0\.1:\d+$/);
assert.equal(ownerCode.length, 64);
assert.equal(visitorCode.length, 64);
const api = (mode) => "/api/reading?mode=" + mode;
const note = "我的核对";
const topic = "缩放点积注意力";

const browser = await playwright.chromium.launch({
  headless: true,
  ...(process.platform === "win32" ? { channel: "msedge" } : {}),
});
const stdin = readline.createInterface({ input: process.stdin });
const replies = stdin[Symbol.asyncIterator]();
const external = [];
const errors = [];
const passed = [];

async function revoke(step) {
  console.log(step);
  assert.equal((await replies.next()).value, "CONTINUE");
}

async function open(viewport) {
  const context = await browser.newContext({ baseURL: url, viewport });
  const page = await context.newPage();
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("request", (request) => {
    if (!request.url().startsWith(url + "/")) external.push(request.url());
  });
  await page.goto("/");
  await page.locator("#login").waitFor();
  return { context, page };
}

async function login(page, code, outcome) {
  await page.getByLabel("本次访问码").fill(code);
  await page.getByRole("button", { name: "打开预览" }).click();
  await page.locator(outcome).waitFor();
}

async function response(context, mode, status = 200) {
  const result = await context.request.get(api(mode));
  assert.equal(result.status(), status);
  assert.equal(result.headers()["cache-control"], "no-store");
  return result.json();
}

async function switchMode(page, name, mode) {
  const button = page.getByRole("button", { name, exact: true });
  await button.click();
  await page.locator('[data-mode="' + mode + '"][aria-pressed="true"]').waitFor();
  await page.locator("#entries[aria-busy=false]").waitFor();
}

try {
  const desktop = await open({ width: 1360, height: 980 });
  assert.deepEqual(await response(desktop.context, "fused", 401), { error: "sign_in_required" });
  assert.ok(!(await desktop.page.content()).includes(note));
  const wrongOrigin = await desktop.context.request.post("/api/session", {
    headers: { Origin: "http://other.invalid" },
    data: { code: ownerCode },
  });
  assert.equal(wrongOrigin.status(), 403);
  assert.equal(wrongOrigin.headers()["cache-control"], "no-store");
  const wrongHost = await desktop.context.request.get("/", { headers: { Host: "other.invalid" } });
  assert.equal(wrongHost.status(), 421);
  const spoofed = await desktop.context.request.post("/api/session", {
    headers: { Origin: url }, data: { code: visitorCode, actor_id: "owner" },
  });
  assert.equal(spoofed.status(), 400);
  passed.push("anonymous_origin_host_and_actor_input");

  await login(desktop.page, ownerCode, "#reader");
  const session = (await desktop.context.cookies()).find((cookie) => cookie.name === "kw_reading_preview");
  assert.ok(session?.httpOnly);
  assert.equal(session.sameSite, "Strict");
  const fused = await response(desktop.context, "fused");
  const original = await response(desktop.context, "original");
  const personal = await response(desktop.context, "personal");
  assert.equal(fused.title, topic);
  assert.deepEqual(fused.view, original.view);
  assert.deepEqual(fused.view, personal.view);
  assert.deepEqual(fused.entries.map((entry) => entry.kind), [
    "original", "original", "personal", "original", "original",
  ]);
  assert.equal(original.entries.length, 4);
  assert.equal(personal.entries.length, 1);
  assert.deepEqual(original.entries[0].block, original.entries[2].block);
  assert.notDeepEqual(original.entries[0].occurrence_path, original.entries[2].occurrence_path);
  assert.equal(fused.entries[2].title, note);
  await desktop.page.locator("#entries [data-kind=personal]").getByText(note, { exact: true }).waitFor();
  assert.equal(await desktop.page.locator("#entries [data-kind=personal] strong").count(), 0);
  assert.match(await desktop.page.locator("#entries [data-kind=personal]").textContent(), /<strong>原样显示<\/strong>/);
  await desktop.page.getByText("来源与版本", { exact: true }).first().click();
  assert.equal(await desktop.page.locator("details[open]").count(), 1);
  const firstPath = fused.entries[0].occurrence_path.join(" / ");
  await desktop.page.locator("details[open]").getByText(firstPath, { exact: true }).waitFor();
  await switchMode(desktop.page, "原文", "original");
  assert.equal(await desktop.page.locator("#entries .entry").count(), 4);
  await switchMode(desktop.page, "个人", "personal");
  assert.equal(await desktop.page.locator("#entries [data-kind=personal]").count(), 1);
  await switchMode(desktop.page, "融合", "fused");
  assert.equal(await desktop.page.locator("#reader").getAttribute("data-revision"), fused.view.revision_id);
  assert.equal(await desktop.page.locator("#entries .entry").count(), 5);
  passed.push("fixed_revision_three_projections_occurrence_provenance_safe_text");

  const mobile = await open({ width: 390, height: 844 });
  await login(mobile.page, ownerCode, "#reader");
  assert.equal(await mobile.page.locator("#reader").getAttribute("data-revision"), fused.view.revision_id);
  assert.ok(await mobile.page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
  await mobile.page.getByRole("button", { name: "个人", exact: true }).focus();
  await mobile.page.keyboard.press("Enter");
  await mobile.page.locator('[data-mode="personal"][aria-pressed="true"]').waitFor();
  assert.equal(await mobile.page.locator("#entries [data-kind=personal]").count(), 1);
  await switchMode(mobile.page, "融合", "fused");
  if (evidence) {
    assert.ok(fs.statSync(evidence).isDirectory());
    await desktop.page.screenshot({ path: path.join(evidence, "reading-desktop.png") });
    await mobile.page.locator("#entries [data-kind=personal]").scrollIntoViewIfNeeded();
    await mobile.page.screenshot({ path: path.join(evidence, "reading-mobile.png") });
  }
  await mobile.page.getByRole("button", { name: "退出", exact: true }).click();
  await mobile.page.locator("#login").waitFor();
  assert.equal(await mobile.page.locator("#entries").textContent(), "");
  assert.deepEqual(await response(mobile.context, "fused", 401), { error: "sign_in_required" });
  passed.push("same_revision_in_second_browser_context_and_keyboard");

  const visitor = await open({ width: 980, height: 780 });
  await login(visitor.page, visitorCode, "#denied");
  for (const mode of ["original", "fused", "personal"]) {
    assert.deepEqual(await response(visitor.context, mode, 404), { error: "not_found" });
  }
  const asset = await visitor.context.request.get("/api/assets/" + original.entries[0].block.block_id);
  assert.equal(asset.status(), 404);
  assert.equal(asset.headers()["cache-control"], "no-store");
  assert.equal(await visitor.page.locator("#entries").textContent(), "");
  assert.equal(await visitor.page.getByText(note, { exact: true }).count(), 0);
  passed.push("second_subject_denied_private_reading_and_unbound_resource");

  await revoke("REVOKE_SOURCE");
  for (const mode of ["original", "fused", "personal"]) {
    const data = await response(desktop.context, mode);
    assert.equal(data.source, null);
    assert.deepEqual(data.entries, []);
    assert.equal(data.unplaced.length, mode === "original" ? 0 : 1);
    const serialized = JSON.stringify(data);
    for (const hidden of [fused.source.composition_id, fused.source.revision_id,
      original.entries[0].block.block_id, original.entries[0].block.revision_id,
      ...original.entries[0].occurrence_path, "查询、键与值", topic]) {
      assert.ok(!serialized.includes(hidden), "revoked source leaked");
    }
  }
  await desktop.page.reload();
  await desktop.page.locator("#unplaced").getByText(note, { exact: true }).waitFor();
  assert.equal(await desktop.page.locator("#entries [data-kind=original]").count(), 0);
  passed.push("same_session_source_revocation_hides_identifiers_and_keeps_detached_note");

  await revoke("REVOKE_PERSONAL");
  assert.deepEqual(await response(desktop.context, "fused", 404), { error: "not_found" });
  await desktop.page.reload();
  await desktop.page.locator("#denied").waitFor();
  assert.equal(await desktop.page.locator("#unplaced").textContent(), "");
  await desktop.page.getByRole("button", { name: "更换访问码" }).click();
  await desktop.page.locator("#login").waitFor();
  const replay = await desktop.context.request.get(api("fused"), {
    headers: { Cookie: session.name + "=" + session.value },
  });
  assert.equal(replay.status(), 401);
  assert.deepEqual(external, []);
  assert.deepEqual(errors, []);
  passed.push("personal_grant_revocation_logout_and_zero_remote_requests");
  console.log(JSON.stringify({ status: "B1_BROWSER_CHECKS_PASSED", passed, browser: browser.version() }));
} finally {
  stdin.close();
  await browser.close();
}
