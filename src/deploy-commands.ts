import { REST, Routes, SlashCommandBuilder } from "discord.js";

const token = process.env.DISCORD_TOKEN;
const clientId = process.env.DISCORD_CLIENT_ID;

if (!token || !clientId) {
  console.error("❌ Missing DISCORD_TOKEN or DISCORD_CLIENT_ID in .env");
  process.exit(1);
}

const commands = [
  new SlashCommandBuilder()
    .setName("play")
    .setDescription("Stream music from YouTube, Spotify, Apple Music, TIDAL, or SoundCloud")
    .setContexts([0, 1, 2])
    .setIntegrationTypes([0, 1])
    .addStringOption((option) =>
      option
        .setName("query")
        .setDescription("Song title or track URL")
        .setRequired(true)
    ),
  new SlashCommandBuilder()
    .setName("skip")
    .setDescription("Skip the currently playing track")
    .setContexts([0, 1, 2])
    .setIntegrationTypes([0, 1]),
  new SlashCommandBuilder()
    .setName("stop")
    .setDescription("Stop playback, clear queue, and leave voice channel")
    .setContexts([0, 1, 2])
    .setIntegrationTypes([0, 1]),
  new SlashCommandBuilder()
    .setName("nowplaying")
    .setDescription("Display live metadata of the currently playing track")
    .setContexts([0, 1, 2])
    .setIntegrationTypes([0, 1]),
  new SlashCommandBuilder()
    .setName("queue")
    .setDescription("View upcoming tracks in the queue")
    .setContexts([0, 1, 2])
    .setIntegrationTypes([0, 1]),
  new SlashCommandBuilder()
    .setName("pause")
    .setDescription("Pause current playback")
    .setContexts([0, 1, 2])
    .setIntegrationTypes([0, 1]),
  new SlashCommandBuilder()
    .setName("resume")
    .setDescription("Resume paused playback")
    .setContexts([0, 1, 2])
    .setIntegrationTypes([0, 1]),
  new SlashCommandBuilder()
    .setName("shuffle")
    .setDescription("Randomize queued tracks")
    .setContexts([0, 1, 2])
    .setIntegrationTypes([0, 1]),
  new SlashCommandBuilder()
    .setName("clear")
    .setDescription("Clear all tracks in the upcoming queue")
    .setContexts([0, 1, 2])
    .setIntegrationTypes([0, 1]),
].map((cmd) => cmd.toJSON());

const rest = new REST({ version: "10" }).setToken(token);

async function deploy() {
  try {
    console.log(`⏳ Updating ${commands.length} application commands with global contexts...`);
    await rest.put(Routes.applicationCommands(clientId as string), { body: commands });
    console.log("✅ Commands updated successfully!");
  } catch (error) {
    console.error("❌ Failed to update commands:", error);
  }
}

deploy();
