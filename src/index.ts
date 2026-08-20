import { Client, GatewayIntentBits, Events, ActivityType } from "discord.js";
import { commandMap } from "./discord/commands";
import {
  getPlayer,
  pause,
  resume,
  skip,
  stop,
  toggleLoop,
  shuffleQueue,
  play,
} from "./music/player";
import { createNowPlayingEmbed, formatDuration } from "./discord/components/playerEmbed";

const client = new Client({
  intents: [
    GatewayIntentBits.Guilds,
    GatewayIntentBits.GuildMessages,
    GatewayIntentBits.MessageContent,
    GatewayIntentBits.GuildVoiceStates,
  ],
});

client.once(Events.ClientReady, (c) => {
  console.log(`[INFO] Logged in as ${c.user.tag}`);

  const activities = [
    { name: "\\help | Minimal Music", type: ActivityType.Listening },
    { name: "High Fidelity Audio", type: ActivityType.Playing },
    { name: "\\play <song title>", type: ActivityType.Listening },
  ];

  let currentIdx = 0;
  c.user.setPresence({
    activities: [activities[0]!],
    status: "online",
  });

  setInterval(() => {
    currentIdx = (currentIdx + 1) % activities.length;
    c.user.setActivity(activities[currentIdx]!);
  }, 60_000);
});

// 1. Message Command Handler (Prefix \)
client.on(Events.MessageCreate, async (message) => {
  if (message.author.bot || !message.content.startsWith("\\")) return;

  const args = message.content.slice(1).trim().split(/\s+/);
  const cmdName = args.shift()?.toLowerCase();
  if (!cmdName) return;

  const command = commandMap.get(cmdName);
  if (command) {
    try {
      await command.execute(message, args);
    } catch (err: any) {
      console.error(`[Command Error: ${cmdName}]`, err);
      await message.reply(`❌ ${err.message || "An error occurred."}`).catch(() => {});
    }
  }
});

// 2. Interaction Handler (Slash Commands & Buttons)
client.on(Events.InteractionCreate, async (interaction) => {
  if (!interaction.guildId) return;
  const guildId = interaction.guildId;

  // Handle Slash Commands
  if (interaction.isChatInputCommand()) {
    const cmd = interaction.commandName;
    try {
      if (cmd === "play") {
        await interaction.deferReply();
        const query = interaction.options.getString("query", true);
        const result = await play(interaction, query);
        if (result.queued) {
          await interaction.editReply(`🎵 Added to queue: **${result.track.info.title}**`);
        } else {
          const ui = createNowPlayingEmbed(result.track.info, false, "off");
          await interaction.editReply(ui);
        }
      } else if (cmd === "skip") {
        const skipped = await skip(guildId);
        await interaction.reply(`⏭️ Skipped: **${skipped?.info.title ?? "Current track"}**`);
      } else if (cmd === "stop") {
        await stop(guildId);
        await interaction.reply("⏹️ Stopped playback and cleared queue.");
      } else if (cmd === "pause") {
        await pause(guildId);
        await interaction.reply("⏸️ Playback paused.");
      } else if (cmd === "resume") {
        await resume(guildId);
        await interaction.reply("▶️ Playback resumed.");
      } else if (cmd === "shuffle") {
        const count = shuffleQueue(guildId);
        await interaction.reply(`🔀 Shuffled **${count}** tracks.`);
      } else {
        await interaction.reply({ content: "Command executed.", ephemeral: true });
      }
    } catch (err: any) {
      if (interaction.deferred) {
        await interaction.editReply(`❌ ${err.message || "An error occurred."}`);
      } else {
        await interaction.reply({ content: `❌ ${err.message || "An error occurred."}`, ephemeral: true });
      }
    }
    return;
  }

  // Handle Buttons
  if (interaction.isButton()) {
    const player = getPlayer(guildId);
    if (!player) {
      await interaction.reply({ content: "❌ No active music player.", ephemeral: true });
      return;
    }

    try {
      switch (interaction.customId) {
        case "btn_pause": {
          if (player.isPaused) {
            await resume(guildId);
          } else {
            await pause(guildId);
          }
          if (player.queue.current) {
            const ui = createNowPlayingEmbed(
              player.queue.current.info,
              player.isPaused,
              player.repeatMode
            );
            await interaction.update(ui);
          } else {
            await interaction.deferUpdate();
          }
          break;
        }
        case "btn_skip": {
          await skip(guildId);
          await interaction.reply({ content: "⏭️ Skipped current track.", ephemeral: true });
          break;
        }
        case "btn_loop": {
          const mode = toggleLoop(guildId);
          if (player.queue.current) {
            const ui = createNowPlayingEmbed(
              player.queue.current.info,
              player.isPaused,
              mode
            );
            await interaction.update(ui);
          } else {
            await interaction.reply({ content: `🔁 Loop mode: \`${mode.toUpperCase()}\``, ephemeral: true });
          }
          break;
        }
        case "btn_shuffle": {
          shuffleQueue(guildId);
          await interaction.reply({ content: "🔀 Queue shuffled.", ephemeral: true });
          break;
        }
        case "btn_stop": {
          await stop(guildId);
          await interaction.reply({ content: "⏹️ Playback stopped and disconnected.", ephemeral: true });
          break;
        }
        case "btn_queue": {
          if (!player.queue.current && player.queue.tracks.length === 0) {
            await interaction.reply({ content: "📭 Queue is empty.", ephemeral: true });
            return;
          }
          const currentTitle = player.queue.current?.info.title || "None";
          const currentUri = player.queue.current?.info.uri || "";
          let txt = `🎶 **Now Playing:** [${currentTitle}](${currentUri})\n\n`;
          if (player.queue.tracks.length > 0) {
            txt += `📜 **Up Next (${player.queue.tracks.length} tracks):**\n`;
            player.queue.tracks.slice(0, 8).forEach((t, i) => {
              txt += `\`${i + 1}.\` [${t.info.title}](${t.info.uri}) - \`${formatDuration(t.info.duration)}\`\n`;
            });
            if (player.queue.tracks.length > 8) {
              txt += `\n*...and ${player.queue.tracks.length - 8} more*`;
            }
          }
          await interaction.reply({ content: txt, ephemeral: true });
          break;
        }
        case "btn_lyrics": {
          if (!player.queue.current) {
            await interaction.reply({ content: "❌ No track currently playing.", ephemeral: true });
            return;
          }
          await interaction.deferReply({ ephemeral: true });
          const query = `${player.queue.current.info.title} ${player.queue.current.info.author}`;
          const res = await fetch(`https://lrclib.net/api/search?q=${encodeURIComponent(query)}`, {
            signal: AbortSignal.timeout(4000),
          });
          const data: any = await res.json();
          if (Array.isArray(data) && data.length > 0 && data[0]?.plainLyrics) {
            await interaction.editReply(`📝 **Lyrics for ${player.queue.current.info.title}:**\n\n${data[0].plainLyrics.slice(0, 1900)}`);
          } else {
            await interaction.editReply(`❌ No lyrics found for **${player.queue.current.info.title}**.`);
          }
          break;
        }
      }
    } catch (err: any) {
      console.error("[Button Error]", err);
    }
  }
});

// 3. Auto-Disconnect When Voice Channel is Empty
client.on(Events.VoiceStateUpdate, (oldState) => {
  const guildId = oldState.guild.id;
  const player = getPlayer(guildId);
  if (!player) return;

  const botChannelId = oldState.guild.members.me?.voice.channelId;
  if (!botChannelId) return;

  const channel = oldState.guild.channels.cache.get(botChannelId);
  if (channel && channel.isVoiceBased()) {
    const nonBotMembers = channel.members.filter((m) => !m.user.bot);
    if (nonBotMembers.size === 0) {
      setTimeout(() => {
        const checkChan = oldState.guild.channels.cache.get(botChannelId);
        if (checkChan && checkChan.isVoiceBased()) {
          const membersNow = checkChan.members.filter((m) => !m.user.bot);
          if (membersNow.size === 0) {
            stop(guildId);
          }
        }
      }, 30_000);
    }
  }
});

client.login(process.env.DISCORD_TOKEN);
