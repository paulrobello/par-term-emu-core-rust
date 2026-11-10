import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    // Only the extensions harness. mods/agent-state/hooks/register.test.ts
    // runs under `claude plugin test`, whose host provides the
    // 'claude-code/testing' module no node toolchain here has.
    include: ["extensions/**/*.test.ts"],
  },
});
