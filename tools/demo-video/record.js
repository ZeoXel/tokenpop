// Records the dsh web UI via CDP screencast with per-frame wall-clock timestamps.
const { chromium } = require('playwright');
const fs = require('fs');
const [url, prompt, evlog, out] = process.argv.slice(2);
const sleep = (ms) => new Promise(r => setTimeout(r, ms));
(async () => {
  fs.rmSync(out + '/frames', { recursive: true, force: true }); fs.mkdirSync(out + '/frames', { recursive: true });
  const b = await chromium.launch();
  const ctx = await b.newContext({ viewport: { width: 1600, height: 900 }, deviceScaleFactor: 1.2 });
  const pg = await ctx.newPage();
  await pg.goto(url); await sleep(3500);
  const c = pg.getByRole('button', { name: 'Continue' }); if (await c.count()) { await c.click(); await sleep(500); }
  const cdp = await ctx.newCDPSession(pg);
  const frames = []; let n = 0;
  cdp.on('Page.screencastFrame', async (f) => {
    const file = `frames/${String(n++).padStart(5, '0')}.jpg`;
    fs.writeFileSync(out + '/' + file, Buffer.from(f.data, 'base64'));
    frames.push({ file, ts: f.metadata.timestamp });
    cdp.send('Page.screencastFrameAck', { sessionId: f.sessionId }).catch(() => {});
  });
  await cdp.send('Page.startScreencast', { format: 'jpeg', quality: 92, maxWidth: 1920, maxHeight: 1080, everyNthFrame: 1 });
  // Keep frames flowing even when the page is static (a 1px repaint each frame).
  await pg.evaluate(() => { const d = document.createElement('div'); d.style.cssText = 'position:fixed;right:0;bottom:0;width:1px;height:1px;z-index:99999;pointer-events:none'; document.body.appendChild(d); let k = 0; (function f() { d.style.opacity = (k++ % 2) ? '0.01' : '0.02'; requestAnimationFrame(f); })(); });
  await sleep(1500);
  const box = pg.locator('[contenteditable="true"]').first();
  await box.click(); await sleep(400);
  await pg.keyboard.type(prompt, { delay: 32 });
  await sleep(700);
  const sendAt = Date.now();
  await pg.getByRole('button', { name: 'Send message' }).click();
  await pg.mouse.move(1590, 450, { steps: 8 });
  let doneAt = 0;
  for (let i = 0; i < 1800; i++) {
    await sleep(100);
    for (const name of [/^Allow once$/i, /^Allow$/i, /^Approve$/i, /^Allow for this session$/i]) {
      const a = pg.getByRole('button', { name }); 
      if (await a.count() && await a.first().isVisible()) { await sleep(1200); await a.first().click().catch(() => {}); console.log('approved', String(name)); break; }
    }
    const lines = fs.existsSync(evlog) ? fs.readFileSync(evlog, 'utf8').trim().split('\n') : [];
    if (lines.some(l => { try { const e = JSON.parse(l); return e.kind === 'done' && e.t > sendAt; } catch { return false; } })) { doneAt = Date.now(); break; }
  }
  await sleep(3500);
  await cdp.send('Page.stopScreencast');
  await sleep(300);
  fs.writeFileSync(out + '/frames.json', JSON.stringify({ frames, sendAt, doneAt, nowAtEnd: Date.now() }));
  console.log('frames', frames.length, 'first ts', frames[0] && frames[0].ts, 'sendAt', sendAt / 1000, 'done in', doneAt ? (doneAt - sendAt) / 1000 : 'timeout');
  await pg.screenshot({ path: out + '/final.png' });
  await b.close();
})();
