import { Elysia } from "elysia";
import { cors } from "@elysiajs/cors";
import type { Client } from "discord.js";
import { getLavalink } from "../music/lavalink";
import { getRecentTracks } from "../database/db";

export function startApiServer(client: Client, port = 3000) {
  const app = new Elysia()
    .use(cors())
    .get("/health", () => ({
      status: "ok",
      uptime: process.uptime(),
      timestamp: new Date().toISOString(),
      bot: {
        tag: client.user?.tag ?? "Offline",
        guilds: client.guilds.cache.size,
        ping: client.ws.ping,
      },
    }))
    .get("/api/stats", () => {
      const memory = process.memoryUsage();
      return {
        ram: {
          rss: `${(memory.rss / 1024 / 1024).toFixed(2)} MB`,
          heapUsed: `${(memory.heapUsed / 1024 / 1024).toFixed(2)} MB`,
        },
        uptimeSec: Math.floor(process.uptime()),
        guilds: client.guilds.cache.size,
      };
    })
    .get("/api/queue/:guildId", ({ params: { guildId }, set }) => {
      try {
        const lavalink = getLavalink();
        const player = lavalink.players.get(guildId);

        if (!player) {
          set.status = 404;
          return { error: "No active player found for this guild." };
        }

        return {
          current: player.queue.current ? {
            title: player.queue.current.info.title,
            author: player.queue.current.info.author,
            uri: player.queue.current.info.uri,
            duration: player.queue.current.info.duration,
            position: player.position,
          } : null,
          queue: player.queue.tracks.map((t) => ({
            title: t.info.title,
            author: t.info.author,
            uri: t.info.uri,
            duration: t.info.duration,
          })),
          paused: player.paused,
          repeatMode: player.repeatMode,
        };
      } catch (err: any) {
        set.status = 500;
        return { error: err.message };
      }
    })
    .get("/api/history/:guildId", ({ params: { guildId } }) => {
      const history = getRecentTracks(guildId, 15);
      return { history };
    })
    .listen(port);

  console.log(`[Elysia] API Server running at http://localhost:${port}`);
  return app;
}
