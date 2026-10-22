import { defineConfig } from 'vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'

export default defineConfig({
  plugins: [svelte()],
  server: {
    proxy: {
      '/api': 'http://127.0.0.1:8080',
      '/input': 'http://127.0.0.1:8080',
      '/output': 'http://127.0.0.1:8080',
      '/workspaces': 'http://127.0.0.1:8080',
    },
  },
})