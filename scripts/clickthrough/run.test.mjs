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

test('shell input waits for the prompt and full command echo before Enter', async (t) => {
  const app = runner(t);
  await app.run(`
    let focused = false;
    let prompts = 0;
    let echoPolls = 0;
    let entered = false;
    let pending = '';
    globalThis.window = globalThis;
    globalThis.ctOutput = ['clickthrough-shell-ready\\r\\n'];
    globalThis.ct = { find: () => ({ focus: () => { focused = true; } }) };
    main = {
      evaluate: async (expression) => {
        const result = eval(expression);
        if (!pending && ++prompts === 2) ctOutput.push('PS fixture> ');
        if (pending && ++echoPolls === 2) { ctOutput.push(pending); pending = ''; }
        return result;
      },
      call: async (method, params) => {
        if (method === 'Input.insertText') {
          assert(focused, 'The terminal must have focus');
          assert(ctOutput.join('').endsWith('> '), 'Wait for an interactive prompt');
          ctOutput.push('e');
          pending = params.text.slice(1);
        } else if (params.type === 'keyDown') {
          assert(ctOutput.join('').includes('echo clickthrough-ok'), 'Wait for the complete echo');
          entered = true;
        }
      },
    };
    await shellCommand('echo clickthrough-ok');
    assert(entered);
    assert(echoPolls >= 2);
  `);
});

test('a missing first character fails instead of executing the truncated shell command', async (t) => {
  const app = runner(t);
  await app.run(`
    let clock = 0;
    let entered = false;
    Date = { now: () => clock += 1000 };
    globalThis.window = globalThis;
    globalThis.ctOutput = ['clickthrough-shell-ready\\r\\nPS fixture> '];
    globalThis.ct = { find: () => ({ focus() {} }) };
    main = {
      evaluate: async (expression) => eval(expression),
      call: async (method, params) => {
        if (method === 'Input.insertText') ctOutput.push(params.text.slice(1));
        else entered = true;
      },
    };
    await assert.rejects(shellCommand('echo clickthrough-ok'), /Timed out waiting for complete shell command echo/);
    assert.equal(entered, false);
  `);
});

test('wrapped PowerShell prompts and command echoes still reach Enter', async (t) => {
  const app = runner(t);
  await app.run(`
    let clock = 0;
    let entered = false;
    Date = { now: () => clock += 1000 };
    globalThis.window = globalThis;
    globalThis.ctOutput = ['clickthrough-shell-ready\\r\\nPS fixture\\r\\n> '];
    globalThis.ct = { find: () => ({ focus() {} }) };
    main = {
      evaluate: async (expression) => eval(expression),
      call: async (method, params) => {
        if (method === 'Input.insertText') ctOutput.push('ec\\r\\nho clickthrough-\\r\\nok');
        else if (params.type === 'keyDown') entered = true;
      },
    };
    await shellCommand('echo clickthrough-ok');
    assert(entered);
  `);
});

test('terminal imports finishing after unmount do not create a detached terminal', async () => {
  const bridgeSource = readFileSync(new URL('../../apps/ui/assets/vendor/legate-xterm.js', import.meta.url), 'utf8');
  const modules = 'data:text/javascript,' + encodeURIComponent(`
    export class Terminal { constructor() { throw new Error('Constructed a detached terminal'); } }
    export class FitAddon {}
    export class WebglAddon {}
  `);
  const window = { legateAsset: { xterm: modules, fit: modules, webgl: modules } };
  new Function('window', 'document', 'getComputedStyle', bridgeSource)(
    window, { documentElement: {} }, () => ({ getPropertyValue: () => '' }),
  );
  const host = { isConnected: true };
  const opening = window.legateXterm.open('unmounted', host, 13);
  host.isConnected = false;
  await assert.rejects(opening, /That terminal view has closed/);
});
