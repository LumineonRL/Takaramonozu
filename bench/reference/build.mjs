// Turns the headless burrito sim into a single file for Node to execute.
// This is done through two build-time substitutions.
// 1) A vue shim with aliases for its helpers
// 2) VITE_APP_DIAGNOSTICS set to false.

import { build } from 'esbuild'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))

await build({
    entryPoints: [join(here, 'runner.ts')],
    outfile: join(here, 'dist/runner.mjs'),
    bundle: true,
    platform: 'node',
    target: 'node20',
    format: 'esm',
    define: { VITE_APP_DIAGNOSTICS: 'false' },
    alias: { vue: join(here, 'vue-shim.ts') },
    logLevel: 'info',
})
