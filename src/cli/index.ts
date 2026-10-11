import { oneSentence } from './format'
import { start } from './start'

assertSupportedNode()

start(process.argv.slice(2)).then(
  (code) => {
    process.exitCode = code
  },
  (err: unknown) => {
    process.stderr.write(`${oneSentence(err)}\n`)
    process.exitCode = 1
  }
)

function assertSupportedNode(): void {
  const version = process.versions.node
  const [majorText = '0', minorText = '0'] = version.split('.')
  const major = Number(majorText)
  const minor = Number(minorText)
  if (major < 22 || (major === 22 && minor < 5)) {
    process.stderr.write(`legate needs Node 22.5 or newer (this is ${version}).\n`)
    process.exit(2)
  }
}
