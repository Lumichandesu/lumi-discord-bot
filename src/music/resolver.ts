import type { Provider } from "./types";

export function detectProvider(input: string): Provider | null {
  let url: URL;

  try {
    url = new URL(input);
  } catch {
    return null;
  }

  const host = url.hostname.toLowerCase();

  if (
    host === "youtube.com" ||
    host === "www.youtube.com" ||
    host === "youtu.be" ||
    host === "music.youtube.com"
  ) {
    return "youtube";
  }

  if (
    host === "soundcloud.com" ||
    host === "www.soundcloud.com" ||
    host === "on.soundcloud.com"
  ) {
    return "soundcloud";
  }

  if (
    host === "open.spotify.com" ||
    host === "spotify.link"
  ) {
    return "spotify";
  }

  if (
    host === "music.apple.com" ||
    host === "apple.co"
  ) {
    return "applemusic";
  }

  if (
    host === "tidal.com" ||
    host === "www.tidal.com" ||
    host === "listen.tidal.com"
  ) {
    return "tidal";
  }

  return null;
}
