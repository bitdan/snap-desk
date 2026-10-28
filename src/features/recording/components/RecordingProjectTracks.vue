<script setup lang="ts">
import { annotationTools } from '@/features/annotations/annotation-tools'
import { moveShape, resizeShape } from '@/features/annotations/annotations'
import { ref, watch } from 'vue'
import type { RecordingProject, TimedAnnotation } from '../recording-annotations'
import { t } from '@/shared/i18n/i18n'
const props=defineProps<{ project:RecordingProject; currentMs:number; disabled:boolean }>()
const emit=defineEmits<{ update:[tracks:TimedAnnotation[]]; seek:[ms:number] }>()
const points=['start','end'] as const
const selected=ref<string | null>(null)
const tracks=ref<TimedAnnotation[]>([])
watch(()=>props.project.annotations, next=>{ tracks.value=next.map(t=>({...t,shape:{...t.shape}})) },{immediate:true})
function change(index:number,patch:Partial<TimedAnnotation>) {
  const next=tracks.value.map((track,i)=>i===index?{...track,...patch}:track)
  tracks.value=next; emit('update',next)
}
function time(index:number,edge:'startMs'|'endMs',value:number) {
  const item=tracks.value[index]
  if (!Number.isFinite(value)) return
  const max=props.project.durationMs
  if (edge==='startMs') change(index,{startMs:Math.max(0,Math.min((item.endMs??max)-20,Math.round(value)))})
  else change(index,{endMs:Math.min(max,Math.max(item.startMs+20,Math.round(value)))})
}
function shapeField(index:number,key:string,value:unknown) { change(index,{shape:{...tracks.value[index].shape,[key]:value}}) }
function coordinate(index:number,point:'start'|'end',axis:'x'|'y',value:number) {
  if(!Number.isFinite(value)) return
  const track=tracks.value[index]
  const crop={x:0,y:0,width:props.project.width,height:props.project.height}
  const next={...track.shape[point],[axis]:Math.max(0,Math.min(axis==='x'?crop.width:crop.height,value))}
  if (track.shape.points?.length) {
    const shape=point==='start' ? moveShape(track.shape,{x:next.x-track.shape.start.x,y:next.y-track.shape.start.y},crop)
      : resizeShape(track.shape,'se',next,crop)
    change(index,{shape})
  } else change(index,{shape:resizeShape(track.shape,point,next,crop)})
}
function timeInput(index:number,edge:'startMs'|'endMs',event:Event) {
  const input=event.target as HTMLInputElement
  const multiplier=input.type==='number'?1000:1
  time(index,edge,input.valueAsNumber*multiplier)
}
function fadeInput(index:number,event:Event) {
  const value=(event.target as HTMLInputElement).valueAsNumber
  if(Number.isFinite(value)) change(index,{fadeOutMs:Math.max(0,Math.min(1000,Math.round(value)))})
}
function shapeInput(index:number,key:'color'|'width'|'text',event:Event) {
  const input=event.target as HTMLInputElement
  shapeField(index,key,key==='width'?input.valueAsNumber:input.value)
}
function coordinateInput(index:number,point:'start'|'end',axis:'x'|'y',event:Event) {
  coordinate(index,point,axis,(event.target as HTMLInputElement).valueAsNumber)
}
function remove(id:string) { tracks.value=tracks.value.filter(t=>t.id!==id); emit('update',tracks.value) }
function add() {
  let n=1; while(tracks.value.some(t=>t.id===`edit-${n}`)) n++
  const startMs=Math.min(Math.max(0,Math.round(props.currentMs)),props.project.durationMs-20)
  const next:TimedAnnotation={id:`edit-${n}`,shape:{kind:'rectangle',start:{x:props.project.width*.25,y:props.project.height*.25},end:{x:props.project.width*.75,y:props.project.height*.75},color:'#ef4444',width:4,opacity:1},startMs,endMs:Math.min(props.project.durationMs,startMs+3000),fadeOutMs:0,order:Math.max(-1,...tracks.value.map(t=>t.order))+1}
  tracks.value=[...tracks.value,next]; selected.value=next.id; emit('update',tracks.value)
}
</script>
<template>
  <section class="project-tracks" :aria-label="t('标注轨道')">
    <header><strong>{{t('标注轨道')}}</strong><button :disabled="disabled || project.durationMs < 20" @click="add">{{t('添加标注')}}</button></header>
    <p v-if="!tracks.length">{{t('没有标注，可从当前播放位置添加。')}}</p>
    <article v-for="(track,index) in tracks" :key="track.id" :class="{selected:selected===track.id}">
      <button class="track-name" :aria-pressed="selected===track.id" @click="selected=track.id; emit('seek',track.startMs)">{{t(annotationTools.find(item => item.tool === track.shape.kind)?.name ?? '标注')}} · {{index+1}}</button>
      <div class="track-time"><span class="track-segment" :style="{left:`${track.startMs/project.durationMs*100}%`,width:`${((track.endMs??project.durationMs)-track.startMs)/project.durationMs*100}%`}" />
        <input type="range" min="0" :max="project.durationMs" step="1" :value="track.startMs" :disabled="disabled" :aria-label="t('标注开始端点')" @change="timeInput(index,'startMs', $event)" />
        <input type="range" min="0" :max="project.durationMs" step="1" :value="track.endMs ?? project.durationMs" :disabled="disabled" :aria-label="t('标注结束端点')" @change="timeInput(index,'endMs', $event)" />
      </div>
      <button :disabled="disabled" :aria-label="t('删除标注')" @click="remove(track.id)">×</button>
      <div v-if="selected===track.id" class="track-fields">
        <label>{{t('开始（秒）')}}<input type="number" min="0" step="0.001" :value="track.startMs/1000" :disabled="disabled" @change="timeInput(index,'startMs', $event)" /></label>
        <label>{{t('结束（秒）')}}<input type="number" min="0" step="0.001" :value="(track.endMs??project.durationMs)/1000" :disabled="disabled" @change="timeInput(index,'endMs', $event)" /></label>
        <label>{{t('淡出（毫秒）')}}<input type="number" min="0" max="1000" step="50" :value="track.fadeOutMs" :disabled="disabled" @change="fadeInput(index, $event)" /></label>
        <label>{{t('颜色')}}<input type="color" :value="track.shape.color" :disabled="disabled" @change="shapeInput(index,'color', $event)" /></label>
        <label>{{t('线宽')}}<input type="number" min="0.5" max="64" step="0.5" :value="track.shape.width" :disabled="disabled" @change="shapeInput(index,'width', $event)" /></label>
        <label v-for="field in points" :key="field">{{t(field==='start'?'起点':'终点')}} X<input type="number" :value="track.shape[field].x" :disabled="disabled" @change="coordinateInput(index,field,'x',$event)" />Y<input type="number" :value="track.shape[field].y" :disabled="disabled" @change="coordinateInput(index,field,'y',$event)" /></label>
        <label v-if="['text','number'].includes(track.shape.kind)">{{t('文字')}}<input type="text" :value="track.shape.text" :disabled="disabled" @change="shapeInput(index,'text', $event)" /></label>
      </div>
    </article>
  </section>
</template>
<style scoped>
.project-tracks { overflow:auto; min-height:0; border-top:1px solid #e1e4e8; padding-top:4px; font-size:12px; }
header { display:flex; align-items:center; justify-content:space-between; } article { display:grid; grid-template-columns:130px minmax(80px,1fr) 28px; gap:4px; padding:4px 0; border-bottom:1px solid #eef0f2; }
article.selected { background:#eff6ff; }.track-name { overflow:hidden; text-overflow:ellipsis; text-align:left; white-space:nowrap; }
.track-time { position:relative; height:24px; background:linear-gradient(transparent 9px,#d8dee4 9px,#d8dee4 15px,transparent 15px); } .track-segment { position:absolute; top:9px; height:6px; background:#93c5fd; } .track-time input { position:absolute; inset:0; width:100%; min-width:0; margin:0; background:transparent; appearance:none; border:0; pointer-events:none; } .track-time input::-webkit-slider-thumb { appearance:none; width:12px; height:22px; background:#eff6ff; border:2px solid #2563eb; border-radius:3px; pointer-events:auto; }
.track-fields { grid-column:1/-1; display:flex; flex-wrap:wrap; gap:6px; } label { display:flex; align-items:center; gap:4px; }
input,button { font:inherit; color:inherit; border:1px solid #d8dee4; border-radius:3px; min-height:24px; background:#fff; }
input[type=number] { width:66px; } input[type=color] { width:30px; } :disabled { opacity:.45; } :focus-visible { outline:2px solid #2563eb; }
p { margin:4px 0; color:#57606a; }
</style>
