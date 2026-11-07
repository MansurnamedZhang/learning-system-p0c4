import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
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
const firstNote = "B2 保存检验 🧪\n<img src=\"https://invalid.example/note.png\"> 按字面保留";
const staleDraft = "另一页的草稿\n检查 softmax 的权重和。";
const editedDraft = staleDraft + "\n再核对掩码。";

const browser = await playwright.chromium.launch({
  headless: true,
  ...(process.platform === "win32" ? { channel: "msedge" } : {}),
});
const stdin = readline.createInterface({ input: process.stdin });
const replies = stdin[Symbol.asyncIterator]();
const external = [];
const errors = [];
const passed = [];

async function checkpoint(step) {
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

async function save(context, data, expectedStatus, origin = url) {
  const result = await context.request.post("/api/notes", {
    headers: { Origin: origin, "Content-Type": "application/json" }, data,
  });
  assert.equal(result.status(), expectedStatus);
  assert.equal(result.headers()["cache-control"], "no-store");
  return result.json();
}

function clickSave(page, label = "保存笔记") {
  return Promise.all([
    page.waitForResponse((result) => result.url() === url + "/api/notes" && result.request().method() === "POST"),
    page.getByRole("button", { name: label, exact: true }).click(),
  ]).then(([response]) => response);
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
  assert.deepEqual(await save(desktop.context, {}, 401), { error: "sign_in_required" });
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

  const payload = { request_id: randomUUID(), base: fused.note_base, text: "拒绝的输入" };
  assert.equal(fused.note_base.reading_revision, fused.view.revision_id);
  assert.deepEqual(await save(visitor.context, payload, 404), { error: "not_found" });
  assert.deepEqual(await save(desktop.context, payload, 403, "http://other.invalid"), { error: "wrong_origin" });
  for (const invalid of [
    { ...payload, actor_id: "forged" },
    { ...payload, base: { ...payload.base, group_id: randomUUID() } },
    { ...payload, text: "注".repeat(1334) },
    { ...payload, text: " \n" },
  ]) {
    assert.deepEqual(await save(desktop.context, invalid, 400), { error: "invalid_request" });
  }
  await visitor.page.locator("#note-editor").waitFor({ state: "hidden" });

  await login(mobile.page, ownerCode, "#reader");
  const draft = mobile.page.getByLabel("笔记正文");
  await draft.fill(staleDraft);
  await desktop.page.getByLabel("笔记正文").fill(firstNote);
  let firstBody;
  let concurrent;
  await desktop.page.route("**/api/notes", async (route) => {
    firstBody = route.request().postData();
    const [lost, duplicate] = await Promise.all([
      route.fetch(),
      desktop.context.request.post("/api/notes", {
        headers: { Origin: url, "Content-Type": "application/json" }, data: firstBody,
      }),
    ]);
    concurrent = {
      statuses: [lost.status(), duplicate.status()],
      receipts: await Promise.all([lost.json(), duplicate.json()]),
    };
    // PostgreSQL has committed. Drop the reply at the browser boundary.
    await route.abort("failed");
  }, { times: 1 });
  await desktop.page.getByRole("button", { name: "保存笔记", exact: true }).click();
  await desktop.page.getByText("保存结果尚未确认，可重试同一次保存。", { exact: true }).waitFor();
  assert.deepEqual(concurrent.statuses, [200, 200]);
  const [saved, duplicate] = concurrent.receipts;
  assert.deepEqual(saved, duplicate);
  assert.equal(saved.request_id, JSON.parse(firstBody).request_id);
  assert.notEqual(saved.revision_id, fused.view.revision_id);
  assert.equal(await desktop.page.getByLabel("笔记正文").inputValue(), firstNote);
  assert.equal(await desktop.page.getByLabel("笔记正文").evaluate((area) => area.readOnly), true);

  const retry = await clickSave(desktop.page, "重试同一次保存");
  assert.equal(retry.status(), 200);
  assert.equal(retry.request().postData(), firstBody);
  assert.deepEqual(await retry.json(), saved);
  await desktop.page.getByText("笔记已保存。", { exact: true }).waitFor();
  assert.equal(await desktop.page.getByLabel("笔记正文").inputValue(), "");
  assert.deepEqual(await save(desktop.context, { ...JSON.parse(firstBody), text: "改过的重试" }, 409),
    { error: "request_id_conflict" });
  passed.push("lost_reply_and_concurrent_identical_requests_return_one_receipt");

  await desktop.page.reload();
  await desktop.page.locator("#reader").waitFor();
  await desktop.page.getByText(firstNote.split("\n")[1], { exact: false }).waitFor();
  const afterFirst = await response(desktop.context, "fused");
  assert.deepEqual(afterFirst.source, fused.source);
  assert.equal(afterFirst.view.view_id, fused.view.view_id);
  assert.equal(afterFirst.view.revision_id, saved.revision_id);
  assert.deepEqual(afterFirst.entries.map((entry) => entry.kind), [
    "original", "original", "personal", "personal", "original", "original",
  ]);
  assert.equal(afterFirst.entries[3].text, firstNote);
  assert.deepEqual(afterFirst.entries.filter((entry) => entry.kind === "original"), original.entries);
  assert.equal(await desktop.page.locator("#entries img").count(), 0);
  passed.push("private_note_commits_at_fixed_gap_and_survives_reload");

  const conflict = await clickSave(mobile.page);
  assert.equal(conflict.status(), 409);
  assert.deepEqual(await conflict.json(), { error: "stale_revision" });
  const rejected = conflict.request().postDataJSON();
  assert.deepEqual(rejected.base, fused.note_base);
  await mobile.page.getByText("阅读已更新，草稿已保留。载入最新修订后再保存。", { exact: true }).waitFor();
  assert.equal(await draft.inputValue(), staleDraft);
  assert.equal(await draft.isEditable(), true);
  assert.equal(await draft.evaluate((area) => document.activeElement === area), true);
  assert.equal(await mobile.page.getByRole("button", { name: "保存笔记", exact: true }).isEnabled(), false);
  if (evidence) await mobile.page.screenshot({ path: path.join(evidence, "note-conflict-mobile.png") });
  await draft.fill(editedDraft);
  await mobile.page.getByRole("button", { name: "载入最新修订", exact: true }).click();
  await mobile.page.getByText("已载入最新修订，草稿已保留。检查后可再次保存。", { exact: true }).waitFor();
  assert.equal(await mobile.page.locator("#reader").getAttribute("data-revision"), saved.revision_id);
  assert.equal(await draft.inputValue(), editedDraft);
  assert.equal((await response(mobile.context, "fused")).entries.some((entry) => entry.text === editedDraft), false);
  const savedFromDraft = await clickSave(mobile.page);
  assert.equal(savedFromDraft.status(), 200);
  const newCommand = savedFromDraft.request().postDataJSON();
  assert.notEqual(newCommand.request_id, rejected.request_id);
  assert.deepEqual(newCommand.base, afterFirst.note_base);
  assert.equal(newCommand.text, editedDraft);
  const second = await savedFromDraft.json();
  await mobile.page.getByText("笔记已保存。", { exact: true }).waitFor();
  await mobile.page.reload();
  await mobile.page.locator("#reader").waitFor();
  const afterSecond = await response(mobile.context, "fused");
  assert.equal(afterSecond.view.revision_id, second.revision_id);
  assert.deepEqual(afterSecond.entries.filter((entry) => entry.kind === "personal").slice(1).map((entry) => entry.text),
    [firstNote, editedDraft]);
  assert.deepEqual(afterSecond.entries.filter((entry) => entry.kind === "original"), original.entries);
  assert.deepEqual(await save(mobile.context, firstBody, 200), saved);
  assert.equal((await response(mobile.context, "fused")).view.revision_id, second.revision_id);
  assert.ok(await mobile.page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
  if (evidence) {
    await desktop.page.reload();
    await desktop.page.locator("#note-editor").waitFor();
    await desktop.page.locator("#note-editor").scrollIntoViewIfNeeded();
    await desktop.page.screenshot({ path: path.join(evidence, "note-editor-desktop.png") });
  }
  passed.push("stale_two_page_conflict_keeps_draft_until_explicit_rebase");

  const freshNote = { request_id: randomUUID(), base: afterSecond.note_base, text: "撤权后的尝试" };
  await checkpoint("VERIFY_WRITES");
  await checkpoint("REVOKE_WRITE");
  assert.equal((await response(desktop.context, "fused")).note_base, null);
  assert.deepEqual(await save(desktop.context, freshNote, 404), { error: "not_found" });
  assert.deepEqual(await save(desktop.context, firstBody, 404), { error: "not_found" });
  await desktop.page.reload();
  await desktop.page.locator("#reader").waitFor();
  await desktop.page.locator("#note-editor").waitFor({ state: "hidden" });

  await checkpoint("REVOKE_SOURCE");
  assert.deepEqual(await save(desktop.context, freshNote, 404), { error: "not_found" });
  assert.deepEqual(await save(desktop.context, firstBody, 404), { error: "not_found" });
  for (const mode of ["original", "fused", "personal"]) {
    const data = await response(desktop.context, mode);
    assert.equal(data.source, null);
    assert.equal(data.note_base, null);
    assert.deepEqual(data.entries, []);
    assert.equal(data.unplaced.length, mode === "original" ? 0 : 3);
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

  await checkpoint("REVOKE_PERSONAL");
  assert.deepEqual(await response(desktop.context, "fused", 404), { error: "not_found" });
  assert.deepEqual(await save(desktop.context, firstBody, 404), { error: "not_found" });
  await desktop.page.reload();
  await desktop.page.locator("#denied").waitFor();
  assert.equal(await desktop.page.locator("#unplaced").textContent(), "");
  await desktop.page.getByRole("button", { name: "更换访问码" }).click();
  await desktop.page.locator("#login").waitFor();
  const replay = await desktop.context.request.get(api("fused"), {
    headers: { Cookie: session.name + "=" + session.value },
  });
  assert.equal(replay.status(), 401);
  const writeReplay = await desktop.context.request.post("/api/notes", {
    headers: { Cookie: session.name + "=" + session.value, Origin: url, "Content-Type": "application/json" },
    data: firstBody,
  });
  assert.equal(writeReplay.status(), 401);
  passed.push("note_write_rejects_unauthorized_inputs_and_revoked_grants");
  assert.deepEqual(external, []);
  assert.deepEqual(errors, []);
  passed.push("personal_grant_revocation_logout_and_zero_remote_requests");
  console.log(JSON.stringify({ status: "B2_BROWSER_CHECKS_PASSED", passed, browser: browser.version() }));
} finally {
  stdin.close();
  await browser.close();
}
