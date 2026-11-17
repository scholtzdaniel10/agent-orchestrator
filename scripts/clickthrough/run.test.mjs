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
    const keys = [];
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
        assert.equal(method, 'Input.dispatchKeyEvent');
        if (params.key === 'Enter') {
          if (params.type === 'keyDown') {
            assert(ctOutput.join('').includes('echo clickthrough-ok'), 'Wait for the complete echo');
            entered = true;
          }
        } else {
          assert(focused, 'The terminal must have focus');
          keys.push([params.type, params.key]);
          if (params.type === 'keyDown') {
            assert.equal(params.key, params.text);
            assert.equal(params.unmodifiedText, params.text);
            assert.equal(params.text.length, 1);
            if (keys.length === 1) {
              assert(ctOutput.join('').endsWith('> '), 'Wait for an interactive prompt');
              ctOutput.push(params.text);
            } else pending += params.text;
          }
        }
      },
    };
    await shellCommand('echo clickthrough-ok');
    assert(entered);
    assert(echoPolls >= 2);
    assert.deepEqual(keys, [...'echo clickthrough-ok'].flatMap(key => [['keyDown', key], ['keyUp', key]]));
  `);
});

test('a missing first character fails instead of executing the truncated shell command', async (t) => {
  const app = runner(t);
  await app.run(`
    let clock = 0;
    let entered = false;
    let typed = 0;
    Date = { now: () => clock += 1000 };
    globalThis.window = globalThis;
    globalThis.ctOutput = ['clickthrough-shell-ready\\r\\nPS fixture> '];
    globalThis.ct = { find: () => ({ focus() {} }) };
    main = {
      evaluate: async (expression) => eval(expression),
      call: async (method, params) => {
        assert.equal(method, 'Input.dispatchKeyEvent');
        if (params.key === 'Enter') entered = true;
        else if (params.type === 'keyDown' && typed++ > 0) ctOutput.push(params.text);
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
        assert.equal(method, 'Input.dispatchKeyEvent');
        if (params.type !== 'keyDown') return;
        if (params.key === 'Enter') entered = true;
        else ctOutput.push(params.text, '\\r\\n');
      },
    };
    await shellCommand('echo clickthrough-ok');
    assert(entered);
  `);
});

test('terminal probe records callback IDs and exact UI writes without changing the calls', async (t) => {
  const app = runner(t);
  await app.run(`
    globalThis.window = globalThis;
    const callbacks = new Map();
    const calls = [];
    const result = Promise.resolve('written');
    const core = Object.freeze({
      invoke(command, args) {
        assert.equal(this, core);
        calls.push([command, args]);
        return result;
      },
    });
    const xterm = {
      write(id, data) { assert.equal(this, xterm); return 'output'; },
      onData(id, callback) { assert.equal(this, xterm); callbacks.set(id, callback); },
    };
    window.__TAURI__ = { core };
    window.legateXterm = xterm;
    eval(terminalProbe);
    const invoke = (...args) => window.__TAURI__.core.invoke(...args);
    xterm.onData('old', data => invoke('write_terminal', { id: 'old', data }));
    assert.equal(callbacks.get('old')('x'), result);
    // Reinstalling for the compact pass must not wrap callbacks twice or lose the old IDs.
    assert.equal(xterm.write('old', 'prompt'), 'output');
    eval(terminalProbe);
    assert.equal(ctOutput.length, 0);
    xterm.onData('new', data => invoke('write_terminal', new Map([['id', 'new'], ['data', data]])));
    assert.equal(callbacks.get('new')('e'), result);
    const args = { id: 'new', data: 'cho \\u001b[1;1R\\r雪' };
    assert.equal(invoke('write_terminal', args), result);
    assert.equal(invoke('list_terminals', {}), result);
    assert.equal(calls[2][1], args);
    assert.equal(JSON.stringify(ctWrites), JSON.stringify([
      { id: 'old', data: 'x' }, { id: 'new', data: 'e' }, { id: 'new', data: args.data },
    ]));
    assert.equal(JSON.stringify(ctInput), JSON.stringify([
      { id: 'old', data: 'x' }, { id: 'new', data: 'e' },
    ]));
    assert.equal(xterm.write('new', 'new prompt'), 'output');
    assert.deepEqual(ctOutput, ['new prompt']);
  `);
});

test('failed shell steps report the first UI write even when the PTY echo loses it', async (t) => {
  const app = runner(t);
  await app.run(`
    globalThis.window = globalThis;
    globalThis.document = { hasFocus: () => true, activeElement: { closest: () => ({ id: 'term-host-new' }) } };
    let clock = 0;
    let typed = 0;
    let entered = false;
    let input;
    Date = { now: () => clock += 1000 };
    window.__TAURI__ = { core: { invoke(command, args) {
      if (typed++ > 0) window.legateXterm.write(args.id, args.data);
      return Promise.resolve();
    } } };
    window.legateXterm = { write() {}, onData(id, callback) { input = callback; } };
    eval(terminalProbe);
    legateXterm.onData('new', data => __TAURI__.core.invoke('write_terminal', { id: 'new', data }));
    legateXterm.write('new', 'clickthrough-shell-ready\\r\\nPS fixture> ');
    globalThis.ct = { find: () => ({ focus() {} }) };
    inspect = async () => {};
    main = {
      id: 'main-window',
      evaluate: async expression => eval(expression),
      call: async (method, params) => {
        if (method === 'Page.captureScreenshot') return { data: '' };
        assert.equal(method, 'Input.dispatchKeyEvent');
        if (params.key === 'Enter') entered = true;
        else if (params.type === 'keyDown') input(params.text);
      },
    };
    await step('900x700-shell-echo', () => shellCommand('echo clickthrough-ok'));
    assert.equal(entered, false);
    await step('after-shell', async () => {});
  `);
  const report = app.report();
  assert.match(report, /\| 900x700-shell-echo \| FAIL \|/);
  assert.match(report, /Timed out waiting for complete shell command echo/);
  assert.match(report, /Terminal input probe:/);
  assert.match(report, /"writes":\[\{"id":"new","data":"e"\}/);
  assert.match(report, /"input":\[\{"id":"new","data":"e"\}/);
  assert.match(report, /term-host-new/);
  assert.match(report, /main-window/);
  assert.match(report, /\| after-shell \| PASS \|/);
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
