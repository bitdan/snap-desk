import { describe, expect, it } from 'vitest'
import { recordingRegion, defaultInputEffects, newerRecording, recordingAudioStatus, recordingRecoveryStatus, recordingBitrate, recordingStats, recordingStageTimes, recordingCopyStats, recordingActive, recordingConfig, recordingStatus, recordingTime, type RecordingView } from '../../src/features/recording/recording'

const emptyStats = { capturePolls: 0, capturedFrames: 0, captureMillis: 0, captureCopyBytes: 0, compositionCopyBytes: 0, nv12OutputBytes: 0, throttledSamples: 0, composedFrames: 0, compositionMillis: 0, queuedFrames: 0, queueDroppedFrames: 0, staleQueuedFrames: 0, rejectedCaptureFrames: 0, rejectedComposedFrames: 0, recentFps: 0 }
const view: RecordingView = { id: 7, revision: 3, phase: 'paused', seconds: 23, frames: 180, droppedFrames: 2, targetFps: 30, systemAudio: false, microphone: false, encodingMillis: 360, targetBitrate: 8_087_040, encoderKind: 'system', encoderFallback: null, countdownRemaining: 0, message: '', path: null, stats: emptyStats }
describe('录屏会话状态', () => {
  it('明确区分可导出、未封装与已导出的残留', () => {
    const raw = { id: 'recording-pending-example', path: 'D:/video.mp4', finalized: false, stage: 'failed:recording', exported: null, sourceExists: true }
    expect(recordingRecoveryStatus(raw)).toContain('不能直接播放')
    expect(recordingRecoveryStatus({ ...raw, finalized: true })).toContain('可重新导出')
    expect(recordingRecoveryStatus({ ...raw, exported: 'D:/final.mp4' })).toContain('D:/final.mp4')
    expect(recordingRecoveryStatus({ ...raw, exported: 'D:/final.mp4' })).toContain('清理源文件残留')
    expect(recordingRecoveryStatus({ ...raw, finalized: true, sourceExists: false })).toContain('源文件已不存在')
  })
  it('按本次录制快照展示四种音频来源组合', () => {
    expect(recordingAudioStatus({ systemAudio: false, microphone: false })).toBe('无声')
    expect(recordingAudioStatus({ systemAudio: true, microphone: false })).toBe('系统声音')
    expect(recordingAudioStatus({ systemAudio: false, microphone: true })).toBe('麦克风')
    expect(recordingAudioStatus({ systemAudio: true, microphone: true })).toBe('系统声音 + 麦克风')
  })
  it('系统声音与麦克风独立传递，GIF 清除声音请求', () => {
    const region = { x: 0, y: 0, width: 100, height: 100 }
    for (const systemAudio of [false, true]) for (const microphone of [false, true]) {
      expect(recordingConfig(region, 30, true, 0, 'auto', 'balanced', defaultInputEffects(), null, 'mp4', systemAudio, microphone))
        .toMatchObject({ systemAudio, microphone })
    }
    expect(recordingConfig(region, 10, true, 0, 'auto', 'balanced', defaultInputEffects(), null, 'gif', true, true))
      .toMatchObject({ systemAudio: false, microphone: false })
  })
  it('GIF 与 MP4 共用配置入口，格式约束帧率且 GIF 不携带视频码率', () => {
    const region = { x: 0, y: 0, width: 101, height: 99 }
    const config = recordingConfig(region, 10, true, 3, 'auto', 'balanced', defaultInputEffects(), 8_000_000, 'gif')
    expect(config).toMatchObject({ format: 'gif', fps: 10, encoding: 'software', bitrate: null, countdownSeconds: 3, region })
    expect(recordingConfig(region).format).toBe('mp4')
    expect(() => recordingConfig(region, 30, true, 0, 'auto', 'balanced', defaultInputEffects(), null, 'gif')).toThrow('帧率')
    expect(() => recordingConfig(region, 10)).toThrow('帧率')
  })
  it('像素传输量分阶段展示，使用 MiB 单位且不将 NV12 转换输出算作额外复制', () => {
    expect(recordingCopyStats(view)).toBe('采集复制 0.0 MiB · 合成复制 0.0 MiB · NV12 输出 0.0 MiB')
    const stats = { ...emptyStats, captureCopyBytes: 1_048_576, compositionCopyBytes: 1_572_864, nv12OutputBytes: 536_870_912 }
    expect(recordingCopyStats({ ...view, stats })).toBe('采集复制 1.0 MiB · 合成复制 1.5 MiB · NV12 输出 512.0 MiB')
  })
  it('自定义码率换算单位、接受边界与小数，留空恢复自动', () => {
    expect(recordingBitrate('')).toBeNull()
    expect(recordingBitrate('  ')).toBeNull()
    expect(recordingBitrate(1)).toBe(1_000_000)
    expect(recordingBitrate('80')).toBe(80_000_000)
    expect(recordingBitrate('8.5')).toBe(8_500_000)
    for (const value of [0, -1, 0.99, 80.01, NaN, Infinity, '错误']) {
      expect(() => recordingBitrate(value)).toThrow('1–80 Mbps')
    }
    const region = { x: 0, y: 0, width: 100, height: 100 }
    expect(recordingConfig(region).bitrate).toBeNull()
    for (const value of [0, 999_999, 80_000_001, 1_000_000.5, NaN, Infinity]) {
      expect(() => recordingConfig(region, 30, true, 0, 'auto', 'balanced', defaultInputEffects(), value)).toThrow('1–80 Mbps')
    }
  })
  it('键鼠特效默认关闭，提交配置独立于表单和后续会话', () => {
    const region = { x: 0, y: 0, width: 1920, height: 1080 }
    const effects = defaultInputEffects()
    for (const field of ['keyboard', 'clicks', 'movement', 'drag', 'wheel'] as const) expect(effects[field]).toBe(false)
    effects.keyboard = true
    effects.clicks = true
    effects.position = 'bottomRight'
    const config = recordingConfig(region, 30, true, 0, 'auto', 'balanced', effects)
    effects.leftColor[0] = 0
    effects.keyboard = false
    expect(config.inputEffects.keyboard).toBe(true)
    expect(config.inputEffects.clicks).toBe(true)
    expect(config.inputEffects.leftColor).toEqual([64, 160, 255])
    expect(config.inputEffects.position).toBe('bottomRight')
    expect(recordingConfig(region).inputEffects.keyboard).toBe(false)
  })
  it('画质显式传递，默认均衡；统计处理无帧及有效录制时间', () => {
    const region = { x: 0, y: 0, width: 1920, height: 1080 }
    expect(recordingConfig(region).quality).toBe('balanced')
    expect(recordingConfig(region, 60, true, 0, 'auto', 'high').quality).toBe('high')
    expect(recordingStats({ ...view, seconds: 6, stats: { ...emptyStats, recentFps: 29.5 } })).toBe('平均 30.0 FPS · 近 2 秒 29.5 FPS')
    expect(recordingStats({ ...view, seconds: 0, frames: 0 })).toBe('平均 0.0 FPS · 近 2 秒 0.0 FPS')
  })
  it('分阶段耗时使用独立分母，空取样及动画重绘不被当成编码帧', () => {
    const stats = { ...emptyStats, capturedFrames: 2, capturePolls: 10, captureMillis: 5, composedFrames: 8, compositionMillis: 12 }
    expect(recordingStageTimes({ ...view, stats })).toBe('采集 0.5 ms/次 · 合成 1.5 ms/帧 · 编码提交 2.0 ms/帧')
    expect(recordingStageTimes({ ...view, frames: 0 })).toBe('采集 0.0 ms/次 · 合成 0.0 ms/帧 · 编码提交 0.0 ms/帧')
  })
  it('编码默认自动，显式软件模式与倒计时独立传递', () => {
    const region = { x: 0, y: 0, width: 100, height: 100 }
    expect(recordingConfig(region).encoding).toBe('auto')
    const config = recordingConfig(region, 60, true, 5, 'software')
    expect(config.encoding).toBe('software')
    expect(config.countdownSeconds).toBe(5)
  })
  it('倒计时只展示后端剩余时间，取消及开始后忽略迟到状态', () => {
    const countdown: RecordingView = { ...view, phase: 'countdown', countdownRemaining: 3, seconds: 0, frames: 0 }
    expect(recordingStatus(countdown)).toBe('3 秒后开始')
    const running: RecordingView = { ...countdown, phase: 'recording', countdownRemaining: 0, revision: 4 }
    expect(newerRecording(running, countdown)).toBe(running)
    expect(recordingStatus(running)).toBe('录制中')
    const cancelled: RecordingView = { ...countdown, phase: 'cancelled', countdownRemaining: 0, revision: 5 }
    expect(newerRecording(cancelled, countdown)).toBe(cancelled)
    expect(recordingActive(cancelled)).toBe(false)
  })
  it('选区使用物理像素快照，30/60 FPS 与光标偏好传到 Rust，默认无声', () => {
    const region = { x: 12, y: 18, width: 101, height: 99 }
    const config = recordingConfig(region, 60, false)
    region.x = 99
    expect(config).toEqual({ source: 'primaryMonitor', region: { x: 12, y: 18, width: 101, height: 99 }, fps: 60, captureCursor: false, format: 'mp4', systemAudio: false, microphone: false, countdownSeconds: 0, encoding: 'auto', quality: 'balanced', inputEffects: defaultInputEffects(), bitrate: null, hideMainWindow: true })
    expect(recordingConfig(region).fps).toBe(60)
    expect(recordingConfig(region).captureCursor).toBe(true)
  })
  it('暂停、结束后不被迟到的采集查询覆盖', () => {
    expect(newerRecording(view, { ...view, revision: 2, phase: 'recording' })).toBe(view)
    const completed: RecordingView = { ...view, revision: 5, phase: 'completed' }
    expect(newerRecording(completed, view)).toBe(completed)
  })
  it('新会话接受较小修订号，拒绝旧会话及空查询', () => {
    const next = { ...view, id: 8, revision: 1 }
    expect(newerRecording(view, next)).toBe(next)
    expect(newerRecording(next, view)).toBe(next)
    expect(newerRecording(next, null)).toBe(next)
    expect(newerRecording(null, null)).toBeNull()
  })
  it('编码完成前保持占用，终态允许重新录制', () => {
    for (const phase of ['ready', 'countdown', 'recording', 'paused', 'encoding'] as const) expect(recordingActive({ ...view, phase })).toBe(true)
    for (const phase of ['completed', 'cancelled', 'failed'] as const) expect(recordingActive({ ...view, phase })).toBe(false)
    expect(recordingActive(null)).toBe(false)
  })
  it('使用后端时间展示，边界不四舍五入到下一分钟', () => {
    expect(recordingTime(59.99)).toBe('00:59')
    expect(recordingTime(180)).toBe('03:00')
    expect(recordingTime(-1)).toBe('00:00')
  })
  it('待开始保留占用，开始后不接受迟到的待开始查询', () => {
    const ready: RecordingView = { ...view, phase: 'ready', revision: 1, seconds: 0, frames: 0 }
    const running: RecordingView = { ...ready, phase: 'recording', revision: 2 }
    expect(recordingActive(ready)).toBe(true)
    expect(newerRecording(ready, running)).toBe(running)
    expect(newerRecording(running, ready)).toBe(running)
  })
})


describe('多屏录制来源坐标', () => {
  const displays = [
    { x: -1920, y: -200, width: 1920, height: 1080 },
    { x: 0, y: 0, width: 2560, height: 1440 },
  ]
  const origin = { x: -1920, y: -200 }
  it('副屏全屏使用原始物理尺寸，不应用主屏 DPI', () => {
    expect(recordingRegion({ x: 100, y: 40, width: 400, height: 300 }, 'display', displays, origin))
      .toEqual({ x: 0, y: 0, width: 1920, height: 1080 })
    expect(recordingRegion({ x: 2000, y: 300, width: 400, height: 300 }, 'display', displays, origin))
      .toEqual({ x: 1920, y: 200, width: 2560, height: 1440 })
  })
  it('保留区域选区，拒绝跨屏及桌面空洞', () => {
    const region = { x: 1800, y: 300, width: 400, height: 300 }
    expect(() => recordingRegion(region, 'region', displays, origin)).toThrow('单个显示器')
    expect(() => recordingRegion(region, 'display', displays, origin)).toThrow('单个显示器')
    expect(() => recordingRegion({ x: 2200, y: 0, width: 20, height: 20 }, 'region', displays, origin)).toThrow()
    const small = { x: 20, y: 40, width: 400, height: 300 }
    expect(recordingRegion(small, 'region', displays, origin)).toEqual(small)
    expect(recordingRegion(small, 'region', displays, origin)).not.toBe(small)
  })
})
