/**
 * Framework-free pane-upload controller (the upload half of QA-008's
 * connection/upload split).
 *
 * Drives the streaming protocol's chunked client upload (docs/MUX.md
 * "Client uploads"): upload_begin -> upload_started (the daemon's staging
 * id) -> upload_chunk* -> upload_commit. Chunk and commit frames ride the
 * socket back-to-back once staging is confirmed - the daemon's single
 * upload worker serializes them, and silence is progress (no per-chunk
 * acks exist on the wire). upload_abort cancels mid-flight; both endings
 * arrive as upload_finished (path set = committed, error set = failed or
 * aborted).
 *
 * Client-side caps mirror the daemon's (crates/par-mux/src/mux/uploads.rs):
 * one chunk at most MAX_UPLOAD_CHUNK_BYTES and a file total at most
 * MAX_UPLOAD_TOTAL_BYTES. A file over the total cap is rejected before any
 * traffic leaves the client.
 */

import {
  createUploadBeginMessage,
  createUploadChunkMessage,
  createUploadCommitMessage,
  createUploadAbortMessage,
} from './protocol';
import type { ClientMessage } from './proto/terminal_pb';

/** Bytes accepted in one upload_chunk frame; the daemon + wire cap. */
export const MAX_UPLOAD_CHUNK_BYTES = 64 * 1024;

/** The largest file an upload may claim; the daemon's staging cap. */
export const MAX_UPLOAD_TOTAL_BYTES = 100 * 1024 * 1024;

/**
 * The file-shaped surface PaneUploader needs; a browser File satisfies it
 * via `{ name, size, bytes: () => file.arrayBuffer().then(b => new Uint8Array(b)) }`.
 */
export interface UploadFile {
  name: string;
  size: number;
  bytes: () => Promise<Uint8Array>;
}

/** Everything the upload UI renders. */
export type UploadUiState =
  | { phase: 'idle' }
  | { phase: 'rejected'; message: string }
  | { phase: 'starting'; filename: string; totalBytes: number }
  | { phase: 'uploading'; filename: string; sentBytes: number; totalBytes: number }
  | { phase: 'settling'; filename: string; totalBytes: number }
  | { phase: 'done'; filename: string; path: string }
  | { phase: 'failed'; filename: string; message: string };

/**
 * The settlement fields PaneUploader consumes from an upload_finished
 * (structurally satisfied by the generated UploadFinished message).
 */
export interface UploadSettlement {
  uploadId: bigint;
  path?: string | undefined;
  error?: string | undefined;
}

export interface PaneUploaderCallbacks {
  /** Send one client message over the connection (e.g. TerminalConnection.send). */
  send(msg: ClientMessage): void;
  /** UI state changes (the only output the component renders). */
  onState(state: UploadUiState): void;
  /**
   * Called between chunk sends so the browser can paint progress; defaults
   * to a macrotask. Tests pass an immediately-resolved promise.
   */
  yieldControl?(): Promise<void>;
}

/** Default inter-chunk yield: one macrotask, so the UI paints progress. */
const defaultYield = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0));

/**
 * One client upload at a time, sequenced against the daemon's replies.
 * All daemon-side state is session-scoped (no ids ride chunk/commit/abort),
 * so the id from upload_started is only used to match this upload's
 * upload_finished and ignore strays from an earlier session peer.
 */
export class PaneUploader {
  private active = false;
  private uploadId: bigint | null = null;
  private filename = '';
  private totalBytes = 0;
  private sentBytes = 0;
  private staged: Uint8Array | null = null;
  private startAcked = false;

  constructor(private readonly callbacks: PaneUploaderCallbacks) {}

  /** Whether an upload is between an accepted start and its settlement. */
  isActive(): boolean {
    return this.active;
  }

  /** Clear a terminal state (done/failed/rejected) back to idle. */
  dismiss(): void {
    if (this.active) return;
    this.callbacks.onState({ phase: 'idle' });
  }

  /**
   * Begin uploading `file`. Over-cap files are rejected with a visible
   * message and zero network traffic. One upload runs at a time.
   */
  start(file: UploadFile): void {
    if (this.active) {
      this.callbacks.onState({
        phase: 'rejected',
        message: `An upload of ${this.filename} is already in progress`,
      });
      return;
    }
    if (file.size > MAX_UPLOAD_TOTAL_BYTES) {
      this.callbacks.onState({
        phase: 'rejected',
        message: `${file.name} is ${file.size} bytes; the upload cap is ${MAX_UPLOAD_TOTAL_BYTES} bytes. Nothing was sent.`,
      });
      return;
    }
    this.active = true;
    this.uploadId = null;
    this.startAcked = false;
    this.filename = file.name;
    this.totalBytes = file.size;
    this.sentBytes = 0;
    this.staged = null;
    this.callbacks.onState({
      phase: 'starting',
      filename: this.filename,
      totalBytes: this.totalBytes,
    });
    this.callbacks.send(
      createUploadBeginMessage(this.filename, this.totalBytes),
    );
    void file.bytes().then(
      (bytes) => this.onBytesLoaded(bytes),
      (err: unknown) => this.onBytesFailed(err),
    );
  }

  /** Staging opened: remember the daemon id, stream once bytes are in hand. */
  handleStarted(uploadId: bigint): void {
    if (!this.active) return;
    this.uploadId = uploadId;
    this.startAcked = true;
    if (this.staged) {
      const data = this.staged;
      this.staged = null;
      void this.streamStaged(data);
    }
  }

  /** An upload settled: committed (path set) or failed/aborted (error set). */
  handleFinished(finished: UploadSettlement): void {
    if (!this.active) return;
    // A refusal of the begin (id 0) can arrive before any upload_started;
    // afterwards, only the finished frame matching our id settles us.
    if (this.uploadId !== null && finished.uploadId !== this.uploadId) return;
    this.active = false;
    this.staged = null;
    this.startAcked = false;
    if (finished.path !== undefined) {
      this.callbacks.onState({
        phase: 'done',
        filename: this.filename,
        path: finished.path,
      });
    } else {
      this.callbacks.onState({
        phase: 'failed',
        filename: this.filename,
        message: finished.error ?? 'upload failed',
      });
    }
  }

  /**
   * Cancel the in-progress upload: the abort frame goes out and the upload
   * ends locally at once (the daemon's matching upload_finished is then a
   * stray and is ignored). Nothing further is streamed after an abort.
   */
  abort(): void {
    if (!this.active) return;
    this.active = false;
    this.staged = null;
    this.callbacks.send(createUploadAbortMessage());
    this.callbacks.onState({
      phase: 'failed',
      filename: this.filename,
      message: 'upload aborted',
    });
  }

  /**
   * The socket dropped (or errored) mid-upload: fail the upload locally.
   * Nothing further can be sent, and the daemon's idle timeout reclaims
   * the staging slot.
   */
  connectionLost(): void {
    if (!this.active) return;
    this.active = false;
    this.staged = null;
    this.callbacks.onState({
      phase: 'failed',
      filename: this.filename,
      message: 'connection closed during upload',
    });
  }

  private onBytesLoaded(bytes: Uint8Array): void {
    if (!this.active) return;
    this.staged = bytes;
    if (this.startAcked) {
      this.staged = null;
      void this.streamStaged(bytes);
    }
  }

  private onBytesFailed(err: unknown): void {
    if (!this.active) return;
    // The begin already went out, so the daemon may hold a staging slot;
    // abort it rather than let it sit out the idle timeout.
    this.callbacks.send(createUploadAbortMessage());
    this.active = false;
    this.staged = null;
    this.callbacks.onState({
      phase: 'failed',
      filename: this.filename,
      message: `could not read file: ${err instanceof Error ? err.message : String(err)}`,
    });
  }

  /**
   * Push the staged bytes as capped chunks, then commit. Chunks ride the
   * socket back-to-back (the daemon serializes them through its single
   * upload worker); the yield between sends lets the browser paint the
   * progress the onState calls describe.
   */
  private async streamStaged(data: Uint8Array): Promise<void> {
    const yieldFn = this.callbacks.yieldControl ?? defaultYield;
    for (let offset = 0; offset < data.length; offset += MAX_UPLOAD_CHUNK_BYTES) {
      if (!this.active) return; // aborted/failed mid-stream: stop sending
      const chunk = data.subarray(offset, Math.min(offset + MAX_UPLOAD_CHUNK_BYTES, data.length));
      this.callbacks.send(createUploadChunkMessage(chunk));
      this.sentBytes = offset + chunk.length;
      this.callbacks.onState({
        phase: 'uploading',
        filename: this.filename,
        sentBytes: this.sentBytes,
        totalBytes: this.totalBytes,
      });
      await yieldFn();
    }
    if (!this.active) return;
    this.callbacks.send(createUploadCommitMessage());
    this.callbacks.onState({
      phase: 'settling',
      filename: this.filename,
      totalBytes: this.totalBytes,
    });
  }
}
