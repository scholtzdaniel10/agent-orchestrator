import assert from 'node:assert/strict';
import { mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';

const root = fileURLToPath(new URL('../../', import.meta.url));
const out = join(root, 'clickthrough-out');
const endpoint = process.env.LEGATE_CDP ?? 'http://127.0.0.1:9222';
const errors = [];
const steps = [];
const clients = new Map();
let current = 'startup';
let main;
let inventory;
mkdirSync(out, { recursive: true });

function record(source, detail, target = main?.id) {
  errors.push({ step: current, target, source, detail });
}

function report() {
  const cell = (value) => String(value ?? '').replaceAll('|', '\\|').replace(/\r?\n/g, ' ');
  writeFileSync(join(out, 'errors.json'), JSON.stringify(errors, null, 2) + '\n');
  writeFileSync(join(out, 'report.md'), [
    '# Desktop click-through', '',
    `Source inventory: ${inventory?.summary ?? 'unavailable'}.`,
    'Each pass covers the same controls at the default viewport and at 900 × 700.',
    'Agent jobs, sign-in, installers, native pickers, external file/browser links, merge and discard are excluded.', '',
    '| Step | Result | Notes |', '| --- | --- | --- |',
    ...steps.map((step) => `| ${cell(step.name)} | ${step.result} | ${cell(step.notes)} |`), '',
    `${errors.length} recorded error(s). See errors.json and the numbered screenshots.`, '',
  ].join('\n'));
}

const watchdog = setTimeout(() => {
  record('timeout', 'The whole click-through exceeded 25 minutes');
  steps.push({ name: current, result: 'FAIL', notes: 'Global deadline exceeded' });
  report();
  process.exit(1);
}, 25 * 60_000);

function choices(source, type) {
  const impl = source.match(new RegExp(`impl ${type} \\{([\\s\\S]*?)\\n\\}`))?.[1];
  assert(impl, `Missing ${type} source`);
  const order = [...impl.match(/const ALL:[\s\S]*?= \[([\s\S]*?)\];/)?.[1].matchAll(/Self::(\w+)/g) ?? []];
  const pairs = (method) => new Map([...impl.match(new RegExp(`fn ${method}\\([\\s\\S]*?\\n    \\}`))?.[0]
    .matchAll(/Self::(\w+) => "([^"]+)"/g) ?? []].map((match) => [match[1], match[2]]));
  const ids = pairs('as_str');
  const labels = pairs('label');
  const result = order.map((match) => ({ id: ids.get(match[1]), label: labels.get(match[1]) }));
  assert(result.length && result.every((item) => item.id && item.label), `Incomplete ${type} inventory`);
  assert.equal(new Set(result.map((item) => item.id)).size, result.length);
  return result;
}

function sourceInventory() {
  const read = (path) => readFileSync(join(root, path), 'utf8');
  const app = read('apps/ui/src/app.rs');
  const componentDir = 'apps/ui/src/components';
  const components = Object.fromEntries(readdirSync(join(root, componentDir))
    .filter((file) => file.endsWith('.rs')).map((file) => [file, read(`${componentDir}/${file}`)]));
  const themeSource = read('apps/ui/src/logic/theme.rs').match(/pub const THEMES:[\s\S]*?= &\[([\s\S]*?)\];/)?.[1];
  const themes = [...themeSource?.matchAll(/\("([^"]+)", "([^"]+)"\)/g) ?? []]
    .map((match) => ({ id: match[1], label: match[2] }));
  assert(themes.length && themes.some((theme) => theme.id === 'system'), 'Missing THEMES');
  const sections = [...components['settings_page.rs'].match(/const SECTIONS:[\s\S]*?= &\[([\s\S]*?)\];/)?.[1]
    .matchAll(/\(\s*"([^"]+)",\s*"([^"]+)",/g) ?? []].map((match) => ({ id: match[1], label: match[2] }));
  const how = [...read('apps/ui/src/logic/new_task.rs').match(/pub const HOW_OPTIONS:[\s\S]*?= \[([\s\S]*?)\];/)?.[1]
    .matchAll(/label: "([^"]+)"/g) ?? []].map((match) => match[1]);
  assert(sections.length && how.length, 'Missing settings or New task choices');
  for (const text of ['New window', 'New task', 'Move panel', 'Close inspector', 'DrawerTab::ALL']) {
    assert(app.includes(text), `App control changed: ${text}`);
  }
  for (const [file, text] of [
    ['project_bar.rs', 'New worktree name'], ['routines.rs', 'setup-commands'],
    ['terminal_pane.rs', 'Open in its own window'], ['error_boundary.rs', 'part-error'],
  ]) assert(components[file].includes(text), `Component control changed: ${file}: ${text}`);
  const layout = read('apps/ui/src/logic/layout.rs');
  const tabs = choices(layout, 'DrawerTab');
  const panes = choices(layout, 'PaneLayoutId');
  for (const tab of tabs) assert(app.includes(`"${tab.id}" =>`), `Panel body missing from app.rs: ${tab.id}`);
  return { tabs, panes, themes, sections, how,
    summary: `app.rs, ${Object.keys(components).length} components, ${tabs.length} panels, ${panes.length} pane layouts, ${themes.length} themes, ${sections.length} settings sections` };
}

// Observes rendered failures throughout a step, including errors that later disappear.
const observer = `(() => {
  const visible = (el) => !!el && el.getClientRects().length > 0 &&
    getComputedStyle(el).visibility !== 'hidden';
  const text = (el) => {
    if (!el) return undefined;
    const copy = el.cloneNode(true);
    copy.querySelectorAll('[aria-hidden="true"]').forEach(node => node.remove());
    return copy.textContent.replace(/\\s+/g, ' ').trim();
  };
  window.ct = {
    visible, text,
    all: (selector) => [...document.querySelectorAll(selector)].filter(visible),
    find: (selector, name) => [...document.querySelectorAll(selector)]
      .find(el => visible(el) && (name == null || text(el) === name)),
    errors: [], seen: new Set(),
    scan() {
      const messages = this.all('.part-error, .setup-error, .setup-failed, .error-text, .field-error, .files-error, [role="alert"]')
        .map(text).filter(Boolean);
      const panic = document.body?.innerText.match(/(?:panicked at|unreachable executed|RuntimeError:|wasm trap|This part of the window hit an error)[^\\n]*/gi) || [];
      for (const message of [...messages, ...panic]) {
        if (!this.seen.has(message)) { this.errors.push(message); this.seen.add(message); }
      }
    }
  };
  new MutationObserver(() => ct.scan()).observe(document, { childList: true, subtree: true, attributes: true, characterData: true });
})()`;

class CDP {
  constructor(target) {
    this.id = target.id;
    this.pending = new Map();
    this.events = new Map();
    this.next = 0;
    this.ws = new WebSocket(target.webSocketDebuggerUrl);
    this.ws.addEventListener('message', (event) => {
      try {
        const message = JSON.parse(event.data);
        const waiting = this.events.get(message.method);
        if (waiting) {
          clearTimeout(waiting.timer);
          this.events.delete(message.method);
          waiting.resolve(message.params);
        }
        if (message.id) {
          const pending = this.pending.get(message.id);
          if (!pending) return;
          this.pending.delete(message.id);
          clearTimeout(pending.timer);
          if (message.error) pending.reject(new Error(JSON.stringify(message.error)));
          else pending.resolve(message.result);
        } else if (message.method === 'Runtime.exceptionThrown' ||
          (message.method === 'Runtime.consoleAPICalled' && message.params.type === 'error') ||
          (message.method === 'Log.entryAdded' && message.params.entry.level === 'error')) {
          record(message.method, message.params, this.id);
        }
      } catch (error) { record('CDP message', error.message, this.id); }
    });
    const rejectPending = () => {
      for (const pending of this.pending.values()) {
        clearTimeout(pending.timer);
        pending.reject(new Error('CDP connection closed'));
      }
      this.pending.clear();
      for (const waiting of this.events.values()) {
        clearTimeout(waiting.timer);
        waiting.reject(new Error('CDP connection closed'));
      }
      this.events.clear();
    };
    this.ws.addEventListener('close', rejectPending);
    this.ws.addEventListener('error', rejectPending);
  }

  async connect() {
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => { this.ws.close(); reject(new Error('CDP connect timed out')); }, 8000);
      this.ws.addEventListener('open', () => { clearTimeout(timer); resolve(); }, { once: true });
      this.ws.addEventListener('error', () => { clearTimeout(timer); reject(new Error('CDP connect failed')); }, { once: true });
    });
    for (const domain of ['Runtime', 'Log', 'Page']) await this.call(`${domain}.enable`);
    await this.call('Page.addScriptToEvaluateOnNewDocument', { source: observer });
    await this.evaluate(observer);
    return this;
  }

  call(method, params = {}) {
    return new Promise((resolve, reject) => {
      const id = ++this.next;
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`${method} timed out after 8 seconds`));
      }, 8000);
      this.pending.set(id, { resolve, reject, timer });
      try { this.ws.send(JSON.stringify({ id, method, params })); }
      catch (error) { clearTimeout(timer); this.pending.delete(id); reject(error); }
    });
  }

  event(method) {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.events.delete(method);
        reject(new Error(`Waiting for ${method} timed out`));
      }, 30_000);
      this.events.set(method, { resolve, reject, timer });
    });
  }

  async evaluate(expression) {
    const result = await this.call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
    if (result.exceptionDetails) throw new Error(result.exceptionDetails.exception?.description ?? result.exceptionDetails.text);
    return result.result.value;
  }
}

async function targets() {
  const response = await fetch(`${endpoint}/json`, { signal: AbortSignal.timeout(8000) });
  assert(response.ok, `CDP discovery returned HTTP ${response.status}`);
  return (await response.json()).filter((target) => target.type === 'page' && target.webSocketDebuggerUrl &&
    (target.title === 'Legate' || /^(https?:\/\/tauri\.localhost|tauri:\/\/localhost)/.test(target.url)));
}

async function attach(target) {
  const client = new CDP(target);
  clients.set(target.id, client);
  await client.connect();
  // Reload after enabling error collection so startup failures are observable in every window.
  await client.evaluate('window.ctBeforeReload = true');
  await Promise.all([client.event('Page.loadEventFired'), client.call('Page.reload', { ignoreCache: true })]);
  await waitFor(`!window.ctBeforeReload && document.readyState === 'complete'`, 'reloaded document', client, 30_000);
  return client;
}

async function waitFor(expression, description, client = main, timeout = 15_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    try {
      if (await client.evaluate(`typeof ct !== 'undefined' && !!(${expression})`)) return;
    } catch (error) {
      if (!/Execution context was destroyed|Cannot find context with specified id|Inspected target navigated/i.test(error.message)) throw error;
    }
    await delay(100);
  }
  throw new Error(`Timed out waiting for ${description}`);
}

const find = (selector, text) => `ct.find(${JSON.stringify(selector)}, ${JSON.stringify(text) ?? 'null'})`;
async function click(selector, text, client = main) {
  const element = find(selector, text);
  await waitFor(`${element} && !${element}.disabled`, `clickable ${selector} ${text ?? ''}`, client);
  await client.evaluate(`${element}.click()`);
  await delay(120);
}

async function value(selector, value, client = main) {
  const element = find(selector);
  await waitFor(element, selector, client);
  await client.evaluate(`(() => {
    const el = ${element};
    if (el.tagName === 'SELECT' && ![...el.options].some(option => option.value === ${JSON.stringify(value)}))
      throw new Error('Missing option: ' + ${JSON.stringify(value)});
    const prototype = el.tagName === 'SELECT' ? HTMLSelectElement.prototype :
      el.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(prototype, 'value').set.call(el, ${JSON.stringify(value)});
    el.dispatchEvent(new Event(el.tagName === 'SELECT' ? 'change' : 'input', { bubbles: true }));
  })()`);
  await waitFor(`${element}?.value === ${JSON.stringify(value)}`, `${selector} value`, client);
  await delay(120);
}

async function key(key, code = key, modifiers = 0, client = main) {
  const virtual = { Enter: 13, Escape: 27, ArrowRight: 39, '\\': 220, b: 66 }[key];
  const params = { key, code, modifiers, windowsVirtualKeyCode: virtual };
  await client.call('Input.dispatchKeyEvent', { type: 'keyDown', ...params });
  await client.call('Input.dispatchKeyEvent', { type: 'keyUp', ...params });
  await delay(120);
}

async function inspect(client) {
  const state = await client.evaluate(`(() => {
    ct.scan();
    return { errors: ct.errors.splice(0), width: innerWidth, scrollWidth: document.documentElement.scrollWidth };
  })()`);
  for (const error of state.errors) record('DOM', error, client.id);
  if (state.scrollWidth > state.width) record('horizontal overflow', state, client.id);
}

async function step(name, action, client = main) {
  current = name;
  const before = errors.length;
  let notes = '';
  try { notes = await action() ?? ''; }
  catch (error) { notes = error.stack ?? error.message; record('step', notes, client?.id); }
  if (client) {
    try {
      await delay(180);
      await inspect(client);
      const screenshot = `${String(steps.length + 1).padStart(2, '0')}-${name.replace(/[^a-z0-9]+/gi, '-').toLowerCase()}.png`;
      const { data } = await client.call('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
      writeFileSync(join(out, screenshot), Buffer.from(data, 'base64'));
      notes = `[Screenshot](${screenshot}) ${notes}`;
    } catch (error) { record('capture/inspect', error.message, client.id); notes += ` ${error.message}`; }
  }
  const result = errors.length === before ? 'PASS' : 'FAIL';
  if (result === 'FAIL') notes += ` Errors: ${errors.slice(before).map((error) => error.source).join(', ')}.`;
  steps.push({ name, result, notes });
  console.log(`${result} ${name}`);
  report();
}

async function panel(id) {
  const tab = inventory.tabs.find((tab) => tab.id === id);
  assert(tab, `Uncovered panel: ${id}`);
  await click(`.command-rail button[title=${JSON.stringify(tab.label)}]`);
  await waitFor(`${find('.drawer .inspector-heading h2', tab.label)}`, `${tab.label} inspector`);
  await waitFor(`ct.all('.drawer .drawer-page:not(.is-hidden)').some(el => el.innerText.trim().length)`, `${tab.label} content`);
}

async function settings() {
  if (!await main.evaluate(`!!${find('.settings-page')}`)) await click('.command-settings');
  await waitFor(find('.settings-page'), 'Settings');
}

async function setupCheck(client = main) {
  await waitFor(`ct.all('.setup-row').length >= 5 && ${find('.setup-check button', 'Re-check')}`, 'completed Setup checks', client, 30_000);
  const rows = await client.evaluate(`ct.all('.setup-row').map(row => ({ label: ct.text(row.querySelector('.setup-label')), state: ct.text(row.querySelector('.setup-mark')) }))`);
  for (const label of ['Claude Code CLI', 'Codex CLI', 'Cursor CLI']) {
    assert.equal(rows.find((row) => row.label === label)?.state, 'Not installed', `${label} should be missing on CI`);
  }
  assert.equal(rows.find((row) => row.label === 'Git')?.state, 'Ready');
  assert(await client.evaluate(`${find('.setup-footer button', 'Start using Legate')}?.disabled`), 'Setup must require an installed agent');
}

async function newWindow(prefix, compact) {
  const existing = new Set((await targets()).map((target) => target.id));
  await click('.app-header button[title="New window"]');
  let target;
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline) {
    target = (await targets()).find((item) => !existing.has(item.id));
    if (target) break;
    await delay(100);
  }
  assert(target, 'New window did not create a Legate page target');
  const client = await attach(target);
  if (compact) await client.call('Emulation.setDeviceMetricsOverride', { width: 900, height: 700, deviceScaleFactor: 1, mobile: false });
  await step(`${prefix}-new-window`, async () => {
    await waitFor(find('.app-header'), 'new window workspace', client);
    if (await client.evaluate(`!!${find('.setup-screen')}`)) {
      await setupCheck(client);
      await click('.setup-footer button', 'Skip for now', client);
    }
    await waitFor(find('.project-group.is-open'), 'new window project', client);
  }, client);
  // Keep extra workspace windows monitored until the workflow kills the process tree.
}

async function pass(prefix, compact) {
  await step(`${prefix}-viewport`, async () => {
    if (compact) {
      await settings();
      await value('select[aria-label="Interface size"]', '100');
      await click('.settings-head button', 'Close');
      await main.call('Emulation.setDeviceMetricsOverride', { width: 900, height: 700, deviceScaleFactor: 1, mobile: false });
      await waitFor('innerWidth === 900 && innerHeight === 700', '900 × 700 viewport');
    }
    const size = await main.evaluate('({ width: innerWidth, height: innerHeight })');
    if (compact) assert.deepEqual(size, { width: 900, height: 700 });
    return JSON.stringify(size);
  });
  await step(`${prefix}-setup`, async () => {
    if (!await main.evaluate(`!!${find('.setup-screen')}`)) {
      await panel('usage');
      await click('.usage button', 'Setup');
    }
    await setupCheck();
  });
  await step(`${prefix}-setup-recheck`, async () => {
    await click('.setup-check button', 'Re-check');
    await setupCheck();
  });
  await step(`${prefix}-workspace`, async () => {
    await click('.setup-footer button', 'Skip for now');
    await waitFor(`!${find('.setup-screen')} && ${find('.project-group.is-open .project-name', 'clickthrough-fixture')}`, 'fixture workspace');
    assert.equal(await main.evaluate(`ct.text(ct.find('.project-group.is-open .project-branch'))`), 'main');
    return JSON.stringify(await main.evaluate(`({ width: innerWidth, height: innerHeight })`));
  });

  await step(`${prefix}-new-task`, async () => {
    await click('.app-header .command-new-task');
    await waitFor(find('.new-task-overlay'), 'New task dialog');
    assert(await main.evaluate(`ct.all('.new-task-who button').length > 0 && ct.all('.new-task-who button').every(el => el.disabled)`), 'Missing agents must be unavailable');
  });
  for (const how of inventory.how) await step(`${prefix}-task-${how}`, async () => {
    await main.evaluate(`(() => {
      const button = ct.all('.new-task-how button').find(el => ct.text(el.querySelector('.new-task-choice-name')) === ${JSON.stringify(how)});
      if (!button || button.disabled) throw new Error('Missing task mode');
      button.click();
    })()`);
    await waitFor(`ct.all('.new-task-how button[aria-checked="true"]').some(el => ct.text(el.querySelector('.new-task-choice-name')) === ${JSON.stringify(how)})`, 'selected task mode');
  });
  await step(`${prefix}-task-cancel`, async () => {
    await click('.new-task-actions button', 'Cancel');
    await waitFor(`!${find('.new-task-overlay')}`, 'closed task dialog');
    await click('.sidebar-new-task');
    await waitFor(find('.new-task-overlay'), 'sidebar New task');
    await click('.new-task-actions button', 'Cancel');
  });

  for (const tab of inventory.tabs) await step(`${prefix}-panel-${tab.id}`, () => panel(tab.id));
  await step(`${prefix}-files-preview`, async () => {
    await panel('files');
    await waitFor(find('[role="treeitem"]', 'README.md'), 'fixture file');
    await click('[role="treeitem"]', 'README.md');
    await waitFor(`ct.all('.files-text').some(el => el.textContent.includes('clickthrough fixture content'))`, 'file preview');
    await value('[aria-label="Filter files"]', 'notes');
    await waitFor(find('[role="treeitem"]', 'notes.txt'), 'filtered file');
    await value('[aria-label="Filter files"]', '');
    await click('.files-tools button', 'Changed');
    await click('.files-tools button', 'Changed');
    await click('.files-head button', 'Refresh');
  });
  await step(`${prefix}-history-detail`, async () => {
    await panel('history');
    await click('.history-row');
    await waitFor(find('.history-message', 'Clickthrough fixture'), 'fixture commit');
    await click('.history-file');
    await waitFor(find('.history-detail .diff-block'), 'commit file diff');
    await click('.history-panel .files-head button', 'Refresh');
  });
  await step(`${prefix}-lead-new-chat-menu`, async () => {
    await panel('lead');
    await click('#lead-new-chat');
    await waitFor(find('[role="menu"][aria-label="New chat"]'), 'chat menu');
    await click('.side-menu button', 'Lead chat');
    await waitFor(`!${find('.side-menu')}`, 'closed chat menu');
  });
  for (const [name, scope, open, form] of [
    ['routine', '.routines-list', 'New routine', '.routines-form'],
    ['goal', '.goals > .panel-head', 'New goal', '.goals > .goal-form'],
    ['trigger', '.goal-triggers-head', 'Add trigger', '.goal-triggers .goal-form'],
  ]) {
    await step(`${prefix}-${name}-form`, async () => {
      await panel(name === 'routine' ? 'routines' : 'goals');
      await click(`${scope} button`, open);
      await waitFor(find(form), `${name} form`);
    });
    if (name === 'trigger') {
      await click('.goal-triggers button[aria-label="When"]');
      const labels = await main.evaluate(`ct.all('.select-list [role="option"]').map(ct.text)`);
      assert(labels.length, 'No trigger choices');
      await key('Escape');
      for (const label of labels) await step(`${prefix}-trigger-${label}`, async () => {
        await click('.goal-triggers button[aria-label="When"]');
        await click('.select-list [role="option"]', label);
        await waitFor(find('.goal-triggers button[aria-label="When"]', label), 'selected trigger kind');
      });
    }
    await step(`${prefix}-${name}-cancel`, async () => {
      await click(name === 'routine' ? `${form} button` : `${scope} button`, name === 'routine' ? 'Cancel' : 'Close');
      await waitFor(`!${find(form)}`, `closed ${name} form`);
    });
  }
  await step(`${prefix}-sidebar-navigation`, async () => {
    await click('.sidebar-nav button', 'Goals');
    await waitFor(find('.drawer .inspector-heading h2', 'Goals'), 'sidebar Goals');
    await click('.sidebar-nav button', 'Automations');
    await waitFor(find('.goal-triggers'), 'Automations');
    await value('[aria-label="Search projects, worktrees and chats"]', 'no-such-clickthrough-item');
    await value('[aria-label="Search projects, worktrees and chats"]', '');
    await click('.project-group.is-open .project-group-fold');
    await click('.project-group.is-open .project-group-fold');
    await waitFor(find('[aria-label="New worktree"]'), 'expanded project');
    await click('.sidebar-chats-toggle');
    await waitFor(`!${find('#sidebar-chats')}`, 'folded chats');
    await click('.sidebar-chats-toggle');
    await waitFor(find('#sidebar-chats'), 'expanded chats');
  });
  for (const option of ['Status', 'Nothing', 'Recent activity', 'Name', 'Created', 'Compact', 'Detailed', 'Hide quiet worktrees', 'Hide quiet worktrees']) {
    await step(`${prefix}-sidebar-option-${option}`, async () => {
      await click('[aria-label="View options"]');
      await click('.side-menu button', option);
    });
  }
  await step(`${prefix}-project-switcher`, async () => {
    await click('[aria-label="Add a project"]');
    await waitFor(find('[role="menu"][aria-label="Projects"]'), 'project switcher');
    await click('.side-menu button', 'clickthrough-fixture');
    await main.evaluate(`ct.find('.project-group-head').dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, clientX: 100, clientY: 160 }))`);
    await waitFor(find('[role="menu"][aria-label="Project actions"]'), 'project context menu');
    await key('Escape');
  });

  for (const section of inventory.sections) await step(`${prefix}-settings-${section.id}`, async () => {
    await settings();
    await click('.settings-nav-item', section.label);
    await waitFor(find('.settings-section-title', section.label), 'settings section');
    if (section.id === 'projects') {
      await value('#workflow-goal', 'Run the project checks before review.');
      await click('.workflow-setup summary', 'Preview setup prompt');
      await waitFor(`${find('[aria-label="Generated setup prompt"]')}?.value.includes('Run the project checks before review.')`, 'generated workflow prompt');
    }
  });
  await step(`${prefix}-settings-search`, async () => {
    await value('[aria-label="Search settings"]', 'Look');
    await waitFor(find('select[aria-label="Look"]'), 'matching setting');
    await value('[aria-label="Search settings"]', '');
    await click('.settings-head button', 'Close');
  });
  for (const theme of inventory.themes) await step(`${prefix}-theme-${theme.id}`, async () => {
    await settings();
    await value('select[aria-label="Look"]', theme.id);
    await waitFor(`document.documentElement.getAttribute('data-theme') === ${JSON.stringify(theme.id === 'system' ? null : theme.id)}`, 'applied theme');
    await click('.settings-head button', 'Close');
    await waitFor(`!${find('.settings-page')}`, 'themed workspace');
  });
  await step(`${prefix}-settings-setup`, async () => {
    await settings();
    await click('.settings-nav-item', 'Agents');
    await click('.settings-page button', 'Open setup');
    await setupCheck();
    await click('.setup-footer button', 'Skip for now');
  });

  for (const dock of ['top', 'bottom', 'left', 'right']) await step(`${prefix}-dock-${dock}`, async () => {
    await click('.workspace-layout-menu summary', 'Layout');
    await value('.workspace-layout-controls label:nth-child(1) select', 'files');
    await value('.workspace-layout-controls label:nth-child(2) select', dock);
    await click('.workspace-layout-controls button', 'Move panel');
    await waitFor(find(dock === 'right' ? '.drawer .files-panel' : `.dock-${dock} #drawer-tab-files`), 'moved Files panel');
    if (dock !== 'right') {
      await click(`#drawer-tab-files`);
      await waitFor(find(`.dock-${dock} .files-panel`), 'dock tab content');
      await main.evaluate(`${find(`.layout-handle-${dock}`)}.focus()`);
      await key('Enter');
    }
    await click('.workspace-layout-menu summary', 'Layout');
  });
  await step(`${prefix}-inspector-close`, async () => {
    await click('[aria-label="Close inspector"]');
    await waitFor(`!${find('.drawer')}`, 'closed inspector');
    await panel('usage');
  });
  for (const selector of ['.layout-handle-sidebar', '.layout-handle-drawer']) await step(`${prefix}-resize-${selector}`, async () => {
    const before = await main.evaluate(`Number(${find(selector)}.getAttribute('aria-valuenow'))`);
    await main.evaluate(`${find(selector)}.focus()`);
    await key('ArrowRight');
    const size = await main.evaluate(`(() => {
      const el = ${find(selector)};
      return { value: Number(el.getAttribute('aria-valuenow')), min: Number(el.getAttribute('aria-valuemin')), max: Number(el.getAttribute('aria-valuemax')) };
    })()`);
    assert(size.value >= size.min && size.value <= size.max, 'Resize must respect bounds');
    await key('Enter');
    return `Keyboard resize from ${before} to ${size.value}, then reset.`;
  });
  for (const [name, keyName, code, selector] of [
    ['sidebar', 'Backslash', 'Backslash', '.project-bar'], ['panel', 'b', 'KeyB', '.drawer'],
  ]) await step(`${prefix}-toggle-${name}`, async () => {
    await main.evaluate('document.activeElement?.blur()');
    await key(keyName === 'Backslash' ? '\\' : keyName, code, 2);
    await waitFor(`!${find(selector)}`, `hidden ${name}`);
    await key(keyName === 'Backslash' ? '\\' : keyName, code, 2);
    await waitFor(find(selector), `restored ${name}`);
  });

  await step(`${prefix}-worktree-setup`, async () => {
    await panel('routines');
    await value('#setup-commands', 'powershell.exe -NoLogo -NoProfile -NoExit -Command "Write-Output clickthrough-shell-ready"');
    await click('.routines-setup button', 'Save');
    await waitFor(find('.routines-setup button', 'Save'), 'saved setup command');
    await main.evaluate(`(() => {
      if (window.ctOutput) { window.ctOutput.length = 0; return; }
      window.ctOutput = [];
      const original = window.legateXterm.write;
      window.legateXterm.write = function(id, data) {
        ctOutput.push(String(data));
        if (ctOutput.length > 2000) ctOutput.shift();
        return original.call(this, id, data);
      };
    })()`);
  });
  const worktree = `clickthrough-${prefix}`;
  await step(`${prefix}-worktree-create`, async () => {
    await click('[aria-label="New worktree"]');
    await value('[aria-label="New worktree name"]', worktree);
    assert.equal(await main.evaluate(`${find('select[aria-label="Start"]')}.value`), '', 'Worktree must start no agent');
    await main.evaluate(`${find('[aria-label="New worktree name"]')}.focus()`);
    await key('Enter');
    await waitFor(find('.project-worktree-name', worktree), 'created worktree', main, 30_000);
  });
  await step(`${prefix}-shell-echo`, async () => {
    await waitFor(`window.ctOutput?.join('').includes('clickthrough-shell-ready') && ${find('.terminal-pane:not(.is-hidden) .xterm-helper-textarea')}`, 'interactive setup shell', main, 30_000);
    await main.evaluate(`ctOutput.length = 0; ${find('.terminal-pane:not(.is-hidden) .xterm-helper-textarea')}.focus()`);
    await main.call('Input.insertText', { text: 'echo clickthrough-ok' });
    await key('Enter');
    await waitFor(`/[\\r\\n]clickthrough-ok[\\r\\n]/.test(ctOutput.join('').replace(/\\x1b\\[[0-?]*[ -/]*[@-~]/g, ''))`, 'shell command output');
    return 'Input.insertText + Enter; actual PTY output contains a standalone clickthrough-ok line.';
  });
  for (const pane of inventory.panes) await step(`${prefix}-panes-${pane.id}`, async () => {
    await value('#pane-layout', pane.id);
    await waitFor(`JSON.parse(localStorage.getItem('orch.layout')).paneLayout === ${JSON.stringify(pane.id)}`, 'persisted pane layout');
    await waitFor(find(`.terminal-stage.is-${pane.id === 'auto' ? '1' : pane.id}`), 'rendered terminal layout');
  });
  await step(`${prefix}-terminal-worktree-toggle`, async () => {
    const before = await main.evaluate(`${find('.tab-worktree')}.getAttribute('aria-pressed')`);
    await click('.tab-worktree');
    await waitFor(`${find('.tab-worktree')}.getAttribute('aria-pressed') !== ${JSON.stringify(before)}`, 'toggled terminal Worktree');
    await click('.tab-worktree');
  });
  await step(`${prefix}-worktree-edit`, async () => {
    await value('#pane-layout', 'auto');
    await main.evaluate(`${find('.terminal-pane:not(.is-hidden) .xterm-helper-textarea')}.focus()`);
    await main.call('Input.insertText', { text: "Add-Content README.md 'clickthrough worktree edit'" });
    await key('Enter');
    await waitFor(find('.tab-review'), 'worktree change to review', main, 30_000);
    await panel('changes');
    await waitFor(find('.change-file'), 'worktree changed file');
  });
  await step(`${prefix}-review`, async () => {
    await click('.tab-review');
    await waitFor(find('.review-pane'), 'worktree review');
    await click('.review-actions button', 'Discard');
    await waitFor(find('.review-confirm'), 'discard confirmation');
    await click('.review-confirm button', 'No');
    await click('.review-actions button', 'Back to terminals');
  });
  let popped;
  await step(`${prefix}-terminal-popout`, async () => {
    const previous = new Set((await targets()).map((target) => target.id));
    await click('.terminal-pane [aria-label="Open in its own window"]');
    let target;
    const deadline = Date.now() + 20_000;
    while (Date.now() < deadline) {
      target = (await targets()).find((target) => !previous.has(target.id));
      if (target) break;
      await delay(100);
    }
    assert(target, 'Terminal pop-out did not create a page');
    popped = await attach(target);
    if (compact) await popped.call('Emulation.setDeviceMetricsOverride', { width: 900, height: 700, deviceScaleFactor: 1, mobile: false });
  });
  if (popped) await step(`${prefix}-terminal-window`, () => waitFor(find('.xterm'), 'popped terminal', popped), popped);
  await step(`${prefix}-terminal-return`, async () => {
    await click('.terminal-popped button', 'Bring back');
    if (popped) { clients.delete(popped.id); popped.ws.close(); }
    await waitFor(find('.terminal-pane:not(.is-hidden) .xterm-helper-textarea'), 'returned terminal');
  });
  await step(`${prefix}-shell-close`, async () => {
    await click('.terminal-pane:not(.is-hidden) .terminal-pane-head button[aria-label^="Close "]');
    await waitFor(`!${find('.terminal-pane:not(.is-hidden) .xterm')}`, 'closed shell terminal');
    await click('.project-worktree-row[title="Project folder"]');
  });
  await step(`${prefix}-status-bar`, async () => {
    for (const selector of ['.status-bar button:nth-last-child(3)', '.status-bar button:nth-last-child(2)', '.status-bar-activity']) await click(selector);
    await waitFor(find('.drawer .inspector-heading h2', 'Activity'), 'status Activity');
  });
  const planButtons = await main.evaluate(`ct.all('.status-bar button').slice(0, -3).map(ct.text)`);
  for (const label of planButtons) await step(`${prefix}-status-${label}`, async () => {
    await click('.status-bar button', label);
    if (await main.evaluate(`!!${find('.setup-screen')}`)) {
      await setupCheck();
      await click('.setup-footer button', 'Skip for now');
    } else await waitFor(find('.drawer .inspector-heading h2', 'Usage'), 'status Usage');
  });
  await newWindow(prefix, compact);
}

try {
  assert.equal(process.env.GITHUB_ACTIONS, 'true', 'Run this script only on the GitHub Actions runner');
  assert.equal(process.env.RUNNER_OS, 'Windows', 'The desktop click-through requires Windows');
  inventory = sourceInventory();
  const pages = await targets();
  assert.equal(pages.length, 1, 'Expected exactly one initial Legate page');
  main = await attach(pages[0]);
  await waitFor(find('.app-header'), 'Legate startup', main, 30_000);
  await pass('default', false);
  await pass('900x700', true);
  for (const client of clients.values()) await inspect(client);
} catch (error) {
  await step(`${current}-fatal`, async () => { throw error; });
} finally {
  clearTimeout(watchdog);
  for (const client of clients.values()) client.ws.close();
  report();
  process.exitCode = errors.length || steps.some((step) => step.result === 'FAIL') ? 1 : 0;
  process.exit(process.exitCode);
}
