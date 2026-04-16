import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["product/**/*.test.ts"],
    testTimeout: 180_000, // 3 min — real Claude sessions
    hookTimeout: 30_000,
    pool: "forks",
    fileParallelism: false, // sequential to avoid overwhelming the agent
  },
});
