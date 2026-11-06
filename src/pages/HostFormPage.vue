<script setup lang="ts">
import { useRouter } from "vue-router";
import { ArrowLeft } from "lucide-vue-next";
import { useI18n } from "vue-i18n";
import HostForm from "../components/HostForm.vue";

const props = defineProps<{
  id?: string;
}>();

const { t } = useI18n();
const router = useRouter();

const hostId = props.id ? Number(props.id) : null;

function goBack() {
  router.push("/hosts");
}
</script>

<template>
  <div class="p-6 max-w-2xl">
    <!-- Header -->
    <div class="flex items-center gap-3 mb-6">
      <button
        class="inline-flex items-center justify-center w-8 h-8 rounded hover:bg-surface-alt text-on-surface-muted"
        :title="t('form.cancel')"
        @click="goBack"
      >
        <ArrowLeft :size="18" />
      </button>
      <h2 class="text-lg font-semibold">
        {{ hostId != null ? t("hosts.editTitle") : t("hosts.add") }}
      </h2>
    </div>

    <HostForm
      :host-id="hostId"
      @save="goBack"
      @cancel="goBack"
    />
  </div>
</template>
