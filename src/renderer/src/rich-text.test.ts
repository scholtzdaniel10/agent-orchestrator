import { expect, test } from 'vitest'
import { parseRichText } from './rich-text'

test('plain text is one paragraph', () => {
  expect(parseRichText('hello')).toEqual([
    { kind: 'paragraph', inlines: [{ kind: 'text', text: 'hello' }] }
  ])
})

test('bold becomes a strong inline', () => {
  expect(parseRichText('say **hi** there')).toEqual([
    {
      kind: 'paragraph',
      inlines: [
        { kind: 'text', text: 'say ' },
        { kind: 'strong', text: 'hi' },
        { kind: 'text', text: ' there' }
      ]
    }
  ])
})

test('inline code becomes a code inline', () => {
  expect(parseRichText('use `npm` here')).toEqual([
    {
      kind: 'paragraph',
      inlines: [
        { kind: 'text', text: 'use ' },
        { kind: 'code', text: 'npm' },
        { kind: 'text', text: ' here' }
      ]
    }
  ])
})

test('a fenced block with a language keeps only the body', () => {
  expect(parseRichText('```ts\nconst n = 1\n```')).toEqual([{ kind: 'code', text: 'const n = 1' }])
})

test('a fenced block without a language keeps only the body', () => {
  expect(parseRichText('```\nconst n = 1\n```')).toEqual([{ kind: 'code', text: 'const n = 1' }])
})

test('an unclosed fence is code to the end', () => {
  expect(parseRichText('```js\nconst n = 1\nstill')).toEqual([
    { kind: 'code', text: 'const n = 1\nstill' }
  ])
})

test('an unclosed bold marker stays literal', () => {
  expect(parseRichText('**nope')).toEqual([
    { kind: 'paragraph', inlines: [{ kind: 'text', text: '**nope' }] }
  ])
})

test('script markup stays plain text', () => {
  expect(parseRichText('see <script>alert(1)</script> now')).toEqual([
    {
      kind: 'paragraph',
      inlines: [{ kind: 'text', text: 'see <script>alert(1)</script> now' }]
    }
  ])
})

test('an empty string has no blocks', () => {
  expect(parseRichText('')).toEqual([])
})

test('blank lines split paragraphs and single newlines stay breaks', () => {
  expect(parseRichText('one\ntwo\n\nthree')).toEqual([
    {
      kind: 'paragraph',
      inlines: [{ kind: 'text', text: 'one' }, { kind: 'break' }, { kind: 'text', text: 'two' }]
    },
    { kind: 'paragraph', inlines: [{ kind: 'text', text: 'three' }] }
  ])
})

test('asterisks inside inline code stay literal', () => {
  expect(parseRichText('`**not bold**`')).toEqual([
    { kind: 'paragraph', inlines: [{ kind: 'code', text: '**not bold**' }] }
  ])
})
