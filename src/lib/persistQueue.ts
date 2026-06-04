/**
 * Serialized persistence queue for editor region ops.
 *
 * The EditPage applies each edit to local state optimistically, then persists
 * the primitive ops (create / update / delete) to the backend. Each backend
 * `region_*` command independently takes the project lock, so if ops were fired
 * concurrently (un-awaited, in a plain loop) two commands touching the SAME
 * region id could reach the DB out of order — e.g. an `update(B)` from a later
 * edit winning the lock race before the `create(B)` from an earlier edit has
 * committed. `region_update` returns NotFound for a missing row, so that update
 * is silently lost while local state already shows it applied: the persisted DB
 * then diverges from the visible timeline and the edit vanishes on reload.
 *
 * This queue removes the race by guaranteeing strict FIFO: op N+1's IPC call is
 * only started after op N has settled (resolved OR rejected). Ordering is the
 * invariant — a failing op never lets a later op jump ahead of it — so the
 * persisted order always matches the order the user applied edits locally.
 *
 * Pure and dependency-free: the caller injects the per-op persist function, so
 * it's exercised in tests with a mock and used with real IPC in the EditPage.
 */

/** Persists one op; resolves on success, rejects on failure. */
export type PersistFn<Op> = (op: Op) => Promise<unknown>;

/** Called once per op that fails to persist, with the op and the error. */
export type OnError<Op> = (op: Op, error: unknown) => void;

/**
 * A FIFO queue that persists ops strictly in the order they were enqueued, one
 * at a time. Each op only begins after the previous one has fully settled, so
 * ops that target the same region id can never reach the backend out of order.
 */
export class PersistQueue<Op> {
  private chain: Promise<void> = Promise.resolve();

  constructor(
    private readonly persist: PersistFn<Op>,
    private readonly onError: OnError<Op>,
  ) {}

  /**
   * Enqueue ops for persistence. They run after everything already queued, and
   * in the given order relative to each other. Returns immediately (the caller
   * has already updated local state optimistically); failures surface through
   * `onError`, never as a rejected promise here.
   */
  enqueue(ops: readonly Op[]): void {
    for (const op of ops) {
      this.chain = this.chain.then(() =>
        this.persist(op).then(
          () => undefined,
          (error) => {
            this.onError(op, error);
          },
        ),
      );
    }
  }

  /**
   * Resolves once the queue has drained (every enqueued op has settled). Used by
   * tests; the live editor never needs to await the queue.
   */
  idle(): Promise<void> {
    return this.chain;
  }
}
