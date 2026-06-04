import { describe, expect, it } from "vitest";

import { PersistQueue } from "@/lib/persistQueue";

/**
 * Regression for the editor op-ordering bug: the EditPage applied edits to local
 * state optimistically and fired each region IPC un-awaited in a plain loop. Two
 * edits touching the same region id could then reach the backend out of order —
 * an `update(B)` from a later edit beating the `create(B)` from an earlier one,
 * which the backend rejects with NotFound. The error was swallowed and local
 * state already showed the edit applied, so the DB silently diverged from the
 * visible timeline (the edit vanished on reload).
 *
 * These tests model that exact race with a tiny in-memory backend whose `create`
 * is slower than its `update` (so an un-sequenced update wins the lock race) and
 * whose `update` of a missing id throws NotFound — and prove the PersistQueue
 * removes the divergence by persisting ops strictly in order.
 */

type Op = { kind: "create"; id: string } | { kind: "update"; id: string };

/** A backend where update-before-create fails, and create is the slower call. */
function makeBackend() {
  const rows = new Set<string>();
  const persisted: string[] = []; // order ops actually committed
  const errors: string[] = [];

  const persist = async (op: Op): Promise<void> => {
    if (op.kind === "create") {
      // Create is the slower path (simulates losing the project-lock race).
      await delay(20);
      rows.add(op.id);
      persisted.push(`create:${op.id}`);
      return;
    }
    // update: fast, but a missing row is a NotFound the UI would swallow.
    await delay(1);
    if (!rows.has(op.id)) {
      errors.push(`NotFound:${op.id}`);
      throw new Error(`NotFound: ${op.id}`);
    }
    persisted.push(`update:${op.id}`);
  };

  return { persist, persisted, errors };
}

function delay(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

describe("PersistQueue", () => {
  it("demonstrates the un-sequenced fire-and-forget race it fixes", async () => {
    // The OLD behaviour: fire each op without awaiting, in order. Because
    // create is slower, the update runs first against an empty backend.
    const { persist, persisted, errors } = makeBackend();
    const swallowed: unknown[] = [];

    const fireAndForget = (ops: Op[]) => {
      for (const op of ops) persist(op).catch((e) => swallowed.push(e));
    };
    // run #1 creates B; run #2 immediately trims (updates) B.
    fireAndForget([{ kind: "create", id: "B" }]);
    fireAndForget([{ kind: "update", id: "B" }]);

    await delay(40);
    // The update raced ahead and hit NotFound — and only the create committed,
    // so the persisted DB does NOT reflect the trim: silent divergence.
    expect(errors).toContain("NotFound:B");
    expect(persisted).toEqual(["create:B"]);
    expect(swallowed).toHaveLength(1);
  });

  it("persists ops strictly in order so the same-id race cannot happen", async () => {
    const { persist, persisted, errors } = makeBackend();
    const seen: Array<{ op: Op; error: unknown }> = [];
    const queue = new PersistQueue<Op>(persist, (op, error) =>
      seen.push({ op, error }),
    );

    // Same scenario: create B, then immediately update B — but via the queue.
    queue.enqueue([{ kind: "create", id: "B" }]);
    queue.enqueue([{ kind: "update", id: "B" }]);
    await queue.idle();

    // create committed first, THEN update — no NotFound, no divergence.
    expect(errors).toHaveLength(0);
    expect(seen).toHaveLength(0);
    expect(persisted).toEqual(["create:B", "update:B"]);
  });

  it("a failing op does not let later ops jump ahead of it", async () => {
    const order: string[] = [];
    const queue = new PersistQueue<Op>(
      async (op) => {
        if (op.kind === "create" && op.id === "X") {
          throw new Error("boom");
        }
        order.push(`${op.kind}:${op.id}`);
      },
      () => {
        order.push("error");
      },
    );

    queue.enqueue([{ kind: "create", id: "X" }]); // fails
    queue.enqueue([{ kind: "create", id: "Y" }]); // must still run, after
    await queue.idle();

    // The failure is observed, then the next op runs — FIFO is preserved.
    expect(order).toEqual(["error", "create:Y"]);
  });
});
