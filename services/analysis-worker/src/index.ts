import { loadConfig } from "./config";
import { healthResponse, readinessResponse, SERVICE_NAME } from "./health";

const config = loadConfig();

const server = Bun.serve({
  hostname: config.hostname,
  port: config.port,
  fetch(request) {
    const { pathname } = new URL(request.url);

    if (request.method === "GET" && pathname === "/healthz") {
      return Response.json(healthResponse());
    }
    if (request.method === "GET" && pathname === "/readyz") {
      return Response.json(readinessResponse());
    }
    return Response.json({ error: "not_found" }, { status: 404 });
  },
});

console.log(
  JSON.stringify({
    level: "info",
    service: SERVICE_NAME,
    event: "listening",
    address: `http://${server.hostname}:${server.port}`,
  }),
);

let shuttingDown = false;

async function shutdown(signal: string): Promise<void> {
  if (shuttingDown) return;
  shuttingDown = true;
  console.log(JSON.stringify({ level: "info", service: SERVICE_NAME, event: "shutdown", signal }));
  await server.stop(true);
  process.exit(0);
}

process.on("SIGINT", () => void shutdown("SIGINT"));
process.on("SIGTERM", () => void shutdown("SIGTERM"));
