import type { EditSession, ItemType } from "../backend";
import { createStore } from "./store";

/** What the item editor is currently doing. `new` starts with the type picker unless a type is given. */
export type EditorRequest =
  | { mode: "new"; type: ItemType | null }
  | { mode: "edit"; id: string; session: EditSession };

export interface UiState {
  editor: EditorRequest | null;
}

export const ui = createStore<UiState>({ editor: null });

export const openNewItem = (type: ItemType | null = null) => ui.set((s) => ({ ...s, editor: { mode: "new", type } }));
export const openEditor = (id: string, session: EditSession) => ui.set((s) => ({ ...s, editor: { mode: "edit", id, session } }));
export const closeEditor = () => ui.set((s) => ({ ...s, editor: null }));
