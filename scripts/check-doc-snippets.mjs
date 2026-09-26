// Compiles every ```mcfc block under docs/ as its own project.
// Put <!-- no-check --> on the line before a block to skip it (signatures, fragments).
// Usage: cargo build --release && node scripts/check-doc-snippets.mjs [path-filter]
import { execFileSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, relative } from 'node:path'

const root = join(import.meta.dirname, '..')
const mcfc = join(root, 'target', 'release', process.platform === 'win32' ? 'mcfc.exe' : 'mcfc')
const filter = process.argv[2] ?? ''

const manifest = `namespace = "doc"
[helper]
backend = "mcfd"
[helper.capabilities]
http = { allow_domains = ["api.example.com"] }
file = { root = "./host_data" }
kv = { root = "./host_data/kv" }
db = { path = "./host_data/data.sqlite" }
time = true
rand = true
[helper.agent]
enabled = true
`

function* markdownFiles(dir) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.name === '.vitepress' || entry.name === 'node_modules') continue
    const path = join(dir, entry.name)
    if (entry.isDirectory()) yield* markdownFiles(path)
    else if (entry.name.endsWith('.md')) yield path
  }
}

function* snippets(file) {
  const lines = readFileSync(file, 'utf8').split(/\r?\n/)
  for (let i = 0; i < lines.length; i++) {
    if (!/^```mcfc\b/.test(lines[i])) continue
    const skip = lines[i - 1]?.trim() === '<!-- no-check -->'
    const start = i + 1
    while (i + 1 < lines.length && !lines[i + 1].startsWith('```')) i++
    if (!skip) yield { line: start, code: lines.slice(start, i + 1).join('\n') + '\n' }
    i++
  }
}

const work = mkdtempSync(join(tmpdir(), 'mcfc-snippets-'))
let total = 0
let failed = 0
for (const file of markdownFiles(join(root, 'docs'))) {
  const rel = relative(root, file).replaceAll('\\', '/')
  if (!rel.includes(filter)) continue
  for (const { line, code } of snippets(file)) {
    total++
    const dir = join(work, String(total))
    mkdirSync(join(dir, 'src'), { recursive: true })
    writeFileSync(join(dir, 'mcfc.toml'), manifest)
    writeFileSync(join(dir, 'src', 'main.mcf'), code)
    try {
      execFileSync(mcfc, ['build', dir, '--out', join(dir, 'out')], { stdio: 'pipe' })
    } catch (error) {
      failed++
      console.log(`${rel}:${line}\n${String(error.stderr || error.stdout).trim()}\n`)
    }
  }
}
rmSync(work, { recursive: true, force: true })
console.log(`${total - failed}/${total} snippets compile`)
process.exit(failed ? 1 : 0)
