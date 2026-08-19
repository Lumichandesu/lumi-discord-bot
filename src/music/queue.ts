import {
  getPlayer,
} from "./player";

export function getQueue(
  guildId: string,
) {
  const player =
    getPlayer(
      guildId,
    );

  if (!player) {
    return null;
  }

  return player.queue;
}

export function getCurrent(
  guildId: string,
) {
  return (
    getQueue(guildId)
      ?.current
    ?? null
  );
}

export function getItems(
  guildId: string,
) {
  return (
    getQueue(guildId)
      ?.tracks
    ?? []
  );
}
