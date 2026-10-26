'use client';

import { useRef } from 'react';
import type { UploadFile, UploadUiState } from '@/lib/pane-upload';

interface UploadSurfaceProps {
  /** The uploader's latest state (drives everything rendered). */
  state: UploadUiState;
  /** False while the socket is not open; the picker is disabled then. */
  enabled: boolean;
  onPickFile: (file: UploadFile) => void;
  onAbort: () => void;
  /** Clear a done/failed/rejected state back to idle. */
  onDismiss: () => void;
}

/**
 * The pane upload surface: a pill at the terminal's top-right that opens a
 * file picker, shows chunk progress while the upload streams, and offers
 * Abort until the upload settles. Errors and completions render in place
 * until dismissed.
 */
export function UploadSurface({ state, enabled, onPickFile, onAbort, onDismiss }: UploadSurfaceProps) {
  const inputRef = useRef<HTMLInputElement>(null);
  const active =
    state.phase === 'starting' || state.phase === 'uploading' || state.phase === 'settling';

  const percent =
    state.phase === 'uploading' && state.totalBytes > 0
      ? Math.min(100, Math.round((state.sentBytes / state.totalBytes) * 100))
      : null;

  const handlePick = (): void => {
    inputRef.current?.click();
  };

  const handleChange = (): void => {
    const input = inputRef.current;
    const file = input?.files?.[0];
    if (file && input) {
      onPickFile({
        name: file.name,
        size: file.size,
        bytes: () => file.arrayBuffer().then((buffer) => new Uint8Array(buffer)),
      });
    }
    if (input) input.value = '';
  };

  return (
    <div
      className="absolute right-2 top-2 z-10 flex flex-col items-end gap-1 text-xs"
      data-testid="upload-surface"
    >
      <input
        ref={inputRef}
        type="file"
        className="hidden"
        data-testid="upload-input"
        onChange={handleChange}
      />
      {active ? (
        <div className="flex items-center gap-2 rounded-md border border-neutral-700 bg-neutral-900/90 px-2 py-1">
          <span className="max-w-40 truncate text-neutral-200">
            {state.phase === 'uploading' && percent !== null
              ? `${state.filename} ${percent}%`
              : state.phase === 'starting'
                ? `starting ${state.filename}`
                : state.phase === 'settling'
                  ? `sealing ${state.filename}`
                  : ''}
          </span>
          {state.phase === 'uploading' && percent !== null ? (
            <span className="h-1 w-24 overflow-hidden rounded bg-neutral-700">
              <span
                className="block h-full bg-blue-500"
                style={{ width: `${percent}%` }}
              />
            </span>
          ) : null}
          <button
            type="button"
            data-testid="upload-abort"
            onClick={onAbort}
            className="rounded px-1.5 py-0.5 text-neutral-300 hover:bg-neutral-700"
          >
            Abort
          </button>
        </div>
      ) : null}
      {state.phase === 'done' ? (
        <div className="flex max-w-64 items-center gap-2 rounded-md border border-green-800 bg-neutral-900/90 px-2 py-1">
          <span className="truncate text-green-400" title={state.path}>
            Uploaded: {state.path}
          </span>
          <button
            type="button"
            data-testid="upload-dismiss"
            onClick={onDismiss}
            className="rounded px-1.5 py-0.5 text-neutral-300 hover:bg-neutral-700"
          >
            OK
          </button>
        </div>
      ) : null}
      {state.phase === 'failed' || state.phase === 'rejected' ? (
        <div className="flex max-w-72 items-center gap-2 rounded-md border border-red-800 bg-neutral-900/90 px-2 py-1">
          <span className="truncate text-red-400" title={state.message}>
            {state.message}
          </span>
          <button
            type="button"
            data-testid="upload-dismiss"
            onClick={onDismiss}
            className="rounded px-1.5 py-0.5 text-neutral-300 hover:bg-neutral-700"
          >
            OK
          </button>
        </div>
      ) : null}
      {state.phase === 'idle' && enabled ? (
        <button
          type="button"
          data-testid="upload-button"
          onClick={handlePick}
          className="rounded-md border border-neutral-700 bg-neutral-900/70 px-2 py-1 text-neutral-300 opacity-60 transition-opacity hover:opacity-100"
        >
          Upload
        </button>
      ) : null}
    </div>
  );
}
