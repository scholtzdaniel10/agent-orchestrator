#!/usr/bin/env node
/**
 * Capture README screenshots from a `dx build --release --platform web` output.
 *
 * Serves the WASM UI, installs a Tauri `window.__TAURI__` stand-in with demo
 * data (no personal names or paths), and writes PNGs into `docs/`:
 * screenshot, activity, goals, changes, usage, settings.
 *
 * Live ConPTY terminals and native Windows file dialogs are not captured here.
 */
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import puppeteer from "puppeteer-core";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "../..");
const publicDir = path.join(root, "target/dx/legate-ui/release/web/public");
const docsDir = path.join(root, "docs");
const chrome =
  process.env.CHROME_PATH ||
  ["/usr/bin/google-chrome", "/usr/bin/chromium", "/usr/bin/chromium-browser"].find((bin) =>
    fs.existsSync(bin),
  );

if (!chrome) {
  console.error("No Chrome/Chromium binary found.");
  process.exit(1);
}
if (!fs.existsSync(path.join(publicDir, "index.html"))) {
  console.error(`Missing UI build at ${publicDir}. Run: cd apps/ui && dx build --release --platform web`);
  process.exit(1);
}

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".wasm": "application/wasm",
  ".json": "application/json",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".woff2": "font/woff2",
};

function serve(dir) {
  return http.createServer((req, res) => {
    const url = new URL(req.url || "/", "http://127.0.0.1");
    let rel = decodeURIComponent(url.pathname);
    if (rel === "/") rel = "/index.html";
    const file = path.normalize(path.join(dir, rel));
    if (!file.startsWith(dir)) {
      res.writeHead(403).end();
      return;
    }
    fs.readFile(file, (err, data) => {
      if (err) {
        res.writeHead(404).end("not found");
        return;
      }
      res.writeHead(200, { "content-type": MIME[path.extname(file)] || "application/octet-stream" });
      res.end(data);
    });
  });
}

async function waitFor(page, fn, label, timeout = 20000) {
  try {
    await page.waitForFunction(fn, { timeout });
  } catch (err) {
    const body = await page.evaluate(() => document.body?.innerText?.slice(0, 2000) || "");
    const calls = await page.evaluate(() => window.__legateShot?.calls?.join(", ") || "");
    throw new Error(`Timed out waiting for ${label}.\nCalls: ${calls}\nPage:\n${body}\n${err}`);
  }
}

async function clickTab(page, title) {
  const sel = `button.command-destination[title="${title}"]`;
  await page.waitForSelector(sel, { timeout: 10000 });
  await page.click(sel);
}

async function shot(page, name) {
  const dest = path.join(docsDir, name);
  await page.evaluate(() => document.activeElement instanceof HTMLElement && document.activeElement.blur());
  await new Promise((r) => setTimeout(r, 250));
  await page.screenshot({ path: dest, type: "png" });
  console.log(`wrote ${path.relative(root, dest)}`);
}

const server = serve(publicDir);
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const { port } = server.address();
const url = `http://127.0.0.1:${port}/`;
console.log(`serving ${publicDir} at ${url}`);

const browser = await puppeteer.launch({
  executablePath: chrome,
  headless: "new",
  args: [
    "--no-sandbox",
    "--disable-dev-shm-usage",
    "--disable-gpu",
    "--hide-scrollbars",
    "--font-render-hinting=none",
    "--force-color-profile=srgb",
  ],
});

const page = await browser.newPage();
await page.emulateMediaFeatures([{ name: "prefers-color-scheme", value: "dark" }]);
await page.setViewport({ width: 1400, height: 820, deviceScaleFactor: 2 });
page.on("pageerror", (err) => console.error("pageerror", err));
page.on("console", (msg) => {
  if (msg.type() === "error") console.error("console", msg.text());
});

await page.evaluateOnNewDocument(() => {
  const now = Date.now();
  const min = 60_000;
  const hour = 60 * min;
  const day = 24 * hour;

  const PROJECT = "C:\\Users\\demo\\Projects\\legate-demo";
  const DATA = "C:\\Users\\demo\\AppData\\Roaming\\legate";
  const JOB_GREETING = "a1b2c3d4e5f60718";
  const JOB_AUTH = "b7c8d9e0f1a20344";
  const JOB_REVIEW = "c9d0e1f2a3b40566";
  const CHANGE_GREETING = JOB_GREETING.slice(0, 8);
  const CHANGE_AUTH = JOB_AUTH.slice(0, 8);
  const MSG_USER = "lm-01";
  const MSG_LEAD = "lm-02";
  const CHAT = "chat-01";
  const GOAL = "goal-01";
  const GOAL_DONE = "goal-02";

  const GREETING_RS = `pub fn greeting(name: &str) -> String {
    if name.trim().is_empty() {
        "hello, world".into()
    } else {
        format!("hello, {name}")
    }
}

#[cfg(test)]
mod tests {
    use super::greeting;

    #[test]
    fn empty_name_uses_the_default() {
        assert_eq!(greeting(""), "hello, world");
    }
}
`;

  const MAIN_RS = `mod greeting;

fn main() {
    let name = std::env::args().nth(1).unwrap_or_default();
    println!("{}", greeting::greeting(&name));
}
`;

  const README = `# legate-demo

A tiny CLI used in Legate screenshots. Fake project; no personal data.
`;

  const CARGO = `[package]
name = "legate-demo"
version = "0.1.0"
edition = "2024"
`;

  const files = {
    "README.md": README,
    "Cargo.toml": CARGO,
    "src/main.rs": MAIN_RS,
    "src/greeting.rs": GREETING_RS,
    "tests/greeting.rs": `use legate_demo::greeting;\n\n#[test]\nfn named() {\n    assert_eq!(greeting("demo"), "hello, demo");\n}\n`,
  };

  const greetingDiff = `diff --git a/src/greeting.rs b/src/greeting.rs
index 1111111..2222222 100644
--- a/src/greeting.rs
+++ b/src/greeting.rs
@@ -1,6 +1,14 @@
-pub fn greeting() -> &'static str {
-    "hello"
+pub fn greeting(name: &str) -> String {
+    if name.trim().is_empty() {
+        "hello, world".into()
+    } else {
+        format!("hello, {name}")
+    }
 }
`;

  const projectFiles = [
    { path: "README.md", status: "clean" },
    { path: "Cargo.toml", status: "clean" },
    { path: "src/main.rs", status: "modified" },
    { path: "src/greeting.rs", status: "modified" },
    { path: "tests/greeting.rs", status: "added" },
  ];
  const greetingFiles = [
    { path: "README.md", status: "clean" },
    { path: "Cargo.toml", status: "clean" },
    { path: "src/main.rs", status: "modified" },
    { path: "src/greeting.rs", status: "modified" },
    { path: "tests/greeting.rs", status: "added" },
  ];
  const authFiles = [
    { path: "README.md", status: "clean" },
    { path: "src/auth.rs", status: "added" },
    { path: "src/main.rs", status: "modified" },
  ];

  function jobSteps(running) {
    const t0 = now - 4 * min;
    return [
      { id: "s1", tool: "think", title: "Plan the helper and tests", status: "done", startedAt: t0, endedAt: t0 + 8000 },
      { id: "s2", tool: "read", title: "src/main.rs", status: "done", startedAt: t0 + 9000, endedAt: t0 + 11000 },
      {
        id: "s3",
        tool: "edit",
        title: "src/greeting.rs",
        status: "done",
        startedAt: t0 + 12000,
        endedAt: t0 + 22000,
        edit: { path: "src/greeting.rs", added: 18, removed: 6 },
      },
      {
        id: "s4",
        tool: "shell",
        title: "cargo test",
        status: running ? "running" : "done",
        startedAt: t0 + 23000,
        endedAt: running ? null : t0 + 40000,
      },
    ];
  }

  let jobs = [
    {
      id: JOB_AUTH,
      type: "debugging",
      prompt: "Session cookies survive a token refresh.",
      provider: "claude",
      status: "done",
      output: "Added a refresh path in src/auth.rs and covered it with a test.\n\nEdited 2 files +41 −8",
      failedOver: [],
      steps: [
        { id: "a1", tool: "read", title: "src/main.rs", status: "done", startedAt: now - 80 * min, endedAt: now - 79 * min },
        {
          id: "a2",
          tool: "edit",
          title: "src/auth.rs",
          status: "done",
          startedAt: now - 78 * min,
          endedAt: now - 70 * min,
          edit: { path: "src/auth.rs", added: 36, removed: 0 },
        },
        { id: "a3", tool: "shell", title: "cargo test", status: "done", startedAt: now - 69 * min, endedAt: now - 64 * min },
      ],
      model: "claude-sonnet-4",
      reason: "debugging prefers grok; claude had more headroom",
      edit: true,
      access: "edit",
      change: CHANGE_AUTH,
      title: "Keep the session after refresh",
      leadMessage: MSG_LEAD,
      sessionId: "ses-auth",
    },
    {
      id: JOB_GREETING,
      type: "refactor",
      prompt: "Add a greeting helper the CLI can call, and keep cargo test green.",
      provider: "grok",
      status: "running",
      output: "",
      failedOver: [],
      steps: jobSteps(true),
      model: "grok-4.7-high",
      reason: "coding prefers grok",
      edit: true,
      access: "edit",
      change: CHANGE_GREETING,
      group: GOAL,
      title: "Greeting helper",
      leadMessage: MSG_LEAD,
      sessionId: "ses-greet",
      goal: GOAL,
    },
    {
      id: JOB_REVIEW,
      type: "review",
      prompt: "Review the greeting helper for missing tests and API shape.",
      provider: "claude",
      status: "queued",
      output: "",
      failedOver: [],
      model: "claude-sonnet-4",
      access: "read",
      title: "Review greeting helper",
      leadMessage: MSG_LEAD,
      goal: GOAL,
    },
  ];

  const plans = [
    {
      id: "claude",
      available: true,
      used: 0.28,
      atRisk: false,
      busy: false,
      running: 0,
      queued: 1,
      model: "claude-sonnet-4",
      windows: [
        { name: "five_hour", used: 0.28, resetsAt: now + 3.4 * hour },
        { name: "seven_day", used: 0.16, resetsAt: now + 4 * day },
      ],
      usageAt: now - 2 * min,
      version: "2.1.292",
      versionStatus: "tested",
      testedVersion: "2.1.292",
    },
    {
      id: "codex",
      available: true,
      used: 0,
      atRisk: false,
      busy: false,
      running: 0,
      queued: 0,
      windows: [],
      version: "0.167.0-alpha.3.2",
      versionStatus: "tested",
      testedVersion: "0.167.0-alpha.3.2",
    },
    {
      id: "cursor",
      available: true,
      used: 0.41,
      atRisk: false,
      busy: true,
      running: 1,
      queued: 0,
      model: "grok-4.7-high",
      windows: [{ name: "dashboard", used: 0.41, resetsAt: now + 12 * day }],
      usageAt: now - 2 * min,
      version: "2026.10.01-e373342",
      versionStatus: "tested",
      testedVersion: "2026.10.01-e373342",
    },
    {
      id: "grok",
      available: true,
      used: 0.41,
      atRisk: false,
      busy: true,
      running: 1,
      queued: 0,
      model: "grok-4.7-high",
      windows: [{ name: "dashboard", used: 0.41, resetsAt: now + 12 * day }],
      usageAt: now - 2 * min,
      version: "2026.10.01-e373342",
      versionStatus: "tested",
      testedVersion: "2026.10.01-e373342",
    },
    {
      id: "local",
      available: true,
      used: 0,
      atRisk: false,
      busy: false,
      running: 0,
      queued: 0,
      model: "coder-agent",
      windows: [],
      version: "ollama",
      versionStatus: "unknown",
      testedVersion: "",
    },
  ];

  const changes = [
    {
      id: CHANGE_GREETING,
      branch: `orch/${CHANGE_GREETING}`,
      path: `${DATA}\\worktrees\\legate-demo\\${CHANGE_GREETING}`,
      files: [
        { path: "src/greeting.rs", insertions: 18, deletions: 6 },
        { path: "src/main.rs", insertions: 4, deletions: 2 },
        { path: "tests/greeting.rs", insertions: 12, deletions: 0 },
      ],
      insertions: 34,
      deletions: 8,
      name: "greeting",
    },
    {
      id: CHANGE_AUTH,
      branch: `orch/${CHANGE_AUTH}`,
      path: `${DATA}\\worktrees\\legate-demo\\${CHANGE_AUTH}`,
      files: [
        { path: "src/auth.rs", insertions: 36, deletions: 0 },
        { path: "src/main.rs", insertions: 5, deletions: 8 },
      ],
      insertions: 41,
      deletions: 8,
      name: "auth-fix",
    },
  ];

  const prefs = {
    maxParallel: null,
    terminalFontSize: 13,
    notify: true,
    cloneDir: "C:\\Users\\demo\\Projects",
    uiScale: 100,
    usageShows: "left",
    leadMode: "manual",
    cursorModels: "own",
    usageCheck: true,
    offPlans: [],
    localUrl: "http://127.0.0.1:11434",
    localModel: "coder-agent",
    localSmallModel: null,
    localNumCtx: 65536,
  };

  const project = { path: PROJECT, isRepo: true, branch: "main" };
  const projects = [
    {
      path: PROJECT,
      isRepo: true,
      branch: "main",
      active: true,
      changes: 2,
      insertions: 75,
      deletions: 16,
      dirty: 3,
      ahead: 0,
      behind: 0,
      chats: 1,
      running: 1,
    },
    {
      path: "C:\\Users\\demo\\Projects\\sample-cli",
      isRepo: true,
      branch: "main",
      active: false,
      changes: 0,
      insertions: 0,
      deletions: 0,
      dirty: 0,
      ahead: 0,
      behind: 0,
      chats: 0,
      running: 0,
      worktrees: [],
    },
  ];

  const chats = [
    { id: CHAT, title: "Greeting helper", updatedAt: now - 5 * min, active: true, claudeSession: true },
  ];

  const messages = [
    {
      id: MSG_USER,
      role: "user",
      text: "Add a greeting helper the CLI can call, and keep cargo test green.",
      status: "done",
    },
    {
      id: MSG_LEAD,
      role: "lead",
      text: "I'll send a coding job to Grok for the helper and queue a Claude review once the tests pass.",
      provider: "claude",
      model: "claude-sonnet-4",
      status: "done",
    },
  ];

  const goals = [
    {
      id: GOAL,
      project: PROJECT,
      text: "Greeting helper with tests that stay green.",
      check: "cargo test",
      maxRounds: 5,
      quick: false,
      worktree: CHANGE_GREETING,
      status: "waiting",
      rounds: [
        {
          n: 1,
          summary: "Introduce greeting() and call it from main.",
          jobs: [
            {
              type: "refactor",
              access: "edit",
              prompt: "Add a greeting helper the CLI can call, and keep cargo test green.",
            },
          ],
          jobIds: [JOB_GREETING],
          problems: [],
          check: null,
        },
      ],
      proposal: {
        summary: "Review the helper and add a named-argument test.",
        jobs: [
          {
            type: "review",
            access: "read",
            prompt: "Review the greeting helper for missing tests and API shape.",
          },
          {
            type: "boilerplate",
            access: "edit",
            prompt: "Add a test that greeting(\"demo\") returns hello, demo.",
          },
        ],
      },
      question: null,
      notes: [],
      outcome: null,
      createdAt: now - 20 * min,
      updatedAt: now - 1 * min,
    },
    {
      id: GOAL_DONE,
      project: PROJECT,
      text: "Session cookies survive a token refresh.",
      check: "cargo test",
      maxRounds: 3,
      quick: false,
      worktree: CHANGE_AUTH,
      status: "done",
      rounds: [
        {
          n: 1,
          summary: "Refresh the session cookie from a new token.",
          jobs: [{ type: "debugging", access: "edit", prompt: "Session cookies survive a token refresh." }],
          jobIds: [JOB_AUTH],
          problems: [],
          check: { code: 0, output: "test result: ok. 4 passed", at: now - 60 * min },
        },
      ],
      proposal: null,
      question: null,
      notes: [],
      outcome: "cargo test passed after one round.",
      createdAt: now - 2 * hour,
      updatedAt: now - 60 * min,
    },
  ];

  const triggers = [
    {
      id: "trg-01",
      project: PROJECT,
      kind: "daily",
      at: "09:00",
      hours: null,
      text: "Run cargo test and open a goal if anything fails.",
      check: "cargo test",
      maxRounds: 3,
      enabled: true,
      lastFired: now - 20 * hour,
      seen: [],
      primed: true,
      lastError: null,
      createdAt: now - 5 * day,
    },
    {
      id: "trg-02",
      project: PROJECT,
      kind: "ci-failed",
      at: null,
      hours: null,
      text: "Fix the failing CI job on this branch.",
      check: "cargo test",
      maxRounds: 4,
      enabled: true,
      lastFired: null,
      seen: [],
      primed: true,
      lastError: null,
      createdAt: now - 2 * day,
    },
  ];

  const routines = [
    {
      id: "rtn-01",
      name: "Run tests",
      prompt: "Run cargo test and fix anything that fails.",
      type: "debugging",
      worker: "auto",
      access: "full",
      after: null,
    },
    {
      id: "rtn-02",
      name: "Review the diff",
      prompt: "Review uncommitted changes for bugs and missing tests.",
      type: "review",
      worker: "claude",
      access: "read",
      after: { jobType: "refactor" },
    },
  ];

  const commits = [
    {
      hash: "9f3c1a2b4d5e6f708192a3b4c5d6e7f8091a2b3c",
      short: "9f3c1a2",
      parents: ["7a1b2c3d4e5f60718293a4b5c6d7e8f9012a3b4c"],
      author: "Demo",
      time: now - 2 * day,
      subject: "Add the CLI entry point",
      refs: ["HEAD -> main", "origin/main"],
    },
    {
      hash: "7a1b2c3d4e5f60718293a4b5c6d7e8f9012a3b4c",
      short: "7a1b2c3",
      parents: ["5c0d1e2f3a4b50617283a4b5c6d7e8f9012a3b4c"],
      author: "Demo",
      time: now - 5 * day,
      subject: "Initial crate",
      refs: [],
    },
  ];

  const branch = {
    branch: "main",
    upstream: "origin/main",
    base: "main",
    ahead: 0,
    behind: 0,
    uncommitted: [
      { path: "src/main.rs", status: "modified" },
      { path: "src/greeting.rs", status: "modified" },
      { path: "tests/greeting.rs", status: "untracked" },
    ],
    commits: 0,
    insertions: 22,
    deletions: 8,
  };

  const models = {
    claude: [
      { id: "claude-sonnet-4", label: "Sonnet 4" },
      { id: "claude-opus-4", label: "Opus 4" },
    ],
    cursor: [
      { id: "grok-4.7-high", label: "Grok 4.7 High" },
      { id: "composer-1", label: "Composer" },
    ],
    grok: [{ id: "grok-4.7-high", label: "Grok 4.7 High" }],
    codex: [{ id: "gpt-5.1-codex", label: "GPT-5.1 Codex" }],
    local: [{ id: "coder-agent", label: "coder-agent" }],
  };

  const setupStatus = [
    {
      id: "git",
      label: "Git",
      need: "required",
      state: "ready",
      version: "2.47.0",
      install: { command: "winget install --id Git.Git -e", url: "https://git-scm.com" },
      note: "Needed for worktrees.",
    },
    {
      id: "claude",
      label: "Claude Code CLI",
      need: "plan",
      state: "ready",
      version: "2.1.292",
      install: { command: "winget install --id Anthropic.ClaudeCode -e", url: "https://code.claude.com" },
      signIn: "claude auth login",
      note: "Signed in.",
    },
    {
      id: "cursor",
      label: "Cursor CLI",
      need: "plan",
      state: "ready",
      version: "2026.10.01-e373342",
      install: { command: "irm 'https://cursor.com/install?win32=true' | iex", url: "https://cursor.com/docs/cli" },
      signIn: "agent login",
      note: "Signed in.",
    },
    {
      id: "codex",
      label: "Codex CLI",
      need: "plan",
      state: "ready",
      version: "0.167.0-alpha.3.2",
      install: { command: "npm install -g @openai/codex", url: "https://github.com/openai/codex" },
      signIn: "codex login",
      note: "Signed in with a ChatGPT account.",
    },
    {
      id: "gh",
      label: "GitHub CLI",
      need: "optional",
      state: "ready",
      version: "2.62.0",
      install: { command: "winget install --id GitHub.cli -e", url: "https://cli.github.com" },
      signIn: "gh auth login",
      note: "Optional. Used to clone and open pull requests.",
    },
  ];

  function arg(args, key) {
    if (args == null) return undefined;
    if (typeof args.get === "function") {
      if (args.has(key)) return args.get(key);
    }
    if (Object.prototype.hasOwnProperty.call(args, key)) return args[key];
    return undefined;
  }

  function filesFor(whereId) {
    if (whereId === CHANGE_GREETING) return { entries: greetingFiles, truncated: false };
    if (whereId === CHANGE_AUTH) return { entries: authFiles, truncated: false };
    return { entries: projectFiles, truncated: false };
  }

  function textFile(pathName) {
    const text = files[pathName];
    if (!text) return { kind: "missing" };
    return { kind: "text", text, bytes: text.length };
  }

  const listeners = {};
  const calls = [];
  window.__legateShot = {
    calls,
    emit(name, payload) {
      for (const handler of [...(listeners[name] || [])]) {
        handler({ event: name, id: 0, payload });
      }
    },
    bumpGreeting() {
      const job = jobs.find((item) => item.id === JOB_GREETING);
      if (!job) return;
      job.steps = [
        ...jobSteps(false),
        {
          id: "s5",
          tool: "edit",
          title: "tests/greeting.rs",
          status: "done",
          startedAt: now - 20_000,
          endedAt: now - 5_000,
          edit: { path: "tests/greeting.rs", added: 12, removed: 0 },
        },
        {
          id: "s6",
          tool: "shell",
          title: "cargo test",
          status: "running",
          startedAt: now - 4_000,
          endedAt: null,
        },
      ];
      window.__legateShot.emit("jobs:update", job);
    },
  };

  function handle(cmd, args) {
    switch (cmd) {
      case "list_jobs":
        return jobs;
      case "get_job":
        return jobs.find((item) => item.id === arg(args, "id")) || null;
      case "list_plans":
      case "recheck_plans":
        return plans;
      case "list_changes":
        return changes;
      case "get_prefs":
      case "set_prefs":
        return prefs;
      case "list_project_terminals":
      case "list_terminals":
      case "restore_terminals":
      case "popped_out":
      case "popped_panels":
        return [];
      case "list_projects":
        return projects;
      case "get_project":
        return project;
      case "list_lead_chats":
        return chats;
      case "list_lead_messages":
        return messages;
      case "github_available":
        return true;
      case "get_lead_mode":
        return null;
      case "get_lead_plan":
        return null;
      case "get_lead_access":
        return "edit";
      case "list_goals":
        return goals;
      case "list_triggers":
        return triggers;
      case "list_routines":
        return routines;
      case "get_project_setup":
        return ["cargo fetch"];
      case "list_files":
        return filesFor(arg(args, "whereId"));
      case "read_file":
        return textFile(arg(args, "path"));
      case "file_diff":
        return arg(args, "path") === "src/greeting.rs" ? greetingDiff : "";
      case "change_diff":
        return greetingDiff;
      case "window_control":
        return false;
      case "get_app_info":
        return {
          version: "0.2.0-rc.5",
          dataDir: DATA,
          worktreesDir: `${DATA}\\worktrees`,
          rulesMaxParallel: 2,
        };
      case "skills_get":
        return {
          builtIn: [
            { name: "legate-delegate", givenTo: "lead" },
            { name: "legate-review", givenTo: "workers" },
            { name: "legate-worktree", givenTo: "workers" },
            { name: "legate-cli", givenTo: "every agent" },
          ],
          folders: [],
        };
      case "commit_log":
        return { commits, more: false };
      case "commit_detail":
        return {
          hash: commits[0].hash,
          author: "Demo",
          email: "demo@example.com",
          time: commits[0].time,
          message: commits[0].subject,
          files: [{ path: "src/main.rs", added: 12, removed: 0 }],
        };
      case "branch_status":
        return branch;
      case "cli_updates":
        return [
          {
            id: "claude",
            label: "Claude Code CLI",
            installed: "2.1.292",
            latest: "2.1.292",
            newer: false,
            canUpdate: true,
          },
          {
            id: "cursor",
            label: "Cursor CLI",
            installed: "2026.10.01-e373342",
            latest: "2026.10.01-e373342",
            newer: false,
            canUpdate: true,
          },
          {
            id: "codex",
            label: "Codex CLI",
            installed: "0.167.0-alpha.3.2",
            latest: "0.167.0-alpha.3.2",
            newer: false,
            canUpdate: true,
          },
        ];
      case "setup_status":
        return setupStatus;
      case "refresh_usage":
        return { ok: true, error: null };
      case "list_models":
        return models[arg(args, "provider")] || [];
      case "search_history":
      case "github_repos":
        return [];
      default:
        return null;
    }
  }

  try {
    localStorage.setItem("orch.setupSeen", "1");
  } catch (_) {
    /* ignore */
  }

  window.__TAURI__ = {
    core: {
      invoke(cmd, args) {
        calls.push(cmd);
        try {
          return Promise.resolve(handle(cmd, args));
        } catch (err) {
          return Promise.reject(String(err));
        }
      },
    },
    event: {
      listen(name, handler) {
        (listeners[name] = listeners[name] || []).push(handler);
        return Promise.resolve(() => {
          const list = listeners[name] || [];
          const at = list.indexOf(handler);
          if (at >= 0) list.splice(at, 1);
        });
      },
    },
  };
});

await page.goto(url, { waitUntil: "networkidle0", timeout: 60000 });
await waitFor(page, () => !!document.querySelector(".roman-workspace"), "the workspace");
await waitFor(
  page,
  () => !document.querySelector(".setup-screen") && !!document.querySelector(".command-rail"),
  "setup to be skipped",
);
await waitFor(
  page,
  () => (document.querySelector(".app-context-project")?.textContent || "").includes("legate-demo"),
  "the demo project name",
);
await waitFor(
  page,
  () => [...document.querySelectorAll(".tab-label")].some((el) => el.textContent.includes("Greeting helper")),
  "the greeting agent tab",
);

const agentTab = await page.evaluateHandle(() =>
  [...document.querySelectorAll("button.tab")].find((el) => (el.getAttribute("title") || "").includes("Greeting helper")),
);
if (agentTab.asElement()) await agentTab.asElement().click();
await waitFor(page, () => !!document.querySelector(".agent-pane-log, .agent-pane-step"), "the agent pane");

const srcFolder = await page.$('[id="files-dir-src"]');
if (srcFolder) await srcFolder.click();
await waitFor(page, () => !!document.querySelector('[id="files-file-src/greeting.rs"]'), "greeting.rs in the tree");
await page.click('[id="files-file-src/greeting.rs"]');
await waitFor(
  page,
  () =>
    !!document.querySelector(".files-preview-path") &&
    (document.querySelector(".files-preview-path")?.textContent || "").includes("greeting.rs"),
  "the file preview",
);
await new Promise((r) => setTimeout(r, 800));
await shot(page, "screenshot.png");

await clickTab(page, "Activity");
await waitFor(page, () => !!document.querySelector(".flow-body"), "the activity view");
await page.evaluate(() => window.__legateShot.bumpGreeting());
await waitFor(page, () => !!document.querySelector(".flow-event"), "activity events");
await new Promise((r) => setTimeout(r, 400));
await shot(page, "activity.png");

await clickTab(page, "Goals");
await waitFor(
  page,
  () => !!document.querySelector(".goals") && (document.body.innerText || "").includes("needs you"),
  "the goals panel",
);
await new Promise((r) => setTimeout(r, 400));
await shot(page, "goals.png");

await clickTab(page, "Changes");
await waitFor(
  page,
  () => (document.body.innerText || "").includes("greeting") && (document.body.innerText || "").includes("auth-fix"),
  "the changes panel",
);
await new Promise((r) => setTimeout(r, 400));
await shot(page, "changes.png");

await clickTab(page, "Usage");
await waitFor(page, () => !!document.querySelector(".usage-card"), "usage cards");
await new Promise((r) => setTimeout(r, 400));
await shot(page, "usage.png");

await page.click("button.command-settings");
await waitFor(page, () => !!document.querySelector(".settings-page"), "settings");
await waitFor(
  page,
  () => (document.body.innerText || "").includes("Interface size"),
  "settings general",
);
await new Promise((r) => setTimeout(r, 400));
await shot(page, "settings.png");

await browser.close();
server.close();
console.log("done");
