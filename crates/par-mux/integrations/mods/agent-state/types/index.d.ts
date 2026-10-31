export type PushedState = { state: string; message: string }

declare module 'claude-code' {
  interface PluginState {
    'agent-state': {
      /** The last (state, message) pair this mod pushed to par-mux; null before the first push. */
      lastPushed: PushedState | null;
      /** The most recent par-mux report error, or null. */
      lastError: string | null;
    };
  }
}
