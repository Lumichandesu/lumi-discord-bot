import { Client, GatewayIntentBits, Events } from "discord.js";
import { registerPrefixCommands } from "./discord/commands";
import { play } from "./music/player";
import { getQueue } from "./music/queue";

const token = process.env.DISCORD_TOKEN;
if (!token) {
  throw new Error("Missing DISCORD_TOKEN in environment");
}

const client = new Client({
  intents: [
    GatewayIntentBits.Guilds,
    GatewayIntentBits.GuildVoiceStates,
    GatewayIntentBits.GuildMessages,
    GatewayIntentBits.MessageContent,
  ],
});

client.once(Events.ClientReady, (readyClient) => {
  console.log(`[Bot] Logged in as ${readyClient.user.tag}`);
});

// 1. Prefix command listener (\play, etc.)
registerPrefixCommands(client);

// 2. Slash command listener (/play, etc.)
client.on(Events.InteractionCreate, async (interaction) => {
  if (!interaction.isChatInputCommand()) return;

  const { commandName, guildId } = interaction;
  const member = interaction.member;

  if (!guildId || !member || !("voice" in member) || !member.voice.channel) {
    await interaction.reply({
      content: "❌ You must be in a Voice Channel to use this command.",
      ephemeral: true,
    });
    return;
  }

  try {
    if (commandName === "play") {
      await interaction.deferReply();
      const query = interaction.options.getString("query", true);
      
      const fakeMessage = {
        guild: interaction.guild,
        member: interaction.member,
        channel: interaction.channel,
        reply: async (payload: any) => interaction.followUp(payload),
      } as any;

      const result = await play(fakeMessage, query);
      await interaction.followUp(
        result.queued
          ? `🎵 Added to queue: **${result.track.info.title}**`
          : `▶️ Playing: **${result.track.info.title}**`
      );
    } else if (commandName === "skip") {
      const q = getQueue(guildId);
      if (!q || !q.currentTrack) {
        await interaction.reply("❌ Nothing is currently playing.");
        return;
      }
      q.skip();
      await interaction.reply("⏭️ Skipped current track.");
    } else if (commandName === "stop") {
      const q = getQueue(guildId);
      if (!q) {
        await interaction.reply("❌ Bot is not connected.");
        return;
      }
      q.destroy();
      await interaction.reply("⏹️ Stopped playback and disconnected.");
    } else if (commandName === "pause") {
      const q = getQueue(guildId);
      if (!q || !q.currentTrack) {
        await interaction.reply("❌ Nothing is playing to pause.");
        return;
      }
      q.pause();
      await interaction.reply("⏸️ Playback paused.");
    } else if (commandName === "resume") {
      const q = getQueue(guildId);
      if (!q || !q.currentTrack) {
        await interaction.reply("❌ Nothing is paused to resume.");
        return;
      }
      q.resume();
      await interaction.reply("▶️ Playback resumed.");
    } else if (commandName === "nowplaying") {
      const q = getQueue(guildId);
      if (!q || !q.currentTrack) {
        await interaction.reply("❌ Nothing is currently playing.");
        return;
      }
      const t = q.currentTrack.info;
      await interaction.reply(`🎶 Now Playing: **${t.title}** by **${t.uploader}**`);
    } else if (commandName === "queue") {
      const q = getQueue(guildId);
      if (!q || (!q.currentTrack && q.tracks.length === 0)) {
        await interaction.reply("📭 Queue is empty.");
        return;
      }
      const list = q.tracks.slice(0, 10).map((tr, i) => `${i + 1}. ${tr.info.title}`).join("\n");
      await interaction.reply(`📋 **Upcoming Queue:**\n${list || "No further tracks."}`);
    } else if (commandName === "clear") {
      const q = getQueue(guildId);
      if (!q) {
        await interaction.reply("📭 Queue is already empty.");
        return;
      }
      q.clear();
      await interaction.reply("🗑️ Queue cleared.");
    } else if (commandName === "shuffle") {
      const q = getQueue(guildId);
      if (!q || q.tracks.length < 2) {
        await interaction.reply("❌ Not enough tracks in queue to shuffle.");
        return;
      }
      q.shuffle();
      await interaction.reply("🔀 Queue shuffled.");
    }
  } catch (error) {
    console.error("[Slash Command]", error);
    if (interaction.deferred || interaction.replied) {
      await interaction.followUp(`❌ ${error instanceof Error ? error.message : "Error executing command."}`);
    } else {
      await interaction.reply(`❌ ${error instanceof Error ? error.message : "Error executing command."}`);
    }
  }
});

client.login(token);
