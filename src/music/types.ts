export type Provider = "youtube" | "soundcloud" | "spotify" | "applemusic" | "tidal" | "search";

export interface ResolvedTrack {
  provider: Provider;
  sourceUrl: string;
  streamUrl: string;
  title: string;
  uploader: string;
  duration: number | null;
  thumbnail: string | null;
}

export interface QueueTrack extends ResolvedTrack {
  requester: string;
}
