"""Static artifact QA only: no browser, network, or application execution."""
from pathlib import Path
import hashlib
import json
import re
import xml.etree.ElementTree as ET
from html.parser import HTMLParser

ROOT = Path(__file__).resolve().parent.parent
NS = {'s': 'http://www.w3.org/2000/svg'}
issues=[]
def box(el):
    return tuple(float(el.get(k)) for k in ('x','y','width','height'))
def overlap(a,b):
    x,y,w,h=a; u,v,r,s=b
    return x < u+r-1 and x+w > u+1 and y < v+s-1 and y+h > v+1
def crossing(p,q,b):
    x,y,w,h=b
    if p[0]==q[0]:
        return x+2<p[0]<x+w-2 and min(p[1],q[1])<y+h-2 and max(p[1],q[1])>y+2
    if p[1]==q[1]:
        return y+2<p[1]<y+h-2 and min(p[0],q[0])<x+w-2 and max(p[0],q[0])>x+2
    return False

for f in sorted((ROOT/'docs/diagrams').glob('knowweave-*.svg')):
    tree=ET.parse(f).getroot()
    _,_,vw,vh=map(float,tree.get('viewBox').split())
    nodes=[]
    for g in tree.findall('s:g',NS):
        r=g.find('s:rect',NS)
        if r is None:continue
        b=box(r);nodes.append((g.get('data-node'),b))
        for t in g.findall('s:text',NS):
            # Conservative readable-width estimate, not browser font rendering.
            label=t.text or ''
            fontsize=16 if t.get('class')=='node-title' else 13
            width=sum(fontsize*(.57 if ord(c)<256 else 1) for c in label)
            if float(t.get('x'))+width>b[0]+b[2]-8:
                issues.append((f.stem,'text-width',label))
    for n,b in nodes:
        if b[0]<0 or b[1]<0 or b[0]+b[2]>vw or b[1]+b[3]>vh:
            issues.append((f.stem,'outside',n))
    for i,(n,a) in enumerate(nodes):
        for m,b in nodes[i+1:]:
            if overlap(a,b):issues.append((f.stem,'node-overlap',n,m))
    paths=tree.findall('s:path',NS)
    for i,p in enumerate(paths):
        pts=[tuple(map(float,m)) for m in re.findall(r'[ML]([\d.-]+),([\d.-]+)',p.get('d',''))]
        for n,b in nodes:
            if any(crossing(a,z,b) for a,z in zip(pts,pts[1:])):
                issues.append((f.stem,'edge-through-node',i,n))
    labels=[box(r) for r in tree.findall('s:rect',NS) if r.get('opacity')]
    for i,a in enumerate(labels):
        for n,b in nodes:
            if overlap(a,b):issues.append((f.stem,'label-on-node',i,n))
        for j,b in enumerate(labels[i+1:],i+1):
            if overlap(a,b):issues.append((f.stem,'label-overlap',i,j))

class Audit(HTMLParser):
    def __init__(self):
        super().__init__()
        self.ids=[];self.hrefs=[];self.scripts=[];self.text=[];self.tasks={};self.active_task=None
    def handle_starttag(self,tag,attrs):
        a=dict(attrs)
        if 'id' in a:self.ids.append(a['id'])
        if 'href' in a:self.hrefs.append(a['href'])
        if tag=='script':self.scripts.append(a)
        if tag=='article' and 'data-task' in a:
            self.active_task=a['data-task']
            if self.active_task in self.tasks:issues.append(('html','duplicate-task',self.active_task))
            self.tasks[self.active_task]={'class':a.get('class',''),'text':[]}
        if tag in ('script','img','link','iframe') and a.get('src',a.get('href','')).startswith(('http:','https:')):
            issues.append(('html','remote-runtime',a))
    def handle_data(self,data):
        self.text.append(data)
        if self.active_task is not None:self.tasks[self.active_task]['text'].append(data)
    def handle_endtag(self,tag):
        if tag=='article':self.active_task=None
p=Audit();p.feed((ROOT/'KnowWeave实施路线与架构.html').read_text('utf-8'))
if len(p.ids)!=len(set(p.ids)):issues.append(('html','duplicate-id'))
for h in p.hrefs:
    if h.startswith('#'):
        if h[1:] not in p.ids:issues.append(('html','broken-anchor',h))
    elif h.startswith(('https://','http://')):
        # Outbound evidence links are not runtime dependencies. Validate their
        # pinned repository paths separately against the Git/GitHub tree.
        continue
    elif not (ROOT/h).exists():issues.append(('html','broken-link',h))
qa=json.loads((ROOT/'.build/knowweave-map.qa.json').read_text('utf-8'))
md_file=ROOT/'docs/KnowWeave实施路线与系统架构.md'
md=md_file.read_text('utf-8')
html_text=' '.join(p.text)
expected_tasks=qa['currentTasks']
if [t['id'] for t in expected_tasks]!=list(range(1,10)):
    issues.append(('data','current-task-sequence'))
if [t['id'] for t in qa['parentMilestones']]!=list(range(1,6)):
    issues.append(('data','parent-milestone-sequence'))
if set(p.tasks)!={f'c4-{t["id"]}' for t in expected_tasks}:
    issues.append(('html','current-task-card-set'))
task_states={'done':'分项已验收','next':'实施中','planned':'未开始'}
for t in expected_tasks:
    label=t.get('statusLabel',task_states[t['status']])
    card=p.tasks.get(f'c4-{t["id"]}',{'class':'','text':[]})
    if t['status'] not in card['class'].split():issues.append(('html','task-status',t['id']))
    for field in (t['title'],t['detail'],t['evidence'],label):
        if field not in ' '.join(card['text']):issues.append(('html','task-content',t['id'],field))
    expected_row=f"| Task {t['id']} · {t['title']} | {label} | {t['detail']} {t['evidence']} |"
    if expected_row not in md:issues.append(('markdown','task-row',t['id']))
for field in ('date','state','sourceCommit','progressSummary','progressEvidence','pendingSummary'):
    value=qa['progress'][field]
    for name,content in (('html',html_text),('markdown',md)):
        if value not in content:issues.append((name,'current-progress-field',field))
for title,target in re.findall(r'\[([^\]]+)\]\(([^)]+)\)',md):
    if not target.startswith(('https:','http:','#')) and not (md_file.parent/target).exists():
        issues.append(('markdown','broken-link',title,target))
expected_states={'done':'已验收','next':'实施中','planned':'待建设','optional':'可选','neutral':'上下文'}
for d in qa['diagrams']:
    tree=ET.parse(ROOT/f'docs/diagrams/knowweave-{d["id"]}.svg').getroot()
    mmd=(ROOT/f'docs/diagrams/knowweave-{d["id"]}.mmd').read_text('utf-8')
    if tree.find('s:desc',NS).text!=d['subtitle']:issues.append((d['id'],'subtitle-mismatch'))
    groups={g.get('data-node'):g for g in tree.findall('s:g',NS)}
    if set(groups)!={n['id'] for n in d['nodeRecords']}:issues.append((d['id'],'node-set-mismatch'))
    for n in d['nodeRecords']:
        g=groups.get(n['id'])
        if g is None:continue
        texts=[t.text or '' for t in g.findall('s:text',NS)]
        if texts!=[expected_states[n['status']],n['title'],*n['lines']]:
            issues.append((d['id'],'node-text-or-status-mismatch',n['id']))
        if n['status'] not in g.get('class','').split():issues.append((d['id'],'node-color-status',n['id']))
        label=f'{n["title"]}<br/>{" · ".join(n["lines"])}<br/>【{expected_states[n["status"]]}】'
        if label not in mmd:issues.append((d['id'],'mermaid-node-mismatch',n['id']))
    for note in d['notes']:
        if note not in html_text or note not in md:issues.append((d['id'],'missing-note',note))
files=qa['files']
if len(files)!=14 or len(set(files))!=14:issues.append(('outputs','artifact-set'))
if len(list((ROOT/'docs/diagrams').glob('knowweave-*.svg')))!=6:
    issues.append(('outputs','svg-count'))
stale_claims=('最新整库导入正式测试已执行但失败','尚无成功恢复或资产闭包验收信用',
              '当前阶段 C4 · 五任务实施线','当前阶段 C4：五任务实施线')
hashes={}
for name in files:
    file=ROOT/name
    if not file.is_file():issues.append(('outputs','missing-file',name));continue
    content=file.read_text('utf-8')
    for claim in stale_claims:
        if claim in content:issues.append((name,'stale-progress',claim))
    hashes[name]=hashlib.sha256(file.read_bytes()).hexdigest()
result={'mode':'static content, links and SVG geometry; browser rendering not verified',
        'date':qa['date'],'source_commit':qa['sourceCommit'],'issues':issues,
        'current_tasks_checked':len(expected_tasks),'parent_milestones_checked':len(qa['parentMilestones']),
        'svg_count':len(qa['diagrams']),'html_links':len(p.hrefs),
        'remote_runtime_dependencies':len(p.scripts),'output_sha256':hashes}
(ROOT/'.build/knowweave-map.static-qa.json').write_text(json.dumps(result,ensure_ascii=False,indent=2),'utf-8')
print(json.dumps(result,ensure_ascii=False,indent=2))
raise SystemExit(bool(issues))
