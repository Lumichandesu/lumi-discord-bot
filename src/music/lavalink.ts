import { LavalinkManager } from "lavalink-client";
import type { Client } from "discord.js";
import { logPlayedTrack } from "../database/db";

let manager: LavalinkManager | null = null;

const NODES = [
  {
    id: "local-node",
    host: "127.0.0.1",
    port: 2333,
    authorization: "youshallnotpass",
    secure: false,
    retryAmount: Infinity,
    retryDelay: 3_000,
  },
];

export async function initLavalink(client: Client): Promise<LavalinkManager> {
  if (manager) return manager;

  const lavalink = new LavalinkManager({
    nodes: NODES,
    sendToShard: (guildId, payload) => {
      const guild = client.guilds.cache.get(guildId);
      if (guild) {
        guild.shard.send(payload);
      }
    },
    autoSkip: true,
    client: {
      id: client.user?.id ?? "",
      username: client.user?.username ?? "Lumi Bot",
    },
    playerOptions: {
      applyVolumeAsFilter: false,
      clientBasedPositionUpdateInterval: 500,
      defaultSearchPlatform: "ytsearch",
      onDisconnect: {
        autoReconnect: true,
        destroyPlayer: true,
      },
      onEmptyQueue: {
        destroyAfterMs: 60_000,
      },
      useUnresolvedData: true,
    },
    queueOptions: {
      maxPreviousTracks: 5,
    },
    linksAllowed: true,
  });

  client.on("voiceStateUpdate", (oldState, newState) => {
    const guildId = oldState.guild.id || newState.guild.id;
    const player = lavalink.players.get(guildId);
    if (!player || !player.voiceChannelId) return;

    const channel = client.channels.cache.get(player.voiceChannelId);
    if (channel && channel.isVoiceBased()) {
      const nonBots = channel.members.filter((m) => !m.user.bot);
      if (nonBots.size === 0) {
        player.destroy("Auto-leave: empty voice channel", true);
      }
    }
  });

  lavalink.nodeManager.on("connect", (node) => {
    console.log(`[Lavalink] Connected: ${node.id}`);
  });

  lavalink.nodeManager.on("reconnecting", (node) => {
    console.warn(`[Lavalink] Reconnecting: ${node.id}`);
  });

  lavalink.nodeManager.on("disconnect", (node, reason) => {
    console.warn(`[Lavalink] Disconnected: ${node.id}`, reason);
  });

  lavalink.nodeManager.on("error", (node, error) => {
    console.error(`[Lavalink] Node error: ${node.id}`, error?.message ?? error);
  });

  lavalink.on("trackStart", (player, track) => {
    if (!track) return;
    console.log(`[Music] Started: ${track.info.title}`);

    const requesterTag = (track.userData as any)?.requester?.tag ?? "Unknown";
    logPlayedTrack(
      player.guildId,
      track.info.title,
      track.info.author,
      track.info.uri ?? "",
      requesterTag,
    );
  });

  await lavalink.init({
    id: client.user!.id,
    username: client.user!.username,
  });

  manager = lavalink;
  console.log("[Lavalink] Manager ready.");
  return lavalink;
}

export function getLavalink(): LavalinkManager {
  if (!manager) {
    throw new Error("Lavalink manager is not initialized.");
  }
  return manager;
}
