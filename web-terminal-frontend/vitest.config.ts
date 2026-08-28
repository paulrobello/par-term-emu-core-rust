import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vitest/config';

// Mirrors tsconfig.json's "@/*" -> "./*" path mapping.
const frontendRoot = fileURLToPath(new URL('.', import.meta.url)).replace(/\/$/, '');

export default defineConfig({
  resolve: {
    alias: {
      '@': frontendRoot,
    },
  },
  test: {
    environment: 'happy-dom',
    include: ['lib/__tests__/**/*.test.ts'],
  },
});
