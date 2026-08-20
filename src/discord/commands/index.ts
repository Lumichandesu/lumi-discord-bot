import type { BotCommand } from "./types";
import { playCommand } from "./play";
import { pauseCommand } from "./pause";
import { resumeCommand } from "./resume";
import { skipCommand } from "./skip";
import { stopCommand } from "./stop";
import { queueCommand } from "./queue";
import { nowPlayingCommand } from "./nowplaying";
import { clearCommand } from "./clear";
import { loopCommand } from "./loop";
import { removeCommand } from "./remove";
import { shuffleCommand } from "./shuffle";
import { pingCommand } from "./ping";
import { lyricsCommand } from "./lyrics";
import { seekCommand } from "./seek";
import { helpCommand } from "./help";

export const commands: BotCommand[] = [
  playCommand,
  pauseCommand,
  resumeCommand,
  skipCommand,
  stopCommand,
  queueCommand,
  nowPlayingCommand,
  clearCommand,
  loopCommand,
  removeCommand,
  shuffleCommand,
  pingCommand,
  lyricsCommand,
  seekCommand,
  helpCommand,
];

export const commandMap = new Map<string, BotCommand>();

// Register main commands
for (const cmd of commands) {
  commandMap.set(cmd.name.toLowerCase(), cmd);
}

// Register shortcut aliases for quick typing
commandMap.set("p", playCommand);
commandMap.set("s", skipCommand);
commandMap.set("np", nowPlayingCommand);
commandMap.set("q", queueCommand);
commandMap.set("h", helpCommand);
commandMap.set("l", lyricsCommand);
