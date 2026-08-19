import type { BotCommand } from "./types";
import { playCommand } from "./play";
import { skipCommand } from "./skip";
import { pauseCommand } from "./pause";
import { resumeCommand } from "./resume";
import { stopCommand } from "./stop";
import { queueCommand } from "./queue";
import { nowPlayingCommand } from "./nowplaying";
import { loopCommand } from "./loop";
import { shuffleCommand } from "./shuffle";
import { clearCommand } from "./clear";
import { removeCommand } from "./remove";
import { helpCommand } from "./help";
import { pingCommand } from "./ping";

export const commands: BotCommand[] = [
  playCommand,
  skipCommand,
  pauseCommand,
  resumeCommand,
  stopCommand,
  queueCommand,
  nowPlayingCommand,
  loopCommand,
  shuffleCommand,
  clearCommand,
  removeCommand,
  helpCommand,
  pingCommand,
];

export const commandMap = new Map<string, BotCommand>();

for (const command of commands) {
  commandMap.set(command.name, command);
}

// Aliases
commandMap.set("next", skipCommand);
commandMap.set("leave", stopCommand);
commandMap.set("q", queueCommand);
commandMap.set("np", nowPlayingCommand);
commandMap.set("repeat", loopCommand);
commandMap.set("h", helpCommand);
commandMap.set("latency", pingCommand);
