const { chromium } = require('playwright');
(async () => {
  const [url, dir, out] = process.argv.slice(2);
  const b = await chromium.launch();
  const pg = await b.newPage({ viewport: { width: 1600, height: 900 } });
  await pg.goto(url); await pg.waitForTimeout(3500);
  const c = pg.getByRole('button', { name: 'Continue' }); if (await c.count()) await c.click();
  await pg.getByText('Choose workspace').first().click(); await pg.waitForTimeout(600);
  await pg.getByRole('button', { name: 'Edit path' }).click(); await pg.waitForTimeout(400);
  const inp = pg.locator('[role=dialog] input').first();
  await inp.fill(dir); await inp.press('Enter'); await pg.waitForTimeout(800);
  await pg.screenshot({ path: out + '/setup1.png' });
  await pg.getByRole('button', { name: 'Open' }).click(); await pg.waitForTimeout(1200);
  await pg.screenshot({ path: out + '/setup2.png' });
  await b.close();
})();
