"use strict";

const login = document.querySelector("#login");
const reader = document.querySelector("#reader");
const denied = document.querySelector("#denied");
const form = document.querySelector("#login-form");
const code = document.querySelector("#code");
const status = document.querySelector("#status");
const entries = document.querySelector("#entries");
const unplaced = document.querySelector("#unplaced");
const detached = document.querySelector("#detached");
const outline = document.querySelector("#outline");
const modeButtons = [...document.querySelectorAll("[data-mode]")];
const noteEditor = document.querySelector("#note-editor");
const noteText = document.querySelector("#note-text");
const noteSave = document.querySelector("#note-save");
const noteLatest = document.querySelector("#note-latest");
const noteStatus = document.querySelector("#note-status");
let mode = "fused";
let pending;
let noteBase;
let draftBase;
let submission;
let saving = false;
let mustLoadLatest = false;

const request = (path, options = {}) => fetch(path, {
  credentials: "same-origin",
  cache: "no-store",
  ...options,
});

function node(tag, text, className) {
  const element = document.createElement(tag);
  element.textContent = text;
  if (className) element.className = className;
  return element;
}

function clearReading() {
  document.querySelector("#document-title").textContent = "阅读";
  document.querySelector("#edition").textContent = "";
  document.querySelector("#rail-description").textContent = "浏览这次预览中的固定阅读版本。";
  for (const target of [entries, unplaced, outline]) target.replaceChildren();
  delete reader.dataset.revision;
  detached.hidden = true;
}

function show(which, message = "") {
  login.hidden = which !== "login";
  reader.hidden = which !== "reader";
  denied.hidden = which !== "denied";
  status.textContent = message;
  status.hidden = message === "";
  if (which !== "reader") {
    clearReading();
    noteBase = undefined;
    clearDraft();
    updateNoteEditor();
  }
}

function clearDraft() {
  noteText.value = "";
  noteStatus.textContent = "";
  draftBase = undefined;
  submission = undefined;
  mustLoadLatest = false;
}

function updateNoteEditor() {
  noteEditor.hidden = !noteBase || mode === "original";
  noteText.readOnly = !!submission;
  noteSave.disabled = saving || !!pending || mustLoadLatest || !noteBase;
  noteSave.textContent = saving ? "正在保存…" : submission ? "重试同一次保存" : "保存笔记";
  noteLatest.hidden = !mustLoadLatest;
  noteLatest.disabled = !!pending || saving;
}

function card(entry, id, source) {
  const article = document.createElement("section");
  article.className = "entry";
  article.dataset.kind = entry.kind;
  article.id = id;
  article.append(
    node("p", entry.kind === "personal" ? "个人笔记" : "原文", "kicker"),
    node("h2", entry.title),
    node("p", entry.text),
  );
  if (entry.kind === "original") {
    const details = document.createElement("details");
    const list = document.createElement("dl");
    for (const [label, value] of [
      ["原文组合修订", source.revision_id],
      ["内容块修订", entry.block.revision_id],
      ["本次出现路径", entry.occurrence_path.join(" / ")],
    ]) list.append(node("dt", label), node("dd", value));
    details.append(node("summary", "来源与版本"), list);
    article.append(details);
  }
  return article;
}

function render(data, useLatest) {
  clearReading();
  reader.dataset.revision = data.view.revision_id;
  document.querySelector("#document-title").textContent = data.title;
  document.querySelector("#edition").textContent = "阅读修订 " + data.view.revision_id;
  document.querySelector("#rail-description").textContent = data.source
    ? "打开原文卡片底部的来源，可查看该块在本处的版本与出现路径。"
    : "原文当前不可访问，仍可查看属于自己的笔记。";
  data.entries.forEach((entry, index) => {
    const id = "entry-" + (index + 1);
    const link = node("a", (index + 1) + " · " + entry.title);
    link.href = "#" + id;
    outline.append(link);
    entries.append(card(entry, id, data.source));
  });
  if (!data.entries.length) {
    entries.append(node("p", data.source ? "当前视图没有条目。" : "原文当前不可访问。", "empty"));
  }
  data.unplaced.forEach((entry, index) => {
    unplaced.append(card(entry, "unplaced-" + (index + 1), null));
  });
  detached.hidden = data.unplaced.length === 0;
  for (const button of modeButtons) button.setAttribute("aria-pressed", String(button.dataset.mode === data.mode));
  noteBase = data.note_base;
  if (!noteBase) {
    clearDraft();
  } else if (useLatest && mustLoadLatest) {
    draftBase = noteBase;
    mustLoadLatest = false;
    noteStatus.textContent = "已载入最新修订，草稿已保留。检查后可再次保存。";
  }
  updateNoteEditor();
  show("reader");
}

async function read({ useLatest = false } = {}) {
  pending?.abort();
  const current = new AbortController();
  pending = current;
  updateNoteEditor();
  clearReading();
  status.textContent = "正在读取当前修订…";
  status.hidden = false;
  entries.setAttribute("aria-busy", "true");
  try {
    const response = await request("/api/reading?mode=" + mode, { signal: current.signal });
    if (response.status === 401) {
      if (pending === current) show("login");
    } else if (response.status === 404) {
      if (pending === current) show("denied");
    } else if (!response.ok) {
      throw new Error("reading");
    } else {
      const data = await response.json();
      if (pending === current) render(data, useLatest);
    }
  } catch {
    if (pending === current) {
      show(reader.hidden ? "login" : "reader", "读取暂不可用。请重试。");
    }
  } finally {
    if (pending === current) {
      pending = undefined;
      entries.setAttribute("aria-busy", "false");
      updateNoteEditor();
    }
  }
}

noteText.addEventListener("input", () => {
  if (noteText.value) draftBase ??= noteBase;
  else if (!mustLoadLatest) draftBase = undefined;
});

noteLatest.addEventListener("click", async () => {
  await read({ useLatest: true });
  if (!noteEditor.hidden) noteText.focus();
});

document.querySelector("#note-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  if (saving || pending || mustLoadLatest || !noteBase) return;
  if (!submission) {
    const text = noteText.value;
    if (!text.trim() || new TextEncoder().encode(text).length > 4000 || text.includes("\0")) {
      noteStatus.textContent = "请输入非空正文，最多 4,000 字节。";
      return;
    }
    const id = crypto.randomUUID();
    // Keep the exact body until the server confirms a receipt or a rejection.
    submission = {
      id,
      body: JSON.stringify({ request_id: id, base: draftBase ?? noteBase, text }),
    };
  }
  saving = true;
  noteStatus.textContent = "正在保存笔记…";
  updateNoteEditor();
  try {
    const response = await request("/api/notes", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: submission.body,
      signal: AbortSignal.timeout(25000),
    });
    if (response.ok) {
      const receipt = await response.json();
      if (receipt.request_id !== submission.id || typeof receipt.revision_id !== "string") {
        throw new Error("unexpected note receipt");
      }
      clearDraft();
      noteStatus.textContent = "笔记已保存。";
      await read();
    } else if (response.status === 409) {
      const { error } = await response.json();
      if (error !== "stale_revision" && error !== "request_id_conflict") {
        throw new Error("unexpected conflict");
      }
      submission = undefined;
      mustLoadLatest = true;
      noteStatus.textContent = error === "stale_revision"
        ? "阅读已更新，草稿已保留。载入最新修订后再保存。"
        : "请求标识冲突，草稿已保留。载入最新修订后再保存。";
      noteText.focus();
    } else if (response.status === 400) {
      submission = undefined;
      noteStatus.textContent = "正文或保存位置无效，草稿已保留。";
    } else if (response.status === 401) {
      show("login", "会话已结束，请重新登录。");
    } else if (response.status === 404) {
      clearDraft();
      await read();
      status.textContent = "当前权限无法保存这条笔记。";
      status.hidden = false;
    } else {
      throw new Error("save unconfirmed");
    }
  } catch {
    noteStatus.textContent = "保存结果尚未确认，可重试同一次保存。";
  } finally {
    saving = false;
    updateNoteEditor();
  }
});

form.addEventListener("submit", async (event) => {
  event.preventDefault();
  const submit = form.querySelector("button");
  submit.disabled = true;
  const value = code.value;
  code.value = "";
  try {
    const response = await request("/api/session", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ code: value }),
    });
    if (response.ok) await read();
    else show("login", response.status === 401 ? "访问码无效。请核对启动终端。" : "登录暂不可用。");
  } catch {
    show("login", "登录暂不可用。");
  } finally {
    submit.disabled = false;
  }
});

async function signOut() {
  if (saving) return;
  pending?.abort();
  pending = undefined;
  show("loading", "正在结束会话…");
  try {
    const response = await request("/api/session", { method: "DELETE" });
    if (!response.ok) throw new Error("sign-out");
    mode = "fused";
    show("login", "会话已结束。");
  } catch {
    show("denied", "暂未确认退出，请再试一次。");
  }
}

document.querySelector("#logout").addEventListener("click", signOut);
document.querySelector("#change-seat").addEventListener("click", signOut);
for (const button of modeButtons) button.addEventListener("click", () => {
  if (saving) return;
  mode = button.dataset.mode;
  read();
});
window.addEventListener("pageshow", (event) => {
  if (event.persisted && !saving) {
    show("loading");
    read();
  }
});
read();
