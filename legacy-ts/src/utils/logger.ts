type LogLevel =
  | "INFO"
  | "WARN"
  | "ERROR"
  | "DEBUG";

function write(
  level: LogLevel,
  message: string,
  ...args: unknown[]
): void {
  const timestamp = new Date().toISOString();

  console.log(
    `[${timestamp}] [${level}] ${message}`,
    ...args,
  );
}

export const logger = {
  info(message: string, ...args: unknown[]): void {
    write("INFO", message, ...args);
  },

  warn(message: string, ...args: unknown[]): void {
    write("WARN", message, ...args);
  },

  error(message: string, ...args: unknown[]): void {
    write("ERROR", message, ...args);
  },

  debug(message: string, ...args: unknown[]): void {
    if (Bun.env.NODE_ENV === "development") {
      write("DEBUG", message, ...args);
    }
  },
};
