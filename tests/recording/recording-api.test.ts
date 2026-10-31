import { describe, expect, it, vi } from 'vitest'
import { defaultRecordingAnnotationTools } from '../helpers/recording-fixtures'
import { defaultInputEffects, recordingConfig } from '../../src/features/recording/recording'
import { defaultSettings } from '../../src/features/settings/settings'

const invoke = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke, isTauri: () => true }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }))
import { api } from '../../src/services/api'

describe('录屏 IPC 契约', () => {
  it('实时标注携带会话 ID，元数据查询携带已有版本', async () => {
    await api.recordingAnnotations()
    expect(invoke).toHaveBeenLastCalledWith('current_recording_annotations', { knownRevision: null })
    await api.recordingAnnotations(3)
    expect(invoke).toHaveBeenLastCalledWith('current_recording_annotations', { knownRevision: 3 })
    await api.setRecordingAnnotationMode(42, true)
    expect(invoke).toHaveBeenLastCalledWith('set_recording_annotation_mode', { id: 42, editing: true })
    await api.setRecordingAnnotationMode(42, false)
    expect(invoke).toHaveBeenLastCalledWith('set_recording_annotation_mode', { id: 42, editing: false })
    await api.updateRecordingAnnotations(42, [])
    expect(invoke).toHaveBeenLastCalledWith('update_recording_annotations', { id: 42, shapes: [], knownRevision: null })
  })
  it('标注工具命令使用会话 ID，不向后端发送文件路径', async()=>{
    const tools=defaultRecordingAnnotationTools()
    await api.setRecordingAnnotationTools(42,tools)
    expect(invoke).toHaveBeenLastCalledWith('set_recording_annotation_tools',{id:42,tools})
    await api.updateRecordingAnnotations(42, [], 3)
    expect(invoke).toHaveBeenLastCalledWith('update_recording_annotations',{id:42,shapes:[],knownRevision:3})
  })
  it('共享编辑器的动作、状态和确认均限定会话及请求编号',async()=>{
    await api.recordingAnnotationEditorAction(42,{kind:'style',style:{color:'#123456'}})
    expect(invoke).toHaveBeenLastCalledWith('recording_annotation_editor_action',{id:42,action:{kind:'style',style:{color:'#123456'}}})
    const status={ready:true,selected:0,pending:false,interacting:false,canUndo:true,canRedo:false,text:'',error:''}
    const tools=defaultRecordingAnnotationTools()
    await api.publishRecordingAnnotationEditor(42,status,tools)
    expect(invoke).toHaveBeenLastCalledWith('publish_recording_annotation_editor',{id:42,editor:status,tools})
    await api.finishRecordingAnnotationEditorAction(42,3,null)
    expect(invoke).toHaveBeenLastCalledWith('finish_recording_annotation_editor_action',{id:42,requestId:3,error:null})
  })
  it('准备阶段的标注连同截图坐标提交，保留窗口来源参数', async () => {
    const rect = { x: 100, y: 50, width: 600, height: 400 }
    const annotations = [{ kind: 'rectangle' as const, start: { x: 110, y: 60 }, end: { x: 150, y: 90 }, color: '#ff0000', width: 2, opacity: 1 }]
    const config = recordingConfig(rect)
    await api.startRecording(42, rect, config, 3, annotations)
    expect(invoke).toHaveBeenLastCalledWith('start_recording', { id: 42, rect, config, target: 3, annotations })
  })
  it('主窗口录屏选区有独立入口，不走普通截图命令', async () => {
    await api.selectRecordingArea()
    expect(invoke).toHaveBeenLastCalledWith('select_recording_area', undefined)
  })
  it('目录 IPC 只发送固定种类，导出与引导完成不携带草稿', async () => {
    await api.openAppDirectory('config')
    expect(invoke).toHaveBeenLastCalledWith('open_app_directory', { kind: 'config' })
    await api.copyAppDirectory('scrollResults')
    expect(invoke).toHaveBeenLastCalledWith('copy_app_directory', { kind: 'scrollResults' })
    await api.exportSettings()
    expect(invoke).toHaveBeenLastCalledWith('export_settings', undefined)
    await api.completeOnboarding()
    expect(invoke).toHaveBeenLastCalledWith('complete_onboarding', undefined)
    await api.setShortcutEditing(true)
    expect(invoke).toHaveBeenLastCalledWith('set_shortcut_editing', { editing: true })
    await api.setShortcutEditing(false)
    expect(invoke).toHaveBeenLastCalledWith('set_shortcut_editing', { editing: false })
  })
  it('快捷键总开关只传布尔值，不携带其他未保存设置', async () => {
    const settings = { ...defaultSettings(), shortcutsEnabled: false }
    invoke.mockResolvedValueOnce(settings)
    expect(await api.setShortcutsEnabled(false)).toEqual(settings)
    expect(invoke).toHaveBeenLastCalledWith('set_shortcuts_enabled', { enabled: false })
  })
  it('查询、选择和恢复录屏目录，取消原生选择器不产生新目录', async () => {
    const directory = { path: 'D:\\Videos\\录屏', isDefault: false }
    invoke.mockResolvedValueOnce(directory)
    expect(await api.currentRecordingDirectory()).toEqual(directory)
    expect(invoke).toHaveBeenLastCalledWith('current_recording_directory', undefined)
    invoke.mockResolvedValueOnce(null)
    expect(await api.chooseRecordingDirectory()).toBeNull()
    expect(invoke).toHaveBeenLastCalledWith('choose_recording_directory', { reset: false })
    const defaultDirectory = { path: 'C:\\AppData\\recordings', isDefault: true }
    invoke.mockResolvedValueOnce(defaultDirectory)
    expect(await api.chooseRecordingDirectory(true)).toEqual(defaultDirectory)
    expect(invoke).toHaveBeenLastCalledWith('choose_recording_directory', { reset: true })
  })
  it('保持原命令及会话 ID，同时发送配置和物理像素选区', async () => {
    const rect = { x: 13, y: 21, width: 101, height: 99 }
    const effects = { ...defaultInputEffects(), keyboard: true, wheel: true, drag: true }
    const config = recordingConfig(rect, 60, false, 5, 'software', 'high', effects, 8_500_000, 'mp4', true, true, false)
    expect(config.hideMainWindow).toBe(false)
    expect(config.systemAudio && config.microphone).toBe(true)
    expect(config.bitrate).toBe(8_500_000)
    expect(config.countdownSeconds).toBe(5)
    expect(config.encoding).toBe('software')
    expect(config.quality).toBe('high')
    expect(config.inputEffects).toEqual(effects)
    invoke.mockResolvedValueOnce(undefined)
    await api.startRecording(42, rect, config)
    expect(invoke).toHaveBeenLastCalledWith('start_recording', { id: 42, rect, config })
    await api.controlRecording(42, 'stop')
    expect(invoke).toHaveBeenLastCalledWith('control_recording', { id: 42, action: 'stop' })
  })

  it('保留平台不支持原因，不在前端假定 Windows 能力', async () => {
    const capabilities = { available: false, reason: 'UnsupportedPlatform：macOS / Linux 原生录屏尚未实现。', sources: [], frameRates: [], captureCursor: false, systemAudio: false, microphone: false, formats: [] }
    invoke.mockResolvedValueOnce(capabilities)
    expect(await api.recordingCapabilities()).toBe(capabilities)
    expect(invoke).toHaveBeenLastCalledWith('recording_capabilities', undefined)
  })
})


it('窗口录制只传当前帧候选编号，不暴露原生句柄', async () => {
  const rect = { x: 20, y: 30, width: 600, height: 400 }
  const config = recordingConfig(rect)
  config.source = 'selectedWindow'
  await api.startRecording(43, rect, config, 2)
  expect(invoke).toHaveBeenLastCalledWith('start_recording', { id: 43, rect, config, target: 2 })
  config.source = 'selectedMonitor'
  await api.startRecording(44, rect, config)
  expect(invoke).toHaveBeenLastCalledWith('start_recording', { id: 44, rect, config })
})
