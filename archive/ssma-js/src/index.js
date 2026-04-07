import { createServer } from './app.js';
import { loadConfig } from './config/env.js';

const config = loadConfig();
const server = createServer(config);

const shutdown = (signal) => {
  console.log(`\n[SSMA] Received ${signal}. Gracefully shutting down...`);
  server.close(() => {
    process.exit(0);
  });
};

server.listen(config.server.port, () => {
  console.log(`[SSMA] Auth service listening on port ${config.server.port}`);
  if (process.env.SSMA_EXIT_AFTER_START === 'true') {
    setTimeout(() => shutdown('startup-check'), 50);
  }
});

process.on('SIGINT', () => shutdown('SIGINT'));
process.on('SIGTERM', () => shutdown('SIGTERM'));
