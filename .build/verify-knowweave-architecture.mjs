import fs from 'node:fs';
import path from 'node:path';
import {createRequire} from 'node:module';
import {fileURLToPath,pathToFileURL} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const qaDir=path.join(root,'.build','qa','architecture-2026-09-30');
fs.mkdirSync(qaDir,{recursive:true});
const modules=path.join(process.env.USERPROFILE,'.cache','codex-runtimes','codex-primary-runtime','dependencies','node','node_modules');
const require=createRequire(import.meta.url);
const {chromium}=require(path.join(modules,'playwright'));
const html=path.join(root,'KnowWeave架构设计.html');
const errors=[],requests=[];
const browser=await chromium.launch({channel:'msedge',headless:true});
const page=await browser.newPage({viewport:{width:1440,height:1000},deviceScaleFactor:1});
page.on('pageerror',e=>errors.push(e.message));
page.on('console',m=>{if(m.type()==='error')errors.push(m.text());});
page.on('request',r=>{if(/^https?:/i.test(r.url()))requests.push(r.url());});
const assert=(condition,message)=>{if(!condition)throw new Error(message);};
try{
 await page.goto(pathToFileURL(html).href,{waitUntil:'load'});
 const counts=await page.evaluate(()=>({chapters:document.querySelectorAll('.chapter').length,diagrams:document.querySelectorAll('svg').length,tables:document.querySelectorAll('table').length,headings:document.querySelectorAll('h2,h3,h4,h5,h6').length,overflow:document.documentElement.scrollWidth>document.documentElement.clientWidth,links:[...document.querySelectorAll('a[href]')].map(a=>a.getAttribute('href')),ids:[...document.querySelectorAll('[id]')].map(x=>x.id)}));
 assert(counts.chapters===10&&counts.diagrams===6&&counts.tables===19,'Unexpected chapter/diagram/table counts');
 assert(!counts.overflow,'Desktop body horizontal overflow');
 assert(new Set(counts.ids).size===counts.ids.length,'Duplicate document IDs');
 let anchors=0,localLinks=0;
 for(const href of counts.links){
  if(href.startsWith('#')){assert(counts.ids.includes(href.slice(1)),'Missing anchor '+href);anchors++;}
  else if(!/^[a-z]+:/i.test(href)){assert(fs.existsSync(path.resolve(path.dirname(html),decodeURIComponent(href.split('#')[0]))),'Missing local target '+href);localLinks++;}
 }
 await page.screenshot({path:path.join(qaDir,'desktop-top.png')});
 await page.locator('#search').fill('guard');
 const matched=await page.locator('.chapter:visible').count();assert(matched>0&&matched<10,'Search does not filter chapters');
 await page.locator('#search').fill('no-such-topic-987654321');
 assert(await page.locator('.chapter:visible').count()===0&&await page.locator('#empty').isVisible(),'Empty search state missing');
 await page.locator('nav a[href="#chapter-recovery"]').click();
 assert(await page.locator('.chapter:visible').count()===10,'Navigation did not clear search');
 await page.waitForFunction(()=>Math.abs(document.querySelector('#chapter-recovery').getBoundingClientRect().top-20)<3,null,{timeout:5000});
 await page.screenshot({path:path.join(qaDir,'desktop-recovery.png')});
 for(const [chapter,name] of [['overview','overview'],['domain','domain'],['modules','crates'],['assets','jobs'],['recovery','portability'],['deployment','deployment']]){
  await page.locator('#chapter-'+chapter+' figure').screenshot({path:path.join(qaDir,'diagram-'+name+'.png')});
 }
 await page.setViewportSize({width:390,height:844});
 await page.goto(pathToFileURL(html).href,{waitUntil:'load'});
 assert(!(await page.evaluate(()=>document.documentElement.scrollWidth>document.documentElement.clientWidth)),'Mobile body horizontal overflow');
 await page.screenshot({path:path.join(qaDir,'mobile-top.png')});
 await page.locator('nav a[href="#chapter-domain"]').click();
 await page.waitForFunction(()=>Math.abs(document.querySelector('#chapter-domain').getBoundingClientRect().top-20)<3,null,{timeout:5000});
 await page.screenshot({path:path.join(qaDir,'mobile-domain.png')});
 assert(errors.length===0,'Browser errors: '+errors.join('; '));assert(requests.length===0,'Unexpected network requests');
 const result={status:'PASSED',chapters:10,diagrams:6,tables:19,headings:counts.headings,anchorLinksChecked:anchors,localLinksChecked:localLinks,desktop:'1440x1000',mobile:'390x844',searchAndEmptyState:'passed',navigationClearsSearch:'passed',chapterNavigationScroll:'passed',horizontalBodyOverflow:false,browserErrors:errors,networkRequests:requests,screenshots:['desktop-top.png','desktop-recovery.png','mobile-top.png','mobile-domain.png',...['overview','domain','crates','jobs','portability','deployment'].map(n=>'diagram-'+n+'.png')]};
 fs.writeFileSync(path.join(qaDir,'browser-qa.json'),JSON.stringify(result,null,2)+'\n');console.log(JSON.stringify(result));
}finally{await browser.close();}
