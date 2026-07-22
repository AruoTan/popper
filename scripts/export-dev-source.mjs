import {
  cpSync,
  existsSync,
  mkdirSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync
} from 'node:fs'
import { basename, dirname, join, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDirectory = dirname(fileURLToPath(import.meta.url))
const root = resolve(scriptDirectory, '..')
const packageJson = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'))
const version = packageJson.version

if (typeof version !== 'string' || !/^\d+\.\d+\.\d+$/u.test(version)) {
  throw new Error(`package.json version is invalid: ${String(version)}`)
}

const exportRoot = join(root, 'portable-dev-sources')
const bundleName = `TextLens-${version}-dev-source`
const outputDirectory = join(exportRoot, bundleName)

const includeEntries = [
  '.env.example',
  '.gitignore',
  'LICENSE.selection-hook',
  'README.md',
  'THIRD_PARTY_NOTICES.md',
  'DEVELOPMENT_HISTORY.md',
  'apps',
  'assets',
  'package.json',
  'pnpm-lock.yaml',
  'pnpm-workspace.yaml',
  'scripts',
  'src',
  'src-tauri',
  'tsconfig.json',
  'tsconfig.node.json',
  'tsconfig.web.json',
  'vite.config.ts',
  'vitest.config.ts',
  'vitest.setup.ts'
]

function normalize(path) {
  return path.replace(/\\/gu, '/')
}

function shouldCopy(sourcePath) {
  const relativePath = normalize(relative(root, sourcePath))
  if (!relativePath || relativePath === '') return true

  const name = basename(sourcePath)
  if (
    name === '.DS_Store'
    || name === '.git'
    || name === '.pnpm-store'
    || name === '.tmp-textlens-start-diagnostic'
    || name === 'artifacts'
    || name === 'coverage'
    || name === 'dist'
    || name === 'node_modules'
    || name === 'out'
    || name === 'portable-dev-sources'
    || name === 'release'
    || name === 'target'
    || name === '.vite'
    || /^archive-pre-/u.test(name)
    || name.endsWith('.tsbuildinfo')
    || name.endsWith('.log')
  ) {
    return false
  }

  return true
}

function copyEntry(relativeEntry) {
  const source = join(root, relativeEntry)
  const destination = join(outputDirectory, relativeEntry)
  if (!existsSync(source)) {
    throw new Error(`Required entry is missing: ${relativeEntry}`)
  }
  cpSync(source, destination, {
    recursive: true,
    force: true,
    filter: (sourcePath) => shouldCopy(sourcePath)
  })
}

function requireExportedPath(relativeEntry, kind) {
  const path = join(outputDirectory, relativeEntry)
  if (!existsSync(path)) {
    throw new Error(`Export validation failed: missing ${kind} ${relativeEntry}`)
  }
  if (kind === 'file' && !statSync(path).isFile()) {
    throw new Error(`Export validation failed: ${relativeEntry} is not a file`)
  }
  if (kind === 'directory' && !statSync(path).isDirectory()) {
    throw new Error(`Export validation failed: ${relativeEntry} is not a directory`)
  }
}

rmSync(outputDirectory, { recursive: true, force: true })
mkdirSync(outputDirectory, { recursive: true })

for (const entry of includeEntries) {
  copyEntry(entry)
}

const manifest = [
  '# 可迁移开发源码包',
  '',
  `这是 TextLens ${version} 的干净源码工作区，只包含继续开发所需的源代码、配置和文档，不包含已编译应用或安装包。`,
  '',
  '## 包含',
  '',
  '- 共享前端源码：`src/`',
  '- 平台代码与资源：`apps/`、`assets/`',
  '- Rust/Tauri 后端：`src-tauri/`（不含 target 编译产物）',
  '- 构建脚本、锁文件、配置、README、开发历史、许可证与第三方说明',
  '',
  '## 不包含',
  '',
  '- 已编译应用、安装包和 `release/` 发布快照',
  '- 前端构建产物 `dist/`',
  '- 依赖目录 `node_modules/`',
  '- Rust 编译目录 `src-tauri/target/`',
  '- 临时诊断、归档目录、缓存和 `.tsbuildinfo` 文件',
  '',
  '## 在新机器上继续开发',
  '',
  '环境要求：',
  '',
  '- Node.js 22+',
  '- pnpm 11+',
  '- Rust 稳定版',
  '- Windows：MSVC 工具链、WebView2',
  '- macOS：Xcode Command Line Tools（Apple Silicon 优先）',
  '',
  '安装依赖：',
  '',
  '    pnpm install',
  '',
  '验证：',
  '',
  '    # Windows',
  '    pnpm verify:windows',
  '',
  '    # Apple Silicon macOS',
  '    pnpm verify',
  '',
  '常用命令：',
  '',
  '    # Windows 开发',
  '    pnpm dev:windows',
  '',
  '    # Windows 打包安装包',
  '    pnpm package:windows',
  '',
  '    # macOS 开发 / 打包',
  '    pnpm dev',
  '    pnpm package',
  '',
  '再次导出干净源码：',
  '',
  '    pnpm export:dev-source',
  '',
  '说明：默认只预置 OpenAI Base URL，API Key 为空；不要把本机密钥、`.env` 或私有配置复制进源码包。'
].join('\n') + '\n'

writeFileSync(join(outputDirectory, 'PORTABLE_DEV_SOURCE.md'), manifest, 'utf8')

for (const [entry, kind] of [
  ['apps', 'directory'],
  ['assets', 'directory'],
  ['DEVELOPMENT_HISTORY.md', 'file'],
  ['src', 'directory'],
  ['src-tauri', 'directory'],
  ['package.json', 'file'],
  ['pnpm-lock.yaml', 'file'],
  ['README.md', 'file'],
  ['PORTABLE_DEV_SOURCE.md', 'file']
]) {
  requireExportedPath(entry, kind)
}

for (const excluded of [
  'artifacts',
  'dist',
  'node_modules',
  'portable-dev-sources',
  'release',
  'src-tauri/target'
]) {
  if (existsSync(join(outputDirectory, excluded))) {
    throw new Error(`Export validation failed: excluded path copied into bundle: ${excluded}`)
  }
}

console.log(`Portable development source created at: ${outputDirectory}`)
