import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {meta,stages,diagrams,principles,c3Tasks,c4Tasks,c4ParentMilestones,references} from './knowweave-map.data.mjs';

const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const out=path.join(root,'docs','diagrams');
fs.mkdirSync(out,{recursive:true});
const esc=s=>String(s).replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const mdRef=ref=>/^https:\/\//.test(ref)?ref:'../'+ref;
const state={done:'已验收',next:'实施中',planned:'待建设',optional:'可选',neutral:'上下文'};
const taskState={done:'分项已验收',next:'实施中',planned:'未开始'};
const taskLabel=t=>t.statusLabel??taskState[t.status];
const palette=`
:root{--paper:#ffffff;--ink:#172338;--muted:#506078;--edge:#6b7b92;--line:#c7d0df;--done:#eaf5ee;--done-ink:#246547;--next:#ebf1ff;--next-ink:#294ea5;--planned:#f2f4f8;--planned-ink:#506078;--optional:#f3ecfb;--optional-ink:#714993;--neutral:#fafbfc;}
@media(prefers-color-scheme:dark){:root{--paper:#131b29;--ink:#e6edf7;--muted:#b0bdd0;--edge:#97a9c0;--line:#415269;--done:#163a2d;--done-ink:#8fd8b3;--next:#203758;--next-ink:#a7c2ff;--planned:#283447;--planned-ink:#c1ccdc;--optional:#382c49;--optional-ink:#d3b5f2;--neutral:#202b3b;}}
`;
const textWidth=s=>[...s].reduce((n,c)=>n+(/[\u0000-\u00ff]/.test(c)?7.1:13),0);
const nodeXY=n=>({x:70+n.col*350,y:70+n.row*158,w:294,h:100});
const port=(n,p)=>{const {x,y,w,h}=nodeXY(n);return {top:[x+w/2,y],bottom:[x+w/2,y+h],left:[x,y+h/2],right:[x+w,y+h/2]}[p]};
const endpoint=(a,b)=>a.row===b.row?(a.col<b.col?['right','left']:['left','right']):(a.row<b.row?['bottom','top']:['top','bottom']);
const qa={date:meta.date,baseline:meta.baseline,sourceCommit:meta.sourceCommit,progress:meta,currentTasks:c4Tasks,parentMilestones:c4ParentMilestones,links:[],diagrams:[],status:'pending'};
for(const [title,ref] of [...references,...stages.map(s=>[s.id,s.link])]){
  if(!/^https:\/\//.test(ref)&&!fs.existsSync(path.join(root,ref)))throw new Error(`Missing document link: ${title}: ${ref}`);
  qa.links.push(ref);
}
function svg(d){
 const height=140+(Math.max(...d.nodes.map(n=>n.row))+1)*158;
 const ids=new Map(d.nodes.map(n=>[n.id,n]));
 if(ids.size!==d.nodes.length)throw new Error('duplicate node: '+d.id);
 let edges='',nodes='';
 for(const e of d.edges){
  const a=ids.get(e.from),b=ids.get(e.to);if(!a||!b)throw new Error('unknown endpoint');
  const [ap,bp]=endpoint(a,b),from=port(a,e.fromPort??ap),to=port(b,e.toPort??bp);
  let pts;
  if(e.route){pts=[from,...e.route,to];}
  else if(a.row===b.row){const pa=port(a,'top'),pb=port(b,'top'),track=pa[1]-28;pts=[pa,[pa[0],track],[pb[0],track],pb];}
  else if(a.col===b.col){pts=[from,to];}
  else{const middle=(from[1]+to[1])/2;pts=[from,[from[0],middle],[to[0],middle],to];}
  const commands=pts.map((p,i)=>(i?'L':'M')+p.join(',')).join(' ');
  edges+=`<path class="edge${e.dashed?' optional-edge':''}" d="${commands}" marker-end="url(#${d.id}-arrow)"/>`;
  if(e.label){
   const segments=pts.slice(1).map((q,i)=>({p:pts[i],q,len:Math.abs(q[0]-pts[i][0])+Math.abs(q[1]-pts[i][1])}));
   const horizontal=segments.filter(s=>s.p[1]===s.q[1]&&s.len>80);
   const {p,q}=(horizontal.length?horizontal:segments).sort((a,b)=>b.len-a.len)[0],vertical=p[0]===q[0];
   const x=e.labelAt?.x??((p[0]+q[0])/2+(vertical?10:0)),y=e.labelAt?.y??((p[1]+q[1])/2-8),anchor=e.labelAt?.anchor??(vertical?'start':'middle');
   const w=textWidth(e.label)+12,rx=anchor==='start'?x-5:x-w/2;
   edges+=`<rect x="${rx}" y="${y-13}" width="${w}" height="19" fill="var(--paper)" opacity=".96"/><text class="edge-label" x="${x}" y="${y}" text-anchor="${anchor}">${esc(e.label)}</text>`;
  }
 }
 for(const n of d.nodes){
  const {x,y,w,h}=nodeXY(n);
  nodes+=`<g class="node ${n.status}" data-node="${n.id}"><rect x="${x}" y="${y}" width="${w}" height="${h}" rx="9"/><text x="${x+16}" y="${y+20}" class="state">${state[n.status]}</text><text x="${x+16}" y="${y+44}" class="node-title">${esc(n.title)}</text>${n.lines.map((line,i)=>`<text x="${x+16}" y="${y+66+i*19}" class="node-body">${esc(line)}</text>`).join('')}</g>`;
 }
 qa.diagrams.push({id:d.id,nodes:d.nodes.length,edges:d.edges.length,width:1120,height,nodeRecords:d.nodes,subtitle:d.subtitle,notes:d.notes});
 return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1120 ${height}" role="img" aria-labelledby="${d.id}-title ${d.id}-desc"><title id="${d.id}-title">${esc(d.title)}</title><desc id="${d.id}-desc">${esc(d.subtitle)}</desc><style>${palette}text{font-family:'Microsoft YaHei','PingFang SC',system-ui,sans-serif;fill:var(--ink)}.node rect{fill:var(--planned);stroke:var(--line);stroke-width:1}.node.done rect{fill:var(--done)}.node.next rect{fill:var(--next)}.node.optional rect{fill:var(--optional)}.node.neutral rect{fill:var(--neutral)}.state{font-size:11px;letter-spacing:1px;fill:var(--planned-ink)}.done .state{fill:var(--done-ink)}.next .state{fill:var(--next-ink)}.optional .state{fill:var(--optional-ink)}.node-title{font-size:16px;font-weight:600}.node-body{font-size:13px;fill:var(--muted)}.edge{fill:none;stroke:var(--edge);stroke-width:1.4}.optional-edge{stroke-dasharray:5 4}.edge-label{font-size:12px;fill:var(--muted)}</style><defs><marker id="${d.id}-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M 0 0 L 10 5 L 0 10 z" fill="var(--edge)"/></marker></defs><rect width="1120" height="${height}" fill="var(--paper)"/>${edges}${nodes}</svg>`;
}
function mermaid(d){
 const safe=s=>String(s).replaceAll('"',"'").replaceAll('|','/');
 return ['flowchart TB',...d.nodes.map(n=>`    ${n.id}["${safe(n.title)}<br/>${safe(n.lines.join(' · '))}<br/>【${state[n.status]}】"]`),...d.edges.map(e=>`    ${e.from} ${e.dashed?'-.':'--'}${e.label?` "${safe(e.label)}" `:''}${e.dashed?'.->':'-->'} ${e.to}`),...['done','next','planned','optional'].map(s=>{const ids=d.nodes.filter(n=>n.status===s).map(n=>n.id);return ids.length?`    class ${ids.join(',')} ${s}`:'';}),'    classDef done fill:#eaf5ee,stroke:#438363,color:#173d2b','    classDef next fill:#ebf1ff,stroke:#4365b4,color:#203866','    classDef planned fill:#f2f4f8,stroke:#8591a3,color:#28354b','    classDef optional fill:#f3ecfb,stroke:#9271b0,color:#4a3362'].filter(Boolean).join('\n');
}
const rendered=diagrams.map(d=>{const s=svg(d);fs.writeFileSync(path.join(out,`knowweave-${d.id}.svg`),s);fs.writeFileSync(path.join(out,`knowweave-${d.id}.mmd`),mermaid(d)+'\n');return {...d,svg:s};});
const stageMd=stages.map(s=>`| ${s.id} ${s.name} | ${state[s.status]} | ${s.deliver} | ${s.gate} | [${s.evidence}](${mdRef(s.link)}) |`).join('\n');
const roadmapMermaid=['flowchart LR',...stages.map((s,i)=>`    M${i}["${s.id}<br/>${s.name}<br/>${state[s.status]}"]`),...stages.slice(1).map((_,i)=>`    M${i} --> M${i+1}`)].join('\n');
const c3Mermaid=['flowchart LR',...c3Tasks.map(t=>`    T${t.id}["Task ${t.id}<br/>${t.title}<br/>${taskLabel(t)}"]`),...c3Tasks.slice(1).map((_,i)=>`    T${i+1} --> T${i+2}`),...['done','next','planned'].map(s=>{const ids=c3Tasks.filter(t=>t.status===s).map(t=>`T${t.id}`);return ids.length?`    class ${ids.join(',')} ${s}`:'';}),'    classDef done fill:#eaf5ee,stroke:#438363,color:#173d2b','    classDef next fill:#ebf1ff,stroke:#4365b4,color:#203866','    classDef planned fill:#f2f4f8,stroke:#8591a3,color:#28354b'].filter(Boolean).join('\n');
const c4Mermaid=['flowchart LR',...c4Tasks.map(t=>`    C4T${t.id}["Task ${t.id}<br/>${t.title}<br/>${taskLabel(t)}"]`),...c4Tasks.slice(1).map((_,i)=>`    C4T${i+1} --> C4T${i+2}`),...['done','next','planned'].map(s=>{const ids=c4Tasks.filter(t=>t.status===s).map(t=>`C4T${t.id}`);return ids.length?`    class ${ids.join(',')} ${s}`:'';}),'    classDef done fill:#eaf5ee,stroke:#438363,color:#173d2b','    classDef next fill:#ebf1ff,stroke:#4365b4,color:#203866','    classDef planned fill:#f2f4f8,stroke:#8591a3,color:#28354b'].filter(Boolean).join('\n');
const nextSteps=meta.nextSteps;
const md=`# ${meta.title}

更新：${meta.date}。当前状态：${meta.state}，\`NOT_PRODUCTION\`。C3 验收源码 HEAD：\`${meta.baseline}\`；当前本地源码检查点：\`${meta.sourceCommit}\`。各项现场验收保留自己的源码身份，不给最新源码转授历史信用。

${meta.intro}

${meta.scope}

[离线图文版](../KnowWeave实施路线与架构.html)。此文为可编辑 Markdown 快照；可维护图数据在 [.build/knowweave-map.data.mjs](../.build/knowweave-map.data.mjs)，生成命令为 \`node .build/build-knowweave-map.mjs\`。

## 实施路线

P0-A、B1–B4、C1–C3 共 8 个分关已验收。${meta.progressSummary}

不用分关数量或累加测试数估算整个项目完成百分比；各阶段历史测试数不能相加。

\`\`\`mermaid
${roadmapMermaid}
\`\`\`

| 阶段 | 状态 | 交付 | 验收门 | 证据或范围 |
| --- | --- | --- | --- | --- |
${stageMd}

## 下一步

${meta.pendingSummary}

${nextSteps.map(([a,b])=>`- **${a}**：${b}`).join('\n')}

## 当前阶段 C4：九任务补齐计划

任务编号依据 [2026-10-04 补齐计划](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/addf1193c98e18c6eadb548ed635c35eaff95707/docs/superpowers/plans/2026-10-04-p0c4-completion.md)，与下面原五任务父里程碑分开。分项通过不关闭父里程碑或 C4。

${meta.progressSummary}

${meta.pendingSummary}

${meta.progressEvidence}

\`\`\`mermaid
${c4Mermaid}
\`\`\`

| 任务 | 状态 | 工作与验收依据 |
| --- | --- | --- |
${c4Tasks.map(t=>`| Task ${t.id} · ${t.title} | ${taskLabel(t)} | ${t.detail} ${t.evidence} |`).join('\n')}

### 原五任务父里程碑（保留原编号）

对应 [2026-09-28 父计划](https://github.com/MansurnamedZhang/learning-system-p0c4/blob/addf1193c98e18c6eadb548ed635c35eaff95707/docs/superpowers/plans/2026-09-28-p0c4-backup-recovery.md)，下表不是当前九任务的编号或完成计数。原 Task3–5 的完整门继续开放。

| 父里程碑 | 状态 | 工作与当前边界 |
| --- | --- | --- |
${c4ParentMilestones.map(t=>`| 原 Task ${t.id} · ${t.title} | ${taskLabel(t)} | ${t.detail} ${t.evidence} |`).join('\n')}

## C3 历史：七任务实施线

Task 1–7 已完成分项审查与隔离验证；Task 7 的完整 Linux/PostgreSQL/Worker 证据经 root 私有只读审计复核，C3 标记为 \`P0_C3_VERIFIED / NOT_PRODUCTION\`。

\`\`\`mermaid
${c3Mermaid}
\`\`\`

| 任务 | 状态 | 工作与验收依据 |
| --- | --- | --- |
${c3Tasks.map(t=>`| Task ${t.id} · ${t.title} | ${taskLabel(t)} | ${t.detail} ${t.evidence} |`).join('\n')}

## 全局约束

${principles.map(([a,b])=>`- **${a}**：${b}`).join('\n')}

${rendered.map(d=>`## ${d.title}\n\n${d.subtitle}\n\n[独立矢量图](diagrams/knowweave-${d.id}.svg) · [Mermaid 源码](diagrams/knowweave-${d.id}.mmd)\n\n\`\`\`mermaid\n${mermaid(d)}\n\`\`\`\n\n${d.notes.map(t=>`- ${t}`).join('\n')}`).join('\n\n')}

## 状态依据

${references.map(([a,b])=>`- [${a}](${mdRef(b)})`).join('\n')}

### 历史证据的范围

${meta.historicalMaintenance}

历史只读子进程、同身份物理克隆、同guard重启与Task5受控用例保留各自原源码；不能相互替代。当前分项采用与历史检查点以验收清单为准；Task8完整十门和四升级已限定通过；规定专项与整组终审、Task9独立故障域仍开放。

路线图只更新文档与图示。${meta.boundarySummary}六张SVG/Mermaid、HTML与Markdown共14份输出由同一数据生成。审核与验证范围见本轮审核记录；历史测试数量不能相加或换算项目完成百分比。
`;
fs.writeFileSync(path.join(root,'docs','KnowWeave实施路线与系统架构.md'),md);
const legend=['done','next','planned','optional'].map(s=>`<span class="legend ${s}"><i></i>${state[s]}</span>`).join('');
const timeline=[...new Set(stages.map(s=>s.phase))].map((phase,i)=>`<div class="phase"><div class="phase-title"><span>0${i+1}</span><h3>${phase}</h3></div><div class="stage-grid">${stages.filter(s=>s.phase===phase).map(s=>`<article class="stage ${s.status}"><div class="stage-top"><strong>${s.id}</strong><span class="pill ${s.status}">${state[s.status]}</span></div><h4>${s.name}</h4><p>${s.deliver}</p><p class="gate"><b>验收：</b>${s.gate}</p><a href="${esc(s.link)}">${esc(s.evidence)} ↗</a></article>`).join('')}</div></div>`).join('');
const graphSections=rendered.map(d=>`<section id="${d.id}" class="graph-section"><div class="section-title"><h2>${esc(d.title)}</h2><a href="docs/diagrams/knowweave-${d.id}.svg">打开矢量图 ↗</a></div><p class="subtitle">${esc(d.subtitle)}</p><div class="diagram-scroll">${d.svg}</div><ul class="notes">${d.notes.map(t=>`<li>${esc(t)}</li>`).join('')}</ul><details><summary>查看对象与连线的文字版本</summary><ul>${d.nodes.map(n=>`<li><b>${esc(n.title)}</b>（${state[n.status]}）：${esc(n.lines.join('；'))}</li>`).join('')}</ul><ol>${d.edges.map(e=>`<li>${esc(d.nodes.find(n=>n.id===e.from).title)} → ${esc(d.nodes.find(n=>n.id===e.to).title)}：${esc(e.label||'依赖')}</li>`).join('')}</ol></details></section>`).join('');
const html=`<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>${meta.title}</title><style>
${palette}
*{box-sizing:border-box}html{scroll-behavior:smooth;scroll-padding-top:90px}body{margin:0;background:var(--paper);color:var(--ink);font:16px/1.8 'Microsoft YaHei','PingFang SC',system-ui,sans-serif}a{color:var(--next-ink);text-underline-offset:4px}a:focus-visible,button:focus-visible,summary:focus-visible{outline:3px solid var(--next-ink);outline-offset:5px}header{max-width:1320px;margin:auto;padding:64px 42px 34px}.eyebrow{font-size:13px;letter-spacing:2px;color:var(--next-ink);font-weight:700}h1{font-size:clamp(30px,4vw,48px);line-height:1.35;letter-spacing:-1px;margin:16px 0}h1 small{display:block;font-weight:400;font-size:.52em;color:var(--muted);margin-top:12px;letter-spacing:0}.intro{max-width:960px;font-size:17px;color:var(--muted)}.stamp{display:flex;align-items:center;gap:14px;flex-wrap:wrap;margin-top:22px;font-size:13px;color:var(--muted)}code{font-family:Consolas,monospace;overflow-wrap:anywhere}.stamp code{padding:4px 9px;background:var(--planned);border-radius:5px}.navbar{position:sticky;top:0;z-index:5;border-top:1px solid var(--line);border-bottom:1px solid var(--line);background:var(--paper)}nav{max-width:1320px;margin:auto;display:flex;gap:22px;flex-wrap:wrap;padding:13px 42px;font-size:14px}nav a{text-decoration:none}main{max-width:1320px;padding:24px 42px 60px;margin:auto}section{margin-bottom:72px}h2{font-size:27px;margin:0;line-height:1.45}h3{font-size:20px;margin:0}h4{font-size:18px;margin:8px 0}.section-title{display:flex;align-items:baseline;justify-content:space-between;gap:16px;flex-wrap:wrap;border-bottom:1px solid var(--line);padding-bottom:14px}.section-title a{font-size:13px}.subtitle{color:var(--muted);margin:14px 0 24px}.legend-row{display:flex;gap:22px;flex-wrap:wrap;font-size:14px;margin:24px 0}.legend{display:inline-flex;gap:8px;align-items:center}.legend i{width:11px;height:11px;border-radius:3px;background:var(--planned-ink)}.legend.done i{background:var(--done-ink)}.legend.next i{background:var(--next-ink)}.legend.optional i{background:var(--optional-ink)}.current{padding:20px 24px;border-left:4px solid var(--next-ink);background:var(--next);margin:22px 0 30px}.current b{display:block;font-size:20px}.current p{margin:6px 0;font-size:14px}.phase{margin:30px 0}.phase-title{display:flex;align-items:center;gap:14px;margin:12px 0}.phase-title span{font:18px Consolas,monospace;color:var(--muted)}.stage-grid{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:16px}.stage{border:1px solid var(--line);border-radius:9px;padding:19px;background:var(--neutral);position:relative}.stage.next{border:2px solid var(--next-ink);background:var(--next);padding:18px}.stage-top{display:flex;justify-content:space-between;gap:8px;align-items:center;font-size:13px}.pill{font-size:12px;border-radius:5px;padding:1px 8px;background:var(--planned);color:var(--planned-ink)}.pill.done{background:var(--done);color:var(--done-ink)}.pill.next{background:var(--next);color:var(--next-ink)}.pill.optional{background:var(--optional);color:var(--optional-ink)}.stage p{font-size:14px;margin:10px 0;color:var(--muted)}.stage .gate{border-top:1px solid var(--line);padding-top:12px}.stage a{font-size:12px}.steps{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:18px 36px}.steps h3{font-size:17px}.steps p{font-size:14px;color:var(--muted);margin-top:6px}.principles{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:18px 36px}.principles dt{font-weight:700}.principles dd{margin:4px 0;color:var(--muted);font-size:14px}.diagram-scroll{overflow-x:auto;border-bottom:1px solid var(--line);margin-bottom:24px}.diagram-scroll svg{display:block;width:100%;height:auto;min-width:900px}.notes{padding-left:23px;font-size:14px;line-height:1.9}.notes li{margin:8px 0}details{font-size:14px;color:var(--muted);margin-top:18px}summary{cursor:pointer;color:var(--ink)}.references{columns:2;column-gap:30px;font-size:14px;padding-left:22px}.references li{break-inside:avoid;margin-bottom:8px}footer{border-top:1px solid var(--line);padding-top:18px;font-size:13px;color:var(--muted)}.footer-actions{display:flex;gap:18px;flex-wrap:wrap;margin:16px 0}button{font:inherit;color:var(--next-ink);background:var(--paper);border:1px solid var(--line);padding:5px 13px;border-radius:5px;cursor:pointer}@media(max-width:850px){header{padding:34px 20px 20px}main{padding:22px 20px}nav{padding:10px 20px;gap:9px 17px}.stage-grid{grid-template-columns:repeat(2,minmax(0,1fr))}.navbar{position:static}}@media(max-width:580px){.stage-grid,.steps,.principles{grid-template-columns:1fr}.references{columns:1}h2{font-size:23px}.intro{font-size:15px}}@media print{.navbar,.footer-actions,details{display:none}header,main{padding:18px 0;max-width:none}.stage-grid{grid-template-columns:repeat(2,minmax(0,1fr))}.stage{break-inside:avoid}.graph-section{break-before:page}.diagram-scroll{overflow:visible}.diagram-scroll svg{min-width:0}a{color:inherit}body{font-size:11pt}section{margin-bottom:28px}}
</style></head><body><header><div class="eyebrow">KNOWWEAVE · ENGINEERING MAP</div><h1>知织 · KnowWeave<small>实施路线与系统架构</small></h1><p class="intro">${esc(meta.intro)}</p><div class="stamp"><span>更新 ${meta.date}</span><code>${meta.state}</code><span>C3 验收 HEAD <code>${meta.baseline.slice(0,7)}</code></span></div></header>
<div class="navbar"><nav aria-label="文档导航"><a href="#roadmap">实施路线</a><a href="#next">当前C4与下一步</a><a href="#history">C3 历史任务线</a>${diagrams.map(d=>`<a href="#${d.id}">${esc(d.title.split(' · ')[1])}</a>`).join('')}<a href="#sources">依据</a></nav></div>
<main><section id="task7" class="current"><b>最新验收快照 · ${esc(meta.date)}</b><p>${esc(meta.snapshotSummary)}</p><p>${esc(meta.progressEvidence)}</p><p><strong>${esc(meta.boundarySummary)}</strong> <a href="docs/p0c4-completion-acceptance.md">验收细节与历史检查点</a></p></section><section id="roadmap"><div class="section-title"><h2>实施路线</h2><a href="docs/KnowWeave实施路线与系统架构.md">可编辑 Markdown ↗</a></div><p class="subtitle">${esc(meta.scope)}</p><div class="legend-row">${legend}</div><div class="current"><b>${esc(meta.roadmapHeadline)}</b><p>${esc(meta.progressSummary)}</p><p>${esc(meta.pendingSummary)}</p><p>不按关数或累计测试数估算总进度百分比。</p></div>${timeline}<p class="subtitle">历史测试数是各阶段当时的验收规模，不能相加；C2 最终全工作区结果为 388 通过、0 失败、3 个既有忽略。</p></section>
<section id="next"><div class="section-title"><h2>当前 C4 · 九任务与下一步</h2><a href="https://github.com/MansurnamedZhang/learning-system-p0c4/blob/addf1193c98e18c6eadb548ed635c35eaff95707/docs/superpowers/plans/2026-10-04-p0c4-completion.md">当前施工单 ↗</a></div><p class="subtitle">${esc(meta.taskPlanScope)}</p><div class="current"><b>下一步执行顺序</b><ol>${nextSteps.map(([a,b])=>`<li><strong>${esc(a)}</strong>：${esc(b)}</li>`).join('')}</ol></div><div class="stage-grid">${c4Tasks.map(t=>`<article class="stage ${t.status}" data-task="c4-${t.id}"><div class="stage-top"><strong>Task ${t.id}</strong><span class="pill ${t.status}">${esc(taskLabel(t))}</span></div><h3>${esc(t.title)}</h3><p>${esc(t.detail)}</p><p class="gate"><b>证据／下一门：</b>${esc(t.evidence)}</p></article>`).join('')}</div><details><summary>原五任务父里程碑（保留原编号，原Task3–5完整门仍开放）</summary><p>对应2026-09-28父计划；下面的编号不是当前九任务编号。</p><ul>${c4ParentMilestones.map(t=>`<li><b>原 Task ${t.id} · ${esc(t.title)}</b>（${esc(taskLabel(t))}）：${esc(t.detail)} ${esc(t.evidence)}</li>`).join('')}</ul></details></section>
<section id="history"><div class="section-title"><h2>C3 历史 · 七任务实施线</h2></div><p class="subtitle">Task 1–7 已完成分项审查及隔离整关验证；602 份原始证据经独立只读审计复核，状态为 P0_C3_VERIFIED / NOT_PRODUCTION。</p><div class="stage-grid">${c3Tasks.map(t=>`<article class="stage ${t.status}"><div class="stage-top"><strong>Task ${t.id}</strong><span class="pill ${t.status}">${taskLabel(t)}</span></div><h3>${esc(t.title)}</h3><p>${esc(t.detail)}</p><p class="gate"><b>验收依据：</b>${esc(t.evidence)}</p></article>`).join('')}</div></section>
<section><div class="section-title"><h2>读图前的六个约定</h2></div><dl class="principles">${principles.map(([a,b])=>`<div><dt>${esc(a)}</dt><dd>${esc(b)}</dd></div>`).join('')}</dl></section>${graphSections}
<section id="sources"><div class="section-title"><h2>状态与架构依据</h2></div><ul class="references">${references.map(([a,b])=>`<li><a href="${esc(b)}">${esc(a)}</a></li>`).join('')}</ul><p class="subtitle">当前本地源码检查点：<code>${meta.sourceCommit}</code><br>C3已验收源码HEAD：<code>${meta.baseline}</code>。历史验收与当前源码分别记录；本次文档生成不新增测试执行；既有实际结果按各自原件和采用范围列出，不重放旧批次。</p><p class="subtitle">${esc(meta.progressEvidence)}</p><details><summary>历史验收证据与当前范围的区别</summary><p>${esc(meta.historicalMaintenance)}</p><p>历史源生命周期、只读子进程和同身份物理克隆记录保留原身份。当前分项采用、历史失败与未观测字段分别留存。${esc(meta.boundarySummary)}</p></details></section>
<footer>图与路线由同一份数据生成；正文与SVG可离线阅读，证据链接部分需联网。当前、历史及目标架构分别标注；候选分支同步不代表main合并或生产部署。验证记录见本轮审核记录。<div class="footer-actions"><button type="button" onclick="window.print()">打印 / 保存 PDF</button><a href="docs/KnowWeave实施路线与系统架构.md">Markdown</a><a href=".build/knowweave-map.data.mjs">图数据</a><a href="#roadmap">返回路线图 ↑</a></div></footer></main></body></html>`;
fs.writeFileSync(path.join(root,'KnowWeave实施路线与架构.html'),html);
qa.status='passed';qa.files=['KnowWeave实施路线与架构.html','docs/KnowWeave实施路线与系统架构.md',...diagrams.flatMap(d=>[`docs/diagrams/knowweave-${d.id}.svg`,`docs/diagrams/knowweave-${d.id}.mmd`])];
fs.writeFileSync(path.join(root,'.build','knowweave-map.qa.json'),JSON.stringify(qa,null,2)+'\n');
console.log(JSON.stringify({status:qa.status,diagrams:diagrams.length,stages:stages.length,links:qa.links.length,files:qa.files.length}));
