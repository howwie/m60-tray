// Run with `npm test` from this folder (see README.md). Drives the page in headless Chromium with
// a stand-in for the Tauri API, so no app build or speaker is needed.
const path = require('path');
const { pathToFileURL } = require('url');
const { chromium } = require('playwright');
const { shot } = require('./helpers');
const page_ = path.join(__dirname, '..', 'ui', 'eq.html');
let fails = 0;
const check = (name, ok, extra = '') => { console.log((ok ? 'PASS ' : 'FAIL ') + name + (ok ? '' : '  ' + extra)); if (!ok) fails++; };

const mock = () => {
  // a tiny fake backend with the same rules as the Rust side
  window.calls = []; window.listeners = {};
  const base = { freqs: [62, 250, 1000, 4000, 8000, 16000], minDb: -3, maxDb: 3, stepDb: 0.5, maxNameBytes: 20 };
  window.db = {
    presets: [
      { id: 1794308922, name: 'Gaming', gains: [-3, -3, 0, 0, 0, 0] },
      { id: 1794308754, name: 'Music', gains: [0.5, 1, -1, 0.5, 0, -1] },
    ],
    activeId: 1794308922, connected: true,
  };
  const dto = () => ({ ...base, presets: JSON.parse(JSON.stringify(db.presets)), activeId: db.activeId, connected: db.connected, controllable: db.controllable ?? db.connected });
  window.emit = () => window.listeners.presets && window.listeners.presets({ payload: dto() });
  let nextId = 1800000000;
  window.__TAURI__ = {
    core: {
      invoke: (cmd, args) => {
        window.calls.push([cmd, args]);
        if (cmd === 'get_presets') return Promise.resolve(dto());
        if (cmd === 'save_preset') {
          const name = args.name.trim();
          if (!name) return Promise.reject('Give the preset a name.');
          if (db.presets.some(p => p.id !== args.id && p.name.toLowerCase() === name.toLowerCase())) return Promise.reject(`There is already a preset called "${name}".`);
          let p;
          if (args.id === null) { p = { id: nextId++, name, gains: [...args.gains] }; db.presets.push(p); }
          else { p = db.presets.find(x => x.id === args.id); p.name = name; p.gains = [...args.gains]; }
          setTimeout(emit, 0);
          return Promise.resolve(JSON.parse(JSON.stringify(p)));
        }
        if (cmd === 'delete_preset') { db.presets = db.presets.filter(p => p.id !== args.id); setTimeout(emit, 0); return Promise.resolve(); }
        if (cmd === 'apply_preset') {
          if (!db.connected) return Promise.reject('The speaker is not connected.');
          db.activeId = args.id; setTimeout(emit, 0); return Promise.resolve();
        }
        return Promise.resolve();
      },
    },
    event: { listen: (name, cb) => { window.listeners[name] = cb; return Promise.resolve(() => {}); } },
  };
};

(async () => {
  const browser = await chromium.launch();
  for (const scheme of ['light', 'dark']) {
    const ctx = await browser.newContext({ viewport: { width: 660, height: 420 }, deviceScaleFactor: 2, colorScheme: scheme });
    const page = await ctx.newPage();
    const errors = [];
    page.on('pageerror', e => errors.push(String(e)));
    page.on('console', m => { if (m.type() === 'error') errors.push(m.text()); });
    await page.addInitScript(mock);
    await page.goto(pathToFileURL(page_).href); // a file:// URL that also works for Windows paths
    await page.waitForTimeout(150);
    const calls = () => page.evaluate(() => window.calls.filter(c => c[0] !== 'get_presets'));
    const clear = () => page.evaluate(() => { window.calls.length = 0; });
    const gains = () => page.$$eval('#bands input', els => els.map(e => Number(e.value)));
    const listText = () => page.$$eval('#list button', els => els.map(e => e.textContent));

    check(`[${scheme}] opens on the preset that is on the speaker`, (await page.inputValue('#name')) === 'Gaming' && JSON.stringify(await gains()) === '[-3,-3,0,0,0,0]');
    check(`[${scheme}] list marks it`, JSON.stringify(await listText()) === JSON.stringify(['GamingOn speaker', 'Music']), JSON.stringify(await listText()));
    check(`[${scheme}] six bands with frequency labels`, JSON.stringify(await page.$$eval('.freq', e => e.map(x => x.textContent))) === JSON.stringify(['62 Hz', '250 Hz', '1 kHz', '4 kHz', '8 kHz', '16 kHz']));
    check(`[${scheme}] clean: Save off, Apply to speaker on`, (await page.isDisabled('#save')) && (await page.textContent('#apply')) === 'Apply to speaker' && !(await page.isDisabled('#apply')));
    await shot(page, `eq-${scheme}.png`);
    if (scheme === 'dark') { check('[dark] no page errors', errors.length === 0, errors.join('; ')); await ctx.close(); continue; }

    // edit with the keyboard
    await page.focus('#bands input >> nth=2');
    await page.keyboard.press('ArrowUp');
    await page.keyboard.press('ArrowUp');
    check('arrow keys move a band in 0.5 dB steps', JSON.stringify(await gains()) === '[-3,-3,1,0,0,0]' && (await page.$$eval('.band output', e => e[2].textContent)) === '+1.0');
    check('dirty: Save on, Save and apply, Discard shown, Delete hidden, dot in list',
      !(await page.isDisabled('#save')) && (await page.textContent('#apply')) === 'Save and apply' && (await page.isVisible('#discard')) && !(await page.isVisible('#delete')) && (await page.$('#list .dirty-dot')) !== null);

    await page.click('#list button >> text=Music');
    check('switching with unsaved changes is refused', (await page.inputValue('#name')) === 'Gaming' && (await page.textContent('#message')).includes('Save or discard'));

    await page.click('#discard');
    check('discard restores the saved values', JSON.stringify(await gains()) === '[-3,-3,0,0,0,0]' && (await page.isDisabled('#save')));

    // wheel
    const band0 = await page.locator('.vslider >> nth=0').boundingBox();
    await page.mouse.move(band0.x + band0.width / 2, band0.y + band0.height / 2);
    await page.mouse.wheel(0, -100);
    await page.waitForTimeout(50);
    check('mouse wheel nudges a band', (await gains())[0] === -2.5);
    await page.mouse.wheel(0, 100); await page.mouse.wheel(0, 100);
    await page.waitForTimeout(50);
    check('and stops at -3 dB', (await gains())[0] === -3);

    // save an edit
    await clear();
    await page.focus('#bands input >> nth=5');
    await page.keyboard.press('End');
    await page.click('#save');
    await page.waitForTimeout(100);
    let c = await calls();
    check('Save sends the id, name and gains', JSON.stringify(c) === JSON.stringify([['save_preset', { id: 1794308922, name: 'Gaming', gains: [-3, -3, 0, 0, 0, 3] }]]), JSON.stringify(c));
    check('then it is clean again', await page.isDisabled('#save') && (await page.textContent('#message')).includes('Saved'));

    // apply a clean preset: no save
    await page.click('#list button >> text=Music');
    await clear();
    await page.click('#apply');
    await page.waitForTimeout(100);
    c = await calls();
    check('Apply on a clean preset only applies', JSON.stringify(c) === JSON.stringify([['apply_preset', { id: 1794308754 }]]), JSON.stringify(c));
    check('badge moves to it', JSON.stringify(await listText()) === JSON.stringify(['Gaming', 'MusicOn speaker']), JSON.stringify(await listText()));

    // new preset, save and apply
    await page.click('#new');
    check('new preset starts flat and unnamed, focus in the name', (await page.inputValue('#name')) === '' && JSON.stringify(await gains()) === '[0,0,0,0,0,0]' && (await page.evaluate(() => document.activeElement.id)) === 'name');
    await page.keyboard.type('A very long preset name that will not fit');
    check('name is cut to 20 bytes', (await page.inputValue('#name')) === 'A very long preset n', await page.inputValue('#name'));
    await page.fill('#name', 'Night');
    await page.focus('#bands input >> nth=0');
    await page.keyboard.press('ArrowDown');
    await clear();
    await page.click('#apply');
    await page.waitForTimeout(150);
    c = await calls();
    check('Save and apply saves a new preset, then applies it', c.length === 2 && c[0][0] === 'save_preset' && c[0][1].id === null && c[0][1].name === 'Night' && c[1][0] === 'apply_preset' && c[1][1].id === 1800000000, JSON.stringify(c));
    check('the list shows it', (await listText()).includes('NightOn speaker'));

    // duplicate name error
    await page.click('#new');
    await page.fill('#name', 'music');
    await page.click('#save');
    await page.waitForTimeout(100);
    check('a duplicate name is reported', (await page.textContent('#message')).includes('already a preset'));
    check('message is shown as an error', (await page.getAttribute('#message', 'class')) === 'error');
    await page.click('#discard');

    // delete needs two clicks
    await page.click('#list button >> text=Gaming');
    await clear();
    await page.click('#delete');
    check('first Delete click only asks', (await calls()).length === 0 && (await page.textContent('#delete')) === 'Click again to delete');
    await page.click('#delete');
    await page.waitForTimeout(100);
    c = await calls();
    check('second click deletes', JSON.stringify(c) === JSON.stringify([['delete_preset', { id: 1794308922 }]]), JSON.stringify(c));
    check('and the editor moves on to another preset', !(await listText()).some(t => t.startsWith('Gaming')) && (await page.inputValue('#name')) === 'Music', await page.inputValue('#name'));

    // disconnected
    await page.evaluate(() => { db.connected = false; emit(); });
    await page.waitForTimeout(50);
    check('apply is disabled without the speaker, saving still works', (await page.isDisabled('#apply')) && (await page.textContent('#connection')).includes('not connected'));
    await page.focus('#bands input >> nth=1');
    await page.keyboard.press('ArrowUp');
    check('...Save stays available', !(await page.isDisabled('#save')) && (await page.isDisabled('#apply')));

    // sharing, released: applying is still possible (it connects briefly)
    await page.click('#discard');
    await page.evaluate(() => { db.connected = false; db.controllable = true; emit(); });
    await page.waitForTimeout(50);
    check('sharing: Apply stays available and the header explains', !(await page.isDisabled('#apply')) && (await page.textContent('#connection')).includes('free for other devices'));
    await page.evaluate(() => { db.controllable = undefined; });

    // empty list
    await page.evaluate(() => { db.presets = []; db.connected = true; emit(); });
    await page.waitForTimeout(50);
    check('an empty list says so and offers a new preset', (await page.textContent('#list')).includes('No presets yet') && (await page.inputValue('#name')) === '');
    await shot(page, 'eq-empty.png');
    check('[light] no page errors', errors.length === 0, errors.join('; '));
    await ctx.close();
  }
  await browser.close();
  console.log(fails ? `${fails} FAILED` : 'ALL PASSED');
  process.exit(fails ? 1 : 0);
})();
