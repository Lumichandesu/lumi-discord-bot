import {
  createAudioPlayer,
  createAudioResource,
  joinVoiceChannel,
  AudioPlayerStatus,
  VoiceConnectionStatus,
  StreamType,
  type AudioPlayer,
  type VoiceConnection,
} from "@discordjs/voice";
import { spawn, type ChildProcess } from "node:child_process";
import type { Message, Guild } from "discord.js";
import { resolveTrack } from "./extractor";
import { logPlayedTrack } from "../database/db";

export type RepeatMode = "off" | "track" | "queue";

export interface TrackInfo {
  title: string;
  author: string;
  uri: string;
  duration?: number | null;
  artworkUrl?: string | null;
}

export interface SearchTrack {
  info: TrackInfo;
  streamUrl: string;
  sourceUrl: string;
  userData: {
    requester: {
      tag: string;
      id: string;
    };
  };
}

export class GuildPlayer {
  guildId: string;
  voiceChannelId: string;
  textChannelId: string;
  connection: VoiceConnection | null = null;
  audioPlayer: AudioPlayer;
  repeatMode: RepeatMode = "off";
  paused = false;
  currentProcess: ChildProcess | null = null;
  queue: {
    current: SearchTrack | null;
    tracks: SearchTrack[];
  } = {
    current: null,
    tracks: [],
  };

  constructor(guildId: string, voiceChannelId: string, textChannelId: string) {
    this.guildId = guildId;
    this.voiceChannelId = voiceChannelId;
    this.textChannelId = textChannelId;
    this.audioPlayer = createAudioPlayer();

    this.audioPlayer.on(AudioPlayerStatus.Idle, async () => {
      this.stopProcess();

      if (this.repeatMode === "track" && this.queue.current) {
        await this.playTrack(this.queue.current);
        return;
      }

      if (this.repeatMode === "queue" && this.queue.current) {
        this.queue.tracks.push(this.queue.current);
      }

      const nextTrack = this.queue.tracks.shift();
      if (nextTrack) {
        await this.playTrack(nextTrack);
      } else {
        this.queue.current = null;
      }
    });

    this.audioPlayer.on("error", (error) => {
      console.error(`[AudioPlayer] Guild ${this.guildId} error:`, error.message);
      this.stopProcess();
      const nextTrack = this.queue.tracks.shift();
      if (nextTrack) {
        this.playTrack(nextTrack);
      } else {
        this.queue.current = null;
      }
    });
  }

  stopProcess() {
    if (this.currentProcess) {
      try {
        this.currentProcess.kill("SIGKILL");
      } catch {}
      this.currentProcess = null;
    }
  }

  async playTrack(track: SearchTrack) {
    this.stopProcess();
    this.queue.current = track;
    this.paused = false;

    let streamUrl = track.streamUrl;
    if (!streamUrl) {
      const resolved = await resolveTrack(track.sourceUrl);
      streamUrl = resolved.streamUrl;
      track.streamUrl = streamUrl;
    }

    const ffmpeg = spawn("ffmpeg", [
      "-reconnect", "1",
      "-reconnect_streamed", "1",
      "-reconnect_delay_max", "5",
      "-i", streamUrl,
      "-analyzeduration", "0",
      "-loglevel", "0",
      "-f", "s16le",
      "-ar", "48000",
      "-ac", "2",
      "pipe:1",
    ], {
      stdio: ["ignore", "pipe", "ignore"],
    });

    this.currentProcess = ffmpeg;

    const resource = createAudioResource(ffmpeg.stdout!, {
      inputType: StreamType.Raw,
      inlineVolume: true,
    });
    resource.volume?.setVolume(1.0);

    this.audioPlayer.play(resource);

    logPlayedTrack(
      this.guildId,
      track.info.title,
      track.info.author,
      track.info.uri,
      track.userData.requester.tag,
    );
  }
}

const players = new Map<string, GuildPlayer>();

export function requireGuild(message: Message): Guild {
  if (!message.guild) throw new Error("คำสั่งนี้ใช้ได้เฉพาะในเซิร์ฟเวอร์เท่านั้น");
  return message.guild;
}

export function requireVoiceChannel(message: Message): string {
  const channel = message.member?.voice.channel;
  if (!channel) throw new Error("กรุณาเข้าห้องเสียงก่อนใช้งานคำสั่ง");
  if (!channel.isVoiceBased()) throw new Error("ห้องเสียงไม่ถูกต้อง");
  return channel.id;
}

export function getPlayer(guildId: string): GuildPlayer | null {
  return players.get(guildId) ?? null;
}

export function getOrCreatePlayer(guild: Guild, voiceChannelId: string, textChannelId: string): GuildPlayer {
  let player = players.get(guild.id);

  if (!player) {
    player = new GuildPlayer(guild.id, voiceChannelId, textChannelId);
    players.set(guild.id, player);
  }

  if (!player.connection || player.connection.state.status === VoiceConnectionStatus.Destroyed) {
    const connection = joinVoiceChannel({
      channelId: voiceChannelId,
      guildId: guild.id,
      adapterCreator: guild.voiceAdapterCreator as any,
      selfDeaf: true,
      selfMute: false,
    });

    connection.subscribe(player.audioPlayer);
    player.connection = connection;
    player.voiceChannelId = voiceChannelId;
  } else if (player.voiceChannelId !== voiceChannelId) {
    player.connection.rejoin({
      channelId: voiceChannelId,
      selfDeaf: true,
      selfMute: false,
    });
    player.voiceChannelId = voiceChannelId;
  }

  return player;
}

export async function play(
  message: Message,
  query: string,
): Promise<{ track: SearchTrack; queued: boolean }> {
  const guild = requireGuild(message);
  const voiceChannelId = requireVoiceChannel(message);
  const player = getOrCreatePlayer(guild, voiceChannelId, message.channel.id);

  const resolved = await resolveTrack(query);

  const track: SearchTrack = {
    info: {
      title: resolved.title,
      author: resolved.uploader,
      uri: resolved.sourceUrl,
      duration: resolved.duration ? resolved.duration * 1000 : null,
      artworkUrl: resolved.thumbnail,
    },
    streamUrl: resolved.streamUrl,
    sourceUrl: resolved.sourceUrl,
    userData: {
      requester: {
        tag: message.author.tag,
        id: message.author.id,
      },
    },
  };

  const wasPlaying = Boolean(player.queue.current);

  if (wasPlaying) {
    player.queue.tracks.push(track);
    return { track, queued: true };
  } else {
    await player.playTrack(track);
    return { track, queued: false };
  }
}

export async function pause(guildId: string): Promise<void> {
  const player = getPlayer(guildId);
  if (!player || !player.queue.current) throw new Error("Nothing is currently playing.");
  if (player.paused) throw new Error("Player is already paused.");
  player.audioPlayer.pause();
  player.paused = true;
}

export async function resume(guildId: string): Promise<void> {
  const player = getPlayer(guildId);
  if (!player || !player.queue.current) throw new Error("No music player is active.");
  if (!player.paused) throw new Error("Player is already playing.");
  player.audioPlayer.unpause();
  player.paused = false;
}

export async function skip(guildId: string): Promise<SearchTrack | null> {
  const player = getPlayer(guildId);
  if (!player || !player.queue.current) throw new Error("Nothing is playing to skip.");

  const currentTrack = player.queue.current;
  const tempMode = player.repeatMode;
  if (tempMode === "track") player.repeatMode = "off";

  player.stopProcess();
  player.audioPlayer.stop();

  if (tempMode === "track") player.repeatMode = "track";
  return currentTrack;
}

export async function stop(guildId: string): Promise<void> {
  const player = getPlayer(guildId);
  if (!player) return;

  player.queue.tracks = [];
  player.queue.current = null;
  player.stopProcess();
  player.audioPlayer.stop();

  if (player.connection) {
    player.connection.destroy();
    player.connection = null;
  }
  players.delete(guildId);
}

export function toggleLoop(guildId: string): RepeatMode {
  const player = getPlayer(guildId);
  if (!player || !player.queue.current) throw new Error("Nothing is currently playing.");

  if (player.repeatMode === "off") player.repeatMode = "track";
  else if (player.repeatMode === "track") player.repeatMode = "queue";
  else player.repeatMode = "off";

  return player.repeatMode;
}

export function shuffleQueue(guildId: string): number {
  const player = getPlayer(guildId);
  if (!player || player.queue.tracks.length === 0) throw new Error("Queue is empty.");

  for (let i = player.queue.tracks.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    const temp = player.queue.tracks[i]!;
    player.queue.tracks[i] = player.queue.tracks[j]!;
    player.queue.tracks[j] = temp;
  }
  return player.queue.tracks.length;
}

export function clearQueue(guildId: string): number {
  const player = getPlayer(guildId);
  if (!player || player.queue.tracks.length === 0) throw new Error("Queue is already empty.");

  const count = player.queue.tracks.length;
  player.queue.tracks = [];
  return count;
}

export function removeQueueTrack(guildId: string, index: number): SearchTrack {
  const player = getPlayer(guildId);
  if (!player || player.queue.tracks.length === 0) throw new Error("Queue is empty.");

  if (index < 1 || index > player.queue.tracks.length) {
    throw new Error(`Invalid track number. Choose 1 to ${player.queue.tracks.length}.`);
  }

  const removed = player.queue.tracks.splice(index - 1, 1)[0];
  if (!removed) throw new Error("Failed to remove track.");
  return removed;
}
