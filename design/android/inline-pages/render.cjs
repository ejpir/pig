#!/usr/bin/env node
// Local browser checks and deterministic screenshots. No native app, Pi, SSH or provider.
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
if (!chrome) throw new Error('Set CHROME to an installed Chromium executable.');
const html = require('./build.cjs').build();
const base = pathToFileURL(path.join(__dirname, 'index.html')).href;
async function calculator(page) {
  const element = await page.$('#calculator-frame');
  const child = await element.contentFrame();
  await child.waitForSelector('#result');
  return child;
}
async function key(page, value) { const child = await calculator(page); await child.click(`[data-key="${value}"]`); }
async function result(page) { return (await calculator(page)).$eval('#result', element => element.textContent); }
async function assertResult(page, expected) {
  await (await calculator(page)).waitForFunction(value => document.getElementById('result').textContent === value, {}, expected);
  assert.equal(await result(page), expected);
}
async function click(page, action) { await page.click(`#phone [data-action="${action}"]`); }
async function scene(page, name) { await page.click(`[data-scene="${name}"]`); }
async function checks(page) {
  await page.setViewport({width:1440,height:1100,deviceScaleFactor:1});
  await page.goto(base,{waitUntil:'load'});
  await page.evaluate(() => document.fonts.ready);
  await assertResult(page,'192');
  assert.equal(await page.$eval('#calculator-frame',el=>el.getAttribute('sandbox')),'allow-scripts');
  assert.equal(await page.$eval('#calculator-frame',el=>el.getAttribute('referrerpolicy')),'no-referrer');
  assert.deepEqual(await page.evaluate(() => {
    const ids=[...document.querySelectorAll('[id]')].map(el=>el.id);
    return ids.filter((id,index)=>ids.indexOf(id)!==index);
  }),[],'Unique IDs');
  assert.equal(await (await calculator(page)).$$eval('button',els=>els.every(el=>{const r=el.getBoundingClientRect();return r.width>=48&&r.height>=48;})),true,'Calculator targets >=48px');
  for (const value of ['AC','1','2','*','8','=']) await key(page,value);
  await assertResult(page,'96');
  await (await calculator(page)).evaluate(()=>{window.retainedMarker='same-document';});
  await click(page,'expand');
  assert.equal(await page.$eval('#phone',el=>el.classList.contains('expanded')),true);
  await assertResult(page,'96');
  await key(page,'+'); await key(page,'4'); await key(page,'=');
  await assertResult(page,'100');
  await click(page,'back');
  assert.equal(await page.$eval('#phone',el=>el.classList.contains('expanded')),false);
  assert.equal(await (await calculator(page)).evaluate(()=>window.retainedMarker),'same-document','Expansion must not navigate/recreate the iframe');
  await page.$eval('#thread',el=>{el.scrollTop=el.scrollHeight;});
  await page.$eval('#thread',el=>{el.scrollTop=0;});
  await assertResult(page,'100');
  await click(page,'source');
  assert.equal(await page.$eval('#sheet-title',el=>el.textContent),'Page source');
  assert.equal(await page.$eval('#source-code',el=>el.textContent),await page.$eval('#calculator-frame',el=>el.srcdoc));
  assert.equal(await page.$eval('#phone-main',el=>el.inert),true);
  assert.equal(await page.$eval('#study-controls',el=>el.inert),true);
  assert.equal(await page.evaluate(()=>document.activeElement.dataset.action),'close-sheet');
  await page.keyboard.down('Shift'); await page.keyboard.press('Tab'); await page.keyboard.up('Shift');
  assert.equal(await page.evaluate(()=>document.activeElement.id),'source-code');
  await page.keyboard.press('Escape');
  assert.equal(await page.$eval('#overlay',el=>el.hidden),true);
  assert.equal(await page.evaluate(()=>document.activeElement.dataset.action),'source');
  await assertResult(page,'100');
  await page.click('#theme-toggle');
  await (await calculator(page)).waitForFunction(()=>document.documentElement.dataset.theme==='evening');
  await assertResult(page,'100');
  await click(page,'sandbox-info');
  assert.match(await page.$eval('#sheet-content',el=>el.textContent),/No network access/);
  assert.match(await page.$eval('#sheet-content',el=>el.textContent),/No app or computer access/);
  await page.keyboard.press('Escape');
  await scene(page,'update');
  await assertResult(page,'100');
  assert.equal(await page.$eval('#revision-notice',el=>el.hidden),false);
  await click(page,'review-update');
  assert.equal(await page.$eval('#running-result',el=>el.textContent),'100');
  await page.click('#sheet-content [data-action="close-sheet"]');
  await assertResult(page,'100');
  await click(page,'review-update'); await click(page,'apply-update');
  await assertResult(page,'0');
  assert.equal(await page.$eval('#artifact-file',el=>el.textContent),'calculator.html · v2');
  assert.equal(await (await calculator(page)).evaluate(()=>window.retainedMarker),undefined,'Only the explicit revision replaces the document');
  for (const value of ['4','2','M+','AC','MR']) await key(page,value);
  await assertResult(page,'42');
  await page.type('#draft','Keep my draft <v2> & result.');
  await scene(page,'offline');
  await assertResult(page,'42');
  assert.equal(await page.$eval('#send-button',el=>el.disabled),true);
  assert.equal(await page.$eval('#draft',el=>el.value),'Keep my draft <v2> & result.');
  for (const value of ['+','8','=']) await key(page,value);
  await assertResult(page,'50');
  await click(page,'reconnect');
  await assertResult(page,'50');
  assert.match(await page.$eval('#toast',el=>el.textContent),/No SSH connection was made/);
  await scene(page,'unavailable');
  assert.equal(await page.$eval('#widget-surface',el=>el.hidden),true);
  assert.equal(await page.$eval('#expand-button',el=>el.disabled),true);
  assert.equal(await page.$eval('[data-action="source"]',el=>el.disabled),true);
  await click(page,'request-rewrite');
  assert.match(await page.$eval('#draft',el=>el.value),/complete, self-contained HTML/);
  assert.equal(await page.$eval('#sample-followup',el=>el.hidden),true,'Drafting does not send');
  await click(page,'saved-page');
  await assertResult(page,'192');
  assert.equal(await page.$eval('#saved-notice',el=>el.hidden),false);
  assert.match(await page.$eval('#saved-notice',el=>el.textContent),/not the current revision/);
  await page.click('#reset-sample');
  await assertResult(page,'192');
  await page.type('#draft','Add <b>memory</b> & keep it local.'); await click(page,'send');
  assert.equal(await page.$eval('#sample-followup-text',el=>el.textContent),'Add <b>memory</b> & keep it local.');
  assert.equal(await page.$('#sample-followup-text b'),null,'User input is text, not HTML');
  assert.match(await page.$eval('#toast',el=>el.textContent),/No prompt was sent/);
  await page.click('#reset-sample');
  for (const value of ['AC','7','/','0','=']) await key(page,value);
  await assertResult(page,'Can’t divide by zero');
  await key(page,'3'); await assertResult(page,'3');
  for (const value of ['AC','1','.','5','+','2','.','2','5','=']) await key(page,value);
  await assertResult(page,'3.75');
  await key(page,'AC');
  await (await calculator(page)).focus('.display');
  await page.keyboard.type('12*8'); await page.keyboard.press('Enter');
  await assertResult(page,'96');
  for (const width of [320,360,412,768,1024,1440]) {
    await page.setViewport({width,height:1000,deviceScaleFactor:1});
    await page.goto(base,{waitUntil:'load'});
    assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false,`Study overflow at ${width}`);
  }
  for (const [width,height] of [[320,640],[360,800],[412,915],[640,360]]) {
    await page.setViewport({width,height,deviceScaleFactor:1});
    for (const name of ['inline','update','offline','unavailable']) {
      await page.goto(`${base}?single=1&scene=${name}`,{waitUntil:'load'});
      assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false,`${name}: page overflow at ${width}×${height}`);
      assert.equal(await page.$eval('#thread',el=>el.scrollWidth>el.clientWidth+1),false,`${name}: thread overflow`);
      if (name!=='unavailable') {
        const child=await calculator(page);
        assert.equal(await child.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false,`${name}: calculator overflow`);
        await click(page,'expand'); await key(page,'AC'); await key(page,'8');
        await assertResult(page,'8'); await click(page,'back');
      }
    }
  }
  console.log('Passed: arithmetic, memory, keyboard, retained iframe/state, explicit updates, source, focus trapping, themes, offline/drafts, honest unavailable fallback, escaping and responsive layouts.');
}
async function isolatedPreview(browser) {
  const page = await browser.newPage();
  const requests=[], errors=[];
  page.on('pageerror',error=>errors.push(String(error)));
  await page.setRequestInterception(true);
  page.on('request',request=>{
    if (request.url().startsWith('data:')) return request.continue();
    requests.push(request.url()); return request.abort();
  });
  try {
    await page.setViewport({width:412,height:915,deviceScaleFactor:1});
    await page.emulateMediaFeatures([{name:'prefers-color-scheme',value:'dark'}]);
    await page.setContent(html,{waitUntil:'load'});
    await page.evaluate(()=>document.fonts.ready);
    assert.equal(await page.$$eval('script[src],link[rel="stylesheet"]',els=>els.length),0);
    assert.equal(await page.evaluate(()=>[...document.fonts].every(font=>font.status==='loaded')),true);
    assert.equal(await page.$eval('html',el=>el.dataset.theme),'evening');
    for (const value of ['AC','2','+','3','=']) await key(page,value);
    await assertResult(page,'5');
    await click(page,'expand'); await assertResult(page,'5'); await click(page,'back');
    await scene(page,'offline'); await assertResult(page,'5');
    await click(page,'source');
    assert.match(await page.$eval('#source-code',el=>el.textContent),/Content-Security-Policy/);
    await page.keyboard.press('Escape');
    assert.deepEqual(requests,[],'HTML-only preview must not load sibling files or network resources');
    assert.deepEqual(errors,[],'HTML-only preview JavaScript errors');
    console.log('Passed: Android-style HTML-only preview with all external requests blocked; embedded fonts, calculator, expand, source and offline states.');
  } finally { await page.close(); }
}
(async()=>{
  const browser=await puppeteer.launch({executablePath:chrome,headless:true,args:['--disable-dev-shm-usage']});
  try {
    const page=await browser.newPage();
    const errors=[], network=[], failed=[];
    page.on('pageerror',error=>errors.push(String(error)));
    page.on('request',request=>{if(/^https?:/.test(request.url()))network.push(request.url());});
    page.on('requestfailed',request=>failed.push(request.url()));
    await page.emulateMediaFeatures([{name:'prefers-color-scheme',value:'light'},{name:'prefers-reduced-motion',value:'reduce'}]);
    await checks(page);
    await isolatedPreview(browser);
    assert.deepEqual(errors,[],'JavaScript errors');
    assert.deepEqual(network,[],'Network requests');
    assert.deepEqual(failed,[],'Missing local resources');
    if (!process.argv.includes('--check')) {
      fs.mkdirSync(path.join(__dirname,'screens'),{recursive:true});
      await page.setViewport({width:412,height:915,deviceScaleFactor:1});
      const shots=[['01-inline',''],['02-update','&scene=update'],['03-expanded','&expanded=1'],['04-source','&sheet=source'],['05-offline','&scene=offline'],['06-unavailable','&scene=unavailable'],['07-evening','&theme=evening']];
      for (const [name,query] of shots) {
        await page.goto(`${base}?single=1${query}`,{waitUntil:'load'});
        await page.evaluate(()=>document.fonts.ready);
        await calculator(page);
        await page.screenshot({path:path.join(__dirname,'screens',`${name}.png`)});
        console.log(`screens/${name}.png`);
      }
      await page.setViewport({width:1440,height:1100,deviceScaleFactor:1});
      await page.goto(base,{waitUntil:'load'});
      await page.evaluate(()=>document.fonts.ready);
      await calculator(page);
      await page.screenshot({path:path.join(__dirname,'overview.png'),fullPage:true});
      console.log('overview.png');
      assert.deepEqual(errors,[]); assert.deepEqual(network,[]); assert.deepEqual(failed,[]);
    }
  } finally { await browser.close(); }
})().catch(error=>{console.error(error);process.exitCode=1;});
