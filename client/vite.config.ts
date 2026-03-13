import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import { resolve } from 'path'

export default defineConfig({
  plugins: [vue()],
  resolve: {
    alias: { '@': resolve(__dirname, 'src') },
  },
  server: {
    port: 5173,
    proxy: {
      '/ws': { target: 'ws://localhost:8080', ws: true },
      '/api': { target: 'http://localhost:8080' },
    },
  },
  build: {
    outDir: '../server/static',
    emptyOutDir: true,
  },
})
