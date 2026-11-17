import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { createContext, runInContext } from 'node:vm';
import test from 'node:test';

const source = readFileSync(new URL('./run.mjs', import.meta.url), 'utf8');

// Load the real runner helpers without its CI entry point or any browser connection.
function runner(t) {
  const root = mkdtempSync(join(tmpdir(), 'legate-clickthrough-test-'));
  t.after(() => {
    assert.equal(dirname(root), tmpdir());
    rmSync(root, { recursive: true, force: true });
  });
  const context = createContext({
    assert, mkdirSync, readFileSync, readdirSync, writeFileSync, join, fileURLToPath,
    URL, Buffer, setTimeout, clearTimeout, delay: async () => {},
    process: { env: {} }, console: { log() {} },
  });
  const definitions = source.slice(source.indexOf('const root ='), source.lastIndexOf('\ntry {\n'))
    .replaceAll('import.meta.url', JSON.stringify(pathToFileURL(join(root, 'scripts/clickthrough/run.mjs')).href));
  runInContext(`${definitions}\nclearTimeout(watchdog);`, context);
  return {
    run: (code) => runInContext(`(async () => { ${code} })()`, context),
    report: () => readFileSync(join(root, 'clickthrough-out/report.md'), 'utf8'),
    errors: () => JSON.parse(readFileSync(join(root, 'clickthrough-out/errors.json'), 'utf8')),
  };
}

test('failed window discovery is reported and the next step runs', async (t) => {
  const app = runner(t);
  await app.run(`
    targets = async () => { throw new Error('discovery unavailable'); };
    await newWindow('default', false);
    await step('after-window', async () => {});
  `);
  assert.match(app.report(), /\| default-new-window-open \| FAIL \|/);
  assert.match(app.report(), /\| after-window \| PASS \|/);
  assert.equal(app.errors()[0].step, 'default-new-window-open');
  assert.match(app.errors()[0].detail, /discovery unavailable/);
});

test('a missing new-window target stays a failed step', async (t) => {
  const app = runner(t);
  await app.run(`
    targets = async () => [];
    click = async () => {};
    let clock = 0;
    Date = { now: () => clock += 21_000 };
    await newWindow('default', false);
    await step('after-missing-window', async () => {});
  `);
  assert.match(app.report(), /\| default-new-window-open \| FAIL \|/);
  assert.match(app.report(), /\| after-missing-window \| PASS \|/);
  assert.match(app.errors()[0].detail, /New window did not create a Legate page target/);
});

test('an unexpected default-pass failure still runs the compact pass', async (t) => {
  const app = runner(t);
  await app.run(`
    const calls = [];
    pass = async (prefix, compact) => {
      calls.push([prefix, compact]);
      if (!compact) throw new Error('default pass failed');
      await step(prefix + '-complete', async () => {});
    };
    await runPasses();
    assert.equal(JSON.stringify(calls), JSON.stringify([['default', false], ['900x700', true]]));
  `);
  assert.match(app.report(), /\| default-pass \| FAIL \|/);
  assert.match(app.report(), /\| 900x700-complete \| PASS \|/);
  assert.match(app.errors()[0].detail, /default pass failed/);
});
