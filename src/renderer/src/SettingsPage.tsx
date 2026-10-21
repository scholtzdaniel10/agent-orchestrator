import { useEffect, useRef, useState } from 'react'
import BotAvatar from './BotAvatar'
import ModelPicker from './ModelPicker'
import { Star } from './Star'
import { THEMES, type ThemeId } from './theme'
import Select from './Select'

type PlanStatus = Awaited<ReturnType<Window['api']['listPlans']>>[number]
type ProjectEntry = Awaited<ReturnType<Window['api']['listProjects']>>[number]
type Prefs = Awaited<ReturnType<Window['api']['getPrefs']>>
type AppInfo = Awaited<ReturnType<Window['api']['getAppInfo']>>
type JobAccess = Awaited<ReturnType<Window['api']['getLeadAccess']>>
type ProviderId = PlanStatus['id']

function planTitle(id: ProviderId): string {
  if (id === 'claude') return 'Claude'
  if (id === 'grok') return 'Grok'
  return 'Cursor'
}
type SkillsState = Awaited<ReturnType<Window['api']['skillsGet']>>

type SectionId = 'general' | 'agents' | 'skills' | 'projects' | 'terminal' | 'shortcuts' | 'about'

const SECTIONS: { id: SectionId; label: string; blurb: string }[] = [
  { id: 'general', label: 'General', blurb: 'Look, jobs at once and notifications.' },
  { id: 'agents', label: 'Agents', blurb: 'Sign-in, models and the lead.' },
  {
    id: 'skills',
    label: 'Skills',
    blurb: 'What Legate teaches its agents, and skills of your own.'
  },
  { id: 'projects', label: 'Projects', blurb: 'Remembered folders, clones and worktree setup.' },
  { id: 'terminal', label: 'Terminal', blurb: 'Text size of every terminal pane.' },
  { id: 'shortcuts', label: 'Shortcuts', blurb: 'Keys that work across the window.' },
  { id: 'about', label: 'About', blurb: 'Version and where your data lives.' }
]

const SHORTCUTS: [string, string][] = [
  ['Ctrl+,', 'Open or close Settings'],
  ['Ctrl+K', 'Search the sidebar'],
  ['Ctrl+\\', 'Show or hide the sidebar'],
  ['Ctrl+B', 'Show or hide the panel'],
  ['Ctrl+1 … Ctrl+9', 'Panel tabs (Files … Goals), in the order they are shown'],
  ['Ctrl+Shift+N', 'New window'],
  ['Ctrl+Enter', 'Send a message to the lead'],
  ['Shift+F10', 'Actions for the focused worktree']
]

const ACCESS_TEXT: Record<JobAccess, string> = {
  read: 'Read only',
  edit: 'Edit files in a worktree',
  full: 'Edit and run commands in a worktree'
}

function errorText(err: unknown): string {
  if (!(err instanceof Error)) return String(err)
  return err.message.replace(/^Error invoking remote method '[^']+': (Error: )?/, '')
}

function folderName(path: string): string {
  const trimmed = path.replace(/[/\\]+$/, '')
  return trimmed.slice(Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\')) + 1)
}

function planState(plan: PlanStatus): string {
  if (plan.problem === 'not-installed') return 'Not installed'
  if (plan.problem === 'signed-out' || !plan.available) return 'Signed out'
  return 'Ready'
}

function planHelp(plan: PlanStatus): string | undefined {
  const parts: string[] = []
  if (plan.id === 'grok') parts.push('Runs on your Cursor plan.')
  if (plan.version !== undefined && plan.version !== null) {
    parts.push(`CLI ${String(plan.version)}`)
  }
  return parts.length === 0 ? undefined : parts.join(' ')
}

function Row({
  label,
  help,
  children
}: {
  label: string
  help?: string
  children: React.ReactNode
}): React.JSX.Element {
  return (
    <div className="settings-row">
      <div className="settings-row-text">
        <span className="settings-row-label">{label}</span>
        {help !== undefined ? <span className="settings-row-help">{help}</span> : null}
      </div>
      <div className="settings-row-control">{children}</div>
    </div>
  )
}

function Stepper({
  value,
  min,
  max,
  label,
  onChange
}: {
  value: number
  min: number
  max: number
  label: string
  onChange: (value: number) => void
}): React.JSX.Element {
  return (
    <span className="settings-stepper" role="group" aria-label={label}>
      <button
        type="button"
        className="btn btn-quiet btn-compact"
        aria-label={`Fewer: ${label}`}
        disabled={value <= min}
        onClick={() => {
          onChange(value - 1)
        }}
      >
        −
      </button>
      <span className="settings-stepper-value" aria-live="polite">
        {value}
      </span>
      <button
        type="button"
        className="btn btn-quiet btn-compact"
        aria-label={`More: ${label}`}
        disabled={value >= max}
        onClick={() => {
          onChange(value + 1)
        }}
      >
        +
      </button>
    </span>
  )
}

/** One setting: its words (for search) and how to draw it. */
interface Entry {
  section: SectionId
  label: string
  help?: string
  keywords?: string
  render: () => React.JSX.Element
}

function SettingsPage({
  plans,
  projects,
  theme,
  onTheme,
  onRemoveProject,
  onOpenSetup,
  onLeadChanged,
  tabOrderCustom,
  onResetTabs,
  onClose
}: {
  plans: readonly PlanStatus[]
  projects: readonly ProjectEntry[]
  theme: ThemeId
  onTheme: (theme: ThemeId) => void
  onRemoveProject: (path: string) => void
  onOpenSetup: () => void
  /** The lead's plan or worker access changed here; the Lead panel should read them again. */
  onLeadChanged: () => void
  /** The panel's tabs were dragged out of their built-in order. */
  tabOrderCustom: boolean
  onResetTabs: () => void
  onClose: () => void
}): React.JSX.Element {
  const [section, setSection] = useState<SectionId>('general')
  const [query, setQuery] = useState('')
  const [prefs, setPrefs] = useState<Prefs | null>(null)
  const [info, setInfo] = useState<AppInfo | null>(null)
  const [leadPlan, setLeadPlan] = useState<ProviderId | 'auto'>('auto')
  const [access, setAccess] = useState<JobAccess>('read')
  const [setupCommands, setSetupCommands] = useState<string | null>(null)
  const [setupSaved, setSetupSaved] = useState(false)
  const [skills, setSkills] = useState<SkillsState | null>(null)
  const [error, setError] = useState<string | null>(null)
  const searchRef = useRef<HTMLInputElement>(null)
  const active = projects.find((entry) => entry.active)

  useEffect(() => {
    let live = true
    const fail = (err: unknown): void => {
      if (live) setError(errorText(err))
    }
    void window.api.getPrefs().then((value) => {
      if (live) setPrefs(value)
    }, fail)
    void window.api.getAppInfo().then((value) => {
      if (live) setInfo(value)
    }, fail)
    void window.api.getLeadPlan().then((value) => {
      if (live) setLeadPlan(value ?? 'auto')
    }, fail)
    void window.api.getLeadAccess().then((value) => {
      if (live) setAccess(value)
    }, fail)
    void window.api.getProjectSetup().then(
      (list) => {
        if (live) setSetupCommands(list.join('\n'))
      },
      () => {
        if (live) setSetupCommands('')
      }
    )
    void window.api.skillsGet().then((value) => {
      if (live) setSkills(value)
    }, fail)
    const off = window.api.onPrefsUpdate((value) => {
      if (live) setPrefs(value)
    })
    searchRef.current?.focus()
    return () => {
      live = false
      off()
    }
  }, [])

  useEffect(() => {
    const onKey = (event: KeyboardEvent): void => {
      if (event.key !== 'Escape' || event.defaultPrevented) return
      event.preventDefault()
      onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('keydown', onKey)
    }
  }, [onClose])

  function run(task: Promise<unknown>): void {
    setError(null)
    task.catch((err: unknown) => {
      setError(errorText(err))
    })
  }

  function savePrefs(patch: Partial<Prefs>): void {
    run(
      window.api.setPrefs(patch).then((value) => {
        setPrefs(value)
      })
    )
  }

  const entries: Entry[] = [
    {
      section: 'general',
      label: 'Interface size',
      help: 'Makes the whole window, terminals included, smaller or larger. 90% leaves the most room.',
      keywords: 'zoom scale size font text smaller larger crammed',
      render: () => (
        <Select
          className="settings-select"
          value={String(prefs?.uiScale ?? 90)}
          aria-label="Interface size"
          onChange={(picked) => {
            savePrefs({ uiScale: Number(picked) })
          }}
        >
          {[80, 90, 100, 110, 120].map((size) => (
            <option key={size} value={size}>
              {`${String(size)}%`}
            </option>
          ))}
        </Select>
      )
    },
    {
      section: 'general',
      label: 'Usage shows',
      help: 'How the usage meters and the status bar read each plan: what is left of it, or what is used, as Claude’s own usage page shows it.',
      keywords: 'usage percent left used remaining meter allowance plan',
      render: () => (
        <Select
          className="settings-select"
          value={prefs?.usageShows ?? 'left'}
          aria-label="Usage shows"
          onChange={(picked) => {
            savePrefs({ usageShows: picked === 'used' ? 'used' : 'left' })
          }}
        >
          <option value="left">What is left</option>
          <option value="used">What is used</option>
        </Select>
      )
    },
    {
      section: 'general',
      label: 'Cursor models',
      help: 'Which models the Cursor pickers list. Cursor’s own are Auto, Composer and Grok, the plan’s Cursor Models pool. All adds Claude, GPT, Gemini and others, which spend the Other Models quota.',
      keywords: 'cursor models composer grok auto other quota list picker clutter',
      render: () => (
        <Select
          className="settings-select"
          value={prefs?.cursorModels ?? 'own'}
          aria-label="Cursor models"
          onChange={(picked) => {
            savePrefs({ cursorModels: picked === 'all' ? 'all' : 'own' })
          }}
        >
          <option value="own">Cursor’s own</option>
          <option value="all">All models</option>
        </Select>
      )
    },
    {
      section: 'general',
      label: 'Look',
      help: 'Colours for the whole window. System follows light or dark mode.',
      keywords: 'theme colour color dark light appearance',
      render: () => (
        <Select
          className="settings-select"
          value={theme}
          aria-label="Look"
          onChange={(picked) => {
            const next = THEMES.find((item) => item.id === picked)
            if (next !== undefined) onTheme(next.id)
          }}
        >
          {THEMES.map((item) => (
            <option key={item.id} value={item.id}>
              {item.name}
            </option>
          ))}
        </Select>
      )
    },
    {
      section: 'general',
      label: 'Jobs at once per plan',
      help: 'How many jobs one plan runs side by side. More is faster and spends the allowance faster.',
      keywords: 'parallel concurrency queue',
      render: () => {
        const fallback = info?.rulesMaxParallel ?? 3
        const value = prefs?.maxParallel ?? fallback
        return (
          <span className="settings-inline">
            <Stepper
              value={value}
              min={1}
              max={8}
              label="Jobs at once per plan"
              onChange={(next) => {
                savePrefs({ maxParallel: next })
              }}
            />
            {prefs?.maxParallel !== null && prefs !== null ? (
              <button
                type="button"
                className="btn btn-quiet btn-compact"
                onClick={() => {
                  savePrefs({ maxParallel: null })
                }}
              >
                {`Default (${String(fallback)})`}
              </button>
            ) : null}
          </span>
        )
      }
    },
    {
      section: 'general',
      label: 'Notifications',
      help: 'A desktop notification when a job finishes or fails while the window is in the background.',
      keywords: 'notify alert desktop done finished',
      render: () => (
        <label className="settings-check">
          <input
            type="checkbox"
            checked={prefs?.notify ?? true}
            disabled={prefs === null}
            onChange={(event) => {
              savePrefs({ notify: event.target.checked })
            }}
          />
          Tell me when a job ends
        </label>
      )
    },
    ...plans.map((plan): Entry => ({
      section: 'agents',
      label: planTitle(plan.id),
      help: planHelp(plan),
      keywords: 'agent plan model sign in login install cli',
      render: () => (
        <div className="settings-plan">
          <span className="settings-plan-head">
            <BotAvatar bot={plan.id} state={plan.available ? 'idle' : 'error'} size={18} />
            <span className={plan.available ? 'settings-ok' : 'settings-bad'}>
              {planState(plan)}
            </span>
            <button
              type="button"
              className="btn btn-quiet btn-compact"
              onClick={() => {
                run(window.api.recheckPlans())
              }}
            >
              Check again
            </button>
            {!plan.available ? (
              <button type="button" className="btn btn-compact" onClick={onOpenSetup}>
                Open Setup
              </button>
            ) : null}
          </span>
          <ModelPicker plan={plan} disabled={!plan.available} />
        </div>
      )
    })),
    {
      section: 'general',
      label: 'Panel tabs',
      help: 'Drag a tab above or below the terminals, under the projects, or out of the window into its own. Right-click a tab to move it too. Reset puts every tab back on the right.',
      keywords: 'tabs order drag reorder panel activity flow',
      render: () => (
        <span className="settings-inline">
          <span className="settings-muted">{tabOrderCustom ? 'Your order' : 'Built-in order'}</span>
          {tabOrderCustom ? (
            <button type="button" className="btn btn-quiet btn-compact" onClick={onResetTabs}>
              Reset
            </button>
          ) : null}
        </span>
      )
    },
    {
      section: 'agents',
      label: 'Lead runs on',
      help: 'The plan that plans and hands out work. Auto picks the one with the most allowance left.',
      keywords: 'lead plan auto delegate',
      render: () => (
        <Select
          className="settings-select"
          value={leadPlan}
          aria-label="Lead runs on"
          onChange={(picked) => {
            const value = picked
            const next =
              value === 'claude' || value === 'cursor' || value === 'grok' ? value : 'auto'
            setLeadPlan(next)
            run(window.api.setLeadPlan(next === 'auto' ? null : next).then(onLeadChanged))
          }}
        >
          <option value="auto">Auto</option>
          {plans.map((plan) => (
            <option key={plan.id} value={plan.id}>
              {planTitle(plan.id)}
            </option>
          ))}
        </Select>
      )
    },
    {
      section: 'agents',
      label: 'Most the lead may hand out',
      help: 'The highest access a job from the lead can get. Jobs you start yourself are not limited.',
      keywords: 'access permission read edit full commands',
      render: () => (
        <Select
          className="settings-select"
          value={access}
          aria-label="Most the lead may hand out"
          onChange={(picked) => {
            const value = picked
            if (value !== 'read' && value !== 'edit' && value !== 'full') return
            setAccess(value)
            run(window.api.setLeadAccess(value).then(onLeadChanged))
          }}
        >
          {(['read', 'edit', 'full'] as const).map((value) => (
            <option key={value} value={value}>
              {ACCESS_TEXT[value]}
            </option>
          ))}
        </Select>
      )
    },
    {
      section: 'agents',
      label: 'Your own plans only',
      help: 'Each plan is the CLI signed in on this computer. Legate never reads, stores or sends your logins.',
      keywords: 'privacy account login security',
      render: () => (
        <button type="button" className="btn btn-quiet btn-compact" onClick={onOpenSetup}>
          Open Setup
        </button>
      )
    },
    {
      section: 'skills',
      label: 'Your skills',
      help: 'A folder with a SKILL.md, or a folder of skill folders. Loaded only into agents Legate starts, never into your other Claude or Cursor sessions.',
      keywords: 'skill skills plugin folder add custom prompt',
      render: () => (
        <span className="settings-skills">
          {skills !== null && skills.folders.length > 0 ? (
            <ul className="settings-projects">
              {skills.folders.map((entry) => (
                <li key={entry.folder} className="settings-project">
                  <span className="settings-project-text">
                    <span className="settings-project-name">
                      {entry.problem === null
                        ? entry.skills.map((skill) => skill.name).join(', ')
                        : folderName(entry.folder)}
                    </span>
                    <span className="settings-path">{entry.folder}</span>
                    {entry.problem !== null ? (
                      <span className="settings-skill-problem">{entry.problem}</span>
                    ) : null}
                  </span>
                  <button
                    type="button"
                    className="btn btn-quiet btn-compact"
                    aria-label={`Remove ${entry.folder}`}
                    onClick={() => {
                      run(window.api.skillsRemove(entry.folder).then(setSkills))
                    }}
                  >
                    Remove
                  </button>
                </li>
              ))}
            </ul>
          ) : (
            <span className="settings-muted">None yet.</span>
          )}
          <span className="settings-inline">
            <button
              type="button"
              className="btn btn-quiet btn-compact"
              onClick={() => {
                run(window.api.skillsAdd().then(setSkills))
              }}
            >
              Add folder…
            </button>
            {skills !== null && skills.folders.length > 0 ? (
              <button
                type="button"
                className="btn btn-quiet btn-compact"
                title="Copy your skills again after editing one"
                onClick={() => {
                  run(window.api.skillsReload().then(setSkills))
                }}
              >
                Reload
              </button>
            ) : null}
          </span>
        </span>
      )
    },
    {
      section: 'skills',
      label: "Legate's skills",
      help: 'Built in. Workers get the worktree rules and a read-only legate CLI, the lead gets how to delegate, and both get how to review a change.',
      keywords: 'skill skills built in legate worktree review delegate cli',
      render: () =>
        skills === null ? (
          <span className="settings-muted">…</span>
        ) : skills.builtIn.length === 0 ? (
          <span className="settings-skill-problem">Not found in this build.</span>
        ) : (
          <ul className="settings-projects">
            {skills.builtIn.map((skill) => (
              <li key={`${skill.givenTo}-${skill.name}`} className="settings-project">
                <span className="settings-project-name">{skill.name}</span>
                <span className="settings-tag">
                  {skill.givenTo === 'workers' ? 'workers & terminals' : skill.givenTo}
                </span>
              </li>
            ))}
          </ul>
        )
    },
    {
      section: 'projects',
      label: 'Clone folder',
      help: 'Where repositories cloned from GitHub go.',
      keywords: 'github clone folder directory',
      render: () => (
        <span className="settings-inline">
          <span className={prefs?.cloneDir != null ? 'settings-path' : 'settings-muted'}>
            {prefs?.cloneDir ?? 'Ask every time'}
          </span>
          <button
            type="button"
            className="btn btn-quiet btn-compact"
            onClick={() => {
              run(
                window.api.chooseCloneDir().then((value) => {
                  if (value !== null) setPrefs(value)
                })
              )
            }}
          >
            Choose…
          </button>
          {prefs?.cloneDir != null ? (
            <button
              type="button"
              className="btn btn-quiet btn-compact"
              onClick={() => {
                savePrefs({ cloneDir: null })
              }}
            >
              Ask every time
            </button>
          ) : null}
        </span>
      )
    },
    {
      section: 'projects',
      label: 'Projects',
      help: 'Folders in the sidebar. Removing one only takes it off the list.',
      keywords: 'folder repository remove list',
      render: () => (
        <ul className="settings-projects">
          {projects.map((entry) => (
            <li key={entry.path} className="settings-project">
              <span className="settings-project-text">
                <span className="settings-project-name">
                  {folderName(entry.path)}
                  {entry.active ? <span className="settings-tag">open</span> : null}
                </span>
                <span className="settings-path">{entry.path}</span>
              </span>
              <span className="settings-inline">
                <button
                  type="button"
                  className="btn btn-quiet btn-compact"
                  onClick={() => {
                    run(window.api.revealProject(entry.path))
                  }}
                >
                  Open folder
                </button>
                <button
                  type="button"
                  className="btn btn-quiet btn-compact"
                  disabled={entry.active}
                  title={entry.active ? 'Switch to another project first' : undefined}
                  onClick={() => {
                    onRemoveProject(entry.path)
                  }}
                >
                  Remove
                </button>
              </span>
            </li>
          ))}
        </ul>
      )
    },
    {
      section: 'projects',
      label: 'Worktree setup',
      help: `Commands run in each new worktree of ${active === undefined ? 'the open project' : folderName(active.path)}, one per line, e.g. pnpm install.`,
      keywords: 'setup script install worktree commands',
      render: () => (
        <span className="settings-setup">
          <textarea
            className="settings-textarea"
            rows={3}
            value={setupCommands ?? ''}
            disabled={setupCommands === null || active?.isRepo !== true}
            aria-label="Worktree setup commands"
            spellCheck={false}
            onChange={(event) => {
              setSetupCommands(event.target.value)
              setSetupSaved(false)
            }}
          />
          <span className="settings-inline">
            <button
              type="button"
              className="btn btn-compact"
              disabled={setupCommands === null || active?.isRepo !== true}
              onClick={() => {
                const list = (setupCommands ?? '')
                  .split('\n')
                  .map((line) => line.trim())
                  .filter((line) => line !== '')
                run(
                  window.api.setProjectSetup(list).then(() => {
                    setSetupSaved(true)
                  })
                )
              }}
            >
              Save
            </button>
            {setupSaved ? <span className="settings-muted">Saved</span> : null}
          </span>
        </span>
      )
    },
    {
      section: 'projects',
      label: 'Worktrees folder',
      help: 'Editing jobs and new worktrees work in copies here until you merge or discard them.',
      keywords: 'worktree folder path',
      render: () => <span className="settings-path">{info?.worktreesDir ?? '…'}</span>
    },
    {
      section: 'terminal',
      label: 'Text size',
      help: 'Applies to every open terminal straight away.',
      keywords: 'font size zoom terminal',
      render: () => (
        <Stepper
          value={prefs?.terminalFontSize ?? 13}
          min={10}
          max={22}
          label="Terminal text size"
          onChange={(next) => {
            savePrefs({ terminalFontSize: next })
          }}
        />
      )
    },
    ...SHORTCUTS.map(([keys, what]): Entry => ({
      section: 'shortcuts',
      label: what,
      keywords: `shortcut key ${keys}`,
      render: () => <kbd className="settings-kbd">{keys}</kbd>
    })),
    {
      section: 'about',
      label: 'Version',
      keywords: 'version update release',
      render: () => <span className="settings-path">{info?.version ?? '…'}</span>
    },
    {
      section: 'about',
      label: 'Data folder',
      help: 'Settings and saved chats. Nothing here leaves this computer.',
      keywords: 'data folder storage chats database',
      render: () => (
        <span className="settings-inline">
          <span className="settings-path">{info?.dataDir ?? '…'}</span>
          <button
            type="button"
            className="btn btn-quiet btn-compact"
            onClick={() => {
              run(window.api.openDataDir())
            }}
          >
            Open folder
          </button>
        </span>
      )
    }
  ]

  const needle = query.trim().toLowerCase()
  const matches =
    needle === ''
      ? entries.filter((entry) => entry.section === section)
      : entries.filter((entry) =>
          `${entry.label} ${entry.help ?? ''} ${entry.keywords ?? ''} ${entry.section}`
            .toLowerCase()
            .includes(needle)
        )
  const shownSections = SECTIONS.filter((item) =>
    matches.some((entry) => entry.section === item.id)
  )

  return (
    <div className="settings-page" role="dialog" aria-modal="true" aria-labelledby="settings-title">
      <div className="settings-head">
        <div className="panel-title">
          <Star size={12} />
          <h2 id="settings-title">Settings</h2>
        </div>
        <button type="button" className="btn btn-quiet btn-compact" onClick={onClose}>
          Close
        </button>
      </div>
      <div className="settings-body">
        <nav className="settings-nav" aria-label="Settings sections">
          <input
            ref={searchRef}
            className="project-search settings-search"
            type="search"
            value={query}
            placeholder="Search settings"
            aria-label="Search settings"
            onChange={(event) => {
              setQuery(event.target.value)
            }}
            onKeyDown={(event) => {
              if (event.key === 'Escape' && query !== '') {
                event.preventDefault()
                setQuery('')
              }
            }}
          />
          <ul className="settings-nav-list">
            {SECTIONS.map((item) => {
              const selected = needle === '' && item.id === section
              return (
                <li key={item.id}>
                  <button
                    type="button"
                    className={`settings-nav-item${selected ? ' is-selected' : ''}`}
                    aria-current={selected ? 'page' : undefined}
                    onClick={() => {
                      setQuery('')
                      setSection(item.id)
                    }}
                  >
                    {item.label}
                  </button>
                </li>
              )
            })}
          </ul>
        </nav>
        <div className="settings-content">
          {error !== null ? (
            <p className="error-text" role="alert">
              {error}
            </p>
          ) : null}
          {matches.length === 0 ? <p className="settings-muted">No settings match.</p> : null}
          {shownSections.map((item) => (
            <section key={item.id} className="settings-section" aria-label={item.label}>
              <h3 className="settings-section-title">{item.label}</h3>
              <p className="settings-section-blurb">{item.blurb}</p>
              {matches
                .filter((entry) => entry.section === item.id)
                .map((entry, index) => (
                  <Row
                    key={`${entry.label}-${String(index)}`}
                    label={entry.label}
                    help={entry.help}
                  >
                    {entry.render()}
                  </Row>
                ))}
            </section>
          ))}
        </div>
      </div>
    </div>
  )
}

export default SettingsPage
