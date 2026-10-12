#!/usr/bin/env node
// Bundle the editable sources into the one document Android's page preview receives.
// Uses Node's standard library only; never installs packages or accesses the network.
'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const root = __dirname;
const templatePath = path.join(root, 'index.template');
const outputPath = path.join(root, 'index.html');
const cssMarker = '<!-- INLINE_SWARM_CSS -->';
const jsMarker = '<!-- INLINE_SWARM_JS -->';

function bundledHTML() {
  const template = fs.readFileSync(templatePath, 'utf8');
  for (const marker of [cssMarker, jsMarker]) {
    assert.equal(template.split(marker).length, 2, `Exactly one ${marker} marker required`);
  }
  const css = fs.readFileSync(path.join(root, 'swarm.css'), 'utf8').replace(
    /url\((['"]?)([^)'"\s]+)\1\)/g,
    (_match, _quote, reference) => {
      const file = path.resolve(root, reference);
      const fontDirectory = path.resolve(root, '../../../assets/fonts') + path.sep;
      assert.ok(file.startsWith(fontDirectory), `Only repository fonts may be bundled: ${reference}`);
      const mime = file.endsWith('.ttf') ? 'font/ttf' : file.endsWith('.otf') ? 'font/otf' : null;
      assert.ok(mime, `Unsupported font: ${reference}`);
      return `url('data:${mime};base64,${fs.readFileSync(file).toString('base64')}')`;
    },
  );
  const js = fs.readFileSync(path.join(root, 'swarm.js'), 'utf8');
  assert.ok(!/<\/style/i.test(css), 'CSS must not close its inline element');
  assert.ok(!/<\/script/i.test(js), 'JavaScript must not close its inline element');
  return template
    .replace(cssMarker, () => `<style>\n${css}\n</style>`)
    .replace(jsMarker, () => `<script>\n${js}\n</script>`);
}

function build() {
  const html = bundledHTML();
  if (!fs.existsSync(outputPath) || fs.readFileSync(outputPath, 'utf8') !== html) {
    fs.writeFileSync(outputPath, html);
  }
  return html;
}

if (require.main === module) {
  const html = bundledHTML();
  if (process.argv.includes('--check')) {
    assert.equal(fs.readFileSync(outputPath, 'utf8'), html, 'index.html is stale; run node design/android/swarm/build.cjs');
    console.log('Self-contained index.html is up to date.');
  } else {
    build();
    console.log(`Built self-contained index.html (${Buffer.byteLength(html)} bytes; styles, scripts and fonts embedded).`);
  }
}
module.exports = { build, bundledHTML };
