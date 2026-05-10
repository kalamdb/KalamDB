import path from 'node:path';
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

const appNodeModules = new URL('./node_modules', import.meta.url).pathname;

export default defineConfig({
  plugins: [react()],
  resolve: {
    preserveSymlinks: true,
    dedupe: ['react', 'react-dom', 'drizzle-orm', '@kalamdb/client', '@kalamdb/orm', '@kalamdb/react'],
    alias: [
      { find: /^react$/, replacement: path.join(appNodeModules, 'react/index.js') },
      { find: /^react\/jsx-runtime$/, replacement: path.join(appNodeModules, 'react/jsx-runtime.js') },
      { find: /^react\/jsx-dev-runtime$/, replacement: path.join(appNodeModules, 'react/jsx-dev-runtime.js') },
      { find: /^react-dom\/client$/, replacement: path.join(appNodeModules, 'react-dom/client.js') },
      { find: /^react-dom$/, replacement: path.join(appNodeModules, 'react-dom/index.js') },
      { find: '@kalamdb/client', replacement: new URL('../../link/sdks/typescript/client/dist/src/index.js', import.meta.url).pathname },
      { find: '@kalamdb/orm', replacement: new URL('../../link/sdks/typescript/orm/dist/index.js', import.meta.url).pathname },
      { find: '@kalamdb/react', replacement: new URL('../../link/sdks/typescript/react-old/dist/index.js', import.meta.url).pathname },
    ],
  },
  server: {
    host: true,
    port: 5176,
    strictPort: true,
    fs: {
      allow: ['.', '../..'],
    },
  },
  optimizeDeps: {
    exclude: ['@kalamdb/client', '@kalamdb/orm', '@kalamdb/react'],
  },
});