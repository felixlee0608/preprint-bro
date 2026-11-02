import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// preprint-bro is served under /preprint-bro on aero.oops.wtf, so every asset URL
// carries the prefix. In dev, the API is the local backend on :8083.
export default defineConfig({
  base: '/preprint-bro/',
  plugins: [react()],
  // Leave the dependencies' @license banners out of the bundle.
  esbuild: { legalComments: 'none' },
  server: {
    proxy: { '/preprint-bro/api': 'http://127.0.0.1:8083' },
  },
})
