<script setup lang="ts">
import { Search } from "lucide-vue-next";

export interface PageFilter {
  id: string | number;
  label: string;
}

const props = defineProps<{
  title: string;
  description: string;
  searchQuery?: string;
  searchPlaceholder?: string;
  filters?: PageFilter[];
  selectedFilter?: string | number;
  countLabel?: string;
}>();

const emit = defineEmits<{
  "update:searchQuery": [value: string];
  "update:selectedFilter": [value: string | number];
}>();
</script>

<template>
  <header class="mb-5">
    <div class="flex flex-wrap items-start justify-between gap-4">
      <div>
        <h2 class="text-title text-on-surface">{{ props.title }}</h2>
        <p class="mt-1 text-body text-on-surface-muted">{{ props.description }}</p>
      </div>
      <slot />
    </div>

    <div v-if="searchPlaceholder || filters?.length" class="mt-5 flex flex-wrap items-center gap-3">
      <div v-if="searchPlaceholder" class="relative flex-1 min-w-[220px] max-w-xl">
        <Search :size="16" class="absolute left-3 top-1/2 icon-center-y text-on-surface-muted" />
        <input
          :value="searchQuery"
          type="text"
          class="w-full bg-surface-card border border-edge rounded-r8 pl-9 pr-3 py-2 text-sm focus:outline-none focus:border-accent"
          :placeholder="searchPlaceholder"
          @input="emit('update:searchQuery', ($event.target as HTMLInputElement).value)"
        />
      </div>

      <div v-if="filters?.length" class="flex flex-wrap items-center gap-2">
        <button
          v-for="filter in filters"
          :key="filter.id"
          type="button"
          class="h-7 px-3 rounded-full border text-xs"
          :class="
            selectedFilter === filter.id
              ? 'border-accent bg-surface-alt text-on-surface'
              : 'border-edge text-on-surface-muted hover:bg-surface-alt'
          "
          @click="emit('update:selectedFilter', filter.id)"
        >
          {{ filter.label }}
        </button>
      </div>

      <span v-if="countLabel" class="ml-auto text-xs text-on-surface-muted">{{ countLabel }}</span>
    </div>
  </header>
</template>
