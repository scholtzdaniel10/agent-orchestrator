#!/usr/bin/env node

import { existsSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const version = process.versions.node
const [majorText = '0', minorText = '0'] = version.split('.')
const major = Number(majorText)
const minor = Number(minorText)
if (major < 22 || (major === 22 && minor < 5)) {
  console.error(`legate needs Node 22.5 or newer (this is ${version}).`)
  process.exit(2)
}

const built = join(dirname(fileURLToPath(import.meta.url)), '..', 'out', 'main', 'cli.js')
if (!existsSync(built)) {
  console.error('legate is not built. Run pnpm build, then try again.')
  process.exit(1)
}

await import(pathToFileURL(built).href)
