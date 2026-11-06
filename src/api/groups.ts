import { invoke } from "@tauri-apps/api/core";

export interface Group {
  id: number;
  name: string;
  parentId: number | null;
  sortOrder: number;
  createdAt: string;
  updatedAt: string;
}

export interface GroupInput {
  name: string;
  parentId?: number | null;
  sortOrder?: number;
}

export const groupsApi = {
  list: () => invoke<Group[]>("groups_list"),
  create: (data: GroupInput) => invoke<Group>("groups_create", { data }),
  update: (id: number, data: GroupInput) =>
    invoke<Group>("groups_update", { id, data }),
  remove: (id: number) => invoke<void>("groups_remove", { id }),
};
