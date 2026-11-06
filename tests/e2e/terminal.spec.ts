import { test } from "@playwright/test";

test.describe("S4-S5 — Connect + send command", () => {
  test.skip("requires live sshd; UI-only test TODO", () => {
    // Full integration test lives in .claude/agents/ssh-tester.md
    // UI-only variant: verify clicking Connect opens a tab with CONNECTING status
  });
});

test.describe("S6-S7 — Multiple tabs + close", () => {
  test.skip("requires live sshd; UI-only test TODO", () => {
    // Full integration test lives in .claude/agents/ssh-tester.md
  });
});
