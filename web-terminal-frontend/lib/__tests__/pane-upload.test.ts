/**
 * PaneUploader tests: the chunk loop's exact frame shapes and the
 * begin/chunk/commit/abort sequencing against the daemon's contract
 * (docs/MUX.md "Client uploads"): chunks ride the socket back-to-back once
 * staging is confirmed (silence is progress — no per-chunk acks), commit
 * follows the final chunk, and abort ends the upload locally at once.
 */

import { describe, it, expect, vi } from 'vitest';
import { PaneUploader } from '@/lib/pane-upload';
import type { PaneUploaderCallbacks, UploadFile, UploadUiState } from '@/lib/pane-upload';
import type { ClientMessage } from '@/lib/proto/terminal_pb';

const KIB = 1024;

/** Capture send calls and state changes; a no-op yield keeps streams synchronous. */
function makeUploader(): {
  uploader: PaneUploader;
  sent: ClientMessage[];
  states: UploadUiState[];
  cases: () => (string | undefined)[];
} {
  const sent: ClientMessage[] = [];
  const states: UploadUiState[] = [];
  const callbacks: PaneUploaderCallbacks = {
    send: (msg) => sent.push(msg),
    onState: (state) => states.push(state),
    yieldControl: async () => {},
  };
  const uploader = new PaneUploader(callbacks);
  return {
    uploader,
    sent,
    states,
    cases: () => sent.map((m) => m.message.case),
  };
}

/** A synthetic file of `size` bytes; content is deterministic filler. */
function makeFile(name: string, size: number): UploadFile {
  const bytes = new Uint8Array(size);
  for (let i = 0; i < size; i += 1) bytes[i] = i % 251;
  return { name, size, bytes: async () => bytes };
}

const chunkSizes = (sent: ClientMessage[]): number[] =>
  sent
    .filter((m) => m.message.case === 'uploadChunk')
    .map((m) => (m.message.value as { data: Uint8Array }).data.length);

const casesOf = (sent: ClientMessage[]): (string | undefined)[] =>
  sent.map((m) => m.message.case);

/** Drain every pending microtask and timer tick (one macrotask). */
const flush = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0));

describe('PaneUploader', () => {
  it('rejects an over-cap file with zero network traffic', () => {
    const { uploader, sent, states } = makeUploader();
    uploader.start(makeFile('big.bin', 100 * KIB * KIB + 1));
    expect(states.at(-1)).toMatchObject({ phase: 'rejected' });
    expect(sent).toHaveLength(0);
    expect(uploader.isActive()).toBe(false);
  });

  it('streams exact chunk sizes including the final partial chunk, then commits', async () => {
    const { uploader, sent, cases } = makeUploader();
    uploader.start(makeFile('report.bin', 150 * KIB));
    uploader.handleStarted(7n);
    await flush();
    expect(cases()).toEqual([
      'uploadBegin',
      'uploadChunk',
      'uploadChunk',
      'uploadChunk',
      'uploadCommit',
    ]);
    expect(chunkSizes(sent)).toEqual([64 * KIB, 64 * KIB, 22 * KIB]);
    const begin = sent[0].message.value as { filename: string; totalBytes: bigint };
    expect(begin.filename).toBe('report.bin');
    expect(begin.totalBytes).toBe(150n * BigInt(KIB));
  });

  it('sends one full-size chunk for an exactly-one-chunk file', async () => {
    const { uploader, sent } = makeUploader();
    uploader.start(makeFile('one.bin', 64 * KIB));
    uploader.handleStarted(1n);
    await flush();
    expect(chunkSizes(sent)).toEqual([64 * KIB]);
    expect(sent.at(-1)?.message.case).toBe('uploadCommit');
  });

  it('commits a zero-byte file without any chunks', async () => {
    const { uploader, sent, cases } = makeUploader();
    uploader.start(makeFile('empty.bin', 0));
    uploader.handleStarted(2n);
    await flush();
    expect(cases()).toEqual(['uploadBegin', 'uploadCommit']);
    const begin = sent[0].message.value as { totalBytes: bigint };
    expect(begin.totalBytes).toBe(0n);
  });

  it('gates chunks on upload_started: nothing streams before the ack', async () => {
    const { uploader, sent, states, cases } = makeUploader();
    uploader.start(makeFile('gated.bin', 80 * KIB));
    await flush();
    expect(cases()).toEqual(['uploadBegin']);
    expect(states.at(-1)).toMatchObject({ phase: 'starting' });
    uploader.handleStarted(9n);
    await flush();
    expect(cases()).toEqual([
      'uploadBegin',
      'uploadChunk',
      'uploadChunk',
      'uploadCommit',
    ]);
    expect(chunkSizes(sent)).toEqual([64 * KIB, 16 * KIB]);
  });

  it('abort mid-stream stops further chunks and ends the upload at once', async () => {
    const sent: ClientMessage[] = [];
    const states: UploadUiState[] = [];
    let abortRef: (() => void) | null = null;
    const uploader = new PaneUploader({
      send: (msg) => sent.push(msg),
      onState: (state) => states.push(state),
      // Abort from inside the stream, after the first chunk's yield.
      yieldControl: async () => {
        abortRef?.();
        abortRef = null;
      },
    });
    abortRef = () => uploader.abort();
    uploader.start(makeFile('aborted.bin', 200 * KIB));
    uploader.handleStarted(4n);
    await flush();
    expect(chunkSizes(sent)).toEqual([64 * KIB]);
    expect(casesOf(sent)).toEqual([
      'uploadBegin',
      'uploadChunk',
      'uploadAbort',
    ]);
    expect(states.at(-1)).toMatchObject({ phase: 'failed', message: 'upload aborted' });
    expect(uploader.isActive()).toBe(false);
  });

  it('a begin refusal (upload_finished before started) fails without chunks', async () => {
    const { uploader, sent, states, cases } = makeUploader();
    uploader.start(makeFile('refused.bin', 10 * KIB));
    uploader.handleFinished({ uploadId: 0n, path: undefined, error: 'no pipe' });
    uploader.handleStarted(5n); // late ack of a settled upload: ignored
    await flush();
    expect(cases()).toEqual(['uploadBegin']);
    expect(states.at(-1)).toMatchObject({ phase: 'failed', message: 'no pipe' });
    expect(uploader.isActive()).toBe(false);
  });

  it('a stray upload_finished with a mismatched id is ignored', async () => {
    const { uploader, sent, states, cases } = makeUploader();
    uploader.start(makeFile('stray.bin', 10 * KIB));
    uploader.handleStarted(5n);
    uploader.handleFinished({ uploadId: 9n, path: '/tmp/x', error: undefined });
    await flush();
    expect(cases()).toEqual(['uploadBegin', 'uploadChunk', 'uploadCommit']);
    expect(states.at(-1)).toMatchObject({ phase: 'settling' });
    uploader.handleFinished({ uploadId: 5n, path: '/tmp/stray.bin', error: undefined });
    expect(states.at(-1)).toMatchObject({ phase: 'done', path: '/tmp/stray.bin' });
    expect(uploader.isActive()).toBe(false);
  });

  it('starting a second upload while one is active is rejected', async () => {
    const { uploader, sent, states, cases } = makeUploader();
    uploader.start(makeFile('first.bin', 10 * KIB));
    uploader.start(makeFile('second.bin', 10 * KIB));
    uploader.handleStarted(3n);
    await flush();
    // The first upload still runs to completion; the second pick was
    // refused with a visible message and produced no second begin.
    expect(cases()).toEqual(['uploadBegin', 'uploadChunk', 'uploadCommit']);
    expect(states).toContainEqual({
      phase: 'rejected',
      message: 'An upload of first.bin is already in progress',
    });
    expect(casesOf(sent).filter((c) => c === 'uploadBegin')).toHaveLength(1);
  });

  it('connectionLost fails the upload without any further traffic', async () => {
    const { uploader, sent, states, cases } = makeUploader();
    uploader.start(makeFile('lost.bin', 80 * KIB));
    uploader.handleStarted(6n);
    await flush();
    expect(cases()).toEqual(['uploadBegin', 'uploadChunk', 'uploadChunk', 'uploadCommit']);
    // A second upload whose socket dies right after the begin.
    const second = makeUploader();
    second.uploader.start(makeFile('lost2.bin', 80 * KIB));
    second.uploader.connectionLost();
    await flush();
    expect(second.cases()).toEqual(['uploadBegin']);
    expect(second.states.at(-1)).toMatchObject({
      phase: 'failed',
      message: 'connection closed during upload',
    });
  });

  it('a read failure aborts the daemon staging slot and fails locally', async () => {
    const sent: ClientMessage[] = [];
    const states: UploadUiState[] = [];
    const uploader = new PaneUploader({
      send: (msg) => sent.push(msg),
      onState: (state) => states.push(state),
      yieldControl: async () => {},
    });
    uploader.start({
      name: 'unreadable.bin',
      size: 1024,
      bytes: async () => {
        throw new Error('disk gone');
      },
    });
    await flush();
    expect(casesOf(sent)).toEqual(['uploadBegin', 'uploadAbort']);
    expect(states.at(-1)).toMatchObject({
      phase: 'failed',
      message: 'could not read file: disk gone',
    });
  });

  it('dismiss returns a settled state to idle', () => {
    const { uploader, states } = makeUploader();
    uploader.start(makeFile('ok.bin', 1024));
    uploader.handleFinished({ uploadId: 0n, path: '/tmp/ok.bin', error: undefined });
    uploader.dismiss();
    expect(states.at(-1)).toMatchObject({ phase: 'idle' });
    expect(uploader.isActive()).toBe(false);
  });
});
