const puppeteer = require('C:/Users/Benjamin/AppData/Roaming/npm/node_modules/puppeteer-core');
(async () => {
  const b = await puppeteer.launch({executablePath:'C:/Users/Benjamin/AppData/Local/ms-playwright/chromium-1248/chrome-win64/chrome.exe', headless:true, args:['--no-sandbox','--allow-file-access-from-files']});
  const p = await b.newPage(); await p.setViewport({width:1400,height:900});
  p.on('pageerror', e=>console.log('PAGEERROR',e.message));
  await p.goto('file:///D:/RE/Sucr/design/sucr-android-motion-prototype.html',{waitUntil:'load'});
  await new Promise(r=>setTimeout(r,600));
  const probe = (sel,label)=>p.evaluate((s,l)=>{const el=document.querySelector(s); if(!el) return label+': MISSING';
    const r=el.getBoundingClientRect(); const cs=getComputedStyle(el);
    const cx=r.x+r.width/2, cy=r.y+Math.min(r.height/2, 20);
    const hit=document.elementFromPoint(cx,cy);
    return l+': '+JSON.stringify({rect:[Math.round(r.x),Math.round(r.y),Math.round(r.width),Math.round(r.height)],display:cs.display,opacity:cs.opacity,anim:cs.animationName,hiddenAncestor:!!el.closest('[hidden]'),hit:hit?hit.className||hit.id||hit.tagName:'null'});},sel,label).then(console.log);
  await probe('#goto-wizard','wizard row initial');
  await p.click('#row-host .row-dl'); await new Promise(r=>setTimeout(r,600));
  await probe('#goto-wizard','wizard row after detail');
  // go back
  const e=await p.$eval('#back-edge',el=>{const r=el.getBoundingClientRect();return{x:r.x,y:r.y+300}});
  await p.mouse.move(e.x+4,e.y); await p.mouse.down(); await p.mouse.move(e.x+280,e.y,{steps:8}); await p.mouse.up();
  await new Promise(r=>setTimeout(r,900));
  await probe('#goto-wizard','wizard row after back');
  await probe('#fab','fab'); await probe('#addbar','addbar'); await probe('#snackbar','snackbar'); await probe('#kbd','kbd strip');
  await b.close();
})();
