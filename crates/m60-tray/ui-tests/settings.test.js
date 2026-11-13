// Run with `npm test` from this folder (see README.md). Drives the page in headless Chromium with
// a stand-in for the Tauri API, so no app build or speaker is needed.
const path = require('path');
const { pathToFileURL } = require('url');
const { chromium } = require('playwright');
const { shot } = require('./helpers');

const page_ = path.join(__dirname, '..', 'ui', 'settings.html');
let fails = 0;
const check = (name, ok, extra = '') => { console.log((ok ? 'PASS ' : 'FAIL ') + name + (ok ? '' : '  ' + extra)); if (!ok) fails++; };

const mock = () => {
  window.calls = []; window.listeners = {};
  // like a hidden window on Windows: no animation frames, so sizing must not depend on them
  window.requestAnimationFrame = () => 0;
  window.settings = {
    startAtLogin: true, share: false, rememberVolume: true, bluetoothOnPowerOff: false, resumeInput: null,
    connected: true, controllable: true,
    model: 'EDIFIER M60', firmware: '2.4.1', appVersion: '0.9.0', buildTime: '2026-11-13 12:57 UTC',
    backlightDuration: 1, backlightSensitivity: 2, btPrompt: true,
    durations: [{ id: 0, name: '5 s' }, { id: 1, name: '10 s' }, { id: 2, name: '20 s' }, { id: 3, name: 'On' }],
    sensitivities: [{ id: 0, name: 'High' }, { id: 1, name: 'General' }, { id: 2, name: 'Low' }],
  };
  window.push = (patch) => { Object.assign(window.settings, patch); window.listeners.settings({ payload: { ...window.settings } }); };
  window.__TAURI__ = {
    core: {
      invoke: (cmd, args) => {
        window.calls.push([cmd, args]);
        if (cmd === 'fit_settings') window.fitHeight = args.height;
        if (cmd === 'get_settings') return Promise.resolve({ ...window.settings });
        if (cmd === 'set_app_setting' && window.failNext) { window.failNext = false; return Promise.reject('could not save'); }
        return Promise.resolve();
      },
    },
    event: { listen: (name, cb) => { window.listeners[name] = cb; return Promise.resolve(() => {}); } },
  };
};

(async () => {
  const browser = await chromium.launch();
  for (const scheme of ['light', 'dark']) {
    const ctx = await browser.newContext({ viewport: { width: 480, height: 720 }, deviceScaleFactor: 2, colorScheme: scheme });
    const page = await ctx.newPage();
    const errors = [];
    page.on('pageerror', e => errors.push(String(e)));
    await page.addInitScript(mock);
    await page.goto(pathToFileURL(page_).href);
    await page.waitForTimeout(150);
    const calls = () => page.evaluate(() => window.calls.filter(c => c[0] !== 'get_settings' && c[0] !== 'fit_settings'));
    const clear = () => page.evaluate(() => { window.calls.length = 0; });
    const checked = (sel) => page.$eval(sel, e => e.checked);

    check(`[${scheme}] app switches show the saved settings`,
      (await checked('[data-app=startAtLogin]')) && !(await checked('[data-app=share]')) && (await checked('[data-app=rememberVolume]')) && !(await checked('[data-app=bluetoothOnPowerOff]')));
    check(`[${scheme}] backlight shows 10 s and Low`, (await checked('#duration-1')) && (await checked('#sensitivity-2')));
    check(`[${scheme}] tone is on`, await checked('#tone'));
    check(`[${scheme}] no connection status in the window`, (await page.$('#status')) === null && !(await page.textContent('body')).includes('Connected - volume'));
    check(`[${scheme}] About: speaker, firmware, app version and build time`,
      (await page.textContent('#model')) === 'EDIFIER M60' && (await page.textContent('#firmware')) === '2.4.1'
      && (await page.textContent('#app-version')) === '0.9.0' && (await page.textContent('#build-time')) === '2026-11-13 12:57 UTC');
    const fitTo = await page.evaluate(() => window.fitHeight ?? null);
    const contentBottom = await page.evaluate(() => document.querySelector('#about-heading').closest('section').getBoundingClientRect().bottom);
    check(`[${scheme}] asks for the window to fit the content (no empty space below About)`,
      fitTo !== null && fitTo >= contentBottom && fitTo - contentBottom <= 20, `asked ${fitTo}, content ends at ${contentBottom}`);
    const scrolls = () => page.evaluate(() => { const m = document.querySelector('main'); return m.scrollHeight > m.clientHeight; });
    // the window as the backend makes it: exactly as tall as asked for
    await page.setViewportSize({ width: 480, height: fitTo });
    await page.waitForTimeout(50);
    check(`[${scheme}] fits without scrolling at the height it asked for`, !(await scrolls()));
    await shot(page, `settings-${scheme}.png`);
    if (scheme === 'dark') { check('[dark] no page errors', errors.length === 0, errors.join('; ')); await ctx.close(); continue; }

    // A hidden window can lay the page out at another width first. Text then wraps onto fewer
    // lines, the first height is too short, and the page must measure again at the real width.
    await page.setViewportSize({ width: 800, height: 720 });
    await page.waitForTimeout(50);
    const wide = await page.evaluate(() => window.fitHeight);
    await page.setViewportSize({ width: 480, height: wide });
    await page.waitForTimeout(50);
    const narrow = await page.evaluate(() => window.fitHeight);
    check('measures again when the real width makes text wrap', narrow > wide, `${wide} -> ${narrow}`);
    await page.setViewportSize({ width: 480, height: narrow });
    await page.waitForTimeout(50);
    check('...and then fits without scrolling', !(await scrolls()));

    await clear();
    await page.click('text=Turn off into Bluetooth');
    check('clicking a row toggles its setting', JSON.stringify(await calls()) === JSON.stringify([['set_app_setting', { key: 'bluetoothOnPowerOff', on: true }]]), JSON.stringify(await calls()));
    await clear();
    await page.click('[data-app=share]');
    check('share toggles', JSON.stringify(await calls()) === JSON.stringify([['set_app_setting', { key: 'share', on: true }]]));

    const before = await page.evaluate(() => window.fitHeight);
    await page.evaluate(() => window.push({ bluetoothOnPowerOff: true, resumeInput: 'AUX' }));
    await page.waitForTimeout(100);
    const after = await page.evaluate(() => window.fitHeight);
    check('the resume note makes the window taller', after > before, `${before} -> ${after}`);
    await page.setViewportSize({ width: 480, height: after });
    await page.waitForTimeout(50);
    check('with that note it still fits', !(await scrolls()));
    check('a pending switch back is explained', (await page.isVisible('#resume')) && (await page.textContent('#resume')).includes('switch it back to AUX'));
    await shot(page, 'settings-resume.png');

    await clear();
    await page.click('label[for=duration-3]');
    await page.click('label[for=sensitivity-0]');
    check('backlight choices send one value at a time',
      JSON.stringify(await calls()) === JSON.stringify([['set_backlight', { duration: 3, sensitivity: null }], ['set_backlight', { duration: null, sensitivity: 0 }]]), JSON.stringify(await calls()));
    await clear();
    await page.click('text=Bluetooth connection tone');
    check('tone toggles', JSON.stringify(await calls()) === JSON.stringify([['set_bt_prompt', { on: false }]]));

    await page.evaluate(() => window.push({ connected: false, controllable: false, model: null, firmware: null, btPrompt: null, backlightDuration: null, backlightSensitivity: null }));
    check('speaker settings are off while the speaker is not reachable', (await page.isDisabled('#duration-0')) && (await page.isDisabled('#tone')) && (await page.isVisible('#speaker-note')));
    check('app settings still work then', !(await page.isDisabled('[data-app=share]')));
    check('About says the speaker is not connected', (await page.textContent('#model')) === 'Not connected' && (await page.textContent('#firmware')) === '-');
    check('...and still shows the app version', (await page.textContent('#app-version')) === '0.9.0');
    await shot(page, 'settings-off.png');

    await page.evaluate(() => window.push({ connected: true, controllable: true, btPrompt: true, backlightDuration: null, backlightSensitivity: 1 }));
    check('backlight waits until both values are known', await page.isDisabled('#duration-0'));

    await page.evaluate(() => { window.failNext = true; });
    await page.click('[data-app=startAtLogin]');
    await page.waitForTimeout(100);
    check('a failed change is reported and the switch goes back', (await page.textContent('#message')).includes('could not save') && (await checked('[data-app=startAtLogin]')));

    check('[light] no page errors', errors.length === 0, errors.join('; '));
    await ctx.close();
  }
  await browser.close();
  console.log(fails ? `${fails} FAILED` : 'ALL PASSED');
  process.exit(fails ? 1 : 0);
})();
