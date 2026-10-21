// Lets a script in this repo import Caravel's own TypeScript modules (src/crypto/*) and its packages,
// so the trading client reuses Caravel's wallet, scanning and fee code rather than a copy of it.
//
//   node --import ./scripts/lib/caravel-hooks.mjs scripts/live.ts <command>
//
// CARAVEL_DIR points at a Caravel checkout (default ~/dev/caravel). Three things are resolved here:
//   1. extensionless relative imports inside Caravel's sources (its bundler convention);
//   2. the aliases Caravel's build configs carry (vendored tari-cipherseed and ONS);
//   3. bare package imports from THIS repo's scripts, which resolve from Caravel's node_modules.
// Plus an in-memory localStorage, which Caravel's stores expect. Nothing is persisted.

import { statSync } from 'node:fs'
import { registerHooks } from 'node:module'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const CARAVEL = path.resolve(process.env.CARAVEL_DIR ?? path.join(os.homedir(), 'dev/caravel'))
const HERE = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')

const ALIASES = new Map([
  ['tari-cipherseed', path.join(CARAVEL, 'vendor/tari-cipherseed/src/index.ts')],
  ['@ootle/name-service', path.join(CARAVEL, 'vendor/ons/dist/index.js')],
])

const isFile = (p) => { try { return statSync(p).isFile() } catch { return false } }
const CANDIDATES = (base) => [`${base}.ts`, `${base}.tsx`, `${base}.js`, path.join(base, 'index.ts')]

registerHooks({
  resolve(specifier, context, nextResolve) {
    const aliased = ALIASES.get(specifier)
    if (aliased) return nextResolve(pathToFileURL(aliased).href, context)

    if (specifier.startsWith('caravel:')) {
      // `caravel:src/crypto/derivation`: a module from the Caravel checkout.
      const target = path.join(CARAVEL, specifier.slice('caravel:'.length))
      for (const c of [target, ...CANDIDATES(target)]) if (isFile(c)) return nextResolve(pathToFileURL(c).href, context)
      throw new Error(`caravel module not found: ${specifier}`)
    }

    const parentPath = context.parentURL?.startsWith('file:') ? fileURLToPath(context.parentURL) : null
    if (!specifier.startsWith('.') && !specifier.startsWith('/') && !specifier.startsWith('file:') && !specifier.startsWith('node:')) {
      // A bare package imported from this repo resolves from Caravel's node_modules.
      if (parentPath && parentPath.startsWith(HERE)) {
        return nextResolve(specifier, { ...context, parentURL: pathToFileURL(path.join(CARAVEL, 'package.json')).href })
      }
      return nextResolve(specifier, context)
    }

    if (parentPath && !specifier.startsWith('file:') && !specifier.startsWith('node:')) {
      const target = path.resolve(path.dirname(parentPath), specifier)
      if (!isFile(target)) for (const c of CANDIDATES(target)) if (isFile(c)) return nextResolve(pathToFileURL(c).href, context)
    }
    return nextResolve(specifier, context)
  },
})

const store = new Map()
Object.defineProperty(globalThis, 'localStorage', {
  configurable: true,
  writable: true,
  value: {
    get length() { return store.size },
    key: (i) => [...store.keys()][i] ?? null,
    getItem: (k) => (store.has(String(k)) ? store.get(String(k)) : null),
    setItem: (k, v) => { store.set(String(k), String(v)) },
    removeItem: (k) => { store.delete(String(k)) },
    clear: () => { store.clear() },
  },
})
