const puppeteer = require('C:/Users/Benjamin/AppData/Roaming/npm/node_modules/puppeteer-core');
const vis = (p,l)=>p.evaluate(()=>({cur:document.querySelector('#appbar-title').textContent,
  screens:[...document.querySelectorAll('.screen')].map(s=>s.id+'='+(s.hidden?'H':'V')).join(' '),
  wizHidden:document.querySelector('#goto-wizard').closest('[hidden]')?true:false,
  sheetT:document.querySelector('#sheet').style.transform})).then(r=>console.log(l,JSON.stringify(r)));
(async () => {
  const b = await puppeteer.launch({executablePath:'C:/Users/Benjamin/AppData/Local/ms-playwright/chromium-1248/chrome-win64/chrome.exe', headless:true, args:['--no-sandbox','--allow-file-access-from-files']});
  const p = await b.newPage(); await p.setViewport({width:1400,height:900});
  p.on('pageerror', e=>console.log('PAGEERROR',e.message));
  await p.goto('file:///D:/RE/Sucr/design/sucr-android-motion-prototype.html',{waitUntil:'load'});
  await new Promise(r=>setTimeout(r,600)); await vis(p,'boot');
  await p.click('#row-host .row-dl'); await new Promise(r=>setTimeout(r,500)); await vis(p,'tap1');
  const e=await p.$eval('#back-edge',el=>{const r=el.getBoundingClientRect();return{x:r.x,y:r.y+300}});
  await p.mouse.move(e.x+4,e.y); await p.mouse.down(); await p.mouse.move(e.x+150,e.y,{steps:8}); await p.mouse.move(e.x+280,e.y,{steps:6}); await p.mouse.up();
  await new Promise(r=>setTimeout(r,900)); await vis(p,'after-back-commit');
  await p.click('#row-host .row-dl'); await new Promise(r=>setTimeout(r,500)); await vis(p,'tap2');
  await p.click('#open-sheet'); await new Promise(r=>setTimeout(r,600)); await vis(p,'sheet-open');
  const sb=await p.$eval('#sheet',el=>{const r=el.getBoundingClientRect();return{x:r.x+r.width/2,y:r.y+12}});
  await p.mouse.move(sb.x,sb.y); await p.mouse.down(); await p.mouse.move(sb.x,sb.y+90,{steps:3}); await p.mouse.move(sb.x,sb.y+210,{steps:3}); await p.mouse.up();
  await new Promise(r=>setTimeout(r,700)); await vis(p,'after-fling');
  await p.click('.nav-item[data-dest="map"]'); await new Promise(r=>setTimeout(r,500)); await vis(p,'nav-map');
  await p.click('.nav-item[data-dest="list"]'); await new Promise(r=>setTimeout(r,400)); await vis(p,'nav-list');
  await p.click('#fab'); await new Promise(r=>setTimeout(r,500)); await vis(p,'fab-open');
  await p.click('#addbar-cancel'); await new Promise(r=>setTimeout(r,400)); await vis(p,'bar-cancel');
  await b.close();
})();
