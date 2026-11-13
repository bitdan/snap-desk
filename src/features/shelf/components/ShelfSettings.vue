<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { api, desktop } from '@/services/api'
import { t, te } from '@/shared/i18n/i18n'
import Button from '@/shared/ui/button/Button.vue'
import SwitchControl from '@/shared/ui/SwitchControl'
import SettingRow from '@/shared/ui/SettingRow'
import { defaultShelfSettings, shelfCategories } from '../types'

const settings = ref(defaultShelfSettings()), ready = ref(false), busy = ref(false), error = ref('')
const recordingEnabled = ref(false)
async function load() {
  if (!desktop) return
  busy.value = true; error.value = ''
  try { const value = await api.shelfSnapshot(); settings.value = value.settings; recordingEnabled.value = value.recordingEnabled; ready.value = true }
  catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
async function save() {
  if (!ready.value || busy.value) return
  busy.value = true; error.value = ''
  try { await api.shelfConfigure({ ...settings.value, categories: [...settings.value.categories] }) }
  catch (e) { error.value = String(e) }
  finally { busy.value = false }
}
function all(event: Event) {
  settings.value.categories = (event.target as HTMLInputElement).checked ? shelfCategories.map(c => c.key) : []
  void save()
}
async function show() {
  try { await api.shelfVisibility(true) } catch (e) { error.value = String(e) }
}
onMounted(load)
</script>
<template>
  <section class="shelf-settings" aria-labelledby="shelf-settings-title">
    <h2 id="shelf-settings-title" class="section-heading">{{ t('顶部暂存架') }}</h2>
    <fieldset class="settings-card" :disabled="!desktop || !ready || busy">
      <SettingRow :label="t('开启顶部暂存架')" for="shelf-enabled">
        <Button variant="outline" :disabled="!settings.enabled" @click="show">{{ t('展开暂存架') }}</Button>
        <SwitchControl id="shelf-enabled" v-model="settings.enabled" @change="save" />
      </SettingRow>
      <SettingRow :label="t('鼠标停留屏幕顶部中央时展开')" for="shelf-top-hover">
        <SwitchControl id="shelf-top-hover" v-model="settings.topHover" :disabled="!settings.enabled" @change="save" />
      </SettingRow>
      <div class="setting-row shelf-categories" role="group" aria-labelledby="shelf-categories-title">
        <span id="shelf-categories-title">{{ t('放置类别') }}</span>
        <div class="shelf-category-options">
          <label class="shelf-option"><input type="checkbox" :checked="settings.categories.length === shelfCategories.length" :indeterminate="settings.categories.length > 0 && settings.categories.length < shelfCategories.length" @change="all" />{{ t('全部') }}</label>
          <label v-for="category in shelfCategories" :key="category.key" class="shelf-option"><input v-model="settings.categories" type="checkbox" :value="category.key" @change="save" />{{ t(category.label) }}</label>
        </div>
      </div>
    </fieldset>
    <p v-if="!settings.categories.length" class="section-note">{{ t('请选择要放置的类别。') }}</p>
    <p v-if="settings.enabled && !recordingEnabled" class="section-note">{{ t('剪贴板记录已关闭，暂存架仅展示已有历史。') }}</p>
    <p v-if="error" role="alert" class="notice error">{{ te(error) }} <Button variant="ghost" :disabled="busy" @click="load">{{ t('重新读取') }}</Button></p>
  </section>
</template>
<style scoped>
fieldset { margin: 0; min-width: 0; }
.shelf-option { display: inline-flex; align-items: center; gap: 6px; font-size: 13px; cursor: pointer; }
.shelf-category-options { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: 8px 16px; }
</style>
