<script setup lang="ts">
import { computed, onMounted, watch } from "vue";
import { useI18n } from "vue-i18n";
import { ArrowRight, ArrowLeft } from "lucide-vue-next";
import PageHeader from "../components/PageHeader.vue";
import FilePane from "../components/FilePane.vue";
import { useFilesStore } from "../stores/files";
import { useHostsStore } from "../stores/hosts";
import { useTransfersStore } from "../stores/transfers";

const { t } = useI18n();
const files = useFilesStore();
const hosts = useHostsStore();
const transfers = useTransfersStore();

onMounted(async () => {
  await hosts.ensureLoaded();
  await files.init();
});

// Передача закончилась — списки могли измениться.
const finished = computed(() => transfers.items.filter((x) => x.status === "DONE").length);
watch(finished, (now, before) => {
  if (now > before) files.refreshBoth();
});

const toRight = computed(() => files.canTransfer("left"));
const toLeft = computed(() => files.canTransfer("right"));
/** Обе панели — этот компьютер: копировать здесь нечем, есть файловый менеджер. */
const bothLocal = computed(
  () => files.left.source?.kind === "local" && files.right.source?.kind === "local",
);
</script>

<template>
  <div class="h-full flex flex-col min-h-0 p-6" data-testid="sftp-page">
    <PageHeader :title="t('files.title')" :description="t('files.subtitle')" />
    <div class="flex-1 min-h-0 grid grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] gap-3">
      <FilePane side="left" />
      <div class="flex flex-col items-center justify-center gap-2">
        <button
          type="button"
          class="p-2 rounded-r8 bg-surface-alt hover:bg-surface-card text-on-surface disabled:opacity-40"
          data-testid="files-copy-right"
          :title="bothLocal ? t('files.sameKind') : t('files.copyRight')"
          :disabled="!toRight"
          @click="files.transfer('left')"
        >
          <ArrowRight :size="16" />
        </button>
        <button
          type="button"
          class="p-2 rounded-r8 bg-surface-alt hover:bg-surface-card text-on-surface disabled:opacity-40"
          data-testid="files-copy-left"
          :title="bothLocal ? t('files.sameKind') : t('files.copyLeft')"
          :disabled="!toLeft"
          @click="files.transfer('right')"
        >
          <ArrowLeft :size="16" />
        </button>
      </div>
      <FilePane side="right" />
    </div>
  </div>
</template>
