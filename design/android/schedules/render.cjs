#!/usr/bin/env node
// Optional local rendering and smoke checks; never invokes Pi or an SSH helper.
// Needs puppeteer-core and a local Chromium (CHROME can override its path).
'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const puppeteer = require('puppeteer-core');
const chrome = process.env.CHROME || [
  '/opt/google/chrome/chrome',
  '/usr/bin/chromium',
  '/usr/bin/chromium-browser',
  '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
].find(file => fs.existsSync(file));
if (!chrome) throw new Error('Set CHROME to a local Chromium executable.');
const base = pathToFileURL(path.join(__dirname, 'index.html')).href;
const screenshots = [
  ['01-schedules', 'list'], ['02-new', 'new'], ['03-details', 'detail'],
  ['04-result', 'result'], ['05-offline', 'offline'], ['06-evening', 'list', 'evening'],
];

async function checks(page) {
  await page.goto(base, { waitUntil: 'load' });
  assert.equal(await page.$$eval('#gallery figure', elements => elements.length), 6);
  assert.deepEqual(await page.evaluate(() => {
    const ids = [...document.querySelectorAll('[id]')].map(el => el.id);
    return ids.filter((id, index) => ids.indexOf(id) !== index);
  }), []);
  await page.click('button[data-view="demo"]');
  await page.click('#demo-phone [data-id="review"]');
  await page.click('#demo-phone [data-action="toggle"]');
  assert.equal(await page.$eval('#demo-phone [role="switch"]', el => el.getAttribute('aria-checked')), 'false');
  assert.match(await page.$eval('#demo-phone', el => el.innerText), /No future runs while paused/);
  await page.click('#demo-phone [data-action="toggle"]');
  await page.click('#demo-phone [data-action="run-now"]');
  assert.equal(await page.evaluate(() => document.activeElement.dataset.action), 'cancel');
  await page.keyboard.press('Escape');
  assert.equal(await page.$('#demo-phone [role="dialog"]'), null);
  await page.click('#demo-phone [data-action="run-now"]');
  await page.click('#demo-phone [data-action="confirm-run"]');
  assert.match(await page.$eval('#demo-phone', el => el.innerText), /Accepted isn’t/);
  assert.match(await page.$eval('#demo-phone', el => el.innerText), /no prompt was submitted/);

  await page.select('#route', 'new');
  await page.click('#demo-phone [data-frequency="cron"]');
  await page.$eval('#demo-phone input[name="cron"]', el => {
    el.value = '61 9 * * 1-5'; el.dispatchEvent(new Event('input', { bubbles: true }));
  });
  await page.click('#demo-phone button[form="schedule-form"]');
  assert.equal(await page.$('#demo-phone [role="dialog"]'), null);
  assert.match(await page.$eval('#toast', el => el.innerText), /between 0 and 59/);
  await page.$eval('#demo-phone input[name="cron"]', el => {
    el.value = '15 10 * * 1-5'; el.dispatchEvent(new Event('input', { bubbles: true }));
  });
  await page.$eval('#demo-phone input[name="name"]', el => {
    el.value = 'Dependency <check>'; el.dispatchEvent(new Event('input', { bubbles: true }));
  });
  await page.select('#demo-phone select[name="zone"]', 'UTC');
  assert.match(await page.$eval('#demo-phone [data-preview]', el => el.innerText), /10:15/);
  await page.click('#demo-phone button[form="schedule-form"]');
  assert.equal(await page.$eval('#demo-phone [data-action="save"]', el => el.disabled), true);
  await page.click('#consent');
  assert.equal(await page.$eval('#demo-phone [data-action="save"]', el => el.disabled), false);
  await page.click('#demo-phone [data-action="save"]');
  assert.equal(await page.$eval('#demo-phone .detail-name', el => el.textContent), 'Dependency <check>');
  assert.match(await page.$eval('#demo-phone .facts', el => el.innerText), /15 10 \* \* 1-5/);
  assert.equal(await page.$('#demo-phone .detail-name check'), null);
  await page.click('#demo-phone [data-action="delete-schedule"]');
  assert.match(await page.$eval('#demo-phone [role="dialog"]', el => el.innerText), /Existing sessions and project files are kept/);
  await page.click('#demo-phone [data-action="delete"]');
  assert.equal(await page.$$eval('#demo-phone .job-row', elements => elements.length), 3);

  await page.select('#route', 'offline');
  assert.match(await page.$eval('#demo-phone', el => el.innerText), /Unknown/);
  assert.equal(await page.$('#demo-phone [data-action="run-now"]'), null);
  await page.click('#demo-phone [data-action="retry"]');
  await page.click('#demo-phone [data-id="review"]');
  await page.click('#demo-phone [data-action="history"]');
  await page.click('#demo-phone [data-action="skipped"]');
  assert.match(await page.$eval('#demo-phone', el => el.innerText), /no prompt was submitted/);
  await page.click('#demo-phone [data-action="history"]');
  await page.click('#demo-phone [data-action="interrupted"]');
  assert.match(await page.$eval('#demo-phone', el => el.innerText), /rather than blindly repeating/);

  assert.equal(await page.evaluate(() => when(occurrences('0 9 * * 1-5', 'UTC')[0], 'UTC').time), '09:00');
  assert.equal(await page.evaluate(() => occurrences('0 9 * * 7', 'UTC')[0].getUTCDay()), 0);
  assert.equal(await page.evaluate(() => occurrences('0 0 1 1 *', 'UTC').length), 0);
  await page.select('#route', 'setup');
  assert.match(await page.$eval('#demo-phone', el => el.innerText), /hasn’t been installed/);

  for (const width of [360, 412, 768, 1280]) {
    await page.setViewport({ width, height: 1000, deviceScaleFactor: 1 });
    await page.goto(base, { waitUntil: 'load' });
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, `Horizontal page overflow at ${width}px`);
  }
  console.log('Passed: navigation, pause/resume, confirmation, cron preview/validation, consent, escaped input, delete, offline/setup, interrupted history, responsive widths.');
}

(async () => {
  const browser = await puppeteer.launch({ executablePath: chrome, headless: true, args: ['--disable-dev-shm-usage'] });
  try {
    const page = await browser.newPage();
    const errors = [], network = [];
    page.on('pageerror', error => errors.push(String(error)));
    page.on('request', request => { if (/^https?:/.test(request.url())) network.push(request.url()); });
    await page.setViewport({ width: 1280, height: 1000, deviceScaleFactor: 1 });
    await checks(page);
    assert.deepEqual(errors, [], 'Browser JavaScript errors');
    assert.deepEqual(network, [], 'Prototype must not make network requests');
    if (!process.argv.includes('--check')) {
      fs.mkdirSync(path.join(__dirname, 'screens'), { recursive: true });
      await page.setViewport({ width: 412, height: 915, deviceScaleFactor: 1 });
      for (const [name, route, theme] of screenshots) {
        await page.goto(`${base}?screen=${route}&single=1${theme ? '&theme='+theme : ''}`, { waitUntil: 'load' });
        await page.screenshot({ path: path.join(__dirname, 'screens', name+'.png') });
        console.log(`screens/${name}.png`);
      }
      await page.setViewport({ width: 1280, height: 1000, deviceScaleFactor: 1 });
      await page.goto(base, { waitUntil: 'load' });
      await page.screenshot({ path: path.join(__dirname, 'overview.png'), fullPage: true });
      console.log('overview.png');
      assert.deepEqual(errors, [], 'Browser JavaScript errors during rendering');
      assert.deepEqual(network, [], 'Network requests during rendering');
    }
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode=1; });
