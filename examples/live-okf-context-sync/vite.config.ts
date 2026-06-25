import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  resolve: {
    preserveSymlinks: true,
  },
  server: {
    strictPort: true,
    host: '127.0.0.1',
    port: 5177,
    fs: {
      allow: ['..', '../..'],
    },
  },
  build: {
    outDir: 'dist',
    sourcemap: true,
    target: 'es2022',
  },
  optimizeDeps: {
    exclude: ['@kalamdb/client'],
  },
});
