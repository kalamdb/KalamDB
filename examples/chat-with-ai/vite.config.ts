import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  resolve: {
    preserveSymlinks: true,
    dedupe: ['drizzle-orm', '@kalamdb/client'],
  },
  server: {
    host: true,
    port: 5174,
    strictPort: true,
    fs: {
      allow: ['..', '../..'],
    },
  },
  optimizeDeps: {
    exclude: ['@kalamdb/client', '@kalamdb/orm'],
  },
});