import { readFileSync } from 'node:fs'

const fixturePath = process.argv[2]
if (!fixturePath) {
  process.stderr.write('missing fixture path\n')
  process.exit(1)
}

process.stdin.on('data', () => {})
process.stdin.on('end', () => {
  const body = readFileSync(fixturePath)
  process.stdout.write(body, () => {
    process.exit(1)
  })
})
