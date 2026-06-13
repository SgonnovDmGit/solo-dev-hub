import { defineConfig } from 'vitest/config';
import path from 'path';

export default defineConfig({
  test: {
    globals: true,
    environment: 'node',
    // vmForks pool required for Node.js v24 compatibility (threads pool crashes
    // with "Cannot read properties of undefined (reading 'config')").
    pool: 'vmForks',
  },
  resolve: {
    alias: {
      '$lib': path.resolve(__dirname, 'src/lib'),
    },
  },
});
