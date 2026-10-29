<script setup lang="ts">
import type { HTMLAttributes } from 'vue'
import { Primitive, type PrimitiveProps } from 'reka-ui'
import { cva } from 'class-variance-authority'
import { cn } from '@/shared/ui/utils'

const props = withDefaults(defineProps<PrimitiveProps & {
  variant?: 'default' | 'outline' | 'ghost' | 'destructive'
  class?: HTMLAttributes['class']
}>(), { as: 'button', variant: 'default' })
const variants = cva('inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md px-4 py-2.5 text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500 disabled:pointer-events-none disabled:opacity-50', {
  variants: { variant: {
    default: 'bg-primary text-white hover:bg-blue-700',
    outline: 'border border-zinc-200 bg-white text-zinc-700 hover:bg-zinc-50',
    ghost: 'text-zinc-500 hover:bg-zinc-100',
    destructive: 'bg-red-600 text-white hover:bg-red-700 focus-visible:ring-red-500',
  } }, defaultVariants: { variant: 'default' },
})
</script>
<template><Primitive :as="as" :as-child="asChild" :class="cn(variants({ variant }), props.class)"><slot /></Primitive></template>
