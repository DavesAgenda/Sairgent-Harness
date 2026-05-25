import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import path from 'path';

export default defineConfig({
  plugins: [react(), tailwindcss()],
  test: {
    globals: true,
    environment: 'jsdom',
    setupFiles: ['./src/test-setup.ts'],
    alias: {
      // Redirect motion/react to a lightweight React-only mock during tests.
      // This prevents the React 18 (local) vs React 19 (monorepo root, used by
      // framer-motion) version conflict that causes "older React" render errors.
      'motion/react': path.resolve(__dirname, 'src/__mocks__/motion-react.tsx'),
    },
  },
});
