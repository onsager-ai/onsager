import { execFileSync, type ExecFileSyncOptions } from "node:child_process";
import { randomUUID } from "node:crypto";

const EXEC_OPTIONS: ExecFileSyncOptions = {
  encoding: "utf-8",
  timeout: 60_000,
  stdio: ["pipe", "pipe", "pipe"],
};

/**
 * Wrapper around the agent-browser CLI for deterministic e2e tests.
 * Each instance manages a single browser session against a target URL.
 */
export class Browser {
  private baseUrl: string;
  private session: string;

  constructor(baseUrl: string, session?: string) {
    this.baseUrl = baseUrl;
    this.session = session ?? `e2e-${randomUUID().slice(0, 8)}`;
  }

  private run(args: string[]): string {
    try {
      return execFileSync(
        "agent-browser",
        ["--session", this.session, ...args],
        EXEC_OPTIONS,
      ) as string;
    } catch (err) {
      const e = err as { stderr?: string; stdout?: string };
      throw new Error(
        `agent-browser command failed: agent-browser ${args.join(" ")}\nstderr: ${e.stderr ?? ""}\nstdout: ${e.stdout ?? ""}`,
      );
    }
  }

  open(path = "/"): string {
    return this.run(["open", `${this.baseUrl}${path}`]);
  }

  snapshot(): string {
    return this.run(["snapshot"]);
  }

  interactiveSnapshot(): string {
    return this.run(["snapshot", "-i"]);
  }

  click(ref: string): string {
    return this.run(["click", ref]);
  }

  fill(ref: string, value: string): string {
    return this.run(["fill", ref, value]);
  }

  screenshot(outputPath?: string): string {
    return outputPath
      ? this.run(["screenshot", "--output", outputPath])
      : this.run(["screenshot"]);
  }

  waitForText(text: string, timeoutMs = 10_000): string {
    return this.run(["wait", "--text", text, "--timeout", String(timeoutMs)]);
  }

  waitForUrl(pattern: string, timeoutMs = 10_000): string {
    return this.run(["wait", "--url", pattern, "--timeout", String(timeoutMs)]);
  }

  title(): string {
    return this.run(["get", "title"]).trim();
  }

  url(): string {
    return this.run(["get", "url"]).trim();
  }

  getText(ref?: string): string {
    return ref ? this.run(["get", "text", ref]) : this.run(["get", "text"]);
  }

  evaluate(js: string): string {
    return this.run(["eval", js]);
  }

  close(): string {
    return this.run(["close"]);
  }
}

/**
 * Create a Browser instance pointed at the running stack.
 * Defaults to ONSAGER_TEST_URL or http://localhost:3000 (the stiglab image).
 */
export function createBrowser(baseUrl?: string): Browser {
  const url =
    baseUrl ?? process.env.ONSAGER_TEST_URL ?? "http://localhost:3000";
  return new Browser(url);
}
