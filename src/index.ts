import { Client, GatewayIntentBits, Events } from "discord.js";
import { env } from "./config/env";
import { startApiServer } from "./server";
import { registerMessageHandler } from "./discord/events/messages";
import { commandMap } from "./discord/commands";

const client = new Client({
  intents: [
    GatewayIntentBits.Guilds,
    GatewayIntentBits.GuildVoiceStates,
    GatewayIntentBits.GuildMessages,
    GatewayIntentBits.MessageContent,
  ],
});

// Register existing prefix command handler (\play, etc.)
registerMessageHandler(client);

// Slash command handler (/play, etc.)
client.on(Events.InteractionCreate, async (interaction) => {
  if (!interaction.isChatInputCommand()) return;

  const command = commandMap.get(interaction.commandName);
  
  if (!command) {
    await interaction.reply({ content: "❌ Command not found.", ephemeral: true });
    return;
  }

  try {
    // Check voice channel requirement
    const member = interaction.member;
    if (!interaction.guildId || !member || !("voice" in member) || !member.voice.channel) {
      await interaction.reply({ content: "❌ You must be in a Voice Channel to use this command.", ephemeral: true });
      return;
    }

    // Convert Slash Command to Fake Message for legacy compatibility
    let args: string[] = [];
    const query = interaction.options.getString("query");
    if (query) args = query.split(" ");
    const mode = interaction.options.getString("mode");
    if (mode) args = [mode];
    
    // Acknowledge interaction (prevents timeout error)
    await interaction.deferReply();

    const fakeMessage = {
      guild: interaction.guild,
      member: interaction.member,
      channel: interaction.channel,
      author: interaction.user,
      reply: async (payload: any) => interaction.followUp(payload),
    } as any;

    await command.execute(fakeMessage, args);
    
  } catch (error) {
    console.error(`[SlashCommand Error - ${interaction.commandName}]`, error);
    if (interaction.deferred || interaction.replied) {
      await interaction.followUp({ content: `❌ Error executing command.` });
    } else {
      await interaction.reply({ content: `❌ Error executing command.` });
    }
  }
});

client.once(Events.ClientReady, () => {
  console.log(`[INFO] Logged in as ${client.user?.tag}`);
  console.log(`[INFO] Bot ID: ${client.user?.id}`);
  console.log(`[INFO] Guild count: ${client.guilds.cache.size}`);

  startApiServer(client, 3000);
  console.log("[INFO] Lumi Bot is ready.");
});

client.login(env.DISCORD_TOKEN);
