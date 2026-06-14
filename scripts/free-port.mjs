// Frees a TCP port before `npm run dev` starts vite.
//
// Why: an aborted `tauri dev` orphans its vite child (the tauri/npm parent
// dies, the node grandchild keeps listening). vite uses `strictPort` — which
// is intentional, the Tauri `devUrl` is pinned to this exact port — so the
// next launch hard-fails with "Port 1420 already in use" instead of falling
// back. This clears the squatter so dev starts clean every time.
//
// Best-effort: always exits 0. If nothing is on the port, or a probe tool is
// missing, it does nothing rather than blocking the dev start.

import { execSync } from 'node:child_process';

const port = process.argv[2] || '1420';
const pids = new Set();

try {
  if (process.platform === 'win32') {
    // Plain `netstat -ano` (no `-p tcp`) — `-p tcp` is IPv4-only on Windows
    // and vite binds IPv6 loopback (`[::1]:1420`), which lives under TCPv6.
    const out = execSync('netstat -ano', { encoding: 'utf8' });
    for (const line of out.split('\n')) {
      const m = line.match(/LISTENING\s+(\d+)\s*$/i);
      if (m && new RegExp(`[:.]${port}\\b`).test(line)) pids.add(m[1]);
    }
    for (const pid of pids) {
      try {
        execSync(`taskkill /F /PID ${pid}`, { stdio: 'ignore' });
        console.log(`free-port: freed :${port} (killed PID ${pid})`);
      } catch {
        // process already gone — ignore
      }
    }
  } else {
    const out = execSync(`lsof -ti tcp:${port} -s tcp:LISTEN`, { encoding: 'utf8' }).trim();
    if (out) {
      const list = out.split('\n').filter(Boolean);
      execSync(`kill -9 ${list.join(' ')}`, { stdio: 'ignore' });
      console.log(`free-port: freed :${port} (killed ${list.join(' ')})`);
    }
  }
} catch {
  // No listener on the port (probe exits non-zero) — nothing to free.
}
