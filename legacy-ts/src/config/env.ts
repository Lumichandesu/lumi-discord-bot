import { z } from "zod";

const envSchema = z.object({
  DISCORD_TOKEN: z.string().min(1),
  DISCORD_CLIENT_ID: z.string().min(1),
  DISCORD_GUILD_ID: z.string().min(1).optional(),
});

export const env = envSchema.parse({
  DISCORD_TOKEN: Bun.env.DISCORD_TOKEN,
  DISCORD_CLIENT_ID: Bun.env.DISCORD_CLIENT_ID,
  DISCORD_GUILD_ID: Bun.env.DISCORD_GUILD_ID,
});
