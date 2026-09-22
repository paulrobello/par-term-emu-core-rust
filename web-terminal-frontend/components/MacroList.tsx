'use client';

import type { Macro } from '@/lib/use-stored-macros';

interface MacroListProps {
  macros: Macro[];
  playingMacro: string | null;
  onPlay: (macro: Macro) => void;
  onStop: () => void;
  onEdit: (macro: Macro) => void;
  onDelete: (id: string) => void;
  onNew: () => void;
}

/**
 * The "+ New" button plus the row of existing macro buttons (with their
 * hover edit/delete controls), split out of `OnscreenKeyboard` (QA-118).
 * Pure UI — all macro state lives in the parent.
 */
export function MacroList({ macros, playingMacro, onPlay, onStop, onEdit, onDelete, onNew }: MacroListProps) {
  return (
    <div>
      <div className="flex flex-wrap gap-1.5 justify-center items-center">
        {/* New macro button */}
        <button
          onClick={onNew}
          tabIndex={-1}
          className="h-9 px-3 rounded-md text-xs sm:text-sm font-medium
            select-none touch-manipulation transition-all duration-100
            bg-amber-600/20 text-amber-400 border border-amber-500/50
            hover:bg-amber-600/30 active:scale-95"
          title="Create new macro"
        >
          + New
        </button>

        {/* Existing user macro buttons */}
        {macros.map((macro) => (
          <div key={macro.id} className="relative group">
            <button
              onClick={() => (playingMacro === macro.id ? onStop() : onPlay(macro))}
              tabIndex={-1}
              className={`h-9 px-3 rounded-md text-xs sm:text-sm font-medium
                select-none touch-manipulation transition-all duration-100
                ${playingMacro === macro.id
                  ? 'bg-red-600/80 text-white border-red-400/50 animate-pulse'
                  : 'bg-[#252525]/90 text-amber-400 border-amber-500/30 hover:bg-[#353535]/90'
                }
                border active:scale-95`}
              title={playingMacro === macro.id ? 'Stop macro' : `Run: ${macro.script.split('\n')[0]}${macro.sendEnter === false ? '' : '...'}${macro.sendEnter === false ? ' (no enter)' : ''}`}
            >
              {playingMacro === macro.id ? (
                <span className="flex items-center gap-1">
                  <svg className="w-3 h-3" fill="currentColor" viewBox="0 0 24 24">
                    <rect x="6" y="6" width="12" height="12" />
                  </svg>
                  Stop
                </span>
              ) : (
                <span className="flex items-center gap-1">
                  {macro.sendEnter !== false && (
                    <svg className="w-3 h-3" fill="currentColor" viewBox="0 0 24 24">
                      <polygon points="5,3 19,12 5,21" />
                    </svg>
                  )}
                  {macro.name}
                </span>
              )}
            </button>

            {/* Edit/Delete dropdown on hover */}
            {playingMacro !== macro.id && (
              <div className="absolute right-0 bottom-full pb-1 hidden group-hover:flex z-10">
                {/* Invisible bridge to prevent hover gap */}
                <div className="flex bg-[#2a2a2a] rounded shadow-lg border border-[#3a3a3a] overflow-hidden">
                  <button
                    onClick={(e) => { e.stopPropagation(); onEdit(macro); }}
                    tabIndex={-1}
                    className="px-2 py-1 text-xs text-[#a0a0a0] hover:bg-[#3a3a3a] hover:text-[#e0e0e0]"
                    title="Edit macro"
                  >
                    Edit
                  </button>
                  <button
                    onClick={(e) => { e.stopPropagation(); onDelete(macro.id); }}
                    tabIndex={-1}
                    className="px-2 py-1 text-xs text-red-400 hover:bg-red-500/20"
                    title="Delete macro"
                  >
                    Del
                  </button>
                </div>
              </div>
            )}
          </div>
        ))}

        {macros.length === 0 && (
          <span className="text-[10px] text-[#606060]">
            No macros yet. Click &quot;+ New&quot; to create one.
          </span>
        )}
      </div>
    </div>
  );
}
