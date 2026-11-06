<script setup lang="ts">
withDefaults(defineProps<{ size?: number }>(), { size: 20 });
</script>

<template>
  <svg
    :width="size"
    :height="size"
    viewBox="0 0 20 20"
    fill="none"
    aria-hidden="true"
  >
    <path
      d="M3 19V9.5C3 5.9 6.1 3 10 3s7 2.9 7 6.5V19"
      stroke="currentColor"
      stroke-width="1.4"
      stroke-linecap="round"
    />
    <path
      d="M6.5 19v-9C6.5 7.6 8 6 10 6s3.5 1.6 3.5 4v9"
      stroke="currentColor"
      stroke-width="1.1"
      opacity="0.5"
    />
  </svg>
</template>
