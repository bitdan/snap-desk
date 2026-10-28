import { computed, type Ref } from 'vue'
import type { AnnotationToolbarEditor } from '@/features/annotations/use-annotations'
import type { RecordingAnnotationView, RecordingAnnotationEditorAction } from './recording-annotations'
import { defaultRecordingAnnotationTools } from './recording-annotations'

/** 只把原画布快照适配为共享工具栏接口；撤销栈、选中图形和样式处理都由画布负责。 */
export function useRecordingAnnotationToolbar(view:Ref<RecordingAnnotationView|null>, execute:(action:RecordingAnnotationEditorAction)=>Promise<void>) {
  return computed<AnnotationToolbarEditor>(()=>{
    const state=view.value?.editor
    const tools=view.value?.tools ?? defaultRecordingAnnotationTools()
    return {...tools,selected:state?.selected ?? -1,pending:state?.pending ?? false,
      interacting:state?.interacting ?? false,error:state?.error ?? '',text:state?.text ?? '',
      history:{present:view.value?.shapes ?? [],past:state?.canUndo ? [[]] : [],future:state?.canRedo ? [[]] : []},
      style:patch=>execute({kind:'style',style:patch}),travel:direction=>execute({kind:direction}),
      remove:()=>execute({kind:'remove'}),clear:()=>execute({kind:'clear'})}
  })
}
