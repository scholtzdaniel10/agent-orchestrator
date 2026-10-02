import { parseRichText, type Inline } from './rich-text'

function renderInlines(inlines: readonly Inline[]): React.JSX.Element[] {
  return inlines.map((inline, index) => {
    if (inline.kind === 'break') return <br key={index} />
    if (inline.kind === 'strong') return <strong key={index}>{inline.text}</strong>
    if (inline.kind === 'code') return <code key={index}>{inline.text}</code>
    return <span key={index}>{inline.text}</span>
  })
}

function RichText({ text }: { text: string }): React.JSX.Element {
  const blocks = parseRichText(text)
  return (
    <div className="rich-text">
      {blocks.map((block, index) =>
        block.kind === 'code' ? (
          <pre key={index}>
            <code>{block.text}</code>
          </pre>
        ) : (
          <p key={index}>{renderInlines(block.inlines)}</p>
        )
      )}
    </div>
  )
}

export default RichText
