import {
  createAudioPlayer,
  createAudioResource,
  AudioPlayerStatus,
  joinVoiceChannel,
  VoiceConnection,
  AudioPlayer,
  StreamType,
  VoiceConnectionStatus,
  entersState,
} from "@discordjs/voice";
import { spawn } from "node:child_process";
import type { Provider, ResolvedTrack } from "./types";
import { resolveTrack } from "./extractor";

export interface PlayerTrack extends ResolvedTrack {
  info: {
    title: string;
    uri: string;
    author: string;
    duration: number;
    thumbnail: string | null;
  };
}

export interface BotPlayer {
  guildId: string;
  voiceChannelId: string;
  connection: VoiceConnection;
  audioPlayer: AudioPlayer;
  repeatMode: "off" | "track" | "queue";
  isPaused: boolean;
  idleTimeout: ReturnType<typeof setTimeout> | null;
  queue: {
    current: PlayerTrack | null;
    tracks: PlayerTrack[];
  };
}

export const players = new Map<string, BotPlayer>();

export function getPlayer(guildId: string): BotPlayer | undefined {
  return players.get(guildId);
}

export function requireGuild(ctx: any): any {
  const guild = ctx.guild || ctx.member?.guild;
  if (!guild) throw new Error("This command can only be used in a Discord server.");
  return guild;
}

export function requireVoiceChannel(ctx: any): any {
  const channel = ctx.member?.voice?.channel;
  if (!channel) throw new Error("You must be in a voice channel to use this command.");
  return channel;
}

function toPlayerTrack(resolved: ResolvedTrack): PlayerTrack {
  return {
    ...resolved,
    info: {
      title: resolved.title,
      uri: resolved.sourceUrl,
      author: resolved.uploader,
      duration: resolved.duration || 0,
      thumbnail: resolved.thumbnail,
    },
  };
}

function createOptimizedStream(streamUrl: string, seekSeconds = 0) {
  const ffmpegArgs = [
    "-reconnect", "1",
    "-reconnect_streamed", "1",
    "-reconnect_delay_max", "5",
  ];

  if (seekSeconds > 0) {
    ffmpegArgs.push("-ss", seekSeconds.toString());
  }

  ffmpegArgs.push(
    "-i", streamUrl,
    "-threads", "1",
    "-analyzeduration", "0",
    "-loglevel", "error",
    "-f", "s16le",
    "-ar", "48000",
    "-ac", "2",
    "pipe:1"
  );

  const ffmpeg = spawn("ffmpeg", ffmpegArgs, {
    stdio: ["ignore", "pipe", "ignore"],
  });

  return createAudioResource(ffmpeg.stdout, {
    inputType: StreamType.Raw,
    inlineVolume: true,
  });
}

function clearPlayerIdleTimeout(player: BotPlayer) {
  if (player.idleTimeout) {
    clearTimeout(player.idleTimeout);
    player.idleTimeout = null;
  }
}

function setPlayerIdleTimeout(player: BotPlayer) {
  clearPlayerIdleTimeout(player);
  player.idleTimeout = setTimeout(() => {
    const currentP = players.get(player.guildId);
    if (currentP && !currentP.queue.current && currentP.queue.tracks.length === 0) {
      stop(player.guildId);
    }
  }, 180_000);
}

function startTrack(player: BotPlayer, track: PlayerTrack, seekSeconds = 0) {
  clearPlayerIdleTimeout(player);
  player.queue.current = track;
  player.isPaused = false;
  try {
    const resource = createOptimizedStream(track.streamUrl, seekSeconds);
    player.audioPlayer.play(resource);
  } catch (err) {
    console.error("[AudioPlayer Play Error]", err);
    const next = player.queue.tracks.shift();
    if (next) startTrack(player, next);
    else {
      player.queue.current = null;
      setPlayerIdleTimeout(player);
    }
  }
}

export async function play(ctx: any, query?: string): Promise<{ queued: boolean; track: PlayerTrack }> {
  const guild = requireGuild(ctx);
  const voiceChannel = requireVoiceChannel(ctx);
  const input = query || ctx.options?.getString?.("query") || ctx.content?.replace(/^\\\S+\s*/, "");

  if (!input) throw new Error("Please provide a track title or URL.");

  let player = players.get(guild.id);

  if (!player) {
    const connection = joinVoiceChannel({
      channelId: voiceChannel.id,
      guildId: guild.id,
      adapterCreator: guild.voiceAdapterCreator,
      selfDeaf: true,
    });

    const audioPlayer = createAudioPlayer();
    connection.subscribe(audioPlayer);

    player = {
      guildId: guild.id,
      voiceChannelId: voiceChannel.id,
      connection,
      audioPlayer,
      repeatMode: "off",
      isPaused: false,
      idleTimeout: null,
      queue: {
        current: null,
        tracks: [],
      },
    };

    players.set(guild.id, player);

    audioPlayer.on(AudioPlayerStatus.Idle, () => {
      const p = players.get(guild.id);
      if (!p) return;

      if (p.repeatMode === "track" && p.queue.current) {
        startTrack(p, p.queue.current);
      } else if (p.repeatMode === "queue" && p.queue.current) {
        p.queue.tracks.push(p.queue.current);
        const next = p.queue.tracks.shift();
        if (next) startTrack(p, next);
        else {
          p.queue.current = null;
          setPlayerIdleTimeout(p);
        }
      } else {
        const next = p.queue.tracks.shift();
        if (next) startTrack(p, next);
        else {
          p.queue.current = null;
          setPlayerIdleTimeout(p);
        }
      }
    });

    audioPlayer.on("error", (err: any) => {
      console.error(`[AudioPlayer Error ${guild.id}]`, err.message);
      const next = player?.queue.tracks.shift();
      if (next && player) startTrack(player, next);
      else if (player) {
        player.queue.current = null;
        setPlayerIdleTimeout(player);
      }
    });

    connection.on(VoiceConnectionStatus.Disconnected, async () => {
      try {
        await Promise.race([
          entersState(connection, VoiceConnectionStatus.Signalling, 5_000),
          entersState(connection, VoiceConnectionStatus.Connecting, 5_000),
        ]);
      } catch {
        stop(guild.id);
      }
    });
  } else {
    player.voiceChannelId = voiceChannel.id;
  }

  const resolved = await resolveTrack(input);
  const track = toPlayerTrack(resolved);

  if (!player.queue.current && player.queue.tracks.length === 0) {
    startTrack(player, track);
    return { queued: false, track };
  } else {
    player.queue.tracks.push(track);
    return { queued: true, track };
  }
}

export async function pause(guildId: string): Promise<boolean> {
  const p = players.get(guildId);
  if (!p || p.isPaused) return false;
  p.audioPlayer.pause();
  p.isPaused = true;
  return true;
}

export async function resume(guildId: string): Promise<boolean> {
  const p = players.get(guildId);
  if (!p || !p.isPaused) return false;
  p.audioPlayer.unpause();
  p.isPaused = false;
  return true;
}

export async function skip(guildId: string): Promise<PlayerTrack | null> {
  const p = players.get(guildId);
  if (!p) return null;
  const current = p.queue.current;
  p.audioPlayer.stop();
  return current;
}

export async function stop(guildId: string): Promise<boolean> {
  const p = players.get(guildId);
  if (!p) return false;
  clearPlayerIdleTimeout(p);
  p.queue.tracks = [];
  p.queue.current = null;
  p.audioPlayer.stop();
  p.connection.destroy();
  players.delete(guildId);
  return true;
}

export function clearQueue(guildId: string): number {
  const p = players.get(guildId);
  if (!p) return 0;
  const count = p.queue.tracks.length;
  p.queue.tracks = [];
  return count;
}

export function shuffleQueue(guildId: string): number {
  const p = players.get(guildId);
  if (!p || p.queue.tracks.length === 0) return 0;
  p.queue.tracks.sort(() => Math.random() - 0.5);
  return p.queue.tracks.length;
}

export function toggleLoop(guildId: string): "off" | "track" | "queue" {
  const p = players.get(guildId);
  if (!p) return "off";
  if (p.repeatMode === "off") p.repeatMode = "track";
  else if (p.repeatMode === "track") p.repeatMode = "queue";
  else p.repeatMode = "off";
  return p.repeatMode;
}

export function removeQueueTrack(guildId: string, index: number): PlayerTrack {
  const p = players.get(guildId);
  if (!p || p.queue.tracks.length === 0) {
    throw new Error("The queue is currently empty.");
  }
  const idx = index > 0 ? index - 1 : index;
  if (idx < 0 || idx >= p.queue.tracks.length) {
    throw new Error(`Track index ${index} not found.`);
  }
  const removed = p.queue.tracks.splice(idx, 1);
  const track = removed[0];
  if (!track) throw new Error("An error occurred while removing the track.");
  return track;
}

export function seek(guildId: string, seconds: number): boolean {
  const p = players.get(guildId);
  if (!p || !p.queue.current) return false;
  startTrack(p, p.queue.current, seconds);
  return true;
}
