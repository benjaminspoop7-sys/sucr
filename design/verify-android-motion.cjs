const puppeteer = require('C:/Users/Benjamin/AppData/Roaming/npm/node_modules/puppeteer-core');
const EXE = 'C:/Users/Benjamin/AppData/Local/ms-playwright/chromium-1248/chrome-win64/chrome.exe';
const URL = 'file:///D:/RE/Sucr/design/sucr-android-motion-prototype.html';
const OUT = 'D:/RE/Sucr/design/shots-android-motion';

function sleep(ms) { return new Promise(r => setTimeout(r, ms)); }

(async () => {
  const fs = require('fs');
  fs.mkdirSync(OUT, { recursive: true });
  const browser = await puppeteer.launch({ executablePath: EXE, headless: true, args: ['--no-sandbox', '--allow-file-access-from-files', '--window-size=1400,900'] });
  const page = await browser.newPage();
  await page.setViewport({ width: 1400, height: 900, deviceScaleFactor: 1 });
  const errors = [];
  page.on('pageerror', e => errors.push('pageerror: ' + e.message));

  const results = [];
  const check = (name, ok, detail) => { results.push((ok ? 'PASS' : 'FAIL') + '  ' + name + (detail ? '  [' + detail + ']' : '')); };

  // Every step gets its own page load: a defect in one demo cannot poison the next.
  async function fresh() {
    await page.goto(URL, { waitUntil: 'load' });
    await sleep(650);
  }
  const shot = async (n) => { await page.screenshot({ path: OUT + '/' + n + '.png', fullPage: false }); };
  const report = () => page.$eval('#report', el => el.textContent.replace(/\s+/g, ' ').trim().slice(0, 200));
  const state = () => page.evaluate(() => ({
    title: document.querySelector('#appbar-title').textContent,
    screens: [...document.querySelectorAll('.screen')].map(s => s.id + '=' + (s.hidden ? 'H' : 'V')).join(' '),
    sheet: document.querySelector('#sheet').style.transform,
    bar: document.querySelector('#addbar').hidden ? 'H' : 'V',
    kbd: document.querySelector('#kbd').classList.contains('up'),
  }));
  const css = (sel, prop) => page.evaluate(([s, p]) => {
    const el = document.querySelector(s);
    return el ? getComputedStyle(el)[p] : 'NO-ELEMENT';
  }, [sel, prop]);

  // Hit-test before clicking so an overlay that swallows the tap is named,
  // instead of being reported as a defect of the element underneath it.
  async function tap(sel, { timeout = 2000 } = {}) {
    const deadline = Date.now() + timeout;
    let why = 'unknown';
    while (Date.now() < deadline) {
      const hit = await page.evaluate((s) => {
        const el = document.querySelector(s);
        if (!el || el.hidden) return { ok: false, why: 'missing-or-hidden' };
        const r = el.getBoundingClientRect();
        if (r.width === 0 || r.height === 0) return { ok: false, why: 'zero-size' };
        const top = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
        if (!top) return { ok: false, why: 'no-point' };
        if (el.contains(top) || top.contains(el)) { el.click(); return { ok: true }; }
        return { ok: false, why: 'blocked by ' + (top.id ? '#' + top.id : String(top.className)) };
      }, sel);
      if (hit.ok) return true;
      why = hit.why;
      await sleep(100);
    }
    results.push('BLOCKED  tap ' + sel + ' -> ' + why + ' @ ' + JSON.stringify(await state()));
    return false;
  }
  async function seg(id, pick) {
    await page.evaluate(([s, p]) => {
      const btns = [...document.querySelectorAll(s + ' button')];
      const b = typeof p === 'number' ? btns[p] : btns.find(x => x.dataset.v === p);
      b.click();
    }, [id, pick]);
    await sleep(250);
  }
  async function toDetail() { await tap('#row-host .row-dl'); await sleep(600); }

  const steps = [];
  const step = (label, fn) => steps.push({ label, fn });

  step('01 boot structure', async () => {
    await fresh(); await shot('01-initial');
    const c = await page.evaluate(() => ({
      screens: document.querySelectorAll('.screen').length,
      visible: [...document.querySelectorAll('.screen')].filter(s => !s.hidden).length,
      rows: document.querySelectorAll('.row-dl').length,
      mapCells: document.querySelectorAll('#map-grid span').length,
      wavePath: (document.querySelector('#wave-path').getAttribute('d') || '').length,
      navItems: document.querySelectorAll('.nav-item').length,
    }));
    check('boot structure', c.screens === 5 && c.visible === 1 && c.rows >= 6 && c.mapCells === 120
      && c.wavePath > 50 && c.navItems === 3, JSON.stringify(c));
  });

  step('02 container transform', async () => {
    await fresh();
    await toDetail();
    const s = await state();
    check('row tap -> detail only', s.screens.includes('scr-detail=V') && s.screens.includes('scr-list=H'), s.screens);
    check('detail title', s.title === 'Download detail', s.title);
    check('ghost cleaned up', await page.evaluate(() => document.querySelector('#ghost').hidden));
    await shot('02-detail');
  });

  step('03 predictive back', async () => {
    await fresh(); await toDetail();
    const box = await page.$eval('#back-edge', el => { const r = el.getBoundingClientRect(); return { x: r.x, y: r.y + 240 }; });
    await page.mouse.move(box.x + 4, box.y);
    await page.mouse.down();
    await page.mouse.move(box.x + 150, box.y, { steps: 8 });
    const mid = await page.evaluate(() => document.querySelector('#scr-detail').style.transform);
    await shot('03-predictive-scrub');
    await page.mouse.move(box.x + 300, box.y, { steps: 6 });
    await page.mouse.up();
    await sleep(900);
    const s = await state();
    check('back scrub moves the detail', /translateX/.test(mid), mid);
    check('back past threshold commits', s.screens.includes('scr-list=V') && s.screens.includes('scr-detail=H'), s.screens);
    check('detail transform reset', await page.evaluate(() => document.querySelector('#scr-detail').style.transform === ''));
  });

  step('04 sheet drag + fling', async () => {
    await fresh(); await toDetail();
    await tap('#open-sheet');
    await sleep(700);
    const up = await page.evaluate(() => document.querySelector('#sheet').style.transform);
    check('sheet opens fully', /translateY\(0%\)/.test(up), up);
    await shot('04-sheet-open');
    const g = await page.$eval('#sheet .grab', el => { const r = el.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; });
    await page.mouse.move(g.x, g.y);
    await page.mouse.down();
    await page.mouse.move(g.x, g.y + 60, { steps: 2 });
    const partial = await page.evaluate(() => document.querySelector('#sheet').style.transform);
    check('sheet follows the finger', /translateY\((?!0%)\d/.test(partial), partial);
    await shot('04b-sheet-dragged');
    await page.mouse.move(g.x, g.y + 230, { steps: 2 });
    await page.mouse.up();
    await sleep(900);
    const rep = await report();
    const down = await page.evaluate(() => document.querySelector('#sheet').style.transform);
    check('fling velocity dismisses the sheet', /translateY\(100%\)/.test(down), down);
    check('release reported the velocity handoff', /px\/frame/.test(rep), rep);
  });

  step('05 sheet action = retraction, not celebration', async () => {
    await fresh(); await toDetail();
    await tap('#open-sheet');
    await sleep(700);
    await tap('#sheet-remove');
    await sleep(500);
    const s = await page.evaluate(() => ({
      sheet: document.querySelector('#sheet').style.transform,
      snack: !document.querySelector('#snackbar').hidden,
    }));
    check('retraction exits down its origin edge', /translateY\(100%\)/.test(s.sheet), s.sheet);
    check('snackbar is non-modal', s.snack, JSON.stringify(s));
    check('retraction reported as T-024/T-027', /T-024|T-027/.test(await report()), await report());
    await shot('05-sheet-remove');
  });

  step('06 overlay never survives a route change', async () => {
    await fresh(); await toDetail();
    await tap('#open-sheet');
    await sleep(700);
    await page.evaluate(() => document.querySelector('.nav-item[data-dest="map"]').click());
    await sleep(800);
    const s = await state();
    check('navigating away dismisses the sheet', /translateY\(100%\)/.test(s.sheet), s.sheet);
    check('route change lands on map', s.screens.includes('scr-map=V') && s.screens.includes('scr-detail=H'), s.screens);
    check('scrim gone', await page.evaluate(() => !document.querySelector('#scrim').classList.contains('show')));
    await page.evaluate(() => document.querySelector('.nav-item[data-dest="list"]').click());
    await sleep(600);
    check('nothing left swallowing taps', await tap('#row-host .row-dl'));
  });

  step('07 top-level cross-fade', async () => {
    await fresh();
    await tap('.nav-item[data-dest="map"]');
    await sleep(450);
    const anim = await page.evaluate(() => getComputedStyle(document.querySelector('#scr-map')).animationName);
    const s = await state();
    check('map cross-fades, does not slide', /fadeEnter/i.test(anim) && s.screens.includes('scr-map=V'), anim + ' | ' + s.screens);
    check('cross-fade justified as unordered peers', /T-011/.test(await report()), await report());
  });

  step('08 FAB -> anchored bar tracks the IME', async () => {
    await fresh();
    await tap('#fab');
    await sleep(900);
    const st = await page.evaluate(() => {
      const bar = document.querySelector('#addbar').getBoundingClientRect();
      const kbd = document.querySelector('#kbd').getBoundingClientRect();
      const r = document.querySelector('#addbar-cancel').getBoundingClientRect();
      const top = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
      return {
        shown: !document.querySelector('#addbar').hidden,
        kbdUp: document.querySelector('#kbd').classList.contains('up'),
        focused: document.activeElement && document.activeElement.id,
        barAboveKbd: bar.bottom <= kbd.top + 1,
        cancelHit: top ? (top.id || String(top.className)) : 'none',
      };
    });
    check('add bar opens anchored + keyboard inset', st.shown && st.kbdUp && st.focused === 'addbar-input', JSON.stringify(st));
    check('bar is inset clear of the keyboard', st.barAboveKbd, 'barBottom<=kbdTop=' + st.barAboveKbd);
    check('Cancel is reachable with the IME up', /addbar-cancel|btn/.test(st.cancelHit), st.cancelHit);
    await shot('06-addbar');
    await tap('#addbar-cancel');
    await sleep(600);
    const after = await state();
    check('add bar closes on cancel', after.bar === 'H', JSON.stringify(after));
    check('keyboard drops with the bar', !after.kbd, 'kbd=' + after.kbd);
  });

  step('09 wizard ordered peers', async () => {
    await fresh();
    const reachable = await tap('#goto-wizard');
    check('wizard row is wired up', reachable);
    await sleep(700);
    const s = await state();
    check('wizard opened from the list', s.screens.includes('scr-wizard=V') && s.screens.includes('scr-list=H'), s.screens);
    await tap('#wiz-next');
    await sleep(400);
    const fwd = await page.evaluate(() => getComputedStyle(document.querySelector('#wiz-body')).animationName);
    check('wizard forward is lateral', /latInR/i.test(fwd), fwd);
    await tap('#wiz-next');
    await sleep(400);
    await tap('#wiz-back');
    await sleep(400);
    const back = await page.evaluate(() => getComputedStyle(document.querySelector('#wiz-body')).animationName);
    check('wizard Back plays the inverse', /latInL/i.test(back), back);
    await shot('07-wizard');
  });

  step('10 O-003 wave control on its own surface', async () => {
    await fresh();
    const before = await page.evaluate(() => document.querySelector('#wave-path').getAttribute('d'));
    await sleep(400);
    const moving = await page.evaluate(() => document.querySelector('#wave-path').getAttribute('d'));
    check('wave animates by default', before !== moving);
    await tap('#wave-pause');
    await sleep(450);
    const a = await page.evaluate(() => document.querySelector('#wave-path').getAttribute('d'));
    await sleep(450);
    const b = await page.evaluate(() => document.querySelector('#wave-path').getAttribute('d'));
    const label = await page.$eval('#wave-pause', e => e.textContent + '/' + e.getAttribute('aria-pressed'));
    check('wave pause freezes the geometry', a === b, label);
    check('wave pause relabels on the surface', /^Resume\/true$/.test(label), label);
    await tap('#wave-pause');
    await sleep(300);
    check('wave resumes', await page.evaluate(() => document.querySelector('#wave-pause').getAttribute('aria-pressed') === 'false'));
  });

  step('11 O-001 reduced motion', async () => {
    await fresh();
    await seg('#motion-seg', 'reduced');
    const r = await page.evaluate(() => {
      const cs = getComputedStyle(document.documentElement);
      return {
        tokens: ['1', '2', '3', '4', '5', '6'].map(n => cs.getPropertyValue('--dur-' + n).trim()),
        rowOpacity: getComputedStyle(document.querySelector('#row-host > *')).opacity,
      };
    });
    check('reduced zeroes every duration token', r.tokens.every(t => /^0m?s$/.test(t)), JSON.stringify(r.tokens));
    check('reduced leaves content visible at rest', r.rowOpacity === '1', 'opacity=' + r.rowOpacity);
    await tap('#row-host .row-dl');
    await sleep(120);
    const s = await state();
    check('reduced motion still navigates (snaps, never waits for animationend)',
      s.screens.includes('scr-detail=V') && s.screens.includes('scr-list=H'), s.screens);
    await shot('08-reduced');
  });

  step('12 T-019 loading sequence', async () => {
    await fresh();
    await page.evaluate(() => document.querySelector('#slow-seg button[data-v="on"]').click());
    await sleep(60);
    const before = await page.evaluate(() => document.querySelectorAll('#row-host .skel-wrap').length);
    await sleep(140);
    const at150 = await page.evaluate(() => document.querySelectorAll('#row-host .skel-wrap').length);
    await sleep(200);
    const during = await page.evaluate(() => document.querySelectorAll('#row-host .skel-wrap').length);
    await sleep(1700);
    const after = await page.evaluate(() => ({ skel: document.querySelectorAll('#row-host .skel-wrap').length, rows: document.querySelectorAll('#row-host .row-dl').length }));
    check('skeleton gated behind the ~150ms wait', before === 0 && during === 3, 't=60ms:' + before + ' t=200ms:' + at150 + ' t=400ms:' + during);
    check('skeleton cross-fades into real rows', after.skel === 0 && after.rows === 4, JSON.stringify(after));
  });

  step('13 anti-pattern toggles', async () => {
    await fresh();
    // rowshim ships off (compliant), the other two ship on (defect shown by default)
    await tap('#toggle-rowshim');
    await sleep(300);
    check('row shimmer defect renders', await page.evaluate(() => document.querySelectorAll('#row-host .shimmer').length >= 4));
    check('row shimmer defect names O-003', /O-003/.test(await report()), await report());
    await tap('#toggle-rowshim');
    await sleep(300);
    check('row shimmer defect clears', await page.evaluate(() => document.querySelectorAll('#row-host .shimmer').length === 0));

    const def = await css('.wiz-dot i', 'transitionTimingFunction');
    check('wizard overshoot ships as the default defect', /1\.56/.test(def), def);
    await tap('#toggle-wizbad');
    await sleep(300);
    const fixed = await css('.wiz-dot i', 'transitionTimingFunction');
    check('wizard overshoot fixes to the standard curve', !/1\.56/.test(fixed), fixed);
    check('wizard fix names T-004', /T-004/.test(await report()), await report());

    await toDetail();
    await tap('#fav-chip');                       // activation
    await sleep(250);
    await tap('#fav-chip');                       // retraction, defect still ON
    await sleep(250);
    const badRep = await report();
    check('defect path: retraction borrows the celebration (T-027)', /T-027.*Anti-pattern|Anti-pattern.*T-027/i.test(badRep), badRep.slice(0, 110));
    await tap('#toggle-favbad');
    await sleep(200);
    const favState = await page.evaluate(() => document.querySelector('#toggle-favbad').getAttribute('aria-pressed'));
    check('retraction-celebration defect toggles off', favState === 'false', 'aria-pressed=' + favState);
    await tap('#fav-chip');
    await sleep(250);
    await tap('#fav-chip');
    await sleep(250);
    const goodRep = await report();
    check('fixed path: retraction is calmer than activation (T-027)', /colour only|faster tier/i.test(goodRep) && /T-027/.test(goodRep), goodRep.slice(0, 110));

    await seg('#scheme-seg', 'expressive');
    const ex = { fab: await css('.fab', 'transitionTimingFunction'), bar: await css('.tbar-fill', 'transitionTimingFunction') };
    check('expressive hits containers, spares data bars', /1\.56/.test(ex.fab) && !/1\.56/.test(ex.bar), JSON.stringify(ex));
    await seg('#scheme-seg', 'standard');
    const std = await css('.fab', 'transitionTimingFunction');
    check('standard scheme really has no overshoot', !/1\.56/.test(std), std);
    await shot('09-final');
  });

  for (const s of steps) {
    try { await s.fn(); }
    catch (e) { results.push('THREW   ' + s.label + ' -> ' + e.message); }
  }

  console.log('=== PAGE ERRORS (' + errors.length + ') ===');
  errors.slice(0, 25).forEach(e => console.log(e));
  console.log('=== CHECKS ===');
  results.forEach(r => console.log(r));
  const bad = results.filter(r => !r.startsWith('PASS')).length;
  console.log('=== ' + (results.length - bad) + '/' + results.length + ' clean, ' + errors.length + ' page errors ===');
  await browser.close();
  process.exit(bad ? 2 : 0);
})().catch(e => { console.error('HARNESS ERROR', e); process.exit(1); });
