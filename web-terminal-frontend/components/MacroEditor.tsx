'use client';

interface MacroEditorProps {
  macroName: string;
  onMacroNameChange: (value: string) => void;
  macroScript: string;
  onMacroScriptChange: (value: string) => void;
  macroSendEnter: boolean;
  onMacroSendEnterChange: (value: boolean) => void;
  isEditing: boolean;
  onCancel: () => void;
  onSave: () => void;
}

/**
 * The macro create/edit form plus its template-command help panel, split
 * out of `OnscreenKeyboard` (QA-118). Pure UI — all macro state lives in the
 * parent (`OnscreenKeyboard`, via `useStoredMacros`).
 */
export function MacroEditor({
  macroName,
  onMacroNameChange,
  macroScript,
  onMacroScriptChange,
  macroSendEnter,
  onMacroSendEnterChange,
  isEditing,
  onCancel,
  onSave,
}: MacroEditorProps) {
  return (
    <div className="flex gap-3">
      {/* Editor panel */}
      <div className="flex-1 space-y-2 p-3 bg-[#1f1f1f]/80 rounded-lg">
        <div className="flex items-center gap-2">
          <input
            type="text"
            value={macroName}
            onChange={(e) => onMacroNameChange(e.target.value)}
            placeholder="Macro name..."
            className="flex-1 px-2 py-1.5 rounded text-sm bg-[#2a2a2a] text-[#e0e0e0]
              border border-[#3a3a3a] focus:border-amber-500/50 focus:outline-none
              placeholder-[#606060]"
            maxLength={30}
          />
        </div>
        <textarea
          value={macroScript}
          onChange={(e) => onMacroScriptChange(e.target.value)}
          placeholder="Enter commands (one per line)..."
          className="w-full px-2 py-1.5 rounded text-sm bg-[#2a2a2a] text-[#e0e0e0]
            border border-[#3a3a3a] focus:border-amber-500/50 focus:outline-none
            placeholder-[#606060] resize-none font-mono"
          rows={5}
        />
        <div className="flex items-center justify-between">
          <label className="flex items-center gap-2 text-xs text-[#a0a0a0] cursor-pointer select-none">
            <input
              type="checkbox"
              checked={macroSendEnter}
              onChange={(e) => onMacroSendEnterChange(e.target.checked)}
              className="w-4 h-4 rounded border-[#3a3a3a] bg-[#2a2a2a] text-amber-500
                focus:ring-amber-500/50 focus:ring-offset-0"
            />
            Send Enter after each line
          </label>
          <div className="flex gap-2">
            <button
              onClick={onCancel}
              tabIndex={-1}
              className="px-3 py-1.5 rounded text-xs font-medium
                bg-[#2a2a2a]/80 text-[#a0a0a0] hover:bg-[#3a3a3a]/80 hover:text-[#e0e0e0]
                transition-colors"
            >
              Cancel
            </button>
            <button
              onClick={onSave}
              tabIndex={-1}
              disabled={!macroName.trim() || !macroScript.trim()}
              className="px-3 py-1.5 rounded text-xs font-medium
                bg-amber-600/80 text-white hover:bg-amber-500/80
                disabled:opacity-50 disabled:cursor-not-allowed
                transition-colors"
            >
              {isEditing ? 'Update' : 'Save'}
            </button>
          </div>
        </div>
      </div>

      {/* Help panel */}
      <div className="w-56 p-2 bg-[#1a1a1a]/80 rounded-lg border border-[#2a2a2a] text-[10px] text-[#808080]">
        <div className="text-[11px] text-[#a0a0a0] font-medium mb-1.5">Template Commands</div>
        <div className="space-y-0.5">
          <div><code className="text-amber-400">[[delay:N]]</code> Wait N seconds</div>
          <div><code className="text-amber-400">[[enter]]</code> Send Enter key</div>
          <div><code className="text-amber-400">[[tab]]</code> Send Tab key</div>
          <div><code className="text-amber-400">[[esc]]</code> Send Escape key</div>
          <div><code className="text-amber-400">[[space]]</code> Send Space</div>
          <div><code className="text-amber-400">[[ctrl+X]]</code> Send Ctrl+X</div>
          <div><code className="text-amber-400">[[shift+X]]</code> Send Shift+X</div>
          <div><code className="text-amber-400">[[ctrl+shift+X]]</code> Ctrl+Shift+X</div>
          <div><code className="text-amber-400">[[shift+tab]]</code> Reverse Tab</div>
          <div><code className="text-amber-400">[[shift+enter]]</code> Shift+Enter</div>
        </div>
      </div>
    </div>
  );
}
