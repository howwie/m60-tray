// Run with `npm test` from this folder (see README.md). Drives the page in headless Chromium with
// a stand-in for the Tauri API, so no app build or speaker is needed.
const path = require('path');
const { pathToFileURL } = require('url');
const { chromium } = require('playwright');
const { shot } = require('./helpers');
const page_ = path.join(__dirname, '..', 'ui', 'index.html');
let fails = 0;
const check = (name, ok, extra = '') => { console.log((ok ? 'PASS ' : 'FAIL ') + name + (ok ? '' : '  ' + extra)); if (!ok) fails++; };

(async () => {
  const browser = await chromium.launch();
  for (const scheme of ['light', 'dark']) {
    const ctx = await browser.newContext({ viewport: { width: 300, height: 214 }, deviceScaleFactor: 2, colorScheme: scheme });
    const page = await ctx.newPage();
    const errors = [];
    page.on('pageerror', e => errors.push(String(e)));
    await page.addInitScript(() => {
      window.calls = []; window.listeners = {};
      window.__TAURI__ = {
        core: { invoke: (cmd, args) => { window.calls.push([cmd, args]); return Promise.resolve(cmd === 'get_state' ? window.initial : undefined); } },
        event: { listen: (name, cb) => { window.listeners[name] = cb; return Promise.resolve(() => {}); } },
      };
      window.initial = { connected: false, controllable: false, statusLine: 'Searching for the speaker...', volume: null, maxVolume: 16, detail: '' };
      window.push = (s) => window.listeners.state({ payload: s });
    });
    await page.goto(pathToFileURL(page_).href); // a file:// URL that also works for Windows paths
    await page.waitForTimeout(100);

    if (scheme === 'light') {
      check('starts disabled and shows the status', await page.locator('#slider').isDisabled() && (await page.textContent('#status')).startsWith('Searching'));
      check('no level bubble when not connected, even on hover', await (async () => { await page.hover('#slider', { force: true }); const o = await page.$eval('#bubble', e => getComputedStyle(e).opacity); await page.mouse.move(0, 0); return o === '0'; })());
      await shot(page, 'disconnected.png');
    }

    await page.evaluate(() => window.push({ connected: true, controllable: true, statusLine: 'Connected - volume 7 - Bluetooth - EQ Music', volume: 7, maxVolume: 16, detail: 'Bluetooth - EQ Music' }));
    check(`[${scheme}] enabled and shows the volume`, !(await page.locator('#slider').isDisabled()) && (await page.inputValue('#slider')) === '7');
    check(`[${scheme}] no big number any more`, (await page.$('#value')) === null && (await page.$('#max')) === null);
    check(`[${scheme}] bubble hidden until hovered`, (await page.$eval('#track', e => e.classList.contains('show'))) === false);
    await page.hover('#slider');
    await page.waitForTimeout(200);
    const shown = await page.$eval('#bubble', e => ({ text: e.textContent, op: getComputedStyle(e).opacity, r: e.getBoundingClientRect() }));
    check(`[${scheme}] hovering shows the level`, shown.text === '7' && shown.op === '1', JSON.stringify(shown));
    const sb = await page.locator('#slider').boundingBox();
    const thumbX = sb.x + 10 + (7 / 16) * (sb.width - 20);
    check(`[${scheme}] bubble sits over the thumb`, Math.abs((shown.r.left + shown.r.width / 2) - thumbX) < 2 && shown.r.bottom <= sb.y + 4, `${shown.r.left + shown.r.width / 2} vs ${thumbX}`);
    check(`[${scheme}] detail line`, (await page.textContent('#detail')) === 'Bluetooth - EQ Music');
    await shot(page, `connected-${scheme}.png`);
    await page.mouse.move(150, 5);
    await page.waitForTimeout(200);
    check(`[${scheme}] leaving hides the level`, (await page.$eval('#bubble', e => getComputedStyle(e).opacity)) === '0');
    await shot(page, `idle-${scheme}.png`);

    if (scheme === 'light') {
      await page.evaluate(() => { window.calls.length = 0; });
      await page.click('#up');
      await page.waitForTimeout(200);
      check('+ sends volume 8', JSON.stringify(await page.evaluate(() => window.calls)) === JSON.stringify([['set_volume', { value: 8 }]]));
      check('slider follows at once, bubble text too', (await page.inputValue('#slider')) === '8' && (await page.textContent('#bubble')) === '8');

      // an older value from the speaker must not yank the slider back right after the user moved it
      await page.evaluate(() => window.push({ connected: true, controllable: true, statusLine: 's', volume: 7, maxVolume: 16, detail: '' }));
      check('stale echo is ignored right after user input', (await page.inputValue('#slider')) === '8');
      await page.waitForTimeout(700);
      await page.evaluate(() => window.push({ connected: true, controllable: true, statusLine: 's', volume: 12, maxVolume: 16, detail: '' }));
      check('a later change from the speaker is shown', (await page.inputValue('#slider')) === '12');

      // dragging sends a throttled stream, always ending on the final value
      await page.evaluate(() => { window.calls.length = 0; });
      const box = await page.locator('#slider').boundingBox();
      await page.mouse.move(box.x + 5, box.y + box.height / 2);
      await page.mouse.down();
      for (let i = 0; i <= 20; i++) { await page.mouse.move(box.x + 5 + (box.width - 10) * i / 20, box.y + box.height / 2); await page.waitForTimeout(10); }
      await page.mouse.move(box.x + box.width - 5, box.y - 40); // drag off the slider: still dragging
      check('level stays visible while dragging, even off the slider', await page.$eval('#track', e => e.classList.contains('show')));
      await page.mouse.up();
      await page.waitForTimeout(300);
      const calls = await page.evaluate(() => window.calls.filter(c => c[0] === 'set_volume').map(c => c[1].value));
      check('drag is throttled (fewer than the 21 moves)', calls.length > 0 && calls.length <= 12, JSON.stringify(calls));
      check('drag ends on the value shown', calls[calls.length - 1] === Number(await page.inputValue('#slider')) && calls[calls.length - 1] === 16, JSON.stringify(calls));

      await page.evaluate(() => { window.calls.length = 0; });
      await page.focus('#slider');
      await page.keyboard.press('Home');
      await page.waitForTimeout(150);
      check('keyboard Home sets 0', (await page.inputValue('#slider')) === '0' && (await page.evaluate(() => window.calls.at(-1)))?.[1]?.value === 0);
      await page.click('#down');
      await page.waitForTimeout(150);
      check('- at 0 stays at 0', (await page.inputValue('#slider')) === '0');

      await page.evaluate(() => { window.calls.length = 0; });
      await page.mouse.move(150, 80);
      await page.mouse.wheel(0, -100);
      await page.waitForTimeout(150);
      check('wheel up adds one', (await page.inputValue('#slider')) === '1');

      await page.evaluate(() => { window.calls.length = 0; });
      await page.keyboard.press('Escape');
      check('Escape asks the backend to hide the popup', (await page.evaluate(() => window.calls)).some(c => c[0] === 'hide_popup'));

      await page.evaluate(() => window.push({ connected: false, controllable: false, statusLine: 'Disconnected - retrying...', volume: null, maxVolume: 16, detail: '' }));
      check('disconnect disables the controls again', await page.locator('#slider').isDisabled() && await page.locator('#up').isDisabled() && (await page.$eval('#track', e => e.classList.contains('show'))) === false);

      await page.evaluate(() => window.push({ connected: true, controllable: true, statusLine: 's', volume: 5, maxVolume: 10, detail: '' }));
      await page.waitForTimeout(700); // past the window in which the page ignores older speaker values
      await page.evaluate(() => window.push({ connected: false, controllable: true, status: 'Speaker free for other devices', statusLine: 'Speaker free for other devices - volume 6', volume: 6, maxVolume: 16, detail: 'AUX - EQ Music' }));
      check('sharing, released: controls stay usable with the last known volume', !(await page.locator('#slider').isDisabled()) && (await page.inputValue('#slider')) === '6');
      check('...and the header says the speaker is free', (await page.textContent('#status')) === 'Speaker free for other devices');
      await page.evaluate(() => { window.calls.length = 0; });
      await page.click('#up');
      await page.waitForTimeout(150);
      check('...a change is sent (the app connects for it)', JSON.stringify(await page.evaluate(() => window.calls)) === JSON.stringify([['set_volume', { value: 7 }]]));
      await page.evaluate(() => window.push({ connected: false, controllable: false, status: 'Speaker not found', statusLine: 'Speaker not found', volume: null, maxVolume: 16, detail: '' }));
      check('nothing known: controls off', await page.locator('#slider').isDisabled());
      await page.evaluate(() => window.push({ connected: true, controllable: true, statusLine: 's', volume: 5, maxVolume: 10, detail: '' }));
      await page.waitForTimeout(700);
      check('a lower maximum from the speaker is respected', (await page.getAttribute('#slider', 'max')) === '10');
    }
    // ---- media ----
    const np = () => page.evaluate(() => [document.getElementById('np-title').textContent, document.getElementById('np-artist').textContent]);
    const mediaDisabled = () => page.$$eval('button.media', bs => bs.map(b => b.disabled));
    const base = { connected: true, controllable: true, status: 'Connected', statusLine: 's', volume: 5, maxVolume: 16, detail: '' };
    await page.evaluate((b) => window.push({ ...b, bluetooth: true, mediaEnabled: true, nowPlaying: { title: 'The history of light (classic)', artist: 'Planet Money \u2022 04/11/2026' } }), base);
    check(`[${scheme}] Bluetooth: shows what is playing`, JSON.stringify(await np()) === JSON.stringify(['The history of light (classic)', 'Planet Money \u2022 04/11/2026']), JSON.stringify(await np()));
    check(`[${scheme}] Bluetooth: media buttons on`, JSON.stringify(await mediaDisabled()) === '[false,false,false,false]');
    await shot(page, `media-${scheme}.png`);
    if (scheme === 'light') {
      await page.evaluate(() => { window.calls.length = 0; });
      for (const id of ['prev', 'play', 'pause', 'next']) await page.click('#' + id);
      const sent = await page.evaluate(() => window.calls.filter(c => c[0] === 'media').map(c => c[1].action));
      check('buttons send previous 5, play 0, pause 1, next 4', JSON.stringify(sent) === '[5,0,1,4]', JSON.stringify(sent));

      await page.evaluate((b) => window.push({ ...b, bluetooth: true, mediaEnabled: true, nowPlaying: null }), base);
      check('Bluetooth with nothing to report: "No track info"', (await np())[0] === 'No track info' && !(await mediaDisabled())[1]);

      await page.evaluate((b) => window.push({ ...b, bluetooth: false, mediaEnabled: false, nowPlaying: null }), base);
      check('other inputs: buttons off with the Bluetooth-only message', (await np())[0] === 'Media controls only work on the Bluetooth input' && (await mediaDisabled()).every(Boolean));
      await shot(page, 'media-aux.png');
      await page.evaluate(() => { window.calls.length = 0; });
      await page.click('#play', { force: true });
      check('...and clicking them does nothing', (await page.evaluate(() => window.calls)).length === 0);

      await page.evaluate((b) => window.push({ ...b, connected: false, controllable: false, volume: null, bluetooth: true, mediaEnabled: false, nowPlaying: null }), base);
      check('not connected: says so, buttons off', (await np())[0] === 'Not connected' && (await mediaDisabled()).every(Boolean));

      const long = 'A very long podcast episode title that will never fit in this little popup window';
      await page.evaluate(([b, t]) => window.push({ ...b, bluetooth: true, mediaEnabled: true, nowPlaying: { title: t, artist: '' } }), [base, long]);
      const fits = await page.$eval('#np-title', e => e.scrollWidth > e.clientWidth && e.getBoundingClientRect().right <= window.innerWidth);
      check('a long title is cut off with an ellipsis, inside the window', fits);
      check('...and its full text is in the tooltip', (await page.getAttribute('#np-title', 'title')) === long);
      const overflow = await page.evaluate(() => document.documentElement.scrollHeight > window.innerHeight);
      check('everything fits the popup height', !overflow);
    }
    check(`[${scheme}] no page errors`, errors.length === 0, errors.join('; '));
    await ctx.close();
  }
  await browser.close();
  console.log(fails ? `${fails} FAILED` : 'ALL PASSED');
  process.exit(fails ? 1 : 0);
})();
