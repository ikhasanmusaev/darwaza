import type { ITheme } from "xterm";

export interface TerminalThemeDef {
  id: string;
  label: string;
  theme: ITheme;
}

export const TERMINAL_THEMES: TerminalThemeDef[] = [
  {
    id: "dark-default",
    label: "Dark Default",
    theme: {
      background: "#101216",
      foreground: "#BFC5BA",
      cursor: "#10b981",
      selectionBackground: "#3f3f46",
      black: "#171717",
      red: "#ef4444",
      green: "#10b981",
      yellow: "#f59e0b",
      blue: "#3b82f6",
      magenta: "#a855f7",
      cyan: "#06b6d4",
      white: "#e5e5e5",
      brightBlack: "#525252",
      brightRed: "#fb7185",
      brightGreen: "#34d399",
      brightYellow: "#fbbf24",
      brightBlue: "#60a5fa",
      brightMagenta: "#c084fc",
      brightCyan: "#22d3ee",
      brightWhite: "#fafafa",
    },
  },
  {
    id: "light-default",
    label: "Light Default",
    theme: {
      background: "#FBFAF6",
      foreground: "#55503F",
      cursor: "#10b981",
      selectionBackground: "#d4d4d8",
      black: "#171717",
      red: "#dc2626",
      green: "#059669",
      yellow: "#d97706",
      blue: "#2563eb",
      magenta: "#7c3aed",
      cyan: "#0891b2",
      white: "#fafafa",
      brightBlack: "#737373",
      brightRed: "#ef4444",
      brightGreen: "#10b981",
      brightYellow: "#f59e0b",
      brightBlue: "#3b82f6",
      brightMagenta: "#a855f7",
      brightCyan: "#06b6d4",
      brightWhite: "#ffffff",
    },
  },
  {
    id: "solarized-dark",
    label: "Solarized Dark",
    theme: {
      background: "#002B36",
      foreground: "#93A1A1",
      cursor: "#93a1a1",
      selectionBackground: "#073642",
      black: "#073642",
      red: "#dc322f",
      green: "#859900",
      yellow: "#b58900",
      blue: "#268bd2",
      magenta: "#d33682",
      cyan: "#2aa198",
      white: "#eee8d5",
      brightBlack: "#002b36",
      brightRed: "#cb4b16",
      brightGreen: "#586e75",
      brightYellow: "#657b83",
      brightBlue: "#839496",
      brightMagenta: "#6c71c4",
      brightCyan: "#93a1a1",
      brightWhite: "#fdf6e3",
    },
  },
  {
    id: "solarized-light",
    label: "Solarized Light",
    theme: {
      background: "#FDF6E3",
      foreground: "#657B83",
      cursor: "#586e75",
      selectionBackground: "#eee8d5",
      black: "#002b36",
      red: "#dc322f",
      green: "#859900",
      yellow: "#b58900",
      blue: "#268bd2",
      magenta: "#d33682",
      cyan: "#2aa198",
      white: "#073642",
      brightBlack: "#eee8d5",
      brightRed: "#cb4b16",
      brightGreen: "#93a1a1",
      brightYellow: "#839496",
      brightBlue: "#657b83",
      brightMagenta: "#6c71c4",
      brightCyan: "#586e75",
      brightWhite: "#002b36",
    },
  },
  {
    id: "dracula",
    label: "Dracula",
    theme: {
      background: "#282a36",
      foreground: "#f8f8f2",
      cursor: "#f8f8f2",
      selectionBackground: "#44475a",
      black: "#21222c",
      red: "#ff5555",
      green: "#50fa7b",
      yellow: "#f1fa8c",
      blue: "#bd93f9",
      magenta: "#ff79c6",
      cyan: "#8be9fd",
      white: "#f8f8f2",
      brightBlack: "#6272a4",
      brightRed: "#ff6e6e",
      brightGreen: "#69ff94",
      brightYellow: "#ffffa5",
      brightBlue: "#d6acff",
      brightMagenta: "#ff92df",
      brightCyan: "#a4ffff",
      brightWhite: "#ffffff",
    },
  },
  {
    id: "gruvbox-dark",
    label: "Gruvbox Dark",
    theme: {
      background: "#282828",
      foreground: "#EBDBB2",
      cursor: "#ebdbb2",
      selectionBackground: "#504945",
      black: "#282828",
      red: "#cc241d",
      green: "#98971a",
      yellow: "#d79921",
      blue: "#458588",
      magenta: "#b16286",
      cyan: "#689d6a",
      white: "#a89984",
      brightBlack: "#928374",
      brightRed: "#fb4934",
      brightGreen: "#b8bb26",
      brightYellow: "#fabd2f",
      brightBlue: "#83a598",
      brightMagenta: "#d3869b",
      brightCyan: "#8ec07c",
      brightWhite: "#ebdbb2",
    },
  },
];

export function getTheme(id: string, accent?: string): ITheme {
  const theme = TERMINAL_THEMES.find((t) => t.id === id)?.theme ?? TERMINAL_THEMES[0].theme;
  const value = (
    accent ??
    (typeof document === "undefined"
      ? ""
      : getComputedStyle(document.documentElement).getPropertyValue("--c-accent"))
  ).trim();
  const channels = value.split(/\s+/);
  if (channels.length !== 3 || channels.some((channel) => !channel)) return { ...theme };
  const [r, g, b] = channels;
  return {
    ...theme,
    cursor: `rgb(${r}, ${g}, ${b})`,
    selectionBackground: `rgba(${r}, ${g}, ${b}, 0.3)`,
  };
}
