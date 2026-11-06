import { defineStore } from "pinia";
import { ref } from "vue";

export const useCommandPaletteStore = defineStore("command-palette", () => {
  const isOpen = ref(false);

  function open() {
    isOpen.value = true;
  }

  function close() {
    isOpen.value = false;
  }

  return {
    isOpen,
    open,
    close,
  };
});
