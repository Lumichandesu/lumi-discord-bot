import type { Message } from "discord.js";
import type { BotCommand } from "./types";
import { getPlayer, requireGuild, requireVoiceChannel, seek } from "../../music/player";
import { formatDuration } from "../components/playerEmbed";

export const seekCommand: BotCommand = {
  name: "seek",
  description: "Seek to a specific timestamp in the current track (e.g. \\seek 1:30 or \\seek 90)",
  async execute(message: Message, args: string[]) {
    const guild = requireGuild(message);
    requireVoiceChannel(message);

    const player = getPlayer(guild.id);
    if (!player || !player.queue.current) {
      await message.reply("❌ No track currently playing.");
      return;
    }

    const timeStr = args[0];
    if (!timeStr) {
      await message.reply("⏩ Usage: `\\seek <mm:ss | seconds>` (e.g. `\\seek 1:45`)");
      return;
    }

    let seconds = 0;
    if (timeStr.includes(":")) {
      const parts = timeStr.split(":").map(Number);
      if (parts.length === 2) {
        seconds = (parts[0] || 0) * 60 + (parts[1] || 0);
      } else if (parts.length === 3) {
        seconds = (parts[0] || 0) * 3600 + (parts[1] || 0) * 60 + (parts[2] || 0);
      }
    } else {
      seconds = parseInt(timeStr, 10);
    }

    if (isNaN(seconds) || seconds < 0) {
      await message.reply("❌ Invalid time format. Use `mm:ss` or seconds.");
      return;
    }

    const duration = player.queue.current.info.duration;
    if (duration > 0 && seconds >= duration) {
      await message.reply(`❌ Cannot seek past song duration (${formatDuration(duration)}).`);
      return;
    }

    const success = seek(guild.id, seconds);
    if (success) {
      await message.reply(`⏩ Jumped to \`${formatDuration(seconds)}\``);
    } else {
      await message.reply("❌ Failed to seek track.");
    }
  },
};
