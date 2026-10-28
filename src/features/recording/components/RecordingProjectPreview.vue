<script setup lang="ts">
import { computed } from 'vue'
import type { RecordingProject } from '../recording-annotations'
import { annotationsAt } from '../recording-annotations'
import AnnotationSurface from '@/features/annotations/components/AnnotationSurface.vue'
import { useAnnotations } from '@/features/annotations/use-annotations'
const props=defineProps<{project:RecordingProject; currentMs:number}>()
const bounds=computed(()=>({x:0,y:0,width:props.project.width,height:props.project.height}))
const base=useAnnotations(()=>bounds.value,()=>'',()=>true,async()=>{})
const editor=computed(()=>({...base,history:{past:[],future:[],present:annotationsAt(props.project.annotations,props.currentMs)}}))
</script>
<template><AnnotationSurface class="project-preview" preserve-aspect-ratio="xMidYMid meet" :editor="editor" :width="project.width" :height="project.height" :bounds="bounds" :enabled="false" /></template>
<style scoped>.project-preview { position:absolute; inset:0; width:100%; height:100%; pointer-events:none; }</style>
