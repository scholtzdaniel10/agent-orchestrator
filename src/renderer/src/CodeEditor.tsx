import { useEffect, useRef, useState } from 'react'
import { basicSetup } from 'codemirror'
import { HighlightStyle, LanguageDescription, syntaxHighlighting } from '@codemirror/language'
import { languages } from '@codemirror/language-data'
import { Compartment, EditorState } from '@codemirror/state'
import { EditorView, keymap } from '@codemirror/view'
import { tags } from '@lezer/highlight'
import { DiffBlock } from './FilesPanel'

function errorText(err: unknown): string {
  if (!(err instanceof Error)) return String(err)
  return err.message.replace(/^Error invoking remote method '[^']+': (Error: )?/, '')
}

/** The language for a file name (PHP, SQL, YAML, Go, Rust… over a hundred), loaded on demand. */
function languageFor(path: string): LanguageDescription | null {
  const name = path.slice(path.lastIndexOf('/') + 1)
  return LanguageDescription.matchFilename(languages, name)
}

// Every colour is a theme token, so the editor follows all six looks, Paper included.
const highlight = HighlightStyle.define([
  {
    tag: [tags.keyword, tags.modifier, tags.operatorKeyword, tags.controlKeyword],
    color: 'var(--accent)'
  },
  { tag: [tags.variableName, tags.self], color: 'var(--text)' },
  {
    tag: [tags.definition(tags.variableName), tags.special(tags.variableName)],
    color: 'var(--bot-cursor)'
  },
  { tag: [tags.meta, tags.processingInstruction, tags.documentMeta], color: 'var(--muted)' },
  { tag: [tags.string, tags.special(tags.string), tags.regexp], color: 'var(--diff-add)' },
  { tag: [tags.number, tags.bool, tags.null, tags.atom], color: 'var(--warn)' },
  {
    tag: [tags.comment, tags.lineComment, tags.blockComment],
    color: 'var(--muted)',
    fontStyle: 'italic'
  },
  {
    tag: [tags.function(tags.variableName), tags.function(tags.propertyName)],
    color: 'var(--bot-cursor)'
  },
  { tag: [tags.typeName, tags.className, tags.namespace], color: 'var(--bot-claude)' },
  { tag: [tags.tagName, tags.heading], color: 'var(--accent)', fontWeight: '600' },
  { tag: [tags.attributeName, tags.propertyName], color: 'var(--text)' },
  { tag: tags.invalid, color: 'var(--bad)' }
])

const look = EditorView.theme({
  '&': { height: '100%', color: 'var(--text)', backgroundColor: 'var(--bg)' },
  '.cm-scroller': { fontFamily: 'var(--font-mono)', fontSize: '13px', lineHeight: '1.5' },
  '.cm-content': { caretColor: 'var(--text)' },
  '.cm-cursor, .cm-dropCursor': { borderLeftColor: 'var(--text)' },
  '.cm-gutters': { backgroundColor: 'var(--bg)', color: 'var(--muted)', border: 'none' },
  '.cm-activeLine': { backgroundColor: 'var(--inset)' },
  '.cm-activeLineGutter': { backgroundColor: 'var(--inset)', color: 'var(--text)' },
  '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection': {
    backgroundColor: 'var(--selected)'
  },
  '.cm-selectionMatch': { backgroundColor: 'var(--raised)' },
  '.cm-panels': { backgroundColor: 'var(--inset)', color: 'var(--text)' },
  '.cm-tooltip': { backgroundColor: 'var(--bg)', border: '1px solid var(--border-strong)' }
})

/**
 * A file of the project open for editing, in place of the terminals: syntax colours, search
 * (Ctrl+F), Ctrl+S to save, and the file's uncommitted changes one click away.
 */
function CodeEditor({
  where,
  path,
  onClose
}: {
  where: string | null
  path: string
  onClose: () => void
}): React.JSX.Element {
  const host = useRef<HTMLDivElement>(null)
  const view = useRef<EditorView | null>(null)
  const saved = useRef('')
  const [state, setState] = useState<'loading' | 'ready' | 'unreadable'>('loading')
  const [note, setNote] = useState<string | null>(null)
  const [dirty, setDirty] = useState(false)
  const [saving, setSaving] = useState(false)
  const [mode, setMode] = useState<'file' | 'diff'>('file')
  const [diff, setDiff] = useState<string | null>(null)
  const [closing, setClosing] = useState(false)
  const saveRef = useRef<() => void>(() => {})

  function save(): void {
    const current = view.current
    if (current === null || saving) return
    const text = current.state.doc.toString()
    setSaving(true)
    setNote(null)
    window.api
      .writeFile(where, path, text)
      .then(() => {
        saved.current = text
        setDirty(current.state.doc.toString() !== text)
        setClosing(false)
      })
      .catch((err: unknown) => {
        setNote(errorText(err))
      })
      .finally(() => {
        setSaving(false)
      })
  }

  // The editor's Ctrl+S binding is made once; it calls whichever save is current.
  useEffect(() => {
    saveRef.current = save
  })

  // The parent keys this component by file, so a new file always starts from fresh state.
  useEffect(() => {
    let live = true
    void window.api.readFile(where, path).then(
      (content) => {
        if (!live || host.current === null) return
        if (content.kind !== 'text') {
          setState('unreadable')
          setNote(
            content.kind === 'binary'
              ? 'This file is binary.'
              : content.kind === 'too-large'
                ? 'This file is too large to edit here.'
                : 'This file is missing.'
          )
          return
        }
        saved.current = content.text
        const lang = new Compartment()
        view.current = new EditorView({
          parent: host.current,
          state: EditorState.create({
            doc: content.text,
            extensions: [
              basicSetup,
              keymap.of([
                {
                  key: 'Mod-s',
                  preventDefault: true,
                  run: () => {
                    saveRef.current()
                    return true
                  }
                }
              ]),
              lang.of([]),
              syntaxHighlighting(highlight),
              look,
              EditorView.updateListener.of((update) => {
                if (update.docChanged) setDirty(update.state.doc.toString() !== saved.current)
              })
            ]
          })
        })
        setState('ready')
        view.current.focus()
        void languageFor(path)
          ?.load()
          .then((support) => {
            if (live) view.current?.dispatch({ effects: lang.reconfigure(support) })
          })
      },
      (err: unknown) => {
        if (!live) return
        setState('unreadable')
        setNote(errorText(err))
      }
    )
    return () => {
      live = false
      view.current?.destroy()
      view.current = null
    }
  }, [where, path])

  useEffect(() => {
    if (mode !== 'diff') return
    let live = true
    void window.api.fileDiff(where, path).then(
      (text) => {
        if (live) setDiff(text)
      },
      () => {
        if (live) setDiff('')
      }
    )
    return () => {
      live = false
    }
  }, [mode, where, path])

  return (
    <section className="code-editor" aria-label={`Editing ${path}`}>
      <div className="code-editor-head">
        <span className="code-editor-path mono" title={path}>
          {path}
          {dirty ? <span className="code-editor-dirty" aria-label="unsaved changes" /> : null}
        </span>
        <div className="code-editor-actions">
          <button
            type="button"
            className="btn btn-quiet btn-compact"
            aria-pressed={mode === 'file'}
            onClick={() => {
              setMode('file')
            }}
          >
            File
          </button>
          <button
            type="button"
            className="btn btn-quiet btn-compact"
            aria-pressed={mode === 'diff'}
            title="Changes since the last commit"
            onClick={() => {
              setMode('diff')
            }}
          >
            Changes
          </button>
          <button
            type="button"
            className="btn btn-compact"
            disabled={!dirty || saving || state !== 'ready'}
            title="Save (Ctrl+S)"
            onClick={() => {
              saveRef.current()
            }}
          >
            {saving ? 'Saving…' : 'Save'}
          </button>
          <button
            type="button"
            className={
              closing ? 'btn btn-quiet btn-compact is-danger-label' : 'btn btn-quiet btn-compact'
            }
            onClick={() => {
              if (dirty && !closing) {
                setClosing(true)
                return
              }
              onClose()
            }}
          >
            {closing ? 'Close without saving?' : 'Close'}
          </button>
        </div>
      </div>
      {note !== null ? (
        <p className={state === 'unreadable' ? 'files-status' : 'error-text'} role="alert">
          {note}
        </p>
      ) : null}
      {mode === 'diff' ? (
        <div className="code-editor-diff">
          <DiffBlock text={diff} />
        </div>
      ) : null}
      <div
        ref={host}
        className="code-editor-body"
        hidden={mode !== 'file' || state === 'unreadable'}
      />
    </section>
  )
}

export default CodeEditor
