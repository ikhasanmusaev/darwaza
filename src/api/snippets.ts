import { invoke } from "@tauri-apps/api/core";

export interface Snippet {
  id: number;
  name: string;
  command: string;
  description: string | null;
  sortOrder: number;
  /** Виден ли сниппет агенту (#220). Решение принимается на устройстве и не синхронизируется. */
  agentAccess: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface SnippetInput {
  name: string;
  command: string;
  description?: string | null;
  sortOrder?: number | null;
  agentAccess?: boolean;
}

export const snippetsApi = {
  list: () => invoke<Snippet[]>("snippets_list"),
  get: (id: number) => invoke<Snippet>("snippets_get", { id }),
  create: (data: SnippetInput) => invoke<Snippet>("snippets_create", { data }),
  update: (id: number, data: SnippetInput) =>
    invoke<Snippet>("snippets_update", { id, data }),
  remove: (id: number) => invoke<void>("snippets_remove", { id }),
};
