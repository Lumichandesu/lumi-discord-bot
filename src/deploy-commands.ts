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
    .addStringOption((option) =>
      option
        .setName("query")
        .setDescription("Song title or track URL")
        .setRequired(true)
    ),
  new SlashCommandBuilder()
    .setName("skip")
    .setDescription("Skip the currently playing track"),
  new SlashCommandBuilder()
    .setName("stop")
    .setDescription("Stop playback, clear queue, and leave voice channel"),
  new SlashCommandBuilder()
    .setName("nowplaying")
    .setDescription("Display live metadata of the currently playing track"),
  new SlashCommandBuilder()
    .setName("queue")
    .setDescription("View upcoming tracks in the queue"),
  new SlashCommandBuilder()
    .setName("pause")
    .setDescription("Pause current playback"),
  new SlashCommandBuilder()
    .setName("resume")
    .setDescription("Resume paused playback"),
  new SlashCommandBuilder()
    .setName("loop")
    .setDescription("Set queue loop mode")
    .addStringOption((option) =>
      option
        .setName("mode")
        .setDescription("Loop mode")
        .setRequired(false)
        .addChoices(
          { name: "Off", value: "off" },
          { name: "Single Track", value: "track" },
          { name: "Whole Queue", value: "queue" }
        )
    ),
  new SlashCommandBuilder()
    .setName("shuffle")
    .setDescription("Randomize queued tracks"),
  new SlashCommandBuilder()
    .setName("clear")
    .setDescription("Clear all tracks in the upcoming queue"),
].map((cmd) => cmd.toJSON());

const rest = new REST({ version: "10" }).setToken(token);

async function deploy() {
  try {
    console.log(`⏳ Registering ${commands.length} application (/) commands to Discord...`);

    await rest.put(Routes.applicationCommands(clientId as string), {
      body: commands,
    });

    console.log("✅ Successfully registered global Slash Commands!");
  } catch (error) {
    console.error("❌ Failed to register commands:", error);
  }
}

deploy();
