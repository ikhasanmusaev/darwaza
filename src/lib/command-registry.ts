import { computed } from "vue";
import type { Router } from "vue-router";
import type { LucideIcon } from "lucide-vue-next";
import {
  Server,
  Key,
  Plus,
  Settings,
  FolderKanban,
  Fingerprint,
  Globe,
  X,
  RotateCw,
  Keyboard,
  Terminal,
} from "lucide-vue-next";
import { useHostsStore } from "../stores/hosts";
import { useIdentitiesStore } from "../stores/identities";
import { useSessionsStore } from "../stores/sessions";
import { useSnippetsStore } from "../stores/snippets";
import { useCommandPaletteStore } from "../stores/command-palette";
import { setLocale } from "../i18n";

export interface CommandItem {
  id: string;
  label: string;
  hint?: string;
  icon: LucideIcon;
  section: "hosts" | "identities" | "snippets" | "actions";
  run: () => void | Promise<void>;
}

export function useCommandItems(router: Router): CommandItem[] {
  const hostsStore = useHostsStore();
  const identitiesStore = useIdentitiesStore();
  const sessionsStore = useSessionsStore();
  const snippetsStore = useSnippetsStore();
  const paletteStore = useCommandPaletteStore();

  const hostCommands = computed<CommandItem[]>(() =>
    hostsStore.hosts.map((h) => ({
      id: `host:${h.id}`,
      label: h.name,
      hint: `${h.username}@${h.hostname}:${h.port}`,
      icon: Server,
      section: "hosts" as const,
      run: () => {
        paletteStore.close();
        sessionsStore.open(h.id, h.name);
      },
    })),
  );

  const identityCommands = computed<CommandItem[]>(() =>
    identitiesStore.identities.map((i) => ({
      id: `identity:${i.id}`,
      label: i.label,
      hint: i.kind,
      icon: Key,
      section: "identities" as const,
      run: () => {
        paletteStore.close();
        // Keychain переехал на top-level /identities (#081); старый путь
        // остаётся только как redirect для сохранённых ссылок.
        router.push(`/identities/${i.id}/edit`);
      },
    })),
  );

  // Сниппеты в палитре имеют смысл только при живой сессии — иначе команда
  // выберется, но отправлять её будет некуда.
  const snippetCommands = computed<CommandItem[]>(() =>
    sessionsStore.activeSession?.status === "CONNECTED"
      ? snippetsStore.snippets.map((s) => ({
          id: `snippet:${s.id}`,
          label: s.name,
          hint: s.command,
          icon: Terminal,
          section: "snippets" as const,
          run: () => {
            paletteStore.close();
            snippetsStore.runOnActiveSession(s).catch(() => undefined);
          },
        }))
      : [],
  );

  const staticCommands: CommandItem[] = [
    {
      id: "action:new-host",
      label: "New host",
      hint: "Ctrl+K → new",
      icon: Plus,
      section: "actions",
      run: () => {
        paletteStore.close();
        router.push("/hosts/new");
      },
    },
    {
      id: "action:settings",
      label: "Open Settings",
      hint: "Ctrl+K → settings",
      icon: Settings,
      section: "actions",
      run: () => {
        paletteStore.close();
        router.push("/settings");
      },
    },
    {
      id: "action:groups",
      label: "Manage groups",
      icon: FolderKanban,
      section: "actions",
      run: () => {
        paletteStore.close();
        // GroupManageModal lives in HostsListPage; navigating there is the best we can do
        router.push("/hosts");
      },
    },
    {
      id: "action:known-hosts",
      label: "Manage known hosts",
      icon: Fingerprint,
      section: "actions",
      run: () => {
        paletteStore.close();
        router.push("/settings/known-hosts");
      },
    },
    {
      id: "action:lang-uz",
      label: "Switch language: Uzbek",
      icon: Globe,
      section: "actions",
      run: () => {
        setLocale("uz");
        paletteStore.close();
      },
    },
    {
      id: "action:lang-ru",
      label: "Switch language: Russian",
      icon: Globe,
      section: "actions",
      run: () => {
        setLocale("ru");
        paletteStore.close();
      },
    },
    {
      id: "action:lang-en",
      label: "Switch language: English",
      icon: Globe,
      section: "actions",
      run: () => {
        setLocale("en");
        paletteStore.close();
      },
    },
    {
      id: "action:close-tab",
      label: "Close active tab",
      icon: X,
      section: "actions",
      run: () => {
        const id = sessionsStore.activeSessionId;
        if (id) sessionsStore.close(id);
        paletteStore.close();
      },
    },
    {
      id: "action:reconnect",
      label: "Reconnect active session",
      icon: RotateCw,
      section: "actions",
      run: () => {
        const session = sessionsStore.activeSession;
        if (session) {
          sessionsStore
            .open(session.hostId, session.hostLabel)
            .then(() => {
              if (sessionsStore.activeSessionId)
                sessionsStore.close(sessionsStore.activeSessionId);
            })
            .catch(() => undefined);
        }
        paletteStore.close();
      },
    },
    {
      id: "action:shortcuts",
      label: "Show keyboard shortcuts",
      icon: Keyboard,
      section: "actions",
      run: () => {
        // TODO: open shortcuts help modal
        paletteStore.close();
      },
    },
  ];

  // Flatten all commands into a single array
  // Note: this is not reactive by itself; CommandPalette.vue will re-compute on input
  return [
    ...hostCommands.value,
    ...identityCommands.value,
    ...snippetCommands.value,
    ...staticCommands,
  ];
}

export function fuzzyFilter(items: CommandItem[], query: string): CommandItem[] {
  const q = query.trim().toLowerCase();
  if (!q) return items;
  const terms = q.split(/\s+/);
  return items.filter((item) => {
    const text = `${item.label} ${item.hint ?? ""}`.toLowerCase();
    return terms.every((t) => text.includes(t));
  });
}
