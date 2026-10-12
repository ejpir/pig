#!/usr/bin/env node
// Local-only browser checks and deterministic PNG generation. No Pi, SSH, providers or native app.
// node design/android/swarm/render.cjs [--check]
// Requires puppeteer-core + Chromium; alternatively use an existing puppeteer install.
'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
let puppeteer;
try { puppeteer = require('puppeteer-core'); } catch { puppeteer = require('puppeteer'); }
let bundledChrome;
try { bundledChrome = require('puppeteer').executablePath(); } catch {}
const chrome = process.env.CHROME || [bundledChrome, '/usr/bin/chromium', '/opt/google/chrome/chrome', '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'].find(file => file && fs.existsSync(file));
if (!chrome) throw new Error('Set CHROME to a local Chromium executable.');
// Build first so the checked-in preview never needs sibling CSS/JS/font resources.
const html = require('./build.cjs').build();
const base = pathToFileURL(path.join(__dirname, 'index.html')).href;
const shots = [
  ['01-live-map','map'], ['02-exchanges','messages'], ['03-agent','agent'], ['04-decision','question'],
  ['05-large-swarm','many'], ['06-offline','offline'], ['07-finished','done'], ['08-evening','map','evening'],
];
async function text(page, selector='#demo-phone') { return page.$eval(selector, el=>el.innerText); }
async function jump(page, route) { await page.select('#route', route); }
async function checks(page) {
  await page.setViewport({width:1380,height:1100,deviceScaleFactor:1});
  await page.goto(base,{waitUntil:'load'});
  assert.equal(await page.$$eval('#gallery figure',els=>els.length),8);
  assert.equal(await page.$$eval('#gallery .node, #gallery .edge-button',els=>els.every(el=> { const r=el.getBoundingClientRect(); return r.width>=48&&r.height>=48; })),true,'Graph hit targets >= 48px');
  assert.deepEqual(await page.evaluate(()=> {
    const ids=[...document.querySelectorAll('[id]')].map(el=>el.id);
    return ids.filter((id,i)=>ids.indexOf(id)!==i);
  }),[], 'Unique IDs, including SVG markers');
  await page.click('[data-view="demo"]');
  assert.match(await text(page),/API is waiting on UI, not on you/);
  await page.click('#demo-phone .edge-button');
  assert.equal(await page.$eval('#demo-phone [role="dialog"]',el=>el.getAttribute('aria-label')),'API ↔ UI');
  assert.match(await text(page),/accepted by broker/);
  assert.equal(await page.evaluate(()=>document.activeElement.dataset.action),'close');
  assert.equal(await page.$eval('.toolbar',el=>el.inert),true);
  await page.keyboard.down('Shift');
  await page.keyboard.press('Tab');
  await page.keyboard.up('Shift');
  assert.equal(await page.evaluate(()=>document.activeElement.dataset.action),'sample-reply');
  await page.keyboard.press('Escape');
  assert.equal(await page.$('#demo-phone [role="dialog"]'),null);
  assert.equal(await page.evaluate(()=>document.activeElement.classList.contains('edge-button')),true);
  await page.click('#demo-phone .edge-button');
  await page.click('#demo-phone [data-action="sample-reply"]');
  assert.match(await text(page),/resolves|resolved by reply #14/);
  await page.keyboard.press('Escape');
  assert.match(await text(page),/Cursor contract agreed/);
  await page.click('#demo-phone [data-route="messages"]');
  await page.click('#demo-phone [data-filter="questions"]');
  assert.equal(await page.$$eval('#demo-phone .exchange',els=>els.length),2);
  await page.click('#demo-phone [data-filter="artifacts"]');
  assert.equal(await page.$$eval('#demo-phone .exchange',els=>els.length),1);
  await page.click('#demo-phone .exchange');
  assert.match(await text(page),/Immutable, read-only reference/);
  await page.keyboard.press('Escape');
  await jump(page,'map');
  await page.click('#demo-phone [data-agent="api"]');
  assert.match(await text(page),/Isolated worktree · api-03/);
  await page.click('#demo-phone [data-action="stop-agent"]');
  await page.keyboard.press('Escape');
  assert.equal(await page.$('#demo-phone [data-action="stop-agent"]')!==null,true);
  await page.click('#demo-phone [data-action="stop-agent"]');
  await page.click('#demo-phone [data-action="confirm-stop-agent"]');
  assert.match(await text(page),/Stop requested, not acknowledged/);
  await page.click('#demo-phone [data-action="ack-stop"]');
  assert.match(await text(page),/This agent has stopped/);
  await page.click('#demo-phone [data-action="exchange"]');
  assert.equal(await page.$('#demo-phone [data-action="sample-reply"]'),null);
  assert.match(await text(page),/Wait cancelled by peer Stop/);
  await page.keyboard.press('Escape');
  await page.click('#demo-phone [data-action="back-map"]');
  assert.match(await text(page),/1 stopped/);
  assert.equal(await page.$eval('#demo-phone .ui small',el=>el.textContent),'● Working');
  await page.click('#reset');
  await jump(page,'question');
  assert.equal(await page.$eval('#demo-phone [data-action="answer"]',el=>el.disabled),true);
  await page.click('#demo-phone [data-choice="keep"]');
  assert.equal(await page.$eval('#demo-phone [data-action="answer"]',el=>el.disabled),false);
  assert.match(await text(page),/Picking an option does not send it/);
  await page.click('#demo-phone [data-action="answer"]');
  assert.match(await text(page),/Sample answer acknowledged/);
  await jump(page,'many');
  await page.click('#demo-phone [data-group="Tests"]');
  assert.equal(await page.$$eval('#demo-phone .sheet .agent-row',els=>els.length),8);
  await page.keyboard.press('Escape');
  await page.click('#demo-phone [data-action="find"]');
  await page.type('#agent-search','API');
  assert.equal(await page.$$eval('#agent-search-results button',els=>els.length),1);
  await page.click('#agent-search-results button');
  assert.match(await text(page),/API · 6 agents/);
  await page.keyboard.press('Escape');
  await page.click('#demo-phone [data-action="back"]');
  assert.match(await text(page),/SWARM \/ S-91/i);
  assert.equal(await page.$('#demo-phone .group-grid')!==null,true);
  await jump(page,'offline');
  assert.match(await text(page),/Current swarm state: unknown/);
  assert.equal(await page.$('#demo-phone [data-action="stop-all"]'),null);
  assert.equal(await page.$('#demo-phone [data-action="steer"]'),null);
  await page.click('#demo-phone [data-action="back"]');
  assert.match(await text(page),/Current swarm state: unknown/);
  await page.keyboard.press('Escape');
  assert.equal(await page.$('#demo-phone [data-action="stop-all"]'),null);
  await page.click('#demo-phone [data-agent="api"]');
  assert.match(await text(page),/Cached before disconnect/);
  assert.equal(await page.$('#demo-phone [data-action="sample-reply"]'),null);
  await page.keyboard.press('Escape');
  await page.click('#demo-phone [data-action="reconnect"]');
  assert.match(await page.$eval('#toast',el=>el.innerText),/No network connection was made/);
  await page.click('#demo-phone [data-action="steer"]');
  await page.type('#steer-draft','Keep <v1> compatibility & review first.');
  await page.keyboard.press('Escape');
  await page.click('#demo-phone [data-action="steer"]');
  assert.equal(await page.$eval('#steer-draft',el=>el.value),'Keep <v1> compatibility & review first.');
  assert.equal(await page.$('#steer-draft v1'),null);
  await page.click('#demo-phone [data-action="send-steer"]');
  assert.match(await page.$eval('#toast',el=>el.innerText),/No prompt was submitted/);
  await jump(page,'done');
  assert.match(await text(page),/main|Main workspace/);
  await page.click('#demo-phone [data-action="review-changes"]');
  assert.match(await text(page),/Nothing is applied by opening Review/);
  await page.keyboard.press('Escape');
  await page.click('#demo-phone [data-action="back"]');
  assert.match(await text(page),/Work handed back/);
  assert.equal(await page.$eval('#demo-phone .api small',el=>el.textContent),'✓ Done');
  assert.equal(await page.$('#demo-phone [data-action="stop-all"]'),null);
  await jump(page,'map');
  await page.click('#demo-phone [data-action="stop-all"]');
  await page.click('#demo-phone [data-action="confirm-stop-all"]');
  assert.match(await text(page),/acknowledgement pending/);
  await page.click('#demo-phone [data-action="ack-stop"]');
  assert.match(await text(page),/Stop acknowledged by studio-mac/);
  assert.equal(await page.$('#demo-phone [data-action="steer"]'),null);
  assert.equal(await page.$eval('#demo-phone .scout small',el=>el.textContent),'✓ Done');
  await page.click('#reset');
  for(const width of [320,360,412,768,1380]) {
    await page.setViewport({width,height:1000,deviceScaleFactor:1});
    await page.goto(base,{waitUntil:'load'});
    assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false,`Gallery overflow at ${width}`);
  }
  for(const [width,height] of [[320,640],[412,915],[640,360]]) {
    await page.setViewport({width,height,deviceScaleFactor:1});
    for(const [,route] of shots.slice(0,7)) {
      await page.goto(`${base}?single=1&screen=${route}`,{waitUntil:'load'});
      assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false,`${route} page overflow at ${width}`);
      assert.equal(await page.$eval('.screen-scroll',el=>el.scrollWidth>el.clientWidth+1),false,`${route} inner overflow at ${width}`);
    }
  }
  console.log('Passed: IDs, map/exchange/receipts, dialog focus, sample replies, filters, artifacts, Stop/cancel, decisions, group roster/search, offline, draft preservation/escaping, Review, responsive layouts.');
}
async function isolatedPreviewChecks(browser) {
  // Android supplies only the HTML text. No checkout/base URL, local files or network.
  const page = await browser.newPage();
  const requested = [], errors = [];
  page.on('pageerror', error => errors.push(String(error)));
  await page.setRequestInterception(true);
  page.on('request', request => {
    if (request.url().startsWith('data:')) return request.continue();
    requested.push(request.url());
    return request.abort();
  });
  try {
    await page.setViewport({width:412, height:915, deviceScaleFactor:1});
    await page.emulateMediaFeatures([{name:'prefers-color-scheme',value:'dark'}]);
    await page.setContent(html, {waitUntil:'load'});
    await page.evaluate(() => document.fonts.ready);
    assert.equal(await page.$$eval('link[rel="stylesheet"], script[src]', els => els.length), 0);
    assert.equal(await page.$$eval('#gallery figure', els => els.length), 8);
    assert.equal(await page.$eval('#gallery', el => getComputedStyle(el).display), 'grid');
    assert.equal(await page.$eval('body', el => getComputedStyle(el).backgroundColor), 'rgb(13, 17, 22)');
    assert.equal(await page.evaluate(() => [...document.fonts].every(font => font.status === 'loaded')), true, 'Embedded fonts load without the checkout');
    await page.click('[data-view="demo"]');
    await page.click('#demo-phone .edge-button');
    assert.match(await text(page), /accepted by broker/);
    await page.click('#demo-phone [data-action="sample-reply"]');
    await page.keyboard.press('Escape');
    assert.match(await text(page), /Cursor contract agreed/);
    await page.select('#route', 'many');
    assert.equal(await page.$$eval('#demo-phone .group-card', els => els.length), 4);
    await page.click('#demo-phone [data-group="Tests"]');
    assert.equal(await page.$$eval('#demo-phone .sheet .agent-row', els => els.length), 8);
    await page.keyboard.press('Escape');
    await page.click('[data-view="gallery"]');
    await page.click('#theme-toggle');
    assert.equal(await page.$eval('body', el => getComputedStyle(el).backgroundColor), 'rgb(236, 231, 225)');
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
    assert.deepEqual(requested, [], 'The HTML-only preview must not request sibling files or network resources');
    assert.deepEqual(errors, [], 'JavaScript errors in the HTML-only preview');
    console.log('Passed: Android-style HTML-only preview, embedded styling/fonts, eight screens, interactions and themes with all external requests blocked.');
  } finally { await page.close(); }
}
(async()=> {
  const browser=await puppeteer.launch({executablePath:chrome,headless:true,args:['--disable-dev-shm-usage']});
  try {
    const page=await browser.newPage();
    const errors=[],network=[],failed=[];
    page.on('pageerror',error=>errors.push(String(error)));
    page.on('request',request=>{if(/^https?:/.test(request.url()))network.push(request.url());});
    page.on('requestfailed',request=>failed.push(request.url()));
    await page.emulateMediaFeatures([{name:'prefers-color-scheme',value:'light'},{name:'prefers-reduced-motion',value:'reduce'}]);
    await checks(page);
    await isolatedPreviewChecks(browser);
    assert.deepEqual(errors,[],'JavaScript errors');
    assert.deepEqual(network,[],'Network requests');
    assert.deepEqual(failed,[],'Missing local assets');
    if(!process.argv.includes('--check')) {
      fs.mkdirSync(path.join(__dirname,'screens'),{recursive:true});
      await page.setViewport({width:412,height:915,deviceScaleFactor:1});
      for(const [name,route,theme] of shots) {
        await page.goto(`${base}?single=1&screen=${route}${theme?'&theme='+theme:''}`,{waitUntil:'load'});
        await page.evaluate(()=>document.fonts.ready);
        await page.screenshot({path:path.join(__dirname,'screens',name+'.png')});
        console.log(`screens/${name}.png`);
      }
      await page.setViewport({width:1380,height:1100,deviceScaleFactor:1});
      await page.goto(base,{waitUntil:'load'});
      await page.evaluate(()=>document.fonts.ready);
      await page.screenshot({path:path.join(__dirname,'overview.png'),fullPage:true});
      console.log('overview.png');
      assert.deepEqual(errors,[],'JavaScript errors during render');
      assert.deepEqual(network,[],'Network requests during render');
      assert.deepEqual(failed,[],'Missing local assets during render');
    }
  } finally { await browser.close(); }
})().catch(error=>{ console.error(error); process.exitCode=1; });
