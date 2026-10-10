// Render repository architecture Markdown as one offline HTML document.
// No application code, databases, credentials or evidence are read by this tool.
import fs from 'node:fs';
import path from 'node:path';
import {createRequire} from 'node:module';
import {fileURLToPath, pathToFileURL} from 'node:url';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const source = path.join(repo, 'docs', 'architecture');
const args = process.argv.slice(2);
if (args.length && (args.length !== 2 || args[0] !== '--output')) {
  throw new Error('Usage: node scripts/build_architecture.mjs [--output path.html]');
}
const output = args.length ? path.resolve(args[1]) : path.join(source, 'KnowWeave架构设计.html');
const require = createRequire(import.meta.url);
let markedPath;
try {
  markedPath = require.resolve('marked');
} catch {
  const bundled = process.env.KNOWWEAVE_DOCS_NODE_MODULES ||
    (process.env.USERPROFILE && path.join(process.env.USERPROFILE,
      '.cache', 'codex-runtimes', 'codex-primary-runtime', 'dependencies', 'node', 'node_modules'));
  if (!bundled) throw new Error('Provide marked via KNOWWEAVE_DOCS_NODE_MODULES or a local Node environment.');
  markedPath = require.resolve(path.join(bundled, 'marked'));
}
const {marked} = await import(pathToFileURL(markedPath).href);
const chapters = [
  ['index', '阅读入口与状态', 'README.md'],
  ['overview', '01 总体架构与产品边界', '01-overview.md'],
  ['domain', '02 块与版本化领域模型', '02-domain-model.md'],
  ['modules', '03 Rust 模块与代码边界', '03-modules.md'],
  ['permissions', '04 事务授权与关系查询', '04-transactions-and-permissions.md'],
  ['assets', '05 原件存储与持久任务', '05-assets-and-jobs.md'],
  ['recovery', '06 快照交换与备份恢复', '06-portability-and-recovery.md'],
  ['interfaces', '07 API 前端与学习闭环', '07-product-interfaces.md'],
  ['deployment', '08 部署运维与架构演进', '08-deployment-and-evolution.md'],
  ['evidence', '09 决策记录与证据索引', '09-decisions-and-evidence.md'],
];
const chapterFiles = new Map(chapters.map(([id, , file]) => [path.join(source, file), id]));
const esc = value => String(value).replace(/[&<>"']/g,
  c => ({'&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;'}[c]));
const relativeUrl = absolute => path.relative(path.dirname(output), absolute).split(path.sep).map(encodeURIComponent).join('/');
const sources = new Set();
const figures = new Set();
let headingCount = 0;
let linkCount = 0;
let tableCount = 0;

const content = chapters.map(([id, title, filename]) => {
  const file = path.join(source, filename);
  sources.add(file);
  let html = marked.parse(fs.readFileSync(file, 'utf8'), {gfm: true, breaks: false});
  let localHeading = 0;
  html = html.replace(/<h([1-6])>([\s\S]*?)<\/h\1>/g, (_, level, inner) => {
    const n = Math.min(Number(level) + 1, 6);
    headingCount++;
    return `<h${n} id="${id}-heading-${++localHeading}">${inner}</h${n}>`;
  });
  html = html.replace(/<a href="([^"]*)"/g, (_, href) => {
    linkCount++;
    if (/^(?:https?:|mailto:|#)/i.test(href)) return `<a href="${esc(href)}"`;
    if (/^[a-z]+:/i.test(href)) throw new Error(`Unsupported URL in ${filename}: ${href}`);
    const [target, fragment] = href.split('#');
    const absolute = path.resolve(source, decodeURIComponent(target));
    if (!fs.existsSync(absolute)) throw new Error(`Missing link in ${filename}: ${href}`);
    const chapter = chapterFiles.get(absolute);
    if (chapter) return `<a href="#chapter-${chapter}"`;
    sources.add(absolute);
    return `<a href="${esc(relativeUrl(absolute) + (fragment ? '#' + fragment : ''))}"`;
  });
  html = html.replace(/<img src="([^"]*)" alt="([^"]*)"\s*\/?\s*>/g, (_, src, alt) => {
    const absolute = path.resolve(source, decodeURIComponent(src));
    if (path.extname(absolute).toLowerCase() !== '.svg' || !fs.existsSync(absolute)) {
      throw new Error(`Expected local SVG diagram: ${src}`);
    }
    figures.add(absolute);
    let svg = fs.readFileSync(absolute, 'utf8').replace(/<\?xml[^>]*\?>/g, '');
    if (/<script\b|<foreignObject\b|(?:href|src)\s*=\s*["'](?:https?:|javascript:)/i.test(svg)) {
      throw new Error(`Diagram must be offline and non-executable: ${src}`);
    }
    return `<figure><div class="diagram">${svg}</div><figcaption>${alt}</figcaption></figure>`;
  });
  html = html.replace(/<p>\s*(<figure>[\s\S]*?<\/figure>)\s*<\/p>/g, '$1');
  html = html.replace(/<table>/g, () => {
    tableCount++;
    return '<div class="table-wrap"><table>';
  }).replace(/<\/table>/g, '</table></div>');
  return `<section class="chapter" id="chapter-${id}" aria-label="${esc(title)}">${html}<p class="chapter-source"><a href="${esc(relativeUrl(file))}">编辑本章 Markdown ↗</a></p></section>`;
}).join('\n');

const nav = chapters.map(([id, title]) => `<a href="#chapter-${id}">${esc(title)}</a>`).join('');
const css = `
:root{color-scheme:light dark;--bg:#f4f6f9;--paper:#fff;--ink:#1b2b3e;--muted:#56667a;--line:#d8e0e8;--accent:#1e655c;--tag:#e3f1eb;--soft:#edf2f7}
@media(prefers-color-scheme:dark){:root{--bg:#111923;--paper:#192533;--ink:#e8edf3;--muted:#adbac9;--line:#354456;--accent:#90d3bb;--tag:#203f35;--soft:#233345}}
*{box-sizing:border-box}html{scroll-behavior:smooth}body{margin:0;font:16px/1.85 'Microsoft YaHei','PingFang SC',system-ui,sans-serif;background:var(--bg);color:var(--ink)}
a{color:var(--accent);text-underline-offset:3px}a:hover{text-decoration-thickness:2px}button,input{font:inherit}button{cursor:pointer}.shell{max-width:1510px;margin:auto;display:grid;grid-template-columns:280px minmax(0,1fr);gap:36px;padding:30px}
aside{position:sticky;top:24px;align-self:start;max-height:calc(100vh - 48px);overflow:auto;padding-right:5px}.brand{font-weight:750;font-size:24px;line-height:1.4;letter-spacing:.4px}.brand small{font-weight:400;display:block;font-size:13px;color:var(--muted);margin:8px 0 20px}
.controls{display:grid;gap:10px}.controls input{width:100%;min-width:0;padding:10px 12px;border:1px solid var(--line);border-radius:8px;background:var(--paper);color:var(--ink)}.controls button{padding:8px 12px;border:1px solid var(--line);border-radius:8px;background:var(--paper);color:var(--ink)}.counter{font-size:12px;color:var(--muted)}
nav{display:grid;gap:5px;margin:20px 0}nav a{font-size:14px;text-decoration:none;padding:7px 10px;border-left:3px solid transparent;line-height:1.6}nav a:hover,nav a.active{border-color:var(--accent);background:var(--soft)}.aside-note{font-size:12px;color:var(--muted);line-height:1.8}
main{min-width:0}header{padding:34px 40px;background:var(--paper);border:1px solid var(--line);border-radius:14px;margin-bottom:26px}.eyebrow{font-size:12px;font-weight:700;letter-spacing:1.5px;color:var(--accent)}h1{font-size:34px;line-height:1.35;margin:12px 0 14px}header p{color:var(--muted);margin:12px 0}.badges{display:flex;gap:8px;flex-wrap:wrap}.badge{padding:3px 10px;background:var(--tag);border-radius:20px;font-size:12px}.chapter{background:var(--paper);border:1px solid var(--line);border-radius:14px;padding:30px 40px;margin-bottom:24px;scroll-margin-top:20px}.chapter[hidden]{display:none}h2{font-size:25px;margin:0 0 24px;line-height:1.5}h3{font-size:20px;margin:32px 0 14px;line-height:1.5}h4{font-size:17px}p{margin:12px 0}ul,ol{padding-left:27px}li{margin:7px 0}.table-wrap{overflow-x:auto;margin:18px 0}table{width:100%;border-collapse:collapse;font-size:14px;min-width:580px}th,td{padding:12px 14px;vertical-align:top;text-align:left;border:1px solid var(--line);line-height:1.7}th{background:var(--soft)}code{font:13px/1.7 Consolas,monospace;overflow-wrap:anywhere;background:var(--soft);padding:2px 5px;border-radius:4px}pre{overflow:auto;padding:18px 20px;background:var(--soft);border:1px solid var(--line);border-radius:8px;line-height:1.7}pre code{background:transparent;padding:0;white-space:pre;overflow-wrap:normal}figure{margin:22px 0}figure .diagram{overflow-x:auto;border:1px solid var(--line);border-radius:10px;background:var(--paper)}svg{display:block;width:100%;min-width:750px;height:auto}figcaption{font-size:13px;color:var(--muted);text-align:center;margin:8px 0}.chapter-source{font-size:12px;color:var(--muted);border-top:1px solid var(--line);padding-top:14px;margin-top:25px}footer{font-size:13px;color:var(--muted);padding:10px 0 30px}.hide-code pre{display:none}.empty{padding:35px;background:var(--paper);border:1px solid var(--line);border-radius:12px}
@media(max-width:1000px){.shell{grid-template-columns:220px minmax(0,1fr);gap:20px;padding:20px}.chapter,header{padding:26px}}
@media(max-width:760px){.shell{display:block;padding:14px}aside{position:static;max-height:none;padding:10px 4px 18px}nav{grid-template-columns:1fr 1fr;gap:3px}.brand{font-size:22px}.controls{grid-template-columns:1fr auto}.counter{grid-column:1/-1}.aside-note{display:none}.chapter,header{padding:22px 18px}h1{font-size:29px}h2{font-size:22px}h3{font-size:19px}}
@media print{body{background:#fff;color:#111}.shell{display:block;padding:0;max-width:none}aside,.chapter-source,.empty{display:none}.chapter,header{border:0;border-radius:0;padding:12px 0;box-shadow:none}h2,h3{break-after:avoid}.table-wrap,.diagram{overflow:visible}table{min-width:0;font-size:10px}svg{min-width:0}pre{white-space:pre-wrap}.chapter{break-before:page}.chapter:first-of-type{break-before:auto}.chapter[hidden]{display:block}.hide-code pre{display:block}a{color:#111;text-decoration:none}footer{font-size:10px}}
`;
const js = `
const chapters=[...document.querySelectorAll('.chapter')];
const input=document.querySelector('#search');const counter=document.querySelector('#counter');
input.addEventListener('input',()=>{const q=input.value.trim().toLocaleLowerCase();let count=0;for(const section of chapters){section.hidden=q!==''&&!section.textContent.toLocaleLowerCase().includes(q);if(!section.hidden)count++;}counter.textContent=q?'匹配 '+count+' / '+chapters.length+' 章':'全部 '+chapters.length+' 章';document.querySelector('#empty').hidden=count!==0;});
document.querySelector('#print').addEventListener('click',()=>window.print());
document.querySelectorAll('nav a').forEach(a=>a.addEventListener('click',()=>{input.value='';input.dispatchEvent(new Event('input'));}));
const observer=new IntersectionObserver(entries=>{for(const e of entries)if(e.isIntersecting){document.querySelectorAll('nav a').forEach(a=>a.classList.toggle('active',a.hash==='#'+e.target.id));}},{rootMargin:'-10% 0px -65% 0px'});chapters.forEach(s=>observer.observe(s));
`;
const html = `<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="description" content="KnowWeave 全项目架构：领域模型、Rust 模块、事务权限、原件任务、交换备份、前端学习与部署演进。"><title>KnowWeave 架构设计文档</title><style>${css}</style></head><body><div class="shell"><aside><div class="brand">知织 · KnowWeave<small>项目架构设计 · 2026-09-30</small></div><div class="controls"><input id="search" type="search" aria-label="搜索架构章节" placeholder="搜索：版本、关系、恢复…"><button id="print" type="button">打印 / PDF</button><div id="counter" class="counter">全部 ${chapters.length} 章</div></div><nav aria-label="章节目录">${nav}</nav><p class="aside-note">正文来自可编辑 Markdown。六张 SVG 图已嵌入，可离线阅读。绿色为阶段已验收，蓝色为实施中，灰色为待建设，紫色为可选。</p></aside><main><header><div class="eyebrow">ARCHITECTURE · 当前实现与目标边界</div><h1>整个项目的架构，在一处读清楚</h1><p>从语义块和个人思考，到事务、原件、任务、快照及恢复；再到待建设的浏览器学习应用与运维体系。</p><div class="badges"><span class="badge">Rust · PostgreSQL 18</span><span class="badge">5 个 crate</span><span class="badge">C3 已隔离验收</span><span class="badge">C4 实施中</span><span class="badge">API / 前端待建设</span></div></header><div id="empty" class="empty" hidden>没有匹配章节。试试“权限”“版本”“原件”或“恢复”。</div>${content}<footer>更新于 2026-09-30。最新受测 C4 源码 273f106；文档核对基线 031f80d。此文档不构成生产部署或实际恢复通过证明。<br><a href="#chapter-index">返回阅读入口 ↑</a> · <a href="${esc(relativeUrl(path.join(source, 'README.md')))}">架构 Markdown 索引</a></footer></main></div><script>${js}</script></body></html>`;
fs.mkdirSync(path.dirname(output), {recursive: true});
fs.writeFileSync(output, html, 'utf8');
const result = {status: 'GENERATED_LINKS_CHECKED', output, chapters: chapters.length, headings: headingCount,
  tables: tableCount, links: linkCount, diagrams: figures.size, checkedLocalTargets: sources.size,
  sourceBaseline: '031f80d9108b4b5110c89e55cd9dcbb1e4d6db76', testedC4Source: '273f106e7addfcbc8bad3d96f5ba4dd7b8ab30c2'};
fs.writeFileSync(output + '.qa.json', JSON.stringify(result, null, 2) + '\n');
console.log(JSON.stringify(result));
