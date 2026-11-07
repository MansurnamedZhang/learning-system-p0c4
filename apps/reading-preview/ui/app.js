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
let mode = "fused";
let pending;

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
  if (which !== "reader") clearReading();
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

function render(data) {
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
  show("reader");
}

async function read() {
  pending?.abort();
  const current = new AbortController();
  pending = current;
  clearReading();
  status.textContent = "正在读取固定版本…";
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
      if (pending === current) render(data);
    }
  } catch {
    if (pending === current) {
      show(reader.hidden ? "login" : "reader", "读取暂不可用。请重试。");
    }
  } finally {
    if (pending === current) entries.setAttribute("aria-busy", "false");
  }
}

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
  mode = button.dataset.mode;
  read();
});
window.addEventListener("pageshow", (event) => {
  if (event.persisted) {
    show("loading");
    read();
  }
});
read();
