/** 唯一的命令调用边界：错误解码、失败日志与监听订阅统一处理。 */
import { invoke as nativeInvoke, isTauri, type InvokeArgs } from '@tauri-apps/api/core'
import { listen as nativeListen, type EventCallback } from '@tauri-apps/api/event'
import { errorMessage } from './errors'
import { logIpcFailure } from './logger'
import type { IpcCommand } from './ipc-commands'

export const desktop = isTauri()
// 在唯一的 API 边界解码错误，截图、贴图与主页均能看到原生错误正文及失败操作。
export async function invoke<T>(command: IpcCommand, args?: InvokeArgs): Promise<T> {
  try { return await nativeInvoke<T>(command, args) }
  catch (error) {
    const message = errorMessage(error)
    logIpcFailure(command, message, args && 'id' in args ? args.id : undefined)
    throw new Error(`${command}：${message}`)
  }
}
export async function listen<T>(event: string, callback: EventCallback<T>, target?: string) {
  try { return await (target ? nativeListen<T>(event, callback, { target }) : nativeListen<T>(event, callback)) }
  catch (error) { throw new Error(`订阅 ${event}：${errorMessage(error)}`) }
}
