const puppeteer = require('C:/Users/Benjamin/AppData/Roaming/npm/node_modules/puppeteer-core');
(async () => {
  const b = await puppeteer.launch({executablePath:'C:/Users/Benjamin/AppData/Local/ms-playwright/chromium-1248/chrome-win64/chrome.exe', headless:true, args:['--no-sandbox','--allow-file-access-from-files']});
  const p = await b.newPage();
  await p.setViewport({width:1400,height:900});
  p.on('pageerror', e => console.log('PAGEERROR', e.message));
  await p.goto('file:///D:/RE/Sucr/design/sucr-android-motion-prototype.html', {waitUntil:'load'});
  await new Promise(r=>setTimeout(r,600));
  await p.click('#row-host .row-dl');
  await new Promise(r=>setTimeout(r,500));
  console.log('after row tap:', JSON.stringify(await p.evaluate(()=>({
    hidden:[...document.querySelectorAll('.screen')].map(s=>s.id+'='+(s.hidden?'H':'V')),
    current: document.querySelector('#appbar-title').textContent
  }))));
  const box = await p.$eval('#back-edge', el=>{const r=el.getBoundingClientRect();return{x:r.x,y:r.y+300}});
  await p.mouse.move(box.x+4, box.y); await p.mouse.down();
  await p.mouse.move(box.x+150, box.y,{steps:8});
  console.log('mid scrub transform:', await p.evaluate(()=>document.querySelector('#scr-detail').style.transform));
  await p.mouse.move(box.x+280, box.y,{steps:6}); await p.mouse.up();
  await new Promise(r=>setTimeout(r,1000));
  console.log('after commit:', JSON.stringify(await p.evaluate(()=>({
    hidden:[...document.querySelectorAll('.screen')].map(s=>s.id+'='+(s.hidden?'H':'V')),
    detailStyle: document.querySelector('#scr-detail').getAttribute('style')
  }))));
  console.log('open-sheet visible?', await p.evaluate(()=>{
    const el=document.querySelector('#open-sheet'); if(!el) return 'missing';
    const r=el.getBoundingClientRect(); const cs=getComputedStyle(el);
    return JSON.stringify({w:r.width,h:r.height,top:r.top,display:cs.display,visibility:cs.visibility,
      detailHidden:document.querySelector('#scr-detail').hidden, listHidden:document.querySelector('#scr-list').hidden});
  }));
  await b.close();
})();
