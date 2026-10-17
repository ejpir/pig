#!/usr/bin/env node
// Bundle the design into the single HTML document Android's page preview receives.
// Uses repository fonts and Node's standard library only. No installs or network.
'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const root = __dirname;
function bundledHTML() {
  let html = fs.readFileSync(path.join(root, 'index.template'), 'utf8');
  const css = fs.readFileSync(path.join(root, 'inline-pages.css'), 'utf8').replace(
    /url\((['"]?)([^)'"\s]+)\1\)/g,
    (_match, _quote, reference) => {
      const file = path.resolve(root, reference);
      assert.ok(file.startsWith(path.resolve(root, '../../../assets/fonts') + path.sep), 'Only repository fonts can be bundled');
      const mime = file.endsWith('.ttf') ? 'font/ttf' : file.endsWith('.otf') ? 'font/otf' : null;
      assert.ok(mime, `Unsupported font ${file}`);
      return `url('data:${mime};base64,${fs.readFileSync(file).toString('base64')}')`;
    },
  );
  const js = fs.readFileSync(path.join(root, 'inline-pages.js'), 'utf8');
  assert.ok(!/<\/style/i.test(css));
  assert.ok(!/<\/script/i.test(js), 'Escape closing script tags inside JavaScript strings');
  for (const [marker, content] of [['<!-- INLINE_CSS -->', `<style>\n${css}\n</style>`], ['<!-- INLINE_JS -->', `<script>\n${js}\n</script>`]]) {
    assert.equal(html.split(marker).length, 2, `Exactly one ${marker} required`);
    html = html.replace(marker, () => content);
  }
  return html;
}
function build() {
  const html = bundledHTML();
  const output = path.join(root, 'index.html');
  if (!fs.existsSync(output) || fs.readFileSync(output, 'utf8') !== html) fs.writeFileSync(output, html);
  return html;
}
if (require.main === module) {
  if (process.argv.includes('--check')) {
    assert.equal(fs.readFileSync(path.join(root, 'index.html'), 'utf8'), bundledHTML(), 'Run node design/android/inline-pages/build.cjs');
    console.log('Self-contained index.html is up to date.');
  } else console.log(`Built self-contained index.html (${Buffer.byteLength(build())} bytes).`);
}
module.exports = { build, bundledHTML };
